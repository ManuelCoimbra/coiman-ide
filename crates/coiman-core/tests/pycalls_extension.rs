//! The Python Calls and Memory extension (`plugins/extensions/pycalls`) in the real plugin host,
//! with the real Python: a recursive function's calls counted, a line holding many objects
//! flagged, the program's own output kept apart, and an exception reported.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Document, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.pycalls";

fn wait_panel(host: &PluginHost, until: &str) -> String {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Panel { blocks, .. } => last = text(&blocks),
                _ => {}
            }
        }
        if last.contains(until) {
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
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
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
            _ => {}
        }
    }
    out
}

fn document(path: &str) -> Document {
    Document {
        path: path.into(),
        language: "python".into(),
        text: String::new(),
        line: 1,
        column: 1,
        dirty: false,
        selection: String::new(),
    }
}

#[test]
fn pycalls_extension() {
    if Command::new("python").arg("--version").output().is_err() {
        eprintln!("python isn't on PATH: skipped");
        return;
    }
    let ws = std::env::temp_dir().join(format!("coiman-pycalls-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join("prog.py"),
        "def fib(n):\n    return n if n < 2 else fib(n - 1) + fib(n - 2)\n\n\
         def sq(x):\n    return x * x\n\n\
         keep = [str(i) for i in range(200000)]\n\
         print('result', fib(15), sum(sq(i) for i in range(100)))\n",
    )
    .unwrap();
    std::fs::write(ws.join("bad.py"), "def f():\n    raise ValueError('nope')\n\nf()\n").unwrap();

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/pycalls");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    host.set_document(Some(document("prog.py")));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["python".to_owned()].into());

    host.run_command(ID, "calls", "");
    let calls = wait_panel(&host, "What the program printed");
    // fib(15): 1973 calls, one from outside.
    assert!(calls.contains("fib | 1973 calls (1 from outside, the rest recursive)"), "{calls}");
    assert!(calls.contains("sq | 100 calls"), "{calls}");
    assert!(calls.contains("[Open prog.py:1]") && calls.contains("[Open prog.py:4]"), "{calls}");
    assert!(calls.contains("not shown") && !calls.contains("_find_and_load"), "only the project's: {calls}");
    assert!(calls.contains("result 610 328350"), "{calls}");

    // With the library's functions too.
    host.deliver(
        ID,
        Event::FormSubmitted(FormValues {
            form: "filter".into(),
            button: "Apply".into(),
            values: vec![("project".into(), "false".into())],
        }),
    );
    let all = wait_panel(&host, "_find_and_load");
    assert!(all.contains("fib | 1973"), "{all}");

    host.run_command(ID, "memory", "");
    let memory = wait_panel(&host, "still held at the end");
    assert!(memory.contains("200001 objects") && memory.contains("prog.py:7"), "{memory}");
    assert!(memory.contains("keep = [str(i) for i in range(200000)]"), "{memory}");
    assert!(memory.contains("1 line holds a lot (in red)"), "{memory}");

    host.deliver(
        ID,
        Event::FormSubmitted(FormValues {
            form: "filter".into(),
            button: "Apply".into(),
            values: vec![("project".into(), "true".into())],
        }),
    );
    host.set_document(Some(document("bad.py")));
    host.run_command(ID, "calls", "");
    let bad = wait_panel(&host, "The program raised ValueError: nope");
    assert!(bad.contains("f | 1 call"), "{bad}");
    let _ = std::fs::remove_dir_all(&ws);
}
