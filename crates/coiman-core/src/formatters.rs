//! Which formatters a web project has set up, and where they would fight.

use std::path::Path;

const PRETTIER_FILES: &[&str] = &[
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yaml",
    ".prettierrc.yml",
    ".prettierrc.js",
    ".prettierrc.cjs",
    ".prettierrc.mjs",
    ".prettierrc.toml",
    "prettier.config.js",
    "prettier.config.cjs",
    "prettier.config.mjs",
];
const BIOME_FILES: &[&str] = &["biome.json", "biome.jsonc"];
const DPRINT_FILES: &[&str] = &["dprint.json", ".dprint.json"];
const ESLINT_FILES: &[&str] = &[
    "eslint.config.js",
    "eslint.config.mjs",
    "eslint.config.cjs",
    "eslint.config.ts",
    ".eslintrc",
    ".eslintrc.json",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.yaml",
    ".eslintrc.yml",
];

/// What's set up in a project's root.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WebTools {
    /// The formatters, by name.
    pub formatters: Vec<&'static str>,
    pub eslint: bool,
    /// ESLint is told to leave formatting to Prettier.
    pub eslint_defers: bool,
}

impl WebTools {
    pub fn of(root: &Path) -> Self {
        let any = |names: &[&str]| names.iter().any(|n| root.join(n).is_file());
        let package: serde_json::Value = std::fs::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let depends =
            |name: &str| ["dependencies", "devDependencies"].iter().any(|table| package[table].get(name).is_some());
        let mut formatters = Vec::new();
        if any(PRETTIER_FILES) || depends("prettier") || package.get("prettier").is_some() {
            formatters.push("Prettier");
        }
        if any(BIOME_FILES) || depends("@biomejs/biome") {
            formatters.push("Biome");
        }
        if any(DPRINT_FILES) || depends("dprint") {
            formatters.push("dprint");
        }
        Self {
            formatters,
            eslint: any(ESLINT_FILES) || depends("eslint") || package.get("eslintConfig").is_some(),
            eslint_defers: depends("eslint-config-prettier") || depends("eslint-plugin-prettier"),
        }
    }

    /// What will go wrong, as sentences.
    pub fn conflicts(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.formatters.len() > 1 {
            out.push(format!(
                "{} are both set up. Each will undo what the other wrote: keep one.",
                self.formatters.join(" and ")
            ));
        }
        if self.eslint && self.formatters.contains(&"Prettier") && !self.eslint_defers {
            out.push(
                "ESLint and Prettier are both set up without eslint-config-prettier, so their style rules can disagree."
                    .to_owned(),
            );
        }
        out
    }

    /// The tool that owns formatting here, if it isn't the language server.
    pub fn owner(&self) -> Option<&'static str> {
        self.formatters.first().copied()
    }
}

/// File types those formatters take care of.
pub fn is_web_file(extension: &str) -> bool {
    matches!(
        extension,
        "js" | "mjs"
            | "cjs"
            | "jsx"
            | "ts"
            | "mts"
            | "cts"
            | "tsx"
            | "css"
            | "scss"
            | "less"
            | "html"
            | "htm"
            | "json"
            | "jsonc"
            | "vue"
            | "svelte"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rc-formatters-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        dir
    }

    #[test]
    fn a_plain_project_has_nothing_to_say() {
        let dir = project(&[("package.json", "{\"dependencies\": {\"react\": \"1\"}}")]);
        let tools = WebTools::of(&dir);
        assert_eq!(tools.owner(), None);
        assert!(tools.conflicts().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn two_formatters_conflict() {
        let dir = project(&[("biome.json", "{}"), (".prettierrc", "{}")]);
        let tools = WebTools::of(&dir);
        assert_eq!(tools.formatters, ["Prettier", "Biome"]);
        assert!(tools.conflicts()[0].contains("Prettier and Biome"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn eslint_with_prettier_needs_the_truce() {
        let dir = project(&[("package.json", "{\"devDependencies\": {\"prettier\": \"3\", \"eslint\": \"9\"}}")]);
        let tools = WebTools::of(&dir);
        assert_eq!(tools.owner(), Some("Prettier"));
        assert!(tools.conflicts()[0].contains("eslint-config-prettier"));
        std::fs::remove_dir_all(&dir).unwrap();
        let dir = project(&[(
            "package.json",
            "{\"devDependencies\": {\"prettier\": \"3\", \"eslint\": \"9\", \"eslint-config-prettier\": \"9\"}}",
        )]);
        assert!(WebTools::of(&dir).conflicts().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_broken_package_file_is_no_trouble() {
        let dir = project(&[("package.json", "{ not json")]);
        assert_eq!(WebTools::of(&dir), WebTools::default());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
