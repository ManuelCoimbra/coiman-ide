//! Shaders: GLSL and WGSL problems while typing, and a live preview of a fragment shader in a
//! panel. WGSL is read and turned into GLSL here with naga; GLSL is checked by the graphics
//! driver itself, through the editor's OpenGL (`compile-shader`), which also draws the preview
//! (a `shader` block).

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod shader;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, Problem as Found, Row, Severity as Level, ShaderCompiled, ShaderStage, ShaderView, Span,
};
use shader::{Kind, Prepared, Problem, Severity, Stage};

const RED: u32 = 0xE5484D;
const YELLOW: u32 = 0xE2A93B;
const DIM: u32 = 0x8B949E;

/// Errors listed under the picture.
const SHOWN_ERRORS: usize = 6;

/// The preview: the file followed, what is drawn and what's wrong with it.
#[derive(Default)]
struct Preview {
    /// The file shown (relative path).
    path: String,
    /// The program drawn, as last sent.
    view: Option<ShaderView>,
    errors: Vec<Problem>,
    paused: bool,
    restart: u32,
}

#[derive(Default)]
struct State {
    /// GLSL files being checked by the driver: id to file and how its lines map.
    checks: HashMap<u64, (String, Prepared)>,
    /// Programs the preview asked to link: id to how their lines map.
    links: HashMap<u64, Prepared>,
    preview: Option<Preview>,
    next_view: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn extension(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e.to_owned()).unwrap_or_default()
}

fn stage(s: Stage) -> ShaderStage {
    match s {
        Stage::Vertex => ShaderStage::Vertex,
        Stage::Fragment => ShaderStage::Fragment,
        Stage::Geometry => ShaderStage::Geometry,
        Stage::Compute => ShaderStage::Compute,
        Stage::TessControl => ShaderStage::TessControl,
        Stage::TessEvaluation => ShaderStage::TessEvaluation,
    }
}

/// The file's text: the active editor's (unsaved changes too) when it's that file, else as
/// saved.
fn text_of(path: &str) -> Option<String> {
    if let Ok(Some(doc)) = host::active_document()
        && doc.path == path
    {
        return Some(doc.text);
    }
    host::read_file(path).ok()
}

fn found(problems: &[Problem]) -> Vec<Found> {
    problems
        .iter()
        .map(|p| Found {
            line: p.line + 1,
            start: p.col,
            end: if p.end_line == p.line { p.end_col.max(p.col + 1) } else { 999 },
            severity: if p.severity == Severity::Warning { Level::Warning } else { Level::Error },
            message: p.message.clone(),
        })
        .collect()
}

/// Checks a shader file: WGSL here, GLSL by the driver (its problems come back later).
fn check(path: &str) {
    let Some(kind) = shader::kind(&extension(path)) else { return };
    let Some(text) = text_of(path) else { return };
    match kind {
        Kind::Wgsl => host::set_problems(path, &found(&shader::check_wgsl(&text))),
        Kind::Glsl(s) => {
            let prepared =
                if s == Stage::Fragment { shader::prepare_fragment(&text) } else { shader::prepare_plain(&text) };
            let id = host::compile_shader(stage(s), &prepared.source);
            with(|st| st.checks.insert(id, (path.to_owned(), prepared)));
        }
        // HLSL has no checker here.
        Kind::Hlsl => {}
    }
}

/// The preview follows a fragment shader: its program goes to the editor to draw.
fn follow(path: &str) {
    if !with(|s| s.preview.is_some()) || !shader::previews(&extension(path)) {
        return;
    }
    let Some(text) = text_of(path) else { return };
    let wgsl = shader::kind(&extension(path)) == Some(Kind::Wgsl);
    let prepared = if wgsl {
        shader::wgsl_to_glsl(&text).map(|glsl| shader::prepare_plain(&glsl))
    } else {
        Ok(shader::prepare_fragment(&text))
    };
    with(|s| {
        let Some(p) = s.preview.as_mut() else { return };
        if p.path != path {
            // Another file: its own clock.
            p.path = path.to_owned();
            p.restart += 1;
        }
        match prepared {
            Ok(prepared) => {
                s.next_view += 1;
                let id = s.next_view;
                let p = s.preview.as_mut().expect("there is a preview");
                p.view = Some(ShaderView {
                    id,
                    fragment: prepared.source.clone(),
                    vertex: prepared.vertex.clone(),
                    top_down: wgsl,
                    paused: p.paused,
                    restart: p.restart,
                });
                p.errors.clear();
                s.links.insert(id, prepared);
            }
            // The last picture that worked stays, with what's wrong under it.
            Err(problems) => p.errors = problems,
        }
    });
    render();
}

fn compiled(c: ShaderCompiled) {
    let check = with(|s| s.checks.remove(&c.id));
    if let Some((path, prepared)) = check {
        let problems = c.log.map(|log| shader::parse_driver_log(&log, &prepared)).unwrap_or_default();
        host::set_problems(&path, &found(&problems));
        return;
    }
    let link = with(|s| s.links.remove(&c.id));
    if let Some(prepared) = link {
        let errors = c.log.map(|log| shader::parse_driver_log(&log, &prepared)).unwrap_or_default();
        with(|s| {
            if let Some(p) = s.preview.as_mut()
                && p.view.as_ref().is_some_and(|v| v.id == c.id)
            {
                p.errors = errors;
            }
        });
        render();
    }
}

fn span(text: impl Into<String>, color: Option<u32>, bold: bool, monospace: bool) -> Span {
    Span { text: text.into(), color, bold, monospace }
}

fn button(label: &str, command: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: String::new() }
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        let Some(p) = &s.preview else { return b };
        let name = if p.path.is_empty() {
            "No shader yet".to_owned()
        } else {
            p.path.rsplit('/').next().unwrap_or(&p.path).to_owned()
        };
        b.push(Block::Rows(vec![Row {
            cells: vec![span(name, None, true, false)],
            buttons: vec![button(if p.paused { "Play" } else { "Pause" }, "pause"), button("Restart", "restart")],
        }]));
        match &p.view {
            Some(view) => b.push(Block::Shader(view.clone())),
            None if p.errors.is_empty() => b.push(Block::Spans(vec![span(
                "Open a fragment shader (.frag, .glsl or .wgsl) to preview it. It's drawn with the uniforms Shadertoy and The Book of Shaders name (iTime, iResolution, iMouse, u_time…).",
                Some(DIM),
                false,
                false,
            )])),
            None => {}
        }
        for e in p.errors.iter().take(SHOWN_ERRORS) {
            let color = if e.severity == Severity::Warning { YELLOW } else { RED };
            b.push(Block::Spans(vec![span(format!("line {}: {}", e.line + 1, e.message), Some(color), false, true)]));
        }
        b
    });
    host::set_panel("shader", &blocks);
}

fn command(id: &str) -> Result<(), String> {
    match id {
        "preview" => {
            with(|s| {
                s.preview.get_or_insert_with(Preview::default);
            });
            let active = host::active_document().ok().flatten().map(|d| d.path).filter(|p| !p.is_empty());
            match active {
                Some(path) if shader::previews(&extension(&path)) => follow(&path),
                _ => {
                    host::show_message("Open a fragment shader (.frag, .glsl or .wgsl) to preview it");
                    render();
                }
            }
            return Ok(());
        }
        "pause" | "restart" => with(|s| {
            let Some(p) = s.preview.as_mut() else { return };
            if id == "pause" {
                p.paused = !p.paused;
            } else {
                p.restart += 1;
            }
            let (paused, restart) = (p.paused, p.restart);
            if let Some(v) = p.view.as_mut() {
                v.paused = paused;
                v.restart = restart;
            }
        }),
        _ => {}
    }
    render();
    Ok(())
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        command(&id)
    }

    fn on_event(event: Event) {
        match event {
            Event::Opened(path) | Event::Saved(path) | Event::Edited(path) => {
                check(&path);
                follow(&path);
            }
            Event::ActiveChanged(path) => follow(&path),
            Event::ShaderCompiled(c) => compiled(c),
            _ => {}
        }
    }
}

export!(Plugin);
