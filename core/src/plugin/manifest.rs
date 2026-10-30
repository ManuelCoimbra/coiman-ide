//! `plugin.toml`: who a plugin is, what it may do (permissions) and what it
//! adds to the editor (contribution points). The editor draws every
//! contribution itself; a plugin only fills them with data.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A capability the user grants a plugin. Host calls that need one fail
/// with `permission denied: <name>` without it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    WorkspaceRead,
    WorkspaceWrite,
    EditorRead,
    EditorWrite,
}

impl Permission {
    pub const ALL: [Permission; 4] =
        [Permission::WorkspaceRead, Permission::WorkspaceWrite, Permission::EditorRead, Permission::EditorWrite];

    pub fn name(self) -> &'static str {
        match self {
            Permission::WorkspaceRead => "workspace-read",
            Permission::WorkspaceWrite => "workspace-write",
            Permission::EditorRead => "editor-read",
            Permission::EditorWrite => "editor-write",
        }
    }

    /// What granting it allows, for the consent prompt.
    pub fn describe(self) -> &'static str {
        match self {
            Permission::WorkspaceRead => "Read files in the open folder",
            Permission::WorkspaceWrite => "Create and change files in the open folder",
            Permission::EditorRead => "Read the text of the active editor, including unsaved changes",
            Permission::EditorWrite => "Type text into the active editor",
        }
    }
}

/// Editor events a plugin can subscribe to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Opened,
    Saved,
    ActiveChanged,
    Edited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Alignment {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandDecl {
    pub id: String,
    /// Shown in the Command Palette, e.g. "Word Count: Show Report".
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusItemDecl {
    pub id: String,
    #[serde(default)]
    pub alignment: Alignment,
    /// Command run when the item is clicked.
    #[serde(default)]
    pub command: Option<String>,
}

/// A bottom panel tab or a tree view section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDecl {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// `<publisher>.<name>`, lowercase letters, digits and `-`.
    pub id: String,
    pub name: String,
    /// `major.minor.patch`.
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    #[serde(default)]
    pub events: BTreeSet<EventKind>,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    #[serde(default)]
    pub status_items: Vec<StatusItemDecl>,
    #[serde(default)]
    pub panels: Vec<ViewDecl>,
    #[serde(default)]
    pub tree_views: Vec<ViewDecl>,
}

/// Parses and checks a `plugin.toml`.
pub fn parse(text: &str) -> Result<Manifest, String> {
    let m: Manifest = toml::from_str(text).map_err(|e| format!("plugin.toml: {}", e.message()))?;
    m.validate()?;
    Ok(m)
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_local_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Manifest {
    pub fn publisher(&self) -> &str {
        self.id.split('.').next().unwrap_or("")
    }

    pub fn validate(&self) -> Result<(), String> {
        match self.id.split_once('.') {
            Some((publisher, name)) if valid_name(publisher) && valid_name(name) => {}
            _ => return Err(format!("plugin id \"{}\" must look like publisher.name (a-z, 0-9, -)", self.id)),
        }
        if self.name.trim().is_empty() {
            return Err("plugin.toml: name is empty".to_owned());
        }
        if parse_version(&self.version).is_none() {
            return Err(format!("version \"{}\" must look like 1.2.3", self.version));
        }
        let groups: [(&str, Vec<&str>); 4] = [
            ("command", self.commands.iter().map(|c| c.id.as_str()).collect()),
            ("status item", self.status_items.iter().map(|c| c.id.as_str()).collect()),
            ("panel", self.panels.iter().map(|c| c.id.as_str()).collect()),
            ("tree view", self.tree_views.iter().map(|c| c.id.as_str()).collect()),
        ];
        for (kind, ids) in &groups {
            let mut seen = BTreeSet::new();
            for id in ids {
                if !valid_local_id(id) {
                    return Err(format!("{kind} id \"{id}\" is not valid (letters, digits, - _ .)"));
                }
                if !seen.insert(*id) {
                    return Err(format!("{kind} id \"{id}\" is declared twice"));
                }
            }
        }
        for item in &self.status_items {
            if let Some(c) = &item.command
                && !self.commands.iter().any(|d| &d.id == c)
            {
                return Err(format!("status item \"{}\" runs undeclared command \"{c}\"", item.id));
            }
        }
        Ok(())
    }
}

/// `"1.2.3"` → `(1, 2, 3)`.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether `candidate` is a later version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// What changes between two versions of a plugin, shown before updating.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestDiff {
    pub added_permissions: Vec<Permission>,
    pub removed_permissions: Vec<Permission>,
    pub added_events: Vec<EventKind>,
    /// Contributions as "Command: Title", "Panel: Title"…
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl ManifestDiff {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn contributions(m: &Manifest) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    out.extend(m.commands.iter().map(|c| format!("Command: {}", c.title)));
    out.extend(m.status_items.iter().map(|c| format!("Status item: {}", c.id)));
    out.extend(m.panels.iter().map(|c| format!("Panel: {}", c.title)));
    out.extend(m.tree_views.iter().map(|c| format!("Tree view: {}", c.title)));
    out
}

pub fn diff(old: &Manifest, new: &Manifest) -> ManifestDiff {
    let (a, b) = (contributions(old), contributions(new));
    ManifestDiff {
        added_permissions: new.permissions.difference(&old.permissions).copied().collect(),
        removed_permissions: old.permissions.difference(&new.permissions).copied().collect(),
        added_events: new.events.difference(&old.events).copied().collect(),
        added: b.difference(&a).cloned().collect(),
        removed: a.difference(&b).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
id = "example.word-count"
name = "Word Count"
version = "1.2.0"
permissions = ["editor-read"]
events = ["saved", "active-changed"]

[[commands]]
id = "report"
title = "Word Count: Show Report"

[[status_items]]
id = "count"
alignment = "right"
command = "report"

[[panels]]
id = "report"
title = "Words"
"#;

    #[test]
    fn parses_validates_and_diffs() {
        let m = parse(SAMPLE).unwrap();
        assert_eq!(m.publisher(), "example");
        assert!(m.permissions.contains(&Permission::EditorRead));
        assert_eq!(m.status_items[0].alignment, Alignment::Right);

        let bad = SAMPLE.replace("command = \"report\"", "command = \"missing\"");
        assert!(parse(&bad).unwrap_err().contains("undeclared command"));
        assert!(parse(&SAMPLE.replace("example.word-count", "WordCount")).is_err());
        assert!(parse(&SAMPLE.replace("1.2.0", "1.2")).is_err());

        let mut next = m.clone();
        next.version = "1.3.0".into();
        next.permissions.insert(Permission::WorkspaceRead);
        next.tree_views.push(ViewDecl { id: "todo".into(), title: "TODOs".into() });
        next.panels.clear();
        let d = diff(&m, &next);
        assert_eq!(d.added_permissions, vec![Permission::WorkspaceRead]);
        assert_eq!(d.added, vec!["Tree view: TODOs".to_owned()]);
        assert_eq!(d.removed, vec!["Panel: Words".to_owned()]);
        assert!(is_newer("1.10.0", "1.9.3") && !is_newer("1.2.0", "1.2.0"));
    }
}
