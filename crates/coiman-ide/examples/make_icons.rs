//! Renders `assets/icon/coiman.svg` into the PNGs and the Windows `.ico` the app and its
//! website use. Run after changing the SVG:
//!
//! ```text
//! cargo run --release --example make_icons
//! ```

use std::io::BufWriter;
use std::path::Path;

const SIZES: [u32; 9] = [16, 24, 32, 48, 64, 128, 256, 512, 1024];
/// What a Windows icon holds.
const ICO_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];

fn render(tree: &resvg::usvg::Tree, size: u32) -> image::RgbaImage {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("a size above zero");
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia's pixels are premultiplied; PNG's aren't.
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for p in pixmap.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    image::RgbaImage::from_raw(size, size, rgba).expect("the pixmap's size")
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets").join("icon");
    let svg = std::fs::read(dir.join("coiman.svg")).expect("assets/icon/coiman.svg");
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default()).expect("a valid SVG");
    for size in SIZES {
        let file = dir.join(format!("coiman-{size}.png"));
        render(&tree, size).save(&file).expect("writing a PNG");
        println!("{}", file.display());
    }
    let frames: Vec<image::codecs::ico::IcoFrame> = ICO_SIZES
        .iter()
        .map(|&size| {
            let img = render(&tree, size);
            image::codecs::ico::IcoFrame::as_png(img.as_raw(), size, size, image::ExtendedColorType::Rgba8)
                .expect("an icon frame")
        })
        .collect();
    let ico = dir.join("coiman.ico");
    let out = BufWriter::new(std::fs::File::create(&ico).expect("writing the icon"));
    image::codecs::ico::IcoEncoder::new(out).encode_images(&frames).expect("encoding the icon");
    println!("{}", ico.display());
}
