//! Problems in HTML and XML, found while typing with no language server: tags left
//! open, closed in the wrong order or never opened, broken tags and repeated attributes.
//! HTML is read with tree-sitter, which knows the tags HTML closes by itself; XML
//! with quick-xml, which stops at the first error as XML does.

use tree_sitter::{Node, Parser};

/// Larger files aren't worth holding up for.
const MAX_SIZE: usize = 2 * 1024 * 1024;

/// Elements that never have content or an end tag.
const VOID: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Elements whose end tag HTML lets you leave out: the next one, or the parent's end, closes them.
const OPTIONAL_END: &[&str] = &[
    "html", "head", "body", "li", "dt", "dd", "p", "rt", "rp", "optgroup", "option", "colgroup", "caption", "thead",
    "tbody", "tfoot", "tr", "td", "th",
];

const XML_EXTENSIONS: &[&str] =
    &["xml", "svg", "xsd", "xsl", "xslt", "xaml", "csproj", "fsproj", "vbproj", "props", "targets", "plist", "resx"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    /// Allowed, but worth knowing.
    Information,
}

/// A place in a file: 0-based lines, columns in UTF-16 units as language servers count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub message: String,
    pub severity: Severity,
}

pub fn checks(extension: &str) -> bool {
    is_html(extension) || XML_EXTENSIONS.contains(&extension)
}

fn is_html(extension: &str) -> bool {
    matches!(extension, "html" | "htm" | "xhtml")
}

pub fn check(extension: &str, source: &str) -> Vec<Problem> {
    if source.len() > MAX_SIZE {
        return Vec::new();
    }
    if is_html(extension) {
        check_html(source)
    } else if XML_EXTENSIONS.contains(&extension) {
        check_xml(source)
    } else {
        Vec::new()
    }
}

/// The UTF-16 column of a byte offset into its line.
fn utf16_col(source: &str, row: usize, byte_col: usize) -> u32 {
    let line = source.lines().nth(row).unwrap_or("");
    let end = byte_col.min(line.len());
    let end = (0..=end).rev().find(|&i| line.is_char_boundary(i)).unwrap_or(0);
    line[..end].encode_utf16().count() as u32
}

fn problem(source: &str, node: Node, message: String, severity: Severity) -> Problem {
    let (start, end) = (node.start_position(), node.end_position());
    // A problem's mark stays on its first line, however far the node runs.
    let (end_row, end_col) = if end.row == start.row {
        (end.row, end.column)
    } else {
        (start.row, source.lines().nth(start.row).map_or(start.column, str::len))
    };
    Problem {
        line: start.row as u32,
        col: utf16_col(source, start.row, start.column),
        end_line: end_row as u32,
        end_col: utf16_col(source, end_row, end_col).max(utf16_col(source, start.row, start.column) + 1),
        message,
        severity,
    }
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// The name of a start tag, end tag or erroneous end tag, in lower case.
fn tag_name(node: Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|c| matches!(c.kind(), "tag_name" | "erroneous_end_tag_name"))
        .map(|c| text(c, source).to_ascii_lowercase())
}

fn check_html(source: &str) -> Vec<Problem> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_html::LANGUAGE.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else { return Vec::new() };
    let mut out = Vec::new();
    walk(tree.root_node(), source, &mut out);
    out.sort_by_key(|p| (p.line, p.col));
    out
}

fn walk(node: Node, source: &str, out: &mut Vec<Problem>) {
    match node.kind() {
        "ERROR" => {
            broken(node, source, out);
            return;
        }
        "element" => element(node, source, out),
        "erroneous_end_tag" => {
            let name = tag_name(node, source).unwrap_or_default();
            out.push(problem(
                source,
                node,
                format!("`</{name}>` closes nothing that's open here: is it in the wrong order, or extra?"),
                Severity::Error,
            ));
        }
        "start_tag" | "self_closing_tag" => attributes(node, source, out),
        _ => {}
    }
    if node.is_missing() {
        out.push(problem(source, node, format!("Missing `{}`", node.kind()), Severity::Error));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

fn element(node: Node, source: &str, out: &mut Vec<Problem>) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let Some(first) = children.first() else { return };
    let Some(name) = tag_name(*first, source) else { return };
    if first.kind() == "self_closing_tag" {
        if !VOID.contains(&name.as_str()) && !name.contains('-') && !is_foreign(&name) {
            out.push(problem(
                source,
                *first,
                format!("`<{name}/>` isn't closed by its `/` in HTML: it opens a `<{name}>` that needs a `</{name}>`"),
                Severity::Warning,
            ));
        }
        return;
    }
    if first.kind() != "start_tag" || VOID.contains(&name.as_str()) {
        return;
    }
    if children.iter().any(|c| c.kind() == "end_tag") {
        return;
    }
    if OPTIONAL_END.contains(&name.as_str()) {
        out.push(problem(
            source,
            *first,
            format!(
                "`<{name}>` has no `</{name}>`. HTML allows that, closing it at the next tag that can't be inside it"
            ),
            Severity::Information,
        ));
    } else {
        out.push(problem(source, *first, format!("`<{name}>` is never closed: add `</{name}>`"), Severity::Error));
    }
}

/// SVG and MathML inside HTML, where `<x/>` does close.
fn is_foreign(name: &str) -> bool {
    matches!(
        name,
        "svg" | "math" | "path" | "circle" | "rect" | "line" | "polyline" | "polygon" | "ellipse" | "use" | "stop"
    )
}

fn attributes(tag: Node, source: &str, out: &mut Vec<Problem>) {
    let mut seen: Vec<String> = Vec::new();
    let mut cursor = tag.walk();
    for attribute in tag.named_children(&mut cursor).filter(|c| c.kind() == "attribute") {
        let mut inner = attribute.walk();
        let Some(name) = attribute.named_children(&mut inner).find(|c| c.kind() == "attribute_name") else {
            continue;
        };
        let lower = text(name, source).to_ascii_lowercase();
        if seen.contains(&lower) {
            out.push(problem(
                source,
                name,
                format!("`{lower}` is set twice on this tag: only the first counts"),
                Severity::Warning,
            ));
        } else {
            seen.push(lower);
        }
    }
}

/// Where a part tree-sitter couldn't read went wrong. The error often starts far
/// before the cause, so the first piece of a broken tag is what gets marked.
fn broken(node: Node, source: &str, out: &mut Vec<Problem>) {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (i, child) in children.iter().enumerate() {
        match child.kind() {
            // Well-formed parts inside it still have their own problems.
            "element" | "erroneous_end_tag" => walk(*child, source, out),
            "tag_name" => {
                let name = text(*child, source).to_ascii_lowercase();
                let unclosed_quote = children[i + 1..].iter().any(|c| {
                    c.kind() == "attribute_value" && text(*c, source).contains('\n')
                        || matches!(c.kind(), "\"" | "'") && c.end_position().row != child.start_position().row
                });
                let message = if unclosed_quote {
                    format!("`<{name}` has an attribute whose quote is never closed")
                } else {
                    format!("`<{name}` isn't a complete tag: check its `>` and quotes")
                };
                out.push(problem(source, *child, message, Severity::Error));
                return;
            }
            "ERROR" => {
                broken(*child, source, out);
                return;
            }
            _ => {}
        }
    }
    out.push(problem(source, node, "This markup can't be read".to_owned(), Severity::Error));
}

fn check_xml(source: &str) -> Vec<Problem> {
    use quick_xml::Reader;
    use quick_xml::events::Event;
    let mut reader = Reader::from_str(source);
    reader.config_mut().check_end_names = true;
    let mut open: Vec<(String, usize)> = Vec::new();
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(tag)) => open.push((String::from_utf8_lossy(tag.name().as_ref()).into_owned(), before)),
            Ok(Event::End(_)) => {
                open.pop();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                let at = (reader.error_position() as usize).min(source.len());
                return vec![xml_problem(source, at, error.to_string())];
            }
        }
    }
    open.last().map(|(name, at)| xml_problem(source, *at, format!("`<{name}>` is never closed"))).into_iter().collect()
}

fn xml_problem(source: &str, at: usize, message: String) -> Problem {
    let at = (0..=at).rev().find(|&i| source.is_char_boundary(i)).unwrap_or(0);
    let before = &source[..at];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = utf16_col(source, line, at - line_start);
    Problem { line: line as u32, col, end_line: line as u32, end_col: col + 1, message, severity: Severity::Error }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(extension: &str, source: &str) -> Vec<(u32, Severity, String)> {
        check(extension, source).into_iter().map(|p| (p.line + 1, p.severity, p.message)).collect()
    }

    #[test]
    fn open_misnested_and_stray_tags() {
        let page = "<!doctype html>\n<html>\n<body>\n<ul>\n  <li>one\n  <li>two</li>\n</ul>\n<div><span></div></span>\n<p>unclosed\n<section>\n<b>bold\n</section>\n<img src=\"a.png\">\n<br/>\n</body>\n</html>\n";
        let found = found("html", page);
        let lines: Vec<(u32, Severity)> = found.iter().map(|(l, s, _)| (*l, *s)).collect();
        assert!(lines.contains(&(5, Severity::Information)), "{found:#?}");
        assert!(lines.contains(&(8, Severity::Error)), "{found:#?}");
        assert!(found.iter().any(|(l, _, m)| *l == 8 && m.contains("`<span>` is never closed")), "{found:#?}");
        assert!(found.iter().any(|(l, _, m)| *l == 8 && m.contains("`</span>` closes nothing")), "{found:#?}");
        assert!(found.iter().any(|(l, s, m)| *l == 9 && *s == Severity::Information && m.contains("<p>")));
        assert!(found.iter().any(|(l, _, m)| *l == 11 && m.contains("`<b>` is never closed")), "{found:#?}");
        // Void elements need no end, and say nothing.
        assert!(!found.iter().any(|(_, _, m)| m.contains("img") || m.contains("br")), "{found:#?}");
    }

    #[test]
    fn a_broken_tag_is_marked_where_it_breaks() {
        let page = "<html>\n<body>\n<p>fine</p>\n<div class=\"x>\n</body>\n</html>\n";
        let found = found("html", page);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].0, 4);
        assert!(found[0].2.contains("`<div`"), "{found:#?}");
    }

    #[test]
    fn repeated_attributes_and_self_closing_divs() {
        let found = found("html", "<div class=\"a\" CLASS=\"b\"></div>\n<div/>\n<my-widget/>\n<svg><path/></svg>\n");
        assert!(found.iter().any(|(l, s, m)| *l == 1 && *s == Severity::Warning && m.contains("`class` is set twice")));
        assert!(found.iter().any(|(l, _, m)| *l == 2 && m.contains("`<div/>`")), "{found:#?}");
        assert_eq!(found.len(), 2, "{found:#?}");
    }

    #[test]
    fn a_good_page_has_nothing_to_say() {
        let page = "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>t</title>\n<style>p > a { color: red }</style></head>\n<body>\n<p>Hi <a href=\"#\">there</a></p>\n<script>if (a < b && c > d) {}</script>\n<!-- <div> in a comment -->\n</body>\n</html>\n";
        assert_eq!(found("html", page), []);
    }

    #[test]
    fn columns_count_utf16_after_wide_text() {
        let problems = check("html", "<p>日本 <b>x</p>\n");
        let bold = problems.iter().find(|p| p.message.contains("<b>")).expect("the open <b>");
        assert_eq!(bold.col, 6);
    }

    #[test]
    fn xml_stops_at_its_first_error() {
        assert_eq!(found("xml", "<a><b></b></a>\n"), []);
        let wrong = found("xml", "<a>\n  <b></c>\n</a>\n");
        assert_eq!(wrong.len(), 1);
        assert_eq!(wrong[0].0, 2);
        let open = found("csproj", "<Project>\n  <ItemGroup>\n");
        assert!(open[0].2.contains("never closed"), "{open:?}");
        assert!(!checks("md"));
        assert!(checks("svg") && checks("htm"));
    }
}
