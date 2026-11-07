//! Language servers that come inside VS Code extensions, run beside a file's own server:
//! ESLint's and Tailwind CSS's. VS Code's extension code can't run here, but these
//! servers are plain Node.js programs speaking LSP, so they're started directly from the
//! installed extension, given the settings the extension would give them, and their
//! problems, fixes and completions shown with the main server's.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// One server that an extension carries.
#[derive(Debug, PartialEq, Eq)]
pub struct Companion {
    /// The extension, as `publisher.name`.
    pub extension: &'static str,
    pub name: &'static str,
    /// The server inside the extension's folder.
    pub script: &'static str,
    /// LSP language ids it reads.
    pub languages: &'static [&'static str],
}

pub const ESLINT: Companion = Companion {
    extension: "dbaeumer.vscode-eslint",
    name: "ESLint",
    script: "server/out/eslintServer.js",
    languages: &["javascript", "javascriptreact", "typescript", "typescriptreact", "vue", "svelte", "astro"],
};

pub const TAILWIND: Companion = Companion {
    extension: "bradlc.vscode-tailwindcss",
    name: "Tailwind CSS",
    script: "dist/tailwindServer.js",
    languages: &[
        "html",
        "css",
        "scss",
        "less",
        "postcss",
        "javascript",
        "javascriptreact",
        "typescript",
        "typescriptreact",
        "vue",
        "svelte",
        "astro",
        "php",
        "markdown",
        "mdx",
    ],
};

pub const ALL: [&Companion; 2] = [&ESLINT, &TAILWIND];

/// The language id these servers know a file by.
pub fn language_of(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "pcss" | "postcss" => "postcss",
        "vue" => "vue",
        "svelte" => "svelte",
        "astro" => "astro",
        "php" => "php",
        "md" => "markdown",
        "mdx" => "mdx",
        _ => return None,
    })
}

const ESLINT_CONFIGS: &[&str] = &[
    "eslint.config.js",
    "eslint.config.mjs",
    "eslint.config.cjs",
    "eslint.config.ts",
    "eslint.config.mts",
    "eslint.config.cts",
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.json",
    ".eslintrc.yml",
    ".eslintrc.yaml",
];

const TAILWIND_CONFIGS: &[&str] =
    &["tailwind.config.js", "tailwind.config.cjs", "tailwind.config.mjs", "tailwind.config.ts", "tailwind.config.mts"];

fn package_mentions(dir: &Path, words: &[&str]) -> bool {
    std::fs::read_to_string(dir.join("package.json")).is_ok_and(|t| words.iter().any(|w| t.contains(w)))
}

impl Companion {
    /// Whether a folder is a project this server is for: it has the tool set up.
    pub fn set_up_in(&self, dir: &Path) -> bool {
        if *self == ESLINT {
            ESLINT_CONFIGS.iter().any(|c| dir.join(c).is_file()) || package_mentions(dir, &["\"eslintConfig\""])
        } else {
            TAILWIND_CONFIGS.iter().any(|c| dir.join(c).is_file())
                || package_mentions(dir, &["\"tailwindcss\"", "\"@tailwindcss/"])
        }
    }

    /// The project folder for a file: the nearest one above it with the tool set up,
    /// within `top` (the open folder) when there is one.
    pub fn root_for(&self, file: &Path, top: Option<&Path>) -> Option<PathBuf> {
        for dir in file.ancestors().skip(1) {
            if top.is_some_and(|t| !dir.starts_with(t)) {
                break;
            }
            if self.set_up_in(dir) {
                return Some(dir.to_path_buf());
            }
        }
        None
    }

    /// The newest copy of the server in VS Code's installed extensions.
    pub fn installed_script(&self) -> Option<PathBuf> {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)?;
        self.script_in(&home.join(".vscode").join("extensions"))
    }

    /// The newest copy of the server among the extensions in `extensions`.
    pub fn script_in(&self, extensions: &Path) -> Option<PathBuf> {
        let prefix = format!("{}-", self.extension);
        let mut found: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(extensions)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                let version = name.strip_prefix(&prefix)?;
                let numbers = version.split(['.', '-']).map_while(|p| p.parse().ok()).collect();
                let script = e.path().join(self.script);
                script.is_file().then_some((numbers, script))
            })
            .collect();
        found.sort();
        found.pop().map(|(_, s)| s)
    }

    /// The answer to `workspace/configuration` for one item, as VS Code's extension would
    /// give it.
    pub fn setting(&self, section: Option<&str>, root: &Path, root_uri: &str) -> Value {
        if *self == ESLINT {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return json!({
                "validate": "on",
                "packageManager": "npm",
                "useESLintClass": false,
                // Left to the server: it asks the ESLint in the project which kind of
                // configuration it reads (flat from ESLint 9). Forcing it picks an old API.
                "useFlatConfig": null,
                "experimental": {},
                "codeAction": {
                    "disableRuleComment": { "enable": true, "location": "separateLine", "commentStyle": "line" },
                    "showDocumentation": { "enable": true }
                },
                "codeActionOnSave": { "enable": false, "mode": "all" },
                "format": false,
                "quiet": false,
                "onIgnoredFiles": "off",
                "options": {},
                "rulesCustomizations": [],
                "run": "onType",
                "problems": { "shortenToSingleLine": false },
                "nodePath": null,
                "workingDirectory": { "mode": "location" },
                "workspaceFolder": { "uri": root_uri, "name": name },
                "timeBudget": { "onValidation": { "warn": 4000, "error": 8000 }, "onFixes": { "warn": 3000, "error": 6000 } }
            });
        }
        match section {
            Some("editor") => json!({ "tabSize": 2 }),
            Some("tailwindCSS") | None => json!({
                "emmetCompletions": false,
                "includeLanguages": {},
                "classAttributes": ["class", "className", "ngClass", "class:list"],
                "classFunctions": [],
                "codeActions": true,
                "hovers": true,
                "suggestions": true,
                "validate": true,
                "colorDecorators": true,
                "rootFontSize": 16,
                "showPixelEquivalents": true,
                "files": { "exclude": ["**/.git/**", "**/node_modules/**", "**/.hg/**", "**/.svn/**"] },
                "experimental": { "classRegex": [], "configFile": null },
                "lint": {
                    "cssConflict": "warning",
                    "invalidApply": "error",
                    "invalidScreen": "error",
                    "invalidVariant": "error",
                    "invalidConfigPath": "error",
                    "invalidTailwindDirective": "error",
                    "invalidSourceDirective": "error",
                    "recommendedVariantOrder": "warning",
                    "usedBlocklistedClass": "warning",
                    "suggestCanonicalClasses": "warning"
                }
            }),
            _ => Value::Null,
        }
    }

    /// The answer to a request only this server sends, if it's one of those.
    pub fn own_request(&self, method: &str) -> Option<Value> {
        match method {
            // "May ESLint from this project run?": the project is trusted already.
            "eslint/confirmESLintExecution" => Some(json!(4)),
            "eslint/openDoc" | "eslint/probeFailed" | "eslint/noLibrary" | "eslint/noConfig" => Some(Value::Null),
            "@/tailwindCSS/getConfiguration" => Some(self.setting(Some("tailwindCSS"), Path::new(""), "")),
            _ => None,
        }
    }

    /// What a message from this server says the user should know, if anything.
    pub fn notice(&self, method: &str, params: &Value) -> Option<String> {
        match method {
            "eslint/noLibrary" => Some("ESLint isn't installed in this project (npm install eslint)".to_owned()),
            "eslint/noConfig" => Some("ESLint found no configuration for this file".to_owned()),
            "eslint/probeFailed" => Some("ESLint couldn't check this file".to_owned()),
            "eslint/status" if params["state"].as_i64() == Some(3) => Some("ESLint failed: see its output".to_owned()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_and_servers_are_found() {
        let dir = std::env::temp_dir().join(format!("rc-companions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let web = dir.join("web");
        std::fs::create_dir_all(web.join("src")).unwrap();
        std::fs::write(web.join("eslint.config.js"), "export default [];").unwrap();
        std::fs::write(web.join("package.json"), r#"{"devDependencies":{"tailwindcss":"^4"}}"#).unwrap();
        let file = web.join("src").join("app.js");
        assert_eq!(ESLINT.root_for(&file, Some(&dir)), Some(web.clone()));
        assert_eq!(TAILWIND.root_for(&file, Some(&dir)), Some(web.clone()));
        assert_eq!(ESLINT.root_for(&dir.join("other.js"), Some(&dir)), None);
        assert_eq!(ESLINT.setting(None, &web, "file:///w")["useFlatConfig"], Value::Null);
        assert_eq!(ESLINT.setting(None, &web, "file:///w")["workspaceFolder"]["name"], json!("web"));
        assert_eq!(TAILWIND.setting(Some("editor"), &web, "")["tabSize"], json!(2));
        assert_eq!(ESLINT.own_request("eslint/confirmESLintExecution"), Some(json!(4)));
        assert!(ESLINT.notice("eslint/noLibrary", &Value::Null).unwrap().contains("npm install eslint"));
        // The newest installed copy.
        let ext = dir.join("extensions");
        for v in ["3.0.9", "3.0.34", "2.4.4"] {
            let s = ext.join(format!("dbaeumer.vscode-eslint-{v}")).join("server/out");
            std::fs::create_dir_all(&s).unwrap();
            std::fs::write(s.join("eslintServer.js"), "").unwrap();
        }
        let found = ESLINT.script_in(&ext).unwrap();
        assert!(found.to_string_lossy().contains("3.0.34"), "{}", found.display());
        assert_eq!(TAILWIND.script_in(&ext), None);
        assert_eq!(language_of("tsx"), Some("typescriptreact"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
