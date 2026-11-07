//! When a new line goes in a level, comes out a level, or a typed word or closer moves
//! its line back: per family of languages, like VS Code's `indentationRules`.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndentRules {
    /// C, Rust, Java, JavaScript, CSS, JSON and the rest: blocks are brackets.
    #[default]
    Brackets,
    Python,
    /// HTML and XML: tags, with brackets for what's inside `<script>` and `<style>`.
    Markup,
    Ruby,
    Lua,
    Shell,
    Elixir,
    /// YAML: a key ending in `:` opens a level.
    Yaml,
    /// Prose (Markdown, plain text, CSV, logs): the indentation is kept, nothing more.
    Plain,
}

use crate::language::html_attributes::VOID;

fn first_word(line: &str) -> &str {
    line.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("")
}

/// The line without a trailing `#` comment, unless the `#` is inside quotes.
fn without_hash_comment(line: &str) -> &str {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '#') => return line[..i].trim_end(),
            _ => {}
        }
    }
    line
}

fn ends_with_word(line: &str, word: &str) -> bool {
    line.strip_suffix(word)
        .is_some_and(|rest| rest.is_empty() || rest.ends_with(|c: char| !c.is_alphanumeric() && c != '_'))
}

/// The tag a line ends inside of: the name of a start tag at its end, left open.
pub fn open_tag(before: &str) -> Option<String> {
    let start = before.rfind('<')?;
    let tag = &before[start..];
    if !tag.ends_with('>') || tag.ends_with("/>") {
        return None;
    }
    let name: String = tag[1..].chars().take_while(|c| c.is_alphanumeric() || matches!(c, '-' | ':' | '_')).collect();
    let lower = name.to_ascii_lowercase();
    if name.is_empty() || VOID.contains(&lower.as_str()) {
        return None;
    }
    Some(name)
}

impl IndentRules {
    pub fn for_extension(extension: &str) -> Self {
        match extension {
            "py" | "pyw" | "pyi" => Self::Python,
            "yaml" | "yml" => Self::Yaml,
            "html" | "htm" | "xhtml" | "xml" | "svg" | "xaml" | "csproj" | "fsproj" | "vbproj" | "props"
            | "targets" | "plist" | "vue" | "svelte" | "jinja" | "jinja2" | "j2" | "njk" | "hbs" | "handlebars"
            | "ejs" | "twig" | "liquid" => Self::Markup,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "lua" => Self::Lua,
            "sh" | "bash" | "zsh" => Self::Shell,
            "ex" | "exs" => Self::Elixir,
            "md" | "markdown" | "txt" | "text" | "csv" | "tsv" | "log" | "rst" | "adoc" => Self::Plain,
            _ => Self::Brackets,
        }
    }

    /// Whether the line after one ending in `before` (the text before the cursor, with
    /// trailing spaces off) goes in a level.
    pub fn opens(self, before: &str) -> bool {
        let brackets = before.ends_with(['{', '(', '[']);
        let trimmed = before.trim_start();
        match self {
            Self::Brackets => brackets,
            Self::Plain => false,
            Self::Python => {
                let code = without_hash_comment(before);
                code.ends_with(':') || code.ends_with(['{', '(', '['])
            }
            Self::Yaml => before.ends_with(':') || before.ends_with(": |") || before.ends_with(": >"),
            Self::Markup => brackets || open_tag(before).is_some(),
            Self::Ruby => {
                let code = without_hash_comment(before);
                matches!(
                    first_word(code.trim_start()),
                    "def"
                        | "class"
                        | "module"
                        | "if"
                        | "unless"
                        | "while"
                        | "until"
                        | "case"
                        | "begin"
                        | "for"
                        | "else"
                        | "elsif"
                        | "when"
                        | "rescue"
                        | "ensure"
                ) && !ends_with_word(code, "end")
                    || ends_with_word(code, "do")
                    || code.trim_end_matches(|c: char| c != '|').ends_with("do |")
                    || code.contains(" do |") && code.ends_with('|')
                    || code.ends_with(['{', '(', '['])
            }
            Self::Lua => {
                (ends_with_word(before, "then") || ends_with_word(before, "do") || ends_with_word(before, "else"))
                    || first_word(trimmed) == "repeat"
                    || (trimmed.contains("function") && before.ends_with(')') && !ends_with_word(before, "end"))
                    || brackets
            }
            Self::Shell => {
                let code = without_hash_comment(before);
                ends_with_word(code, "then")
                    || ends_with_word(code, "do")
                    || ends_with_word(code, "else")
                    || code.ends_with('{')
                    || (first_word(code.trim_start()) == "case" && ends_with_word(code, "in"))
                    || code.ends_with(')') && !code.contains('(')
            }
            Self::Elixir => {
                let code = without_hash_comment(before);
                ends_with_word(code, "do") || code.ends_with("->") || ends_with_word(code, "else") || brackets
            }
        }
    }

    /// Whether the line after this one comes out a level: Python after `return` and the like.
    pub fn ends_block(self, before: &str) -> bool {
        self == Self::Python
            && matches!(first_word(before.trim_start()), "return" | "pass" | "break" | "continue" | "raise")
    }

    /// Whether a line reading `line` (trimmed) belongs a level out from the lines above
    /// it: `else:`, `end`, `fi`. Python's is checked as its `:` is typed; the others once
    /// the word is finished, so that `ending` isn't taken for `end`.
    /// Whether the line is complete as typed: Python's ends in `:`, others need the word ended.
    pub fn dedents_on_colon(self) -> bool {
        self == Self::Python
    }

    pub fn dedents(self, line: &str) -> bool {
        let words: &[&str] = match self {
            Self::Python => {
                let code = without_hash_comment(line);
                return code.ends_with(':') && matches!(first_word(code), "else" | "elif" | "except" | "finally");
            }
            Self::Ruby => &["end", "else", "elsif", "when", "rescue", "ensure"],
            Self::Lua => &["end", "else", "elseif", "until"],
            Self::Shell => &["fi", "done", "esac", "else", "elif"],
            Self::Elixir => &["end", "else"],
            Self::Brackets | Self::Markup | Self::Yaml | Self::Plain => return false,
        };
        words.contains(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::IndentRules::*;
    use super::*;

    #[test]
    fn what_opens_a_level() {
        assert!(Brackets.opens("fn main() {"));
        assert!(!Brackets.opens("let x = 1;"));
        assert!(Python.opens("def f(x):"));
        assert!(Python.opens("if x:  # check"));
        assert!(!Python.opens("x = '#:'"));
        assert!(Python.opens("items = ["));
        assert!(Yaml.opens("server:"));
        assert!(!Yaml.opens("port: 80"));
        assert!(Markup.opens("<div class=\"a\">"));
        assert!(Markup.opens("  <ul>"));
        assert!(!Markup.opens("<br>"));
        assert!(!Markup.opens("<img src=\"a\"/>"));
        assert!(!Markup.opens("<p>text</p>"));
        assert!(Markup.opens("function f() {"));
        assert!(Ruby.opens("def name"));
        assert!(Ruby.opens("items.each do |item|"));
        assert!(Ruby.opens("loop do"));
        assert!(!Ruby.opens("def name = 1 end"));
        assert!(!Ruby.opens("puts 'done'"));
        assert!(Lua.opens("if x then"));
        assert!(Lua.opens("for i = 1, 3 do"));
        assert!(Lua.opens("local function f(a)"));
        assert!(Shell.opens("if [ -f x ]; then"));
        assert!(Shell.opens("for f in *; do"));
        assert!(Shell.opens("case $x in"));
        assert!(Elixir.opens("defmodule A do"));
    }

    #[test]
    fn what_comes_out_a_level() {
        assert!(Python.ends_block("    return x"));
        assert!(Python.ends_block("pass"));
        assert!(!Python.ends_block("returned = 1"));
        assert!(Python.dedents("else:"));
        assert!(Python.dedents("elif x > 1:"));
        assert!(Python.dedents("except ValueError as e:"));
        assert!(!Python.dedents("else_value:"));
        assert!(!Python.dedents("elsewhere = 1"));
        assert!(Ruby.dedents("end"));
        assert!(!Ruby.dedents("ending"));
        assert!(Shell.dedents("fi"));
        assert!(Lua.dedents("until"));
        assert!(!Brackets.dedents("end"));
    }

    #[test]
    fn open_tags() {
        assert_eq!(open_tag("<section id=\"a\">").as_deref(), Some("section"));
        assert_eq!(open_tag("<my-card>").as_deref(), Some("my-card"));
        assert_eq!(open_tag("text"), None);
        assert_eq!(open_tag("<input type=\"x\">"), None);
        assert_eq!(for_ext("tsx"), Brackets);
        assert_eq!(for_ext("py"), Python);
        assert_eq!(for_ext("vue"), Markup);
        assert_eq!(for_ext("md"), Plain);
        assert!(!Plain.opens("list:") && !Plain.opens("("));
    }

    fn for_ext(e: &str) -> IndentRules {
        IndentRules::for_extension(e)
    }
}
