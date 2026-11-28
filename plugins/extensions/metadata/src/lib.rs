//! Project Metadata: the open folder's name, version, description, licence and repository, as
//! its `Cargo.toml`, `package.json` and `pyproject.toml` say them, edited in a form and written
//! back in place (comments and the rest of the file kept).

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod edit;

use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, FieldKind, Form, FormField, FormValues, Span};
use edit::{FIELDS, FILES};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x3FB950;
const DIM: u32 = 0x8B949E;

/// A note under a file's form: (file, text, is it an error).
static NOTE: Mutex<Option<(String, String, bool)>> = Mutex::new(None);
/// Bumped after a save, so the forms show what the file now says.
static SAVES: Mutex<u32> = Mutex::new(0);

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn render() {
    let note = NOTE.lock().map(|n| n.clone()).unwrap_or(None);
    let saves = SAVES.lock().map(|n| *n).unwrap_or(0);
    let mut b = Vec::new();
    let mut any = false;
    for (file, kind) in FILES {
        let Ok(text) = host::read_file(file) else { continue };
        any = true;
        b.push(Block::Heading(file.to_owned()));
        let mut fields = Vec::new();
        let mut fixed = Vec::new();
        for (key, label) in FIELDS {
            match edit::get(&text, kind, key) {
                Some(v) => {
                    fields.push(FormField {
                        id: key.into(),
                        label: label.into(),
                        kind: FieldKind::Text,
                        value: v,
                        options: vec![],
                        hint: String::new(),
                    });
                    fields.push(FormField {
                        id: String::new(),
                        label: String::new(),
                        kind: FieldKind::LineBreak,
                        value: String::new(),
                        options: vec![],
                        hint: String::new(),
                    });
                }
                None => fixed.push(label.to_lowercase()),
            }
        }
        b.push(Block::Form(Form { id: format!("{file}#{saves}"), fields, buttons: vec!["Save".into()] }));
        if !fixed.is_empty() {
            b.push(Block::Spans(vec![span(
                format!("Not shown: {} (not a plain text value here, e.g. from the workspace)", fixed.join(", ")),
                Some(DIM),
            )]));
        }
        if let Some((f, text, error)) = &note
            && f == file
        {
            b.push(Block::Spans(vec![span(text, Some(if *error { RED } else { GREEN }))]));
        }
        b.push(Block::Button(Button { label: "Open file".into(), command: "open".into(), argument: file.into() }));
    }
    if !any {
        b.push(Block::Text("No Cargo.toml, package.json or pyproject.toml at the top of the open folder.".into()));
    }
    host::set_panel("metadata", &b);
}

fn save(f: FormValues) -> Result<String, String> {
    let file = f.form.split('#').next().unwrap_or("").to_owned();
    let (_, kind) = FILES.iter().find(|(n, _)| *n == file).ok_or("unknown file")?;
    let mut text = host::read_file(&file)?;
    let before = text.clone();
    for (key, value) in &f.values {
        let value = value.trim();
        if edit::get(&text, *kind, key).as_deref() == Some(value) {
            continue;
        }
        if let Some(why) = edit::check(&file, key, value) {
            return Err(why);
        }
        text = edit::set(&text, *kind, key, value)?;
    }
    if text == before {
        return Ok("Nothing changed".into());
    }
    host::write_file(&file, &text)?;
    if let Ok(mut n) = SAVES.lock() {
        *n += 1;
    }
    Ok(format!("Saved {file}"))
}

fn note(file: &str, text: String, error: bool) {
    if let Ok(mut n) = NOTE.lock() {
        *n = Some((file.to_owned(), text, error));
    }
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        match id.as_str() {
            "show" => render(),
            "open" => host::open_location(&argument, 1)?,
            _ => return Err(format!("unknown command {id}")),
        }
        Ok(())
    }

    fn on_event(event: Event) {
        match event {
            Event::FormSubmitted(f) => {
                let file = f.form.split('#').next().unwrap_or("").to_owned();
                match save(f) {
                    Ok(t) => note(&file, t, false),
                    Err(e) => note(&file, e, true),
                }
                render();
            }
            Event::Saved(path) if FILES.iter().any(|(f, _)| *f == path) => render(),
            _ => {}
        }
    }
}

export!(Plugin);
