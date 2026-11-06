//! Tags and quotes typed for you in markup, as VS Code's `html.autoClosingTags` and
//! `html.autoCreateQuotes` do: `>` ending a start tag adds its end tag, `</` finishes
//! the innermost open element's, and `=` after an attribute name adds `""`.

use crate::html_attributes::VOID;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TagMode {
    #[default]
    Off,
    /// HTML and its templates: void elements take no end tag.
    Html,
    /// XML: every element needs its end tag.
    Xml,
    /// JSX and TSX: tags only where an expression can start, so `Array<string>` isn't one.
    Jsx,
}

impl TagMode {
    pub fn for_extension(extension: &str) -> Self {
        match extension {
            "html" | "htm" | "xhtml" | "vue" | "svelte" | "jinja" | "jinja2" | "j2" | "njk" | "hbs" | "handlebars"
            | "ejs" | "twig" | "liquid" | "php" => Self::Html,
            "xml" | "svg" | "xaml" | "csproj" | "fsproj" | "vbproj" | "props" | "targets" | "plist" | "resx"
            | "xsd" | "xsl" | "xslt" => Self::Xml,
            "jsx" | "tsx" => Self::Jsx,
            _ => Self::Off,
        }
    }
}

/// What typing a character becomes: the text to put in its place, and where the
/// cursor goes in it, counted in chars.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typed {
    pub text: String,
    pub cursor: usize,
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | ':' | '_' | '.')
}

/// The text of the tag the cursor is in, after its `<`, when it is in one.
fn open_tag_body(before: &str) -> Option<&str> {
    let lt = before.rfind('<')?;
    let body = &before[lt + 1..];
    (!body.contains('>')).then_some(body)
}

/// Whether quotes and (in JSX) braces are all closed, so the cursor isn't inside a value.
fn balanced(body: &str) -> bool {
    let mut quote = None;
    let mut braces = 0i32;
    for c in body.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '{') => braces += 1,
            (None, '}') => braces -= 1,
            _ => {}
        }
    }
    quote.is_none() && braces == 0
}

/// In JSX, whether a `<` at the end of `before` can start an element: after `(`, `=`,
/// `>`, `{`, `,`, `?`, `:`, `&&`, `||`, `return` or the start of a line. After a name
/// it's a type argument or a comparison.
fn jsx_can_start(before: &str) -> bool {
    let code = before.trim_end();
    code.is_empty()
        || code.ends_with(['(', '=', '>', '{', ',', '?', ':', '[', '&', '|'])
        || code.ends_with("return")
        || code.ends_with('\n')
}

/// The elements open at the end of `before`, outermost first.
fn open_elements(before: &str, mode: TagMode) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    let mut rest = before;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt + 1..];
        if let Some(comment) = after.strip_prefix("!--") {
            let Some(end) = comment.find("-->") else { break };
            rest = &comment[end + 3..];
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        rest = &after[gt + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if let Some(at) = open.iter().rposition(|n| n == name) {
                open.truncate(at);
            }
            continue;
        }
        let name: String = tag.chars().take_while(|c| name_char(*c)).collect();
        if name.is_empty() && !(mode == TagMode::Jsx && tag.is_empty()) {
            continue;
        }
        let void = mode == TagMode::Html && VOID.contains(&name.to_ascii_lowercase().as_str());
        if tag.ends_with('/') || void {
            continue;
        }
        // What's in `<script>` and `<style>` isn't markup.
        let lower = name.to_ascii_lowercase();
        if mode == TagMode::Html && (lower == "script" || lower == "style") {
            let close = format!("</{lower}");
            match rest.to_ascii_lowercase().find(&close) {
                Some(end) => {
                    rest = &rest[end..];
                    continue;
                }
                None => {
                    open.push(name);
                    break;
                }
            }
        }
        open.push(name);
    }
    open
}

/// Whether the end of `before` is inside `<script>` or `<style>`, whose text isn't markup.
fn in_raw_text(before: &str, mode: TagMode) -> bool {
    mode == TagMode::Html
        && open_elements(before, mode)
            .last()
            .is_some_and(|n| matches!(n.to_ascii_lowercase().as_str(), "script" | "style"))
}

/// What typing `c` at a cursor between `before` and `after` becomes, if more than `c`.
/// `quotes` turns on `=""`.
pub fn on_type(mode: TagMode, c: char, before: &str, after: &str, quotes: bool) -> Option<Typed> {
    if mode == TagMode::Off {
        return None;
    }
    match c {
        '>' => {
            let body = open_tag_body(before)?;
            if body.starts_with(['/', '!', '?']) || body.ends_with('/') || !balanced(body) {
                return None;
            }
            let lt = before.len() - body.len() - 1;
            if mode == TagMode::Jsx && !jsx_can_start(&before[..lt]) {
                return None;
            }
            let name: String = body.chars().take_while(|c| name_char(*c)).collect();
            // A tag's name ends at a space or the tag's end: `i<n;` in code isn't one.
            if !body[name.len()..].chars().next().is_none_or(char::is_whitespace) {
                return None;
            }
            if in_raw_text(&before[..lt], mode) {
                return None;
            }
            // `<>` is a fragment, in JSX only.
            if name.is_empty() && !(mode == TagMode::Jsx && body.is_empty()) {
                return None;
            }
            if !name.is_empty() && !name.starts_with(|c: char| c.is_alphabetic()) {
                return None;
            }
            if mode == TagMode::Html && VOID.contains(&name.to_ascii_lowercase().as_str()) {
                return None;
            }
            let end = format!("</{name}>");
            if after.starts_with(&end) {
                return None;
            }
            Some(Typed { text: format!(">{end}"), cursor: 1 })
        }
        '/' => {
            let before = before.strip_suffix('<')?;
            if mode == TagMode::Jsx && !jsx_can_start(before) {
                return None;
            }
            let name = open_elements(before, mode).pop()?;
            // `</` in a script's code is a comparison and a regex, not a tag... except `</script`.
            if mode == TagMode::Html
                && !matches!(name.to_ascii_lowercase().as_str(), "script" | "style")
                && in_raw_text(before, mode)
            {
                return None;
            }
            let text = format!("/{name}>");
            // A `>` already there is the tag's own.
            let text = if after.starts_with('>') { text[..text.len() - 1].to_owned() } else { text };
            let cursor = text.chars().count();
            Some(Typed { text, cursor })
        }
        '=' if quotes && mode != TagMode::Jsx => {
            let body = open_tag_body(before)?;
            // Past the tag's name, right after an attribute's.
            let named = body.contains(char::is_whitespace) && body.ends_with(name_char);
            if !named || body.starts_with(['/', '!', '?']) || !balanced(body) || after.starts_with(['"', '\'']) {
                return None;
            }
            Some(Typed { text: "=\"\"".to_owned(), cursor: 2 })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::TagMode::*;
    use super::*;

    fn typed(mode: TagMode, c: char, before: &str, after: &str) -> Option<String> {
        on_type(mode, c, before, after, true).map(|t| {
            let at = t.text.char_indices().nth(t.cursor).map_or(t.text.len(), |(i, _)| i);
            format!("{}|{}", &t.text[..at], &t.text[at..])
        })
    }

    #[test]
    fn a_start_tag_gets_its_end() {
        assert_eq!(typed(Html, '>', "<div class=\"a\"", "").as_deref(), Some(">|</div>"));
        assert_eq!(typed(Html, '>', "<ul>\n  <li", "").as_deref(), Some(">|</li>"));
        assert_eq!(typed(Xml, '>', "<Project", "").as_deref(), Some(">|</Project>"));
        // Void elements, self-closing tags, end tags, comments and doctypes don't.
        for before in ["<br", "<img src=\"a\"", "<div/", "</div", "<!-- x", "<!doctype html", "<?xml version=\"1.0\"?"]
        {
            assert_eq!(typed(Html, '>', before, ""), None, "{before}");
        }
        assert_eq!(typed(Xml, '>', "<br", "").as_deref(), Some(">|</br>"), "XML has no void elements");
        // A `>` inside a quoted value, or with the end tag already there.
        assert_eq!(typed(Html, '>', "<a title=\"x", ""), None);
        assert_eq!(typed(Html, '>', "<p", "</p>"), None);
        // Not in plain text, nor with the mode off.
        assert_eq!(typed(Html, '>', "a ", ""), None);
        // Code in a script: `i<n` is no tag, and nothing there is completed.
        assert_eq!(typed(Html, '>', "<script>for (i=0;i<n;i++) { a ", ""), None);
        assert_eq!(typed(Html, '>', "<script>const t = '<b", ""), None);
        assert_eq!(typed(Html, '/', "<div><script>x = 1;<", "").as_deref(), Some("/script>|"));
        assert_eq!(typed(Off, '>', "<div", ""), None);
    }

    #[test]
    fn jsx_tags_only_where_an_element_can_start() {
        assert_eq!(typed(Jsx, '>', "return <div className=\"a\"", "").as_deref(), Some(">|</div>"));
        assert_eq!(typed(Jsx, '>', "const x = (\n  <Card title={t}", "").as_deref(), Some(">|</Card>"));
        assert_eq!(typed(Jsx, '>', "return <", "").as_deref(), Some(">|</>"), "a fragment");
        assert_eq!(typed(Jsx, '>', "const xs: Array<string", ""), None, "a type argument");
        assert_eq!(typed(Jsx, '>', "useState<number", ""), None);
        assert_eq!(typed(Jsx, '>', "<button onClick={() =", ""), None, "an arrow inside braces");
        assert_eq!(typed(Jsx, '=', "<div className", ""), None, "JSX values are often braces");
    }

    #[test]
    fn slash_finishes_the_innermost_open_element() {
        assert_eq!(typed(Html, '/', "<ul>\n  <li>a</li>\n<", "").as_deref(), Some("/ul>|"));
        assert_eq!(typed(Html, '/', "<div><p>x<br><img src=\"a\"><", "").as_deref(), Some("/p>|"));
        assert_eq!(typed(Html, '/', "<div><!-- <span> --><", "").as_deref(), Some("/div>|"));
        assert_eq!(typed(Html, '/', "<div><script>if (a<b) {}</script><", "").as_deref(), Some("/div>|"));
        assert_eq!(typed(Html, '/', "<p>done</p><", ""), None, "nothing open");
        assert_eq!(typed(Html, '/', "<div><", ">").as_deref(), Some("/div|"), "its `>` is there");
        assert_eq!(typed(Html, '/', "a /", ""), None);
        assert_eq!(typed(Jsx, '/', "return (<Card><", "").as_deref(), Some("/Card>|"));
    }

    #[test]
    fn equals_after_an_attribute_name_gets_quotes() {
        assert_eq!(typed(Html, '=', "<div class", "").as_deref(), Some("=\"|\""));
        assert_eq!(typed(Html, '=', "<input type=\"text\" name", ">").as_deref(), Some("=\"|\""));
        assert_eq!(typed(Html, '=', "<div", ""), None, "the tag's name");
        assert_eq!(typed(Html, '=', "<a href=\"x?y", ""), None, "inside a value");
        assert_eq!(typed(Html, '=', "<div class", "\"a\""), None, "the quote is there");
        assert_eq!(typed(Html, '=', "a = b", ""), None);
        assert_eq!(on_type(Html, '=', "<div class", "", false), None, "quotes turned off");
    }

    #[test]
    fn modes_by_extension() {
        assert_eq!(TagMode::for_extension("html"), Html);
        assert_eq!(TagMode::for_extension("csproj"), Xml);
        assert_eq!(TagMode::for_extension("tsx"), Jsx);
        assert_eq!(TagMode::for_extension("ts"), Off);
    }
}
