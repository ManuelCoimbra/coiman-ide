//! Source control: Git status, gutter change markers, staging and reverting
//! single hunks, inline blame, the side-by-side diff, the Source Control view
//! (commit, stage, discard) and branch switching. Every git command runs on a
//! background thread and reports back with a message, so the UI never waits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::commands::Command;
use coiman_core::project::git::{self, Commit, FileChange, Hunk, Repo, Stash, Status};
use egui::{Align, Color32, Pos2, Rect, RichText, Sense, Vec2};

use crate::document::Document;
use crate::ui::editor_view::{ChangeKind, ChangeMark, Decorations};
use crate::ui::icons::{self, Icon};
use crate::ui::palette::{InputKind, PickerKind};
use crate::ui::theme;
use crate::watcher::same_path;

use crate::app::{CodeApp, SidebarView, repaint};

/// Seconds the cursor stays on a line before its blame is looked up.
const BLAME_DELAY: f64 = 0.4;
/// Minimum seconds between gutter diffs while typing.
const GUTTER_INTERVAL: f64 = 0.15;
const CREATE_BRANCH: &str = "+ Create New Branch...";
/// Commits listed in the history.
const LOG_LIMIT: usize = 200;

/// What was clicked in the stash and history lists.
enum ListAction {
    Restore(String, bool),
    Drop(Stash),
    Expand(String),
    Collapse,
    OpenFile(String, String, String),
}

/// Results of background git work.
enum Msg {
    Repo(Option<Repo>, Option<String>),
    /// Index contents of the open files (`None`: not in the index), then the status.
    Refreshed(Vec<(PathBuf, Option<String>)>, Result<Status, String>),
    Blame {
        uid: u64,
        line: usize,
        version: u64,
        text: Result<String, String>,
    },
    Branches(Result<Vec<String>, String>),
    Diff(DiffSource),
    Lists(Result<(Vec<Commit>, Vec<Stash>), String>),
    CommitFiles(String, Result<Vec<(char, String)>, String>),
    LastMessage(Result<String, String>),
    Done {
        result: Result<String, String>,
    },
}

/// Texts loaded for a diff.
struct DiffSource {
    path: PathBuf,
    rel: String,
    staged: bool,
    left: String,
    /// `None`: use the open document's text.
    right: Option<String>,
    /// A commit's change, not your working files: shown read-only.
    commit: Option<String>,
}

/// One aligned row of the side-by-side diff.
struct DiffRow {
    left: Option<usize>,
    right: Option<usize>,
    hunk: Option<usize>,
}

pub(crate) struct DiffView {
    path: PathBuf,
    rel: String,
    /// HEAD ↔ index (staged changes) instead of index ↔ working tree.
    staged: bool,
    /// A local-history version (left) against the current text (right).
    history: Option<coiman_core::project::history::Entry>,
    /// What a commit changed, against its parent: read-only.
    commit: Option<String>,
    left: String,
    right: String,
    /// The right side is the open document (kept live while editing).
    live: Option<(u64, u64)>,
    ignore_whitespace: bool,
    hunks: Vec<Hunk>,
    rows: Vec<DiffRow>,
    scroll_to: Option<usize>,
    current: usize,
}

impl DiffView {
    fn rebuild(&mut self) {
        let (l, r) = (git::lines(&self.left), git::lines(&self.right));
        self.hunks = git::diff_lines(&l, &r, self.ignore_whitespace);
        self.rows.clear();
        let (mut i, mut j) = (0, 0);
        for (k, h) in self.hunks.iter().enumerate() {
            while i < h.old.start && j < h.new.start {
                self.rows.push(DiffRow { left: Some(i), right: Some(j), hunk: None });
                i += 1;
                j += 1;
            }
            for n in 0..h.old.len().max(h.new.len()) {
                let left = (n < h.old.len()).then(|| h.old.start + n);
                let right = (n < h.new.len()).then(|| h.new.start + n);
                self.rows.push(DiffRow { left, right, hunk: Some(k) });
            }
            (i, j) = (h.old.end, h.new.end);
        }
        while i < l.len() || j < r.len() {
            let row = DiffRow { left: (i < l.len()).then_some(i), right: (j < r.len()).then_some(j), hunk: None };
            self.rows.push(row);
            i += 1;
            j += 1;
        }
        self.current = self.current.min(self.hunks.len().saturating_sub(1));
    }

    fn row_of_hunk(&self, k: usize) -> Option<usize> {
        self.rows.iter().position(|r| r.hunk == Some(k))
    }
}

/// The hunk popup opened from a gutter marker.
struct HunkPopup {
    uid: u64,
    hunk: Hunk,
    anchor: Pos2,
    old_lines: Vec<String>,
    /// Opened this frame: the click that opened it is not an outside click.
    fresh: bool,
}

/// A destructive action waiting for confirmation.
struct Confirm {
    text: String,
    changes: Vec<FileChange>,
    /// Dropping this stash, rather than discarding `changes`.
    stash: Option<String>,
}

pub(crate) struct GitState {
    repo: Option<Repo>,
    /// Folder the repository was looked up for.
    pub(crate) discovered_for: Option<PathBuf>,
    me: Option<String>,
    status: Option<Status>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    status_running: bool,
    refresh_due: Option<f64>,
    /// A long operation in progress (commit, switch), shown in the status bar.
    busy: Option<String>,
    /// Index text by file key; `None` when the file isn't in the index.
    base: HashMap<PathBuf, Option<String>>,
    /// Bumped whenever `base` changes, to know when to re-diff.
    base_gen: u64,
    /// Gutter hunks: (doc uid, buffer version, base generation, hunks).
    gutter: Option<(u64, u64, u64, Vec<Hunk>)>,
    last_gutter: f64,
    blame: Option<(u64, usize, u64, String)>,
    /// Cursor line waiting for blame: (uid, line, version, since, requested).
    blame_rest: Option<(u64, usize, u64, f64, bool)>,
    pub(crate) message: String,
    pub(crate) diff: Option<DiffView>,
    popup: Option<HunkPopup>,
    confirm: Option<Confirm>,
    /// The next commit replaces the last one.
    amend: bool,
    log: Vec<Commit>,
    stashes: Vec<Stash>,
    /// The log and stashes may have changed since they were read.
    lists_stale: bool,
    history_open: bool,
    /// The commit showing its files.
    expanded: Option<(String, Vec<(char, String)>)>,
}

impl GitState {
    /// The repository, for setting its author.
    pub(crate) fn repo_for_setup(&self) -> Option<&Repo> {
        self.repo.as_ref()
    }
}

impl Default for GitState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            repo: None,
            discovered_for: None,
            me: None,
            status: None,
            tx,
            rx,
            status_running: false,
            refresh_due: None,
            busy: None,
            base: HashMap::new(),
            base_gen: 0,
            gutter: None,
            last_gutter: 0.0,
            blame: None,
            blame_rest: None,
            message: String::new(),
            diff: None,
            popup: None,
            confirm: None,
            amend: false,
            log: Vec::new(),
            stashes: Vec::new(),
            lists_stale: true,
            history_open: false,
            expanded: None,
        }
    }
}

fn key(path: &Path) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.to_string_lossy().to_lowercase()) } else { path.to_path_buf() }
}

fn line_ending_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Gutter marker for a hunk (in new-file line numbers).
fn mark_of(h: &Hunk) -> ChangeMark {
    let kind = if h.new.is_empty() {
        ChangeKind::Deleted
    } else if h.old.is_empty() {
        ChangeKind::Added
    } else {
        ChangeKind::Modified
    };
    ChangeMark { kind, lines: h.new.clone() }
}

fn hunk_at(hunks: &[Hunk], line: usize) -> Option<&Hunk> {
    hunks.iter().find(|h| h.new.contains(&line) || (h.new.is_empty() && h.new.start == line))
}

/// Color of a change letter in the Source Control view.
fn kind_color(kind: char) -> Color32 {
    match kind {
        'A' | 'U' => Color32::from_rgb(0x73, 0xC9, 0x91),
        'D' | '!' => Color32::from_rgb(0xF1, 0x4C, 0x4C),
        'R' => Color32::from_rgb(0x4F, 0xC1, 0xFF),
        _ => Color32::from_rgb(0xE2, 0xC0, 0x8D),
    }
}

impl CodeApp {
    // ----- background work -----------------------------------------------------------

    /// Runs `f` with the repository on a background thread.
    fn git_job(&mut self, ctx: &egui::Context, busy: Option<String>, f: impl FnOnce(&Repo) -> Msg + Send + 'static) {
        let Some(repo) = self.git.repo.clone() else { return };
        if busy.is_some() {
            self.git.busy = busy;
        }
        let tx = self.git.tx.clone();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(f(&repo));
            wake();
        });
    }

    /// Schedules a status refresh `delay` seconds from now.
    pub(crate) fn git_refresh_soon(&mut self, ctx: &egui::Context, delay: f64) {
        let at = ctx.input(|i| i.time) + delay;
        self.git.refresh_due = Some(self.git.refresh_due.map_or(at, |d| d.min(at)));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(delay));
    }

    /// Runs every frame: repository lookup, messages, refreshes, gutter, blame.
    /// The open repository, if any.
    pub(crate) fn git_repo(&self) -> Option<&Repo> {
        self.git.repo.as_ref()
    }

    pub(crate) fn pump_git(&mut self, ctx: &egui::Context) {
        self.pump_git_setup(ctx);
        // Keep a diff against the open document current while it is edited.
        if let Some(view) = &mut self.git.diff
            && let Some((uid, version)) = view.live
            && let Some(doc) = self.docs.iter().find(|d| d.uid == uid)
            && doc.buffer.version() != version
        {
            view.right = doc.text();
            view.live = Some((uid, doc.buffer.version()));
            view.rebuild();
        }
        if !self.settings.git.enabled {
            if self.git.repo.is_some() {
                self.git = crate::app::git::GitState::default();
            }
            return;
        }
        let target = self
            .explorer
            .root
            .clone()
            .or_else(|| self.docs.get(self.active).and_then(|d| d.path.as_ref()?.parent().map(Path::to_path_buf)));
        if target != self.git.discovered_for {
            self.git.discovered_for = target.clone();
            if let Some(dir) = target {
                let (tx, wake) = (self.git.tx.clone(), repaint(ctx));
                std::thread::spawn(move || {
                    let repo = Repo::discover(&dir);
                    let me = repo.as_ref().and_then(Repo::user_name);
                    let _ = tx.send(Msg::Repo(repo, me));
                    wake();
                });
            }
        }
        while let Ok(msg) = self.git.rx.try_recv() {
            self.on_git_msg(ctx, msg);
        }
        let now = ctx.input(|i| i.time);
        if self.git.refresh_due.is_some_and(|t| t <= now) && !self.git.status_running && self.git.repo.is_some() {
            self.git.refresh_due = None;
            self.git.status_running = true;
            let files: Vec<(PathBuf, String)> = {
                let repo = self.git.repo.as_ref().expect("checked");
                self.docs
                    .iter()
                    .filter_map(|d| d.path.clone())
                    .filter_map(|p| Some((p.clone(), repo.rel(&p)?)))
                    .collect()
            };
            self.git_job(ctx, None, move |repo| {
                // Index versions of the open files, then the status.
                let bases = files.into_iter().map(|(p, rel)| (p, repo.show(&rel, true))).collect();
                Msg::Refreshed(bases, repo.status())
            });
        }
        self.update_gutter(ctx, now);
        self.update_blame(ctx, now);
    }

    fn on_git_msg(&mut self, ctx: &egui::Context, msg: Msg) {
        match msg {
            Msg::Repo(repo, me) => {
                let found = repo.is_some();
                self.git.repo = repo;
                self.git.me = me;
                self.git.status = None;
                self.git.base.clear();
                self.git.base_gen += 1;
                if found {
                    self.git_refresh_soon(ctx, 0.0);
                }
            }
            Msg::Refreshed(bases, status) => {
                self.git.status_running = false;
                for (path, text) in bases {
                    self.git.base.insert(key(&path), text);
                }
                self.git.base_gen += 1;
                self.git.lists_stale = true;
                match status {
                    Ok(status) => self.git.status = Some(status),
                    Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
                }
            }
            Msg::Blame { uid, line, version, text } => {
                if let Ok(text) = text {
                    self.git.blame = Some((uid, line, version, text));
                }
            }
            Msg::Branches(result) => match result {
                Ok(branches) => {
                    let current = self.git.status.as_ref().map(|s| s.branch.clone()).unwrap_or_default();
                    let mut items = vec![CREATE_BRANCH.to_owned()];
                    items.extend(branches);
                    self.palette.show_picker(PickerKind::Branch, items, &current, "Select a branch to switch to");
                }
                Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
            },
            Msg::Diff(src) => self.show_diff(src),
            Msg::Lists(result) => match result {
                Ok((log, stashes)) => (self.git.log, self.git.stashes) = (log, stashes),
                Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
            },
            Msg::CommitFiles(hash, result) => match result {
                Ok(files) => self.git.expanded = Some((hash, files)),
                Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
            },
            Msg::LastMessage(result) => match result {
                // Unless something was typed in the meantime.
                Ok(message) if self.git.message.is_empty() => self.git.message = message,
                Ok(_) => {}
                Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
            },
            Msg::Done { result } => {
                self.git.busy = None;
                match result {
                    Ok(done) => {
                        if !done.is_empty() {
                            self.status = Some(done);
                        }
                    }
                    Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
                }
                self.git_refresh_soon(ctx, 0.0);
                if let Some(view) = self.git.diff.as_ref().filter(|v| v.history.is_none() && v.commit.is_none()) {
                    let (path, staged) = (view.path.clone(), view.staged);
                    self.open_diff(ctx, &path, staged);
                }
            }
        }
    }

    // ----- gutter and hunks ------------------------------------------------------------

    fn base_text(&self, path: &Path) -> Option<&str> {
        self.git.base.get(&key(path))?.as_deref()
    }

    fn update_gutter(&mut self, ctx: &egui::Context, now: f64) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = doc.path.clone() else { return };
        // A file opened since the last refresh: fetch its index version.
        let in_repo = self.git.repo.as_ref().is_some_and(|r| r.rel(&path).is_some());
        if in_repo && !self.git.base.contains_key(&key(&path)) {
            if !self.git.status_running && self.git.refresh_due.is_none() {
                self.git_refresh_soon(ctx, 0.0);
            }
            return;
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let path = &path;
        let (uid, version, generation) = (doc.uid, doc.buffer.version(), self.git.base_gen);
        if self.git.gutter.as_ref().is_some_and(|g| (g.0, g.1, g.2) == (uid, version, generation)) {
            return;
        }
        let same_doc = self.git.gutter.as_ref().is_some_and(|g| g.0 == uid && g.2 == generation);
        if same_doc && now - self.git.last_gutter < GUTTER_INTERVAL {
            return; // typing fast: catch up shortly
        }
        let hunks = match self.base_text(path) {
            Some(base) => {
                let text = doc.text();
                git::diff_lines(&git::lines(base), &git::lines(&text), false)
            }
            None => Vec::new(),
        };
        self.git.last_gutter = now;
        self.git.gutter = Some((uid, version, generation, hunks));
    }

    fn active_hunks(&self) -> &[Hunk] {
        match (&self.git.gutter, self.docs.get(self.active)) {
            (Some((uid, _, _, h)), Some(doc)) if *uid == doc.uid => h,
            _ => &[],
        }
    }

    /// Gutter markers and blame for document `i`.
    pub(crate) fn git_decorate(&self, i: usize, deco: &mut Decorations) {
        let Some(doc) = self.docs.get(i) else { return };
        if i == self.active {
            deco.changes = self.active_hunks().iter().map(mark_of).collect();
        }
        if let Some((uid, line, version, text)) = &self.git.blame
            && *uid == doc.uid
            && *version == doc.buffer.version()
            && self.settings.git.blame_inline
        {
            deco.blame = Some((*line, text.clone()));
        }
    }

    fn update_blame(&mut self, ctx: &egui::Context, now: f64) {
        if !self.settings.git.blame_inline || self.git.repo.is_none() {
            return;
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = doc.path.clone() else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        if self.git.blame.as_ref().is_some_and(|b| (b.0, b.1, b.2) == (uid, line, version)) {
            return;
        }
        match self.git.blame_rest {
            Some((u, l, v, since, requested)) if (u, l, v) == (uid, line, version) => {
                if requested {
                    return;
                }
                if now - since < BLAME_DELAY {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(BLAME_DELAY - (now - since)));
                    return;
                }
                self.git.blame_rest = Some((u, l, v, since, true));
                let Some(rel) = self.git.repo.as_ref().and_then(|r| r.rel(&path)) else { return };
                // Untracked files have no history.
                if self.git.base.get(&key(&path)).is_some_and(Option::is_none) {
                    return;
                }
                let text = doc.text();
                let me = self.git.me.clone();
                self.git_job(ctx, None, move |repo| {
                    let now =
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    let text =
                        repo.blame_line(&rel, line, &text).map(|b| git::format_blame(&b, now as i64, me.as_deref()));
                    Msg::Blame { uid, line, version, text }
                });
            }
            _ => {
                self.git.blame_rest = Some((uid, line, version, now, false));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(BLAME_DELAY));
            }
        }
    }

    /// Writes `content` as the index version of `path` (stages exactly that).
    fn write_index(&mut self, ctx: &egui::Context, path: &Path, content: String, done: &'static str) {
        let Some(rel) = self.git.repo.as_ref().and_then(|r| r.rel(path)) else { return };
        // Show the result right away; the refresh confirms it.
        self.git.base.insert(key(path), Some(content.clone()));
        self.git.base_gen += 1;
        self.git_job(ctx, None, move |repo| Msg::Done {
            result: repo.set_index_content(&rel, &content).map(|()| done.to_owned()),
        });
    }

    /// Stages one hunk of the change from `base` (index) to `new`.
    fn stage_hunk_text(&mut self, ctx: &egui::Context, path: &Path, base: &str, new: &str, h: &Hunk) {
        self.stage_hunks(ctx, path, base, new, std::slice::from_ref(h));
    }

    fn stage_hunks(&mut self, ctx: &egui::Context, path: &Path, base: &str, new: &str, hunks: &[Hunk]) {
        let content = git::apply_hunks(base, new, hunks, line_ending_of(base));
        self.write_index(ctx, path, content, "Staged change");
    }

    /// Puts the index's lines back in place of hunk `h` in document `i` (undoable).
    fn revert_hunk_in_doc(&mut self, i: usize, base: &str, h: &Hunk) {
        self.revert_hunks_in_doc(i, base, std::slice::from_ref(h));
    }

    /// The same for several hunks, as one undo step.
    fn revert_hunks_in_doc(&mut self, i: usize, base: &str, hunks: &[Hunk]) {
        let Some(doc) = self.docs.get_mut(i) else { return };
        let base_lines = git::lines(base);
        let buf = &doc.buffer;
        let rope = buf.rope();
        let nl = buf.line_ending;
        let ends_with_nl = rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n';
        let edits = hunks
            .iter()
            .map(|h| {
                let start = rope.line_to_char(h.new.start.min(rope.len_lines()));
                let end = rope.line_to_char(h.new.end.min(rope.len_lines()));
                let mut text: String = base_lines[h.old.clone()].iter().map(|l| format!("{l}{nl}")).collect();
                if end == rope.len_chars() && !ends_with_nl && !text.is_empty() {
                    if h.new.is_empty() {
                        // Re-adding lines after a last line that has no line ending.
                        text = format!("{nl}{}", text.strip_suffix(nl).unwrap_or(&text));
                    } else {
                        text.truncate(text.len() - nl.len());
                    }
                }
                (start, end, text)
            })
            .collect();
        doc.buffer.apply_edits(edits);
        doc.reveal = true;
    }

    /// Stage (or revert) the selected lines, or the hunk under the cursor.
    pub(crate) fn hunk_at_cursor(&mut self, ctx: &egui::Context, stage: bool) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let sel = doc.buffer.primary();
        let Some(path) = doc.path.clone() else { return };
        let hunks: Vec<Hunk> = if sel.is_empty() {
            hunk_at(self.active_hunks(), doc.buffer.char_to_line(sel.head)).into_iter().cloned().collect()
        } else {
            // A selection ending at the start of a line doesn't include that line.
            let last = doc.buffer.char_to_line(sel.end());
            let end = if doc.buffer.line_start(last) == sel.end() { last } else { last + 1 };
            git::hunks_in_lines(self.active_hunks(), doc.buffer.char_to_line(sel.start())..end)
        };
        if hunks.is_empty() {
            let place = if sel.is_empty() { "at the cursor" } else { "in the selection" };
            self.status = Some(format!("No change {place}"));
            return;
        }
        let Some(base) = self.base_text(&path).map(str::to_owned) else { return };
        if stage {
            let text = doc.text();
            self.stage_hunks(ctx, &path, &base, &text, &hunks);
        } else {
            self.revert_hunks_in_doc(self.active, &base, &hunks);
        }
    }

    /// Alt+F5 / Shift+Alt+F5: the next/previous change in the file.
    pub(crate) fn next_change(&mut self, forward: bool) {
        let hunks = self.active_hunks().to_vec();
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        if hunks.is_empty() {
            self.status = Some("No changes in this file".to_owned());
            return;
        }
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let target = if forward {
            hunks.iter().find(|h| h.new.start > line).or(hunks.first())
        } else {
            hunks.iter().rev().find(|h| h.new.start < line).or(hunks.last())
        };
        if let Some(h) = target {
            let l = h.new.start.min(doc.buffer.len_lines() - 1);
            let pos = doc.buffer.line_start(l);
            doc.select(pos, pos);
        }
    }

    /// A gutter marker was clicked: show the old text with Stage/Revert.
    pub(crate) fn open_hunk_popup(&mut self, line: usize, anchor: Pos2) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = &doc.path else { return };
        let Some(h) = hunk_at(self.active_hunks(), line).cloned() else { return };
        let old_lines =
            self.base_text(path).map(|b| git::lines(b)[h.old.clone()].iter().map(|l| (*l).to_owned()).collect());
        self.git.popup =
            Some(HunkPopup { uid: doc.uid, hunk: h, anchor, old_lines: old_lines.unwrap_or_default(), fresh: true });
    }

    // ----- diff view ---------------------------------------------------------------------

    /// Loads and shows the diff of `path`: index ↔ working tree, or HEAD ↔ index.
    pub(crate) fn open_diff(&mut self, ctx: &egui::Context, path: &Path, staged: bool) {
        let Some(rel) = self.git.repo.as_ref().and_then(|r| r.rel(path)) else {
            self.status = Some("The file isn't in a Git repository".to_owned());
            return;
        };
        let open = self.docs.iter().any(|d| d.path.as_deref().is_some_and(|p| same_path(p, path)));
        let path = path.to_path_buf();
        self.git_job(ctx, None, move |repo| {
            let (left, right) = if staged {
                (repo.show(&rel, false).unwrap_or_default(), Some(repo.show(&rel, true).unwrap_or_default()))
            } else {
                let left = repo.show(&rel, true).unwrap_or_default();
                let right = if open { None } else { Some(std::fs::read_to_string(&path).unwrap_or_default()) };
                (left, right)
            };
            Msg::Diff(DiffSource { path, rel, staged, left, right, commit: None })
        });
    }

    fn show_diff(&mut self, src: DiffSource) {
        let doc = self.docs.iter().find(|d| d.path.as_deref().is_some_and(|p| same_path(p, &src.path)));
        let (right, live) = match (src.right, doc) {
            (Some(text), _) => (text, None),
            (None, Some(d)) => (d.text(), Some((d.uid, d.buffer.version()))),
            (None, None) => (String::new(), None),
        };
        let keep = self.git.diff.as_ref().filter(|v| same_path(&v.path, &src.path) && v.staged == src.staged);
        let (ignore_whitespace, current) = keep.map_or((false, 0), |v| (v.ignore_whitespace, v.current));
        let mut view = DiffView {
            path: src.path,
            rel: src.rel,
            staged: src.staged,
            history: None,
            commit: src.commit,
            left: src.left,
            right,
            live,
            ignore_whitespace,
            hunks: Vec::new(),
            rows: Vec::new(),
            scroll_to: None,
            current,
        };
        view.rebuild();
        if keep.is_none() {
            view.scroll_to = view.row_of_hunk(0);
        }
        self.git.diff = Some(view);
    }

    /// Shows a local-history version of `path` (document `i`) against its current text.
    pub(crate) fn show_history_diff(
        &mut self,
        path: PathBuf,
        rel: String,
        old: String,
        entry: coiman_core::project::history::Entry,
        i: usize,
    ) {
        let doc = &self.docs[i];
        let mut view = DiffView {
            path,
            rel,
            staged: false,
            history: Some(entry),
            commit: None,
            left: old,
            right: doc.text(),
            live: Some((doc.uid, doc.buffer.version())),
            ignore_whitespace: false,
            hunks: Vec::new(),
            rows: Vec::new(),
            scroll_to: None,
            current: 0,
        };
        view.rebuild();
        view.scroll_to = view.row_of_hunk(0);
        self.git.diff = Some(view);
    }

    /// Stage / unstage / revert hunk `k` of the open diff.
    fn diff_hunk_action(&mut self, ctx: &egui::Context, k: usize, revert: bool) {
        let Some(view) = &self.git.diff else { return };
        let Some(h) = view.hunks.get(k).cloned() else { return };
        let (path, staged, left, right, live) =
            (view.path.clone(), view.staged, view.left.clone(), view.right.clone(), view.live);
        if view.history.is_some() {
            // Bring the old version's lines back into the document (undoable).
            if let Some(i) = live.and_then(|(uid, _)| self.docs.iter().position(|d| d.uid == uid)) {
                self.revert_hunk_in_doc(i, &left, &h);
            }
            return;
        }
        if staged {
            // Unstage: put HEAD's lines back into the index.
            let (head, index) = (git::lines(&left), git::lines(&right));
            let content = git::replace_lines(&index, h.new.clone(), &head[h.old.clone()], line_ending_of(&right));
            self.write_index(ctx, &path, content, "Unstaged change");
        } else if revert {
            let i = match live.and_then(|(uid, _)| self.docs.iter().position(|d| d.uid == uid)) {
                Some(i) => i,
                None => {
                    // Open the file so the revert can be reviewed and undone.
                    let Ok(doc) = Document::open(&path) else { return };
                    self.add_doc(doc);
                    self.docs.len() - 1
                }
            };
            self.revert_hunk_in_doc(i, &left, &h);
            self.open_diff(ctx, &path, false);
        } else {
            self.stage_hunk_text(ctx, &path, &left, &right, &h);
        }
    }

    /// Draws the side-by-side diff in place of the editor.
    pub(crate) fn diff_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(view) = &mut self.git.diff else { return };
        let pal = theme::colors();
        let mut action: Option<(usize, bool)> = None;
        let mut close = false;
        let mut restore = false;
        let history_label = view.history.as_ref().map(|e| {
            let ms =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
            format!("{} {} vs Current", e.source.label(), git::relative_time((ms.saturating_sub(e.time) / 1000) as i64))
        });
        ui.horizontal(|ui| {
            let commit_label = view.commit.as_ref().map(|c| format!("Before vs {c}"));
            let sides = match history_label.as_ref().or(commit_label.as_ref()) {
                Some(label) => label.as_str(),
                None if view.staged => "HEAD vs Index",
                None => "Index vs Working Tree",
            };
            ui.label(RichText::new(&view.rel).strong());
            ui.label(RichText::new(format!("({sides})")).small().color(pal.text_dim));
            ui.label(RichText::new(format!("{} change(s)", view.hunks.len())).small().color(pal.text_dim));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Close").clicked() {
                    close = true;
                }
                if view.history.is_some()
                    && ui
                        .small_button("Restore File")
                        .on_hover_text("Replace the whole file with this version")
                        .clicked()
                {
                    restore = true;
                }
                let n = view.hunks.len();
                if ui.small_button("Next").on_hover_text("Next change").clicked() && n > 0 {
                    view.current = (view.current + 1) % n;
                    view.scroll_to = view.row_of_hunk(view.current);
                }
                if ui.small_button("Previous").on_hover_text("Previous change").clicked() && n > 0 {
                    view.current = (view.current + n - 1) % n;
                    view.scroll_to = view.row_of_hunk(view.current);
                }
                if ui.checkbox(&mut view.ignore_whitespace, "Ignore whitespace").changed() {
                    view.rebuild();
                }
            });
        });
        let font = egui::FontId::monospace(self.font_size);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let (left, right) = (git::lines(&view.left), git::lines(&view.right));
        let removed = Color32::from_rgba_unmultiplied(0xF8, 0x51, 0x49, 40);
        let added = Color32::from_rgba_unmultiplied(0x2E, 0xA0, 0x43, 40);
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(row) = view.scroll_to.take() {
            area = area.vertical_scroll_offset((row as f32 * row_h - 3.0 * row_h).max(0.0));
        }
        let staged = view.staged;
        let history = view.history.is_some();
        let read_only = view.commit.is_some();
        area.show_rows(ui, row_h, view.rows.len(), |ui, range| {
            let width = ui.available_width();
            let actions_w = if staged || history { 64.0 } else { 110.0 };
            let num_w = 5.0 * char_w;
            let half = ((width - actions_w) / 2.0).max(50.0);
            for r in range {
                let row = &view.rows[r];
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::hover());
                let left_rect = Rect::from_min_size(rect.min, Vec2::new(half, row_h));
                let mid = Rect::from_min_size(Pos2::new(rect.left() + half, rect.top()), Vec2::new(actions_w, row_h));
                let right_rect = Rect::from_min_size(Pos2::new(mid.right(), rect.top()), Vec2::new(half, row_h));
                let changed = row.hunk.is_some();
                for (side, idx, lines, tint) in
                    [(left_rect, row.left, &left, removed), (right_rect, row.right, &right, added)]
                {
                    let p = ui.painter().with_clip_rect(side);
                    match idx {
                        Some(n) => {
                            if changed {
                                p.rect_filled(side, 0.0, tint);
                            }
                            p.text(
                                side.left_top() + Vec2::new(num_w - char_w, 0.0),
                                egui::Align2::RIGHT_TOP,
                                (n + 1).to_string(),
                                font.clone(),
                                pal.line_number,
                            );
                            let text = lines.get(n).copied().unwrap_or("").replace('\t', "    ");
                            let at = side.left_top() + Vec2::new(num_w + char_w, 0.0);
                            p.text(at, egui::Align2::LEFT_TOP, text, font.clone(), pal.editor_fg);
                        }
                        // Filler where the other side has extra lines.
                        None if changed => {
                            p.rect_filled(side, 0.0, pal.hover_bg);
                        }
                        None => {}
                    }
                }
                // Hunk buttons on the first row of each change.
                if let Some(k) = row.hunk
                    && (r == 0 || view.rows[r - 1].hunk != Some(k))
                    && !read_only
                {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new().max_rect(mid).layout(egui::Layout::left_to_right(Align::Center)),
                    );
                    if history {
                        if child.small_button("Revert").on_hover_text("Bring back these lines").clicked() {
                            action = Some((k, true));
                        }
                    } else if staged {
                        if child.small_button("Unstage").clicked() {
                            action = Some((k, false));
                        }
                    } else {
                        if child.small_button("Stage").clicked() {
                            action = Some((k, false));
                        }
                        if child.small_button("Revert").clicked() {
                            action = Some((k, true));
                        }
                    }
                }
            }
        });
        if restore
            && let Some(view) = &self.git.diff
            && let Some(entry) = view.history.clone()
        {
            let path = view.path.clone();
            self.restore_history(&path, &entry);
        } else if close {
            self.git.diff = None;
        } else if let Some((k, revert)) = action {
            self.diff_hunk_action(&ctx, k, revert);
        }
    }

    // ----- popups and dialogs ----------------------------------------------------------

    /// The gutter hunk popup and the discard confirmation.
    pub(crate) fn git_popups(&mut self, ctx: &egui::Context) {
        let active_uid = self.docs.get(self.active).map(|d| d.uid);
        let mut act: Option<bool> = None;
        let mut close = false;
        let fresh = self.git.popup.as_mut().is_some_and(|p| std::mem::take(&mut p.fresh));
        if let Some(p) = &self.git.popup {
            if Some(p.uid) != active_uid {
                close = true;
            } else {
                let pal = theme::colors();
                let font = egui::FontId::monospace(self.font_size * 0.95);
                let resp = egui::Area::new(egui::Id::unique("git_hunk_popup"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(p.anchor + Vec2::new(12.0, 0.0))
                    .constrain(true)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).fill(pal.sidebar_bg).inner_margin(8.0).show(ui, |ui| {
                            ui.set_max_width(600.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Changed lines").strong());
                                if ui.small_button("Stage").clicked() {
                                    act = Some(true);
                                }
                                if ui.small_button("Revert").clicked() {
                                    act = Some(false);
                                }
                                if icons::button(ui, Icon::Close, "Close").clicked() {
                                    close = true;
                                }
                            });
                            if p.old_lines.is_empty() {
                                ui.label(RichText::new("(new lines)").color(pal.text_dim));
                            } else {
                                ui.label(RichText::new("Before:").small().color(pal.text_dim));
                                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                    for l in &p.old_lines {
                                        let bg = Color32::from_rgba_unmultiplied(0xF8, 0x51, 0x49, 40);
                                        ui.label(
                                            RichText::new(l.replace('\t', "    "))
                                                .font(font.clone())
                                                .background_color(bg),
                                        );
                                    }
                                });
                            }
                        });
                    });
                if ctx.input(|i| i.key_pressed(egui::Key::Escape))
                    || (!fresh
                        && ctx.input(|i| i.pointer.any_pressed())
                        && !ctx.pointer_interact_pos().is_some_and(|q| resp.response.rect.contains(q)))
                {
                    close = true;
                }
            }
        }
        if let Some(stage) = act
            && let Some(p) = self.git.popup.take()
            && let Some(i) = self.docs.iter().position(|d| d.uid == p.uid)
            && let Some(path) = self.docs[i].path.clone()
            && let Some(base) = self.base_text(&path).map(str::to_owned)
        {
            if stage {
                let text = self.docs[i].text();
                self.stage_hunk_text(ctx, &path, &base, &text, &p.hunk);
            } else {
                self.revert_hunk_in_doc(i, &base, &p.hunk);
            }
        }
        if close {
            self.git.popup = None;
        }

        let mut confirmed = None;
        if let Some(c) = &self.git.confirm {
            egui::Window::new(if c.stash.is_some() { "Drop Stash" } else { "Discard Changes" })
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(&c.text);
                    ui.label(RichText::new("This can't be undone.").color(theme::colors().text_dim));
                    ui.horizontal(|ui| {
                        if ui.button(if c.stash.is_some() { "Drop" } else { "Discard" }).clicked() {
                            confirmed = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            confirmed = Some(false);
                        }
                    });
                });
        }
        if let Some(yes) = confirmed
            && let Some(c) = self.git.confirm.take()
            && yes
        {
            self.git_job(ctx, None, move |repo| Msg::Done {
                result: match &c.stash {
                    Some(name) => repo.stash_drop(name).map(|()| "Dropped stash".to_owned()),
                    None => repo.discard(&c.changes).map(|()| format!("Discarded {} file(s)", c.changes.len())),
                },
            });
        }
    }

    // ----- Source Control view, commit, branches ---------------------------------------

    pub(crate) fn git_commit(&mut self, ctx: &egui::Context) {
        let message = self.git.message.trim().to_owned();
        let Some(status) = &self.git.status else { return };
        if self.git.amend {
            self.git.amend = false;
            self.git.message.clear();
            self.git_job(ctx, Some("Amending…".to_owned()), move |repo| {
                let message = (!message.is_empty()).then_some(message.as_str());
                Msg::Done { result: repo.amend(message).map(|()| "Amended the last commit".to_owned()) }
            });
            return;
        }
        if message.is_empty() {
            self.status = Some("Type a commit message first".to_owned());
            return;
        }
        let stage_all = status.staged.is_empty();
        if stage_all && status.unstaged.is_empty() {
            self.status = Some("There are no changes to commit".to_owned());
            return;
        }
        self.git.message.clear();
        self.git_job(ctx, Some("Committing…".to_owned()), move |repo| {
            let result = if stage_all { repo.stage(&["."]) } else { Ok(()) };
            Msg::Done { result: result.and_then(|()| repo.commit(&message)).map(|()| "Committed".to_owned()) }
        });
    }

    pub(crate) fn git_stash(&mut self, ctx: &egui::Context) {
        if self.git.repo.is_none() {
            self.status = Some("Open a folder in a Git repository first".to_owned());
        } else if self.git.busy.is_none() {
            self.git_job(ctx, Some("Stashing…".to_owned()), |repo| Msg::Done {
                result: repo
                    .stash_push()
                    .map(|any| if any { "Stashed changes" } else { "No changes to stash" }.to_owned()),
            });
        }
    }

    pub(crate) fn git_show_history(&mut self) {
        self.git.history_open = true;
        self.show_source_control();
    }

    /// A file as a commit changed it, against the commit's parent.
    fn open_commit_file(&mut self, ctx: &egui::Context, hash: String, short: String, rel: String) {
        let Some(root) = self.git.repo.as_ref().map(|r| r.root.clone()) else { return };
        self.git_job(ctx, None, move |repo| {
            let left = repo.file_at(&format!("{hash}^"), &rel);
            let right = Some(repo.file_at(&hash, &rel));
            let path = root.join(&rel);
            Msg::Diff(DiffSource { path, rel, staged: false, left, right, commit: Some(short) })
        });
    }

    /// Push, pull or fetch, in the background. Nothing else reaches the network.
    pub(crate) fn git_remote(&mut self, ctx: &egui::Context, command: Command) {
        if self.git.repo.is_none() {
            self.status = Some("Open a folder in a Git repository first".to_owned());
            return;
        }
        if self.git.busy.is_some() {
            return;
        }
        let (busy, done): (&str, &str) = match command {
            Command::GitPush => ("Pushing…", "Pushed"),
            Command::GitPull => ("Pulling…", "Pulled"),
            _ => ("Fetching…", "Fetched"),
        };
        self.git_job(ctx, Some(busy.to_owned()), move |repo| {
            let result = match command {
                Command::GitPush => repo.push(),
                Command::GitPull => repo.pull(),
                _ => repo.fetch(),
            };
            Msg::Done { result: result.map(|()| done.to_owned()) }
        });
    }

    pub(crate) fn git_checkout(&mut self, ctx: &egui::Context) {
        if self.git.repo.is_none() {
            self.status = Some("Open a folder in a Git repository first".to_owned());
            return;
        }
        self.git_job(ctx, None, |repo| Msg::Branches(repo.branches()));
    }

    pub(crate) fn pick_branch(&mut self, ctx: &egui::Context, label: &str) {
        if label == CREATE_BRANCH {
            self.palette.show_input(InputKind::NewBranch, "", "New branch name, then Enter");
            return;
        }
        self.switch_branch(ctx, label.to_owned(), false);
    }

    pub(crate) fn switch_branch(&mut self, ctx: &egui::Context, branch: String, create: bool) {
        let busy = format!("Switching to {branch}…");
        self.git_job(ctx, Some(busy), move |repo| Msg::Done {
            result: repo.switch(&branch, create).map(|()| format!("Switched to {branch}")),
        });
    }

    /// Branch for the status bar: "main*" (with uncommitted changes), "↑1 ↓2".
    pub(crate) fn git_branch_label(&self) -> Option<String> {
        if let Some(busy) = &self.git.busy {
            return Some(busy.clone());
        }
        let s = self.git.status.as_ref()?;
        let dirty = if s.staged.is_empty() && s.unstaged.is_empty() { "" } else { "*" };
        let mut label = format!("{}{dirty}", s.branch);
        if s.ahead > 0 || s.behind > 0 {
            label.push_str(&format!("  {}↓ {}↑", s.behind, s.ahead));
        }
        Some(label)
    }

    /// The Source Control side bar view.
    pub(crate) fn source_control_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        ui.horizontal(|ui| {
            ui.label(RichText::new("SOURCE CONTROL").small().strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if self.git.repo.is_some() && icons::button(ui, Icon::Refresh, "Refresh").clicked() {
                    self.git_refresh_soon(&ctx, 0.0);
                }
            });
        });
        if self.settings.git.enabled && self.git_setup_section(ui) {
            return;
        }
        let Some(status) = self.git.status.clone() else {
            let text = if !self.settings.git.enabled {
                "Git is turned off (git.enabled)."
            } else if self.git.repo.is_none() {
                "The open folder isn't in a Git repository."
            } else {
                "Loading…"
            };
            ui.label(RichText::new(text).color(pal.text_dim));
            return;
        };
        if ui
            .button(format!("Branch: {}", status.branch))
            .on_hover_text("Switch branch (Git: Checkout to...)")
            .clicked()
        {
            self.git_checkout(&ctx);
        }
        self.git_upstream_line(ui, &status.branch, status.upstream.as_deref(), status.ahead, status.behind);
        if self.git.lists_stale {
            self.git.lists_stale = false;
            self.git_job(&ctx, None, |repo| {
                Msg::Lists(repo.log(LOG_LIMIT).and_then(|log| Ok((log, repo.stash_list()?))))
            });
        }
        ui.horizontal(|ui| {
            let busy = self.git.busy.is_some();
            let pull = format!("Pull {}", status.behind);
            let push = format!("Push {}", status.ahead);
            for (label, tip, command) in [
                (
                    if status.behind > 0 { pull.as_str() } else { "Pull" },
                    "Get new commits from the remote",
                    Command::GitPull,
                ),
                (
                    if status.ahead > 0 { push.as_str() } else { "Push" },
                    "Send your commits to the remote",
                    Command::GitPush,
                ),
                ("Fetch", "See what's new on the remote without changing anything here", Command::GitFetch),
            ] {
                if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(tip).clicked() {
                    self.git_remote(&ctx, command);
                }
            }
            let tip = "Put all changes away, untracked files too";
            if ui.add_enabled(!busy, egui::Button::new("Stash")).on_hover_text(tip).clicked() {
                self.git_stash(&ctx);
            }
        });
        self.git_remotes_section(ui);
        let hint = format!("Message (Ctrl+Enter to commit on '{}')", status.branch);
        let edit = ui.add(
            egui::TextEdit::multiline(&mut self.git.message)
                .hint_text(hint)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        let commit_key =
            edit.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        let tip = "Replace the last commit with this message and what's staged. If it was already pushed, your next push will be refused.";
        let was_amending = self.git.amend;
        if ui.checkbox(&mut self.git.amend, "Amend last commit").on_hover_text(tip).changed() {
            if !was_amending && self.git.message.is_empty() {
                self.git_job(&ctx, None, |repo| Msg::LastMessage(repo.last_commit_message()));
            } else if was_amending {
                self.git.message.clear();
            }
        }
        let label = if self.git.amend {
            "Amend"
        } else if status.staged.is_empty() && !status.unstaged.is_empty() {
            "Stage All and Commit"
        } else {
            "Commit"
        };
        let busy = self.git.busy.is_some();
        let commit = ui.add_enabled(!busy, egui::Button::new(label).min_size(Vec2::new(ui.available_width(), 24.0)));
        if commit.clicked() || commit_key {
            self.git_commit(&ctx);
        }
        ui.add_space(4.0);

        let mut open: Option<(PathBuf, bool, char)> = None;
        let mut stage: Vec<String> = Vec::new();
        let mut unstage: Vec<String> = Vec::new();
        let mut discard: Vec<FileChange> = Vec::new();
        let mut list_action = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let sections: [(&str, &Vec<FileChange>, u8); 3] = [
                ("Merge Changes", &status.conflicts, 0),
                ("Staged Changes", &status.staged, 1),
                ("Changes", &status.unstaged, 2),
            ];
            for (title, files, kind) in sections {
                if files.is_empty() {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{title}  {}", files.len())).small().strong());
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| match kind {
                        1 => {
                            if icons::button(ui, Icon::Minus, "Unstage All").clicked() {
                                unstage.extend(files.iter().map(|f| f.rel.clone()));
                            }
                        }
                        _ => {
                            if icons::button(ui, Icon::Plus, "Stage All").clicked() {
                                stage.extend(files.iter().map(|f| f.rel.clone()));
                            }
                            if kind == 2 && icons::button(ui, Icon::Discard, "Discard All Changes").clicked() {
                                discard.extend(files.iter().cloned());
                            }
                        }
                    });
                });
                for f in files {
                    ui.horizontal(|ui| {
                        let name =
                            Path::new(&f.rel).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let dir =
                            Path::new(&f.rel).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                        let resp = ui.add(egui::Label::new(name).sense(Sense::click()).truncate());
                        ui.add(egui::Label::new(RichText::new(dir).small().color(pal.text_dim)).truncate());
                        if resp.clicked() {
                            open = Some((f.path.clone(), kind == 1, f.kind));
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(f.kind.to_string()).color(kind_color(f.kind)).strong());
                            if kind == 1 {
                                if icons::button(ui, Icon::Minus, "Unstage Changes").clicked() {
                                    unstage.push(f.rel.clone());
                                }
                            } else {
                                if icons::button(ui, Icon::Plus, "Stage Changes").clicked() {
                                    stage.push(f.rel.clone());
                                }
                                if kind == 2 && icons::button(ui, Icon::Discard, "Discard Changes").clicked() {
                                    discard.push(f.clone());
                                }
                            }
                        });
                    });
                }
                ui.add_space(4.0);
            }
            if status.staged.is_empty() && status.unstaged.is_empty() && status.conflicts.is_empty() {
                ui.label(RichText::new("No changes.").color(pal.text_dim));
            }
            ui.add_space(8.0);
            if !self.git.stashes.is_empty() {
                ui.label(RichText::new(format!("Stashes  {}", self.git.stashes.len())).small().strong());
                for stash in &self.git.stashes {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Drop").on_hover_text("Delete this stash").clicked() {
                                list_action = Some(ListAction::Drop(stash.clone()));
                            }
                            if ui.small_button("Pop").on_hover_text("Bring it back and remove it").clicked() {
                                list_action = Some(ListAction::Restore(stash.name.clone(), true));
                            }
                            if ui.small_button("Apply").on_hover_text("Bring it back and keep it").clicked() {
                                list_action = Some(ListAction::Restore(stash.name.clone(), false));
                            }
                            ui.add(egui::Label::new(&stash.message).truncate()).on_hover_text(&stash.message);
                        });
                    });
                }
                ui.add_space(8.0);
            }
            let history = egui::CollapsingHeader::new(RichText::new("History").small().strong())
                .id_salt("git_history")
                .open(self.git.history_open.then_some(true))
                .show(ui, |ui| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64);
                    if self.git.log.is_empty() {
                        ui.label(RichText::new("No commits yet.").color(pal.text_dim));
                    }
                    for commit in &self.git.log {
                        let expanded = self.git.expanded.as_ref().filter(|e| e.0 == commit.hash);
                        let row = ui.add(egui::Label::new(&commit.subject).sense(Sense::click()).truncate());
                        let tip = format!(
                            "{}\n{}, {}\n{}",
                            commit.subject,
                            commit.author,
                            git::relative_time(now - commit.time),
                            commit.hash
                        );
                        if row.on_hover_text(tip).clicked() {
                            list_action = Some(if expanded.is_some() {
                                ListAction::Collapse
                            } else {
                                ListAction::Expand(commit.hash.clone())
                            });
                        }
                        let refs = if commit.refs.is_empty() { String::new() } else { format!(" ({})", commit.refs) };
                        let detail = format!(
                            "{}  {}  {}{refs}",
                            commit.short,
                            commit.author,
                            git::relative_time(now - commit.time)
                        );
                        ui.add(egui::Label::new(RichText::new(detail).small().color(pal.text_dim)).truncate());
                        for (kind, rel) in expanded.iter().flat_map(|e| &e.1) {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.label(RichText::new(kind.to_string()).color(kind_color(*kind)).strong());
                                if ui.add(egui::Label::new(rel).sense(Sense::click()).truncate()).clicked() {
                                    list_action = Some(ListAction::OpenFile(
                                        commit.hash.clone(),
                                        commit.short.clone(),
                                        rel.clone(),
                                    ));
                                }
                            });
                        }
                        ui.add_space(3.0);
                    }
                });
            if self.git.history_open {
                self.git.history_open = false;
                let _ = history;
            }
        });

        match list_action {
            Some(ListAction::Restore(name, pop)) => {
                self.git_job(&ctx, Some("Restoring stash…".to_owned()), move |repo| Msg::Done {
                    result: repo.stash_restore(&name, pop).map(|()| "Restored stash".to_owned()),
                });
            }
            Some(ListAction::Drop(stash)) => {
                let text = format!("Delete the stash \"{}\"?", stash.message);
                self.git.confirm = Some(Confirm { text, changes: Vec::new(), stash: Some(stash.name) });
            }
            Some(ListAction::Expand(hash)) => {
                self.git_job(&ctx, None, move |repo| {
                    let files = repo.commit_files(&hash);
                    Msg::CommitFiles(hash, files)
                });
            }
            Some(ListAction::Collapse) => self.git.expanded = None,
            Some(ListAction::OpenFile(hash, short, rel)) => self.open_commit_file(&ctx, hash, short, rel),
            None => {}
        }
        if !stage.is_empty() {
            self.git_job(&ctx, None, move |repo| {
                let rels: Vec<&str> = stage.iter().map(String::as_str).collect();
                Msg::Done { result: repo.stage(&rels).map(|()| String::new()) }
            });
        }
        if !unstage.is_empty() {
            self.git_job(&ctx, None, move |repo| {
                let rels: Vec<&str> = unstage.iter().map(String::as_str).collect();
                Msg::Done { result: repo.unstage(&rels).map(|()| String::new()) }
            });
        }
        if !discard.is_empty() {
            let text = match discard.as_slice() {
                [one] if one.kind == 'U' => format!("Delete the untracked file {}?", one.rel),
                [one] => format!("Discard the changes in {}?", one.rel),
                many => format!("Discard the changes in {} files?", many.len()),
            };
            self.git.confirm = Some(Confirm { text, changes: discard, stash: None });
        }
        if let Some((path, staged, kind)) = open {
            if kind == 'U' {
                self.open_file(&path);
            } else {
                self.open_diff(&ctx, &path, staged);
            }
        }
    }

    pub(crate) fn show_source_control(&mut self) {
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Git;
    }
}
