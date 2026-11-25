//! Shaders: GLSL, HLSL and WGSL files, by kind, and their syntax colors. Checking and the live
//! preview are the Shaders extension's (Graphics pack), through the editor's OpenGL.

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
    }

    #[test]
    fn syntaxes_load() {
        for s in SYNTAXES {
            syntect::parsing::SyntaxDefinition::load_from_str(s, true, None).unwrap();
        }
    }
}
