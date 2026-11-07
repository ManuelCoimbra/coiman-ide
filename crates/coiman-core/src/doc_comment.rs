//! Doxygen comments as VS Code's Doxygen Documentation Generator writes them: typing `/**`
//! and Enter above a function gives `@brief`, one `@param` per parameter and `@return`
//! unless it returns nothing; above anything else, `@brief` alone.

/// What a function's documentation lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub params: Vec<String>,
    /// The return type, when there is one to describe.
    pub returns: Option<String>,
}

const MODIFIERS: &[&str] = &[
    "static",
    "inline",
    "extern",
    "virtual",
    "explicit",
    "constexpr",
    "public",
    "private",
    "protected",
    "final",
    "abstract",
    "synchronized",
    "native",
    "async",
    "export",
    "default",
    "override",
    "unsafe",
];

/// Splits at commas outside brackets.
fn split_top(text: &str) -> Vec<String> {
    let (mut parts, mut depth, mut current) = (Vec::new(), 0i32, String::new());
    for c in text.chars() {
        match c {
            '(' | '[' | '<' | '{' => depth += 1,
            ')' | ']' | '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The last identifier in `text`.
fn last_ident(text: &str) -> Option<String> {
    let end = text.rfind(is_ident)?;
    let start = text[..=end].rfind(|c: char| !is_ident(c)).map_or(0, |i| i + 1);
    Some(text[start..=end].to_owned())
}

/// A parameter's name: `int x`, `char *s`, `int a[10]`, `const std::string &name = ""`,
/// `x: number`, `y?: string`. `None` for `void` and `...`.
fn param_name(param: &str) -> Option<String> {
    let param = param.split('=').next()?.trim();
    if param.is_empty() || param == "void" || param == "..." {
        return None;
    }
    // TypeScript's `name: type` (C++'s `::` is a scope, not this).
    if let Some((name, _)) = param.split_once(':').filter(|_| !param.contains("::")) {
        return last_ident(name.trim_end_matches('?'));
    }
    let without_array = param.split('[').next().unwrap_or(param);
    last_ident(without_array)
}

/// The function declared in `text` (what follows the comment, up to its `{` or `;`), if
/// it is one.
pub fn parse_signature(text: &str) -> Option<Signature> {
    let head = text.split(['{', ';']).next()?.trim();
    if head.starts_with('#') || head.is_empty() {
        return None;
    }
    let open = head.find('(')?;
    let before = head[..open].trim_end();
    let name = last_ident(before)?;
    if matches!(name.as_str(), "if" | "while" | "for" | "switch" | "return" | "sizeof" | "catch") {
        return None;
    }
    // The matching `)`.
    let mut depth = 0;
    let close = head[open..].char_indices().find_map(|(i, c)| {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
        None
    })?;
    let params = split_top(&head[open + 1..close]).iter().filter_map(|p| param_name(p)).collect();
    // `int foo(...)`, `public static int foo(...)`, or TypeScript's `foo(...): number`.
    let after = head[close + 1..].trim();
    let mut returns = after.strip_prefix(':').map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    if returns.is_none() {
        let ty = before[..before.len() - name.len()].trim();
        let words: Vec<&str> = ty.split_whitespace().filter(|w| !MODIFIERS.contains(w)).collect();
        let ty = words.join(" ");
        let ty = ty.trim();
        if !ty.is_empty() && ty != "function" && ty != "def" && ty != "fn" {
            returns = Some(ty.to_owned());
        }
    }
    let returns = returns.filter(|r| r != "void" && r != "Unit");
    Some(Signature { params, returns })
}

/// What Enter after `/**` adds: the comment's lines, closed, and where the cursor goes
/// (chars into the text). `following` is the code below the comment.
pub fn doc_block(following: &str, indent: &str, nl: &str, doxygen: bool) -> (String, usize) {
    let line = |text: &str| format!("{nl}{indent} * {text}");
    let Some(sig) = doxygen.then(|| parse_signature(following)).flatten() else {
        let brief = if doxygen && !following.trim().is_empty() { "@brief " } else { "" };
        let first = format!("{nl}{indent} * {brief}");
        let cursor = first.chars().count();
        return (format!("{first}{nl}{indent} */"), cursor);
    };
    let first = format!("{nl}{indent} * @brief ");
    let cursor = first.chars().count();
    let mut text = first;
    if !sig.params.is_empty() || sig.returns.is_some() {
        text.push_str(&line(""));
    }
    for p in &sig.params {
        text.push_str(&line(&format!("@param {p} ")));
    }
    if let Some(r) = &sig.returns {
        text.push_str(&line(&format!("@return {r} ")));
    }
    text.push_str(&format!("{nl}{indent} */"));
    (text, cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_in_several_languages() {
        let sig = |s: &str| parse_signature(s).map(|s| (s.params, s.returns));
        let p = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            sig("PtVector3d  vector3dCreate(double x, double y, double z);"),
            Some((p(&["x", "y", "z"]), Some("PtVector3d".to_owned())))
        );
        assert_eq!(
            sig("void vector3dPrint(PtVector3d vec, bool lineBreakAfter);"),
            Some((p(&["vec", "lineBreakAfter"]), None))
        );
        assert_eq!(sig("int vector3dDestroy(PtVector3d *pVec);"), Some((p(&["pVec"]), Some("int".to_owned()))));
        assert_eq!(
            sig("static char *copy(const char *s, int n[10]) {"),
            Some((p(&["s", "n"]), Some("char *".to_owned())))
        );
        assert_eq!(sig("int main(void) {"), Some((p(&[]), Some("int".to_owned()))));
        assert_eq!(sig("int sum(int count, ...) {"), Some((p(&["count"]), Some("int".to_owned()))));
        assert_eq!(
            sig("public static List<String> names(Map<String, Integer> m, int limit) {"),
            Some((p(&["m", "limit"]), Some("List<String>".to_owned())))
        );
        assert_eq!(
            sig("std::string greet(const std::string &name = \"x\") const;"),
            Some((p(&["name"]), Some("std::string".to_owned())))
        );
        assert_eq!(sig("function add(a, b) {"), Some((p(&["a", "b"]), None)));
        assert_eq!(
            sig("export function area(w: number, h?: number): number {"),
            Some((p(&["w", "h"]), Some("number".to_owned())))
        );
        assert_eq!(sig("struct vector3d {"), None);
        assert_eq!(sig("#define MAX(a, b) ((a) > (b) ? (a) : (b))"), None);
        assert_eq!(sig("if (x) {"), None);
    }

    #[test]
    fn blocks_as_the_extension_writes_them() {
        let (text, cursor) = doc_block("int vector3dDestroy(PtVector3d *pVec);", "", "\n", true);
        assert_eq!(text, "\n * @brief \n * \n * @param pVec \n * @return int \n */");
        assert_eq!(&text[..cursor], "\n * @brief ");
        let (text, _) = doc_block("void f(void);", "    ", "\n", true);
        assert_eq!(text, "\n     * @brief \n     */");
        let (text, _) = doc_block("struct vector3d {", "", "\n", true);
        assert_eq!(text, "\n * @brief \n */");
        // Without Doxygen: a closed comment, nothing more.
        let (text, cursor) = doc_block("int f(int a);", "", "\n", false);
        assert_eq!((text.as_str(), cursor), ("\n * \n */", 4));
    }
}
