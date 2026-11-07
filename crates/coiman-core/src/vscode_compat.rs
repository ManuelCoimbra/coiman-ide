//! Which VS Code extensions can be used here, and why the others can't. An extension's
//! code needs VS Code's extension host (Node.js and the `vscode` API), which Coiman IDE
//! doesn't have: its data (themes, snippets, languages) comes across, what it does may be
//! built in already, and the VS Code APIs its code calls say what's missing. Nothing in
//! an extension is run: its `package.json` is read, and its code is only searched for the
//! names of the API calls it makes.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::vscode_import;

/// What an extension is here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Only data, which comes across with Extensions: Import from VS Code.
    Imports,
    /// What it does is built in here (named).
    BuiltIn(String),
    /// It only installs other extensions.
    Pack(Vec<String>),
    /// Data this can't use (an icon theme, keybindings, grammars alone), and no code.
    UnusedData(String),
    /// Code that needs VS Code: it can't run. What's similar here, if anything.
    NeedsVsCode(Option<String>),
}

/// One extension, as found.
#[derive(Clone, Debug)]
pub struct Report {
    pub id: String,
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    pub verdict: Verdict,
    /// What of its data comes across (`2 themes, 40 snippets`), if any.
    pub data: Option<String>,
    /// The VS Code APIs its code uses, by family.
    pub apis: Vec<&'static str>,
}

impl Report {
    /// A line for a status message or a list: what it is here, and why.
    pub fn summary(&self) -> String {
        match &self.verdict {
            Verdict::Imports => {
                format!("{}: works, its {} can be imported", self.name, self.data.as_deref().unwrap_or("data"))
            }
            Verdict::BuiltIn(what) => format!("{}: built in here: {what}", self.name),
            Verdict::Pack(of) => format!("{}: a pack of {} extensions, each listed on its own", self.name, of.len()),
            Verdict::UnusedData(what) => format!("{}: holds {what}, which isn't used here", self.name),
            Verdict::NeedsVsCode(instead) => {
                let mut s = format!("{} can't run here: it's code for VS Code's extension host", self.name);
                if !self.apis.is_empty() {
                    s.push_str(&format!(" (it uses {})", self.apis.join(", ")));
                }
                if let Some(data) = &self.data {
                    s.push_str(&format!(". Its {data} can be imported"));
                }
                if let Some(instead) = instead {
                    s.push_str(&format!(". Here: {instead}"));
                }
                s
            }
        }
    }
}

/// VS Code API calls, as they appear in an extension's code (names survive bundling), and
/// the family each belongs to.
const API_MARKERS: &[(&str, &str)] = &[
    ("vscode-languageclient", "a language server client"),
    ("LanguageClient(", "a language server client"),
    ("createWebviewPanel", "webviews"),
    ("registerWebviewViewProvider", "webviews"),
    ("registerCustomEditorProvider", "custom editors"),
    ("registerTreeDataProvider", "tree views"),
    ("createTreeView", "tree views"),
    ("createTextEditorDecorationType", "editor decorations"),
    ("registerCompletionItemProvider", "completion"),
    ("registerHoverProvider", "hovers"),
    ("registerCodeActionsProvider", "code actions"),
    ("registerCodeLensProvider", "code lenses"),
    ("registerDocumentFormattingEditProvider", "formatting"),
    ("registerDefinitionProvider", "go to definition"),
    ("createDiagnosticCollection", "diagnostics"),
    ("registerDebugAdapterDescriptorFactory", "debug adapters"),
    ("registerDebugConfigurationProvider", "debug adapters"),
    ("createTestController", "the testing API"),
    ("createSourceControl", "source control"),
    ("registerTaskProvider", "tasks"),
    ("createTerminal", "terminals"),
    ("registerFileSystemProvider", "file systems"),
    ("getSession(", "sign-in"),
    ("createStatusBarItem", "status bar items"),
    ("onDidChangeTextDocument", "edit events"),
    ("registerCommand", "commands"),
    ("registerTextEditorCommand", "commands"),
];

/// Extensions whose work is built in, by `publisher.name`, and what does it here.
const BUILT_IN: &[(&str, &str)] = &[
    ("editorconfig.editorconfig", "EditorConfig: .editorconfig files are read and applied"),
    ("usernamehw.errorlens", "the message after the line, Error Lens style"),
    ("shardulm94.trailing-spaces", "the Trailing Spaces plugin (Extensions view)"),
    ("wayou.vscode-todo-highlight", "the Comment Highlights plugin (TODO, FIXME...)"),
    ("aaron-bond.better-comments", "the Comment Highlights plugin (TODO, !, ?, *)"),
    ("pkief.material-icon-theme", "file icons, from Material Icon Theme"),
    ("vscode-icons-team.vscode-icons", "file icons for every file type"),
    ("emmanuelbeziat.vscode-great-icons", "file icons for every file type"),
    ("shalldie.background", "background pictures (Background: Choose Image...)"),
    ("naumovs.color-highlight", "color squares beside colors (editor.color_decorators)"),
    ("bierner.color-info", "color squares, Color: Pick Color at Cursor"),
    ("vincaslt.highlight-matching-tag", "matching tags are marked"),
    ("formulahendry.code-runner", "Run (Ctrl+Shift+F10) for the current file"),
    ("ritwickdey.liveserver", "Live Preview (Run on an HTML file)"),
    ("eamodio.gitlens", "inline blame and the Source Control view; no commit graph"),
    ("vscjava.vscode-maven", "the MAVEN section of the Explorer and Maven: commands"),
    ("redhat.java", "the Eclipse JDT Language Server, run directly"),
    ("oracle.oracle-java", "the Eclipse JDT Language Server, run directly"),
    ("ms-dotnettools.csharp", "the C# language server (csharp-ls or OmniSharp), run directly"),
    ("vscjava.vscode-java-test", "the Testing view, with Maven and Gradle"),
    ("vscjava.vscode-java-dependency", "the project tree and the MAVEN dependency tree"),
    ("vscjava.vscode-gradle", "Run and tests through Gradle; no task list"),
    ("ms-dotnettools.vscode-dotnet-runtime", "nothing needed: language servers find .NET themselves"),
    ("formulahendry.auto-close-tag", "tags are closed as you type"),
    ("ms-vscode.cpptools", "clangd for C and C++, gdb and CodeLLDB for debugging"),
    ("llvm-vs-code-extensions.vscode-clangd", "clangd, run directly"),
    ("rust-lang.rust-analyzer", "rust-analyzer, run directly"),
    ("ms-python.python", "Python: pyright, debugpy and the project's environment"),
    ("ms-python.vscode-pylance", "pyright, run directly"),
    ("ms-python.debugpy", "debugpy, run directly"),
    ("golang.go", "gopls and Delve, run directly"),
    ("vadimcn.vscode-lldb", "CodeLLDB, run directly"),
    ("dbaeumer.vscode-eslint", "its ESLint server runs here, from this extension: problems and fixes"),
    ("bradlc.vscode-tailwindcss", "its Tailwind server runs here, from this extension: class names, problems, fixes"),
    ("christian-kohler.path-intellisense", "paths complete in strings (editor.path_completion)"),
    ("ionutvmi.path-autocomplete", "paths complete in strings (editor.path_completion)"),
    ("formulahendry.auto-rename-tag", "tags rename in pairs (editor.rename_tags)"),
];

/// What's like it here, for code that can't run, when there's something.
const SIMILAR: &[(&str, &str)] = &[
    ("vscjava.vscode-java-debug", "no Java debugger yet; Run works"),
    (
        "esbenp.prettier-vscode",
        "format on save leaves Prettier projects to Prettier; run it from a terminal or run configuration",
    ),
    ("ms-dotnettools.csdevkit", "the C# language server and Run; no Solution Explorer"),
    ("mhutchie.git-graph", "the Source Control view and Git: Show History; no graph"),
];

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&crate::theme::strip_jsonc(text.trim_start_matches('\u{feff}'))).ok()
}

/// The VS Code API families in an extension's code (its `main`, and other `.js` files
/// beside it, up to a limit).
fn apis_in(dir: &Path, package: &Value) -> Vec<&'static str> {
    let main = package.get("main").or_else(|| package.get("browser")).and_then(Value::as_str);
    let Some(main) = main else { return Vec::new() };
    let mut file = dir.join(main.trim_start_matches("./"));
    if file.extension().is_none() {
        file.set_extension("js");
    }
    // The main file, then its folder's other scripts: bundles split into chunks.
    let mut files = vec![file.clone()];
    if let Some(folder) = file.parent()
        && let Ok(entries) = std::fs::read_dir(folder)
    {
        files.extend(
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p != &file && p.extension().is_some_and(|x| x == "js"))
                .take(40),
        );
    }
    let mut found: Vec<&'static str> = Vec::new();
    let mut read = 0usize;
    for f in files {
        let Ok(bytes) = std::fs::read(&f) else { continue };
        read += bytes.len();
        let text = String::from_utf8_lossy(&bytes);
        for (marker, family) in API_MARKERS {
            if !found.contains(family) && text.contains(marker) {
                found.push(family);
            }
        }
        if read > 24 * 1024 * 1024 {
            break;
        }
    }
    // In the order of the list: the most telling first.
    let mut ordered: Vec<&'static str> = Vec::new();
    for (_, family) in API_MARKERS {
        if found.contains(family) && !ordered.contains(family) {
            ordered.push(family);
        }
    }
    ordered
}

/// What data an extension has that isn't used here, if any.
fn unused_data(package: &Value) -> Option<String> {
    let contributes = package.get("contributes")?;
    let kinds: Vec<&str> = [
        ("iconThemes", "an icon theme"),
        ("productIconThemes", "a product icon theme"),
        ("keybindings", "keybindings"),
        ("grammars", "TextMate grammars"),
        ("jsonValidation", "JSON schemas"),
    ]
    .into_iter()
    .filter(|(key, _)| contributes.get(*key).is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty())))
    .map(|(_, what)| what)
    .collect();
    (!kinds.is_empty()).then(|| kinds.join(", "))
}

/// Looks at one unpacked extension.
pub fn examine(dir: &Path) -> Result<Report, String> {
    let package = read_json(&dir.join("package.json")).ok_or("This isn't a VS Code extension: no package.json")?;
    let contents = vscode_import::read(dir)?;
    let publisher = package.get("publisher").and_then(Value::as_str).unwrap_or("");
    let short = package.get("name").and_then(Value::as_str).unwrap_or("");
    let id = format!("{publisher}.{short}").to_lowercase();
    let version = package.get("version").and_then(Value::as_str).unwrap_or("").to_owned();
    let has_code = contents.has_code;
    let data = contents.is_useful().then(|| contents.describe());
    let pack: Vec<String> = package
        .get("extensionPack")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    let known = |table: &[(&str, &str)]| table.iter().find(|(k, _)| *k == id).map(|(_, v)| (*v).to_owned());
    // A pack's own code is a welcome page at most: what it installs is what counts.
    let verdict = if let Some(what) = known(BUILT_IN) {
        Verdict::BuiltIn(what)
    } else if has_code && let Some(similar) = known(SIMILAR) {
        // Code of its own that matters (C# Dev Kit), even when it also installs others.
        Verdict::NeedsVsCode(Some(similar))
    } else if !pack.is_empty() {
        Verdict::Pack(pack)
    } else if has_code {
        Verdict::NeedsVsCode(known(SIMILAR))
    } else if data.is_some() {
        Verdict::Imports
    } else {
        Verdict::UnusedData(unused_data(&package).unwrap_or_else(|| "nothing".to_owned()))
    };
    // Only what can't run needs its reasons read out of its code.
    let apis = if matches!(verdict, Verdict::NeedsVsCode(_)) { apis_in(dir, &package) } else { Vec::new() };
    Ok(Report { id, name: contents.name, version, dir: dir.to_path_buf(), verdict, data, apis })
}

/// VS Code's installed extensions, examined, sorted by name.
pub fn installed() -> Vec<Report> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    let Some(entries) = home.and_then(|h| std::fs::read_dir(h.join(".vscode").join("extensions")).ok()) else {
        return Vec::new();
    };
    let mut out: Vec<Report> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|d| examine(&d).ok())
        .collect();
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// The report as Markdown: counts, then a table for each kind.
pub fn markdown(reports: &[Report]) -> String {
    let count = |f: &dyn Fn(&Verdict) -> bool| reports.iter().filter(|r| f(&r.verdict)).count();
    let imports = count(&|v| matches!(v, Verdict::Imports));
    let built = count(&|v| matches!(v, Verdict::BuiltIn(_)));
    let packs = count(&|v| matches!(v, Verdict::Pack(_)));
    let unused = count(&|v| matches!(v, Verdict::UnusedData(_)));
    let code = count(&|v| matches!(v, Verdict::NeedsVsCode(_)));
    let mut s = String::new();
    s.push_str("# VS Code extensions here\n\n");
    s.push_str(&format!(
        "{} extensions found in VS Code. Coiman IDE has no extension host: an extension's code (JavaScript for Node.js and the `vscode` API) doesn't run here. What it holds as data comes across, and much of what popular extensions do is built in.\n\n",
        reports.len()
    ));
    s.push_str(&format!(
        "- **Built in here:** {built}\n- **Data that imports** (themes, snippets, languages): {imports}\n- **Packs** of other extensions: {packs}\n- **Data not used here** (icon themes, keybindings, grammars alone): {unused}\n- **Code that can't run:** {code}\n\n"
    ));
    s.push_str("Import data with **Extensions: Import from VS Code**. Nothing in an extension was run to make this report: its `package.json` was read and its code searched for the VS Code API calls it makes.\n");
    let section = |s: &mut String, title: &str, header: &str, rows: Vec<String>| {
        if rows.is_empty() {
            return;
        }
        s.push_str(&format!("\n## {title}\n\n{header}\n"));
        for row in rows {
            s.push_str(&row);
            s.push('\n');
        }
    };
    let row = |r: &Report, rest: &str| format!("| {} | {} | {} |", cell(&r.name), cell(&r.version), cell(rest));
    section(
        &mut s,
        "Built in here",
        "| Extension | Version | Here |\n| --- | --- | --- |",
        reports
            .iter()
            .filter_map(|r| match &r.verdict {
                Verdict::BuiltIn(what) => Some(row(r, what)),
                _ => None,
            })
            .collect(),
    );
    section(
        &mut s,
        "Data that imports",
        "| Extension | Version | What comes across |\n| --- | --- | --- |",
        reports
            .iter()
            .filter(|r| r.verdict == Verdict::Imports)
            .map(|r| row(r, r.data.as_deref().unwrap_or("")))
            .collect(),
    );
    section(
        &mut s,
        "Code that can't run",
        "| Extension | Version | VS Code APIs it uses | Here instead |\n| --- | --- | --- | --- |",
        reports
            .iter()
            .filter_map(|r| match &r.verdict {
                Verdict::NeedsVsCode(instead) => {
                    let mut here = instead.clone().unwrap_or_else(|| "nothing like it yet".to_owned());
                    if let Some(data) = &r.data {
                        here.push_str(&format!("; its {data} can be imported"));
                    }
                    let apis = if r.apis.is_empty() { "not found in its code".to_owned() } else { r.apis.join(", ") };
                    Some(format!("| {} | {} | {} | {} |", cell(&r.name), cell(&r.version), cell(&apis), cell(&here)))
                }
                _ => None,
            })
            .collect(),
    );
    section(
        &mut s,
        "Packs",
        "| Extension | Version | Installs |\n| --- | --- | --- |",
        reports
            .iter()
            .filter_map(|r| match &r.verdict {
                Verdict::Pack(of) => Some(row(r, &of.join(", "))),
                _ => None,
            })
            .collect(),
    );
    section(
        &mut s,
        "Data not used here",
        "| Extension | Version | Holds |\n| --- | --- | --- |",
        reports
            .iter()
            .filter_map(|r| match &r.verdict {
                Verdict::UnusedData(what) => Some(row(r, what)),
                _ => None,
            })
            .collect(),
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension(root: &Path, folder: &str, package: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), package).unwrap();
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn each_kind_of_extension() {
        let root = std::env::temp_dir().join(format!("rc-compat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let theme = extension(
            &root,
            "a.theme-1.0.0",
            r#"{"publisher":"a","name":"theme","displayName":"Night","version":"1.0.0","contributes":{"themes":[{"label":"Night","uiTheme":"vs-dark","path":"./t.json"}]}}"#,
            &[("t.json", r##"{"name":"Night","type":"dark","colors":{"editor.background":"#000000"}}"##)],
        );
        let code = extension(
            &root,
            "b.lint-2.0.0",
            r#"{"publisher":"b","name":"lint","version":"2.0.0","main":"./dist/ext","contributes":{"snippets":[{"language":"javascript","path":"./s.json"}]}}"#,
            &[
                (
                    "dist/ext.js",
                    "const v=require('vscode');v.window.createWebviewPanel('x');v.commands.registerCommand('a',()=>{});",
                ),
                ("dist/chunk.js", "new LanguageClient('id', opts)"),
                ("s.json", r#"{"log":{"prefix":"log","body":"console.log($1)"}}"#),
            ],
        );
        let builtin = extension(
            &root,
            "editorconfig.editorconfig-0.18.2",
            r#"{"publisher":"EditorConfig","name":"EditorConfig","version":"0.18.2","main":"./out/x.js"}"#,
            &[],
        );
        let pack = extension(
            &root,
            "c.pack-1.0.0",
            r#"{"publisher":"c","name":"pack","version":"1.0.0","main":"./w.js","extensionPack":["x.one","y.two"]}"#,
            &[],
        );
        let icons = extension(
            &root,
            "d.icons-1.0.0",
            r#"{"publisher":"d","name":"icons","version":"1.0.0","contributes":{"iconThemes":[{"id":"i","path":"./i.json"}]}}"#,
            &[],
        );
        let known = extension(
            &root,
            "mhutchie.git-graph-1.30.0",
            r#"{"publisher":"mhutchie","name":"git-graph","version":"1.30.0","main":"./out/e.js"}"#,
            &[("out/e.js", "x.createWebviewPanel()")],
        );

        let r = examine(&theme).unwrap();
        assert_eq!(r.verdict, Verdict::Imports);
        assert_eq!(r.data.as_deref(), Some("1 theme"));
        let r = examine(&code).unwrap();
        assert_eq!(r.verdict, Verdict::NeedsVsCode(None));
        assert_eq!(r.apis, ["a language server client", "webviews", "commands"]);
        assert!(
            r.summary().contains("can't run here") && r.summary().contains("Its 1 snippet can be imported"),
            "{}",
            r.summary()
        );
        let r = examine(&builtin).unwrap();
        assert!(matches!(r.verdict, Verdict::BuiltIn(ref w) if w.starts_with("EditorConfig")));
        assert_eq!(r.id, "editorconfig.editorconfig");
        assert_eq!(examine(&pack).unwrap().verdict, Verdict::Pack(vec!["x.one".into(), "y.two".into()]));
        assert_eq!(examine(&icons).unwrap().verdict, Verdict::UnusedData("an icon theme".into()));
        let r = examine(&known).unwrap();
        assert!(matches!(r.verdict, Verdict::NeedsVsCode(Some(ref s)) if s.contains("no graph")));
        assert!(examine(&root.join("missing")).is_err());

        let all: Vec<Report> = [theme, code, builtin, pack, icons, known].iter().map(|d| examine(d).unwrap()).collect();
        let md = markdown(&all);
        assert!(md.contains("6 extensions found"));
        assert!(md.contains("- **Code that can't run:** 2"));
        assert!(md.contains("| lint | 2.0.0 | a language server client, webviews, commands | nothing like it yet; its 1 snippet can be imported |"), "{md}");
        assert!(md.contains("| Night | 1.0.0 | 1 theme |"));
        assert!(md.contains("## Packs") && md.contains("x.one, y.two"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
