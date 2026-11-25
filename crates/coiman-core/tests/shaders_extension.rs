//! The Shaders extension (`plugins/extensions/shaders`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host. The editor's OpenGL isn't here, so
//! the test answers its `compile-shader` and link requests the way a driver would.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::host::{ShaderCompiled, ShaderStage, ShaderView};
use coiman_core::extensions::plugin::{Block, Document, Event, PluginHost, Problem, Update, registry};

const SHADERS: &str = "coiman.shaders";

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

fn problems(host: &PluginHost, file: &str) -> Vec<Problem> {
    match wait(host, |u| matches!(u, Update::Problems { path, .. } if path == file)) {
        Update::Problems { items, .. } => items,
        _ => unreachable!(),
    }
}

fn compile(host: &PluginHost) -> (u64, ShaderStage, String) {
    match wait(host, |u| matches!(u, Update::CompileShader { .. })) {
        Update::CompileShader { id, stage, source, .. } => (id, stage, source),
        _ => unreachable!(),
    }
}

/// The panel's blocks: the shader drawn, if any, and its text.
fn panel(host: &PluginHost, until: impl Fn(&Option<ShaderView>, &str) -> bool) -> (Option<ShaderView>, String) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            let view = blocks.iter().find_map(|b| if let Block::Shader(v) = b { Some(v.clone()) } else { None });
            let mut text = String::new();
            for b in &blocks {
                match b {
                    Block::Spans(spans) => {
                        text.push_str(&format!("{}\n", spans.iter().map(|s| s.text.as_str()).collect::<String>()))
                    }
                    Block::Rows(rows) => {
                        for r in rows {
                            let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                            let buttons: Vec<&str> = r.buttons.iter().map(|b| b.label.as_str()).collect();
                            text.push_str(&format!("{} {buttons:?}\n", cells.join(" | ")));
                        }
                    }
                    _ => {}
                }
            }
            if until(&view, &text) {
                return (view, text);
            }
        }
    }
    panic!("the panel never got there");
}

fn document(path: &str, text: &str) -> Document {
    Document {
        path: path.into(),
        language: String::new(),
        text: text.into(),
        line: 1,
        column: 1,
        dirty: false,
        selection: String::new(),
    }
}

const TOY: &str =
    "void mainImage(out vec4 c, in vec2 p) {\n  c = vec4(p / iResolution.xy, 0.5 + 0.5 * sin(iTime), 1.0);\n}\n";
const WGSL: &str = "struct Coiman { time: f32, resolution: vec2f, mouse: vec4f }\n\
@group(0) @binding(0) var<uniform> coiman: Coiman;\n\
@fragment\nfn main(@builtin(position) p: vec4f) -> @location(0) vec4f {\n  return vec4f(p.xy / coiman.resolution, sin(coiman.time), 1.0);\n}\n";

#[test]
fn shaders_extension() {
    let root = std::env::temp_dir().join(format!("coiman-shaders-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("bad.wgsl"), "fn f() -> i32 {\n  return 1.5;\n}\n").unwrap();
    std::fs::write(root.join("toy.frag"), TOY).unwrap();
    std::fs::write(root.join("good.wgsl"), WGSL).unwrap();
    std::fs::write(root.join("pass.vert"), "void main() { gl_Position = vec4(0.0); }\n").unwrap();

    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(root.clone()));
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/shaders");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let granted = pkg.manifest.permissions.clone();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, BTreeSet::new());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // WGSL: naga finds a float returned as an int, on its line.
    host.event(coiman_core::extensions::plugin::manifest::EventKind::Opened, Event::Opened("bad.wgsl".into()));
    let found = problems(&host, "bad.wgsl");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].line, 2, "{found:?}");

    // GLSL: the driver compiles what the file becomes (uniforms added, a main for mainImage),
    // and its log comes back on the file's own lines.
    host.event(coiman_core::extensions::plugin::manifest::EventKind::Opened, Event::Opened("toy.frag".into()));
    let (id, stage, source) = compile(&host);
    assert_eq!(stage, ShaderStage::Fragment);
    assert!(source.contains("uniform float iTime;") && source.contains("mainImage(coiman_color"), "{source}");
    let driver_line = source.lines().position(|l| l.contains("c = vec4")).unwrap() + 1;
    let log = format!("0({driver_line}) : error C1008: undefined variable \"q\"");
    host.deliver(SHADERS, Event::ShaderCompiled(ShaderCompiled { id, log: Some(log) }));
    let found = problems(&host, "toy.frag");
    assert_eq!((found.len(), found[0].line), (1, 2), "{found:?}");
    assert_eq!(found[0].message, "undefined variable \"q\"");
    // A vertex shader as it is; fixed, no problems.
    host.event(coiman_core::extensions::plugin::manifest::EventKind::Opened, Event::Opened("pass.vert".into()));
    let (id, stage, _) = compile(&host);
    assert_eq!(stage, ShaderStage::Vertex);
    host.deliver(SHADERS, Event::ShaderCompiled(ShaderCompiled { id, log: None }));
    assert!(problems(&host, "pass.vert").is_empty());

    // The preview: the active fragment shader's program goes to the editor to draw.
    host.set_document(Some(document("toy.frag", TOY)));
    host.run_command(SHADERS, "preview", "");
    let (view, text) = panel(&host, |v, _| v.is_some());
    let view = view.unwrap();
    assert!(text.contains("toy.frag [\"Pause\", \"Restart\"]"), "{text}");
    assert!(view.fragment.contains("mainImage(coiman_color") && view.vertex.contains("coiman_pos"));
    assert!(!view.top_down && !view.paused);
    // A link error shows under the picture, on the file's line.
    let log = format!("0({driver_line}) : error C1008: undefined variable \"q\"");
    host.deliver(SHADERS, Event::ShaderCompiled(ShaderCompiled { id: view.id, log: Some(log) }));
    panel(&host, |_, t| t.contains("line 2: undefined variable \"q\""));
    // Pause keeps the same program, paused.
    host.run_command(SHADERS, "pause", "");
    let (paused, text) = panel(&host, |v, _| v.as_ref().is_some_and(|v| v.paused));
    assert_eq!(paused.unwrap().id, view.id);
    assert!(text.contains("[\"Play\", \"Restart\"]"), "{text}");
    host.run_command(SHADERS, "restart", "");
    let (restarted, _) = panel(&host, |v, _| v.as_ref().is_some_and(|v| v.restart != view.restart));
    assert_eq!(restarted.unwrap().id, view.id);

    // Another file: WGSL turned into GLSL, drawn the other way up.
    host.set_document(Some(document("good.wgsl", WGSL)));
    host.event(
        coiman_core::extensions::plugin::manifest::EventKind::ActiveChanged,
        Event::ActiveChanged("good.wgsl".into()),
    );
    let (wgsl, text) = panel(&host, |v, _| v.as_ref().is_some_and(|v| v.top_down));
    let wgsl = wgsl.unwrap();
    assert!(wgsl.id != view.id && wgsl.fragment.starts_with("#version 330 core"), "{}", wgsl.fragment);
    assert!(text.contains("good.wgsl"), "{text}");
    // Broken while typing: the last picture stays, with what's wrong.
    host.set_document(Some(document("good.wgsl", "fn f() -> i32 {\n  return 1.5;\n}\n")));
    host.event(coiman_core::extensions::plugin::manifest::EventKind::Edited, Event::Edited("good.wgsl".into()));
    let (kept, text) = panel(&host, |_, t| t.contains("line 2:"));
    assert_eq!(kept.unwrap().id, wgsl.id, "{text}");
    drop(host);
    let _ = std::fs::remove_dir_all(&root);
}
