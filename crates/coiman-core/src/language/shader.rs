//! Shaders: GLSL, HLSL and WGSL files. WGSL is read and checked here with naga, and turned
//! into GLSL for the preview. GLSL is checked by the graphics driver itself (the preview's
//! OpenGL), so this prepares the source it gets (uniforms, a `main` for a Shadertoy-style
//! `mainImage`) and reads the driver's log back onto the file's own lines.

use crate::language::markup::{Problem, Severity};

/// What kind of shader a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Glsl(Stage),
    Hlsl,
    Wgsl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Geometry,
    Compute,
    TessControl,
    TessEvaluation,
}

/// The shader kind of a file extension. Plain `.glsl` is taken as a fragment shader.
pub fn kind(extension: &str) -> Option<Kind> {
    Some(match extension.to_ascii_lowercase().as_str() {
        "wgsl" => Kind::Wgsl,
        "hlsl" | "hlsli" | "fx" | "fxh" => Kind::Hlsl,
        "vert" | "vs" | "vsh" => Kind::Glsl(Stage::Vertex),
        "frag" | "fs" | "fsh" | "glsl" | "shader" => Kind::Glsl(Stage::Fragment),
        "geom" | "gs" => Kind::Glsl(Stage::Geometry),
        "comp" => Kind::Glsl(Stage::Compute),
        "tesc" => Kind::Glsl(Stage::TessControl),
        "tese" => Kind::Glsl(Stage::TessEvaluation),
        _ => return None,
    })
}

/// Whether the preview can draw a file of this kind: a fragment shader, GLSL or WGSL.
pub fn previews(extension: &str) -> bool {
    matches!(kind(extension), Some(Kind::Wgsl | Kind::Glsl(Stage::Fragment)))
}

fn problem(line: u32, col: u32, len: u32, message: String) -> Problem {
    Problem { line, col, end_line: line, end_col: col + len.max(1), message, severity: Severity::Error }
}

/// The file's own line (0-based) and column of a byte offset.
fn place(source: &str, offset: usize) -> (u32, u32) {
    let before = &source[..offset.min(source.len())];
    let line = before.matches('\n').count() as u32;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) as u32;
    (line, col)
}

fn wgsl_module(source: &str) -> Result<(naga::Module, naga::valid::ModuleInfo), Vec<Problem>> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| {
        // The first label that marks something (one may be empty), else the error's place.
        let at = e
            .labels()
            .find(|(span, _)| span.to_range().is_some_and(|r| !r.is_empty()))
            .map(|(span, _)| span.location(source))
            .or_else(|| e.location(source));
        let (line, col, len) = match at {
            Some(at) => {
                let (line, col) = place(source, at.offset as usize);
                (line, col, at.length)
            }
            None => (0, 0, 1),
        };
        // The labels say where and what in a few words; the message says what's wrong.
        let mut message = e.message().to_owned();
        for (_, label) in e.labels() {
            if !label.is_empty() && !message.contains(label) {
                message.push_str(&format!(" ({label})"));
            }
        }
        vec![problem(line, col, len, message)]
    })?;
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
    let info = validator.validate(&module).map_err(|e| {
        let (line, col, len) = match e.location(source) {
            Some(at) => {
                let (line, col) = place(source, at.offset as usize);
                (line, col, at.length)
            }
            None => (0, 0, 1),
        };
        // The innermost cause says what's wrong ("Operation Add can't work with…"); the
        // outer ones only say where ("Entry point main…", "Expression [21]…").
        let mut message = e.as_inner().to_string();
        let mut cause = std::error::Error::source(e.as_inner());
        while let Some(c) = cause {
            message = c.to_string();
            cause = c.source();
        }
        vec![problem(line, col, len, message)]
    })?;
    Ok((module, info))
}

/// What's wrong with a WGSL file: its first syntax or validation error, where it is.
pub fn check_wgsl(source: &str) -> Vec<Problem> {
    wgsl_module(source).err().unwrap_or_default()
}

/// The uniforms the preview gives a WGSL shader, in the block it declares at group 0,
/// binding 0 (std140, 32 bytes): `time`, `resolution`, `mouse`.
pub const WGSL_UNIFORMS: &str = "struct Coiman { time: f32, resolution: vec2f, mouse: vec4f }\n\
@group(0) @binding(0) var<uniform> coiman: Coiman;";

/// A WGSL file's fragment entry point as GLSL 3.30 for the preview.
pub fn wgsl_to_glsl(source: &str) -> Result<String, Vec<Problem>> {
    let (module, info) = wgsl_module(source)?;
    let Some(entry) = module.entry_points.iter().find(|e| e.stage == naga::ShaderStage::Fragment) else {
        return Err(vec![problem(0, 0, 1, "No @fragment function to preview".to_owned())]);
    };
    let options = naga::back::glsl::Options {
        version: naga::back::glsl::Version::Desktop(330),
        writer_flags: naga::back::glsl::WriterFlags::empty(),
        ..Default::default()
    };
    let pipeline = naga::back::glsl::PipelineOptions {
        shader_stage: naga::ShaderStage::Fragment,
        entry_point: entry.name.clone(),
        multiview: None,
    };
    let mut out = String::new();
    naga::back::glsl::Writer::new(
        &mut out,
        &module,
        &info,
        &options,
        &pipeline,
        naga::proc::BoundsCheckPolicies::default(),
    )
    .and_then(|mut w| w.write())
    .map_err(|e| vec![problem(0, 0, 1, format!("Can't preview this: {e}"))])?;
    Ok(out)
}

/// GLSL ready for the driver: the source with what the preview adds, and how to read the
/// driver's line numbers back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub source: String,
    /// The preview's vertex shader, in the same GLSL version.
    pub vertex: String,
    /// Lines added after the file's line `after` (1-based; 0 when added before all of it).
    added: u32,
    after: u32,
}

impl Prepared {
    /// The file's line (0-based) for a line the driver reports (1-based).
    pub fn file_line(&self, driver_line: u32) -> u32 {
        let line = if driver_line > self.after + self.added {
            driver_line - self.added
        } else if driver_line > self.after {
            self.after + 1
        } else {
            driver_line
        };
        line.saturating_sub(1)
    }
}

/// The uniforms the preview sets, under the names Shadertoy and The Book of Shaders use.
pub const UNIFORMS: [(&str, &str); 8] = [
    ("iTime", "float"),
    ("iTimeDelta", "float"),
    ("iFrame", "int"),
    ("iResolution", "vec3"),
    ("iMouse", "vec4"),
    ("u_time", "float"),
    ("u_resolution", "vec2"),
    ("u_mouse", "vec2"),
];

fn declares_uniform(source: &str, name: &str) -> bool {
    source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("uniform") && l.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == name)
    })
}

fn has_main(source: &str) -> bool {
    let words: Vec<&str> =
        source.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    words.windows(2).any(|w| w == ["void", "main"])
}

/// A fragment shader as the preview gives it to the driver. Without `#version`, old-style
/// code (`gl_FragColor`) gets 1.20 and the rest 3.30 core. Uniforms the file doesn't declare
/// are added, and a Shadertoy `mainImage` gets a `main` that calls it.
pub fn prepare_fragment(source: &str) -> Prepared {
    // Where `#version` is (it must stay first).
    let version_at = source.lines().position(|l| l.trim_start().starts_with("#version"));
    let version_line = match version_at {
        Some(i) => source.lines().nth(i).unwrap_or("").trim().to_owned(),
        None if source.contains("gl_FragColor") && !source.contains("mainImage") => "#version 120".to_owned(),
        None => "#version 330 core".to_owned(),
    };
    let number: u32 = version_line.split_whitespace().nth(1).and_then(|n| n.parse().ok()).unwrap_or(330);
    let es = version_line.ends_with(" es");
    let old = number < 130 || number == 100;
    let shadertoy = source.contains("mainImage") && !has_main(source);

    let mut header = String::new();
    if version_at.is_none() {
        header.push_str(&version_line);
        header.push('\n');
    }
    if es {
        header.push_str("precision highp float;\n");
    }
    for (name, ty) in UNIFORMS {
        if !declares_uniform(source, name) {
            header.push_str(&format!("uniform {ty} {name};\n"));
        }
    }
    if shadertoy && !old {
        header.push_str("out vec4 coiman_color;\n");
    }
    let added = header.matches('\n').count() as u32;

    let mut out = String::with_capacity(source.len() + header.len() + 120);
    let after = match version_at {
        Some(i) => {
            for (n, line) in source.lines().enumerate() {
                out.push_str(line);
                out.push('\n');
                if n == i {
                    out.push_str(&header);
                }
            }
            i as u32 + 1
        }
        None => {
            out.push_str(&header);
            out.push_str(source);
            out.push('\n');
            0
        }
    };
    if shadertoy {
        out.push_str(if old {
            "void main() { vec4 c = vec4(0.0); mainImage(c, gl_FragCoord.xy); gl_FragColor = c; }\n"
        } else {
            "void main() { coiman_color = vec4(0.0); mainImage(coiman_color, gl_FragCoord.xy); }\n"
        });
    }
    Prepared { source: out, vertex: vertex_for(&version_line), added, after }
}

/// The preview's vertex shader (one triangle over the whole view) in a GLSL version.
fn vertex_for(version_line: &str) -> String {
    let number: u32 = version_line.split_whitespace().nth(1).and_then(|n| n.parse().ok()).unwrap_or(330);
    let input = if number < 130 || number == 100 { "attribute" } else { "in" };
    format!("{version_line}\n{input} vec2 coiman_pos;\nvoid main() {{ gl_Position = vec4(coiman_pos, 0.0, 1.0); }}\n")
}

/// A shader as the driver gets it, the file as it is (any stage, or GLSL naga wrote).
pub fn prepare_plain(source: &str) -> Prepared {
    let version_line = source
        .lines()
        .find(|l| l.trim_start().starts_with("#version"))
        .map_or_else(|| "#version 330 core".to_owned(), |l| l.trim().to_owned());
    Prepared { source: source.to_owned(), vertex: vertex_for(&version_line), added: 0, after: 0 }
}

/// The problems in a driver's compile log, on the file's lines. Reads the forms NVIDIA
/// (`0(12) : error C1008: …`), Mesa and AMD on Linux (`0:12(5): error: …`), and AMD and
/// Intel on Windows (`ERROR: 0:12: …`) write.
pub fn parse_driver_log(log: &str, prepared: &Prepared) -> Vec<Problem> {
    static FORMS: std::sync::OnceLock<[regex::Regex; 3]> = std::sync::OnceLock::new();
    let forms = FORMS.get_or_init(|| {
        [
            regex::Regex::new(r"^\s*\d+\((\d+)\)\s*:\s*(error|warning)\s*\w*\s*:\s*(.*)$").expect("valid"),
            regex::Regex::new(r"^\s*\d+:(\d+)\((\d+)\)\s*:\s*(error|warning)\s*:\s*(.*)$").expect("valid"),
            regex::Regex::new(r"^\s*(ERROR|WARNING)\s*:\s*\d+:(\d+)\s*:\s*(.*)$").expect("valid"),
        ]
    });
    let mut out = Vec::new();
    for line in log.lines() {
        let (at, col, warning, message) = if let Some(c) = forms[0].captures(line) {
            (c[1].parse().unwrap_or(1), 0, &c[2] == "warning", c[3].to_owned())
        } else if let Some(c) = forms[1].captures(line) {
            (
                c[1].parse().unwrap_or(1),
                c[2].parse::<u32>().unwrap_or(1).saturating_sub(1),
                &c[3] == "warning",
                c[4].to_owned(),
            )
        } else if let Some(c) = forms[2].captures(line) {
            (c[2].parse().unwrap_or(1), 0, &c[1] == "WARNING", c[3].to_owned())
        } else {
            continue;
        };
        let line = prepared.file_line(at);
        let mut p = problem(line, col, 1, message.trim().to_owned());
        // Without a column, the whole line.
        if col == 0 {
            p.end_col = 999;
        }
        if warning {
            p.severity = Severity::Warning;
        }
        out.push(p);
    }
    // A log with no line in a known form still says something.
    if out.is_empty() && log.to_ascii_lowercase().contains("error") {
        let first = log.lines().find(|l| !l.trim().is_empty()).unwrap_or("error").trim().to_owned();
        out.push(problem(0, 0, 999, first));
    }
    out
}

/// Syntax definitions for GLSL, HLSL and WGSL (syntect's own set has none).
pub const SYNTAXES: [&str; 3] = [GLSL_SYNTAX, HLSL_SYNTAX, WGSL_SYNTAX];

const GLSL_SYNTAX: &str = r#"%YAML 1.2
---
name: GLSL
file_extensions: [glsl, vert, frag, geom, comp, tesc, tese, vs, fs, gs, vsh, fsh, shader]
scope: source.glsl
contexts:
  main:
    - include: common
    - match: '^\s*#\s*\w+'
      scope: keyword.control.directive.glsl
    - match: '\b(if|else|for|while|do|switch|case|default|break|continue|return|discard)\b'
      scope: keyword.control.glsl
    - match: '\b(uniform|in|out|inout|attribute|varying|const|layout|precision|highp|mediump|lowp|flat|smooth|noperspective|centroid|invariant|precise|buffer|shared|coherent|volatile|restrict|readonly|writeonly|subroutine|patch|sample)\b'
      scope: storage.modifier.glsl
    - match: '\b(struct)\b'
      scope: storage.type.struct.glsl
    - match: '\b(void|bool|int|uint|float|double|[biud]?vec[234]|d?mat[234](x[234])?|[iu]?sampler\w+|[iu]?image\w+|atomic_uint)\b'
      scope: storage.type.glsl
    - match: '\b(gl_\w+)\b'
      scope: variable.language.glsl
    - match: '\b(radians|degrees|sin|cos|tan|asin|acos|atan|sinh|cosh|tanh|pow|exp|log|exp2|log2|sqrt|inversesqrt|abs|sign|floor|trunc|round|ceil|fract|mod|modf|min|max|clamp|mix|step|smoothstep|isnan|isinf|fma|length|distance|dot|cross|normalize|reflect|refract|faceforward|transpose|determinant|inverse|lessThan|greaterThan|equal|notEqual|any|all|not|texture\w*|texelFetch\w*|textureSize|dFdx|dFdy|fwidth|imageLoad|imageStore|barrier|memoryBarrier\w*|emitVertex|endPrimitive)\b(?=\s*\()'
      scope: support.function.glsl
    - match: '\b([A-Za-z_]\w*)\s*(?=\()'
      captures:
        1: entity.name.function.glsl
  common:
    - match: '//'
      scope: punctuation.definition.comment.glsl
      push: line_comment
    - match: '/\*'
      scope: punctuation.definition.comment.glsl
      push: block_comment
    - match: '\b(true|false)\b'
      scope: constant.language.glsl
    - match: '\b(0[xX][0-9a-fA-F]+[uU]?|\d+\.?\d*([eE][-+]?\d+)?[fFuUhH]?|\.\d+([eE][-+]?\d+)?[fF]?)\b'
      scope: constant.numeric.glsl
  line_comment:
    - meta_scope: comment.line.double-slash.glsl
    - match: '$'
      pop: true
  block_comment:
    - meta_scope: comment.block.glsl
    - match: '\*/'
      pop: true
"#;

const HLSL_SYNTAX: &str = r#"%YAML 1.2
---
name: HLSL
file_extensions: [hlsl, hlsli, fx, fxh]
scope: source.hlsl
contexts:
  main:
    - match: '//'
      push: line_comment
    - match: '/\*'
      push: block_comment
    - match: '"'
      push: string
    - match: '^\s*#\s*\w+'
      scope: keyword.control.directive.hlsl
    - match: '\b(if|else|for|while|do|switch|case|default|break|continue|return|discard|clip)\b'
      scope: keyword.control.hlsl
    - match: '\b(cbuffer|tbuffer|struct|typedef|register|packoffset|static|const|uniform|extern|groupshared|in|out|inout|inline|nointerpolation|linear|centroid|noperspective|sample|precise|row_major|column_major|technique\d*|pass)\b'
      scope: storage.modifier.hlsl
    - match: '\b(void|bool|int|uint|dword|half|float|double|min16float|min16int|min16uint|(bool|int|uint|half|float|double|min16float)[1-4](x[1-4])?|matrix|vector|SamplerState|SamplerComparisonState|(RW)?Texture\w*|(RW)?(Structured|ByteAddress)?Buffer|ConstantBuffer|AppendStructuredBuffer|ConsumeStructuredBuffer)\b'
      scope: storage.type.hlsl
    - match: ':\s*(SV_\w+|POSITION\d*|NORMAL\d*|TEXCOORD\d*|COLOR\d*|TANGENT\d*|BINORMAL\d*)\b'
      captures:
        1: variable.language.semantic.hlsl
    - match: '\b(true|false)\b'
      scope: constant.language.hlsl
    - match: '\b(0[xX][0-9a-fA-F]+[uU]?|\d+\.?\d*([eE][-+]?\d+)?[fFhHuUlL]?|\.\d+([eE][-+]?\d+)?[fFhH]?)\b'
      scope: constant.numeric.hlsl
    - match: '\b(abs|acos|all|any|asin|atan2?|ceil|clamp|cos|cosh|cross|ddx|ddy|degrees|determinant|distance|dot|exp2?|floor|fmod|frac|fwidth|length|lerp|log2?|log10|max|min|mul|normalize|pow|radians|reflect|refract|round|rsqrt|saturate|sign|sin|sincos|sinh|smoothstep|sqrt|step|tan|tanh|transpose|trunc|Sample\w*|Load|GetDimensions)\b(?=\s*\()'
      scope: support.function.hlsl
    - match: '\b([A-Za-z_]\w*)\s*(?=\()'
      captures:
        1: entity.name.function.hlsl
  line_comment:
    - meta_scope: comment.line.double-slash.hlsl
    - match: '$'
      pop: true
  block_comment:
    - meta_scope: comment.block.hlsl
    - match: '\*/'
      pop: true
  string:
    - meta_scope: string.quoted.double.hlsl
    - match: '"'
      pop: true
"#;

const WGSL_SYNTAX: &str = r#"%YAML 1.2
---
name: WGSL
file_extensions: [wgsl]
scope: source.wgsl
contexts:
  main:
    - match: '//'
      push: line_comment
    - match: '/\*'
      push: block_comment
    - match: '@\s*\w+'
      scope: entity.other.attribute-name.wgsl
    - match: '\b(if|else|loop|for|while|break|continue|continuing|return|switch|case|default|discard)\b'
      scope: keyword.control.wgsl
    - match: '\b(fn|let|var|const|override|struct|alias|enable|requires|diagnostic)\b'
      scope: storage.type.wgsl
    - match: '\b(uniform|storage|read|write|read_write|function|private|workgroup|push_constant)\b'
      scope: storage.modifier.wgsl
    - match: '\b(bool|i32|u32|f32|f16|vec[234][ihuf]?|mat[234]x[234][hf]?|array|atomic|ptr|sampler|sampler_comparison|texture_\w+)\b'
      scope: support.type.wgsl
    - match: '\b(true|false)\b'
      scope: constant.language.wgsl
    - match: '\b(0[xX][0-9a-fA-F]+[iu]?|\d+\.?\d*([eE][-+]?\d+)?[fhiu]?|\.\d+([eE][-+]?\d+)?[fh]?)\b'
      scope: constant.numeric.wgsl
    - match: '\b(abs|acos|all|any|asin|atan2?|ceil|clamp|cos|cosh|cross|degrees|determinant|distance|dot|exp2?|faceForward|floor|fma|fract|length|log2?|max|min|mix|normalize|pow|radians|reflect|refract|round|saturate|select|sign|sin|sinh|smoothstep|sqrt|step|tan|tanh|transpose|trunc|textureSample\w*|textureLoad|textureStore|textureDimensions|dpdx|dpdy|fwidth|arrayLength|workgroupBarrier|storageBarrier)\b(?=\s*\()'
      scope: support.function.wgsl
    - match: '\bfn\s+([A-Za-z_]\w*)'
      captures:
        1: entity.name.function.wgsl
  line_comment:
    - meta_scope: comment.line.double-slash.wgsl
    - match: '$'
      pop: true
  block_comment:
    - meta_scope: comment.block.wgsl
    - match: '\*/'
      pop: true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind("frag"), Some(Kind::Glsl(Stage::Fragment)));
        assert_eq!(kind("VERT"), Some(Kind::Glsl(Stage::Vertex)));
        assert_eq!(kind("wgsl"), Some(Kind::Wgsl));
        assert_eq!(kind("hlsl"), Some(Kind::Hlsl));
        assert_eq!(kind("rs"), None);
        assert!(previews("glsl") && previews("wgsl") && !previews("vert") && !previews("hlsl"));
    }

    #[test]
    fn wgsl_problems_and_glsl() {
        let good = format!(
            "{WGSL_UNIFORMS}\n@fragment\nfn main(@builtin(position) p: vec4f) -> @location(0) vec4f {{\n  \
             return vec4f(p.xy / coiman.resolution, sin(coiman.time), 1.0);\n}}\n"
        );
        assert_eq!(check_wgsl(&good), []);
        let glsl = wgsl_to_glsl(&good).unwrap();
        assert!(glsl.starts_with("#version 330 core"), "{glsl}");
        assert!(glsl.contains("gl_FragCoord") && glsl.contains("uniform"), "{glsl}");

        // A syntax error: where it is.
        let bad = "fn f() -> f32 {\n  let x = 1.0\n  return x;\n}\n";
        let p = check_wgsl(bad);
        assert_eq!(p.len(), 1);
        assert!(p[0].line == 1 || p[0].line == 2, "{p:?}");
        // A type error the validator finds: an f32 returned as an i32.
        let wrong = "fn f() -> i32 {\n  return 1.5;\n}\n";
        let p = check_wgsl(wrong);
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0].line, 1, "{p:?}");
        // Nothing to preview without a fragment function.
        assert!(wgsl_to_glsl("fn f() {}\n").unwrap_err()[0].message.contains("@fragment"));
    }

    #[test]
    fn shadertoy_and_book_of_shaders() {
        let toy = "void mainImage(out vec4 c, in vec2 p) {\n  c = vec4(p / iResolution.xy, 0.5 + 0.5 * sin(iTime), 1.0);\n}\n";
        let p = prepare_fragment(toy);
        assert!(p.source.starts_with("#version 330 core\n"));
        assert!(p.source.contains("uniform float iTime;") && p.source.contains("out vec4 coiman_color;"));
        assert!(p.source.trim_end().ends_with("mainImage(coiman_color, gl_FragCoord.xy); }"));
        assert!(p.vertex.contains("in vec2 coiman_pos"));
        // The driver's line 1 + added is the file's first line (0-based 0).
        let added = p.source.lines().position(|l| l.starts_with("void mainImage")).unwrap() as u32;
        assert_eq!(p.file_line(added + 2), 1);

        let book = "#ifdef GL_ES\nprecision mediump float;\n#endif\nuniform vec2 u_resolution;\nvoid main() {\n  gl_FragColor = vec4(gl_FragCoord.xy / u_resolution, 0.0, 1.0);\n}\n";
        let p = prepare_fragment(book);
        assert!(p.source.starts_with("#version 120\n"));
        assert_eq!(p.source.matches("uniform vec2 u_resolution;").count(), 1, "not declared twice");
        assert!(!p.source.contains("coiman_color") && p.vertex.contains("attribute"));

        // With its own #version: the header goes after it, and lines map around it.
        let own = "#version 330 core\nout vec4 o;\nvoid main() {\n  o = vec4(iTime);\n}\n";
        let p = prepare_fragment(own);
        assert!(p.source.starts_with("#version 330 core\nuniform float iTime;"));
        assert_eq!(p.file_line(1), 0);
        let body = p.source.lines().position(|l| l.contains("o = vec4")).unwrap() as u32 + 1;
        assert_eq!(p.file_line(body), 3);
    }

    #[test]
    fn driver_logs() {
        let p = prepare_plain("a\nb\nc\n");
        let nvidia = "0(3) : error C1008: undefined variable \"x\"\n0(2) : warning C7533: global variable gl_FragColor is deprecated";
        let found = parse_driver_log(nvidia, &p);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].message.as_str()), (2, "undefined variable \"x\""));
        assert_eq!(found[1].severity, Severity::Warning);
        let mesa = "0:2(10): error: `x' undeclared";
        assert_eq!(parse_driver_log(mesa, &p)[0].line, 1);
        assert_eq!(parse_driver_log(mesa, &p)[0].col, 9);
        let amd = "ERROR: 0:3: 'x' : undeclared identifier\nERROR: 1 compilation errors.  No code generated.";
        let found = parse_driver_log(amd, &p);
        assert_eq!((found.len(), found[0].line), (1, 2));
        assert_eq!(parse_driver_log("Some error happened", &p)[0].message, "Some error happened");
        assert_eq!(parse_driver_log("", &p), []);
    }

    #[test]
    fn syntaxes_load() {
        for s in SYNTAXES {
            syntect::parsing::SyntaxDefinition::load_from_str(s, true, None).unwrap();
        }
    }
}
