//! Markdown for the preview, read by pulldown-cmark: CommonMark with GitHub's tables,
//! strikethrough and task lists. It never fetches anything: what images and links name
//! is left to whoever shows them.

use std::collections::HashMap;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// How a run of text looks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// Where it leads, when it's (part of) a link.
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String, Style),
    Image {
        alt: String,
        url: String,
        link: Option<String>,
    },
    /// A line break inside a paragraph.
    Break,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// `Some(done)` for a task list item.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// `anchor` is what `#anchor` links to it with, as GitHub makes them.
    Heading {
        level: u8,
        text: Vec<Inline>,
        anchor: String,
    },
    Paragraph(Vec<Inline>),
    Code {
        lang: String,
        text: String,
    },
    /// `start` is the first number of a numbered list.
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Quote(Vec<Block>),
    Rule,
    Table {
        align: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

enum Kind {
    Root,
    Quote,
    List(Option<u64>, Vec<Item>),
    Item(Option<bool>),
}

struct Container {
    kind: Kind,
    blocks: Vec<Block>,
}

#[derive(Default)]
struct Table {
    align: Vec<Align>,
    header: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
    row: Vec<Vec<Inline>>,
}

struct Builder {
    containers: Vec<Container>,
    inlines: Vec<Inline>,
    bold: u32,
    italic: u32,
    strike: u32,
    code: u32,
    links: Vec<String>,
    /// An image being read: its address, and its alt text so far.
    image: Option<(String, String)>,
    /// A code block being read: its language and text.
    code_block: Option<(String, String)>,
    html: Option<String>,
    table: Option<Table>,
    anchors: HashMap<String, usize>,
}

impl Builder {
    fn style(&self) -> Style {
        Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code: self.code > 0,
            link: self.links.last().cloned(),
        }
    }

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let style = self.style();
        if let Some(Inline::Text(last, last_style)) = self.inlines.last_mut()
            && *last_style == style
        {
            last.push_str(text);
            return;
        }
        self.inlines.push(Inline::Text(text.to_owned(), style));
    }

    fn line_break(&mut self) {
        if !self.inlines.is_empty() && !matches!(self.inlines.last(), Some(Inline::Break)) {
            self.inlines.push(Inline::Break);
        }
    }

    fn top(&mut self) -> &mut Container {
        self.containers.last_mut().expect("the root is never popped")
    }

    /// What's been read since the last block, as a paragraph.
    fn flush(&mut self) {
        while matches!(self.inlines.last(), Some(Inline::Break)) {
            self.inlines.pop();
        }
        if self.inlines.is_empty() {
            return;
        }
        let inlines = std::mem::take(&mut self.inlines);
        self.top().blocks.push(Block::Paragraph(inlines));
    }

    fn block(&mut self, block: Block) {
        self.flush();
        self.top().blocks.push(block);
    }

    fn anchor(&mut self, text: &[Inline]) -> String {
        let mut slug = String::new();
        for inline in text {
            if let Inline::Text(t, _) = inline {
                for c in t.chars().flat_map(char::to_lowercase) {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        slug.push(c);
                    } else if c == ' ' {
                        slug.push('-');
                    }
                }
            }
        }
        let seen = self.anchors.entry(slug.clone()).or_insert(0);
        *seen += 1;
        if *seen == 1 { slug } else { format!("{slug}-{}", *seen - 1) }
    }

    /// A tag of HTML inside Markdown: the common ones that change how text looks.
    fn html_tag(&mut self, tag: &str) {
        let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
        if inner.starts_with('!') || inner.starts_with('?') {
            return;
        }
        let closing = inner.starts_with('/');
        let inner = inner.trim_start_matches('/');
        let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        let attrs = &inner[name_end..];
        let count = |n: &mut u32| {
            if closing {
                *n = n.saturating_sub(1);
            } else {
                *n += 1;
            }
        };
        match name.as_str() {
            "b" | "strong" | "summary" => {
                count(&mut self.bold);
                if name == "summary" && closing {
                    self.line_break();
                }
            }
            "i" | "em" | "cite" => count(&mut self.italic),
            "s" | "del" | "strike" => count(&mut self.strike),
            "code" | "kbd" | "samp" | "tt" => count(&mut self.code),
            "a" if closing => {
                self.links.pop();
            }
            "a" => self.links.push(attribute(attrs, "href").unwrap_or_default()),
            "br" => self.line_break(),
            "img" => {
                let url = attribute(attrs, "src").unwrap_or_default();
                let alt = attribute(attrs, "alt").unwrap_or_default();
                self.inlines.push(Inline::Image { alt, url, link: self.links.last().cloned() });
            }
            "p" | "div" | "details" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "pre"
            | "table" | "ul" | "ol" | "hr" => self.line_break(),
            _ => {}
        }
    }

    /// A block of raw HTML, as text with the tags above applied.
    fn html_block(&mut self, html: &str) {
        let mut rest = html;
        while !rest.is_empty() {
            if let Some(comment) = rest.strip_prefix("<!--") {
                rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            } else if rest.starts_with('<')
                && let Some(end) = rest.find('>')
            {
                self.html_tag(&rest[..=end]);
                rest = &rest[end + 1..];
            } else {
                let end = rest[1..].find('<').map_or(rest.len(), |i| i + 1);
                let words: Vec<&str> = rest[..end].split_whitespace().collect();
                if !words.is_empty() {
                    let lead = rest[..end].starts_with(char::is_whitespace) && !self.inlines.is_empty();
                    let trail = rest[..end].ends_with(char::is_whitespace);
                    let text =
                        format!("{}{}{}", if lead { " " } else { "" }, words.join(" "), if trail { " " } else { "" });
                    self.text(&unescape(&text));
                }
                rest = &rest[end..];
            }
        }
        // Whatever the HTML left open ends with it.
        (self.bold, self.italic, self.strike, self.code) = (0, 0, 0, 0);
        self.links.clear();
        self.flush();
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::Heading { .. } => self.flush(),
            Tag::BlockQuote(_) => {
                self.flush();
                self.containers.push(Container { kind: Kind::Quote, blocks: Vec::new() });
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split([' ', ',', '{']).next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_block = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                self.containers.push(Container { kind: Kind::List(start, Vec::new()), blocks: Vec::new() });
            }
            Tag::Item => self.containers.push(Container { kind: Kind::Item(None), blocks: Vec::new() }),
            Tag::Table(align) => {
                self.flush();
                let align = align
                    .into_iter()
                    .map(|a| match a {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                self.table = Some(Table { align, ..Default::default() });
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(dest_url.into_string()),
            Tag::Image { dest_url, .. } => self.image = Some((dest_url.into_string(), String::new())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(level) => {
                let text = std::mem::take(&mut self.inlines);
                let anchor = self.anchor(&text);
                self.block(Block::Heading { level: level as u8, text, anchor });
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                if self.containers.len() > 1 {
                    let quote = self.containers.pop().map(|c| c.blocks).unwrap_or_default();
                    self.block(Block::Quote(quote));
                }
            }
            TagEnd::CodeBlock => {
                if let Some((lang, mut text)) = self.code_block.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.block(Block::Code { lang, text });
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html.take() {
                    self.html_block(&html);
                }
            }
            TagEnd::Item => {
                self.flush();
                if let Some(Container { kind: Kind::Item(task), blocks }) = self.containers.pop()
                    && let Some(Container { kind: Kind::List(_, items), .. }) = self.containers.last_mut()
                {
                    items.push(Item { task, blocks });
                }
            }
            TagEnd::List(_) => {
                self.flush();
                if let Some(Container { kind: Kind::List(start, items), .. }) = self.containers.pop() {
                    self.block(Block::List { start, items });
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inlines);
                if let Some(table) = &mut self.table {
                    table.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.header = std::mem::take(&mut table.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(Table { align, header, rows, .. }) = self.table.take() {
                    self.block(Block::Table { align, header, rows });
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    let link = self.links.last().cloned();
                    self.inlines.push(Inline::Image { alt, url, link });
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                if let Some((_, code)) = &mut self.code_block {
                    code.push_str(&text);
                } else if let Some((_, alt)) = &mut self.image {
                    alt.push_str(&text);
                } else {
                    self.text(&text);
                }
            }
            Event::Code(text) => {
                if let Some((_, alt)) = &mut self.image {
                    alt.push_str(&text);
                } else {
                    self.code += 1;
                    self.text(&text);
                    self.code -= 1;
                }
            }
            Event::Html(html) => match &mut self.html {
                Some(block) => block.push_str(&html),
                None => self.html_block(&html),
            },
            Event::InlineHtml(html) => self.html_tag(&html),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.inlines.push(Inline::Break),
            Event::Rule => self.block(Block::Rule),
            Event::FootnoteReference(label) => self.text(&format!("[{label}]")),
            Event::TaskListMarker(done) => {
                if let Some(Container { kind: Kind::Item(task), .. }) = self.containers.last_mut() {
                    *task = Some(done);
                }
            }
        }
    }
}

/// An attribute's value in the inside of an HTML tag.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = attrs[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let text = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or(""),
        };
        return Some(unescape(text));
    }
    None
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

/// A Markdown document as blocks.
pub fn parse(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let mut builder = Builder {
        containers: vec![Container { kind: Kind::Root, blocks: Vec::new() }],
        inlines: Vec::new(),
        bold: 0,
        italic: 0,
        strike: 0,
        code: 0,
        links: Vec::new(),
        image: None,
        code_block: None,
        html: None,
        table: None,
        anchors: HashMap::new(),
    };
    for event in Parser::new_ext(text, options) {
        builder.event(event);
    }
    builder.flush();
    // Anything left open (it can't be, but a parser bug shouldn't lose text) joins the root.
    while builder.containers.len() > 1 {
        let blocks = builder.containers.pop().map(|c| c.blocks).unwrap_or_default();
        builder.top().blocks.extend(blocks);
    }
    builder.containers.pop().map(|c| c.blocks).unwrap_or_default()
}

/// The plain text of some inlines, as a heading's or a cell's.
pub fn plain(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text(t, _) => t.as_str(),
            Inline::Image { alt, .. } => alt.as_str(),
            Inline::Break => " ",
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned(), Style::default())
    }

    fn styled(s: &str, f: impl FnOnce(&mut Style)) -> Inline {
        let mut style = Style::default();
        f(&mut style);
        Inline::Text(s.to_owned(), style)
    }

    #[test]
    fn nested_spans_keep_every_style() {
        let blocks = parse("a **b `c` [d *e*](http://x)** f");
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(
            spans,
            &[
                text("a "),
                styled("b ", |s| s.bold = true),
                styled("c", |s| (s.bold, s.code) = (true, true)),
                styled(" ", |s| s.bold = true),
                styled("d ", |s| (s.bold, s.link) = (true, Some("http://x".into()))),
                styled("e", |s| (s.bold, s.italic, s.link) = (true, true, Some("http://x".into()))),
                text(" f"),
            ]
        );
    }

    #[test]
    fn the_commonmark_cases_the_old_reader_missed() {
        let doc = "Setext\n===\n\n    indented code\n\nhard  \nbreak\n\n[ref][r]\n\n[r]: https://r.example\n\n- item *a\n  lazy* line\n";
        let blocks = parse(doc);
        assert!(matches!(&blocks[0], Block::Heading { level: 1, anchor, .. } if anchor == "setext"));
        assert_eq!(blocks[1], Block::Code { lang: String::new(), text: "indented code".into() });
        assert_eq!(blocks[2], Block::Paragraph(vec![text("hard"), Inline::Break, text("break")]));
        assert_eq!(blocks[3], Block::Paragraph(vec![styled("ref", |s| s.link = Some("https://r.example".into()))]));
        let Block::List { items, .. } = &blocks[4] else { panic!("{:?}", blocks[4]) };
        assert_eq!(
            items[0].blocks,
            [Block::Paragraph(vec![text("item "), styled("a lazy", |s| s.italic = true), text(" line")])]
        );
    }

    #[test]
    fn lists_nest_and_quotes_hold_blocks() {
        let blocks = parse("1. one\n   - two\n     - [x] three\n3. four\n\n> quote\n> - in a list\n>\n> second\n");
        let Block::List { start: Some(1), items } = &blocks[0] else { panic!("{:?}", blocks[0]) };
        assert_eq!(items.len(), 2);
        let Block::List { start: None, items: inner } = &items[0].blocks[1] else { panic!() };
        let Block::List { items: innermost, .. } = &inner[0].blocks[1] else { panic!() };
        assert_eq!(innermost[0].task, Some(true));
        let Block::Quote(quote) = &blocks[1] else { panic!("{:?}", blocks[1]) };
        assert_eq!(quote.len(), 3);
        assert!(matches!(quote[1], Block::List { .. }));
    }

    #[test]
    fn tables_images_code_and_html() {
        // GitHub splits a cell at `|` even inside code, unless it's escaped.
        let doc = "| a | `x\\|y` |\n|:--|--:|\n| 1 | 2 |\n\n![logo](img/a.png)\n\n```json title\n{}\n```\n\n<details><summary>More</summary>Hidden <b>HTML</b> &amp; text</details>\n";
        let blocks = parse(doc);
        let Block::Table { align, header, rows } = &blocks[0] else { panic!("{:?}", blocks[0]) };
        assert_eq!(align, &[Align::Left, Align::Right]);
        assert_eq!(plain(&header[1]), "x|y");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            blocks[1],
            Block::Paragraph(vec![Inline::Image { alt: "logo".into(), url: "img/a.png".into(), link: None }])
        );
        assert_eq!(blocks[2], Block::Code { lang: "json".into(), text: "{}".into() });
        let Block::Paragraph(html) = &blocks[3] else { panic!("{:?}", blocks[3]) };
        assert_eq!(plain(html), "More Hidden HTML & text");
        assert!(html.contains(&styled("More", |s| s.bold = true)));
        assert!(html.contains(&styled("HTML", |s| s.bold = true)));
    }

    #[test]
    fn anchors_are_like_githubs() {
        let blocks = parse("# Hello, World!\n# Hello, World!\n## Café 🚀\n");
        let anchors: Vec<&str> = blocks
            .iter()
            .map(|b| match b {
                Block::Heading { anchor, .. } => anchor.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(anchors, ["hello-world", "hello-world-1", "café-"]);
    }

    #[test]
    fn odd_input_does_not_panic() {
        for odd in ["", "#", "- ", "|", "|\n|-|", "```", ">", "1.", "é - è", "[a](", "![", "*", "**", "`", "<a", "<b>x"]
        {
            let _ = parse(odd);
        }
    }
}
