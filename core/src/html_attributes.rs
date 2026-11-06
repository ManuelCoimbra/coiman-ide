//! Which attributes each HTML element takes, from the WHATWG HTML standard's index of
//! attributes, and the old ones it lists as obsolete.

/// Attributes every element takes.
const GLOBAL: &[&str] = &[
    "accesskey",
    "autocapitalize",
    "autocorrect",
    "autofocus",
    "class",
    "contenteditable",
    "dir",
    "draggable",
    "enterkeyhint",
    "hidden",
    "id",
    "inert",
    "inputmode",
    "is",
    "itemid",
    "itemprop",
    "itemref",
    "itemscope",
    "itemtype",
    "lang",
    "nonce",
    "popover",
    "role",
    "slot",
    "spellcheck",
    "style",
    "tabindex",
    "title",
    "translate",
    "writingsuggestions",
    "xmlns",
];

/// Each element's own attributes. Elements here with none take only the global ones.
const ELEMENTS: &[(&str, &[&str])] = &[
    ("a", &["href", "target", "download", "ping", "rel", "hreflang", "type", "referrerpolicy"]),
    ("abbr", &[]),
    ("address", &[]),
    ("area", &["alt", "coords", "shape", "href", "target", "download", "ping", "rel", "referrerpolicy"]),
    ("article", &[]),
    ("aside", &[]),
    ("audio", &["src", "crossorigin", "preload", "autoplay", "loop", "muted", "controls"]),
    ("b", &[]),
    ("base", &["href", "target"]),
    ("bdi", &[]),
    ("bdo", &[]),
    ("blockquote", &["cite"]),
    ("body", &[]),
    ("br", &[]),
    (
        "button",
        &[
            "command",
            "commandfor",
            "disabled",
            "form",
            "formaction",
            "formenctype",
            "formmethod",
            "formnovalidate",
            "formtarget",
            "name",
            "popovertarget",
            "popovertargetaction",
            "type",
            "value",
        ],
    ),
    ("canvas", &["width", "height"]),
    ("caption", &[]),
    ("cite", &[]),
    ("code", &[]),
    ("col", &["span"]),
    ("colgroup", &["span"]),
    ("data", &["value"]),
    ("datalist", &[]),
    ("dd", &[]),
    ("del", &["cite", "datetime"]),
    ("details", &["name", "open"]),
    ("dfn", &[]),
    ("dialog", &["open", "closedby"]),
    ("div", &[]),
    ("dl", &[]),
    ("dt", &[]),
    ("em", &[]),
    ("embed", &["src", "type", "width", "height"]),
    ("fieldset", &["disabled", "form", "name"]),
    ("figcaption", &[]),
    ("figure", &[]),
    ("footer", &[]),
    ("form", &["accept-charset", "action", "autocomplete", "enctype", "method", "name", "novalidate", "target", "rel"]),
    ("h1", &[]),
    ("h2", &[]),
    ("h3", &[]),
    ("h4", &[]),
    ("h5", &[]),
    ("h6", &[]),
    ("head", &[]),
    ("header", &[]),
    ("hgroup", &[]),
    ("hr", &[]),
    ("html", &["manifest"]),
    ("i", &[]),
    (
        "iframe",
        &[
            "src",
            "srcdoc",
            "name",
            "sandbox",
            "allow",
            "allowfullscreen",
            "width",
            "height",
            "referrerpolicy",
            "loading",
        ],
    ),
    (
        "img",
        &[
            "alt",
            "src",
            "srcset",
            "sizes",
            "crossorigin",
            "usemap",
            "ismap",
            "width",
            "height",
            "referrerpolicy",
            "decoding",
            "loading",
            "fetchpriority",
        ],
    ),
    (
        "input",
        &[
            "accept",
            "alpha",
            "alt",
            "autocomplete",
            "checked",
            "colorspace",
            "dirname",
            "disabled",
            "form",
            "formaction",
            "formenctype",
            "formmethod",
            "formnovalidate",
            "formtarget",
            "height",
            "list",
            "max",
            "maxlength",
            "min",
            "minlength",
            "multiple",
            "name",
            "pattern",
            "placeholder",
            "popovertarget",
            "popovertargetaction",
            "readonly",
            "required",
            "size",
            "src",
            "step",
            "type",
            "value",
            "width",
        ],
    ),
    ("ins", &["cite", "datetime"]),
    ("kbd", &[]),
    ("label", &["for"]),
    ("legend", &[]),
    ("li", &["value"]),
    (
        "link",
        &[
            "href",
            "crossorigin",
            "rel",
            "as",
            "media",
            "hreflang",
            "type",
            "referrerpolicy",
            "sizes",
            "imagesrcset",
            "imagesizes",
            "integrity",
            "blocking",
            "color",
            "disabled",
            "fetchpriority",
        ],
    ),
    ("main", &[]),
    ("map", &["name"]),
    ("mark", &[]),
    ("menu", &[]),
    ("meta", &["name", "http-equiv", "content", "charset", "media"]),
    ("meter", &["value", "min", "max", "low", "high", "optimum"]),
    ("nav", &[]),
    ("noscript", &[]),
    ("object", &["data", "type", "name", "form", "width", "height"]),
    ("ol", &["reversed", "start", "type"]),
    ("optgroup", &["disabled", "label"]),
    ("option", &["disabled", "label", "selected", "value"]),
    ("output", &["for", "form", "name"]),
    ("p", &[]),
    ("picture", &[]),
    ("pre", &[]),
    ("progress", &["value", "max"]),
    ("q", &["cite"]),
    ("rp", &[]),
    ("rt", &[]),
    ("ruby", &[]),
    ("s", &[]),
    ("samp", &[]),
    (
        "script",
        &[
            "src",
            "type",
            "nomodule",
            "async",
            "defer",
            "crossorigin",
            "integrity",
            "referrerpolicy",
            "blocking",
            "fetchpriority",
        ],
    ),
    ("search", &[]),
    ("section", &[]),
    ("select", &["autocomplete", "disabled", "form", "multiple", "name", "required", "size"]),
    ("slot", &["name"]),
    ("small", &[]),
    ("source", &["type", "media", "src", "srcset", "sizes", "width", "height"]),
    ("span", &[]),
    ("strong", &[]),
    ("style", &["media", "blocking"]),
    ("sub", &[]),
    ("summary", &[]),
    ("sup", &[]),
    ("table", &[]),
    ("tbody", &[]),
    ("td", &["colspan", "rowspan", "headers"]),
    ("template", &["shadowrootmode", "shadowrootdelegatesfocus", "shadowrootclonable", "shadowrootserializable"]),
    (
        "textarea",
        &[
            "autocomplete",
            "cols",
            "dirname",
            "disabled",
            "form",
            "maxlength",
            "minlength",
            "name",
            "placeholder",
            "readonly",
            "required",
            "rows",
            "wrap",
        ],
    ),
    ("tfoot", &[]),
    ("th", &["colspan", "rowspan", "headers", "scope", "abbr"]),
    ("thead", &[]),
    ("time", &["datetime"]),
    ("title", &[]),
    ("tr", &[]),
    ("track", &["default", "kind", "label", "src", "srclang"]),
    ("u", &[]),
    ("ul", &[]),
    ("var", &[]),
    (
        "video",
        &[
            "src",
            "crossorigin",
            "poster",
            "preload",
            "autoplay",
            "playsinline",
            "loop",
            "muted",
            "controls",
            "width",
            "height",
        ],
    ),
    ("wbr", &[]),
];

/// Attributes the standard calls obsolete, where they aren't still part of the element.
const OBSOLETE: &[&str] = &[
    "align",
    "valign",
    "bgcolor",
    "background",
    "border",
    "cellpadding",
    "cellspacing",
    "nowrap",
    "frameborder",
    "scrolling",
    "marginheight",
    "marginwidth",
    "language",
    "noshade",
    "clear",
    "hspace",
    "vspace",
    "longdesc",
    "width",
    "height",
    "charset",
    "name",
    "type",
    "summary",
    "axis",
    "char",
    "charoff",
    "compact",
    "rules",
    "frame",
    "link",
    "vlink",
    "alink",
    "text",
    "size",
    "color",
    "face",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Fine,
    Obsolete,
    Unknown,
}

/// Prefixes and characters that belong to frameworks and templates, not to HTML: Vue,
/// Angular, Alpine, htmx, Svelte and the like read these, not the browser.
fn framework(name: &str) -> bool {
    name.contains([':', '@', '[', '(', '#', '*', '.', '{', '}', '$', '%'])
        || ["v-", "x-", "hx-", "ng-", "th:", "wire:", "on:", "bind:", "class:", "use:", "data-", "aria-"]
            .iter()
            .any(|p| name.starts_with(p))
}

/// What HTML makes of `attribute` on `element` (both in lower case). Elements it doesn't
/// know, such as custom elements and SVG's, take anything.
pub fn verdict(element: &str, attribute: &str) -> Verdict {
    let Some((_, own)) = ELEMENTS.iter().find(|(name, _)| *name == element) else { return Verdict::Fine };
    let event_handler = attribute.starts_with("on") && attribute.len() > 2;
    if event_handler || framework(attribute) || GLOBAL.contains(&attribute) || own.contains(&attribute) {
        Verdict::Fine
    } else if OBSOLETE.contains(&attribute) {
        Verdict::Obsolete
    } else {
        Verdict::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_each_elements_attributes() {
        assert_eq!(verdict("img", "alt"), Verdict::Fine);
        assert_eq!(verdict("img", "class"), Verdict::Fine);
        assert_eq!(verdict("div", "onclick"), Verdict::Fine);
        assert_eq!(verdict("div", "data-x"), Verdict::Fine);
        assert_eq!(verdict("div", "aria-label"), Verdict::Fine);
        assert_eq!(verdict("div", "v-if"), Verdict::Fine);
        assert_eq!(verdict("div", ":class"), Verdict::Fine);
        assert_eq!(verdict("div", "@click"), Verdict::Fine);
        assert_eq!(verdict("div", "[hidden]"), Verdict::Fine);
        assert_eq!(verdict("div", "href"), Verdict::Unknown);
        assert_eq!(verdict("div", "clas"), Verdict::Unknown);
        assert_eq!(verdict("td", "bgcolor"), Verdict::Obsolete);
        assert_eq!(verdict("table", "width"), Verdict::Obsolete);
        assert_eq!(verdict("img", "width"), Verdict::Fine);
        assert_eq!(verdict("my-widget", "anything"), Verdict::Fine);
        assert_eq!(verdict("path", "d"), Verdict::Fine);
    }
}
