//! Languages added by plugins: a name and comment style for file extensions the
//! editor doesn't know itself. Languages the editor knows always win.

use std::sync::RwLock;

use crate::extensions::plugin::manifest::LanguageDecl;

struct Language {
    decl: LanguageDecl,
    // Callers keep these for as long as they like, as they do built-in names.
    name: &'static str,
    line_comment: Option<&'static str>,
}

static LANGUAGES: RwLock<Vec<Language>> = RwLock::new(Vec::new());

fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// Replaces the list, if it changed. Runs often, and rarely changes anything.
pub fn set(decls: &[LanguageDecl]) {
    let unchanged =
        LANGUAGES.read().is_ok_and(|l| l.len() == decls.len() && l.iter().zip(decls).all(|(a, b)| a.decl == *b));
    if unchanged {
        return;
    }
    let languages = decls
        .iter()
        .map(|decl| Language {
            decl: decl.clone(),
            name: leak(&decl.name),
            line_comment: decl.line_comment.as_deref().map(leak),
        })
        .collect();
    if let Ok(mut list) = LANGUAGES.write() {
        *list = languages;
    }
}

fn find<T>(ext: &str, pick: impl Fn(&Language) -> T) -> Option<T> {
    let list = LANGUAGES.read().ok()?;
    list.iter().find(|l| l.decl.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))).map(pick)
}

pub fn name(ext: &str) -> Option<&'static str> {
    find(ext, |l| l.name)
}

pub fn line_comment(ext: &str) -> Option<&'static str> {
    find(ext, |l| l.line_comment).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_add_languages_but_never_replace_known_ones() {
        let decl = |name: &str, ext: &str| LanguageDecl {
            name: name.to_owned(),
            extensions: vec![ext.to_owned()],
            line_comment: Some(";;".to_owned()),
        };
        set(&[decl("Demo", "demolang"), decl("Not Rust", "rs")]);
        assert_eq!(crate::editor::document::language_name("demolang"), "Demo");
        assert_eq!(crate::editor::document::language_name("DEMOLANG".to_lowercase().as_str()), "Demo");
        assert_eq!(crate::editor::edit_ops::comment_token("demolang"), Some(";;"));
        assert_eq!(crate::editor::document::language_name("rs"), "Rust");
        assert_eq!(crate::editor::edit_ops::comment_token("rs"), Some("//"));
        set(&[]);
        assert_eq!(crate::editor::document::language_name("demolang"), "Plain Text");
        assert_eq!(crate::editor::edit_ops::comment_token("demolang"), None);
    }
}
