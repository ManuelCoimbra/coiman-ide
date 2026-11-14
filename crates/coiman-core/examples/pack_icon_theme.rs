//! Packs a VS Code icon theme into Coiman's format, for the packs bundled in
//! `data/icon_packs/`:
//!
//!     cargo run -p coiman-core --example pack_icon_theme -- <label> <out.txt> <theme.json> [light-theme.json]
//!
//! The theme's icons must be SVG files; the extension's package is unpacked first.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [label, out, theme, rest @ ..] = args.as_slice() else {
        eprintln!("usage: pack_icon_theme <label> <out.txt> <theme.json> [light-theme.json]");
        std::process::exit(2);
    };
    let light = rest.first().map(Path::new);
    match coiman_core::appearance::file_icons::pack_from_vscode(Path::new(theme), light, label) {
        Ok(text) => {
            std::fs::write(out, &text).expect("couldn't write the pack");
            let icons = text.lines().filter(|l| l.starts_with("I\t")).count();
            println!("{label}: {icons} icons, {:.2} MB", text.len() as f64 / 1e6);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
