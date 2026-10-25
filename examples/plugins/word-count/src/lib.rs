//! Word Count: an example Rust Code plugin.
//!
//! - A status bar item with the active editor's word count (`editor-read`).
//! - "Word Count: Show Report" fills a panel with counts and frequent words.
//! - A "TODOs" tree view lists TODO/FIXME comments of the folder's files
//!   (`workspace-read`); clicking one opens it.

wit_bindgen::generate!({
    path: "../../../core/wit/plugin.wit",
    world: "plugin",
});

use std::collections::HashMap;

use rust_code::plugin::host;
use rust_code::plugin::types::{Block, Button, TreeItem};

struct WordCount;

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '\'').filter(|w| !w.is_empty())
}

fn update_status() {
    match host::active_document() {
        Ok(Some(doc)) => {
            let n = words(&doc.text).count();
            host::set_status_item("count", &format!("{n} words"), "Word Count: Show Report");
        }
        Ok(None) => host::set_status_item("count", "", ""),
        Err(e) => host::set_status_item("count", "words: ?", &e),
    }
}

fn report() -> Result<(), String> {
    let doc = host::active_document()?.ok_or("Open a file to count its words")?;
    let mut freq: HashMap<String, usize> = HashMap::new();
    for w in words(&doc.text).filter(|w| w.chars().count() > 3) {
        *freq.entry(w.to_lowercase()).or_default() += 1;
    }
    let mut top: Vec<(String, usize)> = freq.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let name = if doc.path.is_empty() { "Untitled" } else { doc.path.as_str() };
    let blocks = vec![
        Block::Heading(name.to_owned()),
        Block::Text(format!(
            "{} words, {} lines, {} characters",
            words(&doc.text).count(),
            doc.text.lines().count(),
            doc.text.chars().count()
        )),
        Block::Separator,
        Block::Text("Most frequent words (longer than 3 letters):".to_owned()),
        Block::Bullets(top.iter().take(10).map(|(w, n)| format!("{w}: {n}")).collect()),
        Block::Button(Button { label: "Refresh".to_owned(), command: "report".to_owned(), argument: String::new() }),
    ];
    host::set_panel("report", &blocks);
    Ok(())
}

fn refresh_todos() -> Result<(), String> {
    let mut items = Vec::new();
    let files = host::list_files("", 2000)?;
    let text_like = [".rs", ".toml", ".md", ".txt", ".js", ".ts", ".py", ".c", ".h", ".go"];
    for path in files.iter().filter(|p| text_like.iter().any(|s| p.ends_with(s))) {
        let Ok(text) = host::read_file(path) else { continue };
        let mut found = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if let Some(pos) = line.find("TODO").or_else(|| line.find("FIXME")) {
                found.push((i + 1, line[pos..].trim().to_owned()));
            }
        }
        if found.is_empty() {
            continue;
        }
        items.push(TreeItem {
            id: path.clone(),
            parent: String::new(),
            label: path.clone(),
            description: found.len().to_string(),
            tooltip: String::new(),
            command: None,
        });
        for (line, text) in found {
            items.push(TreeItem {
                id: format!("{path}:{line}"),
                parent: path.clone(),
                label: text,
                description: format!("line {line}"),
                tooltip: format!("{path}:{line}"),
                command: Some("open".to_owned()),
            });
        }
    }
    host::set_tree_items("todos", &items);
    Ok(())
}

impl Guest for WordCount {
    fn activate() -> Result<(), String> {
        update_status();
        if let Err(e) = refresh_todos() {
            host::log(&format!("TODOs: {e}"));
        }
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        match id.as_str() {
            "report" => report(),
            "refresh-todos" => refresh_todos(),
            "open" => {
                let (path, line) = argument.rsplit_once(':').ok_or("bad location")?;
                host::open_location(path, line.parse().unwrap_or(1))
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    fn on_event(event: Event) {
        update_status();
        if matches!(event, Event::Saved(_)) {
            let _ = refresh_todos();
        }
    }
}

export!(WordCount);
