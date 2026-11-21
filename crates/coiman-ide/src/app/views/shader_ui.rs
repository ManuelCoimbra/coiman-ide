//! Shaders in the app: GLSL problems while typing, found by the graphics driver itself, and
//! Preview: Shader, which draws the active fragment shader (GLSL, or WGSL through naga) live,
//! with time, resolution and mouse as Shadertoy and The Book of Shaders name them.
//!
//! The work that needs OpenGL runs in an egui paint callback, on the thread and context that
//! draw the window; the UI hands it jobs and reads results through a shared, locked state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use coiman_core::language::lsp::types as lt;
use coiman_core::language::markup::{Problem, Severity};
use coiman_core::language::shader::{self, Kind, Prepared, Stage};
use eframe::egui::{self, RichText, Stroke};
use eframe::glow::{self, HasContext};

use crate::app::CodeApp;
use crate::app::lsp::path_key;
use crate::ui::theme;

/// Seconds typing must pause before the driver checks the file again.
const REST: f64 = 0.4;

/// A shader for the driver to compile: to check it, or to draw it.
struct Job {
    uid: u64,
    version: u64,
    path: PathBuf,
    stage: Stage,
    prepared: Prepared,
    /// Draw it (link it with the vertex shader and keep the program).
    preview: bool,
    /// WGSL: y grows downward, as in wgpu, so the picture is turned over on the way out.
    top_down: bool,
}

/// What a compile said: the job's file and version, and the driver's log (empty if fine).
struct Compiled {
    uid: u64,
    version: u64,
    path: PathBuf,
    preview: bool,
    problems: Vec<Problem>,
}

#[derive(Default)]
struct Gl {
    jobs: Vec<Job>,
    done: Vec<Compiled>,
    program: Option<glow::Program>,
    /// The program's `top_down`.
    top_down: bool,
    vao: Option<glow::VertexArray>,
    vbo: Option<glow::Buffer>,
    ubo: Option<glow::Buffer>,
    /// The picture is drawn here at its own size, then copied into place: so
    /// `gl_FragCoord` counts from the picture's corner, not the window's.
    target: Option<(glow::Framebuffer, glow::Texture, i32, i32)>,
    /// Uniform values for the next draw.
    time: f32,
    delta: f32,
    frame: i32,
    mouse: [f32; 4],
    hover: [f32; 2],
    /// Shown when the driver couldn't even start (no OpenGL here).
    broken: Option<String>,
}

#[derive(Default)]
pub(crate) struct ShaderUi {
    gl: Arc<Mutex<Gl>>,
    /// GLSL problems by file (key: [`path_key`]).
    found: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
    checked: HashMap<u64, u64>,
    changed_since: Option<f64>,
    /// The preview.
    open: bool,
    /// The file drawn: (doc uid, version sent, path).
    showing: Option<(u64, u64, PathBuf)>,
    /// What's wrong with it, when it doesn't compile (the last good image stays).
    errors: Vec<Problem>,
    start: f64,
    paused_at: Option<f64>,
    last_frame: f64,
    frame: i32,
    mouse: [f32; 4],
    /// Where the pointer is over the picture (The Book of Shaders' `u_mouse`).
    hover: [f32; 2],
}

fn diagnostic(p: &Problem) -> lt::Diagnostic {
    lt::Diagnostic {
        range: lt::Range::new(lt::Position::new(p.line, p.col), lt::Position::new(p.end_line, p.end_col)),
        severity: Some(match p.severity {
            Severity::Error => lt::DiagnosticSeverity::ERROR,
            Severity::Warning => lt::DiagnosticSeverity::WARNING,
            Severity::Information => lt::DiagnosticSeverity::INFORMATION,
        }),
        source: Some("GLSL".to_owned()),
        message: p.message.clone(),
        ..Default::default()
    }
}

fn shader_type(stage: Stage) -> u32 {
    match stage {
        Stage::Vertex => glow::VERTEX_SHADER,
        Stage::Fragment => glow::FRAGMENT_SHADER,
        Stage::Geometry => glow::GEOMETRY_SHADER,
        Stage::Compute => glow::COMPUTE_SHADER,
        Stage::TessControl => glow::TESS_CONTROL_SHADER,
        Stage::TessEvaluation => glow::TESS_EVALUATION_SHADER,
    }
}

/// Compiles one stage: the shader, or the driver's log.
fn compile(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    // SAFETY: called from the paint callback, with egui's current OpenGL context.
    unsafe {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if gl.get_shader_compile_status(shader) {
            Ok(shader)
        } else {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            Err(if log.trim().is_empty() { "error: the shader didn't compile".to_owned() } else { log })
        }
    }
}

/// A program from the preview's vertex shader and this fragment shader.
fn link(gl: &glow::Context, prepared: &Prepared) -> Result<glow::Program, String> {
    let fragment = compile(gl, glow::FRAGMENT_SHADER, &prepared.source)?;
    let vertex = compile(gl, glow::VERTEX_SHADER, &prepared.vertex).inspect_err(|_| {
        // SAFETY: as above.
        unsafe { gl.delete_shader(fragment) }
    })?;
    // SAFETY: as above.
    unsafe {
        let program = gl.create_program()?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.bind_attrib_location(program, 0, "coiman_pos");
        gl.link_program(program);
        gl.detach_shader(program, vertex);
        gl.detach_shader(program, fragment);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
        if gl.get_program_link_status(program) {
            Ok(program)
        } else {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            Err(format!("error: {}", log.trim()))
        }
    }
}

/// The work for the OpenGL thread: compile what's waiting, then draw the preview if asked.
fn run(gl: &glow::Context, state: &mut Gl, draw: Option<[i32; 4]>) {
    for job in std::mem::take(&mut state.jobs) {
        let result = if job.preview {
            link(gl, &job.prepared).map(|program| {
                state.top_down = job.top_down;
                if let Some(old) = state.program.replace(program) {
                    // SAFETY: as above.
                    unsafe { gl.delete_program(old) }
                }
            })
        } else {
            compile(gl, shader_type(job.stage), &job.prepared.source).map(|s| {
                // SAFETY: as above.
                unsafe { gl.delete_shader(s) }
            })
        };
        let problems = match result {
            Ok(()) => Vec::new(),
            // A stage this OpenGL doesn't have (compute before 4.3): nothing to say.
            Err(log) if log.contains("Invalid enum") || log.contains("INVALID_ENUM") => Vec::new(),
            Err(log) => shader::parse_driver_log(&log, &job.prepared),
        };
        state.done.push(Compiled {
            uid: job.uid,
            version: job.version,
            path: job.path,
            preview: job.preview,
            problems,
        });
    }
    let (Some([left, bottom, width, height]), Some(program)) = (draw, state.program) else { return };
    if width <= 0 || height <= 0 {
        return;
    }
    // SAFETY: as above; egui restores its own state after the callback.
    unsafe {
        if state.vao.is_none() {
            let vao = match gl.create_vertex_array() {
                Ok(v) => v,
                Err(e) => {
                    state.broken = Some(e);
                    return;
                }
            };
            let vbo = gl.create_buffer().ok();
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, vbo);
            // One triangle that covers the whole rectangle.
            let corners: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
            let bytes: Vec<u8> = corners.iter().flat_map(|f| f.to_le_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            state.vao = Some(vao);
            state.vbo = vbo;
            state.ubo = gl.create_buffer().ok();
        }
        // Where egui draws (the screen), asked before anything here binds its own.
        let screen = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        // The off-screen picture, made again when the size changes.
        if state.target.is_none_or(|(_, _, w, h)| (w, h) != (width, height)) {
            if let Some((fbo, tex, _, _)) = state.target.take() {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
            }
            let (Ok(fbo), Ok(tex)) = (gl.create_framebuffer(), gl.create_texture()) else { return };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            state.target = Some((fbo, tex, width, height));
        }
        let Some((fbo, ..)) = state.target else { return };
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, width, height);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        gl.use_program(Some(program));
        gl.bind_vertex_array(state.vao);
        let set = |name: &str, f: &dyn Fn(&glow::UniformLocation)| {
            if let Some(at) = gl.get_uniform_location(program, name) {
                f(&at);
            }
        };
        let (w, h) = (width as f32, height as f32);
        let m = state.mouse;
        set("iTime", &|at| gl.uniform_1_f32(Some(at), state.time));
        set("u_time", &|at| gl.uniform_1_f32(Some(at), state.time));
        set("iTimeDelta", &|at| gl.uniform_1_f32(Some(at), state.delta));
        set("iFrame", &|at| gl.uniform_1_i32(Some(at), state.frame));
        set("iResolution", &|at| gl.uniform_3_f32(Some(at), w, h, 1.0));
        set("u_resolution", &|at| gl.uniform_2_f32(Some(at), w, h));
        set("iMouse", &|at| gl.uniform_4_f32(Some(at), m[0], m[1], m[2], m[3]));
        set("u_mouse", &|at| gl.uniform_2_f32(Some(at), state.hover[0], state.hover[1]));
        // WGSL's uniform block (time, resolution, mouse; std140, 32 bytes) at binding 0.
        let blocks = gl.get_program_parameter_i32(program, glow::ACTIVE_UNIFORM_BLOCKS);
        if blocks > 0 && state.ubo.is_some() {
            // Mouse y from the top, as WGSL counts.
            let flip = |y: f32| if y == 0.0 { 0.0 } else { (h - y.abs()) * y.signum() };
            let values: [f32; 8] = [state.time, 0.0, w, h, m[0], flip(m[1]), m[2], flip(m[3])];
            let bytes: Vec<u8> = values.iter().flat_map(|f| f.to_le_bytes()).collect();
            gl.bind_buffer(glow::UNIFORM_BUFFER, state.ubo);
            gl.buffer_data_u8_slice(glow::UNIFORM_BUFFER, &bytes, glow::DYNAMIC_DRAW);
            for i in 0..blocks as u32 {
                gl.uniform_block_binding(program, i, 0);
            }
            gl.bind_buffer_base(glow::UNIFORM_BUFFER, 0, state.ubo);
        }
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.use_program(None);
        // Into place on the screen, inside egui's clip.
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, screen);
        gl.enable(glow::SCISSOR_TEST);
        gl.blit_framebuffer(
            0,
            0,
            width,
            height,
            left,
            if state.top_down { bottom + height } else { bottom },
            left + width,
            if state.top_down { bottom } else { bottom + height },
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
    }
}

/// Asks egui to run the OpenGL work at `rect` (drawing there only for the preview).
fn callback(gl_state: &Arc<Mutex<Gl>>, rect: egui::Rect, draw: bool) -> egui::PaintCallback {
    let state = gl_state.clone();
    egui::PaintCallback {
        rect,
        callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
            let size = info.viewport_in_pixels();
            let draw = draw.then_some([size.left_px, size.from_bottom_px, size.width_px, size.height_px]);
            if let Ok(mut s) = state.lock() {
                run(painter.gl(), &mut s, draw);
            }
        })),
    }
}

impl CodeApp {
    /// GLSL files are compiled by the driver after each pause in typing; what it says
    /// becomes their problems.
    pub(crate) fn pump_shaders(&mut self, ctx: &egui::Context) {
        let done = self.shader.gl.lock().map(|mut g| std::mem::take(&mut g.done)).unwrap_or_default();
        let mut waiting = false;
        for c in done {
            if c.preview {
                if self.shader.showing.as_ref().is_some_and(|(uid, v, _)| *uid == c.uid && *v == c.version) {
                    self.shader.errors = c.problems;
                }
                continue;
            }
            self.shader.checked.insert(c.uid, c.version);
            let found = c.problems.iter().map(diagnostic).collect();
            self.shader.found.insert(path_key(&c.path), (c.path, found));
        }
        let open: Vec<PathBuf> = self.docs.iter().filter_map(|d| d.path.as_deref().map(path_key)).collect();
        self.shader.found.retain(|key, _| open.contains(key));

        let changed = self.docs.iter().find(|d| {
            d.path.is_some()
                && matches!(shader::kind(&d.extension()), Some(Kind::Glsl(_)))
                && self.shader.checked.get(&d.uid) != Some(&d.buffer.version())
        });
        if let Some(doc) = changed {
            let now = ctx.input(|i| i.time);
            let since = *self.shader.changed_since.get_or_insert(now);
            if self.shader.checked.contains_key(&doc.uid) && now - since < REST {
                ctx.request_repaint_after(Duration::from_secs_f64(REST));
            } else if let (Some(Kind::Glsl(stage)), Some(path)) = (shader::kind(&doc.extension()), doc.path.clone()) {
                self.shader.changed_since = None;
                let text = doc.text();
                let prepared = if stage == Stage::Fragment {
                    shader::prepare_fragment(&text)
                } else {
                    shader::prepare_plain(&text)
                };
                let (uid, version) = (doc.uid, doc.buffer.version());
                // Marked now so it isn't sent twice while the driver works.
                self.shader.checked.insert(uid, version);
                if let Ok(mut g) = self.shader.gl.lock() {
                    g.jobs.push(Job { uid, version, path, stage, prepared, preview: false, top_down: false });
                }
            }
        } else {
            self.shader.changed_since = None;
        }
        if let Ok(g) = self.shader.gl.lock() {
            waiting = !g.jobs.is_empty();
        }
        // Compiling needs the OpenGL thread: a callback on one pixel, if the preview isn't
        // drawing anyway.
        if waiting && !self.shader.open {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0));
            ctx.layer_painter(egui::LayerId::background()).add(callback(&self.shader.gl, rect, false));
            ctx.request_repaint();
        }
    }

    pub(crate) fn shader_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        self.shader.found.get(key).map(|(_, d)| d.clone()).unwrap_or_default()
    }

    pub(crate) fn shader_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.shader.found.iter().filter(|(_, (_, d))| !d.is_empty()).map(|(k, (p, _))| (k.clone(), p.clone())).collect()
    }

    pub(crate) fn show_shader_preview(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let s = &mut self.shader;
        if !s.open {
            s.start = now;
            s.paused_at = None;
            s.frame = 0;
        }
        s.open = true;
        let previewable = self.docs.get(self.active).is_some_and(|d| shader::previews(&d.extension()));
        if !previewable && self.shader.showing.is_none() {
            self.status = Some("Open a fragment shader (.frag, .glsl or .wgsl) to preview it".to_owned());
        }
    }

    /// Sends the active fragment shader to the preview when it's new or has changed.
    fn shader_follow(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let ext = doc.extension();
        if !shader::previews(&ext) {
            return;
        }
        let path = doc.path.clone().unwrap_or_else(|| PathBuf::from(format!("untitled.{ext}")));
        let (uid, version) = (doc.uid, doc.buffer.version());
        if self.shader.showing.as_ref().is_some_and(|(u, v, _)| *u == uid && *v == version) {
            return;
        }
        let text = doc.text();
        let top_down = shader::kind(&ext) == Some(Kind::Wgsl);
        let prepared = match shader::kind(&ext) {
            Some(Kind::Wgsl) => match shader::wgsl_to_glsl(&text) {
                Ok(glsl) => shader::prepare_plain(&glsl),
                Err(problems) => {
                    self.shader.showing = Some((uid, version, path));
                    self.shader.errors = problems;
                    return;
                }
            },
            _ => shader::prepare_fragment(&text),
        };
        if self.shader.showing.as_ref().is_none_or(|(u, _, _)| *u != uid) {
            // Another file: its own clock.
            self.shader.frame = 0;
        }
        self.shader.showing = Some((uid, version, path.clone()));
        if let Ok(mut g) = self.shader.gl.lock() {
            g.jobs.push(Job { uid, version, path, stage: Stage::Fragment, prepared, preview: true, top_down });
        }
    }

    pub(crate) fn shader_preview_window(&mut self, ctx: &egui::Context) {
        if !self.shader.open {
            return;
        }
        self.shader_follow();
        let pal = theme::colors();
        let now = ctx.input(|i| i.time);
        let mut open = true;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.4).clamp(320.0, 640.0), (room.y * 0.45).clamp(260.0, 520.0));
        let mut restart = false;
        let mut pause = false;
        egui::Window::new(RichText::new("Shader Preview").size(16.0).strong())
            .id(egui::Id::unique("shader_preview"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(size)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-size.x - 24.0, 60.0))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ctx, |ui| {
                let s = &self.shader;
                let time = s.paused_at.unwrap_or(now) - s.start;
                ui.horizontal(|ui| {
                    let name = s
                        .showing
                        .as_ref()
                        .and_then(|(_, _, p)| p.file_name())
                        .map_or_else(|| "No shader yet".to_owned(), |n| n.to_string_lossy().into_owned());
                    ui.label(RichText::new(name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Restart").on_hover_text("Time back to 0").clicked() {
                            restart = true;
                        }
                        if ui.button(if s.paused_at.is_some() { "Play" } else { "Pause" }).clicked() {
                            pause = true;
                        }
                        ui.label(RichText::new(format!("{time:.1} s")).monospace().color(pal.text_dim));
                    });
                });
                if let Some(broken) = self.shader.gl.lock().ok().and_then(|g| g.broken.clone()) {
                    ui.label(
                        RichText::new(format!("OpenGL couldn't draw here: {broken}")).color(pal.bracket_unexpected),
                    );
                }
                // The errors, under the last image that worked.
                let errors_height = if s.errors.is_empty() { 0.0 } else { 18.0 * s.errors.len().min(6) as f32 + 8.0 };
                let avail = ui.available_size() - egui::vec2(0.0, errors_height);
                let (rect, response) =
                    ui.allocate_exact_size(avail.max(egui::vec2(64.0, 64.0)), egui::Sense::click_and_drag());
                ui.painter().rect_filled(rect, 6.0, egui::Color32::BLACK);
                if s.showing.is_some() {
                    ui.painter().add(callback(&self.shader.gl, rect, true));
                }
                // Mouse, in pixels from the bottom left: xy while pressed, zw where it was
                // pressed (negative once let go), as Shadertoy has it.
                let ppp = ctx.pixels_per_point();
                let mut mouse = self.shader.mouse;
                if let Some(pos) = response.interact_pointer_pos().or_else(|| response.hover_pos()) {
                    let x = (pos.x - rect.left()) * ppp;
                    let y = (rect.bottom() - pos.y) * ppp;
                    if response.is_pointer_button_down_on() {
                        if response.drag_started() || response.clicked() || mouse[2] <= 0.0 {
                            mouse[2] = x;
                            mouse[3] = y;
                        }
                        mouse[0] = x;
                        mouse[1] = y;
                    }
                    self.shader.hover = [x, y];
                }
                if !response.is_pointer_button_down_on() && mouse[2] > 0.0 {
                    mouse[2] = -mouse[2];
                    mouse[3] = -mouse[3];
                }
                self.shader.mouse = mouse;
                let s = &self.shader;
                for e in s.errors.iter().take(6) {
                    let text = format!("line {}: {}", e.line + 1, e.message);
                    let color = if e.severity == Severity::Warning { pal.modified } else { pal.bracket_unexpected };
                    ui.add(egui::Label::new(RichText::new(text).monospace().small().color(color)).truncate());
                }
            });
        let s = &mut self.shader;
        if restart {
            s.start = now;
            s.frame = 0;
            if s.paused_at.is_some() {
                s.paused_at = Some(now);
            }
        }
        if pause {
            match s.paused_at.take() {
                Some(at) => s.start += now - at,
                None => s.paused_at = Some(now),
            }
        }
        if !open {
            s.open = false;
            s.showing = None;
            s.errors.clear();
            return;
        }
        let time = (s.paused_at.unwrap_or(now) - s.start) as f32;
        let delta = (now - s.last_frame).clamp(0.0, 1.0) as f32;
        s.last_frame = now;
        if s.paused_at.is_none() {
            s.frame += 1;
        }
        if let Ok(mut g) = s.gl.lock() {
            g.time = time;
            g.delta = delta;
            g.frame = s.frame;
            g.mouse = s.mouse;
            g.hover = s.hover;
        }
        // Moving pictures need the next frame.
        if s.paused_at.is_none() {
            ctx.request_repaint();
        }
    }
}
