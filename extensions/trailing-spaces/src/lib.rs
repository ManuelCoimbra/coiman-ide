//! Trailing Spaces: marks whitespace at the end of lines.

wit_bindgen::generate!({
    path: "../../core/wit/plugin.wit",
    world: "plugin",
});

use rust_code::plugin::host;
use rust_code::plugin::types::Decoration;

const MAX_BYTES: usize = 2_000_000;

fn update() {
    let Ok(Some(doc)) = host::active_document() else { return };
    let mut items = Vec::new();
    if doc.text.len() <= MAX_BYTES {
        for (n, line) in doc.text.lines().enumerate() {
            let number = n as u32 + 1;
            let trimmed = line.trim_end_matches([' ', '\t']);
            // Not lines that are only indentation.
            if trimmed.len() == line.len() || trimmed.is_empty() {
                continue;
            }
            items.push(Decoration {
                line: number,
                start: trimmed.chars().count() as u32,
                end: line.chars().count() as u32,
                foreground: None,
                background: Some(0x8B2E2E),
                italic: false,
            });
        }
    }
    host::set_decorations("", &items);
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        update();
        Ok(())
    }

    fn run_command(_: String, _: String) -> Result<(), String> {
        Ok(())
    }

    fn on_event(_: Event) {
        update();
    }
}

export!(Plugin);
