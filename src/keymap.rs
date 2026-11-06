//! Keyboard shortcuts as data: built-in VS Code defaults plus the user's
//! `keybindings.toml`, using VS Code command IDs.
//!
//! ```toml
//! [[bindings]]
//! key = "ctrl+shift+d"
//! command = "editor.action.duplicateSelection"
//! when = "editorTextFocus"        # optional: only while the editor has focus
//!
//! [[bindings]]                     # "-" removes a default binding
//! key = "ctrl+d"                   # (omit `key` to remove all of its bindings)
//! command = "-editor.action.addSelectionToNextFindMatch"
//! ```

use std::path::Path;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde::Deserialize;

use crate::commands::{Command, command_from_id, command_id, runs_in_terminal};

/// VS Code's default keybindings on Windows for the commands we have.
/// `true` = only while the editor has focus (`editorTextFocus`).
const DEFAULTS: &[(&str, Command, bool)] = &[
    ("ctrl+shift+p", Command::CommandPalette, false),
    ("f1", Command::CommandPalette, false),
    ("ctrl+p", Command::QuickOpen, false),
    ("ctrl+n", Command::NewFile, false),
    ("ctrl+o", Command::OpenFile, false),
    ("ctrl+k ctrl+o", Command::OpenFolder, false),
    ("ctrl+r", Command::OpenRecent, false),
    ("ctrl+alt+k", Command::ToggleBookmark, true),
    ("ctrl+alt+l", Command::NextBookmark, true),
    ("ctrl+alt+j", Command::PreviousBookmark, true),
    ("ctrl+k v", Command::MarkdownPreview, false),
    ("alt+o", Command::SwitchHeaderSource, true),
    ("ctrl+s", Command::Save, false),
    ("ctrl+shift+s", Command::SaveAs, false),
    ("ctrl+k s", Command::SaveAll, false),
    ("ctrl+w", Command::CloseTab, false),
    ("ctrl+f4", Command::CloseTab, false),
    ("ctrl+b", Command::ToggleSidebar, false),
    ("ctrl+`", Command::ToggleTerminal, false),
    ("ctrl+shift+e", Command::ShowExplorer, false),
    ("ctrl+shift+f", Command::ShowSearch, false),
    ("ctrl+f", Command::Find, false),
    ("ctrl+h", Command::Replace, false),
    ("ctrl+g", Command::GoToLine, false),
    ("ctrl+=", Command::ZoomIn, false),
    ("ctrl+plus", Command::ZoomIn, false),
    ("ctrl+-", Command::ZoomOut, false),
    ("ctrl+0", Command::ResetZoom, false),
    ("ctrl+shift+=", Command::CodeZoomIn, false),
    ("ctrl+shift+plus", Command::CodeZoomIn, false),
    ("ctrl+shift+-", Command::CodeZoomOut, false),
    ("ctrl+shift+0", Command::ResetCodeZoom, false),
    ("ctrl+\\", Command::SplitEditor, false),
    ("ctrl+1", Command::FocusFirstPane, false),
    ("ctrl+2", Command::FocusSecondPane, false),
    ("shift+alt+0", Command::ToggleSplitLayout, false),
    ("ctrl+shift+t", Command::ReopenClosedEditor, false),
    ("alt+z", Command::ToggleWordWrap, false),
    ("ctrl+k z", Command::ToggleZenMode, false),
    ("ctrl+tab", Command::NextEditor, false),
    ("ctrl+shift+tab", Command::PreviousEditor, false),
    ("ctrl+pagedown", Command::NextEditor, false),
    ("ctrl+pageup", Command::PreviousEditor, false),
    ("ctrl+,", Command::OpenUserSettings, false),
    ("ctrl+k ctrl+s", Command::OpenKeybindings, false),
    ("ctrl+k ctrl+t", Command::SelectTheme, false),
    ("ctrl+/", Command::ToggleLineComment, true),
    ("alt+up", Command::MoveLinesUp, true),
    ("alt+down", Command::MoveLinesDown, true),
    ("shift+alt+up", Command::CopyLinesUp, true),
    ("shift+alt+down", Command::CopyLinesDown, true),
    ("ctrl+shift+k", Command::DeleteLines, true),
    ("ctrl+]", Command::IndentLines, true),
    ("ctrl+[", Command::OutdentLines, true),
    ("shift+tab", Command::OutdentLines, true),
    ("ctrl+d", Command::SelectNextOccurrence, true),
    ("ctrl+shift+l", Command::SelectAllOccurrences, true),
    ("ctrl+alt+right", Command::TreeSlurp, true),
    ("ctrl+alt+left", Command::TreeBarf, true),
    ("ctrl+shift+alt+left", Command::TreeSlurpBack, true),
    ("ctrl+shift+alt+right", Command::TreeBarfBack, true),
    ("shift+alt+r", Command::TreeRaise, true),
    ("shift+alt+s", Command::TreeSplice, true),
    ("shift+alt+w", Command::TreeWrapParens, true),
    ("ctrl+alt+up", Command::AddCursorAbove, true),
    ("ctrl+alt+down", Command::AddCursorBelow, true),
    ("ctrl+shift+\\", Command::JumpToBracket, true),
    ("ctrl+space", Command::TriggerSuggest, true),
    ("ctrl+k ctrl+i", Command::ShowHover, true),
    ("f12", Command::GoToDefinition, true),
    ("ctrl+f12", Command::GoToImplementation, true),
    ("ctrl+t", Command::GoToWorkspaceSymbol, false),
    ("ctrl+shift+space", Command::TriggerSignatureHelp, true),
    ("shift+f12", Command::FindReferences, true),
    ("shift+alt+h", Command::ShowIncomingCalls, true),
    ("f2", Command::RenameSymbol, true),
    ("ctrl+.", Command::QuickFix, true),
    ("shift+alt+f", Command::FormatDocument, true),
    ("ctrl+shift+m", Command::ShowProblems, false),
    ("f8", Command::NextProblem, false),
    ("shift+f8", Command::PreviousProblem, false),
    ("ctrl+shift+`", Command::NewTerminal, false),
    ("ctrl+shift+g", Command::ShowSourceControl, false),
    ("ctrl+k ctrl+alt+s", Command::GitStageHunk, true),
    ("ctrl+k ctrl+r", Command::GitRevertHunk, true),
    ("alt+f5", Command::NextChange, true),
    ("shift+alt+f5", Command::PreviousChange, true),
    ("ctrl+f5", Command::RunWithoutDebugging, false),
    ("shift+f10", Command::RunProgram, false),
    ("ctrl+shift+f10", Command::RunCurrentFile, false),
    ("ctrl+enter", Command::SendToRepl, true),
    ("ctrl+; c", Command::TestAtCursor, false),
    ("ctrl+; f", Command::TestCurrentFile, false),
    ("ctrl+; a", Command::TestAll, false),
    ("ctrl+shift+u", Command::ShowOutput, false),
    ("ctrl+shift+[", Command::Fold, true),
    ("ctrl+shift+]", Command::Unfold, true),
    ("ctrl+k ctrl+l", Command::ToggleFold, true),
    ("ctrl+k ctrl+0", Command::FoldAll, true),
    ("ctrl+k ctrl+j", Command::UnfoldAll, true),
    ("ctrl+shift+o", Command::GoToSymbol, false),
    ("shift+alt+right", Command::ExpandSelection, true),
    ("shift+alt+left", Command::ShrinkSelection, true),
    ("f5", Command::StartDebugging, false),
    ("shift+f5", Command::StopDebugging, false),
    ("ctrl+shift+f5", Command::RestartDebugging, false),
    ("f10", Command::StepOver, false),
    ("f11", Command::StepInto, false),
    ("shift+f11", Command::StepOut, false),
    ("f6", Command::PauseDebugging, false),
    ("f9", Command::ToggleBreakpoint, true),
    ("ctrl+shift+d", Command::ShowDebug, false),
    ("ctrl+shift+y", Command::ShowDebugConsole, false),
    ("ctrl+shift+x", Command::ShowExtensions, false),
    ("ctrl+; ctrl+c", Command::DebugTestAtCursor, false),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// One stroke, or two for a chord like `Ctrl+K Ctrl+O`.
    pub keys: Vec<KeyboardShortcut>,
    pub command: Command,
    pub editor_only: bool,
}

#[derive(Clone, Debug)]
pub struct Keymap {
    /// In priority order, lowest first (later bindings win, like VS Code).
    bindings: Vec<Binding>,
    /// Indices into `bindings` in matching order: more modifiers first (egui's
    /// matching ignores extra Shift), then higher priority first.
    order: Vec<usize>,
}

#[derive(Deserialize)]
struct UserFile {
    #[serde(default)]
    bindings: Vec<UserBinding>,
}

#[derive(Deserialize)]
struct UserBinding {
    key: Option<String>,
    command: String,
    when: Option<String>,
}

impl Keymap {
    pub fn defaults() -> Self {
        let bindings = DEFAULTS
            .iter()
            .map(|(k, c, e)| Binding { keys: parse_keys(k).expect("valid default key"), command: *c, editor_only: *e })
            .collect();
        Self::from_bindings(bindings)
    }

    fn from_bindings(bindings: Vec<Binding>) -> Self {
        let mut order: Vec<usize> = (0..bindings.len()).collect();
        let weight = |b: &Binding| {
            let m = b.keys[0].modifiers;
            usize::from(m.command || m.ctrl) + usize::from(m.alt) + usize::from(m.shift)
        };
        order.sort_by(|&a, &b| weight(&bindings[b]).cmp(&weight(&bindings[a])).then(b.cmp(&a)));
        Self { bindings, order }
    }

    /// Defaults plus the user's keybindings file. Problems are returned, not fatal.
    pub fn load(path: Option<&Path>) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut bindings = Self::defaults().bindings;
        let Some(path) = path.filter(|p| p.exists()) else { return (Self::from_bindings(bindings), problems) };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                problems.push(format!("{}: {e}", path.display()));
                return (Self::from_bindings(bindings), problems);
            }
        };
        let file: UserFile = match toml::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                problems.push(format!("{}: {}", path.display(), e.message()));
                return (Self::from_bindings(bindings), problems);
            }
        };
        for b in file.bindings {
            let keys = match b.key.as_deref().map(parse_keys).transpose() {
                Ok(k) => k,
                Err(e) => {
                    problems.push(format!("keybindings: {e}"));
                    continue;
                }
            };
            if let Some(id) = b.command.strip_prefix('-') {
                let Some(cmd) = command_from_id(id) else {
                    problems.push(format!("keybindings: unknown command `{id}`"));
                    continue;
                };
                bindings.retain(|x| !(x.command == cmd && keys.as_ref().is_none_or(|k| *k == x.keys)));
                continue;
            }
            let Some(cmd) = command_from_id(&b.command) else {
                problems.push(format!("keybindings: unknown command `{}`", b.command));
                continue;
            };
            let Some(keys) = keys else {
                problems.push(format!("keybindings: `{}` needs a `key`", b.command));
                continue;
            };
            let editor_only = match b.when.as_deref() {
                None | Some("") => false,
                Some("editorTextFocus" | "editorFocus") => true,
                Some(other) => {
                    problems.push(format!("keybindings: unsupported `when` clause `{other}` (use editorTextFocus)"));
                    true
                }
            };
            bindings.push(Binding { keys, command: cmd, editor_only });
        }
        (Self::from_bindings(bindings), problems)
    }

    /// Shortcut shown in menus and the palette (the highest-priority binding).
    pub fn shortcut_text(&self, cmd: Command) -> String {
        self.bindings.iter().rev().find(|b| b.command == cmd).map(|b| format_keys(&b.keys)).unwrap_or_default()
    }

    pub fn is_editor_only(&self, cmd: Command) -> bool {
        self.bindings.iter().rev().find(|b| b.command == cmd).is_some_and(|b| b.editor_only)
    }

    /// Commands these exact keys run.
    pub fn commands_using(&self, keys: &[KeyboardShortcut]) -> Vec<Command> {
        self.bindings.iter().filter(|b| b.keys == keys).map(|b| b.command).collect()
    }

    /// Consumes a matching shortcut from this frame's input. `chord` holds the
    /// first stroke of a two-stroke binding between frames.
    pub fn process(
        &self,
        ctx: &egui::Context,
        chord: &mut Option<KeyboardShortcut>,
        editor_focused: bool,
        terminal_focused: bool,
    ) -> Option<Command> {
        let applies =
            |b: &Binding| (!b.editor_only || editor_focused) && (!terminal_focused || runs_in_terminal(b.command));
        if let Some(first) = *chord {
            // Wait across idle frames; resolve on the next real key press.
            // Modifiers arrive as keys too, and most people press Ctrl again.
            let key_pressed = ctx.input(|i| {
                i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, .. } if !is_modifier(*key)))
            });
            if !key_pressed {
                return None;
            }
            *chord = None;
            for &i in &self.order {
                let b = &self.bindings[i];
                if b.keys.len() == 2
                    && b.keys[0] == first
                    && applies(b)
                    && ctx.input_mut(|inp| inp.consume_shortcut(&b.keys[1]))
                {
                    // The second stroke may also have produced text (e.g. "z"); drop it.
                    ctx.input_mut(|inp| inp.events.retain(|e| !matches!(e, egui::Event::Text(_))));
                    return Some(b.command);
                }
            }
            return None;
        }
        for &i in &self.order {
            let b = &self.bindings[i];
            // Alt held isn't that shortcut: Ctrl+Alt is how some keyboards type @ and {.
            let alt_differs = !b.keys[0].modifiers.alt && ctx.input(|i| i.modifiers.alt);
            if applies(b) && !alt_differs && ctx.input_mut(|inp| inp.consume_shortcut(&b.keys[0])) {
                if b.keys.len() == 1 {
                    // Alt combinations can also produce text (Shift+Alt+F types "F" on
                    // Windows); it must not reach the editor.
                    let m = b.keys[0].modifiers;
                    if m.alt || m.command || m.ctrl {
                        ctx.input_mut(|inp| inp.events.retain(|e| !matches!(e, egui::Event::Text(_))));
                    }
                    return Some(b.command);
                }
                *chord = Some(b.keys[0]);
                return None;
            }
        }
        None
    }

    /// File contents for a new keybindings file, listing the defaults as comments.
    pub fn default_file_contents(&self) -> String {
        let mut out = String::from(
            "# Rust Code keybindings (VS Code command IDs). Save to apply.\n\
             # Add a binding:\n\
             #   [[bindings]]\n\
             #   key = \"ctrl+shift+d\"\n\
             #   command = \"editor.action.duplicateSelection\"\n\
             #   when = \"editorTextFocus\"   # optional\n\
             # Remove a default: command = \"-editor.action.addSelectionToNextFindMatch\"\n\
             #\n\
             # Defaults:\n",
        );
        for b in &self.bindings {
            let when = if b.editor_only { "  (editorTextFocus)" } else { "" };
            out.push_str(&format!("#   {:<18} {}{when}\n", format_keys(&b.keys), command_id(b.command)));
        }
        out
    }
}

pub fn is_modifier(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::ControlLeft
            | K::ControlRight
            | K::AltLeft
            | K::AltRight
            | K::ShiftLeft
            | K::ShiftRight
            | K::SuperLeft
            | K::SuperRight
    )
}

/// A change to one command's keys in the keybindings file.
pub enum BindingChange {
    /// Replaces its keys.
    Set { keys: String, editor_only: bool },
    /// Leaves it with no keys.
    Remove,
    /// Back to its default keys.
    Reset,
}

/// The keybindings file with `change` made for command `id`. Comments at the
/// top stay; bindings are rewritten, so comments among them don't.
pub fn set_binding_in_text(text: &str, id: &str, change: &BindingChange) -> Result<String, String> {
    use toml::{Table, Value};
    let mut file: Table = toml::from_str(text).map_err(|e| format!("keybindings: {}", e.message()))?;
    let mut bindings = match file.remove("bindings") {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let removal = format!("-{id}");
    bindings.retain(|b| b.get("command").and_then(Value::as_str).is_none_or(|c| c != id && c != removal));
    let binding = |pairs: &[(&str, &str)]| {
        Value::Table(pairs.iter().map(|(k, v)| ((*k).to_owned(), Value::String((*v).to_owned()))).collect())
    };
    match change {
        BindingChange::Reset => {}
        BindingChange::Remove => bindings.push(binding(&[("command", &removal)])),
        BindingChange::Set { keys, editor_only } => {
            bindings.push(binding(&[("command", &removal)]));
            let mut pairs = vec![("key", keys.as_str()), ("command", id)];
            if *editor_only {
                pairs.push(("when", "editorTextFocus"));
            }
            bindings.push(binding(&pairs));
        }
    }
    if !bindings.is_empty() {
        file.insert("bindings".to_owned(), Value::Array(bindings));
    }
    let body = toml::to_string(&file).map_err(|e| format!("keybindings: {e}"))?;
    let comments: String =
        text.lines().take_while(|l| l.starts_with('#') || l.is_empty()).map(|l| format!("{l}\n")).collect();
    Ok(format!("{comments}{body}"))
}

/// Parses "ctrl+shift+k" or a chord "ctrl+k ctrl+o".
pub fn parse_keys(s: &str) -> Result<Vec<KeyboardShortcut>, String> {
    let strokes: Vec<KeyboardShortcut> = s.split_whitespace().map(parse_stroke).collect::<Result<_, _>>()?;
    if strokes.is_empty() || strokes.len() > 2 {
        return Err(format!("`{s}`: expected one key or a two-key chord"));
    }
    Ok(strokes)
}

fn parse_stroke(s: &str) -> Result<KeyboardShortcut, String> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    // Split on '+' but allow "+" itself as the key ("ctrl++").
    let lower = s.to_lowercase();
    let mut parts: Vec<&str> = lower.split('+').collect();
    if lower.ends_with("++") {
        parts.truncate(parts.len() - 2);
        parts.push("plus");
    }
    for part in parts {
        match part {
            "ctrl" | "cmd" | "meta" => mods |= Modifiers::COMMAND,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" => mods |= Modifiers::ALT,
            name => {
                if key.is_some() {
                    return Err(format!("`{s}`: more than one key"));
                }
                key = Some(key_from_name(name).ok_or_else(|| format!("`{s}`: unknown key `{name}`"))?);
            }
        }
    }
    let key = key.ok_or_else(|| format!("`{s}`: missing key"))?;
    Ok(KeyboardShortcut::new(mods, key))
}

fn key_from_name(name: &str) -> Option<Key> {
    Some(match name {
        "`" | "backtick" => Key::Backtick,
        "/" | "slash" => Key::Slash,
        "\\" | "backslash" => Key::Backslash,
        "[" => Key::OpenBracket,
        "]" => Key::CloseBracket,
        "=" | "equal" => Key::Equals,
        "-" | "minus" => Key::Minus,
        "plus" => Key::Plus,
        ";" => Key::Semicolon,
        "'" => Key::Quote,
        "," => Key::Comma,
        "." => Key::Period,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "home" => Key::Home,
        "end" => Key::End,
        "tab" => Key::Tab,
        "enter" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "space" => Key::Space,
        other => {
            // Letters, digits and F-keys use egui's own names ("A", "1", "F5").
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Key::from_name(&c.to_ascii_uppercase().to_string())?,
                (Some(c), None) if c.is_ascii_digit() => Key::from_name(&c.to_string())?,
                _ if other.starts_with('f') => Key::from_name(&other.to_uppercase())?,
                _ => return None,
            }
        }
    })
}

/// "Ctrl+Shift+K", or "Ctrl+K Ctrl+O" for chords.
pub fn format_keys(keys: &[KeyboardShortcut]) -> String {
    keys.iter().map(format_stroke).collect::<Vec<_>>().join(" ")
}

fn format_stroke(k: &KeyboardShortcut) -> String {
    let mut s = String::new();
    if k.modifiers.command || k.modifiers.ctrl {
        s.push_str("Ctrl+");
    }
    if k.modifiers.shift {
        s.push_str("Shift+");
    }
    if k.modifiers.alt {
        s.push_str("Alt+");
    }
    let name = match k.logical_key {
        Key::Backtick => "`".to_owned(),
        Key::Slash => "/".to_owned(),
        Key::Backslash => "\\".to_owned(),
        Key::OpenBracket => "[".to_owned(),
        Key::CloseBracket => "]".to_owned(),
        Key::Equals => "=".to_owned(),
        Key::Minus => "-".to_owned(),
        Key::Plus => "+".to_owned(),
        Key::Comma => ",".to_owned(),
        Key::ArrowUp => "Up".to_owned(),
        Key::ArrowDown => "Down".to_owned(),
        Key::ArrowLeft => "Left".to_owned(),
        Key::ArrowRight => "Right".to_owned(),
        other => other.name().to_owned(),
    };
    s.push_str(&name);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_the_keybindings_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("rust-code-keys-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keybindings.toml");
        let id = command_id(Command::DeleteLines);
        let load = |text: &str| {
            std::fs::write(&path, text).unwrap();
            let (km, problems) = Keymap::load(Some(&path));
            assert_eq!(problems, Vec::<String>::new());
            km
        };

        let header = Keymap::defaults().default_file_contents();
        let set = BindingChange::Set { keys: "Ctrl+K Ctrl+Y".to_owned(), editor_only: true };
        let text = set_binding_in_text(&header, id, &set).unwrap();
        assert!(text.starts_with("# Rust Code keybindings"));
        let km = load(&text);
        assert_eq!(km.shortcut_text(Command::DeleteLines), "Ctrl+K Ctrl+Y");
        assert!(km.is_editor_only(Command::DeleteLines));
        assert_eq!(km.commands_using(&parse_keys("ctrl+shift+k").unwrap()), Vec::new());
        assert_eq!(km.commands_using(&parse_keys("ctrl+k ctrl+y").unwrap()), [Command::DeleteLines]);

        // Changing again replaces, and other commands' bindings stay.
        let other = BindingChange::Set { keys: "F7".to_owned(), editor_only: false };
        let text = set_binding_in_text(&text, command_id(Command::JoinLines), &other).unwrap();
        let again = BindingChange::Set { keys: "F6".to_owned(), editor_only: true };
        let text = set_binding_in_text(&text, id, &again).unwrap();
        let km = load(&text);
        assert_eq!(km.shortcut_text(Command::DeleteLines), "F6");
        assert_eq!(km.shortcut_text(Command::JoinLines), "F7");

        let text = set_binding_in_text(&text, id, &BindingChange::Remove).unwrap();
        assert_eq!(load(&text).shortcut_text(Command::DeleteLines), "");
        let text = set_binding_in_text(&text, id, &BindingChange::Reset).unwrap();
        assert_eq!(load(&text).shortcut_text(Command::DeleteLines), "Ctrl+Shift+K");

        // A broken file is reported, not overwritten.
        assert!(set_binding_in_text("[[[", id, &BindingChange::Reset).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_chord_survives_pressing_ctrl_again() {
        let km = Keymap::defaults();
        let ctx = egui::Context::default();
        let mut chord = None;
        let mut press = |key, modifiers| {
            let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            let max_texture_side = Some(4096);
            ctx.begin_pass(egui::RawInput { events: vec![event], max_texture_side, ..Default::default() });
            let command = km.process(&ctx, &mut chord, true, false);
            ctx.end_pass().textures_delta.clear();
            command
        };
        assert_eq!(press(Key::K, Modifiers::COMMAND), None);
        assert_eq!(press(Key::V, Modifiers::NONE), Some(Command::MarkdownPreview));
        assert_eq!(press(Key::K, Modifiers::COMMAND), None);
        assert_eq!(press(Key::ControlLeft, Modifiers::COMMAND), None);
        assert_eq!(press(Key::R, Modifiers::COMMAND), Some(Command::GitRevertHunk));
        // Any other key still ends the chord.
        assert_eq!(press(Key::K, Modifiers::COMMAND), None);
        assert_eq!(press(Key::A, Modifiers::NONE), None);
        assert_eq!(press(Key::R, Modifiers::COMMAND), Some(Command::OpenRecent));
    }

    #[test]
    fn every_command_has_an_id_and_round_trips() {
        for (cmd, id) in crate::commands::COMMAND_IDS {
            assert_eq!(command_from_id(id), Some(*cmd));
        }
        for (cmd, _) in crate::commands::COMMANDS {
            assert!(!command_id(*cmd).is_empty(), "{cmd:?} has no VS Code id");
        }
    }

    #[test]
    fn parses_and_formats_keys() {
        assert_eq!(format_keys(&parse_keys("ctrl+shift+k").unwrap()), "Ctrl+Shift+K");
        assert_eq!(format_keys(&parse_keys("ctrl+k ctrl+o").unwrap()), "Ctrl+K Ctrl+O");
        assert_eq!(format_keys(&parse_keys("shift+alt+down").unwrap()), "Shift+Alt+Down");
        assert_eq!(format_keys(&parse_keys("ctrl+`").unwrap()), "Ctrl+`");
        assert_eq!(format_keys(&parse_keys("ctrl+0").unwrap()), "Ctrl+0");
        assert_eq!(format_keys(&parse_keys("F5").unwrap()), "F5");
        assert_eq!(format_keys(&parse_keys("ctrl++").unwrap()), "Ctrl++");
        assert!(parse_keys("ctrl+nope").is_err());
        assert!(parse_keys("a b c").is_err());
    }

    #[test]
    fn defaults_are_valid_and_shown() {
        let km = Keymap::defaults();
        assert_eq!(km.shortcut_text(Command::DeleteLines), "Ctrl+Shift+K");
        assert_eq!(km.shortcut_text(Command::OpenFolder), "Ctrl+K Ctrl+O");
    }

    #[test]
    fn more_modifiers_are_matched_first() {
        let km = Keymap::defaults();
        let first_ctrl_k =
            km.order.iter().position(|&i| km.bindings[i].keys[0] == parse_keys("ctrl+k").unwrap()[0]).unwrap();
        let ctrl_shift_k = km.order.iter().position(|&i| km.bindings[i].command == Command::DeleteLines).unwrap();
        assert!(ctrl_shift_k < first_ctrl_k);
    }

    #[test]
    fn user_file_adds_overrides_and_removes() {
        let dir = std::env::temp_dir().join(format!("rust-code-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keybindings.toml");
        std::fs::write(
            &path,
            r#"
            [[bindings]]
            key = "ctrl+shift+d"
            command = "editor.action.duplicateSelection"
            when = "editorTextFocus"

            [[bindings]]
            key = "ctrl+alt+k"
            command = "editor.action.deleteLines"

            [[bindings]]
            command = "-editor.action.addSelectionToNextFindMatch"

            [[bindings]]
            key = "ctrl+q"
            command = "no.such.command"
            "#,
        )
        .unwrap();
        let (km, problems) = Keymap::load(Some(&path));
        assert_eq!(km.shortcut_text(Command::DuplicateSelection), "Ctrl+Shift+D");
        assert_eq!(km.shortcut_text(Command::DeleteLines), "Ctrl+Alt+K", "user binding wins for display");
        assert_eq!(km.shortcut_text(Command::SelectNextOccurrence), "", "removed");
        assert_eq!(problems, vec!["keybindings: unknown command `no.such.command`".to_owned()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
