//! The File Manager: folders in one or two panes, each with tabs, beside places and
//! bookmarks, with a preview of what's picked. It shows in place of the editor like the
//! other views, and works alongside the Explorer: the same clipboard, drags between them, and
//! the same copy, move, rename and delete (open files follow, unsaved work is saved first).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use coiman_core::project::dir_listing::{self, Entry, Saved, SortBy};
use eframe::egui::{self, Key, Modifiers, RichText, Sense};

use crate::app::CodeApp;
use crate::ui::explorer::DraggedFiles;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

/// A folder is listed again this often while it shows, to catch changes made elsewhere.
const RELIST_SECS: f64 = 2.0;
/// Text files up to this size get a preview.
const PREVIEW_BYTES: u64 = 512 * 1024;
const PREVIEW_LINES: usize = 60;
const ROW: f32 = 22.0;
const PLACES_WIDTH: f32 = 170.0;
const PREVIEW_WIDTH: f32 = 260.0;
/// Column widths beside the name.
const SIZE_WIDTH: f32 = 80.0;
const DATE_WIDTH: f32 = 130.0;
const KIND_WIDTH: f32 = 64.0;
/// The name column keeps at least this; other columns go before it gets narrower.
const NAME_MIN: f32 = 150.0;

/// One tab of a pane: a folder, its history and what's picked in it.
struct Folder {
    dir: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    entries: Vec<Entry>,
    error: Option<String>,
    listed_at: f64,
    selected: BTreeSet<PathBuf>,
    /// Where a Shift+click range starts, and where the arrow keys are.
    anchor: Option<PathBuf>,
    cursor: Option<PathBuf>,
    sort: SortBy,
    descending: bool,
    filter: String,
    /// The path box's text while it's being typed in.
    path_text: String,
    scroll_to_cursor: bool,
}

impl Folder {
    fn new(dir: PathBuf) -> Self {
        Self {
            path_text: dir.display().to_string(),
            dir,
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            error: None,
            listed_at: f64::NEG_INFINITY,
            selected: BTreeSet::new(),
            anchor: None,
            cursor: None,
            sort: SortBy::Name,
            descending: false,
            filter: String::new(),
            scroll_to_cursor: false,
        }
    }

    fn title(&self) -> String {
        self.dir.file_name().map_or_else(|| self.dir.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Goes to `dir`, keeping where it was for Back. The folder it came from is picked, so
    /// going up lands on it.
    fn go(&mut self, dir: PathBuf) {
        if dir == self.dir {
            return;
        }
        let came_from = std::mem::replace(&mut self.dir, dir);
        self.forward.clear();
        self.back.push(came_from.clone());
        self.arrive(Some(came_from));
    }

    fn arrive(&mut self, pick: Option<PathBuf>) {
        self.path_text = self.dir.display().to_string();
        self.filter.clear();
        self.selected.clear();
        self.anchor = None;
        self.cursor = None;
        if let Some(pick) = pick.filter(|p| p.parent() == Some(self.dir.as_path())) {
            self.selected.insert(pick.clone());
            self.anchor = Some(pick.clone());
            self.cursor = Some(pick);
            self.scroll_to_cursor = true;
        }
        self.listed_at = f64::NEG_INFINITY;
    }

    fn go_back(&mut self) {
        if let Some(dir) = self.back.pop() {
            let left = std::mem::replace(&mut self.dir, dir);
            self.forward.push(left.clone());
            self.arrive(Some(left));
        }
    }

    fn go_forward(&mut self) {
        if let Some(dir) = self.forward.pop() {
            let left = std::mem::replace(&mut self.dir, dir);
            self.back.push(left);
            self.arrive(None);
        }
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.go(parent);
        }
    }

    /// Lists the folder again when it's due (or `now` is forced with NEG_INFINITY listed_at).
    fn relist(&mut self, now: f64) {
        if now - self.listed_at < RELIST_SECS {
            return;
        }
        self.listed_at = now;
        match dir_listing::list(&self.dir) {
            Ok(mut entries) => {
                dir_listing::sort(&mut entries, self.sort, self.descending);
                self.selected.retain(|p| entries.iter().any(|e| &e.path == p));
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
        }
    }

    fn resort(&mut self) {
        dir_listing::sort(&mut self.entries, self.sort, self.descending);
    }

    /// The entries the filter lets through, in order.
    fn shown(&self, show_hidden: bool) -> Vec<&Entry> {
        self.entries.iter().filter(|e| dir_listing::passes(e, &self.filter, show_hidden)).collect()
    }

    /// What an action works on: the picked entries, in listing order.
    fn picked(&self) -> Vec<PathBuf> {
        self.entries.iter().filter(|e| self.selected.contains(&e.path)).map(|e| e.path.clone()).collect()
    }

    fn pick_only(&mut self, path: &Path) {
        self.selected = BTreeSet::from([path.to_path_buf()]);
        self.anchor = Some(path.to_path_buf());
        self.cursor = Some(path.to_path_buf());
    }

    /// A click with the keys held: alone picks one, Ctrl adds or takes away, Shift picks a range.
    fn click(&mut self, path: &Path, mods: Modifiers, show_hidden: bool) {
        if mods.shift
            && let Some(anchor) = self.anchor.clone()
        {
            let shown: Vec<PathBuf> = self.shown(show_hidden).iter().map(|e| e.path.clone()).collect();
            let (a, b) = (shown.iter().position(|p| *p == anchor), shown.iter().position(|p| p == path));
            if let (Some(a), Some(b)) = (a, b) {
                let range = a.min(b)..=a.max(b);
                if !mods.command {
                    self.selected.clear();
                }
                self.selected.extend(shown[range].iter().cloned());
                self.cursor = Some(path.to_path_buf());
                return;
            }
        }
        if mods.command {
            if !self.selected.remove(path) {
                self.selected.insert(path.to_path_buf());
            }
            self.anchor = Some(path.to_path_buf());
            self.cursor = Some(path.to_path_buf());
        } else {
            self.pick_only(path);
        }
    }

    /// The arrow keys: `by` rows from the cursor; Shift extends the pick.
    fn step(&mut self, by: isize, extend: bool, show_hidden: bool) {
        let shown: Vec<PathBuf> = self.shown(show_hidden).iter().map(|e| e.path.clone()).collect();
        if shown.is_empty() {
            return;
        }
        let at = self.cursor.as_ref().and_then(|c| shown.iter().position(|p| p == c));
        let next = match at {
            None => 0,
            Some(i) => (i as isize + by).clamp(0, shown.len() as isize - 1) as usize,
        };
        let path = shown[next].clone();
        if extend {
            let anchor = self.anchor.clone().unwrap_or_else(|| path.clone());
            let a = shown.iter().position(|p| *p == anchor).unwrap_or(next);
            self.selected = shown[a.min(next)..=a.max(next)].iter().cloned().collect();
            self.anchor = Some(anchor);
            self.cursor = Some(path);
        } else {
            self.pick_only(&path);
        }
        self.scroll_to_cursor = true;
    }
}

/// A pane: its tabs, one in front.
struct Pane {
    tabs: Vec<Folder>,
    tab: usize,
}

impl Pane {
    fn new(dirs: Vec<PathBuf>, tab: usize) -> Self {
        let tabs: Vec<Folder> = dirs.into_iter().map(Folder::new).collect();
        let tab = tab.min(tabs.len().saturating_sub(1));
        Self { tabs, tab }
    }

    fn folder(&self) -> &Folder {
        &self.tabs[self.tab]
    }

    fn folder_mut(&mut self) -> &mut Folder {
        &mut self.tabs[self.tab]
    }

    fn new_tab(&mut self) {
        let dir = self.folder().dir.clone();
        self.tabs.insert(self.tab + 1, Folder::new(dir));
        self.tab += 1;
    }

    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() > 1 && i < self.tabs.len() {
            self.tabs.remove(i);
            if self.tab >= i && self.tab > 0 {
                self.tab -= 1;
            }
        }
    }
}

/// A question waiting at the top of the view.
enum Ask {
    Delete(Vec<PathBuf>, bool),
    NewFolder(PathBuf, String),
    Rename(PathBuf, String),
}

/// What's shown about the one picked entry.
enum Preview {
    Text(String),
    Picture(u32, u32),
    Folder(usize),
    Binary,
    TooLarge,
    Unreadable(String),
}

/// What Open File and Open Folder ask the File Manager to choose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PickFor {
    /// A folder, opened as the project.
    Folder,
    /// Files, opened in the editor.
    Files,
}

/// What the view asks the app to do.
enum Do {
    OpenProject(PathBuf),
    OpenFiles(Vec<PathBuf>),
    /// Picking is over (chosen or cancelled).
    EndPick,
    Open(PathBuf),
    Move(Vec<PathBuf>, PathBuf),
    Copy(Vec<PathBuf>, PathBuf),
    Delete(Vec<PathBuf>, bool),
    Rename(PathBuf, String),
    NewFolder(PathBuf, String),
    Clip(Vec<PathBuf>, bool),
    Paste(PathBuf),
    Note(String),
}

struct View {
    panes: [Pane; 2],
    dual: bool,
    focus: usize,
    showing: bool,
    /// The document that was in front: switching to another file leaves the view.
    over: Option<u64>,
    ask: Option<Ask>,
    show_hidden: bool,
    bookmarks: Vec<PathBuf>,
    preview: Option<(PathBuf, Option<std::time::SystemTime>, Preview)>,
    stored: Saved,
    /// Open File or Open Folder is choosing with it.
    pick: Option<PickFor>,
    /// It was opened for that alone, so it closes once chosen.
    pick_closes: bool,
}

#[derive(Default)]
pub(crate) struct FileManager {
    view: Option<View>,
    /// Asked for since the last frame: open, at this folder when there's one.
    to_open: Option<(Option<PathBuf>, Option<PickFor>)>,
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn count(n: usize) -> String {
    plural(n, "item")
}

/// "1 file", "2 files".
fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn preview_of(entry: &Entry) -> Preview {
    if entry.is_dir {
        return match std::fs::read_dir(&entry.path) {
            Ok(items) => Preview::Folder(items.count()),
            Err(e) => Preview::Unreadable(e.to_string()),
        };
    }
    if crate::app::views::image_view::is_image(&entry.path)
        && let Ok((w, h)) = image::image_dimensions(&entry.path)
    {
        return Preview::Picture(w, h);
    }
    if entry.size > PREVIEW_BYTES {
        return Preview::TooLarge;
    }
    match std::fs::read(&entry.path) {
        Ok(bytes) if bytes.contains(&0) => Preview::Binary,
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            Preview::Text(text.lines().take(PREVIEW_LINES).collect::<Vec<_>>().join("\n"))
        }
        Err(e) => Preview::Unreadable(e.to_string()),
    }
}

impl View {
    fn new(saved: Saved, start: Option<PathBuf>, fallback: PathBuf) -> Self {
        let existing = |dirs: &[PathBuf]| -> Vec<PathBuf> { dirs.iter().filter(|d| d.is_dir()).cloned().collect() };
        let pane = |i: usize| {
            let (dirs, tab) = saved.panes.get(i).cloned().unwrap_or_default();
            let mut dirs = existing(&dirs);
            if dirs.is_empty() {
                dirs.push(fallback.clone());
            }
            Pane::new(dirs, tab)
        };
        let mut view = Self {
            panes: [pane(0), pane(1)],
            dual: saved.dual,
            focus: 0,
            showing: true,
            over: None,
            ask: None,
            show_hidden: saved.show_hidden,
            bookmarks: saved.bookmarks.clone(),
            preview: None,
            stored: saved,
            pick: None,
            pick_closes: false,
        };
        if let Some(dir) = start {
            view.panes[0].folder_mut().go(dir);
        }
        view
    }

    fn saved(&self) -> Saved {
        Saved {
            bookmarks: self.bookmarks.clone(),
            panes: self.panes.iter().map(|p| (p.tabs.iter().map(|t| t.dir.clone()).collect(), p.tab)).collect(),
            dual: self.dual,
            show_hidden: self.show_hidden,
        }
    }

    fn focused(&self) -> &Folder {
        self.panes[self.focus].folder()
    }

    fn focused_mut(&mut self) -> &mut Folder {
        self.panes[self.focus].folder_mut()
    }

    /// The other pane's folder: where Copy and Move send things. None with one pane.
    fn other_dir(&self) -> Option<PathBuf> {
        self.dual.then(|| self.panes[1 - self.focus].folder().dir.clone())
    }

    fn relist_all(&mut self) {
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.listed_at = f64::NEG_INFINITY;
            }
        }
    }

    fn rename_picked(&mut self) {
        if let [one] = self.focused().picked().as_slice() {
            self.ask = Some(Ask::Rename(one.clone(), name_of(one)));
        }
    }

    fn send_to_other(&mut self, out: &mut Vec<Do>, moving: bool) {
        let picked = self.focused().picked();
        match self.other_dir() {
            None => out.push(Do::Note("Copy and Move go to the other pane: show two panes first".to_owned())),
            Some(_) if picked.is_empty() => {}
            Some(to) if moving => out.push(Do::Move(picked, to)),
            Some(to) => out.push(Do::Copy(picked, to)),
        }
    }

    /// The keyboard, while the view is in front and no text box has it.
    fn keys(&mut self, ctx: &egui::Context, out: &mut Vec<Do>) {
        let key = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if let Some(Ask::Delete(paths, permanent)) = &self.ask {
            if key(Modifiers::NONE, Key::Enter) {
                out.push(Do::Delete(paths.clone(), *permanent));
                self.ask = None;
            } else if key(Modifiers::NONE, Key::Escape) {
                self.ask = None;
            }
            return;
        }
        let hidden = self.show_hidden;
        // Shift first: a plain key's check would take the Shift one too.
        if key(Modifiers::SHIFT, Key::ArrowDown) {
            self.focused_mut().step(1, true, hidden);
        }
        if key(Modifiers::NONE, Key::ArrowDown) {
            self.focused_mut().step(1, false, hidden);
        }
        if key(Modifiers::SHIFT, Key::ArrowUp) {
            self.focused_mut().step(-1, true, hidden);
        }
        if key(Modifiers::NONE, Key::ArrowUp) {
            self.focused_mut().step(-1, false, hidden);
        }
        if key(Modifiers::NONE, Key::Home) {
            self.focused_mut().step(isize::MIN / 2, false, hidden);
        }
        if key(Modifiers::NONE, Key::End) {
            self.focused_mut().step(isize::MAX / 2, false, hidden);
        }
        if key(Modifiers::NONE, Key::PageDown) {
            self.focused_mut().step(15, false, hidden);
        }
        if key(Modifiers::NONE, Key::PageUp) {
            self.focused_mut().step(-15, false, hidden);
        }
        if key(Modifiers::NONE, Key::Enter) {
            self.open_cursor(out);
        }
        if key(Modifiers::NONE, Key::Backspace) {
            self.focused_mut().go_up();
        }
        if key(Modifiers::ALT, Key::ArrowLeft) {
            self.focused_mut().go_back();
        }
        if key(Modifiers::ALT, Key::ArrowRight) {
            self.focused_mut().go_forward();
        }
        if key(Modifiers::ALT, Key::ArrowUp) {
            self.focused_mut().go_up();
        }
        if key(Modifiers::SHIFT, Key::Delete) {
            self.ask_delete(true);
        }
        if key(Modifiers::NONE, Key::Delete) {
            self.ask_delete(false);
        }
        if key(Modifiers::NONE, Key::F2) {
            self.rename_picked();
        }
        if key(Modifiers::NONE, Key::F5) {
            self.send_to_other(out, false);
        }
        if key(Modifiers::NONE, Key::F6) {
            self.send_to_other(out, true);
        }
        if key(Modifiers::NONE, Key::F7) || key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N) {
            self.ask = Some(Ask::NewFolder(self.focused().dir.clone(), String::new()));
        }
        if key(Modifiers::COMMAND, Key::A) {
            let shown: BTreeSet<PathBuf> = self.focused().shown(hidden).iter().map(|e| e.path.clone()).collect();
            self.focused_mut().selected = shown;
        }
        // Ctrl+C, X and V arrive as Copy, Cut and Paste events, not as keys.
        let (copy, cut, paste) = ctx.input_mut(|i| {
            let mut found = (false, false, false);
            i.events.retain(|e| match e {
                egui::Event::Copy => {
                    found.0 = true;
                    false
                }
                egui::Event::Cut => {
                    found.1 = true;
                    false
                }
                egui::Event::Paste(_) => {
                    found.2 = true;
                    false
                }
                _ => true,
            });
            found
        });
        if copy || cut {
            let picked = self.focused().picked();
            if !picked.is_empty() {
                out.push(Do::Clip(picked, cut));
            }
        }
        if paste {
            out.push(Do::Paste(self.focused().dir.clone()));
        }
        if key(Modifiers::NONE, Key::Tab) && self.dual {
            self.focus = 1 - self.focus;
        }
        if key(Modifiers::COMMAND, Key::T) {
            self.panes[self.focus].new_tab();
        }
        if key(Modifiers::COMMAND, Key::R) {
            self.relist_all();
        }
        if self.pick.is_some() && key(Modifiers::NONE, Key::Escape) {
            out.push(Do::EndPick);
        }
        if key(Modifiers::NONE, Key::Escape) {
            self.focused_mut().selected.clear();
        }
    }

    fn ask_delete(&mut self, permanent: bool) {
        let picked = self.focused().picked();
        if !picked.is_empty() {
            self.ask = Some(Ask::Delete(picked, permanent));
        }
    }

    /// Enter, or a double click: a folder opens in the pane, a file in the editor.
    fn open_cursor(&mut self, out: &mut Vec<Do>) {
        let folder = self.focused();
        let Some(path) = folder.cursor.clone().filter(|c| folder.selected.contains(c)) else { return };
        if path.is_dir() {
            self.focused_mut().go(path);
        } else {
            out.push(Do::Open(path));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, project: Option<&Path>, can_paste: bool, out: &mut Vec<Do>) {
        let now = ui.input(|i| i.time);
        for pane in &mut self.panes {
            pane.folder_mut().relist(now);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(RELIST_SECS));
        let pal = theme::colors();
        egui::Frame::new().fill(pal.editor_bg).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.pick_bar(ui, out);
            self.toolbar(ui, can_paste, out);
            self.ask_bar(ui, out);
            ui.add_space(4.0);
            let room = ui.available_rect_before_wrap();
            // When room is short the preview narrows, then it and the places step aside for the panes.
            let places_width = if room.width() >= 640.0 { PLACES_WIDTH } else { 0.0 };
            let preview_width = match room.width() {
                w if w >= 980.0 => PREVIEW_WIDTH,
                w if w >= 820.0 => 200.0,
                _ => 0.0,
            };
            let places = egui::Rect::from_min_size(room.min, egui::vec2(places_width, room.height()));
            let preview = egui::Rect::from_min_max(egui::pos2(room.right() - preview_width, room.top()), room.max);
            let gap = |w: f32| if w > 0.0 { 8.0 } else { 0.0 };
            let panes = egui::Rect::from_min_max(
                egui::pos2(places.right() + gap(places_width), room.top()),
                egui::pos2(preview.left() - gap(preview_width), room.bottom()),
            );
            if places_width > 0.0 {
                ui.scope_builder(egui::UiBuilder::new().max_rect(places), |ui| self.places(ui, project));
            }
            if preview_width > 0.0 {
                ui.scope_builder(egui::UiBuilder::new().max_rect(preview), |ui| self.preview_ui(ui));
            }
            if self.dual {
                let half = (panes.width() - 8.0) / 2.0;
                let left = egui::Rect::from_min_size(panes.min, egui::vec2(half, panes.height()));
                let right = egui::Rect::from_min_max(egui::pos2(left.right() + 8.0, panes.top()), panes.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| self.pane_ui(ui, 0, out));
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| self.pane_ui(ui, 1, out));
            } else {
                self.focus = 0;
                ui.scope_builder(egui::UiBuilder::new().max_rect(panes), |ui| self.pane_ui(ui, 0, out));
            }
            ui.allocate_rect(room, Sense::hover());
        });
    }

    /// While Open File or Open Folder is choosing: what will open, and the button that does it.
    fn pick_bar(&mut self, ui: &mut egui::Ui, out: &mut Vec<Do>) {
        let Some(pick) = self.pick else { return };
        let pal = theme::colors();
        let folder = self.focused();
        let picked = folder.picked();
        egui::Frame::new()
            .fill(pal.accent.gamma_multiply(0.15))
            .stroke(egui::Stroke::new(1.0, pal.accent))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let open = match pick {
                        PickFor::Folder => {
                            // The one folder picked, else the folder showing.
                            let target = match picked.as_slice() {
                                [one] if one.is_dir() => one.clone(),
                                _ => folder.dir.clone(),
                            };
                            ui.label("Choose a folder to open as the project.");
                            let label = format!("Open \"{}\"", name_of(&target));
                            ui.button(RichText::new(label).strong()).clicked().then_some(Do::OpenProject(target))
                        }
                        PickFor::Files => {
                            let files: Vec<PathBuf> = picked.iter().filter(|p| !p.is_dir()).cloned().collect();
                            ui.label("Choose files to open (double-click one, or pick several).");
                            let label = match files.as_slice() {
                                [] => "Open".to_owned(),
                                [one] => format!("Open \"{}\"", name_of(one)),
                                many => format!("Open {} files", many.len()),
                            };
                            let button = egui::Button::new(RichText::new(label).strong());
                            ui.add_enabled(!files.is_empty(), button).clicked().then_some(Do::OpenFiles(files))
                        }
                    };
                    if let Some(open) = open {
                        out.push(open);
                    }
                    if ui.button("Cancel").on_hover_text("Escape").clicked() {
                        out.push(Do::EndPick);
                    }
                });
            });
        ui.add_space(4.0);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, can_paste: bool, out: &mut Vec<Do>) {
        let picked = self.focused().picked();
        let one = picked.len() == 1;
        let any = !picked.is_empty();
        let to_other = self.dual && any;
        ui.horizontal(|ui| {
            if ui.button("New folder").on_hover_text("F7").clicked() {
                self.ask = Some(Ask::NewFolder(self.focused().dir.clone(), String::new()));
            }
            if ui.add_enabled(one, egui::Button::new("Rename")).on_hover_text("F2").clicked() {
                self.rename_picked();
            }
            if ui.add_enabled(to_other, egui::Button::new("Copy to other pane")).on_hover_text("F5").clicked() {
                self.send_to_other(out, false);
            }
            if ui.add_enabled(to_other, egui::Button::new("Move to other pane")).on_hover_text("F6").clicked() {
                self.send_to_other(out, true);
            }
            if ui.add_enabled(any, egui::Button::new("Delete")).on_hover_text("Delete; Shift+Delete for good").clicked()
            {
                self.ask_delete(false);
            }
            ui.separator();
            if ui.add_enabled(any, egui::Button::new("Copy")).on_hover_text("Ctrl+C").clicked() {
                out.push(Do::Clip(picked.clone(), false));
            }
            if ui.add_enabled(any, egui::Button::new("Cut")).on_hover_text("Ctrl+X").clicked() {
                out.push(Do::Clip(picked.clone(), true));
            }
            if ui.add_enabled(can_paste, egui::Button::new("Paste")).on_hover_text("Ctrl+V").clicked() {
                out.push(Do::Paste(self.focused().dir.clone()));
            }
            ui.separator();
            ui.toggle_value(&mut self.dual, "Two panes").on_hover_text("Tab moves between them");
            ui.toggle_value(&mut self.show_hidden, "Hidden files");
            let here = self.focused().dir.clone();
            let marked = self.bookmarks.contains(&here);
            if ui.selectable_label(marked, "Bookmark").on_hover_text("Keep this folder under Bookmarks").clicked() {
                if marked {
                    self.bookmarks.retain(|b| *b != here);
                } else {
                    self.bookmarks.push(here);
                }
            }
        });
    }

    fn ask_bar(&mut self, ui: &mut egui::Ui, out: &mut Vec<Do>) {
        let Some(ask) = &mut self.ask else { return };
        let pal = theme::colors();
        let mut done: Option<bool> = None;
        egui::Frame::new().fill(pal.hover_bg).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 6)).show(
            ui,
            |ui| {
                ui.horizontal(|ui| match ask {
                    Ask::Delete(paths, permanent) => {
                        let what = match paths.as_slice() {
                            [one] => format!("\"{}\"", name_of(one)),
                            many => format!("these {}", count(many.len())),
                        };
                        if *permanent {
                            ui.label(format!("Delete {what} for good? This can't be undone."));
                        } else {
                            ui.label(format!("Move {what} to the Recycle Bin?"));
                        }
                        let yes = if *permanent { "Delete" } else { "Move to Recycle Bin" };
                        if ui.button(yes).clicked() {
                            done = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            done = Some(false);
                        }
                    }
                    Ask::NewFolder(_, name) | Ask::Rename(_, name) => {
                        let edit = ui.add(egui::TextEdit::singleline(name).hint_text("Name").desired_width(260.0));
                        if !edit.has_focus() && !edit.lost_focus() {
                            edit.request_focus();
                        }
                        if edit.lost_focus() {
                            let enter = ui.input(|i| i.key_pressed(Key::Enter));
                            done = Some(enter);
                        }
                        if ui.button("OK").clicked() {
                            done = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            done = Some(false);
                        }
                    }
                });
            },
        );
        match (done, self.ask.take()) {
            (Some(true), Some(Ask::Delete(paths, permanent))) => out.push(Do::Delete(paths, permanent)),
            (Some(true), Some(Ask::NewFolder(dir, name))) if !name.trim().is_empty() => {
                out.push(Do::NewFolder(dir, name.trim().to_owned()));
            }
            (Some(true), Some(Ask::Rename(path, name))) if !name.trim().is_empty() => {
                out.push(Do::Rename(path, name.trim().to_owned()));
            }
            (Some(_), _) => {}
            (None, ask) => self.ask = ask,
        }
    }

    fn places(&mut self, ui: &mut egui::Ui, project: Option<&Path>) {
        let pal = theme::colors();
        let mut go = None;
        let mut unmark = None;
        egui::ScrollArea::vertical().id_salt("fm_places").auto_shrink([false, false]).show(ui, |ui| {
            let heading = |ui: &mut egui::Ui, text: &str| {
                ui.add_space(4.0);
                ui.label(RichText::new(text).small().strong().color(pal.text_dim));
            };
            let here = self.focused().dir.clone();
            let mut place = |ui: &mut egui::Ui, name: &str, dir: &Path, icon: Icon| {
                let current = here == dir;
                let resp = ui
                    .horizontal(|ui| {
                        icons::show(ui, icon, 14.0, pal.text_dim);
                        ui.selectable_label(current, name)
                    })
                    .inner
                    .on_hover_text(dir.display().to_string());
                if resp.clicked() {
                    go = Some(dir.to_path_buf());
                }
                resp
            };
            if let Some(root) = project {
                heading(ui, "PROJECT");
                place(ui, &name_of(root), root, Icon::FolderOpen);
            }
            heading(ui, "PLACES");
            for (name, dir) in dir_listing::places() {
                place(ui, &name, &dir, Icon::Folder);
            }
            heading(ui, "BOOKMARKS");
            if self.bookmarks.is_empty() {
                ui.label(RichText::new("None yet: Bookmark keeps the folder showing.").small().color(pal.text_dim));
            }
            for dir in self.bookmarks.clone() {
                let resp = place(ui, &name_of(&dir), &dir, Icon::Pin);
                resp.context_menu(|ui| {
                    if ui.button("Remove bookmark").clicked() {
                        unmark = Some(dir.clone());
                        ui.close();
                    }
                });
            }
        });
        if let Some(dir) = go {
            self.focused_mut().go(dir);
        }
        if let Some(dir) = unmark {
            self.bookmarks.retain(|b| *b != dir);
        }
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, which: usize, out: &mut Vec<Do>) {
        let pal = theme::colors();
        let focused = self.focus == which && self.dual;
        let rect = ui.max_rect();
        if ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))) {
            self.focus = which;
        }
        let stroke = if focused { pal.accent } else { pal.border };
        ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        let inner = rect.shrink(6.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            self.tabs_row(ui, which);
            self.nav_row(ui, which, out);
            self.list(ui, which, out);
        });
    }

    fn tabs_row(&mut self, ui: &mut egui::Ui, which: usize) {
        let pane = &mut self.panes[which];
        let mut close = None;
        let mut pick = None;
        ui.horizontal(|ui| {
            for (i, tab) in pane.tabs.iter().enumerate() {
                let resp = ui.selectable_label(i == pane.tab, tab.title()).on_hover_text(tab.dir.display().to_string());
                if resp.clicked() {
                    pick = Some(i);
                }
                if resp.middle_clicked() {
                    close = Some(i);
                }
                resp.context_menu(|ui| {
                    if ui.add_enabled(pane.tabs.len() > 1, egui::Button::new("Close tab")).clicked() {
                        close = Some(i);
                        ui.close();
                    }
                });
            }
            if ui.small_button("+").on_hover_text("New tab (Ctrl+T)").clicked() {
                pick = Some(usize::MAX);
            }
        });
        match pick {
            Some(usize::MAX) => pane.new_tab(),
            Some(i) => pane.tab = i,
            None => {}
        }
        if let Some(i) = close {
            pane.close_tab(i);
        }
    }

    fn nav_row(&mut self, ui: &mut egui::Ui, which: usize, out: &mut Vec<Do>) {
        let folder = self.panes[which].folder_mut();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!folder.back.is_empty(), egui::Button::new("←"))
                .on_hover_text("Back (Alt+Left)")
                .clicked()
            {
                folder.go_back();
            }
            if ui
                .add_enabled(!folder.forward.is_empty(), egui::Button::new("→"))
                .on_hover_text("Forward (Alt+Right)")
                .clicked()
            {
                folder.go_forward();
            }
            if ui
                .add_enabled(folder.dir.parent().is_some(), egui::Button::new("↑"))
                .on_hover_text("Up (Backspace)")
                .clicked()
            {
                folder.go_up();
            }
            let filter_width = 140.0_f32.min(ui.available_width() * 0.35);
            let path_width = (ui.available_width() - filter_width - 8.0).max(60.0);
            let path = ui.add(egui::TextEdit::singleline(&mut folder.path_text).desired_width(path_width));
            if path.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    let typed = PathBuf::from(folder.path_text.trim());
                    if typed.is_dir() {
                        folder.go(typed);
                    } else {
                        out.push(Do::Note(format!("No folder at {}", typed.display())));
                        folder.path_text = folder.dir.display().to_string();
                    }
                } else {
                    folder.path_text = folder.dir.display().to_string();
                }
            }
            ui.add(
                egui::TextEdit::singleline(&mut folder.filter).hint_text("Filter, or *.rs").desired_width(filter_width),
            );
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, which: usize, out: &mut Vec<Do>) {
        let pal = theme::colors();
        let hidden = self.show_hidden;
        let folder = self.panes[which].folder_mut();
        if let Some(e) = &folder.error {
            ui.label(RichText::new(format!("Can't read this folder: {e}")).color(pal.text_dim));
            return;
        }
        let width = ui.available_width();
        // Columns beside the name, as many as fit: Type goes first, then Modified, then Size.
        let mut columns = Vec::new();
        let mut used = NAME_MIN;
        for (by, label, w) in [
            (SortBy::Size, "Size", SIZE_WIDTH),
            (SortBy::Modified, "Modified", DATE_WIDTH),
            (SortBy::Kind, "Type", KIND_WIDTH),
        ] {
            if used + w <= width {
                used += w;
                columns.push((by, label, w));
            }
        }
        let name_width = width - columns.iter().map(|c| c.2).sum::<f32>();
        // Headers: a click sorts by the column, again reverses.
        let mut sort = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (by, label, w) in std::iter::once((SortBy::Name, "Name", name_width)).chain(columns.iter().copied()) {
                let arrow = if folder.sort == by { if folder.descending { " ▼" } else { " ▲" } } else { "" };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, ROW), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, pal.hover_bg);
                }
                ui.painter().text(
                    rect.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{label}{arrow}"),
                    egui::FontId::proportional(12.5),
                    pal.text_dim,
                );
                if resp.clicked() {
                    sort = Some(by);
                }
            }
        });
        if let Some(by) = sort {
            folder.descending = folder.sort == by && !folder.descending;
            folder.sort = by;
            folder.resort();
        }
        ui.separator();
        let shown: Vec<Entry> = folder.shown(hidden).into_iter().cloned().collect();
        let dir = folder.dir.clone();
        let mods = ui.input(|i| i.modifiers);
        let mut clicked = None;
        let mut opened = None;
        let mut dropped: Option<(Vec<PathBuf>, PathBuf)> = None;
        let scroll_to = folder.scroll_to_cursor.then(|| folder.cursor.clone()).flatten();
        folder.scroll_to_cursor = false;
        let font = egui::FontId::proportional(13.0);
        let area = egui::ScrollArea::vertical().id_salt(("fm_list", which)).auto_shrink([false, false]);
        let list = area.show(ui, |ui| {
            if shown.is_empty() {
                let note =
                    if folder.entries.is_empty() { "This folder is empty." } else { "Nothing matches the filter." };
                ui.label(RichText::new(note).color(pal.text_dim));
            }
            for entry in &shown {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW), Sense::click_and_drag());
                if scroll_to.as_ref() == Some(&entry.path) {
                    ui.scroll_to_rect(rect, None);
                }
                let picked = folder.selected.contains(&entry.path);
                let painter = ui.painter();
                if picked {
                    painter.rect_filled(rect, 4.0, pal.accent.gamma_multiply(0.25));
                } else if resp.hovered() {
                    painter.rect_filled(rect, 4.0, pal.hover_bg);
                }
                let icon =
                    egui::Rect::from_center_size(rect.left_center() + egui::vec2(14.0, 0.0), egui::vec2(15.0, 15.0));
                if entry.is_dir {
                    icons::paint(painter, icon, Icon::Folder, pal.text_dim);
                } else {
                    icons::paint_file(painter, icon, &entry.name);
                }
                let color = if entry.hidden { pal.text_dim } else { pal.text };
                let name_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 28.0, rect.top()),
                    egui::pos2(rect.left() + name_width - 4.0, rect.bottom()),
                );
                painter.with_clip_rect(name_rect).text(
                    name_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    &entry.name,
                    font.clone(),
                    color,
                );
                let mut x = rect.left() + name_width;
                let size = if entry.is_dir { String::new() } else { dir_listing::human_size(entry.size) };
                let date = entry.modified.map(dir_listing::date_text).unwrap_or_default();
                for (by, _, w) in &columns {
                    let (text, w, right) = match by {
                        SortBy::Size => (size.clone(), *w, true),
                        SortBy::Modified => (date.clone(), *w, false),
                        _ => (dir_listing::kind(entry), *w, false),
                    };
                    let (pos, align) = if right {
                        (egui::pos2(x + w - 12.0, rect.center().y), egui::Align2::RIGHT_CENTER)
                    } else {
                        (egui::pos2(x + 6.0, rect.center().y), egui::Align2::LEFT_CENTER)
                    };
                    painter.text(pos, align, text, font.clone(), pal.text_dim);
                    x += w;
                }
                // Dragging carries the picked entries, or this one alone if it isn't picked.
                let carried = if picked { folder.picked() } else { vec![entry.path.clone()] };
                resp.dnd_set_drag_payload(DraggedFiles(carried));
                if entry.is_dir {
                    if resp.dnd_hover_payload::<DraggedFiles>().is_some() {
                        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                    }
                    if let Some(files) = resp.dnd_release_payload::<DraggedFiles>() {
                        dropped = Some((files.0.clone(), entry.path.clone()));
                    }
                }
                if resp.clicked() {
                    clicked = Some(entry.path.clone());
                }
                if resp.double_clicked() {
                    opened = Some(entry.clone());
                }
                resp.context_menu(|ui| {
                    if !folder.selected.contains(&entry.path) {
                        folder.pick_only(&entry.path);
                    }
                    let picked = folder.picked();
                    if ui.button(if entry.is_dir { "Open" } else { "Open in editor" }).clicked() {
                        opened = Some(entry.clone());
                        ui.close();
                    }
                    if ui.button("Copy").clicked() {
                        out.push(Do::Clip(picked.clone(), false));
                        ui.close();
                    }
                    if ui.button("Cut").clicked() {
                        out.push(Do::Clip(picked.clone(), true));
                        ui.close();
                    }
                    if ui.add_enabled(picked.len() == 1, egui::Button::new("Rename")).clicked() {
                        self.ask = Some(Ask::Rename(entry.path.clone(), entry.name.clone()));
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        self.ask = Some(Ask::Delete(picked.clone(), false));
                        ui.close();
                    }
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(entry.path.display().to_string());
                        ui.close();
                    }
                });
            }
        });
        // Let go on the list itself (not on a folder in it): into this pane's folder.
        let background = ui.interact(list.inner_rect, egui::Id::unique(("fm_drop", which)), Sense::hover());
        if dropped.is_none()
            && let Some(files) = background.dnd_release_payload::<DraggedFiles>()
        {
            dropped = Some((files.0.clone(), dir.clone()));
        }
        if let Some(path) = clicked {
            folder.click(&path, mods, hidden);
        }
        if let Some(entry) = opened {
            folder.pick_only(&entry.path);
            if entry.is_dir {
                folder.go(entry.path);
            } else {
                out.push(Do::Open(entry.path));
            }
        }
        if let Some((files, to)) = dropped {
            let files: Vec<PathBuf> = files.into_iter().filter(|f| *f != to).collect();
            if !files.is_empty() {
                // Ctrl held copies, as desktop file managers do; otherwise it moves.
                out.push(if mods.command { Do::Copy(files, to) } else { Do::Move(files, to) });
            }
        }
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        // The pane's fields only, so the preview's cache can change meanwhile.
        let hidden = self.show_hidden;
        let folder = self.panes[self.focus].folder();
        let picked: Vec<&Entry> = folder.entries.iter().filter(|e| folder.selected.contains(&e.path)).collect();
        egui::ScrollArea::vertical().id_salt("fm_preview").auto_shrink([false, false]).show(ui, |ui| {
            match picked.as_slice() {
                [] => {
                    ui.label(RichText::new(folder.title()).strong());
                    let shown = folder.shown(hidden);
                    let files = shown.iter().filter(|e| !e.is_dir).count();
                    let dirs = shown.len() - files;
                    let bytes: u64 = shown.iter().map(|e| e.size).sum();
                    ui.label(
                        RichText::new(format!(
                            "{}, {}, {}",
                            plural(dirs, "folder"),
                            plural(files, "file"),
                            dir_listing::human_size(bytes)
                        ))
                        .color(pal.text_dim),
                    );
                }
                [entry] => {
                    let entry = (*entry).clone();
                    let fresh = self.preview.as_ref().is_some_and(|(p, m, _)| *p == entry.path && *m == entry.modified);
                    if !fresh {
                        self.preview = Some((entry.path.clone(), entry.modified, preview_of(&entry)));
                    }
                    ui.label(RichText::new(&entry.name).strong());
                    let row = |ui: &mut egui::Ui, key: &str, value: String| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(key).color(pal.text_dim));
                            ui.label(value);
                        });
                    };
                    row(ui, "Type", dir_listing::kind(&entry));
                    if !entry.is_dir {
                        row(ui, "Size", format!("{} ({} bytes)", dir_listing::human_size(entry.size), entry.size));
                    }
                    if let Some(m) = entry.modified {
                        row(ui, "Modified", dir_listing::date_text(m));
                    }
                    if entry.readonly {
                        row(ui, "Access", "read-only".to_owned());
                    }
                    ui.label(RichText::new(entry.path.display().to_string()).small().color(pal.text_dim));
                    ui.separator();
                    match self.preview.as_ref().map(|(.., p)| p) {
                        Some(Preview::Text(text)) => {
                            ui.label(RichText::new(text).monospace().size(11.5));
                        }
                        Some(Preview::Picture(w, h)) => {
                            ui.label(format!("Picture, {w} × {h} pixels"));
                        }
                        Some(Preview::Folder(n)) => {
                            ui.label(format!("{} inside", count(*n)));
                        }
                        Some(Preview::Binary) => {
                            ui.label(RichText::new("Not text: no preview").color(pal.text_dim));
                        }
                        Some(Preview::TooLarge) => {
                            ui.label(RichText::new("Too large to preview").color(pal.text_dim));
                        }
                        Some(Preview::Unreadable(e)) => {
                            ui.label(RichText::new(format!("Can't read it: {e}")).color(pal.text_dim));
                        }
                        None => {}
                    }
                }
                many => {
                    let bytes: u64 = many.iter().map(|e| e.size).sum();
                    ui.label(RichText::new(format!("{} picked", count(many.len()))).strong());
                    ui.label(RichText::new(dir_listing::human_size(bytes)).color(pal.text_dim));
                }
            }
        });
    }
}

impl CodeApp {
    /// Opens the File Manager, at `dir` when given (else where it was, the project, or home).
    pub(crate) fn open_file_manager(&mut self, dir: Option<PathBuf>) {
        self.file_manager.to_open = Some((dir, None));
    }

    /// Open File and Open Folder: the File Manager chooses, starting where that last chose.
    pub(crate) fn pick_with_file_manager(&mut self, pick: PickFor) {
        let kind = if pick == PickFor::Folder { "open_folder" } else { "open_file" };
        let dir = self.dialog_dirs.get(kind).map(Path::to_path_buf).or_else(|| self.explorer.root.clone());
        self.file_manager.to_open = Some((dir, Some(pick)));
    }

    /// Picking is over: the File Manager goes if it came for that alone.
    fn end_pick(&mut self) {
        let closes = self.file_manager.view.as_ref().is_some_and(|v| v.pick_closes);
        if closes {
            self.close_file_manager_tab();
        } else if let Some(view) = &mut self.file_manager.view {
            view.pick = None;
        }
    }

    pub(crate) fn file_manager_tab(&self) -> Option<(String, bool)> {
        let view = self.file_manager.view.as_ref()?;
        Some((format!("Files: {}", view.focused().title()), view.showing))
    }

    pub(crate) fn hide_file_manager(&mut self) {
        if let Some(view) = &mut self.file_manager.view {
            view.showing = false;
        }
    }

    pub(crate) fn show_file_manager_tab(&mut self) {
        let over = self.docs.get(self.active).map(|d| d.uid);
        if let Some(view) = &mut self.file_manager.view {
            view.showing = true;
            view.over = over;
        }
    }

    pub(crate) fn close_file_manager_tab(&mut self) {
        self.file_manager.view = None;
    }

    /// The keys, taken before the shortcuts when the view is in front and no text box or the
    /// terminal has the keyboard. Says whether it took any.
    pub(crate) fn file_manager_keys(&mut self, ctx: &egui::Context) -> bool {
        let Some(view) = self.file_manager.view.as_mut().filter(|v| v.showing) else { return false };
        if ctx.egui_wants_keyboard_input() || self.terminal.has_focus(ctx) {
            return false;
        }
        let before = ctx.input(|i| i.events.len());
        let mut out = Vec::new();
        view.keys(ctx, &mut out);
        let took = ctx.input(|i| i.events.len()) != before || !out.is_empty();
        self.file_manager_do(ctx, out);
        took
    }

    /// Shows the File Manager in place of the editor. Says whether it did.
    pub(crate) fn file_manager_view(&mut self, ui: &mut egui::Ui) -> bool {
        let active = self.docs.get(self.active).map(|d| d.uid);
        if let Some((dir, pick)) = self.file_manager.to_open.take() {
            let was_showing = self.file_manager.view.as_ref().is_some_and(|v| v.showing && v.over == active);
            self.hide_logs();
            self.hide_bytes();
            self.hide_image();
            let dir = dir.filter(|d| d.is_dir());
            match &mut self.file_manager.view {
                Some(view) => {
                    if let Some(dir) = dir {
                        view.focused_mut().go(dir);
                    }
                }
                None => {
                    let fallback = self
                        .explorer
                        .root
                        .clone()
                        .or_else(|| dir_listing::places().first().map(|p| p.1.clone()))
                        .unwrap_or_else(|| PathBuf::from("."));
                    self.file_manager.view = Some(View::new(Saved::load(), dir, fallback));
                }
            }
            if let Some(view) = &mut self.file_manager.view {
                view.showing = true;
                view.over = active;
                view.pick = pick;
                view.pick_closes = pick.is_some() && !was_showing;
            }
        }
        if let Some(view) = self.file_manager.view.as_mut().filter(|v| v.over != active) {
            view.showing = false;
        }
        let Some(mut view) = self.file_manager.view.take() else { return false };
        if !view.showing {
            self.file_manager.view = Some(view);
            return false;
        }
        let mut out = Vec::new();
        let project = self.explorer.root.clone();
        view.ui(ui, project.as_deref(), self.explorer.clipboard.is_some(), &mut out);
        let saved = view.saved();
        if saved != view.stored {
            if let Err(e) = saved.store() {
                self.status = Some(format!("Could not save the File Manager's bookmarks: {e}"));
            }
            view.stored = saved;
        }
        self.file_manager.view = Some(view);
        self.file_manager_do(ui.ctx(), out);
        true
    }

    fn file_manager_do(&mut self, ctx: &egui::Context, out: Vec<Do>) {
        if out.is_empty() {
            return;
        }
        let picking_files = self.file_manager.view.as_ref().is_some_and(|v| v.pick == Some(PickFor::Files));
        for action in out {
            match action {
                // Choosing for Open File: a file opened is the choice.
                Do::Open(path) if picking_files => {
                    self.dialog_chose("open_file", &path);
                    self.end_pick();
                    self.open_file(&path);
                }
                Do::Open(path) => self.open_file(&path),
                Do::OpenProject(dir) => {
                    self.dialog_chose("open_folder", &dir);
                    self.end_pick();
                    self.open_folder(dir);
                }
                Do::OpenFiles(files) => {
                    if let Some(first) = files.first() {
                        self.dialog_chose("open_file", first);
                    }
                    self.end_pick();
                    for file in &files {
                        self.open_file(file);
                    }
                }
                Do::EndPick => self.end_pick(),
                Do::Move(files, dir) => self.move_into(&files, &dir),
                Do::Copy(files, dir) => {
                    let failed = Self::copy_into_paths(&files, &dir);
                    self.explorer.refresh();
                    self.finder.mark_stale();
                    self.status = Some(if failed.is_empty() {
                        format!("Copied {} to {}", count(files.len()), name_of(&dir))
                    } else {
                        format!("Could not copy {}", failed.join("; "))
                    });
                }
                Do::Delete(paths, permanent) => self.start_delete(ctx, paths, permanent),
                Do::Rename(path, name) => {
                    self.path_target = Some(path);
                    self.finish_rename_path(&name);
                }
                Do::NewFolder(dir, name) => {
                    self.path_target = Some(dir);
                    self.finish_new_path(&name, true);
                }
                Do::Clip(files, cut) => {
                    let what = count(files.len());
                    self.explorer.clipboard = Some((files, cut));
                    self.status = Some(format!(
                        "{} {what}: paste in a folder here or in the Explorer",
                        if cut { "Cut" } else { "Copied" }
                    ));
                }
                Do::Paste(dir) => self.paste_into(&dir),
                Do::Note(text) => self.status = Some(text),
            }
        }
        if let Some(view) = &mut self.file_manager.view {
            view.relist_all();
        }
    }
}
