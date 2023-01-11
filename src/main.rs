extern crate resvg;
extern crate ndarray;
extern crate quick_xml;
extern crate base64;
extern crate byteorder;
extern crate crossbeam;

use quick_xml::events::BytesText;
use quick_xml::writer::Writer;
use resvg::usvg;
use resvg::tiny_skia;
use ndarray::prelude::*;
use ndarray::{Array3};
use byteorder::{ByteOrder, LittleEndian};
use std::sync::{Mutex};

const WIDTH: u32 = 150;
const DEPTH: u32 = 50;
const HEIGHT_OFFSET: usize = 2;
const HEIGHT_OFFSET32: u32 = HEIGHT_OFFSET as u32;
const ALPHA_THRESHOLD: u8 = 128;
const STRENGTH_DECAY: f64 = 0.6;
const COUNT_DECAY: f64 = 0.5;
const FINAL_STRENGTH_DIFF_START: f64 = 2.0;
const FINAL_STRENGTH_DIFF_DECAY: f64 = 0.55;
const CUT_WIDTH: f64 = 0.3;
const MAX_POSTFILL_DISTANCE: usize = 5;
const MAX_OTHER_COUNT: usize = 2;
const ITERATIONS: usize = 2;

const X1: usize = 0;
const X2: usize = WIDTH as usize;
const Z1: usize = 0;
const Z2: usize = DEPTH as usize;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Constraint {
    RedMust,
    RedArea,
    RedPostfill,
    BlueMust,
    BlueArea,
    BluePostfill,
    Mustnt,
    Any,
    RedMustnt,
    BlueMustnt,
}

// TODO
// - option: nacitat z vti alebo zo svg
//   --svg path1 path2 path3 path4
//   --vti path
// - po kazdej iteracii zapisat do suboru
//   --out out1 out2 out3 ...
// - urobit vysek --x1 --x2 --z1 --z2

use Constraint::*;

fn change_path_width(tree: &mut usvg::Tree, width: f64) {
    let path = tree.node_by_id("the_path").unwrap();
    match &mut *path.borrow_mut() {
        usvg::NodeKind::Path(path) => {
            path.stroke.as_mut().unwrap().width =
                unsafe{usvg::NonZeroPositiveF64::new_unchecked(width)};
        },
        _ => panic!("the_path is not a path"),
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
        usvg::FitTo::Size(WIDTH, height as u32),
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

fn apply_cut_path(
    tree: &mut usvg::Tree,
    out: &mut Array3<Constraint>,
    is_blue: bool,
) {
    change_path_width(tree, CUT_WIDTH);

    let tree_size = tree.size;
    let tree_height: f32 = tree_size.height() as f32;
    let tree_width: f32 = tree_size.width() as f32;

    let height = (*out).dim().1;
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, DEPTH).unwrap();
    resvg::render(
        &tree,
        usvg::FitTo::Original,
        tiny_skia::Transform::from_scale(WIDTH as f32 / tree_width, (DEPTH + 1) as f32 / tree_height),
        pixmap.as_mut(),
    ).unwrap();

    for x in 0..WIDTH {
        let xsz = x as usize;
        for z in 0..DEPTH {
            let zsz = z as usize;
            if pixmap.pixel(x, z).unwrap().alpha() > ALPHA_THRESHOLD {
                for y in 0..height {
                    let old = out[(xsz, y, zsz)];
                    out[(xsz, y, zsz)] =
                        if old == Mustnt { Mustnt }
                        else if is_blue {
                            if old == RedMustnt { Mustnt } else { BlueMustnt }
                        } else {
                            if old == BlueMustnt { Mustnt } else { RedMustnt }
                        };
                }
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

    let svg_data = std::fs::read("resources/wire1.svg").unwrap();
    let mut tree1 = usvg::Tree::from_data(&svg_data, &opt).unwrap();

    let svg_data = std::fs::read("resources/predel.svg").unwrap();
    let mut tree_cut = usvg::Tree::from_data(&svg_data, &opt).unwrap();

    let svg_data = std::fs::read("resources/predel1.svg").unwrap();
    let mut tree_cut1 = usvg::Tree::from_data(&svg_data, &opt).unwrap();

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
    apply_path(&mut tree, &mut constraints, 1.4, 0);
    apply_path(&mut tree, &mut constraints, 1.1, 1);
    apply_path(&mut tree, &mut constraints, 0.5, 2);

    apply_path(&mut tree1, &mut constraints, 1.4, depth - 1);
    apply_path(&mut tree1, &mut constraints, 1.1, depth - 2);
    apply_path(&mut tree1, &mut constraints, 0.5, depth - 3);

    constraints.slice_mut(s![.., 0..2, ..]).fill(BlueMust);
    constraints.slice_mut(s![.., height - 2 .. height, ..]).fill(RedMust);

    apply_cut_path(&mut tree_cut, &mut constraints, false);
    apply_cut_path(&mut tree_cut1, &mut constraints, true);

    let add_neighbours = |pos: Dim, target: &mut Vec<Dim>| {
        let (xm, ym, zm) = pos;
        if xm > X1 {
            target.push((xm - 1, ym, zm));
        }
        if xm < X2 - 1 {
            target.push((xm + 1, ym, zm));
        }
        if ym > 0 {
            target.push((xm, ym - 1, zm));
        }
        if ym < height - 1 {
            target.push((xm, ym + 1, zm));
        }
        if zm > Z1 {
            target.push((xm, ym, zm - 1));
        }
        if zm < Z2 - 1 {
            target.push((xm, ym, zm + 1));
        }
    };

    let add_horizontal_neighbours = |pos: Dim, target: &mut Vec<Dim>| {
        let (xm, ym, zm) = pos;
        if xm > X1 {
            target.push((xm - 1, ym, zm));
        }
        if xm < X2 - 1 {
            target.push((xm + 1, ym, zm));
        }
        if zm > Z1 {
            target.push((xm, ym, zm - 1));
        }
        if zm < Z2 - 1 {
            target.push((xm, ym, zm + 1));
        }
    };

    // A*
    {
        for iteration in 0..ITERATIONS {
            // Count generators
            let mut origin_count = 0;
            for x in X1..X2 {
                for y in 0..height {
                    for z in Z1..Z2 {
                        if constraints[(x, y, z)] == BlueMust ||
                            constraints[(x, y, z)] == RedMust {
                            origin_count += 1;
                        }
                    }
                }
            }

            let mut jobs = vec![];

            // Perform A*
            for x in X1..X2 {
                for y in 0..height {
                    for z in Z1..Z2 {
                        for (generator, constr1, constr2, signum, my_mustnt, other_postfill, other_mustnt, my_postfill) in [
                            (BlueMust, RedMust, RedArea, 1.0, BlueMustnt, RedPostfill, RedMustnt, BluePostfill),
                            (RedMust, BlueMust, BlueArea, -1.0, RedMustnt, BluePostfill, BlueMustnt, RedPostfill),
                        ] {
                            if constraints[(x, y, z)] == generator {
                                jobs.push((x, y, z, constr1, constr2, signum, my_mustnt, other_postfill, other_mustnt, my_postfill));
                                break;
                            }
                        }
                    }
                }
            }

            let jobs = Mutex::new(jobs);
            let mutexed = Mutex::new((Array3::from_elem(dims, 0.0), 0));

            crossbeam::scope(|scope| {
                for _ in 0..8 {
                    scope.spawn(|_| {
                        loop {
                            let job = jobs.lock().unwrap().pop();
                            match job {
                                Some((x, y, z, constr1, constr2, signum, my_mustnt, other_postfill, other_mustnt, my_postfill)) => {
                                    let mut generator_astar_raster = Array3::from_elem(dims, 0.0);
                                    let mut final_strength = 1.0;
                                    let mut final_strength_diff = FINAL_STRENGTH_DIFF_START;
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
                                            if constr == Mustnt || constr == my_mustnt || constr == other_postfill || constr == constr1 || constr == constr2 {
                                                final_strength *= 1.0 + final_strength_diff;
                                            }
                                            if constr == Mustnt { continue; }
                                            if constr == constr1 { continue; }
                                            if constr == my_mustnt || constr == other_postfill { continue; }
                                            if constr == other_mustnt || constr == my_postfill {
                                                add_horizontal_neighbours(pos, &mut todo_next);
                                            } else if constr == constr2 {
                                                generator_astar_raster[pos] += strength * stranger_penalty;
                                                add_neighbours(pos, &mut todo_next3);
                                            } else {
                                                generator_astar_raster[pos] += strength;
                                                add_neighbours(pos, &mut todo_next);
                                                count += 1;
                                            }
                                        }
                                        if count != 0 {
                                            last_count = (1.0 / count as f64) * COUNT_DECAY + 1.0 - COUNT_DECAY;
                                            strength *= STRENGTH_DECAY * last_count;
                                            final_strength_diff *= FINAL_STRENGTH_DIFF_DECAY;
                                        }
                                    }

                                    generator_astar_raster *= signum * final_strength;

                                    let (astar_raster, processed_origin_count) = &mut *mutexed.lock().unwrap();
                                    if *processed_origin_count % 25 == 0 {
                                        println!("origin_count {} {}/{}", iteration, processed_origin_count, origin_count);
                                    }
                                    *processed_origin_count += 1;
                                    *astar_raster += &generator_astar_raster;
                                },
                                None => break,
                            }
                        }
                    });
                }
            }).unwrap();

            let (astar_raster, _) = mutexed.into_inner().unwrap();

            // Count areas
            for x in X1..X2 {
                for z in Z1..Z2 {
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

    // Final RedMustnt and BlueMustnt removal.
    for y in 0..height {
        for z in Z1..Z2 {
            'nextpt: for x in X1..X2 {
                let constr = constraints[(x, y, z)];
                if constr == RedMustnt {
                    let mut visited = Array3::from_elem(dims, false);
                    let mut todo_next = vec![(x, y, z)];
                    let mut other_count = 0;
                    for _ in 0..MAX_POSTFILL_DISTANCE {
                        if todo_next.is_empty() {
                            break;
                        }
                        let todo = todo_next;
                        todo_next = vec![];
                        let mut has_other = false;
                        for pos in todo {
                            if visited[pos] { continue; }
                            visited[pos] = true;
                            let constr2 = constraints[pos];
                            if matches!(constr2, BlueMust | BlueArea) {
                                constraints[(x, y, z)] = BluePostfill;
                                continue 'nextpt;
                            } else if matches!(constr2, RedMustnt | BluePostfill) {
                                add_horizontal_neighbours(pos, &mut todo_next);
                            } else {
                                has_other = true;
                            }
                        }
                        if has_other {
                            other_count += 1;
                            if other_count == MAX_OTHER_COUNT {
                                break;
                            }
                        }
                    }
                } else if constr == BlueMustnt {
                    let mut visited = Array3::from_elem(dims, false);
                    let mut todo_next = vec![(x, y, z)];
                    let mut other_count = 0;
                    for _ in 0..MAX_POSTFILL_DISTANCE {
                        if todo_next.is_empty() {
                            break;
                        }
                        let todo = todo_next;
                        todo_next = vec![];
                        let mut has_other = false;
                        for pos in todo {
                            if visited[pos] { continue; }
                            visited[pos] = true;
                            let constr2 = constraints[pos];
                            if matches!(constr2, RedMust | RedArea) {
                                constraints[(x, y, z)] = RedPostfill;
                                continue 'nextpt;
                            } else if matches!(constr2, BlueMustnt | RedPostfill) {
                                add_horizontal_neighbours(pos, &mut todo_next);
                            } else {
                                has_other = true;
                            }
                        }
                        if has_other {
                            other_count += 1;
                            if other_count == MAX_OTHER_COUNT {
                                break;
                            }
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
