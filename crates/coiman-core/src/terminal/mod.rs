//! The terminal: pseudo-terminals, search in their text, and what extensions hear of it.

pub mod history;
pub mod profiles;
pub mod pty;
pub mod termfind;

/// Text without its color codes: terminal escapes, and the `^[[31;1m` PowerShell
/// writes in their place.
pub fn strip_colors(text: &str) -> String {
    static CODES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = CODES.get_or_init(|| regex::Regex::new(r"(?:\x1b|\^\[)\[[0-9;?]*[A-Za-z]").expect("valid pattern"));
    re.replace_all(text, "").into_owned()
}
