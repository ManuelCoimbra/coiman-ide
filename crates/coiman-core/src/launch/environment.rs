//! One answer to "which Python is this project's?", shared by the terminal, Run
//! and the language server, with the reason it was chosen. Without a single
//! answer each tool guesses on its own, and they fail by quietly disagreeing.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::language::lsp::servers::which;

/// A Python and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonEnv {
    pub python: PathBuf,
    /// The environment's folder; `None` for a system Python, which needs no activating.
    pub prefix: Option<PathBuf>,
    /// Short name for the status bar, like `.venv`.
    pub name: String,
    /// Why this one, for people.
    pub reason: String,
    pub version: Option<String>,
    /// A conda environment, which sets different variables.
    pub conda: bool,
}

/// Folder names of a project's own virtual environment.
const VENV_NAMES: &[&str] = &[".venv", "venv", "env"];

/// A choice from `python.interpreter`: an environment folder, or a Python.
static CHOSEN: RwLock<String> = RwLock::new(String::new());

/// Sets the user's choice, for everything that resolves from now on.
pub fn set_chosen(chosen: &str) {
    *CHOSEN.write().unwrap_or_else(|e| e.into_inner()) = chosen.trim().to_owned();
}

fn python_in(prefix: &Path) -> Option<PathBuf> {
    let inside: &[&[&str]] = if cfg!(windows) {
        &[&["Scripts", "python.exe"], &["python.exe"]]
    } else {
        &[&["bin", "python"], &["bin", "python3"]]
    };
    inside
        .iter()
        .map(|parts| parts.iter().fold(prefix.to_path_buf(), |path, part| path.join(part)))
        .find(|p| p.is_file())
}

/// The Python version an environment records about itself. Reading it needs no process.
fn recorded_version(prefix: &Path) -> Option<String> {
    let config = std::fs::read_to_string(prefix.join("pyvenv.cfg")).ok()?;
    config.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        matches!(key.trim(), "version" | "version_info").then(|| value.trim().to_owned())
    })
}

fn from_prefix(prefix: &Path, reason: String, conda: bool) -> Option<PythonEnv> {
    let python = python_in(prefix)?;
    let name = prefix.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let version = recorded_version(prefix);
    Some(PythonEnv { python, prefix: Some(prefix.to_path_buf()), name, reason, version, conda })
}

/// Folders from `start` up to the open folder, or the filesystem's top without one.
fn upward<'a>(start: &'a Path, root: Option<&'a Path>) -> impl Iterator<Item = &'a Path> {
    let stop = root.filter(|r| start.starts_with(r));
    let mut done = false;
    start.ancestors().take_while(move |dir| {
        let keep = !done;
        done = done || stop.is_some_and(|r| *dir == r);
        keep
    })
}

fn project_venv(start: &Path, root: Option<&Path>) -> Option<PythonEnv> {
    upward(start, root).find_map(|dir| {
        VENV_NAMES.iter().find_map(|name| {
            let prefix = dir.join(name);
            let uv = dir.join("uv.lock").is_file();
            let reason = if uv {
                format!("`{name}` in {}, the project's uv environment", dir.display())
            } else {
                format!("`{name}` in {}", dir.display())
            };
            from_prefix(&prefix, reason, false)
        })
    })
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// A `.python-version` file naming a version pyenv has installed.
fn pyenv(start: &Path, root: Option<&Path>, home: Option<&Path>) -> Option<PythonEnv> {
    let (dir, wanted) = upward(start, root).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join(".python-version")).ok()?;
        Some((dir, text.lines().next()?.trim().to_owned()))
    })?;
    let home = home?;
    let roots = [home.join(".pyenv/pyenv-win/versions"), home.join(".pyenv/versions")];
    roots.iter().find_map(|versions| {
        let reason = format!("`.python-version` in {} asks for {wanted}, from pyenv", dir.display());
        let mut found = from_prefix(&versions.join(&wanted), reason, false)?;
        found.version = Some(wanted.clone());
        // An installed Python needs no activating, but tools must find it first.
        found.name = format!("pyenv {wanted}");
        Some(found)
    })
}

/// The name in an `environment.yml`, and where conda keeps it.
fn conda_env(start: &Path, root: Option<&Path>, home: Option<&Path>) -> Option<PythonEnv> {
    let (dir, name) = upward(start, root).find_map(|dir| {
        let text =
            ["environment.yml", "environment.yaml"].iter().find_map(|f| std::fs::read_to_string(dir.join(f)).ok())?;
        let name = text.lines().find_map(|line| line.strip_prefix("name:"))?;
        Some((dir, name.trim().trim_matches(['"', '\'']).to_owned()))
    })?;
    let home = home?;
    ["miniconda3", "anaconda3", "miniforge3", "mambaforge", ".conda"].iter().find_map(|base| {
        let reason = format!("`environment.yml` in {} names the conda environment {name}", dir.display());
        from_prefix(&home.join(base).join("envs").join(&name), reason, true)
    })
}

/// An environment activated before the editor started.
fn inherited() -> Option<PythonEnv> {
    let from = |var: &str, conda| {
        let prefix = PathBuf::from(std::env::var_os(var)?);
        from_prefix(&prefix, format!("`{var}` was set when Coiman IDE started"), conda)
    };
    from("VIRTUAL_ENV", false).or_else(|| from("CONDA_PREFIX", true))
}

fn system() -> Option<PythonEnv> {
    let python = ["python", "python3", "py"].iter().find_map(|p| which(p))?;
    Some(PythonEnv {
        python,
        prefix: None,
        name: "system".to_owned(),
        reason: "No environment for this project: the first Python on PATH".to_owned(),
        version: None,
        conda: false,
    })
}

/// The user's choice, as an environment.
fn chosen_env(chosen: &str, start: &Path, root: Option<&Path>) -> Option<PythonEnv> {
    let path = PathBuf::from(chosen);
    let path = if path.is_relative() { root.unwrap_or(start).join(path) } else { path };
    let reason = "Chosen in `python.interpreter`".to_owned();
    if path.is_dir() {
        return from_prefix(&path, reason, false);
    }
    if !path.is_file() {
        return None;
    }
    // A Python inside an environment activates it; any other just runs.
    let prefix = path
        .parent()
        .and_then(Path::parent)
        .filter(|p| p.join("pyvenv.cfg").is_file())
        .or_else(|| path.parent().filter(|p| p.join("pyvenv.cfg").is_file()));
    match prefix {
        Some(prefix) => from_prefix(prefix, reason, false),
        None => Some(PythonEnv {
            name: path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            python: path,
            prefix: None,
            reason,
            version: None,
            conda: false,
        }),
    }
}

/// The project's Python, for a file in `start`.
pub fn resolve(start: &Path, root: Option<&Path>) -> Option<PythonEnv> {
    let chosen = CHOSEN.read().unwrap_or_else(|e| e.into_inner()).clone();
    if !chosen.is_empty()
        && let Some(env) = chosen_env(&chosen, start, root)
    {
        return Some(env);
    }
    let home = home();
    project_venv(start, root)
        .or_else(|| pyenv(start, root, home.as_deref()))
        .or_else(|| conda_env(start, root, home.as_deref()))
        .or_else(inherited)
        .or_else(system)
}

/// Every Python to offer: the project's, then the system's.
pub fn candidates(start: &Path, root: Option<&Path>) -> Vec<PythonEnv> {
    let mut found = Vec::new();
    for dir in upward(start, root) {
        for name in VENV_NAMES {
            found.extend(from_prefix(&dir.join(name), format!("`{name}` in {}", dir.display()), false));
        }
        // Any other name: an environment says what it is with `pyvenv.cfg`.
        let mut others: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("pyvenv.cfg").is_file())
            .collect();
        others.sort();
        for prefix in others {
            found.extend(from_prefix(&prefix, format!("A virtual environment in {}", dir.display()), false));
        }
    }
    let home = home();
    found.extend(pyenv(start, root, home.as_deref()));
    found.extend(conda_env(start, root, home.as_deref()));
    found.extend(inherited());
    found.extend(system());
    let mut seen = std::collections::HashSet::new();
    found.retain(|env| seen.insert(env.python.clone()));
    found
}

impl PythonEnv {
    /// What a program needs set for this environment to be the active one.
    pub fn variables(&self) -> Vec<(String, String)> {
        let Some(prefix) = &self.prefix else { return Vec::new() };
        let folders: Vec<PathBuf> = if cfg!(windows) {
            let mut list = vec![prefix.join("Scripts")];
            if self.conda {
                list.extend([prefix.clone(), prefix.join("Library").join("bin")]);
            }
            list
        } else {
            vec![prefix.join("bin")]
        };
        let current = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(folders.into_iter().chain(std::env::split_paths(&current)))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let variable = if self.conda { "CONDA_PREFIX" } else { "VIRTUAL_ENV" };
        vec![(variable.to_owned(), prefix.to_string_lossy().into_owned()), ("PATH".to_owned(), joined)]
    }

    /// For the status bar: `.venv 3.12.1`.
    pub fn label(&self) -> String {
        let version = self.version.as_deref().filter(|_| !self.name.contains(' '));
        version.map_or_else(|| self.name.clone(), |v| format!("{} {v}", self.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust-code-env-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_env(prefix: &Path, version: &str) {
        let inside = if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" };
        let python = prefix.join(inside);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(python, "").unwrap();
        std::fs::write(prefix.join("pyvenv.cfg"), format!("home = x\nversion = {version}\n")).unwrap();
    }

    #[test]
    fn the_nearest_project_environment_wins() {
        let dir = folder("near");
        let sub = dir.join("pkg/src");
        std::fs::create_dir_all(&sub).unwrap();
        fake_env(&dir.join(".venv"), "3.12.1");
        let found = project_venv(&sub, Some(&dir)).unwrap();
        assert_eq!(found.label(), ".venv 3.12.1");
        assert!(found.reason.contains("`.venv`"));
        fake_env(&dir.join("pkg/venv"), "3.11.0");
        assert_eq!(project_venv(&sub, Some(&dir)).unwrap().name, "venv");
        // Not above the open folder.
        assert!(project_venv(&sub, Some(&dir.join("pkg/src"))).is_none());
        std::fs::write(dir.join("uv.lock"), "").unwrap();
        std::fs::remove_dir_all(dir.join("pkg/venv")).unwrap();
        assert!(project_venv(&sub, Some(&dir)).unwrap().reason.contains("uv"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn any_environment_in_the_project_is_offered() {
        let dir = folder("offer");
        fake_env(&dir.join(".venv"), "3.12.1");
        fake_env(&dir.join("second"), "3.11.0");
        std::fs::create_dir_all(dir.join("plain")).unwrap();
        let found = candidates(&dir, Some(&dir));
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".venv") && names.contains(&"second"), "{names:?}");
        assert!(!names.contains(&"plain"));
        assert_eq!(names.iter().filter(|n| **n == ".venv").count(), 1, "offered once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pyenv_and_conda_come_from_project_files() {
        let dir = folder("files");
        let home = dir.join("home");
        let project = dir.join("p");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(".python-version"), "3.10.4\n").unwrap();
        assert!(pyenv(&project, None, Some(&home)).is_none(), "not installed");
        fake_env(&home.join(".pyenv/versions/3.10.4"), "x");
        let found = pyenv(&project, None, Some(&home)).unwrap();
        assert_eq!((found.label().as_str(), found.version.as_deref()), ("pyenv 3.10.4", Some("3.10.4")));
        std::fs::write(project.join("environment.yml"), "name: science\nchannels: []\n").unwrap();
        fake_env(&home.join("miniconda3/envs/science"), "x");
        let found = conda_env(&project, None, Some(&home)).unwrap();
        assert!(found.conda && found.reason.contains("science"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_choice_can_be_a_folder_or_a_python() {
        let dir = folder("choice");
        fake_env(&dir.join("mine"), "3.9.0");
        let inside = if cfg!(windows) { "mine/Scripts/python.exe" } else { "mine/bin/python" };
        assert_eq!(chosen_env("mine", &dir, Some(&dir)).unwrap().name, "mine");
        let by_python = chosen_env(inside, &dir, Some(&dir)).unwrap();
        assert_eq!(by_python.prefix.as_deref(), Some(dir.join("mine").as_path()), "it activates its environment");
        std::fs::write(dir.join("loose.exe"), "").unwrap();
        assert_eq!(chosen_env("loose.exe", &dir, Some(&dir)).unwrap().prefix, None);
        assert!(chosen_env("gone", &dir, Some(&dir)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn activating_puts_the_environment_first() {
        let dir = folder("vars");
        fake_env(&dir.join(".venv"), "3.12.1");
        let found = from_prefix(&dir.join(".venv"), String::new(), false).unwrap();
        let variables = found.variables();
        assert_eq!(variables[0], ("VIRTUAL_ENV".to_owned(), dir.join(".venv").to_string_lossy().into_owned()));
        let first = std::env::split_paths(&variables[1].1).next().unwrap();
        assert!(first.starts_with(dir.join(".venv")));
        let plain = PythonEnv { prefix: None, ..found };
        assert!(plain.variables().is_empty(), "a system Python needs nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
