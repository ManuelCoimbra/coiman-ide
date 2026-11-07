//! What clangd reads for a file: the compilation databases of a project, the
//! flags one holds for a file, and what to do when there are none. Wrong or
//! missing flags only look like clangd being unhelpful, so this says which it is.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

pub const DATABASE: &str = "compile_commands.json";
pub const FLAGS_FILE: &str = "compile_flags.txt";

/// Configurations of multi-config generators, as they appear in output paths.
const CONFIGS: [&str; 4] = ["Debug", "Release", "RelWithDebInfo", "MinSizeRel"];

/// One compile command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub file: PathBuf,
    pub directory: PathBuf,
    pub args: Vec<String>,
    pub config: Option<&'static str>,
    /// The entry as it was written.
    raw: Value,
}

/// A database, read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
}

/// Splits a shell command as the database format defines it: quotes group, a
/// backslash escapes only a quote or another backslash, so Windows paths survive.
pub fn split_command(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', q) if matches!(chars.peek(), Some('"' | '\'' | '\\')) && q != Some('\'') => {
                word.extend(chars.next());
                in_word = true;
            }
            (q @ ('"' | '\''), None) => {
                quote = Some(q);
                in_word = true;
            }
            (q, Some(open)) if q == open => quote = None,
            (w, None) if w.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (other, _) => {
                word.push(other);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(word);
    }
    out
}

/// The configuration an output path names, if any.
fn config_of(output: &str) -> Option<&'static str> {
    output.split(['/', '\\']).find_map(|part| CONFIGS.iter().find(|c| **c == part).copied())
}

/// Removes `.` and `..`, without touching the disk: a database's file may be gone.
fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Whether two paths name one file, as this system compares them.
fn same_file(a: &Path, b: &Path) -> bool {
    let text = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    text(a) == text(b)
}

impl Database {
    /// Reads a database. What it can't make sense of is an error, not an empty list.
    pub fn parse(path: &Path, text: &str) -> Result<Self, String> {
        let name = path.display();
        let value: Value = serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        let items = value.as_array().ok_or_else(|| format!("{name}: expected a list of compile commands"))?;
        let base = path.parent().unwrap_or(Path::new(""));
        let mut entries = Vec::with_capacity(items.len());
        for item in items {
            let directory = base.join(item["directory"].as_str().unwrap_or(""));
            let Some(file) = item["file"].as_str() else { continue };
            let args = match (item["arguments"].as_array(), item["command"].as_str()) {
                (Some(list), _) => list.iter().filter_map(|a| a.as_str().map(str::to_owned)).collect(),
                (None, Some(command)) => split_command(command),
                (None, None) => continue,
            };
            let config = item["output"].as_str().and_then(config_of);
            let file = tidy(&directory.join(file));
            entries.push(Entry { file, directory: tidy(&directory), args, config, raw: item.clone() });
        }
        Ok(Self { path: path.to_path_buf(), entries })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(path, &text)
    }

    /// Configurations this database mixes, in the order they first appear.
    pub fn configs(&self) -> Vec<&'static str> {
        let mut found = Vec::new();
        for config in self.entries.iter().filter_map(|e| e.config) {
            if !found.contains(&config) {
                found.push(config);
            }
        }
        found
    }

    /// The entries for a file, one per configuration at most in usual projects.
    pub fn entries_for(&self, file: &Path) -> Vec<&Entry> {
        self.entries.iter().filter(|e| same_file(&e.file, file)).collect()
    }

    /// Text of a database with only one configuration's entries. Each entry's
    /// directory becomes absolute, because the result lives somewhere else.
    pub fn filtered_text(&self, config: &str) -> String {
        let kept: Vec<Value> = self
            .entries
            .iter()
            .filter(|e| e.config == Some(CONFIGS.iter().find(|c| **c == config).copied().unwrap_or("")))
            .map(|e| {
                let mut raw = e.raw.clone();
                raw["directory"] = Value::String(e.directory.to_string_lossy().into_owned());
                raw
            })
            .collect();
        serde_json::to_string_pretty(&kept).unwrap_or_else(|_| "[]".to_owned())
    }
}

/// What flags say, sorted out for reading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub compiler: Option<String>,
    pub standard: Option<String>,
    pub target: Option<String>,
    pub defines: Vec<String>,
    pub includes: Vec<String>,
    pub other: Vec<String>,
}

/// Sorts a compile command's words. `with_compiler`: the first word is the compiler.
pub fn read_flags(args: &[String], with_compiler: bool) -> Flags {
    let mut flags = Flags::default();
    let mut words = args.iter().peekable();
    if with_compiler {
        flags.compiler = words.next().cloned();
    }
    while let Some(word) = words.next() {
        let w = word.as_str();
        // Both a joined value and the next word.
        let mut value_of = |names: &[&str]| -> Option<String> {
            for name in names {
                if let Some(rest) = w.strip_prefix(name) {
                    return if rest.is_empty() { words.next().cloned() } else { Some(rest.to_owned()) };
                }
            }
            None
        };
        if w == "--" {
            break;
        }
        if w == "-c" || w == "/c" {
            continue;
        }
        if w == "-o" {
            words.next();
            continue;
        }
        if w.starts_with("-o") || w.starts_with("/Fo") {
            continue;
        }
        if let Some(rest) = w.strip_prefix("-std=").or_else(|| w.strip_prefix("/std:")) {
            flags.standard = Some(rest.to_owned());
        } else if let Some(rest) = w.strip_prefix("--target=") {
            flags.target = Some(rest.to_owned());
        } else if w == "-target" {
            flags.target = words.next().cloned();
        } else if w.starts_with("-isystem") || w.starts_with("-iquote") {
            let v = value_of(&["-isystem", "-iquote"]);
            flags.includes.extend(v);
        } else if w.starts_with("-I") || w.starts_with("/I") {
            let v = value_of(&["-I", "/I"]);
            flags.includes.extend(v);
        } else if w.starts_with("-D") || w.starts_with("/D") {
            let v = value_of(&["-D", "/D"]);
            flags.defines.extend(v);
        } else if w.starts_with('-') || w.starts_with('/') && !Path::new(w).extension().is_some_and(is_source_extension)
        {
            flags.other.push(word.clone());
        }
    }
    flags
}

fn is_source_extension(ext: &std::ffi::OsStr) -> bool {
    let ext = ext.to_string_lossy().to_lowercase();
    SOURCES.contains(&ext.as_str())
}

/// A place a database might be, and whether clangd finds it unaided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub dir: PathBuf,
    /// Its folder, relative to the project.
    pub label: String,
}

fn has_database(dir: &Path) -> bool {
    dir.join(DATABASE).is_file()
}

/// Where clangd looks for a file: each folder upward, and its `build` folder.
pub fn clangd_finds(file: &Path) -> Option<PathBuf> {
    for dir in file.ancestors().skip(1) {
        for place in [dir.join(DATABASE), dir.join("build").join(DATABASE), dir.join(FLAGS_FILE)] {
            if place.is_file() {
                return Some(place);
            }
        }
    }
    None
}

/// Databases around a project, wherever build tools usually leave them.
pub fn candidates(root: &Path) -> Vec<Candidate> {
    let mut dirs = vec![root.to_path_buf(), root.join("build")];
    let subfolders = |dir: &Path| -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut found: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        found.sort();
        found
    };
    for folder in subfolders(root) {
        let name = folder.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if name.starts_with("build-") || name.starts_with("cmake-build-") {
            dirs.push(folder);
        }
    }
    // Visual Studio's CMake folder, and one folder per configuration.
    dirs.extend(subfolders(&root.join("out").join("build")));
    dirs.extend(subfolders(&root.join("build")));
    let label = |dir: &Path| match dir.strip_prefix(root) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".to_owned(),
        Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
        Err(_) => dir.display().to_string(),
    };
    dirs.into_iter().filter(|d| has_database(d)).map(|dir| Candidate { label: label(&dir), dir }).collect()
}

/// Build folders CMake has configured in a project.
fn cmake_build_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.join("build")];
    let mut add_children = |parent: &Path, wanted: fn(&str) -> bool| {
        let Ok(read) = std::fs::read_dir(parent) else { return };
        let mut found: Vec<PathBuf> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| wanted(&n.to_string_lossy().to_lowercase())))
            .collect();
        found.sort();
        dirs.extend(found);
    };
    add_children(root, |n| n.starts_with("build-") || n.starts_with("cmake-build-"));
    add_children(&root.join("out").join("build"), |_| true);
    add_children(&root.join("build"), |_| true);
    dirs.into_iter().filter(|d| d.join("CMakeCache.txt").is_file()).collect()
}

/// The generator a CMake build folder was configured with.
pub fn generator_of(build_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(build_dir.join("CMakeCache.txt")).ok()?;
    text.lines().find_map(|l| l.strip_prefix("CMAKE_GENERATOR:INTERNAL=")).map(|g| g.trim().to_owned())
}

/// What to do about having no database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub why: String,
    /// A command to run from the project's folder, if one will do.
    pub command: Option<String>,
    /// What's left to do after it.
    pub then: Option<String>,
}

/// Generators that write a database; CMake ignores the setting for all others.
fn writes_database(generator: &str) -> bool {
    generator.contains("Ninja") || generator.contains("Makefiles")
}

/// How this project can get a database.
pub fn fix_for(root: &Path) -> Fix {
    const EXPORT: &str = "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON";
    if root.join("CMakeLists.txt").is_file() {
        let configured: Vec<(PathBuf, String)> =
            cmake_build_dirs(root).into_iter().filter_map(|d| generator_of(&d).map(|g| (d, g))).collect();
        if let Some((dir, generator)) = configured.iter().find(|(_, g)| writes_database(g)) {
            let name = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy().replace('\\', "/");
            return Fix {
                why: format!("{name} uses {generator}, which can write a database, but it wasn't asked to."),
                command: Some(format!("cmake -B {name} {EXPORT}")),
                then: None,
            };
        }
        if let Some((_, generator)) = configured.first() {
            // Its folder can't change generator, so this needs another one.
            return Fix {
                why: format!(
                    "{generator} doesn't write a database: CMake ignores {EXPORT} for it. \
                     A second build folder using Ninja gives clangd one, and your builds can stay as they are."
                ),
                command: Some(format!("cmake -S . -B build-clangd -G Ninja {EXPORT}")),
                then: Some("Then choose build-clangd here. Run it where your compiler is on PATH.".to_owned()),
            };
        }
        return Fix {
            why: "This CMake project hasn't been configured to write a database.".to_owned(),
            command: Some(format!("cmake -S . -B build -G Ninja {EXPORT}")),
            then: Some("clangd finds build/ by itself. Run it where your compiler is on PATH.".to_owned()),
        };
    }
    if ["Makefile", "makefile", "GNUmakefile"].iter().any(|m| root.join(m).is_file()) {
        return Fix {
            why: "Make doesn't write a database, but Bear can watch a build and write one.".to_owned(),
            command: Some("bear -- make -B".to_owned()),
            then: Some("This rebuilds everything once. Bear has to be installed.".to_owned()),
        };
    }
    let names =
        std::fs::read_dir(root).map(|r| r.flatten().map(|e| e.file_name()).collect::<Vec<_>>()).unwrap_or_default();
    let vs = names.iter().any(|n: &std::ffi::OsString| {
        let n = n.to_string_lossy().to_lowercase();
        n.ends_with(".sln") || n.ends_with(".slnx") || n.ends_with(".vcxproj")
    });
    if vs {
        return Fix {
            why: "Visual Studio projects don't write a database. Put the flags clangd needs in compile_flags.txt, \
                  one per line."
                .to_owned(),
            command: None,
            then: None,
        };
    }
    Fix {
        why: "No build system here says how files compile. Put the flags in compile_flags.txt, one per line, \
              such as -std=c++20 and -Iinclude."
            .to_owned(),
        command: None,
        then: None,
    }
}

/// When a file was last changed.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether the project's build files changed after the database was written.
pub fn is_stale(database: &Path, root: &Path) -> bool {
    let Some(written) = modified(database) else { return false };
    modified(&root.join("CMakeLists.txt")).is_some_and(|m| m > written)
}

/// Where a database cut down to one configuration is kept.
pub fn filtered_dir(database: &Path, config: &str) -> Option<PathBuf> {
    let mut hash = DefaultHasher::new();
    database.hash(&mut hash);
    let folder = format!("{:016x}", hash.finish());
    crate::settings::config_dir().map(|d| d.join("compile-db").join(folder).join(config))
}

/// Writes the database of one configuration. Returns its folder, for clangd.
pub fn write_filtered(database: &Path, config: &str) -> Result<PathBuf, String> {
    let dir = filtered_dir(database, config).ok_or("No config directory available")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = Database::load(database)?.filtered_text(config);
    std::fs::write(dir.join(DATABASE), text).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Writes a configuration's database again if the original has changed since.
fn refresh_filtered(database: &Path, config: &str) {
    let copy = filtered_dir(database, config).map(|d| d.join(DATABASE));
    let older = copy.is_some_and(|c| c.is_file() && modified(database) > modified(&c));
    if older {
        let _ = write_filtered(database, config);
    }
}

/// Where a file stands with clangd.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Real flags for this file.
    Flags,
    /// A header with no entry of its own: clangd borrows a nearby file's flags.
    Borrowed,
    /// A database exists, but doesn't mention this file.
    NotListed,
    /// A database exists, but clangd won't look there unless told.
    OutOfReach,
    /// Nothing to go on: clangd guesses, with default flags.
    Nothing,
    /// The database can't be read.
    Broken,
}

/// All of it, for one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub standing: Standing,
    pub database: Option<PathBuf>,
    pub error: Option<String>,
    pub candidates: Vec<Candidate>,
    /// Configurations the database mixes.
    pub configs: Vec<&'static str>,
    /// The one being read.
    pub config: Option<String>,
    /// The configuration was chosen, not just the first one clangd came to.
    pub config_chosen: bool,
    pub flags: Option<Flags>,
    pub stale: bool,
    pub fix: Option<Fix>,
    pub entries: usize,
    is_header: bool,
    uses_flags_file: bool,
    /// The flags come from the project's `.clangd`.
    uses_clangd_file: bool,
}

/// What a project's `.clangd` file says about compiling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClangdFile {
    pub database: Option<String>,
    pub add: Vec<String>,
}

/// Reads the `CompileFlags` part of a `.clangd`, in the forms it's written in.
pub fn parse_clangd_file(text: &str) -> ClangdFile {
    let mut out = ClangdFile::default();
    let (mut in_flags, mut in_add) = (false, false);
    let clean = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').to_owned();
    for line in text.lines() {
        let content = line.split(" #").next().unwrap_or(line);
        let trimmed = content.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "---" {
            (in_flags, in_add) = (false, false);
            continue;
        }
        let indented = content.starts_with(char::is_whitespace);
        if !indented {
            in_flags = trimmed.starts_with("CompileFlags:");
            in_add = false;
            continue;
        }
        if !in_flags {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("CompilationDatabase:") {
            let value = clean(value);
            out.database = (!value.is_empty() && value != "Ancestors" && value != "None").then_some(value);
            in_add = false;
        } else if let Some(value) = trimmed.strip_prefix("Add:") {
            let value = value.trim();
            in_add = value.is_empty();
            if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                out.add.extend(list.split(',').map(clean).filter(|f| !f.is_empty()));
            } else if !value.is_empty() {
                out.add.push(clean(value));
            }
        } else if let (true, Some(item)) = (in_add, trimmed.strip_prefix("- ")) {
            out.add.push(clean(item));
        } else {
            in_add = false;
        }
    }
    out
}

/// A chosen folder, relative to the project unless it's absolute.
fn chosen_folder(root: &Path, chosen: &str) -> Option<PathBuf> {
    let chosen = chosen.trim();
    (!chosen.is_empty()).then(|| root.join(chosen))
}

/// Looks at everything that decides what clangd reads for `file`.
pub fn report(file: &Path, root: &Path, chosen_dir: &str, chosen_config: &str) -> Report {
    let chosen = chosen_folder(root, chosen_dir);
    let is_header = file.extension().is_some_and(|e| HEADERS.contains(&e.to_string_lossy().to_lowercase().as_str()));
    let mut report = Report {
        standing: Standing::Nothing,
        database: None,
        error: None,
        candidates: candidates(root),
        configs: Vec::new(),
        config: None,
        config_chosen: false,
        flags: None,
        stale: false,
        fix: None,
        entries: 0,
        is_header,
        uses_flags_file: false,
        uses_clangd_file: false,
    };
    let clangd_file = std::fs::read_to_string(root.join(".clangd")).map(|t| parse_clangd_file(&t)).unwrap_or_default();
    let found = match (&chosen, &clangd_file.database) {
        (Some(dir), _) => Some(dir.join(DATABASE)),
        (None, Some(dir)) => Some(root.join(dir).join(DATABASE)),
        (None, None) => clangd_finds(file),
    };
    let Some(found) = found else {
        if !clangd_file.add.is_empty() {
            report.flags = Some(read_flags(&clangd_file.add, false));
            report.standing = Standing::Flags;
            report.uses_clangd_file = true;
            return report;
        }
        report.standing = if report.candidates.is_empty() { Standing::Nothing } else { Standing::OutOfReach };
        if report.candidates.is_empty() {
            report.fix = Some(fix_for(root));
        }
        return report;
    };
    report.stale = is_stale(&found, root);
    report.database = Some(found.clone());
    if found.file_name().is_some_and(|n| n == FLAGS_FILE) {
        let text = std::fs::read_to_string(&found).unwrap_or_default();
        let words: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
        report.flags = Some(read_flags(&words, false));
        report.standing = Standing::Flags;
        report.uses_flags_file = true;
        return report;
    }
    let database = match Database::load(&found) {
        Ok(database) => database,
        Err(why) => {
            report.standing = Standing::Broken;
            report.error = Some(why);
            report.fix = Some(fix_for(root));
            return report;
        }
    };
    report.entries = database.entries.len();
    report.configs = database.configs();
    // clangd takes the first of several entries for a file, so that's the truth
    // until a configuration is chosen.
    let wanted = report.configs.iter().find(|c| **c == chosen_config.trim()).copied();
    report.config_chosen = wanted.is_some();
    // clangd watches the copy it reads, so a new build's flags reach it.
    if let Some(config) = wanted {
        refresh_filtered(&found, config);
    }
    let entries = database.entries_for(file);
    let entry = match wanted {
        Some(config) => entries.iter().find(|e| e.config == Some(config)),
        None => entries.first(),
    };
    report.config = wanted.or_else(|| report.configs.first().copied()).map(str::to_owned);
    match entry {
        Some(entry) => {
            report.flags = Some(read_flags(&entry.args, true));
            report.standing = Standing::Flags;
        }
        None if is_header => report.standing = Standing::Borrowed,
        None => report.standing = Standing::NotListed,
    }
    report
}

impl Report {
    /// A few words for the status bar.
    pub fn short(&self) -> String {
        match self.standing {
            Standing::Flags if self.uses_flags_file => "compile_flags.txt".to_owned(),
            Standing::Flags if self.uses_clangd_file => ".clangd".to_owned(),
            Standing::Flags => self.config.clone().unwrap_or_else(|| "compile flags found".to_owned()),
            Standing::Borrowed => "flags borrowed".to_owned(),
            Standing::NotListed => "not in database".to_owned(),
            Standing::OutOfReach => "choose a database".to_owned(),
            Standing::Nothing => "no compile flags".to_owned(),
            Standing::Broken => "database broken".to_owned(),
        }
    }

    /// A sentence saying where things stand.
    pub fn headline(&self) -> String {
        match self.standing {
            Standing::Flags if self.uses_flags_file => {
                "clangd gives every file the flags in compile_flags.txt.".to_owned()
            }
            Standing::Flags if self.uses_clangd_file => {
                "There's no database, so clangd guesses, and adds the flags in .clangd.".to_owned()
            }
            Standing::Flags => "clangd compiles this file with these flags.".to_owned(),
            Standing::Borrowed => {
                "This header has no entry of its own, so clangd borrows the flags of a similar file.".to_owned()
            }
            Standing::NotListed => {
                format!("The database has {} entries, but none for this file: clangd guesses its flags.", self.entries)
            }
            Standing::OutOfReach => {
                "There's a database, but clangd won't look there by itself. Choose it below.".to_owned()
            }
            Standing::Nothing => {
                "clangd has no compile flags for this file: it guesses, so includes and defines may be wrong."
                    .to_owned()
            }
            Standing::Broken => "clangd can't use this database.".to_owned(),
        }
    }

    /// What else is worth knowing, as warnings.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        notes.extend(self.error.clone());
        if self.stale {
            notes.push("CMakeLists.txt changed after this database was written: configure again.".to_owned());
        }
        if self.configs.len() > 1 && !self.config_chosen {
            notes.push(format!(
                "This database mixes {} and clangd takes the first it finds. Choose a configuration to be sure.",
                self.configs.join(", ")
            ));
        }
        notes
    }
}

/// The folder to send clangd to, for what's chosen. A chosen configuration gets
/// a database of its own, written afresh.
pub fn clangd_directory(root: &Path, chosen_dir: &str, chosen_config: &str) -> Result<Option<PathBuf>, String> {
    let chosen = chosen_folder(root, chosen_dir);
    let config = chosen_config.trim();
    let database = match &chosen {
        Some(dir) => Some(dir.join(DATABASE)),
        None => clangd_finds(&root.join("_")).filter(|p| p.file_name().is_some_and(|n| n == DATABASE)),
    };
    if !config.is_empty()
        && let Some(database) = &database
        && Database::load(database)?.configs().contains(&config)
    {
        return write_filtered(database, config).map(Some);
    }
    Ok(chosen)
}

const HEADERS: [&str; 6] = ["h", "hh", "hpp", "hxx", "h++", "inl"];
const SOURCES: [&str; 7] = ["c", "cc", "cpp", "cxx", "c++", "m", "mm"];

pub fn is_c_family(extension: &str) -> bool {
    HEADERS.contains(&extension) || SOURCES.contains(&extension)
}

/// The header of a source file or the source of a header, by name: beside it,
/// then across the usual `include` and `src` folders.
pub fn counterpart(file: &Path, root: Option<&Path>) -> Option<PathBuf> {
    let ext = file.extension()?.to_string_lossy().to_lowercase();
    let stem = file.file_stem()?;
    let wanted: &[&str] = if HEADERS.contains(&ext.as_str()) {
        &SOURCES
    } else if SOURCES.contains(&ext.as_str()) {
        &HEADERS
    } else {
        return None;
    };
    let in_dir = |dir: &Path| wanted.iter().map(|e| dir.join(stem).with_extension(e)).find(|p| p.is_file());
    let dir = file.parent()?;
    if let Some(found) = in_dir(dir) {
        return Some(found);
    }
    // src/a/b.cpp <-> include/a/b.h, and include/<project>/b.h.
    const PAIRS: [(&str, &str); 4] = [("src", "include"), ("source", "include"), ("include", "src"), ("inc", "src")];
    for ancestor in dir.ancestors().take_while(|a| root.is_none_or(|r| a.starts_with(r))) {
        let Some(name) = ancestor.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { continue };
        let Some(parent) = ancestor.parent() else { continue };
        let rest = dir.strip_prefix(ancestor).unwrap_or(Path::new(""));
        for (from, to) in PAIRS {
            if name == from {
                let across = parent.join(to);
                if let Some(found) = in_dir(&across.join(rest)).or_else(|| in_dir(&across)) {
                    return Some(found);
                }
                let Ok(read) = std::fs::read_dir(&across) else { continue };
                for sub in read.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                    if let Some(found) = in_dir(&sub) {
                        return Some(found);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod clangd_file_tests {
    use super::*;

    #[test]
    fn reads_the_forms_it_comes_in() {
        let file = parse_clangd_file(
            "CompileFlags:\n  Add: [-std=c++20, \"-Iinclude\"]\n  Remove: -W*\n  CompilationDatabase: build-x\n",
        );
        assert_eq!(file.add, ["-std=c++20", "-Iinclude"]);
        assert_eq!(file.database.as_deref(), Some("build-x"));
        let file =
            parse_clangd_file("# note\nCompileFlags:\n  Add:\n    - -DA=1 # why\n    - -Ib\nDiagnostics:\n  Add: -x\n");
        assert_eq!(file.add, ["-DA=1", "-Ib"]);
        let file = parse_clangd_file("CompileFlags:\n  Add: -DONE\n  CompilationDatabase: Ancestors\n");
        assert_eq!(file.add, ["-DONE"]);
        assert_eq!(file.database, None);
        assert_eq!(parse_clangd_file("not: yaml\n"), ClangdFile::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rc-cdb-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn splits_commands_like_the_format_says() {
        let words = |c: &str| split_command(c);
        assert_eq!(words("g++ -c a.cpp"), ["g++", "-c", "a.cpp"]);
        assert_eq!(words(r#"cc "-DX=\"a b\"" -I"C:/My Dir""#), ["cc", r#"-DX="a b""#, "-IC:/My Dir"]);
        assert_eq!(words(r"C:\tools\cl.exe /c a.cpp"), [r"C:\tools\cl.exe", "/c", "a.cpp"]);
        assert_eq!(words(r#"a\\b 'c d' """#), [r"a\b", "c d", ""]);
    }

    #[test]
    fn reads_both_forms_of_entry() {
        let text = r#"[
            {"directory": "/w/b", "file": "../a.cpp", "command": "g++ -c ../a.cpp"},
            {"directory": "/w/b", "file": "/w/c.cpp", "arguments": ["g++", "-DX", "/w/c.cpp"]},
            {"directory": "/w/b", "command": "no file"}
        ]"#;
        let db = Database::parse(Path::new("/w/b/compile_commands.json"), text).unwrap();
        assert_eq!(db.entries.len(), 2);
        assert_eq!(db.entries_for(Path::new("/w/a.cpp")).len(), 1);
        assert_eq!(db.entries[1].args[1], "-DX");
        assert!(db.entries_for(Path::new("/w/none.cpp")).is_empty());
    }

    #[test]
    fn says_what_is_wrong_with_a_database() {
        assert!(Database::parse(Path::new("x.json"), "{ nope").unwrap_err().contains("x.json"));
        assert!(Database::parse(Path::new("x.json"), "{}").unwrap_err().contains("list"));
    }

    #[test]
    fn sorts_flags_out() {
        let args: Vec<String> = split_command(
            "clang++ --target=x86_64-w64-mingw32 -std=c++20 -DA=1 -D B -Iinc -I other -isystem /sys -Wall -O2 -c a.cpp -o a.o",
        );
        let flags = read_flags(&args, true);
        assert_eq!(flags.compiler.as_deref(), Some("clang++"));
        assert_eq!(flags.standard.as_deref(), Some("c++20"));
        assert_eq!(flags.target.as_deref(), Some("x86_64-w64-mingw32"));
        assert_eq!(flags.defines, ["A=1", "B"]);
        assert_eq!(flags.includes, ["inc", "other", "/sys"]);
        assert_eq!(flags.other, ["-Wall", "-O2"]);
    }

    #[test]
    fn sorts_msvc_flags_too() {
        let args = split_command("cl.exe /std:c++20 /DWIN32 /Iinc /W4 /c a.cpp /Foa.obj");
        let flags = read_flags(&args, true);
        assert_eq!(flags.standard.as_deref(), Some("c++20"));
        assert_eq!(flags.defines, ["WIN32"]);
        assert_eq!(flags.includes, ["inc"]);
        assert_eq!(flags.other, ["/W4"]);
    }

    #[test]
    fn flags_file_has_no_compiler() {
        let flags = read_flags(&["-std=c11".to_owned(), "-Iinc".to_owned()], false);
        assert_eq!(flags.compiler, None);
        assert_eq!(flags.standard.as_deref(), Some("c11"));
    }

    const MULTI: &str = r#"[
        {"directory": "/w/b", "file": "/w/a.cpp", "command": "g++ -DDBG /w/a.cpp", "output": "CMakeFiles/a.dir/Debug/a.cpp.o"},
        {"directory": "/w/b", "file": "/w/a.cpp", "command": "g++ -DNDEBUG /w/a.cpp", "output": "CMakeFiles/a.dir/Release/a.cpp.o"}
    ]"#;

    #[test]
    fn tells_configurations_apart_and_filters_them() {
        let db = Database::parse(Path::new("/w/b/compile_commands.json"), MULTI).unwrap();
        assert_eq!(db.configs(), ["Debug", "Release"]);
        let release = Database::parse(Path::new("x"), &db.filtered_text("Release")).unwrap();
        assert_eq!(release.entries.len(), 1);
        assert_eq!(release.entries[0].args[1], "-DNDEBUG");
        assert!(release.entries[0].directory.is_absolute() || cfg!(windows));
        assert_eq!(Database::parse(Path::new("x"), &db.filtered_text("Nope")).unwrap().entries.len(), 0);
    }

    #[test]
    fn a_single_config_database_has_none_to_pick() {
        let text = r#"[{"directory": "/w", "file": "a.c", "command": "cc a.c", "output": "a.o"}]"#;
        assert!(Database::parse(Path::new("x"), text).unwrap().configs().is_empty());
    }

    #[test]
    fn finds_what_clangd_would_and_what_it_would_not() {
        let root = temp("find");
        let file = root.join("src").join("a.cpp");
        write(&file, "");
        assert_eq!(clangd_finds(&file).filter(|p| p.starts_with(&root)), None);
        write(&root.join("out/build/x64-Debug").join(DATABASE), "[]");
        write(&root.join("cmake-build-debug").join(DATABASE), "[]");
        assert_eq!(clangd_finds(&file).filter(|p| p.starts_with(&root)), None, "too deep for clangd");
        let labels: Vec<String> = candidates(&root).into_iter().map(|c| c.label).collect();
        assert_eq!(labels, ["cmake-build-debug", "out/build/x64-Debug"]);
        write(&root.join("build").join(DATABASE), "[]");
        assert_eq!(clangd_finds(&file), Some(root.join("build").join(DATABASE)));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_fix_depends_on_the_generator() {
        let root = temp("fix");
        assert!(fix_for(&root).why.contains("compile_flags.txt"));
        write(&root.join("a.sln"), "");
        assert!(fix_for(&root).why.contains("Visual Studio"));
        std::fs::remove_file(root.join("a.sln")).unwrap();
        write(&root.join("Makefile"), "");
        assert!(fix_for(&root).command.unwrap().starts_with("bear"));
        write(&root.join("CMakeLists.txt"), "");
        assert_eq!(
            fix_for(&root).command.as_deref(),
            Some("cmake -S . -B build -G Ninja -DCMAKE_EXPORT_COMPILE_COMMANDS=ON")
        );
        write(&root.join("build/CMakeCache.txt"), "CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n");
        let vs = fix_for(&root);
        assert!(vs.why.contains("Visual Studio 17 2022"));
        assert!(vs.command.unwrap().contains("build-clangd"));
        write(&root.join("build/CMakeCache.txt"), "CMAKE_GENERATOR:INTERNAL=Ninja\n");
        assert_eq!(fix_for(&root).command.as_deref(), Some("cmake -B build -DCMAKE_EXPORT_COMPILE_COMMANDS=ON"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn switches_between_header_and_source() {
        let root = temp("switch");
        for name in ["a.cpp", "a.h", "src/b/c.cpp", "include/b/c.hpp", "src/d.cc", "include/proj/d.h", "lone.cpp"] {
            write(&root.join(name), "");
        }
        let other = |name: &str| counterpart(&root.join(name), Some(&root));
        assert_eq!(other("a.cpp"), Some(root.join("a.h")));
        assert_eq!(other("a.h"), Some(root.join("a.cpp")));
        assert_eq!(other("src/b/c.cpp"), Some(root.join("include/b/c.hpp")));
        assert_eq!(other("include/b/c.hpp"), Some(root.join("src/b/c.cpp")));
        assert_eq!(other("src/d.cc"), Some(root.join("include/proj/d.h")));
        assert_eq!(other("lone.cpp"), None);
        assert_eq!(counterpart(Path::new("x.rs"), None), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_where_a_file_stands() {
        let root = temp("report");
        let file = root.join("a.cpp");
        let header = root.join("a.h");
        write(&file, "");
        let inside = |standing: Standing| standing;
        // Anything above the temp folder could answer, so only look when it's ours.
        if clangd_finds(&file).is_none() {
            let nothing = report(&file, &root, "", "");
            assert_eq!(inside(nothing.standing), Standing::Nothing);
            assert!(nothing.fix.is_some());
            write(&root.join("cmake-build-debug").join(DATABASE), "[]");
            assert_eq!(report(&file, &root, "", "").standing, Standing::OutOfReach);
        }
        let quoted = file.to_string_lossy().replace('\\', "/");
        write(
            &root.join("b").join(DATABASE),
            &format!(
                r#"[
                {{"directory": ".", "file": "{quoted}", "command": "g++ -DDBG -c a.cpp", "output": "x/Debug/a.o"}},
                {{"directory": ".", "file": "{quoted}", "command": "g++ -DREL -c a.cpp", "output": "x/Release/a.o"}}
            ]"#
            ),
        );
        let first = report(&file, &root, "b", "");
        assert_eq!(first.standing, Standing::Flags);
        assert_eq!(first.flags.as_ref().unwrap().defines, ["DBG"]);
        assert_eq!(first.short(), "Debug");
        assert!(first.notes()[0].contains("mixes"));
        let release = report(&file, &root, "b", "Release");
        assert_eq!(release.flags.as_ref().unwrap().defines, ["REL"]);
        assert!(release.notes().is_empty());
        assert_eq!(report(&header, &root, "b", "").standing, Standing::Borrowed);
        assert_eq!(report(&root.join("c.cpp"), &root, "b", "").standing, Standing::NotListed);
        write(&root.join("bad").join(DATABASE), "{");
        assert_eq!(report(&file, &root, "bad", "").standing, Standing::Broken);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_chosen_configuration_gets_a_database_of_its_own() {
        let root = temp("directory");
        write(&root.join("b").join(DATABASE), MULTI);
        assert_eq!(clangd_directory(&root, "", "").unwrap(), None);
        assert_eq!(clangd_directory(&root, "b", "").unwrap(), Some(root.join("b")));
        assert_eq!(clangd_directory(&root, "b", "Nope").unwrap(), Some(root.join("b")));
        if let Some(dir) =
            clangd_directory(&root, "b", "Release").unwrap().filter(|_| crate::settings::config_dir().is_some())
        {
            let written = Database::load(&dir.join(DATABASE)).unwrap();
            assert_eq!(written.entries.len(), 1);
            // A new build reaches the copy clangd reads.
            let past = SystemTime::now() - std::time::Duration::from_secs(3600);
            std::fs::File::options().write(true).open(dir.join(DATABASE)).unwrap().set_modified(past).unwrap();
            write(&root.join("b").join(DATABASE), &MULTI.replace("-DNDEBUG", "-DFRESH"));
            report(&root.join("a.cpp"), &root, "b", "Release");
            let fresh = std::fs::read_to_string(dir.join(DATABASE)).unwrap();
            assert!(fresh.contains("-DFRESH"), "{fresh}");
            std::fs::remove_dir_all(dir).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn notices_a_database_older_than_its_project() {
        let root = temp("stale");
        write(&root.join(DATABASE), "[]");
        assert!(!is_stale(&root.join(DATABASE), &root), "no CMake file");
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(root.join(DATABASE)).unwrap().set_modified(past).unwrap();
        write(&root.join("CMakeLists.txt"), "");
        assert!(is_stale(&root.join(DATABASE), &root));
        std::fs::remove_dir_all(root).unwrap();
    }
}
