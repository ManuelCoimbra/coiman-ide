//! Reading a project as a whole: what imports what, cycles, coupling and the reach of a change,
//! and its health: secrets, complexity, duplicates, licences and audits.

pub mod arch;
pub mod complexity;
pub mod deps;
pub mod duplicates;
pub mod packages;
pub mod secrets;

/// Folders of downloaded or built things, never the project's own code: skipped even where
/// no `.gitignore` says so.
const NOT_THE_PROJECTS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".gradle",
    ".next",
    "coverage",
    ".tox",
    "bower_components",
];

/// The project's files: `.gitignore` heeded, downloaded and built folders left out. `hidden`
/// takes in dot files too (`.env`).
pub fn project_files(root: &std::path::Path, hidden: bool) -> impl Iterator<Item = ignore::DirEntry> {
    ignore::WalkBuilder::new(root)
        .hidden(!hidden)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && NOT_THE_PROJECTS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
}
