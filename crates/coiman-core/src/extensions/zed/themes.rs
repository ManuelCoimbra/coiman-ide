//! Zed color theme (`https://zed.dev/schema/themes/v0.2.0.json`) loader and mapper
//! into Coiman [`Theme`] and [`Palette`].

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::appearance::theme::{
    self, Color, Palette, SemanticRule, Theme, ThemeEntry, TokenRule, parse_color, strip_jsonc,
};

use super::registry::InstalledExtension;

#[derive(Debug, Deserialize)]
struct ZedThemeFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    themes: Vec<ZedThemeVariant>,
}

#[derive(Debug, Deserialize)]
struct ZedThemeVariant {
    name: String,
    #[serde(default)]
    appearance: String,
    #[serde(default)]
    style: ZedThemeStyle,
}

#[derive(Debug, Default, Deserialize)]
struct ZedThemeStyle {
    #[serde(default)]
    accents: Vec<String>,
    #[serde(default)]
    players: Vec<ZedPlayerStyle>,
    #[serde(default)]
    syntax: BTreeMap<String, ZedSyntaxStyle>,
    #[serde(flatten)]
    colors: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct ZedPlayerStyle {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    selection: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ZedSyntaxStyle {
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    font_style: Option<String>,
    #[serde(default)]
    font_weight: Option<u16>,
}

/// Checks whether a parsed JSON value looks like a Zed theme file rather than a VS Code theme.
pub fn looks_like_zed_theme(value: &Value) -> bool {
    value.get("$schema").and_then(Value::as_str).is_some_and(|s| s.contains("zed.dev/schema/themes"))
        || value
            .get("themes")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|t| t.get("style").is_some() && t.get("appearance").is_some()))
}

/// Discovers theme entries exposed by enabled Zed extensions.
pub fn discover_entries(extensions: &[InstalledExtension]) -> Vec<ThemeEntry> {
    let mut out = Vec::new();
    for ext in extensions {
        for rel in &ext.manifest.themes {
            let path = ext.path.join(rel);
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(file) = serde_json::from_str::<ZedThemeFile>(&strip_jsonc(&text)) else {
                continue;
            };
            for variant in file.themes {
                if variant.name.trim().is_empty() {
                    continue;
                }
                out.push(ThemeEntry {
                    label: variant.name.clone(),
                    path: path.clone(),
                    ui_theme: Some(format!("zed:{}", variant.name)),
                });
            }
        }
    }
    out
}

/// Loads a Zed theme JSON file, picking `variant_name` if supplied (else the first variant).
pub fn load_theme(path: &Path, variant_name: Option<&str>) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_theme_json(&text, variant_name).map_err(|e| format!("{}: {e}", path.display()))
}

/// Parses Zed theme JSON/JSONC into a Coiman [`Theme`].
pub fn parse_theme_json(text: &str, variant_name: Option<&str>) -> Result<Theme, String> {
    let stripped = strip_jsonc(text);
    let file: ZedThemeFile = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
    let variant = match variant_name {
        Some(target) => file
            .themes
            .into_iter()
            .find(|v| v.name.eq_ignore_ascii_case(target))
            .ok_or_else(|| format!("Theme variant `{target}` not found"))?,
        None => file.themes.into_iter().next().ok_or_else(|| "Zed theme file has no `themes` entries".to_owned())?,
    };
    let name = if variant.name.is_empty() { file.name } else { variant.name };
    let dark = !variant.appearance.eq_ignore_ascii_case("light");
    let base = if dark { theme::coiman().palette } else { theme::coiman_light().palette };
    let style = &variant.style;

    let color_of = |key: &str| -> Option<Color> {
        style.colors.get(key).and_then(Value::as_str).and_then(parse_color)
    };
    let first_of = |keys: &[&str], fallback: Color| -> Color {
        keys.iter().find_map(|k| color_of(k)).unwrap_or(fallback)
    };

    let editor_bg = first_of(&["editor.background", "background"], base.editor_bg);
    let editor_fg = first_of(&["editor.foreground", "text"], base.editor_fg);
    let sidebar_bg = first_of(&["panel.background", "surface.background", "background"], editor_bg);
    let activity_bg = first_of(&["title_bar.background", "status_bar.background", "background"], sidebar_bg);
    let text = first_of(&["text", "editor.foreground"], editor_fg);
    let accent_0 = style.accents.first().and_then(|s| parse_color(s));
    let accent = accent_0.unwrap_or_else(|| {
        first_of(&["border.focused", "text.accent", "icon.accent"], base.accent)
    });

    let p0 = style.players.first();
    let cursor = p0
        .and_then(|p| p.cursor.as_deref())
        .and_then(parse_color)
        .unwrap_or(accent);
    let selection = p0
        .and_then(|p| p.selection.as_deref())
        .and_then(parse_color)
        .unwrap_or_else(|| first_of(&["element.selected", "ghost_element.selected"], base.selection));

    let rainbow = [
        style.accents.get(1).and_then(|s| parse_color(s)).unwrap_or(base.rainbow[0]),
        style.accents.get(2).and_then(|s| parse_color(s)).unwrap_or(base.rainbow[1]),
        style.accents.get(3).and_then(|s| parse_color(s)).unwrap_or(base.rainbow[2]),
    ];

    let palette = Palette {
        dark,
        editor_bg,
        editor_fg,
        sidebar_bg,
        activity_bg,
        tab_active_bg: first_of(&["tab.active_background"], editor_bg),
        tab_inactive_bg: first_of(&["tab.inactive_background", "tab_bar.background"], sidebar_bg),
        tab_active_fg: text,
        tab_inactive_fg: first_of(&["text.muted", "text.disabled"], base.tab_inactive_fg),
        accent,
        status_bg: first_of(&["status_bar.background", "title_bar.background"], activity_bg),
        status_fg: text,
        border: first_of(&["border", "pane_group.border", "border.variant"], base.border),
        text,
        text_dim: first_of(&["text.muted", "text.disabled", "icon.muted"], base.text_dim),
        line_number: first_of(&["editor.line_number"], base.line_number),
        line_number_active: first_of(&["editor.active_line_number"], text),
        modified: first_of(&["modified", "version_control.modified"], base.modified),
        added: first_of(&["created", "version_control.added"], base.added),
        removed: first_of(&["deleted", "version_control.deleted"], base.removed),
        selection,
        cursor,
        indent_guide: first_of(&["editor.indent_guide", "panel.indent_guide"], base.indent_guide),
        indent_guide_active: first_of(
            &["editor.indent_guide_active", "panel.indent_guide_active"],
            base.indent_guide_active,
        ),
        current_line: first_of(&["editor.active_line.background", "element.hover"], base.current_line),
        bracket_match: first_of(&["editor.document_highlight.bracket_background"], base.bracket_match),
        rainbow,
        bracket_unexpected: first_of(&["error", "unreachable"], base.bracket_unexpected),
        widget_bg: first_of(&["elevated_surface.background", "element.background"], sidebar_bg),
        hover_bg: first_of(&["element.hover", "ghost_element.hover"], base.hover_bg),
    };

    let mut rules = Vec::new();
    let mut semantic = Vec::new();
    for (token, st) in &style.syntax {
        let color = st.color.as_deref().and_then(parse_color);
        let italic = st.font_style.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("italic") || s.eq_ignore_ascii_case("oblique"));
        let bold = st.font_weight.is_some_and(|w| w >= 600);
        if color.is_none() && !italic && !bold {
            continue;
        }
        // Keep the exact Zed syntax capture key plus standard TextMate scopes used by Coiman/syntect.
        let mut scopes = vec![token.clone()];
        scopes.extend(zed_token_to_tm_scopes(token).iter().map(|s| (*s).to_owned()));
        for scope in scopes {
            rules.push(TokenRule { scope, color, italic, bold });
        }
        if let Some(mapped) = zed_token_to_semantic_type(token) {
            semantic.push(SemanticRule {
                token_type: mapped.to_owned(),
                modifiers: Vec::new(),
                color,
                italic: italic.then_some(true),
                strike: None,
            });
        }
    }

    Ok(Theme { name, palette, rules, semantic })
}

fn zed_token_to_tm_scopes(token: &str) -> &'static [&'static str] {
    match token {
        "comment" => &["comment", "comment.line", "comment.block"],
        "comment.doc" => &["comment.block.documentation", "comment.documentation"],
        "string" => &["string", "string.quoted"],
        "string.escape" => &["constant.character.escape"],
        "string.regex" => &["string.regexp"],
        "string.special" => &["string.other", "string.quoted.other"],
        "string.special.symbol" => &["constant.other.symbol"],
        "number" => &["constant.numeric"],
        "boolean" => &["constant.language.boolean"],
        "constant" => &["constant", "constant.language"],
        "constant.builtin" => &["constant.language"],
        "keyword" | "keyword.control" => &["keyword", "keyword.control"],
        "keyword.operator" | "operator" => &["keyword.operator"],
        "function" | "function.definition" | "function.method" => {
            &["entity.name.function", "support.function", "meta.function-call"]
        }
        "function.builtin" => &["support.function"],
        "function.macro" | "preproc" => &["entity.name.function.macro", "meta.preprocessor"],
        "type" | "type.interface" | "type.class" => &["entity.name.type", "support.type", "entity.name.class"],
        "type.builtin" => &["support.type.builtin", "storage.type"],
        "constructor" => &["entity.name.type", "entity.name.function.constructor"],
        "variable" => &["variable", "variable.other"],
        "variable.builtin" => &["variable.language"],
        "variable.parameter" => &["variable.parameter"],
        "variable.member" | "property" => &["variable.other.member", "entity.name.tag.property", "meta.object-literal.key"],
        "attribute" => &["entity.other.attribute-name"],
        "tag" => &["entity.name.tag"],
        "label" => &["entity.name.label"],
        "namespace" | "module" => &["entity.name.namespace", "entity.name.module"],
        "punctuation" => &["punctuation"],
        "punctuation.bracket" => &["punctuation.section", "meta.brace"],
        "punctuation.delimiter" => &["punctuation.separator", "punctuation.terminator"],
        "title" | "emphasis.strong" => &["markup.heading", "markup.bold"],
        "emphasis" => &["markup.italic"],
        "link_uri" | "link_text" => &["markup.underline.link"],
        _ => &[],
    }
}

fn zed_token_to_semantic_type(token: &str) -> Option<&'static str> {
    Some(match token {
        "comment" | "comment.doc" => "comment",
        "string" | "string.special" => "string",
        "number" => "number",
        "keyword" => "keyword",
        "operator" => "operator",
        "function" | "function.method" => "function",
        "function.macro" => "macro",
        "type" | "type.builtin" => "type",
        "type.interface" => "interface",
        "variable" => "variable",
        "variable.parameter" => "parameter",
        "property" | "variable.member" => "property",
        "namespace" => "namespace",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_zed_theme_v0_2_0_to_coiman_theme() {
        let json = r##"{
            "$schema": "https://zed.dev/schema/themes/v0.2.0.json",
            "name": "Sample",
            "themes": [
                {
                    "name": "Sample Light",
                    "appearance": "light",
                    "style": {
                        "background": "#dce0e8",
                        "editor.background": "#eff1f5",
                        "editor.foreground": "#4c4f69",
                        "panel.background": "#e6e9ef",
                        "text": "#4c4f69",
                        "text.accent": "#8839ef",
                        "syntax": {
                            "comment": { "color": "#8c8fa1", "font_style": "italic" },
                            "string": { "color": "#40a02b" },
                            "keyword": { "color": "#8839ef", "font_weight": 700 }
                        }
                    }
                }
            ]
        }"##;
        let t = parse_theme_json(json, Some("Sample Light")).unwrap();
        assert_eq!(t.name, "Sample Light");
        assert!(!t.palette.dark);
        assert_eq!(t.palette.editor_bg, parse_color("#eff1f5").unwrap());
        assert!(t.rules.iter().any(|r| r.scope == "comment" && r.italic));
        assert!(t.rules.iter().any(|r| r.scope == "keyword" && r.bold));
    }

    #[test]
    fn parses_real_catppuccin_zed_theme_if_present() {
        let path = Path::new("/tmp/toml_ext/themes/catppuccin-mauve.json");
        if path.is_file() {
            let t = load_theme(path, Some("Catppuccin Mocha")).unwrap();
            assert_eq!(t.name, "Catppuccin Mocha");
            assert!(t.palette.dark);
            assert!(!t.rules.is_empty());
        }
    }
}
