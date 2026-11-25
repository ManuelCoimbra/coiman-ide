//! The Binary Size and Serial Monitor extensions (`plugins/extensions/binary-size`,
//! `plugins/extensions/serial-monitor`, rebuilt with `scripts/build-plugins.ps1`) in the real
//! plugin host: object files rustc makes, read through `read-bytes` a page at a time; and a
//! named pipe standing in for a board on a serial port.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::manifest::Permission;
use coiman_core::extensions::plugin::{Block, Event, FormValues, PluginHost, Update, registry};

thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if let Update::Stopped { error, .. } = &u {
                    panic!("the extension stopped: {error}");
                }
                if want(&u) {
                    return Some(u);
                }
            }
            None
        });
        if let Some(u) = found {
            return u;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

/// The panel's blocks once its text holds `until`.
fn panel(host: &PluginHost, until: &str) -> (String, Vec<Block>) {
    let mut last = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            last = text(&blocks);
            if last.contains(until) {
                return (last, blocks);
            }
        }
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(spans) => {
                out.push_str(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Form(f) => {
                let fields: Vec<String> = f.fields.iter().filter(|f| !f.id.is_empty()).map(|f| f.id.clone()).collect();
                out.push_str(&format!("form {} ({}) {:?}\n", f.id, fields.join(","), f.buttons));
            }
            Block::Log(log) => {
                for line in &log.lines {
                    out.push_str(&format!("log {line}\n"));
                }
            }
            Block::Plot(series) => {
                for s in series {
                    out.push_str(&format!("plot {} {:?}\n", s.name, s.values));
                }
            }
            _ => {}
        }
    }
    out
}

fn submit(host: &PluginHost, id: &str, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(id, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

fn load(host: &PluginHost, folder: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions").join(folder);
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let granted = pkg.manifest.permissions.clone();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, BTreeSet::new());
    wait(host, |u| matches!(u, Update::Activated { .. }));
}

/// An object file rustc makes from `source` (a `sizes` module), at `out`.
fn object_file(source: &str, out: &Path) {
    let src = out.with_extension("rs");
    std::fs::write(&src, source).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let made = std::process::Command::new(rustc)
        .args(["--crate-type", "lib", "--emit", "obj", "-C", "opt-level=1", "-C", "panic=abort", "-o"])
        .arg(out)
        .arg(&src)
        .output()
        .unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    std::fs::remove_file(src).unwrap();
}

const SIZES: &str = "#![no_std]\n\
    pub mod sizes {\n\
      #[no_mangle] pub static TABLE: [u32; 1000] = [7; 1000];\n\
      #[inline(never)] pub fn big(x: &mut [u32]) { for i in 0..x.len() { x[i] = x[i].wrapping_mul(31).rotate_left(i as u32) ^ TABLE[i % 1000]; if x[i] % 3 == 0 { x[i] /= 7 } else if x[i] % 5 == 0 { x[i] = x[i].wrapping_sub(11) } } }\n\
      #[inline(never)] pub fn small() -> u32 { 1 }\n\
    }\n";

#[test]
fn binary_size_extension() {
    let root = std::env::temp_dir().join(format!("coiman-binsize-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let release = root.join("target").join("release");
    std::fs::create_dir_all(release.join("deps")).unwrap();
    object_file(SIZES, &release.join("sizes.o"));
    // Not offered: text, and what Cargo keeps in deps.
    std::fs::write(release.join("notes.txt"), "hello").unwrap();
    std::fs::copy(release.join("sizes.o"), release.join("deps").join("other.o")).unwrap();

    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(root.clone()));
    load(&host, "binary-size");
    host.run_command("coiman.binary-size", "show", "");
    let (shown, blocks) = panel(&host, "object file");
    assert!(shown.contains("Groups"), "{shown}");
    assert!(shown.lines().any(|l| l.starts_with("sizes | ")), "{shown}");
    // The build folder's file is offered, the others aren't.
    let Some(Block::Form(file)) = blocks.first() else { panic!("{shown}") };
    assert_eq!(file.fields[0].options, ["target/release/sizes.o"]);

    host.run_command("coiman.binary-size", "tab", "symbols");
    let (shown, _) = panel(&host, "sizes::big");
    assert!(shown.contains("TABLE") && shown.contains("3.9 KiB"), "{shown}");
    submit(&host, "coiman.binary-size", "filter", "Filter", &[("filter", "small")]);
    let (shown, _) = panel(&host, "sizes::small");
    assert!(!shown.contains("sizes::big"), "{shown}");
    host.run_command("coiman.binary-size", "tab", "sections");
    panel(&host, ".text");

    // Built again without the big function: what changed.
    object_file(&SIZES.replace("#[inline(never)] pub fn big", "#[inline(never)] fn _gone"), &release.join("sizes.o"));
    host.run_command("coiman.binary-size", "tab", "groups");
    panel(&host, "object file");
    submit(&host, "coiman.binary-size", "file", "Analyze again", &[]);
    let (shown, _) = panel(&host, "since last time");
    assert!(shown.lines().any(|l| l.starts_with("sizes | ") && l.contains("| -")), "{shown}");

    // A path typed in, that isn't a program.
    submit(&host, "coiman.binary-size", "file", "Analyze", &[("path", "target/release/notes.txt")]);
    panel(&host, "Not a program or object file");
    // One outside the folder isn't read.
    submit(&host, "coiman.binary-size", "file", "Analyze", &[("path", "../elsewhere.o")]);
    panel(&host, "isn't a path inside the open folder");
    drop(host);
    std::fs::remove_dir_all(&root).unwrap();
}

/// This repository's own release build (hundreds of MB, about 200 000 symbols) read in steps,
/// each well inside a call's time limit: `cargo test --release --test embedded_extensions --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn binary_size_of_a_large_program() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    assert!(root.join("target/release/coiman-ide.exe").exists(), "build it first");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(root));
    load(&host, "binary-size");
    let start = Instant::now();
    submit(&host, "coiman.binary-size", "file", "Analyze", &[("path", "target/release/coiman-ide.exe")]);
    let (shown, _) = panel(&host, "PE program");
    eprintln!("{:?}\n{}", start.elapsed(), shown.lines().take(8).collect::<Vec<_>>().join("\n"));
    assert!(shown.contains("Symbols 1") || shown.contains("Symbols 2"), "{shown}");
}

#[test]
fn serial_monitor_extension() {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};

    // A program on the other end of a named pipe, standing in for a board.
    let name = format!(r"\\.\pipe\coiman-serial-ext-{}", std::process::id());
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a valid name; default security.
    let server = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            std::ptr::null(),
        )
    };
    assert_ne!(server, INVALID_HANDLE_VALUE);
    let server = server as usize;
    let (said_tx, said_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (more_tx, more_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let board = std::thread::spawn(move || {
        let h = server as windows_sys::Win32::Foundation::HANDLE;
        // SAFETY: the pipe made above; buffers live through each call.
        unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
        let write = |data: &[u8]| {
            let mut put = 0u32;
            unsafe { WriteFile(h, data.as_ptr(), data.len() as u32, &mut put, std::ptr::null_mut()) };
        };
        write(b"boot\r\ntemp:21.5 hum:40\r\ntemp:22");
        // What the monitor sends.
        let mut buf = [0u8; 64];
        let mut got = 0u32;
        unsafe { ReadFile(h, buf.as_mut_ptr(), 64, &mut got, std::ptr::null_mut()) };
        let _ = said_tx.send(buf[..got as usize].to_vec());
        // More, when the test says, then the board goes away.
        if let Ok(more) = more_rx.recv() {
            write(&more);
        }
        std::thread::sleep(Duration::from_millis(300));
        unsafe { CloseHandle(h) };
    });

    let host = PluginHost::start(|| {}).unwrap();
    load(&host, "serial-monitor");
    host.run_command("coiman.serial-monitor", "show", "");
    let (shown, _) = panel(&host, "Not connected");
    assert!(shown.contains("form port (port,path,baud,other,format,dtr,rts)"), "{shown}");

    let port = [
        ("port", ""),
        ("path", name.as_str()),
        ("baud", "115200"),
        ("other", ""),
        ("format", "8N1"),
        ("dtr", "false"),
        ("rts", "false"),
    ];
    submit(&host, "coiman.serial-monitor", "port", "Connect", &port);
    let (shown, _) = panel(&host, "log temp:22");
    assert!(shown.contains("log boot\nlog temp:21.5 hum:40\n"), "{shown}");
    assert!(shown.contains("form send (text,ending)"), "{shown}");

    submit(&host, "coiman.serial-monitor", "send", "Send", &[("text", "ping"), ("ending", "Both (CR LF)")]);
    assert_eq!(said_rx.recv_timeout(Duration::from_secs(10)).unwrap(), b"ping\r\n");
    panel(&host, "[Send again again 0]");

    host.run_command("coiman.serial-monitor", "times", "");
    let (shown, _) = panel(&host, "log 00:0");
    assert!(shown.lines().any(|l| l.starts_with("log 00:0") && l.ends_with(" boot")), "{shown}");

    more_tx.send(b".5\r\ntemp:23\r\n".to_vec()).unwrap();
    host.run_command("coiman.serial-monitor", "view", "plot");
    let (shown, _) = panel(&host, "plot temp [21.5, 22.5, 23.0]");
    assert!(shown.contains("plot hum [40.0]"), "{shown}");
    host.run_command("coiman.serial-monitor", "view", "hex");
    let (shown, _) = panel(&host, "log 00000000  62 6F 6F 74 0D 0A");
    assert!(shown.contains("boot..temp:21.5 "), "{shown}");

    // The board hangs up.
    let (shown, _) = panel(&host, "closed: the device went away");
    assert!(shown.contains("Not connected"), "{shown}");
    board.join().unwrap();

    // A port that isn't there.
    let missing = [
        ("path", r"\\.\pipe\coiman-no-such-pipe"),
        ("baud", "115200"),
        ("format", "8N1"),
        ("dtr", "true"),
        ("rts", "true"),
    ];
    submit(&host, "coiman.serial-monitor", "port", "Connect", &missing);
    panel(&host, "isn't there");
    // A bad speed.
    submit(
        &host,
        "coiman.serial-monitor",
        "port",
        "Connect",
        &[("path", "COM99"), ("other", "fast"), ("format", "8N1")],
    );
    panel(&host, "The speed is a number of baud");
}

#[test]
fn serial_needs_its_permission() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/serial-monitor");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    assert!(pkg.manifest.permissions.contains(&Permission::Serial));
    let host = PluginHost::start(|| {}).unwrap();
    host.load_with_programs(pkg.manifest, pkg.wasm, BTreeSet::new(), BTreeSet::new());
    wait(&host, |u| matches!(u, Update::Activated { .. }));
    host.run_command("coiman.serial-monitor", "show", "");
    panel(&host, "permission denied: serial");
}
