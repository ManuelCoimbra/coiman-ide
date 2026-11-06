//! Problems in HTML and XML, found while typing with no language server: tags left
//! open, closed in the wrong order or never opened, broken tags, repeated attributes,
//! and attributes HTML doesn't have. HTML is read with tree-sitter, which knows the tags
//! HTML closes by itself, after template tags (Jinja, Handlebars, EJS, PHP) are blanked
//! out. XML is read with quick-xml, matching tags here so that one mistake doesn't hide
//! the rest.

use tree_sitter::{Node, Parser};

use crate::html_attributes::{self, VOID, Verdict};

/// Larger files aren't worth holding up for.
const MAX_SIZE: usize = 2 * 1024 * 1024;
/// Past this many, more problems only bury the first ones.
const MAX_PROBLEMS: usize = 200;

/// Elements whose end tag HTML lets you leave out: the next one, or the parent's end, closes them.
const OPTIONAL_END: &[&str] = &[
    "html", "head", "body", "li", "dt", "dd", "p", "rt", "rp", "optgroup", "option", "colgroup", "caption", "thead",
    "tbody", "tfoot", "tr", "td", "th",
];

/// HTML, and HTML templates whose own tags are blanked out before reading.
const HTML_EXTENSIONS: &[&str] =
    &["html", "htm", "xhtml", "jinja", "jinja2", "j2", "njk", "hbs", "handlebars", "mustache", "ejs", "twig", "liquid"];

const XML_EXTENSIONS: &[&str] =
    &["xml", "svg", "xsd", "xsl", "xslt", "xaml", "csproj", "fsproj", "vbproj", "props", "targets", "plist", "resx"];

/// Template tags: how each opens and closes.
const TEMPLATE_TAGS: &[(&str, &str)] = &[("{%", "%}"), ("{{", "}}"), ("{#", "#}"), ("<%", "%>"), ("<?", "?>")];

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
    HTML_EXTENSIONS.contains(&extension) || XML_EXTENSIONS.contains(&extension) || matches!(extension, "vue" | "svelte")
}

pub fn check(extension: &str, source: &str) -> Vec<Problem> {
    if source.len() > MAX_SIZE {
        return Vec::new();
    }
    let mut found = if HTML_EXTENSIONS.contains(&extension) {
        check_html(source, Dialect::Html)
    } else if extension == "vue" {
        // A template in another language, such as Pug, isn't HTML to check.
        let pug = source.contains("<template lang=\"pug\"") || source.contains("<template lang='pug'");
        if pug { Vec::new() } else { check_html(source, Dialect::Vue) }
    } else if extension == "svelte" {
        check_html(source, Dialect::Svelte)
    } else if XML_EXTENSIONS.contains(&extension) {
        check_xml(source)
    } else {
        Vec::new()
    };
    found.truncate(MAX_PROBLEMS);
    found
}

/// The UTF-16 column of a byte offset into its line.
fn utf16_col(source: &str, row: usize, byte_col: usize) -> u32 {
    let line = source.lines().nth(row).unwrap_or("");
    let end = byte_col.min(line.len());
    let end = (0..=end).rev().find(|&i| line.is_char_boundary(i)).unwrap_or(0);
    line[..end].encode_utf16().count() as u32
}

/// Where the HTML comes from: a page, or a component's template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    Html,
    Vue,
    Svelte,
}

/// Svelte's `{...}`: blocks (`{#if}`, `{:else}`, `{/if}`), tags (`{@html}`) and
/// expressions, in text or as attribute values, blanked like template tags.
fn blank_svelte(source: &str) -> (String, bool) {
    let mut bytes = source.as_bytes().to_vec();
    let mut blocks = false;
    let (mut depth, mut start, mut quote) = (0usize, 0usize, None);
    for (i, c) in source.char_indices() {
        match (depth, quote, c) {
            (0, _, '{') => {
                depth = 1;
                start = i;
                blocks |= source[i + 1..].starts_with(['#', ':', '/']);
            }
            (0, ..) => {}
            (_, None, '"' | '\'' | '`') => quote = Some(c),
            (_, Some(q), _) if c == q => quote = None,
            (_, None, '{') => depth += 1,
            (_, None, '}') => {
                depth -= 1;
                if depth == 0 {
                    for b in &mut bytes[start..=i] {
                        if *b != b'\n' {
                            *b = b' ';
                        }
                    }
                    // As an attribute's value it stays a value, quoted, so the tag still reads.
                    if source[..start].ends_with('=') {
                        (bytes[start], bytes[i]) = (b'"', b'"');
                    }
                }
            }
            _ => {}
        }
    }
    (String::from_utf8(bytes).unwrap_or_else(|_| source.to_owned()), blocks)
}

/// Template tags replaced by spaces, byte for byte, so positions stay where they were.
/// Also says whether any are blocks (`{% if %}`, `{{#each}}`, `<% } %>`), which can
/// open and close HTML tags in branches.
fn blank_templates(source: &str) -> (String, bool) {
    let mut bytes = source.as_bytes().to_vec();
    let mut blocks = false;
    let mut at = 0;
    while at < source.len() {
        let next = TEMPLATE_TAGS
            .iter()
            .filter_map(|(open, close)| source[at..].find(open).map(|i| (at + i, *open, *close)))
            .min_by_key(|(i, ..)| *i);
        let Some((start, open, close)) = next else { break };
        let Some(length) = source[start + open.len()..].find(close) else { break };
        let end = start + open.len() + length + close.len();
        let inner = source[start + open.len()..end - close.len()].trim();
        blocks |= match open {
            "{%" => true,
            "{{" => inner.starts_with(['#', '/', '^']) || inner.starts_with("else"),
            "<%" | "<?" => inner.contains(['{', '}']) || inner.contains("endif") || inner.contains("endforeach"),
            _ => false,
        };
        for b in &mut bytes[start..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        at = end;
    }
    // Every replaced byte is ASCII, and whole characters were replaced.
    (String::from_utf8(bytes).unwrap_or_else(|_| source.to_owned()), blocks)
}

/// A page being read: the text tree-sitter saw, the text as written, and what it found.
struct Html<'a> {
    text: &'a str,
    raw: &'a str,
    blocks: bool,
    /// A Vue or Svelte component: any tag may close itself, and its blocks take the framework's attributes.
    vue: bool,
    out: Vec<Problem>,
}

impl Html<'_> {
    fn problem(&mut self, node: Node, message: String, severity: Severity) {
        let (start, end) = (node.start_position(), node.end_position());
        // A problem's mark stays on its first line, however far the node runs.
        let (end_row, end_col) = if end.row == start.row {
            (end.row, end.column)
        } else {
            (start.row, self.raw.lines().nth(start.row).map_or(start.column, str::len))
        };
        let col = utf16_col(self.raw, start.row, start.column);
        self.out.push(Problem {
            line: start.row as u32,
            col,
            end_line: end_row as u32,
            end_col: utf16_col(self.raw, end_row, end_col).max(col + 1),
            message,
            severity,
        });
    }

    /// A tag left open or closed out of order. Template branches can make a page that
    /// looks unbalanced come out right, so in a page with them it's only a note.
    fn balance(&mut self, node: Node, message: String) {
        if self.blocks {
            let message = format!("{message} (unless the template's branches balance it)");
            self.problem(node, message, Severity::Information);
        } else {
            self.problem(node, message, Severity::Error);
        }
    }

    fn text(&self, node: Node) -> &str {
        &self.text[node.byte_range()]
    }

    /// The name of a start tag, end tag or erroneous end tag, in lower case.
    fn tag_name(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|c| matches!(c.kind(), "tag_name" | "erroneous_end_tag_name"))
            .map(|c| self.text(c).to_ascii_lowercase())
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "ERROR" => {
                self.broken(node);
                return;
            }
            "element" => self.element(node),
            "erroneous_end_tag" => {
                let name = self.tag_name(node).unwrap_or_default();
                self.balance(
                    node,
                    format!("`</{name}>` closes nothing that's open here: is it in the wrong order, or extra?"),
                );
            }
            "start_tag" | "self_closing_tag" => self.attributes(node),
            _ => {}
        }
        if node.is_missing() {
            self.problem(node, format!("Missing `{}`", node.kind()), Severity::Error);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child);
        }
    }

    fn element(&mut self, node: Node) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        let Some(first) = children.first().copied() else { return };
        let Some(name) = self.tag_name(first) else { return };
        if first.kind() == "self_closing_tag" {
            if !self.vue && !VOID.contains(&name.as_str()) && !name.contains('-') && !is_foreign(&name) {
                self.problem(
                    first,
                    format!(
                        "`<{name}/>` isn't closed by its `/` in HTML: it opens a `<{name}>` that needs a `</{name}>`"
                    ),
                    Severity::Warning,
                );
            }
            return;
        }
        if first.kind() != "start_tag"
            || VOID.contains(&name.as_str())
            || children.iter().any(|c| c.kind() == "end_tag")
        {
            return;
        }
        if OPTIONAL_END.contains(&name.as_str()) {
            self.problem(
                first,
                format!("`<{name}>` has no `</{name}>`. HTML allows that, closing it at the next tag that can't be inside it"),
                Severity::Information,
            );
        } else {
            self.balance(first, format!("`<{name}>` is never closed: add `</{name}>`"));
        }
    }

    fn attributes(&mut self, tag: Node) {
        let element = self.tag_name(tag).unwrap_or_default();
        // A template inside the tag can choose between attributes, so twice may be once.
        let templated = TEMPLATE_TAGS.iter().any(|(open, _)| self.raw[tag.byte_range()].contains(open));
        let mut seen: Vec<String> = Vec::new();
        let mut cursor = tag.walk();
        let names: Vec<Node> = tag
            .named_children(&mut cursor)
            .filter(|c| c.kind() == "attribute")
            .filter_map(|attribute| {
                let mut inner = attribute.walk();
                attribute.named_children(&mut inner).find(|c| c.kind() == "attribute_name")
            })
            .collect();
        for name in names {
            let lower = self.text(name).to_ascii_lowercase();
            let verdict = if self.vue && vue_block_attribute(&element, &lower) {
                Verdict::Fine
            } else {
                html_attributes::verdict(&element, &lower)
            };
            match verdict {
                Verdict::Fine => {}
                Verdict::Obsolete => self.problem(
                    name,
                    format!("`{lower}` on `<{element}>` is obsolete in HTML: use CSS or the element's own attributes"),
                    Severity::Information,
                ),
                Verdict::Unknown => self.problem(
                    name,
                    format!("`<{element}>` has no `{lower}` attribute in HTML. For your own data, use `data-{lower}`"),
                    Severity::Warning,
                ),
            }
            if seen.contains(&lower) {
                if !templated {
                    self.problem(
                        name,
                        format!("`{lower}` is set twice on this tag: only the first counts"),
                        Severity::Warning,
                    );
                }
            } else {
                seen.push(lower);
            }
        }
    }

    /// Where a part tree-sitter couldn't read went wrong. The error often starts far
    /// before the cause, so the first piece of a broken tag is what gets marked.
    fn broken(&mut self, node: Node) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        // With no broken tag in it, tree-sitter wraps tags still open when the file
        // ends: each is a tag never closed.
        let broken_tag = children.iter().any(|c| matches!(c.kind(), "tag_name" | "ERROR"));
        if !broken_tag && children.iter().any(|c| c.kind() == "start_tag") {
            for child in children {
                match child.kind() {
                    "element" | "erroneous_end_tag" => self.walk(child),
                    "start_tag" => {
                        self.attributes(child);
                        let name = self.tag_name(child).unwrap_or_default();
                        if VOID.contains(&name.as_str()) {
                            continue;
                        }
                        if OPTIONAL_END.contains(&name.as_str()) {
                            self.problem(
                                child,
                                format!("`<{name}>` has no `</{name}>`. HTML allows that, closing it at the end"),
                                Severity::Information,
                            );
                        } else {
                            self.balance(child, format!("`<{name}>` is never closed: add `</{name}>`"));
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        for (i, child) in children.iter().enumerate() {
            match child.kind() {
                // Well-formed parts inside it still have their own problems.
                "element" | "erroneous_end_tag" => self.walk(*child),
                "tag_name" => {
                    let name = self.text(*child).to_ascii_lowercase();
                    let unclosed_quote = children[i + 1..].iter().any(|c| {
                        c.kind() == "attribute_value" && self.text(*c).contains('\n')
                            || matches!(c.kind(), "\"" | "'") && c.end_position().row != child.start_position().row
                    });
                    let message = if unclosed_quote {
                        format!("`<{name}` has an attribute whose quote is never closed")
                    } else {
                        format!("`<{name}` isn't a complete tag: check its `>` and quotes")
                    };
                    self.problem(*child, message, Severity::Error);
                    return;
                }
                "ERROR" => {
                    self.broken(*child);
                    return;
                }
                _ => {}
            }
        }
        self.problem(node, "This markup can't be read".to_owned(), Severity::Error);
    }
}

/// SVG and MathML inside HTML, where `<x/>` does close.
fn is_foreign(name: &str) -> bool {
    matches!(
        name,
        "svg" | "math" | "path" | "circle" | "rect" | "line" | "polyline" | "polygon" | "ellipse" | "use" | "stop"
    )
}

/// Attributes of a Vue single-file component's blocks: `<script setup>`, `<style scoped>`.
fn vue_block_attribute(element: &str, attribute: &str) -> bool {
    match element {
        "script" => matches!(attribute, "setup" | "lang" | "generic" | "src" | "context"),
        "style" => matches!(attribute, "scoped" | "module" | "lang" | "src"),
        "template" => matches!(attribute, "lang" | "functional" | "src"),
        _ => false,
    }
}

fn check_html(source: &str, dialect: Dialect) -> Vec<Problem> {
    let (text, blocks) = if dialect == Dialect::Svelte { blank_svelte(source) } else { blank_templates(source) };
    let vue = dialect != Dialect::Html;
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_html::LANGUAGE.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(&text, None) else { return Vec::new() };
    let mut page = Html { text: &text, raw: source, blocks, vue, out: Vec::new() };
    page.walk(tree.root_node());
    let mut out = page.out;
    out.sort_by_key(|p| (p.line, p.col));
    out
}

fn check_xml(source: &str) -> Vec<Problem> {
    use quick_xml::Reader;
    use quick_xml::events::{BytesStart, Event};
    let mut reader = Reader::from_str(source);
    // Tags are matched here, so that a wrong one is reported and reading goes on.
    reader.config_mut().check_end_names = false;
    reader.config_mut().allow_unmatched_ends = true;
    let mut out = Vec::new();
    let mut open: Vec<(String, usize)> = Vec::new();
    let duplicates = |tag: &BytesStart, at: usize, out: &mut Vec<Problem>| {
        use quick_xml::events::attributes::AttrError;
        let content: &[u8] = tag;
        for attribute in tag.attributes() {
            match attribute {
                Ok(_) => {}
                // Positions count from just after the tag's `<`.
                Err(AttrError::Duplicated(position, _)) => {
                    let rest = &content[position.min(content.len())..];
                    let end = rest.iter().position(|b| *b == b'=' || b.is_ascii_whitespace()).unwrap_or(rest.len());
                    let name = String::from_utf8_lossy(&rest[..end]).into_owned();
                    let start = at + source[at..].len() - source[at..].trim_start().len();
                    let message = format!("`{name}` is set twice on this tag, which XML doesn't allow");
                    out.push(xml_problem(source, start + 1 + position, message));
                }
                Err(error) => out.push(xml_problem(source, at, error.to_string())),
            }
        }
    };
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                duplicates(&tag, before, &mut out);
                open.push((String::from_utf8_lossy(tag.name().as_ref()).into_owned(), before));
            }
            Ok(Event::Empty(tag)) => duplicates(&tag, before, &mut out),
            Ok(Event::End(tag)) => {
                let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
                match open.iter().rposition(|(n, _)| *n == name) {
                    Some(at) => {
                        for (inner, start) in open.drain(at + 1..).rev() {
                            out.push(xml_problem(
                                source,
                                start,
                                format!("`<{inner}>` is never closed before `</{name}>`"),
                            ));
                        }
                        open.pop();
                    }
                    None => out.push(xml_problem(source, before, format!("`</{name}>` closes nothing that's open"))),
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                let at = (reader.error_position() as usize).min(source.len());
                out.push(xml_problem(source, at, error.to_string()));
                // Read on from past the error, unless it can't move.
                if reader.buffer_position() as usize <= before || out.len() >= MAX_PROBLEMS {
                    break;
                }
            }
        }
    }
    for (name, at) in open.into_iter().rev() {
        out.push(xml_problem(source, at, format!("`<{name}>` is never closed")));
    }
    out.sort_by_key(|p| (p.line, p.col));
    out
}

fn xml_problem(source: &str, at: usize, message: String) -> Problem {
    let at = (0..=at.min(source.len())).rev().find(|&i| source.is_char_boundary(i)).unwrap_or(0);
    // Past leading spaces, onto the tag itself.
    let at = at + source[at..].len() - source[at..].trim_start().len();
    let at = at.min(source.len());
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
    fn attribute_names_are_checked_against_htmls() {
        let page = "<div clas=\"a\" data-x=\"1\" aria-label=\"b\" onclick=\"f()\" v-if=\"ok\" :title=\"t\"></div>\n<a hreff=\"#\">x</a>\n<table bgcolor=\"red\"><tr><td>1</td></tr></table>\n<my-card anything=\"1\"></my-card>\n";
        let found = found("html", page);
        assert!(
            found.iter().any(|(l, s, m)| *l == 1 && *s == Severity::Warning && m.contains("no `clas`")),
            "{found:#?}"
        );
        assert!(found.iter().any(|(l, _, m)| *l == 2 && m.contains("no `hreff`")), "{found:#?}");
        assert!(found.iter().any(|(l, s, m)| *l == 3 && *s == Severity::Information && m.contains("obsolete")));
        assert_eq!(found.len(), 3, "{found:#?}");
    }

    #[test]
    fn template_tags_are_read_around() {
        // Expressions and statements inside tags and text break nothing.
        let jinja = "<ul>\n{% for item in items %}\n  <li class=\"{{ item.kind }}\" {% if item.on %}data-on{% endif %}>{{ item.name }}</li>\n{% endfor %}\n</ul>\n<p>{# a comment with <b> #}</p>\n";
        assert_eq!(found("jinja", jinja), []);
        assert_eq!(found("html", jinja), []);
        // Branches that each open a tag closed once are only noted.
        let branches = "{% if wide %}<div class=\"wide\">{% else %}<div>{% endif %}\n  text\n</div>\n";
        assert!(
            found("html", branches).iter().all(|(_, s, _)| *s == Severity::Information),
            "{:#?}",
            found("html", branches)
        );
        let handlebars = "{{#if user}}<p>Hi {{user.name}}</p>{{else}}<p>Sign in</p>{{/if}}\n";
        assert_eq!(found("hbs", handlebars), []);
        let ejs = "<% if (user) { %>\n<p><%= user.name %></p>\n<% } %>\n";
        assert_eq!(found("ejs", ejs), []);
        // Tags still open where a template ends are said to be, not "can't be read".
        let open = "{% block body %}
{% if wide %}<div class=\"wide\">{% else %}<div>{% endif %}
  text
</div>
<span>never closed
{% endblock %}
";
        let said = found("html", open);
        assert!(said.iter().any(|(l, _, m)| *l == 5 && m.contains("`<span>` is never closed")), "{said:#?}");
        assert!(!said.iter().any(|(_, _, m)| m.contains("can't be read")), "{said:#?}");
        let plain = found(
            "html",
            "<div>
<span>x
",
        );
        assert_eq!(plain.iter().filter(|(_, s, _)| *s == Severity::Error).count(), 2, "{plain:#?}");
        // A template's attributes may be chosen between, so "twice" isn't said.
        let either = "<div {% if a %}class=\"x\"{% else %}class=\"y\"{% endif %}></div>\n";
        assert_eq!(found("html", either), []);
        // Problems are still where they are in the file as written.
        let wrong = "<p>{{ \"日本\" }} <b>x</p>\n";
        let open = check("html", wrong).into_iter().find(|p| p.message.contains("<b>")).expect("the open <b>");
        assert_eq!(open.col, 14);
    }

    #[test]
    fn vue_templates_follow_vues_rules() {
        let sfc = "<template>\n  <div :class=\"c\" @click=\"go\" v-if=\"ok\">\n    <MyCard title=\"t\" />\n    <span>{{ a > b ? 1 : 2 }}</span>\n    <div />\n  </div>\n</template>\n<script setup lang=\"ts\">\nconst a = 1 < 2\n</script>\n<style scoped>\np > a { color: red }\n</style>\n";
        assert_eq!(found("vue", sfc), []);
        // Its HTML is still checked.
        let wrong = found("vue", "<template>\n  <div><span></div>\n  <img clas=\"x\">\n</template>\n");
        assert!(wrong.iter().any(|(l, _, m)| *l == 2 && m.contains("`<span>` is never closed")), "{wrong:#?}");
        assert!(wrong.iter().any(|(l, _, m)| *l == 3 && m.contains("no `clas`")), "{wrong:#?}");
        // Pug isn't HTML.
        assert_eq!(found("vue", "<template lang=\"pug\">\ndiv(class=\"a\")\n</template>\n"), []);
        assert!(checks("vue"));
    }

    #[test]
    fn svelte_blocks_and_expressions_are_read_around() {
        let component = "<script context=\"module\">\n  export const x = 1;\n</script>\n<script>\n  let n = 0;\n</script>\n{#if n > 1}\n  <p class:big={n > 5} on:click={() => n++}>{n}</p>\n{:else}\n  <Card {n} />\n{/if}\n{@html raw}\n<style>\n  p { color: red }\n</style>\n";
        assert_eq!(found("svelte", component), []);
        let wrong = found("svelte", "{#each xs as x}\n  <li><b>{x}</li>\n{/each}\n");
        assert!(wrong.iter().any(|(_, _, m)| m.contains("<b>")), "{wrong:#?}");
        assert!(checks("svelte"));
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
    fn xml_reports_every_mismatch_and_reads_on() {
        assert_eq!(found("xml", "<a><b></b></a>\n"), []);
        let wrong = found("xml", "<a>\n  <b></c>\n  <d x=\"1\" x=\"2\"/>\n  <e>\n</a>\n</f>\n");
        assert!(wrong.iter().any(|(l, _, m)| *l == 2 && m.contains("`</c>` closes nothing")), "{wrong:#?}");
        assert!(wrong.iter().any(|(l, _, m)| *l == 2 && m.contains("`<b>` is never closed")), "{wrong:#?}");
        assert!(wrong.iter().any(|(l, _, m)| *l == 3 && m.contains("`x` is set twice")), "{wrong:#?}");
        assert!(wrong.iter().any(|(l, _, m)| *l == 4 && m.contains("`<e>` is never closed")), "{wrong:#?}");
        assert!(wrong.iter().any(|(l, _, m)| *l == 6 && m.contains("`</f>`")), "{wrong:#?}");
        let open = found("csproj", "<Project>\n  <ItemGroup>\n");
        assert_eq!(open.len(), 2, "{open:?}");
        assert!(!checks("md"));
        assert!(checks("svg") && checks("htm") && checks("j2") && checks("hbs"));
        // Broken syntax still ends.
        let _ = found("xml", "<a b=\"1><c><<<>>></");
    }
}
