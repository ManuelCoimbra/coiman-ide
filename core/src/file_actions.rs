//! The buttons a kind of file gets beside the tabs: what can be done with it that's easy to
//! miss in the command palette.

use crate::commands::Command;
use crate::file_icons::is_test_file;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileAction {
    pub label: &'static str,
    pub tip: &'static str,
    pub command: Command,
}

const fn action(label: &'static str, tip: &'static str, command: Command) -> FileAction {
    FileAction { label, tip, command }
}

/// What a file at `relative` in the project gets, most useful first.
pub fn for_file(relative: &str) -> Vec<FileAction> {
    let name = relative.rsplit(['/', '\\']).next().unwrap_or(relative).to_lowercase();
    let extension = name.rsplit_once('.').map_or("", |n| n.1);
    let mut actions = Vec::new();
    match extension {
        "md" | "markdown" => {
            actions.push(action("Preview", "Show it rendered beside the text", Command::MarkdownPreview));
        }
        "csv" | "tsv" => actions.push(action("Table", "Show it as a table", Command::ToggleTableView)),
        "svg" => actions.push(action("Preview", "Show the picture beside the text", Command::ToggleSvgPreview)),
        "html" | "htm" => {
            actions.push(action(
                "Live Preview",
                "Open it in the browser, updating as you type",
                Command::OpenLivePreview,
            ));
        }
        "json" | "jsonc" | "yaml" | "yml" | "toml" => {
            actions.push(action("Tree", "Show its structure beside the text", Command::ToggleDataTree));
            actions.push(action("Format", "Tidy its layout", Command::FormatDocument));
        }
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => {
            actions.push(action("Header/Source", "Go to its other half", Command::SwitchHeaderSource));
            actions.push(action("Flags", "How it's compiled", Command::ShowCompileInfo));
        }
        _ => {}
    }
    if name == "package.json" {
        actions.insert(0, action("Scripts", "Choose one of its scripts to run", Command::RunProgram));
    }
    if extension == "log" || name.contains(".log.") {
        actions.push(action("Log View", "Filter and color it, however large", Command::OpenLogView));
    }
    if is_test_file(relative) {
        actions.push(action("Run Tests", "Run the tests in this file", Command::TestCurrentFile));
        actions.push(action("Coverage", "Show which lines tests reached", Command::ToggleCoverage));
    }
    if crate::repl::for_extension(extension).is_some() {
        actions.push(action("REPL", "Send the line, selection or cell to a live session", Command::SendToRepl));
    }
    actions.push(action("Hex", "Show its bytes", Command::OpenHexView));
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(file: &str) -> Vec<&'static str> {
        for_file(file).iter().map(|a| a.label).collect()
    }

    #[test]
    fn each_kind_of_file_gets_its_own() {
        assert_eq!(labels("notes.md"), ["Preview", "Hex"]);
        assert_eq!(labels("data.CSV"), ["Table", "Hex"]);
        assert_eq!(labels("data.tsv"), ["Table", "Hex"]);
        assert_eq!(labels("a.svg"), ["Preview", "Hex"]);
        assert_eq!(labels("index.html"), ["Live Preview", "Hex"]);
        assert_eq!(labels("a.yaml"), ["Tree", "Format", "Hex"]);
        assert_eq!(labels("Cargo.toml"), ["Tree", "Format", "Hex"]);
        assert_eq!(labels("package.json"), ["Scripts", "Tree", "Format", "Hex"]);
        assert_eq!(labels("a.c"), ["Header/Source", "Flags", "Hex"]);
        assert_eq!(labels("app.log"), ["Log View", "Hex"]);
        assert_eq!(labels("app.log.1"), ["Log View", "Hex"]);
        assert_eq!(labels("noextension"), ["Hex"]);
    }

    #[test]
    fn tests_get_run_and_coverage() {
        for file in ["a.test.ts", "src\\test\\Helper.java"] {
            let found = labels(file);
            assert!(found.contains(&"Run Tests") && found.contains(&"Coverage"), "{file}: {found:?}");
        }
        assert!(!labels("a.ts").contains(&"Run Tests"));
    }

    #[test]
    fn languages_with_a_session_offer_it() {
        assert!(labels("a.py").contains(&"REPL"));
        assert!(!labels("a.css").contains(&"REPL"));
    }
}
