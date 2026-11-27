//! The Agents extension (`plugins/extensions/agents`, rebuilt with `scripts/build-plugins.ps1`)
//! in the real plugin host, with a stand-in `opencode` on PATH: what it is asked, and the chat
//! view it fills.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::host::{ChatAction, ChatMessage, ChatPart, ChatView};
use coiman_core::extensions::plugin::{Event, PluginHost, Update, registry};

const ID: &str = "coiman.agents";

/// The chat view as the editor keeps it: each update applied over the last.
#[derive(Default)]
struct Seen {
    messages: Vec<ChatMessage>,
    sessions: Vec<String>,
    models: Vec<String>,
    last: Option<ChatView>,
}

impl Seen {
    fn apply(&mut self, mut view: ChatView) {
        let from = (view.messages_from as usize).min(self.messages.len());
        self.messages.truncate(from);
        self.messages.append(&mut view.messages);
        if let Some(s) = view.sessions.take() {
            self.sessions = s.into_iter().map(|s| format!("{} {}", s.id, s.title)).collect();
        }
        if let Some(m) = view.models.take() {
            self.models = m;
        }
        self.last = Some(view);
    }

    fn view(&self) -> &ChatView {
        self.last.as_ref().expect("a view")
    }

    /// The transcript as text: `> ` for the user, tools as `[name status: output]`.
    fn transcript(&self) -> String {
        let mut out = String::new();
        for m in &self.messages {
            for p in &m.parts {
                match p {
                    ChatPart::Text(t) if m.from_user => out.push_str(&format!("> {t}\n")),
                    ChatPart::Text(t) => out.push_str(&format!("{t}\n")),
                    ChatPart::Thinking(t) => out.push_str(&format!("(thinking: {t})\n")),
                    ChatPart::Tool(t) => out.push_str(&format!("[{} {}: {}]\n", t.name, t.status, t.output.trim())),
                }
            }
        }
        out
    }
}

/// Applies the chat views as they come until `done` holds.
fn until(host: &PluginHost, seen: &mut Seen, what: &str, mut done: impl FnMut(&Seen) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Chat { view, .. } => seen.apply(view),
                _ => {}
            }
        }
        if seen.last.is_some() && done(seen) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = seen.last.as_ref().map(|v| v.status.clone()).unwrap_or_default();
    panic!("never {what}; status {status:?}; transcript:\n{}", seen.transcript());
}

fn act(host: &PluginHost, action: &str, argument: &str, attachments: &[&str]) {
    let action = ChatAction {
        action: action.to_owned(),
        argument: argument.to_owned(),
        attachments: attachments.iter().map(|a| (*a).to_owned()).collect(),
    };
    host.deliver(ID, Event::ChatAction(action));
}

fn stand_in(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    w(
        "run.jsonl",
        "{\"type\":\"step_start\",\"sessionID\":\"ses_fake\",\"part\":{\"type\":\"step-start\"}}\n\
         {\"type\":\"reasoning\",\"part\":{\"type\":\"reasoning\",\"text\":\"Looking.\"}}\n\
         {\"type\":\"tool_use\",\"part\":{\"id\":\"t1\",\"tool\":\"bash\",\"state\":{\"title\":\"ls\",\"input\":{\"command\":\"ls\"},\"output\":\"a.rs\",\"status\":\"completed\"}}}\n\
         {\"type\":\"session.text.delta\",\"data\":{\"delta\":\"Hello \"}}\n\
         {\"type\":\"session.text.delta\",\"data\":{\"delta\":\"there\"}}\n",
    );
    w(
        "sessions.json",
        "[{\"id\":\"ses_old\",\"title\":\"Old chat\",\"updated\":1},{\"id\":\"ses_fake\",\"title\":\"Fake chat\",\"updated\":9}]\n",
    );
    w(
        "export.json",
        "{\"messages\":[{\"type\":\"user\",\"text\":\"earlier\"},{\"type\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"from before\"}]}]}\n",
    );
    w("models.txt", "a/one\nb/two\n");
    if cfg!(windows) {
        // %1 is --standalone for the calls a newer opencode gets, as this one says it is one.
        w(
            "opencode.cmd",
            "@echo off\r\n>>\"%~dp0calls.log\" echo %* sid=%OPENCODE_SESSION_ID%\r\n\
             if \"%1\"==\"run\" if \"%2\"==\"--help\" echo   --standalone  run without the service & exit /b 0\r\n\
             if \"%1\"==\"models\" type \"%~dp0models.txt\" & exit /b 0\r\n\
             if \"%2\"==\"session\" if \"%3\"==\"list\" type \"%~dp0sessions.json\" & exit /b 0\r\n\
             if \"%2\"==\"session\" if \"%3\"==\"export\" type \"%~dp0export.json\" & exit /b 0\r\n\
             if \"%2\"==\"session\" if \"%3\"==\"delete\" exit /b 0\r\n\
             if \"%2\"==\"run\" (\r\n  findstr \"^\" > \"%~dp0stdin.txt\"\r\n  findstr /c:\"slow\" \"%~dp0stdin.txt\" >nul && ping -n 60 127.0.0.1 >nul\r\n  type \"%~dp0run.jsonl\"\r\n  >&2 echo ! permission requested: edit ^(a.rs^); auto-rejecting\r\n  exit /b 0\r\n)\r\n\
             exit /b 1\r\n",
        );
    } else {
        w(
            "opencode",
            "#!/bin/sh\nd=$(dirname \"$0\")\necho \"$* sid=$OPENCODE_SESSION_ID\" >> \"$d/calls.log\"\n\
             [ \"$1 $2\" = 'run --help' ] && { echo '  --standalone  run without the service'; exit 0; }\n\
             [ \"$1\" = models ] && { cat \"$d/models.txt\"; exit 0; }\n\
             case \"$2 $3\" in 'session list') cat \"$d/sessions.json\"; exit 0;; 'session export') cat \"$d/export.json\"; exit 0;; 'session delete') exit 0;; esac\n\
             if [ \"$2\" = run ]; then cat > \"$d/stdin.txt\"; grep -q slow \"$d/stdin.txt\" && sleep 60; cat \"$d/run.jsonl\"; echo '! permission requested: edit (a.rs); auto-rejecting' >&2; exit 0; fi\n\
             exit 1\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("opencode"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

fn calls(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

#[test]
fn agents_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-agents-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    stand_in(&bin);
    let ws = tmp.join("project");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        // As if Coiman ran inside an opencode session: the programs it starts don't join it.
        std::env::set_var("OPENCODE_SESSION_ID", "ses_outer");
        // The extension's saved choices go to a config folder of the test's own.
        std::env::set_var("APPDATA", tmp.join("config"));
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/agents");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    assert!(pkg.manifest.permissions.is_empty(), "it needs no permission, only its program");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    host.load_with_programs(pkg.manifest, pkg.wasm, BTreeSet::new(), ["opencode".to_owned()].into());
    let mut seen = Seen::default();

    // At start, the saved choices only: nothing runs.
    until(&host, &mut seen, "filled at start", |s| s.view().name == "OpenCode");
    assert_eq!((seen.view().mode.as_str(), seen.view().permissions.as_str()), ("build", "manual"));
    assert!(calls(&bin).is_empty(), "nothing runs at start: {}", calls(&bin));

    // The lists, when the panel asks.
    act(&host, "sessions", "", &[]);
    act(&host, "models", "", &[]);
    until(&host, &mut seen, "listed", |s| s.sessions.len() == 2 && s.models.len() == 2);
    assert_eq!(seen.sessions, ["ses_fake Fake chat", "ses_old Old chat"], "newest first");
    assert_eq!(seen.models, ["a/one", "b/two"]);

    // A prompt: its words on stdin as typed, the attachment as --file, the reply as it comes.
    act(&host, "send", "look at\nthis", &["src/a.rs"]);
    until(&host, &mut seen, "answered", |s| !s.view().running && s.view().status == "Ready");
    let transcript = seen.transcript();
    // stdout and stderr are read apart, so where opencode's note (stderr) lands among the
    // reply's lines depends on timing: each is checked, not their order.
    let note = "Not allowed in Manual: edit (a.rs). Switch to Auto to let the agent do this.";
    assert!(transcript.contains(note), "{transcript}");
    // Without the note and blank lines, wherever they landed.
    let mut reply: String = transcript
        .lines()
        .filter(|l| !l.trim().is_empty() && l.trim() != note)
        .map(|l| format!("{}\n", l.replace(note, "")))
        .collect();
    if !transcript.ends_with('\n') {
        reply.pop();
    }
    assert!(
        reply.starts_with("> @src/a.rs\nlook at\nthis\n(thinking: Looking.)\n[bash completed: a.rs]\n"),
        "{transcript}"
    );
    assert!(reply.contains("Hello there") || reply.contains("Hello \nthere"), "{transcript}");
    let stdin = std::fs::read_to_string(bin.join("stdin.txt")).unwrap();
    assert_eq!(stdin.replace("\r\n", "\n").trim_end(), "look at\nthis");
    let log = calls(&bin);
    let run = log.lines().find(|l| l.contains(" run --thinking")).unwrap_or_default();
    assert!(run.starts_with("--standalone run --thinking --format json --session ses_coiman_"), "{log}");
    assert!(run.contains("--agent build --file src/a.rs") && !run.contains("--auto"), "{run}");
    assert!(log.lines().all(|l| l.ends_with("sid=")), "the outer session isn't passed on: {log}");
    // The session opencode named, and its title from the list that comes after.
    until(&host, &mut seen, "titled", |s| s.view().session == "ses_fake" && s.view().title == "Fake chat");
    assert!(seen.view().turn_seconds.is_some());

    // Plan and Auto go with the next prompt, in the same session; they're kept.
    act(&host, "mode", "plan", &[]);
    act(&host, "permissions", "auto", &[]);
    act(&host, "model", "b/two", &[]);
    act(&host, "send", "again", &[]);
    until(&host, &mut seen, "answered again", |s| !s.view().running && s.messages.len() == 4);
    let log = calls(&bin);
    let run = log.lines().filter(|l| l.contains(" run --thinking")).nth(1).unwrap_or_default();
    assert!(run.contains("--session ses_fake --auto --agent plan --model b/two"), "{run}");
    let saved = std::fs::read_to_string(coiman_core::extensions::plugin::host::storage_file(ID).unwrap()).unwrap();
    assert!(saved.contains("\"plan\"") && saved.contains("\"auto\"") && saved.contains("b/two"), "{saved}");

    // Stop ends a run that takes long.
    act(&host, "send", "slow one", &[]);
    until(&host, &mut seen, "running", |s| s.view().running);
    act(&host, "stop", "", &[]);
    until(&host, &mut seen, "stopped", |s| !s.view().running && s.view().status == "Stopped");

    // A past session opens from its export; deleting the open one starts afresh.
    act(&host, "open", "ses_old", &[]);
    until(&host, &mut seen, "resumed", |s| s.view().status == "Resumed session");
    assert_eq!(seen.transcript(), "> earlier\nfrom before\n");
    assert_eq!((seen.view().session.as_str(), seen.view().title.as_str()), ("ses_old", "Old chat"));
    act(&host, "delete", "ses_old", &[]);
    until(&host, &mut seen, "deleted", |s| s.view().status == "Session deleted");
    assert!(seen.messages.is_empty() && seen.view().session.is_empty());
    assert!(calls(&bin).contains("--standalone session delete ses_old"));

    // Continue opens the newest.
    act(&host, "continue", "", &[]);
    until(&host, &mut seen, "continued", |s| s.view().session == "ses_fake" && s.view().status == "Resumed session");

    let _ = std::fs::remove_dir_all(&tmp);
}
