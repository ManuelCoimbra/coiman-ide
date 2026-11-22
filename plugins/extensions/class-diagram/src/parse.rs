//! Reads the types a source file declares: classes, interfaces, enums, structs, traits,
//! what each extends or implements, its fields (with the types they name) and its methods.
//! Read from the text alone, without a compiler: comments and strings are blanked first,
//! then declarations are found by their keywords and braces (indentation for Python).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Java,
    Kotlin,
    CSharp,
    TypeScript,
    Cpp,
    Swift,
    Dart,
    Scala,
    Php,
    Python,
    Rust,
    Go,
}

impl Lang {
    /// By the file's extension.
    pub fn of(path: &str) -> Option<Lang> {
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        Some(match ext.as_str() {
            "java" => Lang::Java,
            "kt" | "kts" => Lang::Kotlin,
            "cs" => Lang::CSharp,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Lang::TypeScript,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "h" | "ipp" => Lang::Cpp,
            "swift" => Lang::Swift,
            "dart" => Lang::Dart,
            "scala" | "sc" => Lang::Scala,
            "php" => Lang::Php,
            "py" | "pyi" => Lang::Python,
            "rs" => Lang::Rust,
            "go" => Lang::Go,
            _ => return None,
        })
    }

    /// Parameters and fields are written `name: Type` (else `Type name`).
    fn name_first(self) -> bool {
        matches!(
            self,
            Lang::Kotlin | Lang::TypeScript | Lang::Swift | Lang::Scala | Lang::Python | Lang::Rust | Lang::Go
        )
    }

    /// The visibility a member has when nothing is written.
    fn default_visibility(self) -> char {
        match self {
            Lang::Java | Lang::Swift => '~',
            Lang::CSharp | Lang::Cpp => '-',
            _ => '+',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rel {
    Extends,
    Implements,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// "+ name: Type".
    pub text: String,
    /// Names in its type that may be types of the project.
    pub types: Vec<String>,
    /// Go's embedded struct.
    pub embedded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub name: String,
    /// With its generic parameters: `Box<T>`.
    pub label: String,
    /// "class", "interface", "enum", "abstract", "struct", "trait", "record", "object"...
    pub kind: &'static str,
    pub parents: Vec<(String, Rel)>,
    pub fields: Vec<Field>,
    pub methods: Vec<String>,
    /// 1-based.
    pub line: u32,
    /// Package, namespace or module, else "".
    pub group: String,
}

impl Type {
    fn new(name: &str, label: String, kind: &'static str, line: u32) -> Self {
        Type {
            name: name.to_owned(),
            label,
            kind,
            parents: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            line,
            group: String::new(),
        }
    }
}

/// Every type `text` declares.
pub fn parse(lang: Lang, text: &str) -> Vec<Type> {
    let clean = strip(text, lang);
    let mut types = match lang {
        Lang::Python => python(&clean),
        Lang::Rust => rust(&clean),
        Lang::Go => go(&clean),
        _ => c_like(lang, &clean),
    };
    let group = group_of(lang, &clean);
    for t in &mut types {
        if t.group.is_empty() {
            t.group = group.clone();
        }
    }
    types
}

// ----- text --------------------------------------------------------------------------

/// The text with comments and string contents turned to spaces; lines stay where they were.
pub fn strip(text: &str, lang: Lang) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let hash_comments = matches!(lang, Lang::Python | Lang::Php);
    let slash_comments = lang != Lang::Python;
    let backtick = matches!(lang, Lang::TypeScript | Lang::Go | Lang::Kotlin);
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if slash_comments && c == '/' && next == Some('/') || hash_comments && c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if slash_comments && c == '/' && next == Some('*') {
            out.push_str("  ");
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                out.push(blank(chars[i]));
                i += 1;
            }
            out.push_str("  ");
            i += 2;
            continue;
        }
        // Python's triple quotes.
        if lang == Lang::Python && (c == '"' || c == '\'') && next == Some(c) && chars.get(i + 2) == Some(&c) {
            out.push_str(&c.to_string().repeat(3));
            i += 3;
            while i < chars.len() && !(chars[i] == c && chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c)) {
                out.push(blank(chars[i]));
                i += 1;
            }
            out.push_str(&c.to_string().repeat(3));
            i += 3;
            continue;
        }
        let quote = c == '"' || backtick && c == '`' || c == '\'' && single_quote_string(lang, &chars, i);
        if quote {
            out.push(c);
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < chars.len() {
                    out.push(blank(chars[i]));
                    i += 1;
                }
                // A plain string doesn't run past its line.
                if c != '`' && chars.get(i) == Some(&'\n') {
                    break;
                }
            }
            if i < chars.len() && chars[i] == c {
                out.push(c);
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Whether a `'` starts a string or character there: Rust's lifetimes (`'a`) don't.
fn single_quote_string(lang: Lang, chars: &[char], i: usize) -> bool {
    if lang != Lang::Rust {
        return true;
    }
    // 'x' or '\n'
    chars.get(i + 2) == Some(&'\'') || chars.get(i + 1) == Some(&'\\')
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The words of a piece of code (identifiers and numbers).
fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !is_ident(c)).filter(|w| !w.is_empty()).collect()
}

/// `a.b.Name<T>` → `Name`.
fn simple_name(s: &str) -> String {
    let s = s.trim();
    let base = s.split(['<', '[', '(']).next().unwrap_or(s).trim();
    base.rsplit(['.', ':', '\\']).next().unwrap_or(base).trim().to_owned()
}

/// Splits on `sep` outside brackets.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == sep && depth <= 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    parts.push(cur);
    parts.into_iter().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()
}

/// Names starting with a capital in a type: the ones that may be the project's types.
fn type_names(ty: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words(ty) {
        if w.chars().next().is_some_and(char::is_uppercase) && !out.iter().any(|o| o == w) {
            out.push(w.to_owned());
        }
    }
    out
}

fn line_of(text: &str, at: usize) -> u32 {
    text[..at].matches('\n').count() as u32 + 1
}

/// The package, namespace or module named at the top of a file.
fn group_of(lang: Lang, text: &str) -> String {
    let keyword = match lang {
        Lang::Java | Lang::Kotlin | Lang::Scala | Lang::Go | Lang::Dart => "package",
        Lang::CSharp | Lang::Php => "namespace",
        _ => return String::new(),
    };
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(keyword).filter(|r| r.starts_with(char::is_whitespace)) {
            return rest.split([';', '{']).next().unwrap_or("").trim().to_owned();
        }
    }
    String::new()
}

fn shorten(s: &str, most: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= most { s } else { format!("{}…", s.chars().take(most).collect::<String>()) }
}

// ----- languages with braces -----------------------------------------------------------

const DECL: [&str; 9] = ["class", "interface", "enum", "struct", "record", "trait", "object", "protocol", "union"];

/// What a scope is: a type (its index) or anything else.
#[derive(Clone, Copy)]
enum Scope {
    Type(usize),
    Other,
}

/// A declaration's keyword at `at`, if one starts there: the kind, and where its name begins.
fn declaration(lang: Lang, s: &str, at: usize) -> Option<(&'static str, usize)> {
    let before = s[..at].chars().next_back();
    if before.is_some_and(|c| is_ident(c) || c == '.' || c == ':' && lang != Lang::Cpp) {
        return None;
    }
    let rest = &s[at..];
    let word: String = rest.chars().take_while(|c| is_ident(*c)).collect();
    if !DECL.contains(&word.as_str()) {
        return None;
    }
    let after = &rest[word.len()..];
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    // The words before it on the line: `abstract class`, `data class`, `enum class`.
    let line_start = s[..at].rfind(['\n', ';', '{', '}']).map_or(0, |i| i + 1);
    let before_words = words(&s[line_start..at]);
    let mut kind: &'static str = match word.as_str() {
        "class" => "class",
        "interface" => "interface",
        "enum" => "enum",
        "struct" => "struct",
        "record" => "record",
        "trait" => "trait",
        "object" => "object",
        "protocol" => "protocol",
        _ => "union",
    };
    if lang == Lang::Cpp && kind == "union" {
        return None;
    }
    // `object` is Kotlin's and Scala's; `record` is Java's and C#'s.
    if kind == "object" && !matches!(lang, Lang::Kotlin | Lang::Scala) {
        return None;
    }
    if kind == "record" && !matches!(lang, Lang::Java | Lang::CSharp) {
        return None;
    }
    if kind == "trait" && !matches!(lang, Lang::Scala | Lang::Php) {
        return None;
    }
    if kind == "protocol" && lang != Lang::Swift {
        return None;
    }
    // A statement that only mentions the word (`return class`, `x = new class`).
    if before_words.iter().any(|w| matches!(*w, "return" | "new" | "typeof" | "instanceof" | "case")) {
        return None;
    }
    if before_words.contains(&"abstract") && kind == "class" {
        kind = "abstract";
    }
    if before_words.contains(&"enum") && kind == "class" {
        kind = "enum";
    }
    if before_words.contains(&"data") && kind == "class" && lang == Lang::Kotlin {
        kind = "data";
    }
    if before_words.contains(&"sealed") && kind == "class" {
        kind = "sealed";
    }
    if lang == Lang::Kotlin && before_words.contains(&"annotation") || before_words.contains(&"@interface") {
        kind = "annotation";
    }
    let name_at = at + word.len() + after.len() - after.trim_start().len();
    Some((kind, name_at))
}

/// The header of a declaration, from its name to where its body or end is: `{`, `;`, or a
/// line end once brackets are closed and the next line doesn't go on with it.
fn header_end(s: &str, from: usize) -> (usize, Option<usize>) {
    let mut depth = 0i32;
    for (off, c) in s[from..].char_indices() {
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth -= 1,
            '{' if depth <= 0 => return (from + off, Some(from + off)),
            ';' if depth <= 0 => return (from + off, None),
            '\n' if depth <= 0 => {
                let rest = s[from + off..].trim_start();
                let goes_on = rest.starts_with([':', ',', '{', '(', '<'])
                    || ["extends", "implements", "with", "where", "permits", "throws"]
                        .iter()
                        .any(|k| rest.starts_with(k) && rest[k.len()..].starts_with(char::is_whitespace));
                if !goes_on {
                    return (from + off, None);
                }
            }
            _ => {}
        }
        if off > 2000 {
            return (from + off, None);
        }
    }
    (s.len(), None)
}

/// The name, its label with generics, what follows it, and the parameter list right after
/// the name (Kotlin's and Scala's primary constructor, C#'s record).
fn split_header(header: &str) -> (String, String, String, String) {
    let name: String = header.chars().take_while(|c| is_ident(*c)).collect();
    let mut rest = &header[name.len()..];
    let mut label = name.clone();
    if rest.trim_start().starts_with('<') {
        let r = rest.trim_start();
        let mut depth = 0;
        let mut end = r.len();
        for (i, c) in r.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        label.push_str(&r[..end].split_whitespace().collect::<Vec<_>>().join(" "));
        rest = &r[end..];
    }
    let mut params = String::new();
    let trimmed = rest.trim_start();
    if trimmed.starts_with('(') {
        let mut depth = 0;
        for (i, c) in trimmed.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        params = trimmed[1..i].to_owned();
                        rest = &trimmed[i + 1..];
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    (name, label, rest.to_owned(), params)
}

/// What a type extends and implements, read from what follows its name.
fn parents(lang: Lang, kind: &str, after: &str) -> Vec<(String, Rel)> {
    let mut out = Vec::new();
    let mut push = |name: &str, rel: Rel| {
        let n = simple_name(name);
        let skip = ["public", "private", "protected", "virtual", "final", "out", "in"];
        let n = n.split_whitespace().rfind(|w| !skip.contains(w)).unwrap_or("").to_owned();
        if !n.is_empty()
            && n.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && !out.iter().any(|(o, _): &(String, Rel)| *o == n)
        {
            out.push((n, rel));
        }
    };
    let interface = matches!(kind, "interface" | "protocol" | "trait");
    match lang {
        Lang::Java | Lang::TypeScript | Lang::Dart | Lang::Php | Lang::Scala => {
            // extends A implements B, C  ·  Dart: with M  ·  Scala: extends A with B
            let mut mode: Option<Rel> = None;
            let mut buf = String::new();
            let flush = |buf: &mut String, mode: Option<Rel>, push: &mut dyn FnMut(&str, Rel)| {
                if let Some(rel) = mode {
                    for p in split_top(buf, ',') {
                        push(&p, rel);
                    }
                }
                buf.clear();
            };
            for w in split_keep(after) {
                match w.as_str() {
                    "extends" => {
                        flush(&mut buf, mode, &mut push);
                        mode = Some(Rel::Extends);
                    }
                    "implements" | "with" => {
                        flush(&mut buf, mode, &mut push);
                        mode = Some(if interface { Rel::Extends } else { Rel::Implements });
                    }
                    "permits" | "where" | "throws" => {
                        flush(&mut buf, mode, &mut push);
                        mode = None;
                    }
                    _ => buf.push_str(&w),
                }
            }
            flush(&mut buf, mode, &mut push);
        }
        Lang::CSharp | Lang::Kotlin | Lang::Swift | Lang::Cpp => {
            let Some(colon) = top_colon(after) else { return out };
            let list = after[colon + 1..].split(" where ").next().unwrap_or("");
            for (i, p) in split_top(list, ',').iter().enumerate() {
                let rel = if interface {
                    Rel::Extends
                } else {
                    match lang {
                        Lang::Cpp => Rel::Extends,
                        Lang::Kotlin => {
                            if p.contains('(') {
                                Rel::Extends
                            } else {
                                Rel::Implements
                            }
                        }
                        Lang::CSharp => {
                            let n = simple_name(p);
                            let iface =
                                n.len() > 1 && n.starts_with('I') && n.chars().nth(1).is_some_and(char::is_uppercase);
                            if i == 0 && !iface && kind != "struct" { Rel::Extends } else { Rel::Implements }
                        }
                        _ => {
                            if i == 0 && kind == "class" {
                                Rel::Extends
                            } else {
                                Rel::Implements
                            }
                        }
                    }
                };
                push(p, rel);
            }
        }
        _ => {}
    }
    out
}

/// The words and the rest of a text, keeping the separators, so keywords can be found.
fn split_keep(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if is_ident(c) {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The first `:` outside brackets that isn't part of `::`.
fn top_colon(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let b = s.as_bytes();
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ':' if depth <= 0 && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':') => return Some(i),
            _ => {}
        }
    }
    None
}

const NOT_METHODS: [&str; 16] = [
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "return",
    "new",
    "super",
    "this",
    "throw",
    "foreach",
    "using",
    "lock",
    "synchronized",
    "when",
    "sizeof",
];

const MODIFIERS: [&str; 28] = [
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "final",
    "abstract",
    "virtual",
    "override",
    "readonly",
    "const",
    "constexpr",
    "inline",
    "async",
    "open",
    "sealed",
    "lateinit",
    "volatile",
    "transient",
    "mutable",
    "explicit",
    "extern",
    "default",
    "native",
    "partial",
    "required",
    "unsafe",
    "fileprivate",
];

fn visibility(lang: Lang, ws: &[&str], name: &str, section: Option<char>) -> char {
    for w in ws {
        match *w {
            "public" | "open" => return '+',
            "private" | "fileprivate" => return '-',
            "protected" => return '#',
            "internal" => return '~',
            _ => {}
        }
    }
    if let Some(s) = section {
        return s;
    }
    if name.starts_with('#') || lang == Lang::Dart && name.starts_with('_') {
        return '-';
    }
    lang.default_visibility()
}

/// The parameter names of a parameter list, joined: `a, b`.
fn param_names(lang: Lang, params: &str) -> String {
    let names: Vec<String> = split_top(params, ',')
        .iter()
        .filter_map(|p| {
            let p = p.split('=').next().unwrap_or(p);
            let ws: Vec<&str> = words(p)
                .into_iter()
                .filter(|w| {
                    !MODIFIERS.contains(w)
                        && !matches!(*w, "val" | "var" | "let" | "ref" | "out" | "in" | "params" | "this" | "mut")
                })
                .collect();
            if lang.name_first() || p.contains(':') && !matches!(lang, Lang::Cpp | Lang::CSharp | Lang::Java) {
                ws.first().map(|w| (*w).to_owned())
            } else {
                ws.last().map(|w| (*w).to_owned())
            }
        })
        .filter(|n| n != "self" && n != "cls")
        .collect();
    names.join(", ")
}

/// A member statement read inside a type's body: a method or a field, if it is one.
fn member(lang: Lang, stmt: &str, type_name: &str, section: Option<char>, t: &mut Type) {
    let mut s = stmt.trim().to_owned();
    // Annotations and attributes go: @Override, @Column(name = "x"), [Required].
    while s.starts_with('@') || s.starts_with('[') {
        let mut depth = 0;
        let mut end = s.len();
        for (i, c) in s.char_indices().skip(1) {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => {
                    depth -= 1;
                    if depth < 0 || (depth == 0 && s.starts_with('[') && c == ']') {
                        end = i + 1;
                        break;
                    }
                }
                c if depth == 0 && s.starts_with('@') && (c.is_whitespace()) => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        s = s[end.min(s.len())..].trim_start().to_owned();
        if end == 0 {
            break;
        }
    }
    if s.is_empty() {
        return;
    }
    let first = words(&s).first().copied().unwrap_or("");
    if matches!(
        first,
        "return"
            | "import"
            | "package"
            | "using"
            | "case"
            | "default"
            | "typedef"
            | "friend"
            | "namespace"
            | "init"
            | "get"
            | "set"
            | "static_assert"
            | "template"
    ) && !s.contains('(')
    {
        return;
    }
    let paren = s.find('(');
    let eq = s.find('=');
    let arrow_fn = eq.is_some_and(|e| s[e..].contains("=>") && s[e..].contains('(')) && lang == Lang::TypeScript;
    let is_method = paren.is_some_and(|p| eq.is_none_or(|e| p < e)) || arrow_fn;
    if is_method {
        let p = paren.unwrap_or(0);
        let head = &s[..p];
        let hw = words(head);
        let Some(name) = hw.last().copied() else { return };
        if NOT_METHODS.contains(&name) || hw.iter().any(|w| matches!(*w, "new" | "return")) || head.contains('~') {
            return;
        }
        let name = if name == type_name || name == "constructor" || name == "init" || name == "__init__" {
            type_name.to_owned()
        } else {
            name.to_owned()
        };
        let mut depth = 0;
        let mut close = s.len();
        for (i, c) in s[p..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = p + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let params = if close > p { &s[p + 1..close] } else { "" };
        let vis = visibility(lang, &hw, &name, section);
        let text = format!("{vis} {name}({})", shorten(&param_names(lang, params), 40));
        if !t.methods.contains(&text) {
            t.methods.push(text);
        }
        return;
    }
    // A field.
    let decl = s.split('=').next().unwrap_or(&s).trim().trim_end_matches([',', ';']).trim();
    let ws: Vec<&str> = words(decl);
    if ws.is_empty() {
        return;
    }
    let (name, ty) = if lang.name_first()
        || decl.contains(':') && !matches!(lang, Lang::Cpp | Lang::CSharp | Lang::Java)
    {
        let (left, right) = decl.split_once(':').unwrap_or((decl, ""));
        let lw: Vec<&str> = words(left)
            .into_iter()
            .filter(|w| {
                !MODIFIERS.contains(w)
                    && !matches!(*w, "val" | "var" | "let" | "readonly" | "declare" | "weak" | "lazy" | "override")
            })
            .collect();
        let Some(name) = lw.last() else { return };
        let name = if left.contains(&format!("#{name}")) { format!("#{name}") } else { (*name).to_owned() };
        (name, right.trim().to_owned())
    } else {
        // Type name: the last word is the name, what comes before it the type.
        let kept: Vec<&str> = ws.iter().copied().filter(|w| !MODIFIERS.contains(w)).collect();
        if kept.len() < 2 {
            return; // enum constants, statements
        }
        let name = kept[kept.len() - 1];
        let at = decl.rfind(name).unwrap_or(0);
        let ty_text = decl[..at].split_whitespace().filter(|w| !MODIFIERS.contains(w)).collect::<Vec<_>>().join(" ");
        (name.to_owned(), ty_text)
    };
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '#' || c == '$') {
        return;
    }
    let vis = visibility(lang, &ws, &name, section);
    let text = if ty.is_empty() { format!("{vis} {name}") } else { format!("{vis} {name}: {}", shorten(&ty, 40)) };
    if !t.fields.iter().any(|f| f.text == text) {
        t.fields.push(Field { text, types: type_names(&ty), embedded: false });
    }
}

fn c_like(lang: Lang, s: &str) -> Vec<Type> {
    let mut types: Vec<Type> = Vec::new();
    let mut scopes: Vec<Scope> = Vec::new();
    // Inside a type's own body: the statement being read, its bracket depth, and C++'s
    // `public:` sections.
    let mut stmt = String::new();
    let mut paren = 0i32;
    let mut sections: Vec<Option<char>> = Vec::new();
    let mut pending: Option<usize> = None;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        if !s.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let c = s[i..].chars().next().unwrap_or(' ');
        // A declaration starts.
        if is_ident(c)
            && (i == 0 || !is_ident(s[..i].chars().next_back().unwrap_or(' ')))
            && let Some((kind, name_at)) = declaration(lang, s, i)
        {
            {
                let (end, brace) = header_end(s, name_at);
                let header = &s[name_at..end];
                let (name, label, after, params) = split_header(header);
                if !name.is_empty() {
                    let mut t = Type::new(&name, label, kind, line_of(s, i));
                    t.parents = parents(lang, kind, &after);
                    // Kotlin's and Scala's primary constructor, C#'s and Java's record.
                    if !params.trim().is_empty()
                        && matches!(lang, Lang::Kotlin | Lang::Scala | Lang::CSharp | Lang::Java)
                    {
                        for p in split_top(&params, ',') {
                            let keeps = matches!(lang, Lang::CSharp | Lang::Java)
                                || words(&p).iter().any(|w| *w == "val" || *w == "var");
                            if keeps {
                                member(lang, &p, &name, None, &mut t);
                            }
                        }
                    }
                    // Nested in a type: named after it.
                    if let Some(Scope::Type(outer)) = scopes.last() {
                        t.group = types[*outer].group.clone();
                    }
                    types.push(t);
                    let index = types.len() - 1;
                    match brace {
                        Some(b) => {
                            i = b;
                            pending = Some(index);
                        }
                        None => {
                            i = end;
                            continue;
                        }
                    }
                    stmt.clear();
                    paren = 0;
                }
            }
        }
        let c = s[i..].chars().next().unwrap_or(' ');
        let in_type = matches!(scopes.last(), Some(Scope::Type(_)));
        match c {
            '{' => {
                if let Some(t) = pending.take() {
                    scopes.push(Scope::Type(t));
                    // What members are when nothing is written: C++'s struct and class differ,
                    // and an interface's members are public.
                    let section = if lang == Lang::Cpp {
                        Some(if types[t].kind == "struct" { '+' } else { '-' })
                    } else if matches!(types[t].kind, "interface" | "protocol" | "annotation") {
                        Some('+')
                    } else {
                        None
                    };
                    sections.push(section);
                    stmt.clear();
                    paren = 0;
                } else {
                    if in_type && paren == 0 {
                        // A method body, a property's accessors, an initializer.
                        if let Some(Scope::Type(t)) = scopes.last() {
                            let t = *t;
                            let section = sections.last().copied().flatten();
                            let text = stmt.clone();
                            let name = types[t].name.clone();
                            member(lang, &text, &name, section, &mut types[t]);
                        }
                        stmt.clear();
                    }
                    scopes.push(Scope::Other);
                }
            }
            '}' => {
                if let Some(Scope::Type(t)) = scopes.last()
                    && !stmt.trim().is_empty()
                {
                    let t = *t;
                    let section = sections.last().copied().flatten();
                    let (text, name) = (stmt.clone(), types[t].name.clone());
                    member(lang, &text, &name, section, &mut types[t]);
                }
                if let Some(Scope::Type(_)) = scopes.pop() {
                    sections.pop();
                }
                stmt.clear();
                paren = 0;
            }
            _ if in_type => {
                match c {
                    '(' | '[' => paren += 1,
                    ')' | ']' => paren -= 1,
                    _ => {}
                }
                let ends = c == ';'
                    || c == ',' && paren == 0 && types_is_enum(&types, &scopes)
                    || c == '\n'
                        && paren <= 0
                        && matches!(lang, Lang::Kotlin | Lang::TypeScript | Lang::Swift | Lang::Scala | Lang::Dart)
                        && !stmt.trim_end().ends_with([',', '(', '=', ':', '|', '&'])
                        && !next_line_goes_on(s, i);
                if ends {
                    let text = std::mem::take(&mut stmt);
                    if let Some(Scope::Type(t)) = scopes.last() {
                        let t = *t;
                        let trimmed = text.trim();
                        // C++'s access sections.
                        if lang == Lang::Cpp && matches!(trimmed, "public:" | "private:" | "protected:") {
                        } else if !types_is_enum(&types, &scopes) || !enum_constant(&text) {
                            let section = sections.last().copied().flatten();
                            let name = types[t].name.clone();
                            member(lang, &text, &name, section, &mut types[t]);
                        }
                    }
                    paren = 0;
                } else if lang == Lang::Cpp
                    && c == ':'
                    && bytes.get(i + 1) != Some(&b':')
                    && (i == 0 || bytes[i - 1] != b':')
                {
                    let t = stmt.trim();
                    let sec = match t {
                        "public" => Some('+'),
                        "private" => Some('-'),
                        "protected" => Some('#'),
                        _ => None,
                    };
                    if let Some(sec) = sec {
                        if let Some(last) = sections.last_mut() {
                            *last = Some(sec);
                        }
                        stmt.clear();
                    } else {
                        stmt.push(c);
                    }
                } else {
                    stmt.push(c);
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    types
}

/// Whether the innermost scope is an enum's body: its constants end with `,`.
fn types_is_enum(types: &[Type], scopes: &[Scope]) -> bool {
    matches!(scopes.last(), Some(Scope::Type(t)) if types[*t].kind == "enum")
}

/// An enum's constant: `RED`, `GREEN("g")`, `BLUE { … }`: one word before its arguments.
fn enum_constant(stmt: &str) -> bool {
    let head = stmt.split(['(', '{', '=']).next().unwrap_or(stmt);
    let w = words(head);
    w.len() <= 1 && !stmt.trim_start().starts_with('@')
}

/// Whether the line after `at` (a line end) continues the statement: `.call()`, `?:`...
fn next_line_goes_on(s: &str, at: usize) -> bool {
    let rest = s[at + 1..].trim_start();
    rest.starts_with(['.', '?', ':', '=', '+', '-', '*', '/', '|', '&', ')', ']']) && !rest.starts_with("//")
}

// ----- Python ---------------------------------------------------------------------------

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn python(s: &str) -> Vec<Type> {
    let lines: Vec<&str> = s.lines().collect();
    let mut types = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("class ") else {
            i += 1;
            continue;
        };
        let class_indent = indent(line);
        let name: String = rest.chars().take_while(|c| is_ident(*c)).collect();
        if name.is_empty() {
            i += 1;
            continue;
        }
        // The bases, maybe over several lines.
        let mut header = rest[name.len()..].to_owned();
        let mut j = i;
        while !header.contains(':') && j + 1 < lines.len() {
            j += 1;
            header.push_str(lines[j]);
        }
        let mut ty = Type::new(&name, name.clone(), "class", i as u32 + 1);
        if let (Some(open), Some(close)) = (header.find('('), header.rfind(')')) {
            for base in split_top(&header[open + 1..close], ',') {
                if base.contains('=') {
                    continue; // metaclass=…
                }
                let b = simple_name(&base);
                match b.as_str() {
                    "object" | "" => {}
                    "ABC" => ty.kind = "abstract",
                    "Protocol" => ty.kind = "interface",
                    "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" => ty.kind = "enum",
                    _ => ty.parents.push((b, Rel::Extends)),
                }
            }
        }
        // The body: lines indented more than `class`.
        let mut body_indent = None;
        let mut k = j + 1;
        while k < lines.len() {
            let l = lines[k];
            if l.trim().is_empty() {
                k += 1;
                continue;
            }
            let ind = indent(l);
            if ind <= class_indent {
                break;
            }
            let bi = *body_indent.get_or_insert(ind);
            let lt = l.trim();
            if ind == bi {
                if let Some(def) = lt.strip_prefix("def ").or_else(|| lt.strip_prefix("async def ")) {
                    let mname: String = def.chars().take_while(|c| is_ident(*c)).collect();
                    let params =
                        def.find('(').and_then(|o| def.rfind(')').map(|c| &def[o + 1..c.max(o + 1)])).unwrap_or("");
                    let shown = if mname == "__init__" { name.clone() } else { mname.clone() };
                    let vis = if mname.starts_with("__") && !mname.ends_with("__") {
                        '-'
                    } else if mname.starts_with('_') && !mname.ends_with("__") {
                        '#'
                    } else {
                        '+'
                    };
                    if !mname.starts_with("__") || mname == "__init__" {
                        ty.methods.push(format!("{vis} {shown}({})", shorten(&param_names(Lang::Python, params), 40)));
                    }
                } else if !lt.starts_with('@') && !lt.starts_with("class ") {
                    // Class-level `name: Type = …` or `NAME = …`.
                    let decl = lt.split('=').next().unwrap_or(lt);
                    let (fname, fty) = decl.split_once(':').map_or((decl.trim(), ""), |(a, b)| (a.trim(), b.trim()));
                    if !fname.is_empty() && fname.chars().all(is_ident) && (lt.contains('=') || lt.contains(':')) {
                        let vis = if fname.starts_with('_') { '-' } else { '+' };
                        let text =
                            if fty.is_empty() { format!("{vis} {fname}") } else { format!("{vis} {fname}: {fty}") };
                        if !ty.fields.iter().any(|f| f.text == text) {
                            ty.fields.push(Field { text, types: type_names(fty), embedded: false });
                        }
                    }
                }
            } else if let Some(at) = lt.find("self.") {
                // `self.name = …` (or `self.name: Type = …`) in a method.
                let after = &lt[at + 5..];
                let fname: String = after.chars().take_while(|c| is_ident(*c)).collect();
                let rest = after[fname.len()..].trim_start();
                if !fname.is_empty()
                    && at == 0
                    && (rest.starts_with('=') && !rest.starts_with("==") || rest.starts_with(':'))
                {
                    let fty = rest.strip_prefix(':').map(|r| r.split('=').next().unwrap_or("").trim()).unwrap_or("");
                    let vis = if fname.starts_with('_') { '-' } else { '+' };
                    let text = if fty.is_empty() { format!("{vis} {fname}") } else { format!("{vis} {fname}: {fty}") };
                    let known = ty
                        .fields
                        .iter()
                        .any(|f| f.text.split(':').next().map(str::trim) == Some(&format!("{vis} {fname}")));
                    if !known {
                        ty.fields.push(Field { text, types: type_names(fty), embedded: false });
                    }
                }
            }
            k += 1;
        }
        types.push(ty);
        i = j + 1;
    }
    types
}

// ----- Rust -----------------------------------------------------------------------------

/// The text between the brace at `open` and its match.
fn braced(s: &str, open: usize) -> &str {
    let mut depth = 0;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &s[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    &s[open + 1..]
}

/// The items at the top level of a block: each from its start to its `;` or closing brace.
fn top_items(body: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in body.chars() {
        match c {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
        if depth == 0 && (c == ';' || c == ',' || c == '}') {
            if c == '}' {
                cur.push(c);
            }
            items.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    items.push(cur);
    items.into_iter().map(|i| i.trim().to_owned()).filter(|i| !i.is_empty()).collect()
}

fn rust_find<'a>(types: &'a mut Vec<Type>, name: &str) -> &'a mut Type {
    if let Some(i) = types.iter().position(|t| t.name == name) {
        return &mut types[i];
    }
    types.push(Type::new(name, name.to_owned(), "struct", 0));
    types.last_mut().expect("just pushed")
}

fn rust(s: &str) -> Vec<Type> {
    let mut types: Vec<Type> = Vec::new();
    let mut impls: Vec<(String, Option<String>, usize)> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let Some(off) = s[i..].find(|c: char| is_ident(c)) else { break };
        let at = i + off;
        let word: String = s[at..].chars().take_while(|c| is_ident(*c)).collect();
        let before_ok = at == 0 || !is_ident(s[..at].chars().next_back().unwrap_or(' '));
        let after = at + word.len();
        i = after;
        if !before_ok || !matches!(word.as_str(), "struct" | "enum" | "trait" | "impl" | "union") {
            continue;
        }
        let (end, brace) = header_end(s, after);
        let header = s[after..end].trim();
        let line = line_of(s, at);
        match word.as_str() {
            "impl" => {
                // impl<T> Trait<T> for Type<T> where … { … }
                let mut h = header.to_owned();
                if h.starts_with('<') {
                    let mut depth = 0;
                    for (k, c) in h.char_indices() {
                        match c {
                            '<' => depth += 1,
                            '>' => {
                                depth -= 1;
                                if depth == 0 {
                                    h = h[k + 1..].to_owned();
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let h = h.split(" where ").next().unwrap_or(&h).trim().to_owned();
                let (tr, ty) = match h.split_once(" for ") {
                    Some((tr, ty)) => (Some(simple_name(tr.trim_start_matches(['!', '&']))), simple_name(ty)),
                    None => (None, simple_name(&h)),
                };
                if let Some(b) = brace {
                    impls.push((ty, tr, b));
                }
            }
            kw => {
                let (name, label, rest, params) = split_header(header);
                if name.is_empty() {
                    continue;
                }
                let kind = match kw {
                    "struct" => "struct",
                    "enum" => "enum",
                    "union" => "union",
                    _ => "trait",
                };
                let mut t = Type::new(&name, label, kind, line);
                if kind == "trait"
                    && let Some(colon) = top_colon(&rest)
                {
                    for p in split_top(rest[colon + 1..].split(" where ").next().unwrap_or(""), '+') {
                        let n = simple_name(&p);
                        if !n.is_empty() && n.chars().next().is_some_and(char::is_alphabetic) && !n.starts_with('\'') {
                            t.parents.push((n, Rel::Extends));
                        }
                    }
                }
                match (kind, brace) {
                    ("struct" | "union", Some(b)) => {
                        for item in top_items(braced(s, b)) {
                            rust_field(&item, &mut t);
                        }
                    }
                    ("struct", None) if !params.is_empty() => {
                        // A tuple struct: its fields are numbered.
                        for (n, p) in split_top(&params, ',').iter().enumerate() {
                            let vis = if p.trim_start().starts_with("pub") { '+' } else { '-' };
                            let ty = p.trim_start_matches("pub(crate)").trim_start_matches("pub").trim();
                            t.fields.push(Field {
                                text: format!("{vis} {n}: {}", shorten(ty, 40)),
                                types: type_names(ty),
                                embedded: false,
                            });
                        }
                    }
                    ("enum", Some(b)) => {
                        for item in top_items(braced(s, b)) {
                            let v: String = item.trim_start_matches('#').chars().take_while(|c| is_ident(*c)).collect();
                            if !v.is_empty() && !item.starts_with('#') {
                                t.fields.push(Field { text: v, types: type_names(&item), embedded: false });
                            }
                        }
                    }
                    ("trait", Some(b)) => {
                        rust_methods(braced(s, b), &mut t, true);
                    }
                    _ => {}
                }
                // A type declared before its impl keeps the impl's methods (and the other
                // way round).
                match types.iter().position(|x| x.name == t.name) {
                    Some(k) => {
                        let old = std::mem::replace(&mut types[k], t);
                        types[k].methods.extend(old.methods);
                        types[k].parents.extend(old.parents);
                    }
                    None => types.push(t),
                }
                if let Some(b) = brace {
                    i = b + 1;
                }
            }
        }
    }
    for (ty, tr, b) in impls {
        let body = braced(s, b).to_owned();
        let t = rust_find(&mut types, &ty);
        if let Some(tr) = tr {
            if !t.parents.iter().any(|(p, _)| *p == tr) {
                t.parents.push((tr, Rel::Implements));
            }
        } else {
            rust_methods(&body, t, false);
        }
    }
    types.retain(|t| t.line > 0);
    types
}

fn rust_field(item: &str, t: &mut Type) {
    let item = item.trim();
    if item.starts_with('#') {
        return;
    }
    let vis = if item.starts_with("pub") { '+' } else { '-' };
    let rest = item.trim_start_matches("pub(crate)").trim_start_matches("pub(super)").trim_start_matches("pub").trim();
    let Some((name, ty)) = rest.split_once(':') else { return };
    let name = name.trim();
    if name.is_empty() || !name.chars().all(is_ident) {
        return;
    }
    t.fields.push(Field {
        text: format!("{vis} {name}: {}", shorten(ty.trim(), 40)),
        types: type_names(ty),
        embedded: false,
    });
}

fn rust_methods(body: &str, t: &mut Type, in_trait: bool) {
    let mut depth = 0i32;
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    for (k, &(at, c)) in chars.iter().enumerate() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            'f' if depth == 0 && body[at..].starts_with("fn ") && (k == 0 || !is_ident(chars[k - 1].1)) => {
                let head = &body[..at];
                let line_start = head.rfind(['\n', ';', '}']).map_or(0, |i| i + 1);
                let vis = if in_trait || head[line_start..].contains("pub") { '+' } else { '-' };
                let rest = &body[at + 3..];
                let name: String = rest.chars().take_while(|c| is_ident(*c)).collect();
                let params = rest.find('(').and_then(|o| {
                    let mut d = 0;
                    for (i, ch) in rest[o..].char_indices() {
                        match ch {
                            '(' => d += 1,
                            ')' => {
                                d -= 1;
                                if d == 0 {
                                    return Some(&rest[o + 1..o + i]);
                                }
                            }
                            _ => {}
                        }
                    }
                    None
                });
                let params: Vec<String> = split_top(params.unwrap_or(""), ',')
                    .iter()
                    .filter(|p| !p.contains("self"))
                    .filter_map(|p| p.split(':').next().map(|n| n.trim().trim_start_matches("mut ").to_owned()))
                    .collect();
                let text = format!("{vis} {name}({})", shorten(&params.join(", "), 40));
                if !name.is_empty() && !t.methods.contains(&text) {
                    t.methods.push(text);
                }
            }
            _ => {}
        }
    }
}

// ----- Go -------------------------------------------------------------------------------

fn go(s: &str) -> Vec<Type> {
    let mut types: Vec<Type> = Vec::new();
    let mut methods: Vec<(String, String)> = Vec::new();
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    // Inside `type ( … )` groups.
    let mut grouped = false;
    while i < lines.len() {
        let t = lines[i].trim();
        if t == "type (" {
            grouped = true;
            i += 1;
            continue;
        }
        if grouped && t == ")" {
            grouped = false;
            i += 1;
            continue;
        }
        let decl = t.strip_prefix("type ").or(if grouped { Some(t) } else { None });
        if let Some(d) = decl {
            let ws: Vec<&str> = d.split_whitespace().collect();
            if ws.len() >= 2 && (ws[1].starts_with("struct") || ws[1].starts_with("interface")) {
                let name = ws[0].split('[').next().unwrap_or(ws[0]);
                let kind = if ws[1].starts_with("struct") { "struct" } else { "interface" };
                let mut ty = Type::new(name, ws[0].to_owned(), kind, i as u32 + 1);
                if d.contains('{') && !d.trim_end().ends_with('}') {
                    let mut k = i + 1;
                    while k < lines.len() && lines[k].trim() != "}" {
                        let l = lines[k].trim();
                        let lw: Vec<&str> = l.split_whitespace().collect();
                        if !l.is_empty() {
                            if kind == "interface" {
                                if let Some(p) = l.find('(') {
                                    let mname = l[..p].trim();
                                    let vis = if mname.starts_with(char::is_uppercase) { '+' } else { '-' };
                                    let params = l[p + 1..].split(')').next().unwrap_or("");
                                    ty.methods.push(format!(
                                        "{vis} {mname}({})",
                                        shorten(&param_names(Lang::Go, params), 40)
                                    ));
                                } else if lw.len() == 1 {
                                    ty.parents.push((simple_name(lw[0]), Rel::Extends));
                                }
                            } else if lw.len() == 1 || lw.len() == 2 && lw[1].starts_with('`') {
                                // Embedded.
                                let e = simple_name(lw[0].trim_start_matches('*'));
                                ty.fields.push(Field {
                                    text: format!("+ {e}"),
                                    types: vec![e.clone()],
                                    embedded: true,
                                });
                            } else if lw.len() >= 2 {
                                // `a, b Type`: the names end at the first word without a comma.
                                let named = lw.iter().position(|w| !w.ends_with(',')).map_or(1, |p| p + 1);
                                let ftype = lw[named..]
                                    .iter()
                                    .take_while(|w| !w.starts_with('`'))
                                    .copied()
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                for n in lw[..named].iter().flat_map(|w| w.split(',')) {
                                    let n = n.trim();
                                    if n.is_empty() {
                                        continue;
                                    }
                                    let vis = if n.starts_with(char::is_uppercase) { '+' } else { '-' };
                                    ty.fields.push(Field {
                                        text: format!("{vis} {n}: {}", shorten(&ftype, 40)),
                                        types: type_names(&ftype),
                                        embedded: false,
                                    });
                                }
                            }
                        }
                        k += 1;
                    }
                    i = k;
                }
                types.push(ty);
            }
        }
        // func (r *Type) Name(params) …
        if let Some(rest) = t.strip_prefix("func (")
            && let Some(close) = rest.find(')')
        {
            let recv = &rest[..close];
            let ty = simple_name(recv.split_whitespace().last().unwrap_or("").trim_start_matches('*'));
            let after = rest[close + 1..].trim_start();
            let name: String = after.chars().take_while(|c| is_ident(*c)).collect();
            let params = after.find('(').map(|o| after[o + 1..].split(')').next().unwrap_or("")).unwrap_or("");
            if !name.is_empty() {
                let vis = if name.starts_with(char::is_uppercase) { '+' } else { '-' };
                methods.push((ty, format!("{vis} {name}({})", shorten(&param_names(Lang::Go, params), 40))));
            }
        }
        i += 1;
    }
    for (ty, m) in methods {
        if let Some(t) = types.iter_mut().find(|t| t.name == ty)
            && !t.methods.contains(&m)
        {
            t.methods.push(m);
        }
    }
    types
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one<'a>(types: &'a [Type], name: &str) -> &'a Type {
        types.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no {name} in {types:#?}"))
    }

    fn parents(t: &Type) -> Vec<(&str, Rel)> {
        t.parents.iter().map(|(n, r)| (n.as_str(), *r)).collect()
    }

    fn fields(t: &Type) -> Vec<&str> {
        t.fields.iter().map(|f| f.text.as_str()).collect()
    }

    #[test]
    fn java() {
        let src = r#"
package com.shop.model;

import java.util.List;

/** A pet. class Fake extends Nothing */
public abstract class Animal<T> implements Comparable<Animal<T>>, Named {
    protected String name; // the name
    private final List<Toy> toys = new ArrayList<>();
    int age;

    public Animal(String name, int age) { this.name = name; }

    @Override
    public int compareTo(Animal<T> other) {
        if (other == null) { return 0; }
        return 1;
    }
    abstract void speak();
    static class Inner {}
}

public interface Named extends HasId { String name(); }

public enum Color { RED, GREEN("g"), BLUE; private final String code; }

record Point(int x, int y) {}
"#;
        let t = parse(Lang::Java, src);
        let animal = one(&t, "Animal");
        assert_eq!(animal.kind, "abstract");
        assert_eq!(animal.label, "Animal<T>");
        assert_eq!(animal.group, "com.shop.model");
        assert_eq!(parents(animal), [("Comparable", Rel::Implements), ("Named", Rel::Implements)]);
        assert_eq!(fields(animal), ["# name: String", "- toys: List<Toy>", "~ age: int"]);
        assert!(animal.fields[1].types.contains(&"Toy".to_owned()));
        assert_eq!(animal.methods, ["+ Animal(name, age)", "+ compareTo(other)", "~ speak()"]);
        assert_eq!(animal.line, 7);
        assert_eq!(one(&t, "Inner").kind, "class");
        let named = one(&t, "Named");
        assert_eq!((named.kind, parents(named)), ("interface", vec![("HasId", Rel::Extends)]));
        assert_eq!(named.methods, ["+ name()"]);
        let color = one(&t, "Color");
        assert_eq!(color.kind, "enum");
        assert_eq!(fields(color), ["- code: String"]);
        assert_eq!(fields(one(&t, "Point")), ["~ x: int", "~ y: int"]);
        assert!(!t.iter().any(|x| x.name == "Fake"), "comments are skipped");
    }

    #[test]
    fn csharp_kotlin_cpp() {
        let cs = r#"
namespace Shop.Orders {
    public class Order : Entity, IOrder, IDisposable {
        public int Id { get; set; }
        private readonly List<Line> lines = new();
        public void Add(Line line, int qty) { }
    }
    public interface IOrder : IEntity { void Add(Line line, int qty); }
    public struct Money : IEquatable<Money> { public decimal Amount; }
}
"#;
        let t = parse(Lang::CSharp, cs);
        let order = one(&t, "Order");
        assert_eq!(order.group, "Shop.Orders");
        assert_eq!(
            parents(order),
            [("Entity", Rel::Extends), ("IOrder", Rel::Implements), ("IDisposable", Rel::Implements)]
        );
        assert_eq!(fields(order), ["+ Id: int", "- lines: List<Line>"]);
        assert_eq!(order.methods, ["+ Add(line, qty)"]);
        assert_eq!(parents(one(&t, "IOrder")), [("IEntity", Rel::Extends)]);
        assert_eq!(parents(one(&t, "Money")), [("IEquatable", Rel::Implements)]);
        assert_eq!(fields(one(&t, "Money")), ["+ Amount: decimal"]);

        let kt = r#"
package shop
data class User(val id: Long, var name: String, age: Int) : Person(name), Serializable {
    val tags: List<Tag> = emptyList()
    fun greet(other: User): String = "hi"
    private fun secret() { }
}
interface Repo<T> : Base<T> { fun find(id: Long): T? }
sealed class Shape
object Registry : Store()
"#;
        let t = parse(Lang::Kotlin, kt);
        let user = one(&t, "User");
        assert_eq!(user.kind, "data");
        assert_eq!(parents(user), [("Person", Rel::Extends), ("Serializable", Rel::Implements)]);
        assert_eq!(fields(user), ["+ id: Long", "+ name: String", "+ tags: List<Tag>"]);
        assert_eq!(user.methods, ["+ greet(other)", "- secret()"]);
        assert_eq!(one(&t, "Repo").methods, ["+ find(id)"]);
        assert_eq!(parents(one(&t, "Repo")), [("Base", Rel::Extends)]);
        assert_eq!(one(&t, "Shape").kind, "sealed");
        assert_eq!(parents(one(&t, "Registry")), [("Store", Rel::Extends)]);

        let cpp = r#"
class Shape { public: virtual double area() const = 0; virtual ~Shape() {} };
class Circle : public Shape, private Logger {
    double r;
public:
    explicit Circle(double r) : r(r) {}
    double area() const override { return 3.14 * r * r; }
    std::vector<Point> points;
};
struct Point { int x, y; };
"#;
        let t = parse(Lang::Cpp, cpp);
        let circle = one(&t, "Circle");
        assert_eq!(parents(circle), [("Shape", Rel::Extends), ("Logger", Rel::Extends)]);
        assert_eq!(fields(circle), ["- r: double", "+ points: std::vector<Point>"]);
        assert_eq!(circle.methods, ["+ Circle(r)", "+ area()"]);
        assert_eq!(one(&t, "Shape").methods, ["+ area()"]);
        assert_eq!(one(&t, "Point").kind, "struct");
    }

    #[test]
    fn typescript_python() {
        let ts = r#"
export abstract class Store<T> extends Base implements Saver, Loader {
  private items: Map<string, Item> = new Map();
  #secret = 1;
  readonly name: string;
  constructor(name: string, private db: Db) { super(); }
  async save(item: T): Promise<void> { }
  load = (id: string) => this.items.get(id);
}
export interface Saver extends Thing { save(item: Item): void; }
const x = class {};
"#;
        let t = parse(Lang::TypeScript, ts);
        let store = one(&t, "Store");
        assert_eq!(store.kind, "abstract");
        assert_eq!(parents(store), [("Base", Rel::Extends), ("Saver", Rel::Implements), ("Loader", Rel::Implements)]);
        assert_eq!(fields(store), ["- items: Map<string, Item>", "- #secret", "+ name: string"]);
        assert_eq!(store.methods, ["+ Store(name, db)", "+ save(item)", "+ load(id)"]);
        assert_eq!(parents(one(&t, "Saver")), [("Thing", Rel::Extends)]);

        let py = r#"
from abc import ABC
class Animal(ABC):
    """A class Fake(Base): in a docstring."""
    kingdom: str = "animalia"
    def __init__(self, name: str, owner: "Owner"):
        self.name = name
        self._owner: Owner = owner
    def speak(self, loud=False):
        pass
    def _private(self): ...
class Dog(Animal, Walker, metaclass=Meta):
    def speak(self):
        return "woof"
class Color(Enum):
    RED = 1
"#;
        let t = parse(Lang::Python, py);
        let animal = one(&t, "Animal");
        assert_eq!(animal.kind, "abstract");
        assert_eq!(fields(animal), ["+ kingdom: str", "+ name", "- _owner: Owner"]);
        assert_eq!(animal.methods, ["+ Animal(name, owner)", "+ speak(loud)", "# _private()"]);
        assert_eq!(parents(one(&t, "Dog")), [("Animal", Rel::Extends), ("Walker", Rel::Extends)]);
        assert_eq!(one(&t, "Color").kind, "enum");
        assert!(!t.iter().any(|x| x.name == "Fake"));
    }

    #[test]
    fn rust_and_go() {
        let rs = r#"
pub struct Parser<'a> {
    pub input: &'a str,
    tokens: Vec<Token>,
}
pub struct Id(pub u32, String);
pub enum Token { Word(String), Num(i64), End }
pub trait Visitor: Debug + Clone { fn visit(&mut self, t: &Token) -> bool; fn done(&self) {} }
impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self { Self { input, tokens: vec![] } }
    fn next(&mut self) -> Option<char> { let c = 'x'; None }
}
impl Visitor for Parser<'_> { fn visit(&mut self, t: &Token) -> bool { true } }
impl std::fmt::Display for Id { }
"#;
        let t = parse(Lang::Rust, rs);
        let p = one(&t, "Parser");
        assert_eq!(p.label, "Parser<'a>");
        assert_eq!(fields(p), ["+ input: &'a str", "- tokens: Vec<Token>"]);
        assert_eq!(p.methods, ["+ new(input)", "- next()"]);
        assert_eq!(parents(p), [("Visitor", Rel::Implements)]);
        assert_eq!(fields(one(&t, "Id")), ["+ 0: u32", "- 1: String"]);
        assert_eq!(parents(one(&t, "Id")), [("Display", Rel::Implements)]);
        assert_eq!(fields(one(&t, "Token")), ["Word", "Num", "End"]);
        let v = one(&t, "Visitor");
        assert_eq!(parents(v), [("Debug", Rel::Extends), ("Clone", Rel::Extends)]);
        assert_eq!(v.methods, ["+ visit(t)", "+ done()"]);

        let go_src = r#"
package shop

type (
    Reader interface {
        io.Closer
        Read(p []byte) (int, error)
    }
)

type Store struct {
    Base
    *sync.Mutex
    Items    []Item `json:"items"`
    name, kind string
}

func (s *Store) Add(item Item, qty int) error { return nil }
func (s Store) count() int { return 0 }
"#;
        let t = parse(Lang::Go, go_src);
        let store = one(&t, "Store");
        assert_eq!(store.group, "shop");
        assert_eq!(fields(store), ["+ Base", "+ Mutex", "+ Items: []Item", "- name: string", "- kind: string"]);
        assert!(store.fields[0].embedded);
        assert_eq!(store.methods, ["+ Add(item, qty)", "- count()"]);
        let r = one(&t, "Reader");
        assert_eq!(r.kind, "interface");
        assert_eq!(parents(r), [("Closer", Rel::Extends)]);
        assert_eq!(r.methods, ["+ Read(p)"]);
    }
}
