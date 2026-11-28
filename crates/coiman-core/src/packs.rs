//! Packs: what goes beyond editing comes in packs by kind of work, installed from the
//! Extensions view. A pack that isn't installed leaves no trace: its commands, windows,
//! panels, code lenses and background work are gone, and its extensions aren't installed.
//!
//! `[packs] installed` in the settings lists the installed ones (a project's own settings can
//! name its own); `"all"` is every pack, which is what a settings file from before packs gets.

/// A pack of features for one kind of work.
pub struct Pack {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// What it brings, for the Extensions view.
    pub features: &'static [&'static str],
    /// Extensions (plugin ids) that come with Coiman and belong to it: installed with it.
    pub extensions: &'static [&'static str],
}

pub const WEB: &str = "web";
pub const DATA: &str = "data";
pub const DEVOPS: &str = "devops";
pub const SYSTEMS: &str = "systems";
pub const MOBILE: &str = "mobile";
pub const GRAPHICS: &str = "graphics";
pub const ANALYSIS: &str = "analysis";
pub const GIT: &str = "git";
pub const AGENTS: &str = "agents";
pub const TERMINAL: &str = "terminal";

/// Every pack, in the order the Extensions view lists them.
pub const PACKS: [Pack; 10] = [
    Pack {
        id: WEB,
        name: "Web and APIs",
        description: "Send HTTP requests and read OpenAPI specs.",
        features: &["HTTP client (.http files, environments, history)", "OpenAPI and Swagger endpoints and models"],
        extensions: &["coiman.http-client", "coiman.openapi"],
    },
    Pack {
        id: DATA,
        name: "Databases",
        description: "Query and browse databases from the editor.",
        features: &[
            "SQLite, PostgreSQL, MySQL and SQL Server: tables, queries, plans, ER diagrams",
            "Redis keys and values, and a console; MongoDB collections and documents",
            "Jupyter notebooks",
        ],
        extensions: &["coiman.sql", "coiman.redis", "coiman.mongodb"],
    },
    Pack {
        id: DEVOPS,
        name: "Containers, remote and CI",
        description: "Docker, GitHub Actions, and working on other machines over SSH.",
        features: &[
            "Docker: containers, images, logs, a shell inside",
            "GitHub Actions: runs, failed logs, re-runs",
            "Remote folders over SSH, SSH terminals, remote commands",
            "Remote debugging through gdbserver",
        ],
        extensions: &["coiman.docker", "coiman.github-actions", "coiman.remote", "coiman.remote-debug"],
    },
    Pack {
        id: SYSTEMS,
        name: "Embedded and systems",
        description: "Boards, binaries and the machine underneath.",
        features: &[
            "Serial monitor (text, hex, plot)",
            "Binary size by crate, section and symbol",
            "Memory and disassembly views while debugging",
        ],
        extensions: &["coiman.serial-monitor", "coiman.binary-size"],
    },
    Pack {
        id: MOBILE,
        name: "Mobile",
        description: "Android devices over adb.",
        features: &["Android: devices, logcat, a shell, installing an APK"],
        extensions: &["coiman.android"],
    },
    Pack {
        id: GRAPHICS,
        name: "Graphics",
        description: "Shaders, checked and previewed.",
        features: &["GLSL and WGSL problems while typing", "Live shader preview"],
        extensions: &["coiman.shaders"],
    },
    Pack {
        id: ANALYSIS,
        name: "Code analysis",
        description: "See how the code is built and how it behaves.",
        features: &[
            "Class diagrams from the code (UML)",
            "Architecture: dependency graph, cycles, impact; a function's flow",
            "Code health, project overview, session statistics",
            "Profiler, coverage, run history, stack trace analysis",
            "Rules, and search by syntax tree",
            "Project docs: an index, the links between documents, outdated documents",
            "Project metadata: name, version, description, licence, edited in place",
        ],
        extensions: &[
            "coiman.class-diagram",
            "coiman.architecture",
            "coiman.code-health",
            "coiman.docs",
            "coiman.metadata",
        ],
    },
    Pack {
        id: GIT,
        name: "Git, more of it",
        description: "Beyond commits and branches.",
        features: &[
            "Git Graph: rebase, cherry-pick, compare, worktrees",
            "History search, submodules, hooks, .gitignore rules",
            "Project snapshots: keep the folder, compare, restore",
        ],
        extensions: &["coiman.git-graph", "coiman.git-tools", "coiman.snapshots"],
    },
    Pack {
        id: AGENTS,
        name: "Agents",
        description: "AI agents through opencode, and MCP servers.",
        features: &["Agents panel (opencode)", "MCP servers"],
        extensions: &["coiman.agents"],
    },
    Pack {
        id: TERMINAL,
        name: "Terminal extras",
        description: "More from the terminal.",
        features: &[
            "Command history and Ctrl+Space suggestions: history, project commands, paths",
            "Routines: the project's lists of commands, run one after the other",
        ],
        extensions: &["coiman.terminal-commands"],
    },
];

pub fn pack(id: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|p| p.id == id)
}

/// The pack an extension that comes with Coiman belongs to.
pub fn pack_of_extension(plugin: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|p| p.extensions.contains(&plugin))
}

/// Whether `installed` (the setting) has pack `id`.
pub fn is_on(installed: &[String], id: &str) -> bool {
    installed.iter().any(|p| p == id || p == "all")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_are_listed_once_and_found() {
        let mut ids: Vec<&str> = PACKS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PACKS.len());
        assert_eq!(pack(DATA).map(|p| p.name), Some("Databases"));
        assert_eq!(pack_of_extension("coiman.class-diagram").map(|p| p.id), Some(ANALYSIS));
        let none: Vec<String> = Vec::new();
        assert!(!is_on(&none, WEB));
        assert!(is_on(&["all".to_owned()], WEB));
        assert!(is_on(&["web".to_owned(), "git".to_owned()], GIT) && !is_on(&["web".to_owned()], GIT));
    }

    #[test]
    fn every_pack_brings_something() {
        use crate::editor::commands::COMMANDS;
        for p in &PACKS {
            let commands = COMMANDS.iter().filter(|(c, _)| c.pack() == Some(p.id)).count();
            assert!(commands > 0 || !p.extensions.is_empty(), "{} brings nothing", p.id);
        }
        // Every pack a command names exists.
        for (c, _) in COMMANDS {
            if let Some(id) = c.pack() {
                assert!(pack(id).is_some(), "{c:?} names {id}");
            }
        }
    }
}
