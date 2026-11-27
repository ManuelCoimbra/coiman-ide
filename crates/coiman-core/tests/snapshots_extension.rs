//! The Project Snapshots extension (`plugins/extensions/snapshots`) in the real plugin host, with
//! the real `git`: take a snapshot, change the folder, compare, restore a file, restore it all,
//! compare with another folder, delete. The project's own Git repository is never touched.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.snapshots";

fn wait_panel(host: &PluginHost, until: &str) -> String {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(120) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Panel { blocks, .. } => last = text(&blocks),
                _ => {}
            }
        }
        if last.contains(until) && !last.contains("Working…") {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) | Block::Diff(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(s) => {
                out.push_str(&s.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|c| c.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {}]", b.label, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Form(f) => {
                for x in &f.fields {
                    out.push_str(&format!("<{}: {} of {:?}>\n", x.id, x.value, x.options));
                }
            }
            _ => {}
        }
    }
    out
}

fn submit(host: &PluginHost, form: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: String::new(), values }));
}

/// The tag of the snapshot named `name`, from its row's Restore button.
fn tag_of(panel: &str, name: &str) -> String {
    let row = panel.lines().find(|l| l.starts_with(&format!("{name} |"))).expect("its row");
    let at = row.find("[Restore… ").expect("a restore button") + "[Restore… ".len();
    row[at..].split(']').next().unwrap().to_owned()
}

#[test]
fn snapshots_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-snapshots-ext-{}", std::process::id()));
    let other = std::env::temp_dir().join(format!("coiman-snapshots-other-{}", std::process::id()));
    for d in [&ws, &other] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("target")).unwrap();
    }
    let w = |p: &str, t: &str| std::fs::write(ws.join(p), t).unwrap();
    w("a.txt", "one\ntwo\n");
    w("src/main.rs", "fn main() {}\n");
    w(".gitignore", "secret.txt\n");
    w("secret.txt", "not kept\n");
    w("target/big.bin", "build output\n");

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/snapshots");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::WorkspaceWrite].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());

    host.run_command(ID, "show", "");
    let empty = wait_panel(&host, "Snapshots (0)");
    assert!(empty.contains("None yet"), "{empty}");

    submit(&host, "take", &[("name", "Before the change")]);
    let one = wait_panel(&host, "Snapshots (1)");
    let first = tag_of(&one, "Before the change");

    // Change the folder: edit, remove, add, and something ignored.
    w("a.txt", "one\nTWO\nthree\n");
    std::fs::remove_file(ws.join("src/main.rs")).unwrap();
    w("new.txt", "new\n");
    w("secret.txt", "changed, still not kept\n");
    w("target/big.bin", "other output\n");

    host.run_command(ID, "compare", &first);
    let cmp = wait_panel(&host, "3 files changed");
    assert!(cmp.contains("changed | a.txt | +2 −1"), "{cmp}");
    assert!(cmp.contains("added | new.txt | +1 −0"), "{cmp}");
    assert!(cmp.contains("removed | src/main.rs | +0 −1"), "{cmp}");
    assert!(!cmp.contains("secret.txt") && !cmp.contains("big.bin"), "ignored files aren't kept: {cmp}");
    assert!(cmp.contains("[Restore file src/main.rs]") && !cmp.contains("[Restore file new.txt]"), "{cmp}");

    host.run_command(ID, "diff", "a.txt");
    let diff = wait_panel(&host, "+TWO");
    assert!(diff.contains("-two") && diff.contains("+three"), "{diff}");

    // One file back.
    host.run_command(ID, "restore-file", "src/main.rs");
    let after = wait_panel(&host, "2 files changed");
    assert_eq!(std::fs::read_to_string(ws.join("src/main.rs")).unwrap(), "fn main() {}\n");
    assert!(!after.contains("src/main.rs |"), "{after}");

    // All of it back, after keeping what there is now.
    host.run_command(ID, "restore", &first);
    let ask = wait_panel(&host, "Restore Before the change");
    assert!(ask.contains("files made since stay"), "{ask}");
    host.run_command(ID, "restore-yes", &first);
    let restored = wait_panel(&host, "Snapshots (2)");
    assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "one\ntwo\n");
    assert!(ws.join("new.txt").exists(), "files made since stay");
    assert_eq!(std::fs::read_to_string(ws.join("secret.txt")).unwrap(), "changed, still not kept\n");
    let before = tag_of(&restored, "Before restoring Before the change");

    // The two snapshots, compared: the one taken before restoring has the edit.
    submit(&host, "compare", &[("left", &label_of(&restored, &first)), ("right", &label_of(&restored, &before))]);
    let two = wait_panel(&host, "2 files changed");
    assert!(two.contains("changed | a.txt | +2 −1") && two.contains("added | new.txt"), "{two}");
    assert!(!two.contains("[Restore file"), "only a snapshot against now restores a file: {two}");

    // Another folder.
    std::fs::write(other.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(other.join("src/main.rs"), "fn main() { todo!() }\n").unwrap();
    std::fs::write(other.join("target/x"), "never read\n").unwrap();
    submit(&host, "folder", &[("path", &other.to_string_lossy())]);
    let folder = wait_panel(&host, &format!("now → {}", other.to_string_lossy()));
    assert!(folder.contains("changed | src/main.rs | +1 −1"), "{folder}");
    assert!(folder.contains("removed | new.txt"), "{folder}");
    assert!(folder.contains("removed | .gitignore") && !folder.contains("| target/"), "{folder}");

    // Delete one, after asking.
    host.run_command(ID, "delete", &before);
    wait_panel(&host, "It can't be brought back");
    host.run_command(ID, "delete-yes", &before);
    let left = wait_panel(&host, "Snapshots (1)");
    assert!(left.contains("Before the change |"), "{left}");

    // Nothing was made in the project itself.
    assert!(!ws.join(".git").exists());
    let data = coiman_core::settings::config_dir().unwrap().join("plugins/data").join(ID);
    let sub = format!("{:016x}", fnv(&ws.to_string_lossy()));
    assert!(data.join(&sub).join("HEAD").exists(), "the snapshots' own repository");
    let _ = std::fs::remove_dir_all(data.join(sub));
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&other);
}

/// A snapshot's choice in the compare form: "name · date".
fn label_of(panel: &str, tag: &str) -> String {
    let row = panel.lines().find(|l| l.contains(&format!("[Restore… {tag}]"))).expect("its row");
    let mut cells = row.split(" | ");
    let name = cells.next().unwrap();
    let date = cells.next().unwrap().split(" [").next().unwrap();
    format!("{name} · {date}")
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}
