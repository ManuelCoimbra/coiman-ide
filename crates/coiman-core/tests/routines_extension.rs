//! The Terminal Commands extension's routines in the real plugin host: the project's routines
//! offered in a pick list, and one run in a terminal tab through an approved shell, with its
//! `@` steps put in.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Event, Permission, PickAction, PickChoice, PluginHost, Update, registry};

const ID: &str = "coiman.terminal-commands";

fn wait(host: &PluginHost, want: impl Fn(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            if let Update::Stopped { error, .. } = &u {
                panic!("the extension stopped: {error}");
            }
            if want(&u) {
                return u;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

#[test]
fn routines_extension() {
    let root = std::env::temp_dir().join(format!("coiman-routines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join(".coiman")).unwrap();
    // SAFETY: the only test in this binary, set before anything reads the environment.
    unsafe { std::env::set_var("APPDATA", root.join("config")) };

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/terminal-commands");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::Terminal, Permission::WorkspaceRead].into();
    let shells: BTreeSet<String> = ["pwsh", "powershell", "sh"].map(str::to_owned).into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, shells);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // None yet: it says where they go.
    host.run_command(ID, "routine", "");
    match wait(&host, |u| matches!(u, Update::Message { .. })) {
        Update::Message { text, .. } => assert!(text.contains(".coiman/routines.toml"), "{text}"),
        _ => unreachable!(),
    }

    std::fs::write(
        ws.join(".coiman/routines.toml"),
        "[[routine]]\nname = \"check\"\ndescription = \"What CI runs\"\nsteps = [\"echo fmt\", \"@test\"]\n\
         [[routine]]\nname = \"test\"\nsteps = [\"echo test\"]\nshell = \"sh\"\n",
    )
    .unwrap();
    host.run_command(ID, "routine", "");
    let Update::Pick { pick, .. } = wait(&host, |u| matches!(u, Update::Pick { .. })) else { unreachable!() };
    let rows: Vec<(String, String)> = pick.items.iter().map(|i| (i.text.clone(), i.detail.clone())).collect();
    assert_eq!(
        rows,
        [("check".to_owned(), "What CI runs · 2 steps".to_owned()), ("test".to_owned(), "echo test".to_owned())]
    );

    // sh only: its script runs each step and stops at a failure.
    host.deliver(
        ID,
        Event::Picked(PickChoice { pick: "routines".into(), text: "test".into(), action: PickAction::Run }),
    );
    let Update::RunInTerminal { title, program, args, folder, .. } =
        wait(&host, |u| matches!(u, Update::RunInTerminal { .. }) || matches!(u, Update::Message { .. }))
    else {
        panic!("no terminal")
    };
    assert_eq!(title, "Routine: test");
    assert!(program.to_string_lossy().to_lowercase().contains("sh"), "{program:?}");
    assert_eq!(args[0], "-c");
    assert!(args[1].contains("echo test || {"), "{}", args[1]);
    assert_eq!(folder.canonicalize().unwrap(), ws.canonicalize().unwrap());

    // The default: PowerShell where it's here, its script encoded, @test put in.
    host.run_command(ID, "routine", "check");
    let Update::RunInTerminal { program, args, .. } = wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) else {
        unreachable!()
    };
    let name = program.file_stem().unwrap().to_string_lossy().to_lowercase();
    if name == "pwsh" || name == "powershell" {
        assert_eq!(args[..3], ["-NoLogo", "-NoProfile", "-EncodedCommand"]);
    } else {
        assert!(args[1].contains("echo fmt") && args[1].contains("echo test"), "{args:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
