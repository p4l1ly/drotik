extern crate resvg;
extern crate ndarray;
extern crate quick_xml;
extern crate base64;
extern crate byteorder;

use quick_xml::events::BytesText;
use quick_xml::writer::Writer;
use resvg::usvg;
use resvg::tiny_skia;
use ndarray::prelude::*;
use ndarray::{Array3};
use byteorder::{ByteOrder, LittleEndian};

const WIDTH: u32 = 60;
const HEIGHT_OFFSET: usize = 2;
const HEIGHT_OFFSET32: u32 = HEIGHT_OFFSET as u32;
const DEPTH: u32 = 20;
const ALPHA_THRESHOLD: u8 = 50;
const STRENGTH_DECAY: f64 = 0.85;
const COUNT_DECAY: f64 = 0.05;
const FINAL_STRENGTH_DIFF_DECAY: f64 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Constraint {
    RedMust,
    RedArea,
    BlueMust,
    BlueArea,
    Mustnt,
    Any,
}

use Constraint::*;

fn change_path_width(tree: &mut usvg::Tree, width: f64) {
    let path = tree.node_by_id("path1326").unwrap();
    match &mut *path.borrow_mut() {
        usvg::NodeKind::Path(path) => {
            path.stroke.as_mut().unwrap().width =
                unsafe{usvg::NonZeroPositiveF64::new_unchecked(width)};
        },
        _ => panic!("path1175 is not a path"),
    };
}

fn apply_path(
    tree: &mut usvg::Tree,
    out: &mut Array3<Constraint>,
    width: f64,
    z: usize,
) {
    change_path_width(tree, width);

    let height = (*out).dim().1 - 2 * HEIGHT_OFFSET;
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, height as u32).unwrap();
    resvg::render(
        &tree,
        usvg::FitTo::Size(WIDTH, (*out).len() as u32),
        tiny_skia::Transform::default(),
        pixmap.as_mut(),
    ).unwrap();

    for x in 0..WIDTH {
        let xsz = x as usize;
        let mut above = false;
        let mut above_last = false;
        for y in 0..height + 2*HEIGHT_OFFSET {
            if y >= HEIGHT_OFFSET && y < height + HEIGHT_OFFSET && pixmap.pixel(x, y as u32 - HEIGHT_OFFSET32).unwrap().alpha() > ALPHA_THRESHOLD {
                out[(xsz, y, z)] = Mustnt;
                above = true;
                if above_last {
                    above_last = false;
                    let mut y2 = y - 1;
                    loop {
                        let color2 = pixmap.pixel(x, y2 as u32 - HEIGHT_OFFSET32).unwrap();
                        if color2.alpha() > ALPHA_THRESHOLD {
                            break;
                        }
                        out[(xsz, y2, z)] = Mustnt;
                        y2 -= 1;
                    }
                }
            } else if above {
                above_last = true;
                out[(xsz, y, z)] = RedMust;
            } else {
                out[(xsz, y, z)] = BlueMust;
            }
        }
    }
}

type Dim = (usize, usize, usize);

fn main() {
    // Load SVG
    let mut opt = usvg::Options::default();
    opt.resources_dir = std::fs::canonicalize("resources").ok();
    let svg_data = std::fs::read("resources/wire.svg").unwrap();
    let mut tree = usvg::Tree::from_data(&svg_data, &opt).unwrap();

    let svg_data1 = std::fs::read("resources/wire1.svg").unwrap();
    let mut tree1 = usvg::Tree::from_data(&svg_data1, &opt).unwrap();

    // get raster height
    let tree_size = tree.size;
    let height: u32 = (tree_size.height() * WIDTH as f64 / tree_size.width()).ceil() as u32 + 2 * HEIGHT_OFFSET32;

    // create raster
    let dims = [WIDTH as usize, height as usize, DEPTH as usize];
    let mut constraints = Array3::from_elem(dims, Any);

    let width = WIDTH as usize;
    let height = height as usize;
    let depth = DEPTH as usize;

    // create basic constraints
    apply_path(&mut tree, &mut constraints, 0.8, 0);
    apply_path(&mut tree, &mut constraints, 0.4, 1);

    apply_path(&mut tree1, &mut constraints, 0.8, depth - 1);
    apply_path(&mut tree1, &mut constraints, 0.4, depth - 2);

    constraints.slice_mut(s![.., 0..2, ..]).fill(BlueMust);
    constraints.slice_mut(s![.., height - 2 .. height, ..]).fill(RedMust);

    let add_neighbours = |pos: Dim, target: &mut Vec<Dim>| {
        let (xm, ym, zm) = pos;
        if xm > 0 {
            target.push((xm - 1, ym, zm));
        }
        if xm < width - 1 {
            target.push((xm + 1, ym, zm));
        }
        if ym > 0 {
            target.push((xm, ym - 1, zm));
        }
        if ym < height - 1 {
            target.push((xm, ym + 1, zm));
        }
        if zm > 0 {
            target.push((xm, ym, zm - 1));
        }
        if zm < depth - 1 {
            target.push((xm, ym, zm + 1));
        }
    };

    // A*
    {
        for _ in 0..3 {
            let mut astar_raster = Array3::from_elem(dims, 0.0);

            // Count generators
            let mut origin_count = 0;
            for x in 0..(WIDTH as usize) {
                for y in 0..(height as usize) {
                    for z in 0..(DEPTH as usize) {
                        if constraints[(x, y, z)] == BlueMust ||
                            constraints[(x, y, z)] == RedMust {
                            origin_count += 1;
                        }
                    }
                }
            }

            // Perform A*
            let mut processed_origin_count = 0;
            for x in 0..width {
                for y in 0..height {
                    for z in 0..depth {
                        for (generator, constr1, constr2, signum) in [
                            (BlueMust, RedMust, RedArea, 1.0),
                            (RedMust, BlueMust, BlueArea, -1.0),
                        ] {
                            if constraints[(x, y, z)] == generator {
                                let mut generator_astar_raster = Array3::from_elem(dims, 0.0);
                                let mut final_strength = 1.0;
                                let mut final_strength_diff = 2.0;
                                let mut strength = 1.0;
                                let mut visited = Array3::from_elem(dims, false);
                                let mut todo_next = vec![(x, y, z)];
                                let mut todo_next2 = vec![];
                                let mut todo_next3 = vec![];
                                // let mut todo_next4 = vec![];

                                let mut last_count = 1.0;

                                while !todo_next.is_empty() {
                                    let todo = todo_next;
                                    todo_next = todo_next2;
                                    todo_next2 = todo_next3;
                                    todo_next3 = vec![];
                                    // todo_next4 = vec![];

                                    let stranger_penalty = {STRENGTH_DECAY * STRENGTH_DECAY} / (last_count * last_count);
                                    let mut count = 0;
                                    for pos in todo {
                                        if visited[pos] { continue; }
                                        visited[pos] = true;
                                        let constr = constraints[pos];
                                        if constr == Mustnt || constr == constr1 || constr == constr2 {
                                            final_strength *= 1.0 + final_strength_diff;
                                        }
                                        if constr == Mustnt { continue; }
                                        if constr == constr1 { continue; }
                                        if constr == constr2 {
                                            generator_astar_raster[pos] += strength * stranger_penalty;
                                            add_neighbours(pos, &mut todo_next3);
                                        } else {
                                            generator_astar_raster[pos] += strength;
                                            add_neighbours(pos, &mut todo_next);
                                            count += 1;
                                        }
                                    }
                                    if count != 0 {
                                        last_count = count as f64 * COUNT_DECAY + 1.0 * (1.0 - COUNT_DECAY);
                                        strength *= STRENGTH_DECAY / last_count;
                                        final_strength_diff *= FINAL_STRENGTH_DIFF_DECAY;
                                    }
                                }

                                if processed_origin_count % 25 == 0 {
                                    println!("origin_count {} / {}", processed_origin_count, origin_count);
                                }
                                processed_origin_count += 1;
                                generator_astar_raster *= signum * final_strength;
                                astar_raster += &generator_astar_raster;
                                break;
                            }
                        }
                    }
                }
            }

            // Count areas
            for x in 0..width {
                for z in 0..depth {
                    let mut nonblue = false;
                    for y in 0..height {
                        let constr = constraints[(x, y, z)];
                        if matches!(constr, Any | RedArea | BlueArea) {
                            let astar_val = astar_raster[(x, y, z)];
                            constraints[(x, y, z)] =
                                if nonblue {
                                    RedArea
                                } else if astar_val > 0.0 {
                                    BlueArea
                                } else if astar_val < 0.0 {
                                    nonblue = true;
                                    RedArea
                                } else {
                                    BlueArea
                                };
                        } else if matches!(constr, RedMust | Mustnt) {
                            nonblue = true;
                        }
                    }
                }
            }
        }
    }


    // output
    let mut outbytes = vec![0; 4 + constraints.len()];
    LittleEndian::write_u32(&mut outbytes, constraints.len() as u32);
    let mut i = 4;
    for z in 0..depth {
        for y in 0..height {
            for x in 0..width {
                outbytes[i] = constraints[(x, y, z)] as u8;
                i += 1;
            }
        }
    }

    println!("OLEN {} {} {} {}", width, height, depth, outbytes.len());

    let extent = &*format!("0 {} 0 {} 0 {}", width, height, depth);
    let mut outfile = std::fs::File::create("/tmp/out.vti").unwrap();
    let mut writer = Writer::new(&mut outfile);
    writer.create_element("VTKFile")
        .with_attribute(("type", "ImageData"))
        .with_attribute(("version", "0.1"))
        .with_attribute(("byte_order", "LittleEndian"))
        .write_inner_content(|writer| {
            writer.create_element("ImageData")
                .with_attribute(("WholeExtent", extent))
                .with_attribute(("Origin", "0 0 0"))
                .with_attribute(("Spacing", "1 1 1"))
                .write_inner_content(|writer| {
                    writer.create_element("Piece")
                        .with_attribute(("Extent", extent))
                        .write_inner_content(|writer| {
                            writer.create_element("CellData")
                                .with_attribute(("Scalars", "cell_scalars"))
                                .write_inner_content(|writer| {
                                    writer.create_element("DataArray")
                                        .with_attribute(("Name", "cell_scalars"))
                                        .with_attribute(("type", "UInt8"))
                                        .with_attribute(("format", "binary"))
                                        .write_text_content(
                                            BytesText::new(&*base64::encode(&outbytes))
                                        )?;
                                    Ok(())
                                })?;
                            Ok(())
                        })?;
                    Ok(())
                })?;
            Ok(())
        }).unwrap();
}
