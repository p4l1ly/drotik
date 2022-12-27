extern crate resvg;
extern crate ndarray;
extern crate quick_xml;
extern crate base64;
extern crate byteorder;

use quick_xml::events::BytesText;
use quick_xml::writer::Writer;
use resvg::usvg;
use resvg::tiny_skia;
use ndarray::{Array3};
use byteorder::{ByteOrder, LittleEndian};

const WIDTH: u32 = 1000;
const DEPTH: u32 = 200;

#[derive(Clone, Copy)]
enum Point {
    Top,
    Bottom,
    Mix(u32, u32),
    Forbidden,
}

fn change_path_width(tree: &mut usvg::Tree, width: f64) {
    let path = tree.node_by_id("path1175").unwrap();
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
    out: &mut Array3<Point>,
    width: f64
) {
    change_path_width(tree, width);

    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, (*out).dim().1 as u32).unwrap();
    resvg::render(
        &tree,
        usvg::FitTo::Size(WIDTH, (*out).len() as u32),
        tiny_skia::Transform::default(),
        pixmap.as_mut(),
    ).unwrap();
}

fn main() {
    let mut opt = usvg::Options::default();
    opt.resources_dir = std::fs::canonicalize("resources").ok();

    let svg_data = std::fs::read("resources/wire.svg").unwrap();
    let mut tree = usvg::Tree::from_data(&svg_data, &opt).unwrap();
    let tree_size = tree.size;
    let height: u32 = (tree_size.height() * WIDTH as f64 / tree_size.width()).ceil() as u32;

    let mut out = Array3::from_elem(
        [WIDTH as usize, height as usize, DEPTH as usize],
        Point::Top
    );

    apply_path(&mut tree, &mut out, 0.1);

    let mut outfile = std::fs::File::create("/tmp/out.vti").unwrap();
    let mut writer = Writer::new(&mut outfile);

    let outbytes0 = out.mapv(|x| {
        match x {
            Point::Top => 0,
            Point::Bottom => 1,
            Point::Mix(_, _) => 2,
            Point::Forbidden => 3 as u8,
        }
    });
    let outbytes1 = outbytes0.as_slice().unwrap();
    let mut outbytes2 = vec![0; 4];
    LittleEndian::write_u32(&mut outbytes2, out.len() as u32);
    outbytes2.extend(outbytes1);

    println!("OLEN {} {} {} {}", WIDTH, height, DEPTH, outbytes2.len());
    let extent = &*format!("0 {} 0 {} 0 {}", WIDTH, height, DEPTH);
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
                                            BytesText::new(&*base64::encode(&outbytes2))
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
