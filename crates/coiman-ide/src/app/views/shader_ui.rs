//! The editor's OpenGL, offered to extensions: a GLSL shader compiled by the graphics driver
//! itself (`compile-shader`, answered with its log as `shader-compiled`), and `shader` panel
//! blocks drawn live, with time, resolution and mouse as Shadertoy and The Book of Shaders
//! name them. The Shaders extension does the rest (WGSL, problems, the preview's panel).
//!
//! The work that needs OpenGL runs in an egui paint callback, on the thread and context that
//! draw the window; the UI hands it jobs and reads results through a shared, locked state.

use std::sync::{Arc, Mutex};

use coiman_core::extensions::plugin::Event;
use coiman_core::extensions::plugin::host::{ShaderCompiled, ShaderStage, ShaderView};
use eframe::egui::{self, RichText};
use eframe::glow::{self, HasContext};

use crate::app::CodeApp;
use crate::ui::theme;

/// What the driver is asked.
enum What {
    /// Compile one stage, to check it.
    Compile(ShaderStage, String),
    /// Link a program for a `shader` block (it replaces the one drawn if it links).
    Link { fragment: String, vertex: String, top_down: bool },
}

/// A job for the OpenGL thread, for a plugin, with the plugin's id for it.
struct Job {
    plugin: String,
    id: u64,
    what: What,
}

#[derive(Default)]
pub(crate) struct Gl {
    jobs: Vec<Job>,
    /// Answers to deliver: plugin, id, the driver's log (none when it went through).
    done: Vec<(String, u64, Option<String>)>,
    program: Option<glow::Program>,
    /// The `shader` block whose program is drawn, and the one asked for (plugin, id).
    linked: Option<(String, u64)>,
    asked: Option<(String, u64)>,
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
    /// The clock: when time was 0, when it was paused, the last frame drawn, and the block's
    /// `restart` number last seen.
    start: f64,
    paused_at: Option<f64>,
    last_frame: f64,
    restart: Option<u32>,
    /// Shown when the driver couldn't even start (no OpenGL here).
    broken: Option<String>,
}

#[derive(Default)]
pub(crate) struct ShaderUi {
    pub(crate) gl: Arc<Mutex<Gl>>,
}

fn shader_type(stage: ShaderStage) -> u32 {
    match stage {
        ShaderStage::Vertex => glow::VERTEX_SHADER,
        ShaderStage::Fragment => glow::FRAGMENT_SHADER,
        ShaderStage::Geometry => glow::GEOMETRY_SHADER,
        ShaderStage::Compute => glow::COMPUTE_SHADER,
        ShaderStage::TessControl => glow::TESS_CONTROL_SHADER,
        ShaderStage::TessEvaluation => glow::TESS_EVALUATION_SHADER,
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

/// A program from a vertex and a fragment shader.
fn link(gl: &glow::Context, fragment: &str, vertex: &str) -> Result<glow::Program, String> {
    let fragment = compile(gl, glow::FRAGMENT_SHADER, fragment)?;
    let vertex = compile(gl, glow::VERTEX_SHADER, vertex).inspect_err(|_| {
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

/// The work for the OpenGL thread: compile what's waiting, then draw the picture if asked.
fn run(gl: &glow::Context, state: &mut Gl, draw: Option<[i32; 4]>) {
    for job in std::mem::take(&mut state.jobs) {
        let result = match job.what {
            What::Link { fragment, vertex, top_down } => link(gl, &fragment, &vertex).map(|program| {
                state.top_down = top_down;
                state.linked = Some((job.plugin.clone(), job.id));
                if let Some(old) = state.program.replace(program) {
                    // SAFETY: as above.
                    unsafe { gl.delete_program(old) }
                }
            }),
            What::Compile(stage, source) => compile(gl, shader_type(stage), &source).map(|s| {
                // SAFETY: as above.
                unsafe { gl.delete_shader(s) }
            }),
        };
        let log = match result {
            Ok(()) => None,
            // A stage this OpenGL doesn't have (compute before 4.3): nothing to say.
            Err(log) if log.contains("Invalid enum") || log.contains("INVALID_ENUM") => None,
            Err(log) => Some(log),
        };
        state.done.push((job.plugin, job.id, log));
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
    /// A shader for the driver to compile, for a plugin.
    pub(crate) fn shader_compile(&mut self, plugin: String, id: u64, stage: ShaderStage, source: String) {
        if let Ok(mut g) = self.shader.gl.lock() {
            g.jobs.push(Job { plugin, id, what: What::Compile(stage, source) });
        }
    }

    /// Delivers what the driver said, and has it work when jobs wait and no picture is drawn.
    pub(crate) fn pump_shaders(&mut self, ctx: &egui::Context) {
        let (done, waiting) = match self.shader.gl.lock() {
            Ok(mut g) => (std::mem::take(&mut g.done), !g.jobs.is_empty()),
            Err(_) => return,
        };
        for (plugin, id, log) in done {
            self.deliver_to_plugin(&plugin, Event::ShaderCompiled(ShaderCompiled { id, log }));
        }
        // Compiling needs the OpenGL thread: a callback on one pixel.
        if waiting {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0));
            ctx.layer_painter(egui::LayerId::background()).add(callback(&self.shader.gl, rect, false));
            ctx.request_repaint();
        }
    }
}

/// A `shader` block: the plugin's program drawn live, `height` points tall.
pub(crate) fn shader_block_ui(ui: &mut egui::Ui, gl: &Arc<Mutex<Gl>>, plugin: &str, view: &ShaderView, height: f32) {
    let pal = theme::colors();
    let ctx = ui.ctx().clone();
    let now = ui.input(|i| i.time);
    let Ok(mut g) = gl.lock() else { return };
    let wanted = Some((plugin.to_owned(), view.id));
    if g.linked != wanted && g.asked != wanted {
        g.asked = wanted;
        let what = What::Link { fragment: view.fragment.clone(), vertex: view.vertex.clone(), top_down: view.top_down };
        g.jobs.push(Job { plugin: plugin.to_owned(), id: view.id, what });
    }
    // The clock: a new restart number sets it to 0; pausing holds it.
    if g.restart != Some(view.restart) {
        g.restart = Some(view.restart);
        g.start = now;
        g.frame = 0;
        g.paused_at = view.paused.then_some(now);
    }
    match (view.paused, g.paused_at) {
        (true, None) => g.paused_at = Some(now),
        (false, Some(at)) => {
            g.start += now - at;
            g.paused_at = None;
        }
        _ => {}
    }
    let time = g.paused_at.unwrap_or(now) - g.start;
    if let Some(broken) = &g.broken {
        ui.label(RichText::new(format!("OpenGL couldn't draw here: {broken}")).color(pal.bracket_unexpected));
    }
    let size = egui::vec2(ui.available_width().max(64.0), height.max(64.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    ui.painter().rect_filled(rect, 6.0, egui::Color32::BLACK);
    if g.program.is_some() && g.linked.as_ref().is_some_and(|(p, _)| p == plugin) {
        ui.painter().add(callback(gl, rect, true));
    }
    ui.painter().text(
        rect.right_top() + egui::vec2(-8.0, 6.0),
        egui::Align2::RIGHT_TOP,
        format!("{time:.1} s"),
        egui::FontId::monospace(12.0),
        egui::Color32::from_white_alpha(200),
    );
    // Mouse, in pixels from the bottom left: xy while pressed, zw where it was pressed
    // (negative once let go), as Shadertoy has it.
    let ppp = ctx.pixels_per_point();
    let mut mouse = g.mouse;
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
        g.hover = [x, y];
    }
    if !response.is_pointer_button_down_on() && mouse[2] > 0.0 {
        mouse[2] = -mouse[2];
        mouse[3] = -mouse[3];
    }
    g.mouse = mouse;
    g.time = time as f32;
    g.delta = (now - g.last_frame).clamp(0.0, 1.0) as f32;
    g.last_frame = now;
    if g.paused_at.is_none() {
        g.frame += 1;
        // Moving pictures need the next frame.
        ctx.request_repaint();
    }
}
