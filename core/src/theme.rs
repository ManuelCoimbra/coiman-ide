//! Color themes. The built-in theme is VS Code's "Dark Modern"; any VS Code
//! color theme (`.json`, comments and trailing commas allowed, `include`
//! chains followed) can be loaded for workbench colors and token colors.
//! Installed VS Code themes are discovered for the Color Theme picker.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde_json::{Map, Value};

/// Workbench and editor colors, generic over the color type so a UI can
/// convert them once (see [`Palette::map`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette<C = Color> {
    pub dark: bool,
    pub editor_bg: C,
    pub editor_fg: C,
    pub sidebar_bg: C,
    pub activity_bg: C,
    pub tab_active_bg: C,
    pub tab_inactive_bg: C,
    pub tab_active_fg: C,
    pub tab_inactive_fg: C,
    pub accent: C,
    pub status_bg: C,
    pub status_fg: C,
    pub border: C,
    pub text: C,
    pub text_dim: C,
    pub line_number: C,
    pub line_number_active: C,
    pub modified: C,
    pub selection: C,
    pub cursor: C,
    pub indent_guide: C,
    pub current_line: C,
    pub bracket_match: C,
    pub rainbow: [C; 3],
    pub widget_bg: C,
    pub hover_bg: C,
}

impl<C: Copy> Palette<C> {
    /// The same palette with every color converted by `f`.
    pub fn map<D>(&self, f: impl Fn(C) -> D) -> Palette<D> {
        Palette {
            dark: self.dark,
            editor_bg: f(self.editor_bg),
            editor_fg: f(self.editor_fg),
            sidebar_bg: f(self.sidebar_bg),
            activity_bg: f(self.activity_bg),
            tab_active_bg: f(self.tab_active_bg),
            tab_inactive_bg: f(self.tab_inactive_bg),
            tab_active_fg: f(self.tab_active_fg),
            tab_inactive_fg: f(self.tab_inactive_fg),
            accent: f(self.accent),
            status_bg: f(self.status_bg),
            status_fg: f(self.status_fg),
            border: f(self.border),
            text: f(self.text),
            text_dim: f(self.text_dim),
            line_number: f(self.line_number),
            line_number_active: f(self.line_number_active),
            modified: f(self.modified),
            selection: f(self.selection),
            cursor: f(self.cursor),
            indent_guide: f(self.indent_guide),
            current_line: f(self.current_line),
            bracket_match: f(self.bracket_match),
            rainbow: self.rainbow.map(&f),
            widget_bg: f(self.widget_bg),
            hover_bg: f(self.hover_bg),
        }
    }
}

/// An sRGB color with unmultiplied alpha (UI-toolkit independent).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Color = Color::from_rgb(255, 255, 255);

    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const DARK: Palette = Palette {
    dark: true,
    editor_bg: rgb(0x1f1f1f),
    editor_fg: rgb(0xcccccc),
    sidebar_bg: rgb(0x181818),
    activity_bg: rgb(0x181818),
    tab_active_bg: rgb(0x1f1f1f),
    tab_inactive_bg: rgb(0x181818),
    tab_active_fg: rgb(0xffffff),
    tab_inactive_fg: rgb(0x9d9d9d),
    accent: rgb(0x0078d4),
    status_bg: rgb(0x007acc),
    status_fg: rgb(0xffffff),
    border: rgb(0x2b2b2b),
    text: rgb(0xcccccc),
    text_dim: rgb(0x858585),
    line_number: rgb(0x6e7681),
    line_number_active: rgb(0xcccccc),
    modified: rgb(0xe2c08d),
    selection: rgb(0x264f78),
    cursor: rgb(0xaeafad),
    indent_guide: rgb(0x404040),
    current_line: rgb(0x282828),
    bracket_match: rgb(0x888888),
    rainbow: [rgb(0xffd700), rgb(0xda70d6), rgb(0x179fff)],
    widget_bg: rgb(0x313131),
    hover_bg: rgb(0x2a2d2e),
};

const LIGHT: Palette = Palette {
    dark: false,
    editor_bg: rgb(0xffffff),
    editor_fg: rgb(0x3b3b3b),
    sidebar_bg: rgb(0xf8f8f8),
    activity_bg: rgb(0xf8f8f8),
    tab_active_bg: rgb(0xffffff),
    tab_inactive_bg: rgb(0xf8f8f8),
    tab_active_fg: rgb(0x3b3b3b),
    tab_inactive_fg: rgb(0x868686),
    accent: rgb(0x005fb8),
    status_bg: rgb(0x007acc),
    status_fg: rgb(0xffffff),
    border: rgb(0xe5e5e5),
    text: rgb(0x3b3b3b),
    text_dim: rgb(0x767676),
    line_number: rgb(0x6e7681),
    line_number_active: rgb(0x171184),
    modified: rgb(0x895503),
    selection: rgb(0xadd6ff),
    cursor: rgb(0x000000),
    indent_guide: rgb(0xd3d3d3),
    current_line: rgb(0xeeeeee),
    bracket_match: rgb(0xb9b9b9),
    rainbow: [rgb(0x0431fa), rgb(0x319331), rgb(0x7b3814)],
    widget_bg: rgb(0xffffff),
    hover_bg: rgb(0xf2f2f2),
};

/// One `tokenColors` rule: a TextMate scope (no selectors) and its style.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenRule {
    pub scope: String,
    pub color: Option<Color>,
    pub italic: bool,
    pub bold: bool,
}

pub struct Theme {
    pub name: String,
    pub palette: Palette,
    /// In file order; later rules win ties, as in VS Code.
    pub rules: Vec<TokenRule>,
}

/// Dark+ token colors (the syntax colors behind "Dark Modern").
const BUILTIN_RULES: &[(&str, u32, bool)] = &[
    ("comment", 0x6a9955, false),
    ("string", 0xce9178, false),
    ("constant.numeric", 0xb5cea8, false),
    ("constant.language", 0x569cd6, false),
    ("constant.character.escape", 0xd7ba7d, false),
    ("keyword", 0x569cd6, false),
    ("keyword.control", 0xc586c0, false),
    ("keyword.operator", 0xd4d4d4, false),
    ("storage", 0x569cd6, false),
    ("storage.type", 0x569cd6, false),
    ("entity.name.function", 0xdcdcaa, false),
    ("support.function", 0xdcdcaa, false),
    ("entity.name.type", 0x4ec9b0, false),
    ("support.type", 0x4ec9b0, false),
    ("entity.name.tag", 0x569cd6, false),
    ("entity.other.attribute-name", 0x9cdcfe, false),
    ("variable", 0x9cdcfe, false),
    ("variable.language", 0x569cd6, false),
    ("variable.other.constant", 0x4fc1ff, false),
    ("support.type.property-name", 0x9cdcfe, false),
    ("markup.heading", 0x569cd6, false),
    ("markup.inline.raw", 0xce9178, false),
    ("markup.underline.link", 0x3794ff, false),
    ("punctuation.definition.tag", 0x808080, false),
];

pub fn builtin() -> Theme {
    let rules = BUILTIN_RULES
        .iter()
        .map(|&(scope, c, italic)| TokenRule { scope: scope.to_owned(), color: Some(rgb(c)), italic, bold: false })
        .collect();
    Theme { name: "Dark Modern".to_owned(), palette: DARK, rules }
}

// ----- current theme -----------------------------------------------------------------

static CURRENT: RwLock<Option<Arc<Theme>>> = RwLock::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn current() -> Arc<Theme> {
    if let Some(t) = CURRENT.read().ok().and_then(|g| g.clone()) {
        return t;
    }
    let t = Arc::new(builtin());
    if let Ok(mut g) = CURRENT.write() {
        g.get_or_insert_with(|| t.clone());
    }
    t
}

/// Bumped on every theme change, so highlight caches can invalidate.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Makes `theme` the current theme for highlighting.
pub fn set_current(theme: Theme) {
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(Arc::new(theme));
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

// ----- token styles ------------------------------------------------------------------

/// TextMate scopes to try, in order, for a tree-sitter capture name.
fn scopes_for_capture(capture: &str) -> &'static [&'static str] {
    match capture {
        "comment" => &["comment"],
        "comment.documentation" => &["comment.block.documentation", "comment"],
        "string.special.key" => {
            &["support.type.property-name.json", "support.type.property-name", "variable.other.property"]
        }
        "string.escape" | "escape" => &["constant.character.escape"],
        "string" | "string.special" | "character" => &["string"],
        "number" => &["constant.numeric"],
        "boolean" => &["constant.language.boolean", "constant.language"],
        "keyword" => &["keyword", "storage.type", "storage"],
        "function" | "function.method" => &["entity.name.function", "support.function"],
        "function.macro" => &["entity.name.function.macro", "support.function", "entity.name.function"],
        "type" | "type.builtin" => &["entity.name.type", "support.type", "storage.type"],
        "constructor" => &["entity.name.type", "entity.name.function"],
        "variable" => &["variable"],
        "variable.parameter" => &["variable.parameter", "variable"],
        "variable.builtin" => &["variable.language"],
        "property" => &["variable.other.property", "support.type.property-name", "variable"],
        "constant" => &["variable.other.constant", "constant"],
        "constant.builtin" => &["constant.language"],
        "attribute" => &["entity.other.attribute-name", "meta.attribute"],
        "operator" => &["keyword.operator"],
        "punctuation.special" => &["punctuation.definition", "punctuation"],
        "text.title" => &["markup.heading", "entity.name.section"],
        "text.literal" => &["markup.inline.raw", "markup.raw"],
        "text.uri" | "text.reference" => &["markup.underline.link"],
        "tag" => &["entity.name.tag"],
        _ => &[],
    }
}

/// Best rule for a token scope: the most specific rule scope that is the token
/// scope or a dot-prefix of it; later rules win ties.
fn match_scope<'a>(rules: &'a [TokenRule], scope: &str) -> Option<&'a TokenRule> {
    let mut best: Option<(usize, &TokenRule)> = None;
    for r in rules {
        let matches =
            scope == r.scope || (scope.starts_with(&r.scope) && scope.as_bytes().get(r.scope.len()) == Some(&b'.'));
        if matches && r.color.is_some() {
            let score = r.scope.split('.').count();
            if best.is_none_or(|(s, _)| score >= s) {
                best = Some((score, r));
            }
        }
    }
    best.map(|(_, r)| r)
}

/// Color and italic flag for a tree-sitter capture under `theme`.
pub fn capture_style(theme: &Theme, capture: &str) -> Option<(Color, bool)> {
    // Fall back through the dotted capture name ("function.method" -> "function").
    let mut name = capture;
    loop {
        for scope in scopes_for_capture(name) {
            if let Some(r) = match_scope(&theme.rules, scope) {
                return r.color.map(|c| (c, r.italic));
            }
        }
        name = &name[..name.rfind('.')?];
    }
}

/// The theme's token rules as a syntect theme (for non-tree-sitter languages).
pub fn syntect_theme(theme: &Theme) -> syntect::highlighting::Theme {
    use std::str::FromStr;
    use syntect::highlighting::{
        Color as SynColor, FontStyle, ScopeSelectors, StyleModifier, ThemeItem, ThemeSettings,
    };
    let c = |c: Color| SynColor { r: c.r, g: c.g, b: c.b, a: c.a };
    let scopes = theme
        .rules
        .iter()
        .filter_map(|r| {
            let mut font = FontStyle::empty();
            if r.italic {
                font |= FontStyle::ITALIC;
            }
            if r.bold {
                font |= FontStyle::BOLD;
            }
            Some(ThemeItem {
                scope: ScopeSelectors::from_str(&r.scope).ok()?,
                style: StyleModifier { foreground: r.color.map(c), background: None, font_style: Some(font) },
            })
        })
        .collect();
    syntect::highlighting::Theme {
        name: Some(theme.name.clone()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(c(theme.palette.editor_fg)),
            background: Some(c(theme.palette.editor_bg)),
            ..Default::default()
        },
        scopes,
    }
}

// ----- VS Code theme files -----------------------------------------------------------

/// Removes `//` and `/* */` comments and trailing commas, outside strings.
fn strip_jsonc(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match (c, chars.get(i + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            (',', _) => {
                // Drop the comma if only whitespace/comments lead to a closing bracket.
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if !matches!(chars.get(j), Some('}' | ']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// "#rgb", "#rgba", "#rrggbb" or "#rrggbbaa".
fn parse_color(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
    let (r, g, b, a) = match digits.len() {
        3 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, 255),
        4 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, digits[3] * 17),
        6 => (digits[0] * 16 + digits[1], digits[2] * 16 + digits[3], digits[4] * 16 + digits[5], 255),
        8 => (
            digits[0] * 16 + digits[1],
            digits[2] * 16 + digits[3],
            digits[4] * 16 + digits[5],
            digits[6] * 16 + digits[7],
        ),
        _ => return None,
    };
    Some(Color::from_rgba(r, g, b, a))
}

#[derive(Default)]
struct RawTheme {
    name: Option<String>,
    kind: Option<String>,
    colors: Map<String, Value>,
    token_colors: Vec<Value>,
}

fn read_theme_file(path: &Path, depth: usize) -> Result<RawTheme, String> {
    if depth > 8 {
        return Err(format!("{}: `include` nests too deeply", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: Value = serde_json::from_str(&strip_jsonc(&text)).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut raw = match json.get("include").and_then(Value::as_str) {
        Some(inc) => read_theme_file(&path.parent().unwrap_or(Path::new(".")).join(inc), depth + 1)?,
        None => RawTheme::default(),
    };
    if let Some(n) = json.get("name").and_then(Value::as_str) {
        raw.name = Some(n.to_owned());
    }
    if let Some(t) = json.get("type").and_then(Value::as_str) {
        raw.kind = Some(t.to_owned());
    }
    if let Some(Value::Object(colors)) = json.get("colors") {
        for (k, v) in colors {
            raw.colors.insert(k.clone(), v.clone());
        }
    }
    if let Some(Value::Array(tokens)) = json.get("tokenColors") {
        raw.token_colors.extend(tokens.iter().cloned());
    }
    Ok(raw)
}

/// Loads a VS Code color theme. `ui_theme` is the manifest's "vs"/"vs-dark"/"hc-black"
/// hint, used when the file has no `type`.
pub fn load_vscode_theme(path: &Path, ui_theme: Option<&str>) -> Result<Theme, String> {
    let raw = read_theme_file(path, 0)?;
    let kind = raw.kind.as_deref().or(ui_theme).unwrap_or("dark");
    let dark = !matches!(kind, "light" | "vs" | "hc-light");
    let base = if dark { DARK } else { LIGHT };
    let col = |keys: &[&str], fallback: Color| {
        keys.iter().find_map(|k| raw.colors.get(*k).and_then(Value::as_str).and_then(parse_color)).unwrap_or(fallback)
    };

    let editor_bg = col(&["editor.background"], base.editor_bg);
    let editor_fg = col(&["editor.foreground", "foreground"], base.editor_fg);
    let mut rainbow = base.rainbow;
    for (i, r) in rainbow.iter_mut().enumerate() {
        *r = col(&[&format!("editorBracketHighlight.foreground{}", i + 1)], *r);
    }
    let palette = Palette {
        dark,
        editor_bg,
        editor_fg,
        sidebar_bg: col(&["sideBar.background"], editor_bg),
        activity_bg: col(&["activityBar.background", "sideBar.background"], base.activity_bg),
        tab_active_bg: col(&["tab.activeBackground"], editor_bg),
        tab_inactive_bg: col(&["tab.inactiveBackground", "editorGroupHeader.tabsBackground"], base.tab_inactive_bg),
        tab_active_fg: col(&["tab.activeForeground"], base.tab_active_fg),
        tab_inactive_fg: col(&["tab.inactiveForeground"], base.tab_inactive_fg),
        accent: col(&["focusBorder", "activityBar.activeBorder", "button.background"], base.accent),
        status_bg: col(&["statusBar.background"], base.status_bg),
        status_fg: col(&["statusBar.foreground"], base.status_fg),
        border: col(&["sideBar.border", "panel.border", "editorGroup.border"], base.border),
        text: col(&["foreground", "sideBar.foreground", "editor.foreground"], base.text),
        text_dim: col(&["descriptionForeground", "tab.inactiveForeground"], base.text_dim),
        line_number: col(&["editorLineNumber.foreground"], base.line_number),
        line_number_active: col(&["editorLineNumber.activeForeground"], editor_fg),
        modified: col(&["gitDecoration.modifiedResourceForeground"], base.modified),
        selection: col(&["editor.selectionBackground"], base.selection),
        cursor: col(&["editorCursor.foreground"], editor_fg),
        indent_guide: col(&["editorIndentGuide.background1", "editorIndentGuide.background"], base.indent_guide),
        current_line: col(&["editor.lineHighlightBorder", "editor.lineHighlightBackground"], base.current_line),
        bracket_match: col(&["editorBracketMatch.border"], base.bracket_match),
        rainbow,
        widget_bg: col(&["input.background", "dropdown.background"], base.widget_bg),
        hover_bg: col(&["list.hoverBackground"], base.hover_bg),
    };

    let mut rules = Vec::new();
    for tc in &raw.token_colors {
        let settings = tc.get("settings");
        let color = settings.and_then(|s| s.get("foreground")).and_then(Value::as_str).and_then(parse_color);
        let font = settings.and_then(|s| s.get("fontStyle")).and_then(Value::as_str).unwrap_or("");
        let scopes: Vec<String> = match tc.get("scope") {
            Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_owned()).collect(),
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|x| x.trim().to_owned()).collect(),
            _ => continue, // A scope-less entry sets default colors, already covered by `colors`.
        };
        for scope in scopes.into_iter().filter(|s| !s.is_empty()) {
            rules.push(TokenRule { scope, color, italic: font.contains("italic"), bold: font.contains("bold") });
        }
    }
    let name =
        raw.name.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(Theme { name, palette, rules })
}

/// A theme offered in the Color Theme picker.
#[derive(Clone, Debug)]
pub struct ThemeEntry {
    pub label: String,
    pub path: PathBuf,
    pub ui_theme: Option<String>,
}

/// VS Code's bundled extensions folder (newest install) and the user's extensions.
fn vscode_extension_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let install = PathBuf::from(local).join("Programs").join("Microsoft VS Code");
        let newest = std::fs::read_dir(&install).ok().and_then(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path().join("resources").join("app").join("extensions"))
                .filter(|p| p.is_dir())
                .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        });
        dirs.extend(newest.or_else(|| Some(install.join("resources").join("app").join("extensions"))));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        dirs.push(PathBuf::from(home).join(".vscode").join("extensions"));
    }
    dirs
}

/// Themes from installed VS Code (bundled + user extensions) and
/// `<config dir>/rust-code/themes/*.json`, deduplicated by label.
pub fn discover() -> Vec<ThemeEntry> {
    let mut out: Vec<ThemeEntry> = Vec::new();
    for ext_root in vscode_extension_dirs() {
        let Ok(rd) = std::fs::read_dir(&ext_root) else { continue };
        for ext in rd.filter_map(Result::ok).map(|e| e.path()) {
            out.extend(themes_in_extension(&ext));
        }
    }
    if let Some(dir) = crate::settings::config_dir().map(|d| d.join("themes"))
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for p in rd.filter_map(Result::ok).map(|e| e.path()) {
            if p.extension().is_some_and(|e| e == "json") {
                let label = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(ThemeEntry { label, path: p, ui_theme: None });
            }
        }
    }
    out.sort_by_key(|a| a.label.to_lowercase());
    out.dedup_by(|a, b| a.label == b.label);
    out
}

fn themes_in_extension(ext: &Path) -> Vec<ThemeEntry> {
    let read_json =
        |p: PathBuf| std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Value>(&strip_jsonc(&t)).ok());
    let Some(pkg) = read_json(ext.join("package.json")) else { return Vec::new() };
    let Some(themes) = pkg.pointer("/contributes/themes").and_then(Value::as_array) else { return Vec::new() };
    let nls = read_json(ext.join("package.nls.json"));
    themes
        .iter()
        .filter_map(|t| {
            let path = ext.join(t.get("path")?.as_str()?);
            let id = t.get("id").and_then(Value::as_str);
            let mut label = t.get("label").and_then(Value::as_str).map(str::to_owned);
            // "%key%" labels are localized through package.nls.json.
            if let Some(key) = label.as_deref().and_then(|l| l.strip_prefix('%')).and_then(|l| l.strip_suffix('%')) {
                label = nls
                    .as_ref()
                    .and_then(|n| n.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(id.map(str::to_owned));
            }
            let label = label.or(id.map(str::to_owned))?;
            Some(ThemeEntry { label, path, ui_theme: t.get("uiTheme").and_then(Value::as_str).map(str::to_owned) })
        })
        .collect()
}

/// Resolves the `workbench.color_theme` setting: a picker label, or a path to a
/// theme file. Falls back to the built-in theme with an error message.
pub fn resolve(name: &str, available: &[ThemeEntry]) -> (Theme, Option<String>) {
    if let Some(e) = available.iter().find(|e| e.label.eq_ignore_ascii_case(name)) {
        return match load_vscode_theme(&e.path, e.ui_theme.as_deref()) {
            Ok(mut t) => {
                t.name = e.label.clone();
                (t, None)
            }
            Err(err) => (builtin(), Some(err)),
        };
    }
    let path = Path::new(name);
    if path.extension().is_some_and(|e| e == "json") {
        return match load_vscode_theme(path, None) {
            Ok(t) => (t, None),
            Err(err) => (builtin(), Some(err)),
        };
    }
    let error = (!name.eq_ignore_ascii_case("Dark Modern")).then(|| format!("Color theme `{name}` not found"));
    (builtin(), error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_comments_and_trailing_commas() {
        let src = r#"{ // comment
            "a": "x // not a comment", /* block */
            "b": [1, 2,],
        }"#;
        let v: Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(v["a"], "x // not a comment");
        assert_eq!(v["b"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn colors_in_all_hex_forms() {
        assert_eq!(parse_color("#fff"), Some(Color::WHITE));
        assert_eq!(parse_color("#1e1e1e"), Some(rgb(0x1e1e1e)));
        assert_eq!(parse_color("#00000000"), Some(Color::from_rgba(0, 0, 0, 0)));
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn most_specific_scope_wins() {
        let t = builtin();
        assert_eq!(match_scope(&t.rules, "keyword.operator.rust").unwrap().color, Some(rgb(0xd4d4d4)));
        assert_eq!(match_scope(&t.rules, "keyword.other").unwrap().color, Some(rgb(0x569cd6)));
        assert!(match_scope(&t.rules, "keywordish").is_none());
        assert_eq!(capture_style(&t, "function.method"), Some((rgb(0xdcdcaa), false)));
        assert_eq!(capture_style(&t, "string.special.key"), Some((rgb(0x9cdcfe), false)));
    }

    #[test]
    fn vscode_theme_with_include_and_light_type() {
        let dir = std::env::temp_dir().join(format!("rust-code-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("base.json"),
            r##"{ "colors": { "editor.background": "#ffffff", "editor.foreground": "#111111" },
                 "tokenColors": [ { "scope": "comment", "settings": { "foreground": "#008000", "fontStyle": "italic" } } ] }"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("child.json"),
            r##"{ "name": "Child", "type": "light", "include": "./base.json",
                 // child overrides background, adds a keyword rule
                 "colors": { "editor.background": "#fafafa", },
                 "tokenColors": [ { "scope": ["keyword", "storage"], "settings": { "foreground": "#0000ff" } } ] }"##,
        )
        .unwrap();
        let t = load_vscode_theme(&dir.join("child.json"), None).unwrap();
        assert_eq!(t.name, "Child");
        assert!(!t.palette.dark);
        assert_eq!(t.palette.editor_bg, rgb(0xfafafa));
        assert_eq!(t.palette.editor_fg, rgb(0x111111), "inherited from include");
        assert_eq!(capture_style(&t, "comment"), Some((rgb(0x008000), true)));
        assert_eq!(capture_style(&t, "keyword"), Some((rgb(0x0000ff), false)));
        assert!(!syntect_theme(&t).scopes.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installed_vscode_themes_load_when_present() {
        // Only meaningful on a machine with VS Code; passes trivially otherwise.
        for e in discover().iter().take(30) {
            let r = load_vscode_theme(&e.path, e.ui_theme.as_deref());
            assert!(r.is_ok(), "{}: {:?}", e.label, r.err());
        }
    }
}
