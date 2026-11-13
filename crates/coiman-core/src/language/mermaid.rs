//! Mermaid diagrams drawn without a browser, through merman: flowcharts, class, sequence, state
//! and ER diagrams, and the other kinds Mermaid knows. Text is measured with the fonts on this
//! computer, the same ones the picture is painted with, so labels fit their boxes.

use std::sync::OnceLock;

use merman::{OperationControl, PngRequest, RenderOutput, RenderRequest, Renderer, SvgRequest};
use merman_export::{NativeFontContext, NativeFontOptions, RasterOptions};
use merman_render::resources::{RenderResourcePolicy, ResourceLimitId};

/// How much layout work a diagram may take. merman's interactive budget (14.1 million units)
/// stops at a project's folder graph of a few dozen folders; these diagrams are the user's own
/// or made here, so they get ten times that, still with a ceiling.
const LAYOUT_WORK: usize = 150_000_000;

/// Light or dark: Mermaid's `default` or `dark` theme, unless the diagram picks its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Look {
    Light,
    Dark,
}

/// Families Mermaid's themes name (`"trebuchet ms", verdana, arial, sans-serif`), then fallbacks
/// for other systems and for symbols. Only these are read: all of a system's fonts can be
/// hundreds of megabytes.
const FAMILIES: &[&str] = &[
    "Trebuchet MS",
    "Verdana",
    "Arial",
    "Helvetica",
    "Segoe UI",
    "DejaVu Sans",
    "Liberation Sans",
    "Noto Sans",
    "Segoe UI Symbol",
];

/// The font files of [`FAMILIES`] found on this computer.
fn font_files() -> Vec<std::path::PathBuf> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut files = Vec::new();
    for face in db.faces() {
        let named = face.families.iter().any(|(name, _)| FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(name)));
        if let (true, fontdb::Source::File(path)) = (named, &face.source)
            && !files.contains(path)
        {
            files.push(path.clone());
        }
    }
    files
}

/// The fonts, read once.
fn fonts() -> Result<&'static NativeFontContext, String> {
    static FONTS: OnceLock<Result<NativeFontContext, String>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let files = font_files();
            // None of the usual ones: whatever the system has.
            let options = NativeFontOptions::default().with_system_fonts(files.is_empty());
            let options = files.into_iter().fold(options, NativeFontOptions::with_font_file);
            options.load(&OperationControl::new()).map_err(|e| format!("no fonts to draw with: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The kind of diagram, from its first word (`flowchart`, `classDiagram`...), skipping
/// front matter, directives and comments.
pub fn kind(source: &str) -> Option<&str> {
    let mut lines = source.lines().map(str::trim).filter(|l| !l.is_empty());
    let mut line = lines.next()?;
    if line == "---" {
        // YAML front matter, up to its closing line.
        lines.by_ref().find(|l| *l == "---")?;
        line = lines.next()?;
    }
    while line.starts_with("%%") {
        line = lines.next()?;
    }
    line.split_whitespace().next()
}

/// `source` with Mermaid's theme for `look`, unless it sets one itself (`%%{init: ...}%%`
/// or a `config:` in front matter).
pub fn themed(source: &str, look: Look) -> String {
    let own = source.contains("%%{") || (source.trim_start().starts_with("---") && source.contains("theme"));
    if own {
        return source.to_owned();
    }
    let theme = match look {
        Look::Light => "default",
        Look::Dark => "dark",
    };
    format!("%%{{init: {{\"theme\": \"{theme}\"}}}}%%\n{source}")
}

/// The diagram as a PNG, `scale` times its natural size, on a transparent background.
pub fn png(source: &str, look: Look, scale: f32) -> Result<Vec<u8>, String> {
    if kind(source).is_none() {
        return Err("the diagram is empty".to_owned());
    }
    let fonts = fonts()?;
    let control = OperationControl::new();
    let mut svg = SvgRequest::default();
    svg.environment = svg.environment.with_native_font_context(fonts.clone()).map_err(|e| e.to_string())?;
    let budget = RenderResourcePolicy::trusted_native()
        .with_limit(ResourceLimitId::MaxLayoutWorkUnits, LAYOUT_WORK)
        .map_err(|e| e.to_string())?;
    svg.environment = svg.environment.with_resource_policy(budget);
    let options =
        RasterOptions { scale: scale.clamp(0.25, 4.0), font_context: Some(fonts.clone()), ..RasterOptions::default() };
    let source = themed(source, look);
    let request = RenderRequest::png(&source, control, PngRequest { svg, options });
    match Renderer::new().render(request).map_err(|e| first_line(&e.to_string()))? {
        RenderOutput::Png(Some(out)) => Ok(out.bytes),
        _ => Err("this isn't a Mermaid diagram".to_owned()),
    }
}

/// Error messages can carry the whole input after them; the first line says what's wrong.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(png: &[u8]) -> (u32, u32) {
        // Width and height are big-endian at bytes 16..24 of the IHDR chunk.
        let n = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
        (n(16), n(20))
    }

    #[test]
    fn finds_the_kind() {
        assert_eq!(kind("flowchart LR\n A-->B"), Some("flowchart"));
        assert_eq!(kind("%% a comment\n\nclassDiagram\n class A"), Some("classDiagram"));
        assert_eq!(kind("---\ntitle: x\n---\nsequenceDiagram\n A->>B: hi"), Some("sequenceDiagram"));
        assert_eq!(kind("  \n "), None);
    }

    #[test]
    fn picks_a_theme_unless_the_diagram_does() {
        assert!(themed("graph TD\nA-->B", Look::Dark).starts_with("%%{init: {\"theme\": \"dark\"}}%%\n"));
        assert!(themed("graph TD\nA-->B", Look::Light).contains("\"default\""));
        let own = "%%{init: {\"theme\": \"forest\"}}%%\ngraph TD\nA-->B";
        assert_eq!(themed(own, Look::Dark), own);
    }

    #[test]
    fn draws_each_kind() {
        let diagrams = [
            "flowchart LR\n  A[Start] --> B{Valid?}\n  B -- Yes --> C(Save)\n  B -- No --> D[[Error]]",
            "classDiagram\n  class Shape {\n    <<interface>>\n    +area() double\n  }\n  Shape <|.. Circle",
            "sequenceDiagram\n  actor User\n  User->>App: Open\n  App-->>User: Done",
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running : start\n  Running --> [*]",
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  CUSTOMER {\n    int id PK\n  }",
        ];
        for d in diagrams {
            let png = png(d, Look::Light, 1.0).unwrap_or_else(|e| panic!("{d}: {e}"));
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            let (w, h) = size(&png);
            assert!(w > 50 && h > 50, "{d}: {w}x{h}");
            // Twice the scale, twice the pixels.
            let (w2, _) = size(&png_twice(d));
            assert!((w2 as i64 - 2 * w as i64).abs() <= 2, "{d}: {w} then {w2}");
        }
    }

    fn png_twice(d: &str) -> Vec<u8> {
        png(d, Look::Dark, 2.0).unwrap()
    }

    #[test]
    fn says_what_is_wrong() {
        let e = png("flowchart LR\n  A --> ", Look::Light, 1.0).unwrap_err();
        assert!(!e.is_empty() && !e.contains('\n'), "{e}");
        assert!(png("", Look::Light, 1.0).unwrap_err().contains("empty"));
        assert!(png("notADiagram foo", Look::Light, 1.0).is_err());
    }
}
