//! The Android extension (`plugins/extensions/android`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host, against a stand-in `adb` that answers
//! as the real one does: devices, logcat with a crash whose stack frame is this project's file,
//! `pidof`, and a shell and an install in a terminal.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, PluginHost, Update, registry};

thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
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
    while start.elapsed() < Duration::from_secs(60) {
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
                let fields: Vec<String> =
                    f.fields.iter().map(|f| format!("{}={}{:?}", f.id, f.value, f.options)).collect();
                out.push_str(&format!("form {} ({}) {:?}\n", f.id, fields.join(","), f.buttons));
            }
            Block::Log(log) => {
                for l in &log.lines {
                    out.push_str(&format!("log {}\n", l.text));
                }
            }
            _ => {}
        }
    }
    out
}

fn submit(host: &PluginHost, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    let event = Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values });
    host.deliver(ANDROID, event);
}

const ANDROID: &str = "coiman.android";

/// A stand-in `adb`: it logs its arguments and answers each kind of call.
const ADB: &str = "@echo off\r\n\
>>\"%~dp0adb.log\" echo %*\r\n\
echo %* | findstr /c:\"devices\" >nul && goto devices\r\n\
echo %* | findstr /c:\"logcat\" >nul && goto logcat\r\n\
echo %* | findstr /c:\"pidof\" >nul && goto pidof\r\n\
echo error: unknown command 1>&2\r\n\
exit /b 1\r\n\
:devices\r\n\
echo List of devices attached\r\n\
echo emulator-5554          device product:sdk_gphone64 model:sdk_gphone64_x86_64 transport_id:2\r\n\
echo db425be55d15           unauthorized transport_id:1\r\n\
exit /b 0\r\n\
:logcat\r\n\
echo --------- beginning of main\r\n\
echo 11-20 16:30:00.100   900   900 D Zygote: forked\r\n\
echo 11-20 16:30:01.000  1234  1234 I ActivityManager: Start proc 1234 com.example.app\r\n\
echo 11-20 16:30:01.123  1234  1250 E AndroidRuntime: FATAL EXCEPTION: main\r\n\
echo 11-20 16:30:01.124  1234  1250 E AndroidRuntime: at com.example.app.MainActivity.onCreate(MainActivity.kt:42)\r\n\
echo 11-20 16:30:01.125  1234  1250 E AndroidRuntime: at android.app.Activity.performCreate(Activity.java:8000)\r\n\
echo 11-20 16:30:02.000  5555  5555 W OtherApp: something else\r\n\
ping -n 60 127.0.0.1 >nul\r\n\
exit /b 0\r\n\
:pidof\r\n\
echo 1234\r\n\
exit /b 0\r\n";

#[test]
fn android_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-android-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let (bin, root) = (tmp.join("bin"), tmp.join("project"));
    let src = root.join("app/src/main/java/com/example/app");
    let apk = root.join("app/build/outputs/apk/debug");
    for d in [&bin, &src, &apk] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(src.join("MainActivity.kt"), "package com.example.app\n").unwrap();
    std::fs::write(apk.join("app-debug.apk"), "PK").unwrap();
    std::fs::write(bin.join("adb.cmd"), ADB).unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::set_var("APPDATA", tmp.join("appdata"));
        std::env::remove_var("ANDROID_HOME");
        std::env::remove_var("ANDROID_SDK_ROOT");
    }
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(root.clone()));
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/android");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let granted = pkg.manifest.permissions.clone();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, BTreeSet::from(["adb".to_owned()]));
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // The devices: one ready, one waiting for the user's OK.
    host.run_command(ANDROID, "show", "");
    let (shown, _) = panel(&host, "sdk gphone64 x86 64");
    assert!(shown.contains("[Logcat logcat emulator-5554] [Shell shell emulator-5554]"), "{shown}");
    assert!(shown.contains("db425be55d15 | db425be55d15  unauthorized | Accept the"), "{shown}");
    assert!(shown.contains("apk=app/build/outputs/apk/debug/app-debug.apk"), "{shown}");

    // Logcat: Info and up, the crash's frame a link to this project's file.
    host.run_command(ANDROID, "logcat", "emulator-5554");
    let (shown, blocks) = panel(&host, "OtherApp: something else");
    assert!(!shown.contains("Zygote"), "Debug is below Info: {shown}");
    assert!(shown.contains("log 16:30:01.123  1234 E AndroidRuntime: FATAL EXCEPTION: main"), "{shown}");
    assert!(shown.contains("[Stop logcat stop ]"), "{shown}");
    let Some(Block::Log(log)) = blocks.iter().find(|b| matches!(b, Block::Log(_))) else { panic!("{shown}") };
    let frame = log.lines.iter().find(|l| l.text.contains("MainActivity.onCreate")).unwrap();
    assert_eq!((frame.path.as_str(), frame.line), ("app/src/main/java/com/example/app/MainActivity.kt", 42));
    assert_eq!(frame.color, Some(0xE5534B));
    let android = log.lines.iter().find(|l| l.text.contains("performCreate")).unwrap();
    assert_eq!(android.path, "", "not this project's");

    // Errors only, then only the app's processes.
    submit(&host, "filter", "Filter", &[("level", "Error"), ("text", ""), ("package", "")]);
    let (shown, _) = panel(&host, "3 of 6 lines");
    assert!(!shown.contains("OtherApp"), "{shown}");
    submit(&host, "filter", "Filter", &[("level", "Verbose"), ("text", ""), ("package", "com.example.app")]);
    let (shown, _) = panel(&host, "4 of 6 lines");
    assert!(!shown.contains("OtherApp") && !shown.contains("Zygote"), "{shown}");

    // A shell and an install go to terminals, in the open folder.
    host.run_command(ANDROID, "shell", "emulator-5554");
    let Update::RunInTerminal { args, folder, .. } = wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) else {
        unreachable!()
    };
    assert_eq!(args, ["-s", "emulator-5554", "shell"]);
    assert_eq!(folder, root);
    submit(
        &host,
        "install",
        "Install APK",
        &[("device", "emulator-5554"), ("apk", "app/build/outputs/apk/debug/app-debug.apk"), ("path", "")],
    );
    let Update::RunInTerminal { args, title, .. } = wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) else {
        unreachable!()
    };
    assert_eq!(title, "install app-debug.apk");
    assert_eq!(args, ["-s", "emulator-5554", "install", "-r", "app/build/outputs/apk/debug/app-debug.apk"]);

    // Stopping logcat ends its adb.
    host.run_command(ANDROID, "stop", "");
    let (shown, _) = panel(&host, "[Logcat logcat emulator-5554]");
    assert!(!shown.contains("form filter"), "{shown}");
    let log = std::fs::read_to_string(bin.join("adb.log")).unwrap();
    assert!(log.contains("-s emulator-5554 logcat -v threadtime -T 2000"), "{log}");
    assert!(log.contains("-s emulator-5554 shell pidof com.example.app"), "{log}");
    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
