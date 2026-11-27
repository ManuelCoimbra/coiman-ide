//! For plugin authors: a plugin's documentation written from its `plugin.toml`, and its tests
//! run in a plugin host with no window: `tests.toml` beside `plugin.toml` lists commands to run
//! and what the plugin must show.
//!
//! ```toml
//! [[test]]
//! name = "counts the lines"
//! fixture = "tests/small"        # a folder, copied, to be the open folder (optional)
//! programs = ["git"]             # programs approved for it (optional)
//! document = { path = "a.txt", text = "one\ntwo\n" }   # the file in front (optional)
//!
//! [[test.step]]
//! command = "count"              # runs a command of the plugin (with `argument`)
//! [[test.step]]
//! panel = "2 lines"              # a panel's text has this, within `timeout` seconds (5)
//! [[test.step]]
//! message = "Counted"            # a message (status bar) has this
//! [[test.step]]
//! status = "2 lines"             # a status bar item has this
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::host::{Block, Document, PluginHost, Update};
use super::manifest::Manifest;
use super::registry;

/// A plugin's `plugin.toml` as Markdown: what it does, what it asks for, what it adds.
pub fn docs(m: &Manifest) -> String {
    let mut s = format!("# {}\n\n`{}` {}\n\n", m.name, m.id, m.version);
    if !m.description.is_empty() {
        s.push_str(&format!("{}\n\n", m.description));
    }
    s.push_str("## What it asks for\n\n");
    if m.permissions.is_empty() && m.programs.is_empty() {
        s.push_str("Nothing: it runs entirely inside the sandbox.\n\n");
    }
    for p in &m.permissions {
        s.push_str(&format!("- `{}`: {}\n", p.name(), p.describe()));
    }
    for p in &m.programs {
        s.push_str(&format!("- Program `{p}`, started outside the sandbox when approved\n"));
    }
    if let Some(mb) = m.memory {
        s.push_str(&format!("- Up to {mb} MB of memory\n"));
    }
    if !m.permissions.is_empty() || !m.programs.is_empty() || m.memory.is_some() {
        s.push('\n');
    }
    if !m.commands.is_empty() {
        s.push_str("## Commands\n\n| Command | Id |\n|---|---|\n");
        for c in &m.commands {
            s.push_str(&format!("| {} | `{}` |\n", c.title, c.id));
        }
        s.push('\n');
    }
    let views: Vec<String> = m
        .panels
        .iter()
        .map(|p| format!("- Panel **{}** (`{}`)", p.title, p.id))
        .chain(m.tree_views.iter().map(|t| format!("- Explorer section **{}** (`{}`)", t.title, t.id)))
        .chain(m.status_items.iter().map(|i| {
            let click = i.command.as_ref().map(|c| format!(", runs `{c}` when clicked")).unwrap_or_default();
            format!("- Status bar item `{}`{click}", i.id)
        }))
        .collect();
    if !views.is_empty() {
        s.push_str(&format!("## What it shows\n\n{}\n\n", views.join("\n")));
    }
    if !m.events.is_empty() {
        let names: Vec<String> = m.events.iter().map(|e| format!("`{}`", kebab(&format!("{e:?}")))).collect();
        s.push_str(&format!("## Events it hears\n\n{}\n\n", names.join(", ")));
    }
    if !m.languages.is_empty() || !m.openers.is_empty() {
        s.push_str("## Files\n\n");
        for l in &m.languages {
            s.push_str(&format!("- Language **{}**: `.{}`\n", l.name, l.extensions.join("`, `.")));
        }
        for o in &m.openers {
            s.push_str(&format!("- Opens `.{}` files with `{}`\n", o.extensions.join("`, `."), o.command));
        }
        s.push('\n');
    }
    s
}

fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// One test's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub name: String,
    /// None when it passed; else which step and why.
    pub failure: Option<String>,
    pub millis: u128,
}

/// A panel's blocks as text, a line each, for tests to look into.
pub fn blocks_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) | Block::Code(t) | Block::Diff(t) => out.push_str(&format!("{t}\n")),
            Block::Bullets(items) => out.push_str(&format!("{}\n", items.join("\n"))),
            Block::Button(b) => out.push_str(&format!("[{}]\n", b.label)),
            Block::Spans(spans) => {
                out.push_str(&format!("{}\n", spans.iter().map(|s| s.text.as_str()).collect::<String>()))
            }
            Block::Table(t) => {
                out.push_str(&format!("{}\n", t.headers.join(" | ")));
                for r in &t.rows {
                    out.push_str(&format!("{}\n", r.join(" | ")));
                }
            }
            Block::Links(links) => {
                for l in links {
                    out.push_str(&format!("{} {}\n", l.text, l.detail));
                }
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|c| c.text.as_str()).collect();
                    let buttons: Vec<String> = r.buttons.iter().map(|b| format!("[{}]", b.label)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Form(f) => {
                out.push_str(&format!("{}\n", f.buttons.iter().map(|b| format!("[{b}]")).collect::<String>()))
            }
            Block::Log(l) => {
                for line in &l.lines {
                    out.push_str(&format!("{}\n", line.text));
                }
            }
            _ => {}
        }
    }
    out
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// What the plugin has shown so far.
#[derive(Default)]
struct Seen {
    panels: String,
    messages: String,
    status: String,
    stopped: Option<String>,
}

impl Seen {
    fn take(&mut self, host: &PluginHost) {
        for u in host.poll() {
            match u {
                Update::Panel { blocks, .. } => self.panels = blocks_text(&blocks),
                Update::Message { text, .. } => self.messages.push_str(&format!("{text}\n")),
                Update::Status { text, tooltip, .. } => self.status.push_str(&format!("{text} {tooltip}\n")),
                Update::Stopped { error, .. } => self.stopped = Some(error),
                _ => {}
            }
        }
    }
}

/// Runs the tests in `folder/tests.toml` against the plugin built in `folder`.
pub fn run(folder: &Path) -> Result<Vec<Outcome>, String> {
    let text = std::fs::read_to_string(folder.join("tests.toml")).map_err(|e| format!("tests.toml: {e}"))?;
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| format!("tests.toml: {e}"))?;
    let pkg = registry::read_folder(folder)?;
    let tests = table.get("test").and_then(toml::Value::as_array).cloned().unwrap_or_default();
    if tests.is_empty() {
        return Err("tests.toml has no [[test]]".into());
    }
    let mut out = Vec::new();
    for (i, t) in tests.iter().enumerate() {
        let name = t.get("name").and_then(toml::Value::as_str).map_or(format!("test {}", i + 1), str::to_owned);
        let started = Instant::now();
        let failure = run_one(folder, &pkg.manifest, &pkg.wasm, t).err();
        out.push(Outcome { name, failure, millis: started.elapsed().as_millis() });
    }
    Ok(out)
}

fn run_one(folder: &Path, manifest: &Manifest, wasm: &[u8], t: &toml::Value) -> Result<(), String> {
    let get = |k: &str| t.get(k).and_then(toml::Value::as_str).unwrap_or("");
    // The fixture is copied, so a test that writes leaves it as it was.
    let ws: PathBuf = std::env::temp_dir().join(format!("coiman-plugin-test-{}-{}", std::process::id(), rand_suffix()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
    if !get("fixture").is_empty() {
        copy_dir(&folder.join(get("fixture")), &ws).map_err(|e| format!("fixture {}: {e}", get("fixture")))?;
    }
    let host = PluginHost::start(|| {})?;
    host.set_workspace(Some(ws.clone()));
    if let Some(doc) = t.get("document").and_then(toml::Value::as_table) {
        let path = doc.get("path").and_then(toml::Value::as_str).unwrap_or("").to_owned();
        let text = doc.get("text").and_then(toml::Value::as_str).unwrap_or("").to_owned();
        host.set_document(Some(Document {
            path,
            language: String::new(),
            text,
            line: 1,
            column: 1,
            dirty: false,
            selection: String::new(),
        }));
    }
    let programs: BTreeSet<String> = t
        .get("programs")
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    host.load_with_programs(manifest.clone(), wasm.to_vec(), manifest.permissions.clone(), programs);
    let mut seen = Seen::default();
    let result = steps(&host, &mut seen, manifest, t);
    let _ = std::fs::remove_dir_all(&ws);
    result
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

fn steps(host: &PluginHost, seen: &mut Seen, manifest: &Manifest, t: &toml::Value) -> Result<(), String> {
    for (i, step) in t.get("step").and_then(toml::Value::as_array).into_iter().flatten().enumerate() {
        let n = i + 1;
        let get = |k: &str| step.get(k).and_then(toml::Value::as_str);
        let timeout = Duration::from_secs_f64(step.get("timeout").and_then(toml::Value::as_float).unwrap_or(5.0));
        if let Some(cmd) = get("command") {
            host.run_command(&manifest.id, cmd, get("argument").unwrap_or(""));
            continue;
        }
        let (what, want) = match (get("panel"), get("message"), get("status")) {
            (Some(w), _, _) => ("panel", w),
            (_, Some(w), _) => ("message", w),
            (_, _, Some(w)) => ("status bar", w),
            _ => return Err(format!("step {n}: say `command`, `panel`, `message` or `status`")),
        };
        let start = Instant::now();
        loop {
            seen.take(host);
            if let Some(e) = &seen.stopped {
                return Err(format!("step {n}: the plugin stopped: {e}"));
            }
            let have = match what {
                "panel" => &seen.panels,
                "message" => &seen.messages,
                _ => &seen.status,
            };
            if have.contains(want) {
                break;
            }
            if start.elapsed() > timeout {
                let shown =
                    if have.trim().is_empty() { "nothing".to_owned() } else { format!("\n{}", have.trim_end()) };
                return Err(format!("step {n}: the {what} never had {want:?}; it had {shown}"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_from_a_manifest() {
        let m = super::super::manifest::parse(
            "id = \"me.count\"\nname = \"Count\"\nversion = \"1.2.0\"\ndescription = \"Counts lines.\"\n\
             permissions = [\"editor-read\"]\nprograms = [\"git\"]\nevents = [\"active-changed\"]\n\
             [[commands]]\nid = \"count\"\ntitle = \"Count: Lines\"\n\
             [[panels]]\nid = \"p\"\ntitle = \"Counts\"\n\
             [[status_items]]\nid = \"lines\"\ncommand = \"count\"\n",
        )
        .unwrap();
        let d = docs(&m);
        assert!(d.starts_with("# Count\n\n`me.count` 1.2.0\n\nCounts lines.\n"), "{d}");
        assert!(d.contains("- `editor-read`: ") && d.contains("- Program `git`"), "{d}");
        assert!(d.contains("| Count: Lines | `count` |"), "{d}");
        assert!(d.contains("- Panel **Counts** (`p`)") && d.contains("runs `count` when clicked"), "{d}");
        assert!(d.contains("`active-changed`"), "{d}");
    }
}
