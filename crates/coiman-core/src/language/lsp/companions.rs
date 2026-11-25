//! Language servers that come inside VS Code extensions, run beside a file's own server.
//! VS Code's extension code can't run here, but most servers are plain Node.js programs
//! speaking LSP. They are started directly from the installed extension, given the
//! settings the extension would give them, and their problems, fixes and completions
//! join the main server's. Only trusted folders may start one.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// One server that an extension carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Companion {
    /// The extension, as `publisher.name`.
    pub extension: String,
    pub name: String,
    /// The server's path inside the extension's folder.
    pub script: String,
    /// LSP language ids it reads.
    pub languages: Vec<String>,
    /// Project files that say this tool is set up.
    pub configs: Vec<String>,
    /// Words in `package.json` that say the same.
    pub package_words: Vec<String>,
    /// An exact script found while scanning an installed extension.
    pub script_path: Option<PathBuf>,
    /// No project marker: the open folder is enough (YAML).
    pub always_root: bool,
}

fn known(extension: &str, name: &str, script: &str, languages: &[&str], configs: &[&str], words: &[&str]) -> Companion {
    Companion {
        extension: extension.to_owned(),
        name: name.to_owned(),
        script: script.to_owned(),
        languages: languages.iter().map(|s| (*s).to_owned()).collect(),
        configs: configs.iter().map(|s| (*s).to_owned()).collect(),
        package_words: words.iter().map(|s| (*s).to_owned()).collect(),
        script_path: None,
        always_root: false,
    }
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
const STYLELINT_CONFIGS: &[&str] = &[
    "stylelint.config.js",
    "stylelint.config.mjs",
    "stylelint.config.cjs",
    ".stylelintrc",
    ".stylelintrc.json",
    ".stylelintrc.yaml",
    ".stylelintrc.yml",
];
const YAML_CONFIGS: &[&str] = &[".yamllint", ".yamllint.yaml", ".yamllint.yml", ".yamlfmt"];

/// Servers whose entrypoints were checked against their published VSIX files.
pub fn known_companions() -> Vec<Companion> {
    let mut out = vec![
        known(
            "dbaeumer.vscode-eslint",
            "ESLint",
            "server/out/eslintServer.js",
            &["javascript", "javascriptreact", "typescript", "typescriptreact", "astro"],
            ESLINT_CONFIGS,
            &["\"eslintConfig\"", "\"eslint\""],
        ),
        known(
            "bradlc.vscode-tailwindcss",
            "Tailwind CSS",
            "dist/tailwindServer.js",
            &[
                "html",
                "css",
                "scss",
                "less",
                "postcss",
                "javascript",
                "javascriptreact",
                "typescript",
                "typescriptreact",
                "astro",
                "php",
                "markdown",
                "mdx",
            ],
            TAILWIND_CONFIGS,
            &["\"tailwindcss\"", "\"@tailwindcss/"],
        ),
        known(
            "stylelint.vscode-stylelint",
            "Stylelint",
            "dist/start-server.js",
            &["css", "scss", "less", "postcss"],
            STYLELINT_CONFIGS,
            &["\"stylelint\""],
        ),
    ];
    out.push(known(
        "ms-python.vscode-pylance",
        "Pylance",
        "dist/server.bundle.js",
        &["python"],
        &["pyrightconfig.json", "mypy.ini"],
        &["\"pylance\"", "\"pyright\""],
    ));
    let mut yaml = known(
        "redhat.vscode-yaml",
        "YAML Language Server",
        "dist/languageserver.js",
        &["yaml"],
        YAML_CONFIGS,
        &["\"yaml\""],
    );
    yaml.always_root = true;
    out.push(yaml);
    out
}

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
        "astro" => "astro",
        "php" => "php",
        "md" | "markdown" => "markdown",
        "mdx" => "mdx",
        "yaml" | "yml" => "yaml",
        "rs" => "rust",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "sh" | "bash" => "shell",
        "sql" => "sql",
        "json" | "jsonc" => "json",
        _ => return None,
    })
}

fn package_mentions(dir: &Path, words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    std::fs::read_to_string(dir.join("package.json")).is_ok_and(|text| words.iter().any(|word| text.contains(word)))
}

fn extension_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        out.push(home.join(".vscode").join("extensions"));
        out.push(home.join(".vscode-insiders").join("extensions"));
        out.push(home.join(".vscode-server").join("extensions"));
    }
    if let Some(dir) = crate::extensions::vscode_import::coiman_extensions_dir() {
        out.push(dir);
    }
    out
}

impl Companion {
    /// Whether a folder is a project this server is for: it has the tool set up.
    pub fn set_up_in(&self, dir: &Path) -> bool {
        self.configs.iter().any(|c| dir.join(c).is_file()) || package_mentions(dir, &self.package_words)
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
        if self.always_root {
            return top.map(Path::to_path_buf);
        }
        None
    }

    /// The newest copy of the server, in VS Code's extensions or in the ones
    /// Coiman downloaded from Open VSX.
    pub fn installed_script(&self) -> Option<PathBuf> {
        if let Some(exact) = &self.script_path
            && exact.is_file()
        {
            return Some(exact.clone());
        }
        extension_dirs().into_iter().find_map(|dir| self.script_in(&dir))
    }

    /// The newest copy of the server among the extensions in `extensions`.
    pub fn script_in(&self, extensions: &Path) -> Option<PathBuf> {
        let prefix = format!("{}-", self.extension.to_lowercase());
        let mut found: Vec<(Vec<u32>, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(extensions).into_iter().flatten().filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let Some(version) = name.strip_prefix(&prefix) else { continue };
            let numbers = version.split(['.', '-']).map_while(|p| p.parse().ok()).collect();
            let script = entry.path().join(&self.script);
            if script.is_file() {
                found.push((numbers, script));
            }
        }
        found.sort();
        found.pop().map(|(_, script)| script)
    }

    /// The answer to `workspace/configuration` for one item, as VS Code's extension would
    /// give it.
    pub fn setting(&self, section: Option<&str>, root: &Path, root_uri: &str) -> Value {
        if self.extension == "dbaeumer.vscode-eslint" {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return json!({
                "validate": "on", "packageManager": "npm", "useESLintClass": false, "useFlatConfig": null,
                "experimental": {},
                "codeAction": {
                    "disableRuleComment": { "enable": true, "location": "separateLine", "commentStyle": "line" },
                    "showDocumentation": { "enable": true }
                },
                "codeActionOnSave": { "enable": false, "mode": "all" },
                "format": false, "quiet": false, "onIgnoredFiles": "off", "options": {},
                "rulesCustomizations": [], "run": "onType", "problems": { "shortenToSingleLine": false },
                "nodePath": null, "workingDirectory": { "mode": "location" },
                "workspaceFolder": { "uri": root_uri, "name": name },
                "timeBudget": { "onValidation": { "warn": 4000, "error": 8000 }, "onFixes": { "warn": 3000, "error": 6000 } }
            });
        }
        if self.extension == "redhat.vscode-yaml" {
            return match section {
                Some("yaml") | None => json!({
                    "validate": true, "completion": true, "hover": true, "schemas": Value::Null,
                    "schemaStore": { "enable": true, "url": "https://json.schemastore.org/schema-catalog.json", "useFirstMatch": false },
                    "format": { "enable": true, "singleQuote": false, "printWidth": 80, "tabWidth": 2, "proseWrap": "preserve" }
                }),
                Some("editor") => json!({ "tabSize": 2 }),
                _ => Value::Null,
            };
        }
        if self.extension == "stylelint.vscode-stylelint" {
            return match section {
                Some("stylelint") | None => json!({ "enable": true, "validate": true, "lint": {} }),
                Some("editor") => json!({ "tabSize": 2 }),
                _ => Value::Null,
            };
        }
        match section {
            Some("editor") => json!({ "tabSize": 2 }),
            Some("tailwindCSS") | None => json!({
                "emmetCompletions": false, "includeLanguages": {},
                "classAttributes": ["class", "className", "ngClass", "class:list"], "classFunctions": [],
                "codeActions": true, "hovers": true, "suggestions": true, "validate": true,
                "colorDecorators": true, "rootFontSize": 16, "showPixelEquivalents": true,
                "files": { "exclude": ["**/.git/**", "**/node_modules/**", "**/.hg/**", "**/.svn/**"] },
                "experimental": { "classRegex": [], "configFile": null },
                "lint": {
                    "cssConflict": "warning", "invalidApply": "error", "invalidScreen": "error",
                    "invalidVariant": "error", "invalidConfigPath": "error", "invalidTailwindDirective": "error",
                    "invalidSourceDirective": "error", "recommendedVariantOrder": "warning",
                    "usedBlocklistedClass": "warning", "suggestCanonicalClasses": "warning"
                }
            }),
            _ => Value::Null,
        }
    }

    /// The answer to a request only this server sends, if it's one of those.
    pub fn own_request(&self, method: &str) -> Option<Value> {
        match method {
            "eslint/confirmESLintExecution" if self.extension == "dbaeumer.vscode-eslint" => Some(json!(4)),
            "eslint/openDoc" | "eslint/probeFailed" | "eslint/noLibrary" | "eslint/noConfig"
                if self.extension == "dbaeumer.vscode-eslint" =>
            {
                Some(Value::Null)
            }
            "@/tailwindCSS/getConfiguration" if self.extension == "bradlc.vscode-tailwindcss" => {
                Some(self.setting(Some("tailwindCSS"), Path::new(""), ""))
            }
            _ => None,
        }
    }

    /// What a message from this server says the user should know, if anything.
    pub fn notice(&self, method: &str, _params: &Value) -> Option<String> {
        match method {
            "eslint/noLibrary" if self.extension == "dbaeumer.vscode-eslint" => {
                Some("ESLint isn't installed in this project (npm install eslint)".to_owned())
            }
            "eslint/noConfig" if self.extension == "dbaeumer.vscode-eslint" => {
                Some("ESLint found no configuration for this file".to_owned())
            }
            _ => None,
        }
    }
}

/// A VS Code language id narrowed to one Coiman uses.
fn our_language(id: &str) -> Option<String> {
    let id = id.to_lowercase();
    let ours = match id.as_str() {
        "javascript" | "javascriptreact" | "typescript" | "typescriptreact" | "html" | "css" | "scss" | "less"
        | "postcss" | "astro" | "php" | "markdown" | "mdx" | "yaml" | "rust" | "python" | "go" | "java" | "c"
        | "cpp" | "csharp" | "ruby" | "shellscript" | "shell" | "sql" | "json" | "jsonc" => id,
        "dockercompose" | "ansible" | "github-actions-workflow" => "yaml".to_owned(),
        _ => return None,
    };
    Some(ours)
}

fn server_candidates(dir: &Path) -> Vec<(u8, String, PathBuf)> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    // A few folders deep is enough: servers live in dist/, server/out/, out/, bundled/.
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((folder, depth)) = stack.pop() {
        if depth > 4 || seen > 600 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for entry in entries.filter_map(Result::ok) {
            seen += 1;
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if !matches!(name.as_str(), "node_modules" | "test" | "tests" | ".git") {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !kind.is_file() || path.extension().is_none_or(|e| e != "js" && e != "cjs") {
                continue;
            }
            let base = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            if ["extension", "main", "copilot", "bundle", "web", "test", "mock"].iter().any(|w| base.contains(w)) {
                continue;
            }
            let priority = if matches!(
                base.as_str(),
                "language-server.js"
                    | "languageserver.js"
                    | "langserver.js"
                    | "server.js"
                    | "start-server.js"
                    | "eslintserver.js"
                    | "tailwindserver.js"
            ) || base.contains("languageserver")
                || base.contains("language-server")
            {
                0
            } else if base.contains("server") {
                1
            } else {
                continue;
            };
            let relative = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            out.push((priority, relative, path));
        }
    }
    out.sort_by_key(|(priority, relative, _)| (*priority, relative.clone()));
    out.dedup_by(|a, b| a.1 == b.1);
    out
}

/// Servers found in installed extensions' manifests and folder layouts.
pub fn discovered_companions() -> Vec<Companion> {
    let mut seen = std::collections::HashSet::new();
    extension_dirs()
        .into_iter()
        .flat_map(|dir| discover_in(&dir))
        .filter(|companion| seen.insert((companion.extension.clone(), companion.script.clone())))
        .collect()
}

/// Servers found in one extensions folder.
pub fn discover_in(extensions: &Path) -> Vec<Companion> {
    let mut out = Vec::new();
    {
        let dir = extensions;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.filter_map(Result::ok) {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let Some(package) = std::fs::read_to_string(folder.join("package.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&crate::appearance::theme::strip_jsonc(&text)).ok())
            else {
                continue;
            };
            let id = format!(
                "{}.{}",
                package.get("publisher").and_then(Value::as_str).unwrap_or(""),
                package.get("name").and_then(Value::as_str).unwrap_or("")
            );
            if !id.contains('.')
                || id.starts_with('.')
                || id.to_lowercase().contains("vue")
                || id.to_lowercase().contains("svelte")
            {
                continue;
            }
            let mut languages: Vec<String> = Vec::new();
            for contribution in
                package.pointer("/contributes/languages").and_then(Value::as_array).into_iter().flatten()
            {
                let id = contribution.get("id").and_then(Value::as_str).unwrap_or_default();
                if let Some(ours) = our_language(id) {
                    languages.push(ours);
                }
            }
            for event in package.get("activationEvents").and_then(Value::as_array).into_iter().flatten() {
                let event = event.as_str().unwrap_or_default();
                if let Some(id) = event.strip_prefix("onLanguage:")
                    && let Some(ours) = our_language(id)
                {
                    languages.push(ours);
                }
            }
            languages.sort();
            languages.dedup();
            if languages.is_empty() {
                continue;
            }
            let Some((_, script, path)) = server_candidates(&folder).into_iter().next() else { continue };
            let name = package
                .get("displayName")
                .or_else(|| package.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_owned();
            out.push(Companion {
                extension: id,
                name,
                script,
                languages,
                configs: Vec::new(),
                package_words: Vec::new(),
                script_path: Some(path),
                always_root: false,
            });
        }
    }
    out
}

/// Known servers plus ones found in installed extensions, deduplicated.
pub fn all() -> Vec<Companion> {
    let mut out = known_companions();
    let known: Vec<String> = out.iter().map(|c| c.extension.to_lowercase()).collect();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for mut found in discovered_companions() {
        let id = found.extension.to_lowercase();
        if known.contains(&id) || !seen.insert((id, found.script.clone())) {
            continue;
        }
        if let Some(script) = found.script_path.clone() {
            // Prefer the exact script we found over the guessed relative path.
            found.script = script.to_string_lossy().replace('\\', "/");
        }
        out.push(found);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_discovered_from_an_installed_extension() {
        let dir = std::env::temp_dir().join(format!("rc-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let extension = dir.join("example.python-tools-1.2.3");
        let dist = extension.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(
            extension.join("package.json"),
            r#"{"publisher":"example","name":"python-tools","displayName":"Python Tools","version":"1.2.3",
                "contributes":{"languages":[{"id":"python","extensions":[".py"]}]}}"#,
        )
        .unwrap();
        std::fs::write(dist.join("language-server.js"), "// server").unwrap();
        let found = discover_in(&dir);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].extension, "example.python-tools");
        assert_eq!(found[0].languages, vec!["python"]);
        assert!(found[0].script.ends_with("language-server.js"));
        assert!(found[0].script_path.as_ref().is_some_and(|p| p.is_file()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn projects_and_servers_are_found() {
        let dir = std::env::temp_dir().join(format!("rc-companions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let web = dir.join("web");
        std::fs::create_dir_all(web.join("src")).unwrap();
        std::fs::write(web.join("eslint.config.js"), "export default [];").unwrap();
        std::fs::write(web.join("package.json"), r#"{"devDependencies":{"tailwindcss":"^4"}}"#).unwrap();
        let file = web.join("src").join("app.js");
        let eslint = &known_companions()[0];
        let tailwind = &known_companions()[1];
        assert_eq!(eslint.root_for(&file, Some(&dir)), Some(web.clone()));
        assert_eq!(tailwind.root_for(&file, Some(&dir)), Some(web.clone()));
        assert_eq!(eslint.root_for(&dir.join("other.js"), Some(&dir)), None);
        assert_eq!(eslint.setting(None, &web, "file:///w")["useFlatConfig"], Value::Null);
        assert_eq!(eslint.setting(None, &web, "file:///w")["workspaceFolder"]["name"], json!("web"));
        assert_eq!(tailwind.setting(Some("editor"), &web, "")["tabSize"], json!(2));
        assert_eq!(eslint.own_request("eslint/confirmESLintExecution"), Some(json!(4)));
        assert!(eslint.notice("eslint/noLibrary", &Value::Null).unwrap().contains("npm install eslint"));

        let ext = dir.join("extensions");
        let server = ext.join("dbaeumer.vscode-eslint-3.0.34").join("server/out");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("eslintServer.js"), "").unwrap();
        let mut spec = eslint.clone();
        spec.script_path = None;
        let found = spec.script_in(&ext).expect("the newest server is found");
        assert!(found.to_string_lossy().contains("3.0.34"), "{}", found.display());

        assert_eq!(language_of("tsx"), Some("typescriptreact"));

        let yaml = known_companions().pop().unwrap();
        assert!(yaml.always_root);
        assert_eq!(yaml.root_for(&dir.join("config.yml"), Some(&dir)), Some(dir.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
