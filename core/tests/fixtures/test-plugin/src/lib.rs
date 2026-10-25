//! Test plugin: `run-command("run", "<op> <arg>")` performs one host call
//! and reports the result through `show-message` as `ok: …` or `err: …`.

wit_bindgen::generate!({
    path: "../../../wit/plugin.wit",
    world: "plugin",
});

use rust_code::plugin::host;
use rust_code::plugin::types::{Block, TreeItem};

struct TestPlugin;

impl Guest for TestPlugin {
    fn activate() -> Result<(), String> {
        host::set_status_item("state", "active", "");
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        if id != "run" {
            return Err(format!("unknown command {id}"));
        }
        let (op, arg) = argument.split_once(' ').unwrap_or((argument.as_str(), ""));
        let result: Result<String, String> = match op {
            "read" => host::read_file(arg),
            "write" => host::write_file(arg, "written by plugin").map(|()| "written".into()),
            "list" => host::list_files(arg, 100).map(|f| f.join(",")),
            "doc" => host::active_document().map(|d| d.map_or("none".into(), |d| format!("{}:{}", d.path, d.text.len()))),
            "insert" => host::insert_text(arg).map(|()| "inserted".into()),
            "panel" => {
                host::set_panel("out", &[Block::Heading(arg.into()), Block::Separator]);
                Ok("panel".into())
            }
            "tree" => {
                let item = TreeItem {
                    id: "a".into(),
                    parent: String::new(),
                    label: arg.into(),
                    description: String::new(),
                    tooltip: String::new(),
                    command: None,
                };
                host::set_tree_items("files", &[item]);
                Ok("tree".into())
            }
            "undeclared" => {
                host::set_panel("nope", &[]);
                Ok("sent".into())
            }
            "spin" => loop {
                std::hint::black_box(0);
            },
            "trap" => panic!("boom"),
            "fail" => return Err("failed on purpose".into()),
            other => Err(format!("unknown op {other}")),
        };
        match result {
            Ok(v) => host::show_message(&format!("ok: {v}")),
            Err(e) => host::show_message(&format!("err: {e}")),
        }
        Ok(())
    }

    fn on_event(event: Event) {
        if let Event::Saved(path) = event {
            host::log(&format!("saved {path}"));
        }
    }
}

export!(TestPlugin);
