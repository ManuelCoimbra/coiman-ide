//! Emmet: short abbreviations that grow into HTML and CSS. The result is a snippet
//! body (`crate::editor::snippets::prepare` inserts it), with a stop in each empty place.

/// How the text around an abbreviation is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Html,
    /// JSX: `className`, `htmlFor`, and `<br />`.
    Jsx,
    Css,
}

impl Syntax {
    pub fn for_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "html" | "htm" | "xhtml" | "vue" | "svelte" | "php" => Self::Html,
            "jsx" | "tsx" => Self::Jsx,
            "css" | "scss" | "less" => Self::Css,
            _ => return None,
        })
    }

    /// Whether Tab may expand here. In `.php` and JSX a word is usually code,
    /// so those only expand on the command.
    pub fn expands_on_tab(ext: &str) -> bool {
        matches!(ext, "html" | "htm" | "xhtml" | "vue" | "svelte" | "css" | "scss" | "less")
    }
}

const TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];
const VOID: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "button", "cite", "code", "data", "del", "dfn", "em", "i", "ins", "kbd", "label",
    "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "u", "var", "br", "img", "input",
];
type Alias = (&'static str, &'static str, &'static [(&'static str, &'static str)]);
/// Short names, with what they stand for: a tag and its attributes.
const ALIASES: &[Alias] = &[
    ("btn", "button", &[]),
    ("a:link", "a", &[("href", "http://")]),
    ("a:mail", "a", &[("href", "mailto:")]),
    ("link:css", "link", &[("rel", "stylesheet"), ("href", "style.css")]),
    ("script:src", "script", &[("src", "")]),
    ("input:text", "input", &[("type", "text"), ("name", ""), ("id", "")]),
    ("input:email", "input", &[("type", "email"), ("name", ""), ("id", "")]),
    ("input:password", "input", &[("type", "password"), ("name", ""), ("id", "")]),
    ("input:checkbox", "input", &[("type", "checkbox"), ("name", ""), ("id", "")]),
    ("input:radio", "input", &[("type", "radio"), ("name", ""), ("id", "")]),
    ("input:submit", "input", &[("type", "submit"), ("value", "")]),
    ("meta:vp", "meta", &[("name", "viewport"), ("content", "width=device-width, initial-scale=1.0")]),
];
/// Attributes a bare element is rarely useful without.
fn default_attrs(tag: &str) -> &'static [(&'static str, &'static str)] {
    match tag {
        "a" => &[("href", "")],
        "img" => &[("src", ""), ("alt", "")],
        "input" => &[("type", "text")],
        "link" => &[("rel", "stylesheet"), ("href", "")],
        "form" => &[("action", "")],
        "label" => &[("for", "")],
        "iframe" | "embed" | "source" | "audio" | "video" => &[("src", "")],
        "area" => &[("shape", ""), ("coords", ""), ("href", ""), ("alt", "")],
        "object" => &[("data", "")],
        "time" => &[("datetime", "")],
        _ => &[],
    }
}

const PAGE: &str = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n\t<meta charset=\"UTF-8\">\n\t<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\t<title>${1:Document}</title>\n</head>\n<body>\n\t$0\n</body>\n</html>";

#[derive(Debug, Default)]
struct Node {
    /// Empty: worked out from the parent.
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, String)>,
    text: Option<String>,
    repeat: usize,
    children: Vec<Node>,
    /// A `(group)`: only its children are written.
    group: bool,
    alias: bool,
}

/// More than this many elements is a typo, and expanding it would hang the UI.
const MAX_NODES: usize = 2000;

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        self.at += usize::from(found);
        found
    }

    fn word(&mut self, also: &str) -> String {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || "-_$@".contains(c) || also.contains(c)) {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect()
    }

    fn until(&mut self, close: char) -> Option<String> {
        let start = self.at;
        while self.peek()? != close {
            self.at += 1;
        }
        let text = self.chars[start..self.at].iter().collect();
        self.at += 1;
        Some(text)
    }

    fn attrs(&mut self, node: &mut Node) -> Option<()> {
        loop {
            while self.eat(' ') {}
            if self.eat(']') {
                return Some(());
            }
            let name = self.word(":.");
            if name.is_empty() {
                return None;
            }
            let value = if self.eat('=') {
                match self.peek()? {
                    q @ ('"' | '\'') => {
                        self.at += 1;
                        self.until(q)?
                    }
                    _ => {
                        let start = self.at;
                        while !matches!(self.peek()?, ' ' | ']') {
                            self.at += 1;
                        }
                        self.chars[start..self.at].iter().collect()
                    }
                }
            } else {
                String::new()
            };
            node.attrs.push((name, value));
        }
    }

    /// One element, with what is written directly on it.
    fn element(&mut self) -> Option<Node> {
        let mut node = Node { repeat: 1, ..Node::default() };
        if self.eat('(') {
            node.group = true;
            node.children = self.list()?;
            if !self.eat(')') {
                return None;
            }
        } else {
            node.tag = self.word(":");
        }
        loop {
            match self.peek() {
                Some('#') if !node.group => {
                    self.at += 1;
                    let id = self.word("");
                    if id.is_empty() {
                        return None;
                    }
                    node.id = Some(id);
                }
                Some('.') if !node.group => {
                    self.at += 1;
                    let class = self.word("");
                    if class.is_empty() {
                        return None;
                    }
                    node.classes.push(class);
                }
                Some('[') if !node.group => {
                    self.at += 1;
                    self.attrs(&mut node)?;
                }
                Some('{') if !node.group => {
                    self.at += 1;
                    node.text = Some(self.until('}')?);
                }
                Some('*') => {
                    self.at += 1;
                    let start = self.at;
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.at += 1;
                    }
                    let digits: String = self.chars[start..self.at].iter().collect();
                    node.repeat = digits.parse().ok().filter(|n| *n > 0)?;
                }
                _ => break,
            }
        }
        let empty = node.tag.is_empty() && node.id.is_none() && node.classes.is_empty();
        let bare = node.attrs.is_empty() && node.text.is_none();
        if !node.group && empty && bare {
            return None;
        }
        Some(node)
    }

    /// Siblings, each with what's nested under it, up to a `)` or a climb out.
    fn list(&mut self) -> Option<Vec<Node>> {
        // A stack of open levels: the last one is being added to.
        let mut levels: Vec<Vec<Node>> = vec![Vec::new()];
        levels[0].push(self.element()?);
        loop {
            match self.peek() {
                Some('+') => {
                    self.at += 1;
                    let node = self.element()?;
                    levels.last_mut()?.push(node);
                }
                Some('>') => {
                    self.at += 1;
                    levels.push(vec![self.element()?]);
                }
                Some('^') => {
                    while self.eat('^') {
                        if levels.len() > 1 {
                            let done = levels.pop()?;
                            levels.last_mut()?.last_mut()?.children.extend(done);
                        }
                    }
                    let node = self.element()?;
                    levels.last_mut()?.push(node);
                }
                _ => break,
            }
        }
        while levels.len() > 1 {
            let done = levels.pop()?;
            levels.last_mut()?.last_mut()?.children.extend(done);
        }
        levels.pop()
    }
}

fn parse(abbreviation: &str) -> Option<Vec<Node>> {
    let mut parser = Parser { chars: abbreviation.chars().collect(), at: 0 };
    let nodes = parser.list()?;
    (parser.at == parser.chars.len()).then_some(nodes)
}

/// `$` counts repeats; `$$` pads to two digits.
fn numbered(text: &str, index: usize) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let mut width = 1;
        while chars.peek() == Some(&'$') {
            chars.next();
            width += 1;
        }
        out.push_str(&format!("{index:0width$}"));
    }
    out
}

/// Text that must not be read as a snippet.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('$', "\\$").replace('}', "\\}")
}

fn implicit_tag(parent: &str) -> &'static str {
    match parent {
        "ul" | "ol" | "menu" => "li",
        "table" | "tbody" | "thead" | "tfoot" => "tr",
        "tr" => "td",
        "select" | "optgroup" => "option",
        "colgroup" => "col",
        "picture" | "audio" | "video" => "source",
        "em" | "strong" | "span" | "a" | "b" | "i" | "p" => "span",
        _ => "div",
    }
}

struct Writer {
    syntax: Syntax,
    stop: u32,
    nodes: usize,
}

impl Writer {
    fn next_stop(&mut self) -> String {
        self.stop += 1;
        format!("${{{}}}", self.stop)
    }

    fn attr_name(&self, name: &str) -> String {
        match (self.syntax, name) {
            (Syntax::Jsx, "class") => "className".to_owned(),
            (Syntax::Jsx, "for") => "htmlFor".to_owned(),
            _ => name.to_owned(),
        }
    }

    fn attr(&mut self, out: &mut String, name: &str, value: &str, index: usize) {
        let name = self.attr_name(name);
        let value = if value.is_empty() { self.next_stop() } else { literal(&numbered(value, index)) };
        out.push_str(&format!(" {name}=\"{value}\""));
    }

    fn is_inline(node: &Node, tag: &str) -> bool {
        (INLINE.contains(&tag) || node.tag.is_empty() && node.children.is_empty() && tag == "span")
            && node.children.iter().all(|c| !c.group && Self::is_inline(c, &c.tag))
    }

    fn one_inline_child(node: &Node) -> bool {
        matches!(&node.children[..], [only] if only.repeat == 1 && !only.group && Self::is_inline(only, &only.tag))
    }

    fn write(&mut self, node: &Node, parent: &str, depth: usize, out: &mut Vec<String>, index: usize) -> Option<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return None;
        }
        if node.group {
            for child in &node.children {
                self.write_repeated(child, parent, depth, out)?;
            }
            return Some(());
        }
        let alias = ALIASES.iter().find(|a| a.0 == node.tag);
        let tag = alias.map_or(if node.tag.is_empty() { implicit_tag(parent) } else { node.tag.as_str() }, |a| a.1);
        let mut line = format!("<{tag}");
        if let Some(id) = &node.id {
            self.attr(&mut line, "id", id, index);
        }
        if !node.classes.is_empty() {
            let classes: Vec<String> = node.classes.iter().map(|c| numbered(c, index)).collect();
            self.attr(&mut line, "class", &classes.join(" "), index);
        }
        let given = |name: &str| node.attrs.iter().any(|(n, _)| n == name);
        let defaults: &[(&str, &str)] = alias.map_or(default_attrs(tag), |a| a.2);
        for (name, value) in defaults {
            if !given(name) && !node.alias {
                self.attr(&mut line, name, value, index);
            }
        }
        for (name, value) in &node.attrs {
            self.attr(&mut line, name, value, index);
        }
        let pad = "\t".repeat(depth);
        if VOID.contains(&tag) {
            line.push_str(if self.syntax == Syntax::Jsx { " />" } else { ">" });
            out.push(format!("{pad}{line}"));
            return Some(());
        }
        line.push('>');
        let text = node.text.as_deref().map(|t| literal(&numbered(t, index)));
        if node.children.is_empty() {
            let inner = text.unwrap_or_else(|| self.next_stop());
            out.push(format!("{pad}{line}{inner}</{tag}>"));
        } else if Self::is_inline(node, tag) || Self::one_inline_child(node) {
            let mut inner = Vec::new();
            for child in &node.children {
                self.write_repeated(child, tag, 0, &mut inner)?;
            }
            out.push(format!("{pad}{line}{}{}</{tag}>", text.unwrap_or_default(), inner.concat()));
        } else {
            out.push(format!("{pad}{line}{}", text.unwrap_or_default()));
            for child in &node.children {
                self.write_repeated(child, tag, depth + 1, out)?;
            }
            out.push(format!("{pad}</{tag}>"));
        }
        Some(())
    }

    fn write_repeated(&mut self, node: &Node, parent: &str, depth: usize, out: &mut Vec<String>) -> Option<()> {
        for index in 1..=node.repeat {
            self.write(node, parent, depth, out, index)?;
        }
        Some(())
    }
}

fn tag_is_known(tag: &str, syntax: Syntax) -> bool {
    tag.is_empty()
        || TAGS.contains(&tag)
        || ALIASES.iter().any(|a| a.0 == tag)
        // Custom elements, and components.
        || (tag.contains('-') && !tag.starts_with('-'))
        || (syntax == Syntax::Jsx && tag.starts_with(char::is_uppercase))
}

fn all_known(nodes: &[Node], syntax: Syntax) -> bool {
    nodes.iter().all(|n| (n.group || tag_is_known(&n.tag, syntax)) && all_known(&n.children, syntax))
}

fn expand_markup(abbreviation: &str, syntax: Syntax) -> Option<String> {
    if abbreviation == "!" || abbreviation == "html:5" {
        return (syntax == Syntax::Html).then(|| PAGE.to_owned());
    }
    let nodes = parse(abbreviation)?;
    if !all_known(&nodes, syntax) {
        return None;
    }
    let mut writer = Writer { syntax, stop: 0, nodes: 0 };
    let mut lines = Vec::new();
    for node in &nodes {
        writer.write_repeated(node, "", 0, &mut lines)?;
    }
    Some(lines.join("\n"))
}

// ----- CSS -------------------------------------------------------------------------

const PROPERTIES: &[(&str, &str)] = &[
    ("m", "margin"),
    ("mt", "margin-top"),
    ("mr", "margin-right"),
    ("mb", "margin-bottom"),
    ("ml", "margin-left"),
    ("p", "padding"),
    ("pt", "padding-top"),
    ("pr", "padding-right"),
    ("pb", "padding-bottom"),
    ("pl", "padding-left"),
    ("w", "width"),
    ("h", "height"),
    ("maw", "max-width"),
    ("mah", "max-height"),
    ("miw", "min-width"),
    ("mih", "min-height"),
    ("t", "top"),
    ("r", "right"),
    ("b", "bottom"),
    ("l", "left"),
    ("fz", "font-size"),
    ("fw", "font-weight"),
    ("ff", "font-family"),
    ("lh", "line-height"),
    ("lts", "letter-spacing"),
    ("c", "color"),
    ("bg", "background"),
    ("bgc", "background-color"),
    ("bd", "border"),
    ("bdrs", "border-radius"),
    ("gap", "gap"),
    ("z", "z-index"),
    ("op", "opacity"),
    ("d", "display"),
    ("pos", "position"),
    ("ta", "text-align"),
    ("td", "text-decoration"),
    ("tt", "text-transform"),
    ("fl", "float"),
    ("ov", "overflow"),
    ("cur", "cursor"),
    ("jc", "justify-content"),
    ("ai", "align-items"),
    ("fxd", "flex-direction"),
    ("fx", "flex"),
    ("trs", "transition"),
    ("trf", "transform"),
    ("bxsh", "box-shadow"),
    ("bxz", "box-sizing"),
];
/// Whole declarations, written as one word.
const KEYWORDS: &[(&str, &str)] = &[
    ("dn", "display: none"),
    ("db", "display: block"),
    ("di", "display: inline"),
    ("dib", "display: inline-block"),
    ("df", "display: flex"),
    ("dif", "display: inline-flex"),
    ("dg", "display: grid"),
    ("posa", "position: absolute"),
    ("posr", "position: relative"),
    ("posf", "position: fixed"),
    ("poss", "position: sticky"),
    ("tac", "text-align: center"),
    ("tal", "text-align: left"),
    ("tar", "text-align: right"),
    ("fwb", "font-weight: bold"),
    ("fwn", "font-weight: normal"),
    ("tdn", "text-decoration: none"),
    ("ttu", "text-transform: uppercase"),
    ("ovh", "overflow: hidden"),
    ("ova", "overflow: auto"),
    ("curp", "cursor: pointer"),
    ("jcc", "justify-content: center"),
    ("jcsb", "justify-content: space-between"),
    ("aic", "align-items: center"),
    ("fxdc", "flex-direction: column"),
    ("bxzb", "box-sizing: border-box"),
];
/// Values that stay bare numbers.
const UNITLESS: &[&str] = &["z-index", "opacity", "font-weight", "line-height", "flex"];

fn css_value(value: &str, property: &str) -> Option<String> {
    let number_end = value.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(value.len());
    let (number, unit) = value.split_at(number_end);
    if number.is_empty() || number.parse::<f64>().is_err() {
        return None;
    }
    let unit = match unit {
        "" if number == "0" || UNITLESS.contains(&property) => "",
        "" => "px",
        "p" => "%",
        "e" => "em",
        "r" => "rem",
        "x" => "ex",
        "px" | "em" | "rem" | "vh" | "vw" | "fr" | "ch" | "s" | "ms" => unit,
        _ => return None,
    };
    Some(format!("{number}{unit}"))
}

fn expand_css(abbreviation: &str) -> Option<String> {
    if let Some((_, full)) = KEYWORDS.iter().find(|(short, _)| *short == abbreviation) {
        return Some(format!("{full};"));
    }
    let name_end = abbreviation.find(|c: char| !c.is_ascii_lowercase()).unwrap_or(abbreviation.len());
    let (name, rest) = abbreviation.split_at(name_end);
    let (_, property) = PROPERTIES.iter().find(|(short, _)| *short == name)?;
    if rest.is_empty() {
        return Some(format!("{property}: $1;"));
    }
    if rest.starts_with('#') && rest.len() > 1 {
        return Some(format!("{property}: {rest};"));
    }
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    // Dashes separate values, or make the first one negative.
    let (negative, rest) = rest.strip_prefix('-').map_or((false, rest), |r| (true, r));
    let mut values = Vec::new();
    for (i, part) in rest.split('-').enumerate() {
        let value = css_value(part, property)?;
        values.push(if negative && i == 0 && value != "0" { format!("-{value}") } else { value });
    }
    Some(format!("{property}: {};", values.join(" ")))
}

// ----- finding it in the text ------------------------------------------------------

/// The abbreviation ending a line's text before the cursor: its start, in chars.
fn abbreviation_start(before: &[char]) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = before.len();
    while i > 0 {
        let c = before[i - 1];
        match c {
            ']' | '}' | ')' => depth += 1,
            '[' | '{' | '(' if depth > 0 => depth -= 1,
            '[' | '{' => break,
            _ if depth > 0 => {}
            // After a tag: what's past its `>` is the abbreviation.
            '<' => {
                i += before[i..].iter().position(|c| *c == '>')? + 1;
                break;
            }
            _ if c.is_whitespace() => break,
            _ => {}
        }
        i -= 1;
    }
    (depth == 0 && i < before.len()).then_some(i)
}

/// Whether a cursor after `text` is in the words of a tag, a comment, or a script
/// or style block, where HTML abbreviations mean nothing. For a page, says which.
fn page_context(text: &str) -> PageContext {
    let lower = text.to_ascii_lowercase();
    let last = |needle: &str| lower.rfind(needle);
    if last("<!--") > last("-->") {
        return PageContext::Elsewhere;
    }
    if last("<script") > last("</script") {
        return PageContext::Elsewhere;
    }
    if last("<style") > last("</style") {
        return PageContext::Style;
    }
    if last("<") > last(">") { PageContext::Elsewhere } else { PageContext::Markup }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageContext {
    Markup,
    Style,
    Elsewhere,
}

/// Inside a rule's braces, in a place a declaration can start.
fn in_declaration_block(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0i32;
    for segment_end in chars.iter() {
        match segment_end {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// What Tab or the command would put in place of the abbreviation before the cursor.
#[derive(Debug, PartialEq, Eq)]
pub struct Expansion {
    /// Chars before the cursor to replace.
    pub length: usize,
    pub body: String,
}

/// Expands what ends `before` (the file's text up to the cursor).
/// `on_tab` also asks that it stands alone at the start of its line or after a
/// tag, since on Tab a word in running text is far more likely to be just a word.
pub fn expand(before: &str, syntax: Syntax, on_tab: bool) -> Option<Expansion> {
    if before.ends_with(char::is_whitespace) || before.is_empty() {
        return None;
    }
    let line = before.rsplit('\n').next().unwrap_or(before);
    let chars: Vec<char> = line.chars().collect();
    let start = abbreviation_start(&chars)?;
    let abbreviation: String = chars[start..].iter().collect();
    let head: String = chars[..start].iter().collect();
    let earlier = &before[..before.len() - abbreviation.len()];
    let as_css = match syntax {
        Syntax::Css => true,
        Syntax::Html => match page_context(earlier) {
            PageContext::Markup => false,
            PageContext::Style => true,
            PageContext::Elsewhere => return None,
        },
        Syntax::Jsx => false,
    };
    let body = if as_css {
        if !in_declaration_block(earlier) {
            return None;
        }
        // In the middle of `property: value` a word is a value.
        if !head.trim().is_empty() && !head.trim_end().ends_with(['{', ';']) {
            return None;
        }
        expand_css(&abbreviation)?
    } else {
        if on_tab && !(head.trim().is_empty() || head.trim_end().ends_with('>')) {
            return None;
        }
        expand_markup(&abbreviation, syntax)?
    };
    Some(Expansion { length: chars.len() - start, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(a: &str) -> String {
        let (text, _) = crate::language::lsp::convert::expand_snippet(&expand_markup(a, Syntax::Html).unwrap());
        text
    }

    #[test]
    fn expands_elements_with_their_parts() {
        assert_eq!(html("div"), "<div></div>");
        assert_eq!(html("div#a.b.c"), "<div id=\"a\" class=\"b c\"></div>");
        assert_eq!(html(".x"), "<div class=\"x\"></div>");
        assert_eq!(html("p{Hi there}"), "<p>Hi there</p>");
        assert_eq!(html("a[href=x.html target=\"_blank\"]"), "<a href=\"x.html\" target=\"_blank\"></a>");
        assert_eq!(html("img"), "<img src=\"\" alt=\"\">");
        assert_eq!(html("input:email"), "<input type=\"email\" name=\"\" id=\"\">");
        assert_eq!(html("btn"), "<button></button>");
        assert!(html("!").starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn nests_climbs_and_repeats() {
        assert_eq!(html("ul>li*2"), "<ul>\n\t<li></li>\n\t<li></li>\n</ul>");
        assert_eq!(html("ul>.i$*2"), "<ul>\n\t<li class=\"i1\"></li>\n\t<li class=\"i2\"></li>\n</ul>");
        assert_eq!(html("div+p"), "<div></div>\n<p></p>");
        assert_eq!(html("div>p^h1"), "<div>\n\t<p></p>\n</div>\n<h1></h1>");
        assert_eq!(html("div>a>b^^p"), "<div><a href=\"\"><b></b></a></div>\n<p></p>");
        assert_eq!(html("(dt+dd)*2"), "<dt></dt>\n<dd></dd>\n<dt></dt>\n<dd></dd>");
        assert_eq!(html("table>tr>td"), "<table>\n\t<tr>\n\t\t<td></td>\n\t</tr>\n</table>");
        assert_eq!(html("li.a$$*1"), "<li class=\"a01\"></li>");
        assert_eq!(html("nav>ul>li*1>a"), "<nav>\n\t<ul>\n\t\t<li><a href=\"\"></a></li>\n\t</ul>\n</nav>");
    }

    #[test]
    fn puts_the_cursor_in_the_first_empty_place() {
        let (text, (a, b)) =
            crate::language::lsp::convert::expand_snippet(&expand_markup("a>b", Syntax::Html).unwrap());
        assert_eq!(text.chars().take(a).collect::<String>(), "<a href=\"");
        assert_eq!(a, b);
    }

    #[test]
    fn text_with_snippet_marks_stays_text() {
        assert_eq!(html("p{cost: $}"), "<p>cost: 1</p>");
        let body = expand_markup("p[title=a}b]", Syntax::Html).unwrap();
        assert!(body.contains("a\\}b"));
    }

    #[test]
    fn writes_jsx() {
        let jsx = |a: &str| crate::language::lsp::convert::expand_snippet(&expand_markup(a, Syntax::Jsx).unwrap()).0;
        assert_eq!(jsx(".a"), "<div className=\"a\"></div>");
        assert_eq!(jsx("br"), "<br />");
        assert_eq!(jsx("label"), "<label htmlFor=\"\"></label>");
        assert_eq!(jsx("Card.x"), "<Card className=\"x\"></Card>");
        assert_eq!(expand_markup("!", Syntax::Jsx), None);
    }

    #[test]
    fn refuses_what_isnt_an_abbreviation() {
        for a in ["hello", "file.txt", "e.g.", "div>", "div*", "div*0", "(div", "p{x", "+", "a..b", "div[", "foo>bar"] {
            assert_eq!(expand_markup(a, Syntax::Html), None, "{a}");
        }
        assert_eq!(expand_markup("div*999999", Syntax::Html), None, "too many");
        assert!(expand_markup("my-card.x", Syntax::Html).is_some());
    }

    #[test]
    fn expands_css() {
        let css = |a: &str| expand_css(a);
        assert_eq!(css("m10").as_deref(), Some("margin: 10px;"));
        assert_eq!(css("m10-20").as_deref(), Some("margin: 10px 20px;"));
        assert_eq!(css("m-10").as_deref(), Some("margin: -10px;"));
        assert_eq!(css("m0").as_deref(), Some("margin: 0;"));
        assert_eq!(css("w50p").as_deref(), Some("width: 50%;"));
        assert_eq!(css("fz1.5r").as_deref(), Some("font-size: 1.5rem;"));
        assert_eq!(css("z10").as_deref(), Some("z-index: 10;"));
        assert_eq!(css("df").as_deref(), Some("display: flex;"));
        assert_eq!(css("c#fff").as_deref(), Some("color: #fff;"));
        assert_eq!(css("p").as_deref(), Some("padding: $1;"));
        assert_eq!(css("nope"), None);
        assert_eq!(css("m10q"), None);
    }

    #[test]
    fn finds_it_in_the_text() {
        let page = |before: &str| expand(before, Syntax::Html, true).map(|e| (e.length, e.body));
        assert_eq!(page("  ul>li").unwrap().0, 5);
        assert_eq!(page("<p>x</p>a{b c}").unwrap().0, 6);
        assert!(page("<body>\ndiv.a").is_some());
        assert_eq!(page("some words div"), None, "running text");
        assert_eq!(page("<div div"), None, "inside a tag");
        assert_eq!(page("<div class=\"a\ndiv"), None, "inside a tag, on the next line");
        assert_eq!(page("<!-- \ndiv"), None, "in a comment");
        assert_eq!(page("<script>\ndiv"), None, "in a script");
        assert_eq!(page("div "), None);
        assert_eq!(page(""), None);
        assert_eq!(page("<style>a {\n  m10").map(|p| p.1).as_deref(), Some("margin: 10px;"));
        assert!(expand("some words div", Syntax::Html, false).is_some(), "the command is asked outright");
    }

    #[test]
    fn css_needs_a_block_and_a_place_for_a_declaration() {
        let css = |before: &str| expand(before, Syntax::Css, true).map(|e| e.body);
        assert_eq!(css("a {\n  m10").as_deref(), Some("margin: 10px;"));
        assert_eq!(css("a { m10").as_deref(), Some("margin: 10px;"));
        assert_eq!(css("a { color: red; m10").as_deref(), Some("margin: 10px;"));
        assert_eq!(css("m10"), None, "not in a block");
        assert_eq!(css("a { width: m10"), None, "in a value");
        assert_eq!(css("a { } m10"), None);
    }
}
