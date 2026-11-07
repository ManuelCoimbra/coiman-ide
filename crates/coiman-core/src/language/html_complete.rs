//! Suggestions in HTML and Vue templates with no language server: element names after
//! `<`, an element's attributes inside its tag, and the values of attributes that take
//! a fixed set of them. Vue adds its directives and built-in components.

use crate::language::html_attributes::{ELEMENTS, GLOBAL};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Html,
    /// A Vue single-file component's template.
    Vue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Element,
    Attribute,
    Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    /// What goes in, as a snippet: `$1` is where the cursor ends up.
    pub insert: String,
    pub detail: &'static str,
    pub kind: Kind,
}

/// Suggestions for the cursor at the end of `before`, and how many chars back the word
/// they replace starts.
pub struct Suggestions {
    pub back: usize,
    pub items: Vec<Suggestion>,
}

/// Attributes that need no value: present means on.
const BOOLEAN: &[&str] = &[
    "allowfullscreen",
    "async",
    "autofocus",
    "autoplay",
    "checked",
    "controls",
    "default",
    "defer",
    "disabled",
    "formnovalidate",
    "hidden",
    "inert",
    "ismap",
    "itemscope",
    "loop",
    "multiple",
    "muted",
    "nomodule",
    "novalidate",
    "open",
    "playsinline",
    "readonly",
    "required",
    "reversed",
    "selected",
];

/// Event handlers worth offering on any element.
const EVENTS: &[&str] = &[
    "onclick",
    "ondblclick",
    "oninput",
    "onchange",
    "onsubmit",
    "onkeydown",
    "onkeyup",
    "onfocus",
    "onblur",
    "onmouseenter",
    "onmouseleave",
    "onload",
    "onerror",
    "onscroll",
];

/// Values for attributes that take one of a known set: (element or `*`, attribute, values).
const VALUES: &[(&str, &str, &[&str])] = &[
    (
        "input",
        "type",
        &[
            "text",
            "password",
            "email",
            "number",
            "checkbox",
            "radio",
            "file",
            "date",
            "time",
            "datetime-local",
            "month",
            "week",
            "color",
            "range",
            "search",
            "tel",
            "url",
            "hidden",
            "submit",
            "reset",
            "button",
            "image",
        ],
    ),
    ("button", "type", &["button", "submit", "reset"]),
    ("script", "type", &["module", "importmap", "text/javascript", "application/json"]),
    ("ol", "type", &["1", "a", "A", "i", "I"]),
    ("*", "target", &["_self", "_blank", "_parent", "_top"]),
    (
        "*",
        "rel",
        &[
            "stylesheet",
            "icon",
            "preload",
            "prefetch",
            "preconnect",
            "dns-prefetch",
            "modulepreload",
            "canonical",
            "alternate",
            "manifest",
            "noopener",
            "noreferrer",
            "nofollow",
            "author",
            "license",
        ],
    ),
    ("link", "as", &["script", "style", "font", "image", "fetch", "document", "audio", "video", "track", "worker"]),
    ("form", "method", &["get", "post", "dialog"]),
    ("form", "enctype", &["application/x-www-form-urlencoded", "multipart/form-data", "text/plain"]),
    ("*", "loading", &["lazy", "eager"]),
    ("*", "decoding", &["async", "sync", "auto"]),
    ("*", "fetchpriority", &["high", "low", "auto"]),
    ("*", "crossorigin", &["anonymous", "use-credentials"]),
    (
        "*",
        "referrerpolicy",
        &[
            "no-referrer",
            "no-referrer-when-downgrade",
            "origin",
            "origin-when-cross-origin",
            "same-origin",
            "strict-origin",
            "strict-origin-when-cross-origin",
            "unsafe-url",
        ],
    ),
    ("*", "dir", &["ltr", "rtl", "auto"]),
    ("*", "contenteditable", &["true", "false", "plaintext-only"]),
    ("*", "draggable", &["true", "false"]),
    ("*", "spellcheck", &["true", "false"]),
    ("*", "translate", &["yes", "no"]),
    ("*", "hidden", &["until-found"]),
    ("*", "popover", &["auto", "manual"]),
    ("*", "popovertargetaction", &["toggle", "show", "hide"]),
    ("*", "inputmode", &["none", "text", "decimal", "numeric", "tel", "search", "email", "url"]),
    ("*", "enterkeyhint", &["enter", "done", "go", "next", "previous", "search", "send"]),
    ("*", "autocapitalize", &["off", "none", "on", "sentences", "words", "characters"]),
    (
        "*",
        "autocomplete",
        &[
            "on",
            "off",
            "name",
            "email",
            "username",
            "new-password",
            "current-password",
            "one-time-code",
            "tel",
            "street-address",
            "postal-code",
            "country",
            "bday",
            "cc-number",
        ],
    ),
    ("*", "preload", &["none", "metadata", "auto"]),
    ("th", "scope", &["row", "col", "rowgroup", "colgroup"]),
    ("area", "shape", &["rect", "circle", "poly", "default"]),
    ("track", "kind", &["subtitles", "captions", "chapters", "metadata", "descriptions"]),
    ("textarea", "wrap", &["soft", "hard"]),
    (
        "iframe",
        "sandbox",
        &[
            "allow-forms",
            "allow-modals",
            "allow-popups",
            "allow-same-origin",
            "allow-scripts",
            "allow-downloads",
            "allow-top-navigation",
        ],
    ),
    ("meta", "name", &["viewport", "description", "author", "keywords", "theme-color", "robots", "color-scheme"]),
    ("meta", "charset", &["utf-8"]),
    ("meta", "http-equiv", &["content-security-policy", "refresh", "x-ua-compatible", "content-type"]),
    ("template", "lang", &["html", "pug"]),
    ("script", "lang", &["ts", "tsx", "js", "jsx"]),
    ("style", "lang", &["css", "scss", "sass", "less", "stylus"]),
];

/// Vue's directives, with whether they take a value.
const VUE_DIRECTIVES: &[(&str, bool)] = &[
    ("v-if", true),
    ("v-else-if", true),
    ("v-else", false),
    ("v-for", true),
    ("v-show", true),
    ("v-model", true),
    ("v-bind", true),
    ("v-on", true),
    ("v-slot", false),
    ("v-html", true),
    ("v-text", true),
    ("v-once", false),
    ("v-memo", true),
    ("v-pre", false),
    ("v-cloak", false),
    ("ref", true),
    ("key", true),
    ("is", true),
];

/// Vue's own components.
const VUE_COMPONENTS: &[&str] =
    &["template", "slot", "component", "transition", "transition-group", "keep-alive", "teleport", "suspense"];

/// Attributes of Vue's single-file component blocks.
const VUE_BLOCK_ATTRIBUTES: &[(&str, &[&str])] = &[
    ("script", &["setup", "lang", "generic", "src"]),
    ("style", &["scoped", "module", "lang", "src"]),
    ("template", &["lang", "functional", "src"]),
];

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | ':' | '_' | '.' | '@' | '#')
}

/// The attributes an element takes, its own first.
pub fn attributes_of(element: &str) -> Vec<&'static str> {
    let own = ELEMENTS.iter().find(|(name, _)| *name == element).map_or(&[][..], |(_, own)| own);
    own.iter().chain(GLOBAL).copied().collect()
}

/// Whether the end of `before` is inside `<script>` or `<style>`: what's there isn't markup.
fn in_raw_text(before: &str) -> bool {
    let lower = before.to_ascii_lowercase();
    ["script", "style"].iter().any(|tag| {
        let open = lower.rfind(&format!("<{tag}"));
        let close = lower.rfind(&format!("</{tag}"));
        open.is_some_and(|o| close.is_none_or(|c| c < o) && lower[o..].contains('>'))
    })
}

fn suggestion(label: &str, insert: String, detail: &'static str, kind: Kind) -> Suggestion {
    Suggestion { label: label.to_owned(), insert, detail, kind }
}

/// What to suggest at the end of `before`, if anything.
pub fn suggest(flavor: Flavor, before: &str) -> Option<Suggestions> {
    if in_raw_text(before) {
        return None;
    }
    let lt = before.rfind('<')?;
    let tag = &before[lt + 1..];
    if tag.contains('>') || tag.starts_with(['/', '!', '?']) {
        return None;
    }
    let element: String = tag.chars().take_while(|c| name_char(*c)).collect();
    // Still writing the element's name.
    if element.len() == tag.len() {
        let mut items: Vec<Suggestion> =
            ELEMENTS.iter().map(|(name, _)| suggestion(name, (*name).to_owned(), "element", Kind::Element)).collect();
        if flavor == Flavor::Vue {
            items.extend(VUE_COMPONENTS.iter().map(|name| suggestion(name, (*name).to_owned(), "Vue", Kind::Element)));
        }
        return Some(Suggestions { back: element.chars().count(), items });
    }
    let lower = element.to_ascii_lowercase();
    let rest = &tag[element.len()..];
    // In a quoted value: `name="partial` with the quote still open.
    let mut quote = None;
    let mut value_start = 0;
    for (i, c) in rest.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => {
                quote = Some(c);
                value_start = i + 1;
            }
            (Some(q), _) if c == q => quote = None,
            _ => {}
        }
    }
    if quote.is_some() {
        let head = rest[..value_start - 1].trim_end().strip_suffix('=')?.trim_end();
        let attribute: String =
            head.chars().rev().take_while(|c| name_char(*c)).collect::<String>().chars().rev().collect();
        let attribute = attribute.to_ascii_lowercase();
        let value = &rest[value_start..];
        // Lists like `rel` take several, one word at a time.
        let word = value.rsplit(' ').next().unwrap_or("");
        let values = VALUES
            .iter()
            .filter(|(el, attr, _)| *attr == attribute && (*el == "*" || *el == lower))
            .flat_map(|(_, _, values)| values.iter());
        let items: Vec<Suggestion> = values.map(|v| suggestion(v, (*v).to_owned(), "value", Kind::Value)).collect();
        return (!items.is_empty()).then(|| Suggestions { back: word.chars().count(), items });
    }
    // Writing an attribute's name: after a space, in a word of name characters.
    let word: String = rest.chars().rev().take_while(|c| name_char(*c)).collect::<String>().chars().rev().collect();
    let before_word = &rest[..rest.len() - word.len()];
    if !before_word.ends_with(char::is_whitespace) {
        return None;
    }
    let present: Vec<String> =
        before_word.split_whitespace().filter_map(|a| a.split('=').next()).map(|a| a.to_ascii_lowercase()).collect();
    let named = |name: &str, takes_value: bool, detail: &'static str| {
        let insert = if takes_value { format!("{name}=\"$1\"") } else { name.to_owned() };
        suggestion(name, insert, detail, Kind::Attribute)
    };
    let mut items: Vec<Suggestion> = Vec::new();
    // `:` and `@` in Vue bind a prop or listen to an event.
    if flavor == Flavor::Vue && (word.starts_with(':') || word.starts_with('@')) {
        let (prefix, names): (&str, Vec<&str>) = if word.starts_with(':') {
            (":", attributes_of(&lower))
        } else {
            ("@", EVENTS.iter().map(|e| &e[2..]).collect())
        };
        for name in names {
            let label = format!("{prefix}{name}");
            items.push(suggestion(&label, format!("{label}=\"$1\""), "Vue", Kind::Attribute));
        }
        return Some(Suggestions { back: word.chars().count(), items });
    }
    if flavor == Flavor::Vue
        && let Some((_, attrs)) = VUE_BLOCK_ATTRIBUTES.iter().find(|(block, _)| *block == lower)
    {
        items
            .extend(attrs.iter().map(|a| named(a, !matches!(*a, "setup" | "scoped" | "module" | "functional"), "Vue")));
    }
    items.extend(attributes_of(&lower).into_iter().map(|a| named(a, !BOOLEAN.contains(&a), "attribute")));
    items.extend(EVENTS.iter().map(|e| named(e, true, "event")));
    if flavor == Flavor::Vue {
        items.extend(VUE_DIRECTIVES.iter().map(|(d, value)| named(d, *value, "Vue")));
    }
    items.retain(|s| !present.contains(&s.label.to_ascii_lowercase()));
    let mut seen = std::collections::HashSet::new();
    items.retain(|s| seen.insert(s.label.clone()));
    Some(Suggestions { back: word.chars().count(), items })
}

#[cfg(test)]
mod tests {
    use super::Flavor::*;
    use super::*;

    fn labels(flavor: Flavor, before: &str) -> Option<(usize, Vec<String>)> {
        suggest(flavor, before).map(|s| (s.back, s.items.into_iter().map(|i| i.label).collect()))
    }

    fn insert_of(flavor: Flavor, before: &str, label: &str) -> String {
        suggest(flavor, before).unwrap().items.into_iter().find(|i| i.label == label).unwrap().insert
    }

    #[test]
    fn elements_after_a_less_than() {
        let (back, names) = labels(Html, "<body>\n  <di").unwrap();
        assert_eq!(back, 2);
        assert!(names.contains(&"div".to_owned()) && names.contains(&"dialog".to_owned()));
        assert!(!names.contains(&"transition".to_owned()));
        assert!(labels(Vue, "<tran").unwrap().1.contains(&"transition".to_owned()));
        assert_eq!(labels(Html, "</di"), None);
        assert_eq!(labels(Html, "text"), None);
    }

    #[test]
    fn an_elements_own_attributes_first_and_with_quotes() {
        let (back, names) = labels(Html, "<img sr").unwrap();
        assert_eq!(back, 2);
        assert_eq!(names[0], "alt", "the element's own come first");
        assert!(names.contains(&"src".to_owned()) && names.contains(&"class".to_owned()));
        assert!(!names.contains(&"href".to_owned()), "an <a> attribute");
        assert_eq!(insert_of(Html, "<img ", "src"), "src=\"$1\"");
        assert_eq!(insert_of(Html, "<input ", "disabled"), "disabled", "boolean");
        // Those already there aren't offered again.
        assert!(!labels(Html, "<img src=\"a\" ").unwrap().1.contains(&"src".to_owned()));
        assert_eq!(labels(Html, "<img"), Some((3, labels(Html, "<img").unwrap().1)), "still the element's name");
    }

    #[test]
    fn values_of_attributes_that_take_a_set() {
        let (back, values) = labels(Html, "<input type=\"chec").unwrap();
        assert_eq!(back, 4);
        assert!(values.contains(&"checkbox".to_owned()) && values.contains(&"email".to_owned()));
        assert!(labels(Html, "<button type=\"").unwrap().1.contains(&"submit".to_owned()));
        assert!(!labels(Html, "<button type=\"").unwrap().1.contains(&"checkbox".to_owned()));
        // A list takes one word at a time.
        assert_eq!(labels(Html, "<a rel=\"noopener nore").unwrap().0, 4);
        assert_eq!(labels(Html, "<div title=\""), None, "free text");
        assert_eq!(labels(Html, "<input type=\"text\" "), labels(Html, "<input type=\"text\" "), "after the value");
    }

    #[test]
    fn vue_directives_bindings_and_blocks() {
        let names = labels(Vue, "<div v-").unwrap().1;
        assert!(names.contains(&"v-if".to_owned()) && names.contains(&"v-for".to_owned()));
        assert_eq!(insert_of(Vue, "<div ", "v-else"), "v-else");
        assert_eq!(insert_of(Vue, "<div ", "v-model"), "v-model=\"$1\"");
        let bound = labels(Vue, "<img :sr").unwrap();
        assert_eq!(bound.0, 3);
        assert!(bound.1.contains(&":src".to_owned()));
        assert!(labels(Vue, "<button @cl").unwrap().1.contains(&"@click".to_owned()));
        assert!(labels(Vue, "<script ").unwrap().1.contains(&"setup".to_owned()));
        assert!(labels(Vue, "<style ").unwrap().1.contains(&"scoped".to_owned()));
        assert!(!labels(Html, "<div ").unwrap().1.contains(&"v-if".to_owned()));
        // Inside a script block, nothing.
        assert_eq!(labels(Vue, "<script setup>\nconst a = 1 <b"), None);
        assert!(labels(Vue, "<script setup>x</script>\n<template>\n  <di").is_some());
    }
}
