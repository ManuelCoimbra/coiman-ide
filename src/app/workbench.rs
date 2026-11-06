//! Workbench UI around the editor: menus, side bar, tabs, find bar, status bar and dialogs.

use crate::commands::Command;
use crate::document::{DiskState, Document};
use crate::explorer::ExplorerAction;
use crate::icons::{self, Icon};
use crate::terminal::Action as TerminalAction;
use crate::theme;
use egui::{Align, Color32, Key, RichText, Sense, Stroke};
use rust_code_core::find;
use rust_code_core::project_tree::{self, FileRole};

use super::{CodeApp, FindCache, PanelTab, PendingClose, SidebarView};

fn font_for_target(ui: &egui::Ui) -> egui::FontId {
    egui::FontId::proportional(small_size(ui))
}

impl CodeApp {
    /// Bar above the editor when the file changed or vanished on disk.
    pub(super) fn disk_conflict_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let message = match doc.disk {
            DiskState::InSync => return,
            DiskState::Changed => "This file was changed on disk, and you have unsaved changes.",
            DiskState::Deleted => "This file was deleted on disk. Save to recreate it.",
        };
        let deleted = doc.disk == DiskState::Deleted;
        let mut action = None;
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(message).color(theme::colors().modified));
                if !deleted && ui.button("Reload (discard my changes)").clicked() {
                    action = Some(true);
                }
                if ui.button(if deleted { "Close" } else { "Keep mine" }).clicked() {
                    action = Some(false);
                }
            });
        });
        match (action, deleted) {
            (Some(true), _) => {
                let doc = &mut self.docs[self.active];
                if let Err(e) = doc.reload_from_disk() {
                    self.status = Some(format!("Reload failed: {e}"));
                }
            }
            (Some(false), true) => self.close_tab(self.active),
            // Keep mine: saving will overwrite the file on disk.
            (Some(false), false) => self.docs[self.active].disk = DiskState::InSync,
            (None, _) => {}
        }
    }

    /// The window's title bar: menus, the title, and minimize/maximize/close.
    /// Dragging it moves the window; double-clicking maximizes or restores.
    pub(super) fn title_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        if theme::islands() {
            return self.top_bar(ui);
        }
        const HEIGHT: f32 = 30.0;
        const RUN_BUTTON: f32 = 40.0;
        let ctx = ui.ctx().clone();
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let bar = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(ui.max_rect().width(), HEIGHT));
        // The background first, so the menus and buttons drawn on top get their clicks.
        let drag = ui.interact(bar, egui::Id::unique("title_bar_drag"), Sense::click_and_drag());
        if drag.double_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        ui.set_min_height(HEIGHT);

        let mut cmd = None;
        let menus_right = ui
            .scope_builder(egui::UiBuilder::new().max_rect(bar.with_max_x(bar.max.x - 3.0 * WINDOW_BUTTON)), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    let logo = egui::Rect::from_center_size(
                        ui.cursor().min + egui::vec2(8.0, HEIGHT / 2.0),
                        egui::vec2(16.0, 16.0),
                    );
                    paint_logo(ui.painter(), logo);
                    ui.add_space(24.0);
                    let (c, end) = self.menu_bar(ui);
                    cmd = c;
                    end
                })
                .inner
            })
            .inner;

        // The title, centered in the window but never over the menus or buttons.
        let pal = theme::colors();
        let font = egui::FontId::proportional(small_size(ui));
        // The UI font has no "●" (unsaved marker in the OS title); use a bullet.
        let title = self.last_title.replace('●', "•");
        let galley = ui.painter().layout_no_wrap(title, font, pal.text_dim);
        let target = self.run_target_label();
        let target_galley = target.map(|t| ui.painter().layout_no_wrap(t, font_for_target(ui), pal.text_dim));
        let target_width = target_galley.as_ref().map_or(0.0, |g| g.size().x.min(180.0) + 28.0);
        let room =
            egui::Rangef::new(menus_right + 16.0, bar.max.x - 3.0 * WINDOW_BUTTON - RUN_BUTTON - target_width - 16.0);
        if room.span() > 40.0 {
            let width = galley.size().x.min(room.span());
            let x = (bar.center().x - width / 2.0).clamp(room.min, room.max - width);
            let clip = egui::Rect::from_x_y_ranges(room, bar.y_range());
            ui.painter().with_clip_rect(clip).galley(
                egui::pos2(x, bar.center().y - galley.size().y / 2.0),
                galley,
                pal.text_dim,
            );
        }

        // Run Program, just left of the window buttons.
        let run_rect = egui::Rect::from_min_size(
            egui::pos2(bar.max.x - 3.0 * WINDOW_BUTTON - RUN_BUTTON, bar.min.y),
            egui::vec2(RUN_BUTTON, HEIGHT),
        );
        // What Run starts, just left of it.
        if let Some(galley) = target_galley {
            let rect = egui::Rect::from_min_size(
                egui::pos2(run_rect.min.x - target_width, bar.min.y),
                egui::vec2(target_width, HEIGHT),
            );
            let choose = ui.interact(rect, egui::Id::unique("run_target"), Sense::click());
            if choose.hovered() {
                ui.painter().rect_filled(rect, 0.0, pal.hover_bg);
            }
            let text = egui::pos2(rect.min.x + 8.0, rect.center().y - galley.size().y / 2.0);
            let clip = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - 18.0, rect.max.y));
            ui.painter().with_clip_rect(clip).galley(text, galley, pal.text_dim);
            let (x, y) = (rect.max.x - 13.0, rect.center().y);
            ui.painter().add(egui::Shape::convex_polygon(
                vec![egui::pos2(x - 3.0, y - 2.0), egui::pos2(x + 3.0, y - 2.0), egui::pos2(x, y + 2.0)],
                pal.text_dim,
                Stroke::NONE,
            ));
            if choose.on_hover_text("Choose what Run starts").clicked() {
                cmd = Some(Command::ChooseRunTarget);
            }
        }
        if self.run_button_visible() {
            let run = ui.interact(run_rect, egui::Id::unique("run_program"), Sense::click());
            if run.hovered() {
                ui.painter().rect_filled(run_rect, 0.0, pal.hover_bg);
            }
            let icon = egui::Rect::from_center_size(run_rect.center(), egui::vec2(16.0, 16.0));
            icons::paint(ui.painter(), icon, Icon::Play, Color32::from_rgb(0x89, 0xD1, 0x85));
            if run.on_hover_text(self.run_button_tip()).clicked() {
                cmd = Some(Command::RunProgram);
            }
        }

        window_buttons(ui, bar, maximized);
        cmd
    }

    /// The island layout's bar, as in JetBrains IDEs: everything's menu behind one button,
    /// the project and branch on the left, what runs and the tools on the right.
    fn top_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        const HEIGHT: f32 = 40.0;
        let ctx = ui.ctx().clone();
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let bar = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(ui.max_rect().width(), HEIGHT));
        let drag = ui.interact(bar, egui::Id::unique("title_bar_drag"), Sense::click_and_drag());
        if drag.double_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        ui.set_min_height(HEIGHT);
        let pal = theme::colors();
        let root = self.explorer.root.clone();
        let name = root
            .as_ref()
            .map(|r| r.file_name().map_or_else(|| r.display().to_string(), |n| n.to_string_lossy().into_owned()));
        let badge = name.as_deref().map(project_tree::project_badge);

        // The project's color fading out behind the left of the bar.
        if let Some((_, [r, g, b])) = badge {
            let glow = Color32::from_rgba_unmultiplied(r, g, b, 50);
            let end = (bar.min.x + 460.0).min(bar.max.x);
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(bar.left_top(), glow);
            mesh.colored_vertex(egui::pos2(end, bar.min.y), Color32::TRANSPARENT);
            mesh.colored_vertex(egui::pos2(end, bar.max.y), Color32::TRANSPARENT);
            mesh.colored_vertex(bar.left_bottom(), glow);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(mesh);
        }

        let mut cmd = None;
        let mut open_folder = None;
        let inside = bar.with_max_x(bar.max.x - 3.0 * WINDOW_BUTTON);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inside), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(8.0);
                let menu = tool_button(ui, Icon::Menu, pal.text, "Main Menu");
                egui::Popup::menu(&menu).show(|ui| cmd = self.menus(ui).0);

                let project = text_chip(ui, name.as_deref().unwrap_or("No Folder"), 30.0, |painter, rect| {
                    let (letters, [r, g, b]) = badge.clone().unwrap_or(("?".to_owned(), [0x80, 0x80, 0x80]));
                    painter.rect_filled(rect, 5.0, Color32::from_rgb(r, g, b));
                    let font = egui::FontId::proportional(11.0);
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, letters, font, Color32::WHITE);
                });
                let project =
                    project.on_hover_text(root.as_ref().map_or_else(String::new, |r| r.display().to_string()));
                egui::Popup::menu(&project).show(|ui| {
                    if ui.button("Open Folder...").clicked() {
                        cmd = Some(Command::OpenFolder);
                        ui.close();
                    }
                    let others: Vec<_> = self
                        .recent_folders
                        .iter()
                        .filter(|f| Some(*f) != root.as_ref() && f.is_dir())
                        .take(8)
                        .cloned()
                        .collect();
                    if !others.is_empty() {
                        ui.separator();
                    }
                    for folder in others {
                        let label = folder
                            .file_name()
                            .map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned());
                        if ui.button(label).on_hover_text(folder.display().to_string()).clicked() {
                            open_folder = Some(folder);
                            ui.close();
                        }
                    }
                });

                if let Some(branch) = self.git_branch_label() {
                    let chip = text_chip(ui, &branch, 22.0, |painter, rect| {
                        icons::paint(painter, rect.shrink(3.0), Icon::SourceControl, pal.text_dim);
                    });
                    if chip.on_hover_text("Switch branch").clicked() {
                        cmd = Some(Command::GitCheckout);
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(6.0);
                    if tool_button(ui, Icon::Gear, pal.text, "Settings").clicked() {
                        cmd = Some(Command::OpenUserSettings);
                    }
                    if tool_button(ui, Icon::Search, pal.text, "Search Everywhere (Double Shift)").clicked() {
                        cmd = Some(Command::SearchEverywhere);
                    }
                    ui.add_space(8.0);
                    let green = Color32::from_rgb(0x57, 0xC0, 0x6A);
                    // Run stays put whether or not this file can be debugged.
                    if !self.can_debug() {
                        ui.add_space(36.0);
                    } else if tool_button(ui, Icon::Bug, green, "Debug (F5)").clicked() {
                        cmd = Some(Command::StartDebugging);
                    }
                    if self.run_button_visible() && tool_button(ui, Icon::Play, green, &self.run_button_tip()).clicked()
                    {
                        cmd = Some(Command::RunProgram);
                    }
                    let target = self.run_target_label().unwrap_or_else(|| "Current File".to_owned());
                    if text_chip(ui, &target, 0.0, |_, _| {}).on_hover_text("Choose what Run starts").clicked() {
                        cmd = Some(Command::ChooseRunTarget);
                    }
                });
            });
        });
        if let Some(folder) = open_folder {
            self.open_folder(folder);
        }
        window_buttons(ui, bar.with_max_y(bar.min.y + HEIGHT), maximized);
        cmd
    }

    /// Resizes the borderless window from its edges and corners.
    pub(super) fn window_resize_edges(&mut self, ctx: &egui::Context) {
        use egui::viewport::ResizeDirection as Dir;
        const EDGE: f32 = 5.0;
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if maximized || fullscreen {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let r = ctx.content_rect();
        let (left, right) = (pos.x - r.min.x < EDGE, r.max.x - pos.x < EDGE);
        let (top, bottom) = (pos.y - r.min.y < EDGE, r.max.y - pos.y < EDGE);
        let dir = match (left, right, top, bottom) {
            (true, _, true, _) => Dir::NorthWest,
            (_, true, true, _) => Dir::NorthEast,
            (true, _, _, true) => Dir::SouthWest,
            (_, true, _, true) => Dir::SouthEast,
            (true, ..) => Dir::West,
            (_, true, ..) => Dir::East,
            (_, _, true, _) => Dir::North,
            (.., true) => Dir::South,
            _ => return,
        };
        let icon = match dir {
            Dir::North | Dir::South => egui::CursorIcon::ResizeVertical,
            Dir::East | Dir::West => egui::CursorIcon::ResizeHorizontal,
            Dir::NorthWest | Dir::SouthEast => egui::CursorIcon::ResizeNwSe,
            Dir::NorthEast | Dir::SouthWest => egui::CursorIcon::ResizeNeSw,
        };
        ctx.set_cursor_icon(icon);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }

    /// The menus in a row; also returns where the last one ends.
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) -> (Option<Command>, f32) {
        let mut out = (None, 0.0);
        egui::MenuBar::new().ui(ui, |ui| out = self.menus(ui));
        out
    }

    /// Each menu, in whatever `ui` lays them out.
    fn menus(&mut self, ui: &mut egui::Ui) -> (Option<Command>, f32) {
        let mut cmd = None;
        let end;
        let keymap = &self.keymap;
        let cargo_project = self.cargo_project;
        let markdown = self.active_is_markdown();
        {
            let mut item = |ui: &mut egui::Ui, label: &str, c: Command| {
                if ui.add(egui::Button::new(label).shortcut_text(keymap.shortcut_text(c))).clicked() {
                    cmd = Some(c);
                }
            };
            ui.menu_button("File", |ui| {
                item(ui, "New Text File", Command::NewFile);
                ui.separator();
                item(ui, "Open File...", Command::OpenFile);
                item(ui, "Open Folder...", Command::OpenFolder);
                item(ui, "Open Recent...", Command::OpenRecent);
                ui.separator();
                item(ui, "Save", Command::Save);
                item(ui, "Save As...", Command::SaveAs);
                item(ui, "Save All", Command::SaveAll);
                ui.separator();
                ui.menu_button("Preferences", |ui| {
                    item(ui, "Settings", Command::OpenUserSettings);
                    item(ui, "Workspace Settings", Command::OpenWorkspaceSettings);
                    item(ui, "Keyboard Shortcuts", Command::OpenKeybindings);
                    item(ui, "Snippets", Command::ManageSnippets);
                    item(ui, "Color Theme", Command::SelectTheme);
                });
                ui.separator();
                item(ui, "Close Editor", Command::CloseTab);
                item(ui, "Reopen Closed Editor", Command::ReopenClosedEditor);
                item(ui, "Exit", Command::Quit);
            });
            ui.menu_button("Edit", |ui| {
                item(ui, "Find", Command::Find);
                item(ui, "Find in Files", Command::ShowSearch);
                ui.separator();
                item(ui, "Toggle Line Comment", Command::ToggleLineComment);
                item(ui, "Insert Snippet...", Command::InsertSnippet);
                ui.separator();
                item(ui, "Rename Symbol", Command::RenameSymbol);
                item(ui, "Quick Fix...", Command::QuickFix);
                item(ui, "Format Document", Command::FormatDocument);
            });
            ui.menu_button("Selection", |ui| {
                item(ui, "Copy Line Up", Command::CopyLinesUp);
                item(ui, "Copy Line Down", Command::CopyLinesDown);
                item(ui, "Move Line Up", Command::MoveLinesUp);
                item(ui, "Move Line Down", Command::MoveLinesDown);
                item(ui, "Duplicate Selection", Command::DuplicateSelection);
                ui.separator();
                item(ui, "Add Next Occurrence", Command::SelectNextOccurrence);
                item(ui, "Select All Occurrences", Command::SelectAllOccurrences);
                ui.separator();
                item(ui, "Sort Lines Ascending", Command::SortLinesAscending);
                item(ui, "Sort Lines Descending", Command::SortLinesDescending);
                item(ui, "Join Lines", Command::JoinLines);
                ui.separator();
                item(ui, "Expand Selection", Command::ExpandSelection);
                item(ui, "Shrink Selection", Command::ShrinkSelection);
                item(ui, "Select Function", Command::SelectFunction);
                item(ui, "Select Function Body", Command::SelectFunctionBody);
                item(ui, "Select Type", Command::SelectType);
                item(ui, "Select Argument", Command::SelectArgument);
                item(ui, "Select Comment", Command::SelectComment);
            });
            ui.menu_button("View", |ui| {
                item(ui, "Command Palette...", Command::CommandPalette);
                ui.separator();
                item(ui, "Explorer", Command::ShowExplorer);
                item(ui, "Search", Command::ShowSearch);
                item(ui, "Problems", Command::ShowProblems);
                item(ui, "Terminal", Command::ToggleTerminal);
                item(ui, "Toggle Side Bar", Command::ToggleSidebar);
                item(ui, "Zen Mode", Command::ToggleZenMode);
                ui.separator();
                item(ui, "Split Editor", Command::SplitEditor);
                item(ui, "Single Editor", Command::CloseSplit);
                item(ui, "Flip Split Layout", Command::ToggleSplitLayout);
                item(ui, "Word Wrap", Command::ToggleWordWrap);
                if markdown {
                    item(ui, "Markdown Preview", Command::MarkdownPreview);
                }
                ui.separator();
                item(ui, "Fold", Command::Fold);
                item(ui, "Unfold", Command::Unfold);
                item(ui, "Fold All", Command::FoldAll);
                item(ui, "Unfold All", Command::UnfoldAll);
                ui.separator();
                item(ui, "Zoom In", Command::ZoomIn);
                item(ui, "Zoom Out", Command::ZoomOut);
                item(ui, "Reset Zoom", Command::ResetZoom);
                item(ui, "Zoom Code In", Command::CodeZoomIn);
                item(ui, "Zoom Code Out", Command::CodeZoomOut);
                item(ui, "Reset Code Zoom", Command::ResetCodeZoom);
            });
            ui.menu_button("Go", |ui| {
                item(ui, "Search Everywhere", Command::SearchEverywhere);
                item(ui, "Go to File...", Command::QuickOpen);
                item(ui, "Go to Bracket", Command::JumpToBracket);
                item(ui, "Go to Line...", Command::GoToLine);
                item(ui, "Go to Symbol in Editor...", Command::GoToSymbol);
                item(ui, "Next Function", Command::NextFunction);
                item(ui, "Previous Function", Command::PreviousFunction);
                ui.separator();
                item(ui, "Go to Definition", Command::GoToDefinition);
                item(ui, "Go to Implementations", Command::GoToImplementation);
                item(ui, "Go to Type Definition", Command::GoToTypeDefinition);
                item(ui, "Go to Symbol in Workspace...", Command::GoToWorkspaceSymbol);
                item(ui, "Go to References", Command::FindReferences);
                item(ui, "Show Incoming Calls", Command::ShowIncomingCalls);
                item(ui, "Show Outgoing Calls", Command::ShowOutgoingCalls);
                item(ui, "Show Supertypes", Command::ShowSupertypes);
                item(ui, "Show Subtypes", Command::ShowSubtypes);
                ui.separator();
                item(ui, "Toggle Bookmark", Command::ToggleBookmark);
                item(ui, "Next Bookmark", Command::NextBookmark);
                item(ui, "Previous Bookmark", Command::PreviousBookmark);
                item(ui, "All Bookmarks...", Command::ListBookmarks);
                ui.separator();
                item(ui, "Next Problem", Command::NextProblem);
                item(ui, "Previous Problem", Command::PreviousProblem);
            });
            ui.menu_button("Terminal", |ui| {
                item(ui, "New Terminal", Command::NewTerminal);
                item(ui, "Toggle Terminal", Command::ToggleTerminal);
                ui.separator();
                item(ui, "Find in Terminal", Command::TerminalFind);
                item(ui, "Clear Terminal", Command::ClearTerminal);
                item(ui, "Kill Terminal", Command::KillTerminal);
                item(ui, "Move Terminal into New Window", Command::DetachTerminal);
            });
            let run = ui.menu_button("Run", |ui| {
                if cargo_project {
                    item(ui, "Start Debugging", Command::StartDebugging);
                    item(ui, "Stop Debugging", Command::StopDebugging);
                    item(ui, "Restart Debugging", Command::RestartDebugging);
                    item(ui, "Step Over", Command::StepOver);
                    item(ui, "Step Into", Command::StepInto);
                    item(ui, "Step Out", Command::StepOut);
                    ui.separator();
                    item(ui, "Toggle Breakpoint", Command::ToggleBreakpoint);
                    item(ui, "Conditional Breakpoint...", Command::ConditionalBreakpoint);
                    item(ui, "Logpoint...", Command::AddLogpoint);
                    ui.separator();
                    item(ui, "dbg! Output", Command::ShowDbgOutput);
                    item(ui, "Remove dbg! from This File", Command::RemoveDbgFromFile);
                    item(ui, "Remove dbg! from Workspace...", Command::RemoveDbgFromWorkspace);
                    ui.separator();
                }
                item(ui, "Run Program", Command::RunProgram);
                item(ui, "Run Current File", Command::RunCurrentFile);
                item(ui, "Run Without Debugging", Command::RunWithoutDebugging);
                item(ui, "Choose What Run Starts...", Command::ChooseRunTarget);
                item(ui, "Send Line, Selection or Cell to REPL", Command::SendToRepl);
                item(ui, "Constant Preview", Command::ConstantPreview);
                item(ui, "Open Live Preview", Command::OpenLivePreview);
                if self.preview_running() {
                    item(ui, "Stop Live Preview", Command::StopLivePreview);
                }
                item(ui, "Select and Run Configuration...", Command::SelectRunConfig);
                item(ui, "Open Configurations", Command::OpenRunConfigs);
                if cargo_project {
                    ui.separator();
                    item(ui, "Run Test at Cursor", Command::TestAtCursor);
                    item(ui, "Run Tests in Current File", Command::TestCurrentFile);
                    item(ui, "Run All Tests", Command::TestAll);
                    item(ui, "Cancel Test Run", Command::CancelTests);
                    ui.separator();
                    item(ui, "Cargo Check", Command::CargoCheck);
                    item(ui, "Cargo Clippy", Command::CargoClippy);
                }
            });
            end = run.response.rect.max.x;
        }
        (cmd, end)
    }

    pub(super) fn activity_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        ui.add_space(6.0);
        let mut icon = |ui: &mut egui::Ui, glyph: Icon, tip: &str, active: bool, c: Command| {
            let color = if active { theme::colors().tab_active_fg } else { theme::colors().text_dim };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(48.0, 44.0), Sense::click());
            if active && theme::islands() {
                ui.painter().rect_filled(rect.shrink2(egui::vec2(6.0, 2.0)), 8.0, theme::colors().selection);
            } else if active {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                    0.0,
                    theme::colors().accent,
                );
            }
            let color = if resp.hovered() { theme::colors().tab_active_fg } else { color };
            icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)),
                glyph,
                color,
            );
            if resp.on_hover_text(tip).clicked() {
                cmd = Some(c);
            }
        };
        let side = self.sidebar_visible.then_some(self.sidebar_view);
        icon(ui, Icon::Files, "Explorer (Ctrl+Shift+E)", side == Some(SidebarView::Explorer), Command::ShowExplorer);
        icon(ui, Icon::Search, "Search (Ctrl+Shift+F)", side == Some(SidebarView::Search), Command::ShowSearch);
        let git = side == Some(SidebarView::Git);
        icon(ui, Icon::SourceControl, "Source Control (Ctrl+Shift+G)", git, Command::ShowSourceControl);
        if self.testing_available() {
            let testing = side == Some(SidebarView::Testing);
            icon(ui, Icon::Beaker, "Testing", testing, Command::ShowTesting);
        }
        if self.can_debug() {
            let debugging = side == Some(SidebarView::Debug);
            icon(ui, Icon::Bug, "Run and Debug (Ctrl+Shift+D)", debugging, Command::ShowDebug);
        }
        let extensions = side == Some(SidebarView::Extensions);
        icon(ui, Icon::Extensions, "Extensions (Ctrl+Shift+X)", extensions, Command::ShowExtensions);
        let terminal = self.panel_visible && self.panel_tab == PanelTab::Terminal;
        icon(ui, Icon::Terminal, "Terminal (Ctrl+`)", terminal, Command::ToggleTerminal);
        cmd
    }

    /// The bottom panel: Problems, References and Terminal tabs.
    pub(super) fn panel(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        // Docked at the side there's no room for the tabs and the buttons on one line.
        let stacked = ui.available_width() < 760.0;
        let header = |ui: &mut egui::Ui| {
            let mut tabs = vec![
                (PanelTab::Problems, self.problems_tab_label()),
                (PanelTab::References, "REFERENCES".to_owned()),
                (PanelTab::Output, "OUTPUT".to_owned()),
                (PanelTab::DebugConsole, "DEBUG CONSOLE".to_owned()),
                (PanelTab::Prints, self.prints_tab_label()),
                (PanelTab::Terminal, "TERMINAL".to_owned()),
            ];
            let can_debug = self.can_debug();
            tabs.retain(|t| match t.0 {
                PanelTab::DebugConsole => can_debug,
                PanelTab::Prints => self.cargo_project,
                _ => true,
            });
            if self.preview_running() {
                tabs.push((PanelTab::PageConsole, "PAGE".to_owned()));
            }
            if self.at_cursor_tab_shown() {
                tabs.push((PanelTab::AtCursor, "AT CURSOR".to_owned()));
            }
            if self.tree_search_tab_shown() {
                tabs.push((PanelTab::TreeSearch, "TREE SEARCH".to_owned()));
            }
            if self.live_tab_shown() {
                tabs.push((PanelTab::Live, "LIVE".to_owned()));
            }
            if self.constant_tab_shown() {
                tabs.push((PanelTab::Constant, "PREVIEW".to_owned()));
            }
            if self.has_coverage() {
                tabs.push((PanelTab::Coverage, "COVERAGE".to_owned()));
            }
            if self.traces.used {
                tabs.push((PanelTab::StackTrace, "STACK TRACE".to_owned()));
            }
            for (i, p) in self.plugins.panels.iter().enumerate() {
                tabs.push((PanelTab::Plugin(i), p.title.to_uppercase()));
            }
            for (tab, label) in tabs {
                let active = self.panel_tab == tab;
                let color = if active { theme::colors().tab_active_fg } else { theme::colors().text_dim };
                let resp = ui.add(egui::Label::new(RichText::new(label).small().color(color)).sense(Sense::click()));
                if active {
                    let r = resp.rect;
                    ui.painter().hline(r.x_range(), r.bottom() + 2.0, Stroke::new(1.0, theme::colors().accent));
                }
                if resp.clicked() {
                    self.panel_tab = tab;
                    self.terminal.focus_requested = tab == PanelTab::Terminal;
                }
                ui.add_space(8.0);
            }
            if !stacked {
                action = self.panel_buttons(ui);
            }
        };
        if stacked {
            ui.horizontal_wrapped(header);
            ui.horizontal(|ui| action = self.panel_buttons(ui));
        } else {
            ui.horizontal(header);
        }
        ui.add_space(4.0);
        match self.panel_tab {
            PanelTab::Problems => self.problems_panel(ui),
            PanelTab::References => self.references_panel(ui),
            PanelTab::Output => self.output_panel(ui),
            PanelTab::DebugConsole => self.debug_console(ui),
            PanelTab::Prints => self.prints_panel(ui),
            PanelTab::Plugin(i) => self.plugin_panel(ui, i),
            PanelTab::Terminal if self.terminal.detached => {
                ui.label(RichText::new("The terminal is in its own window.").color(theme::colors().text_dim));
                if ui.button("Move It Back to the Panel").clicked() {
                    action = Some(TerminalAction::Attach);
                }
            }
            PanelTab::PageConsole => self.page_console(ui),
            PanelTab::Live => self.live_panel(ui),
            PanelTab::Constant => self.constant_panel(ui),
            PanelTab::TreeSearch => self.tree_search_panel(ui),
            PanelTab::AtCursor => self.at_cursor_panel(ui),
            PanelTab::Coverage => self.coverage_panel(ui),
            PanelTab::StackTrace => self.stack_trace_panel(ui),
            PanelTab::Terminal => {
                let font_size = self.terminal_font();
                self.terminal.body(ui, font_size);
            }
        }
        self.terminal_action(ui.ctx(), action);
    }

    /// Closing the panel, and the terminal's own buttons, at the right.
    fn panel_buttons(&mut self, ui: &mut egui::Ui) -> Option<TerminalAction> {
        let mut action = None;
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if icons::button(ui, Icon::Close, "Close Panel").clicked() {
                self.panel_visible = false;
            }
            if self.panel_tab == PanelTab::Terminal {
                ui.add_space(8.0);
                action = self.terminal.header(ui);
            }
        });
        action
    }

    /// Applies a button from the terminal header.
    pub(super) fn terminal_action(&mut self, ctx: &egui::Context, action: Option<TerminalAction>) {
        match action {
            Some(TerminalAction::New) => self.new_terminal(ctx),
            Some(TerminalAction::Kill) => self.terminal.kill_active(),
            Some(TerminalAction::Detach) => self.run_command(ctx, Command::DetachTerminal),
            Some(TerminalAction::Attach) => {
                self.terminal.detached = false;
                self.panel_visible = true;
                self.panel_tab = PanelTab::Terminal;
                self.terminal.focus_requested = true;
            }
            None => {}
        }
    }

    /// The terminal in its own window (when popped out of the panel).
    pub(super) fn terminal_window(&mut self, ctx: &egui::Context) {
        if !self.terminal.detached {
            return;
        }
        let builder =
            egui::ViewportBuilder::default().with_title("Terminal - Rust Code").with_inner_size([900.0, 520.0]);
        let mut action = None;
        let mut closed = false;
        let font_size = self.terminal_font();
        let terminal = &mut self.terminal;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("terminal_window"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                closed = true;
            }
            egui::Frame::new().fill(theme::colors().editor_bg).inner_margin(egui::Margin::symmetric(8, 6)).show(
                ui,
                |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("TERMINAL").small().color(theme::colors().tab_active_fg));
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| action = terminal.header(ui));
                    });
                    ui.add_space(4.0);
                    terminal.body(ui, font_size);
                },
            );
        });
        if closed {
            action = Some(TerminalAction::Attach);
        }
        self.terminal_action(ctx, action);
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        // A picture showing in place of the editor is what the explorer marks.
        let active_path = self
            .shown_image()
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.docs.get(self.active).and_then(|d| d.path.clone()));
        match self.sidebar_view {
            SidebarView::Explorer => {
                self.plugin_tree_sections(ui);
                let open = self.timeline_open;
                let panel =
                    egui::Panel::bottom("timeline").resizable(open).frame(egui::Frame::NONE).show_separator_line(true);
                let panel =
                    if open { panel.default_size(180.0).size_range(80.0..=500.0) } else { panel.exact_size(26.0) };
                panel.show(ui, |ui| {
                    let header = ui
                        .horizontal(|ui| {
                            let size = ui.text_style_height(&egui::TextStyle::Body);
                            let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
                            let a = icons::show(ui, chevron, size, theme::colors().text_dim)
                                .interact(Sense::click())
                                .clicked();
                            let b = ui
                                .add(egui::Label::new(RichText::new("TIMELINE").small().strong()).sense(Sense::click()))
                                .clicked();
                            a || b
                        })
                        .inner;
                    if header {
                        self.timeline_open = !self.timeline_open;
                    }
                    if open {
                        self.timeline_view(ui);
                    }
                });
                let open = self.outline_open;
                let panel =
                    egui::Panel::bottom("outline").resizable(open).frame(egui::Frame::NONE).show_separator_line(true);
                let panel =
                    if open { panel.default_size(220.0).size_range(80.0..=600.0) } else { panel.exact_size(26.0) };
                panel.show(ui, |ui| {
                    let header = ui
                        .horizontal(|ui| {
                            let size = ui.text_style_height(&egui::TextStyle::Body);
                            let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
                            let a = icons::show(ui, chevron, size, theme::colors().text_dim)
                                .interact(Sense::click())
                                .clicked();
                            let b = ui
                                .add(egui::Label::new(RichText::new("OUTLINE").small().strong()).sense(Sense::click()))
                                .clicked();
                            a || b
                        })
                        .inner;
                    if header {
                        self.outline_open = !self.outline_open;
                    }
                    if open {
                        self.outline_view(ui);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PROJECT").small().strong());
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if self.explorer.root.is_some() {
                            if icons::button(ui, Icon::CollapseAll, "Collapse All").clicked() {
                                self.explorer.collapse_all();
                            }
                            if let Some(path) = active_path.as_deref()
                                && icons::button(ui, Icon::Locate, "Select Opened File").clicked()
                            {
                                self.explorer.reveal(path);
                            }
                            if icons::button(ui, Icon::Refresh, "Refresh Explorer").clicked() {
                                self.explorer.refresh();
                                self.finder.mark_stale();
                            }
                            if icons::button(ui, Icon::Folder, "New Folder...").clicked() {
                                self.start_explorer_action(ExplorerAction::NewFolder(None));
                            }
                            let new = icons::button(ui, Icon::Plus, "New...");
                            egui::Popup::menu(&new).show(|ui| self.explorer.new_menu(ui, None));
                        }
                    });
                });
                if self.explorer.root.is_none()
                    && ui.button("Open Folder").clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                {
                    self.open_folder(path);
                }
                self.delete_prompt(ui);
                self.explorer.compact = self.settings.workbench.compact_folders;
                self.explorer.nest = self.settings.workbench.nest_files;
                if let Some(path) = self.explorer.ui(ui, active_path.as_deref()) {
                    self.open_file(&path);
                }
                if let Some(action) = self.explorer.action.take() {
                    self.start_explorer_action(action);
                }
            }
            SidebarView::Git => self.source_control_view(ui),
            SidebarView::Testing => self.testing_view(ui),
            SidebarView::Debug => self.debug_view(ui),
            SidebarView::Extensions => self.extensions_view(ui),
            SidebarView::Search => {
                ui.label(RichText::new("SEARCH").small().strong());
                let jump = self.search.ui(ui, self.explorer.root.as_deref());
                if std::mem::take(&mut self.search.replace_requested) {
                    self.replace_in_project(ui.input(|i| i.time));
                }
                if let Some(jump) = jump {
                    self.open_file(&jump.path);
                    if let Some(doc) = self.active_doc() {
                        // Select the match itself, not just the line.
                        let line = jump.line.saturating_sub(1).min(doc.buffer.len_lines() - 1);
                        let start = doc.buffer.line_start(line);
                        let end = doc.buffer.line_end(line);
                        doc.select((start + jump.cols.0).min(end), (start + jump.cols.1).min(end));
                    }
                }
            }
        }
    }

    pub(super) fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        let mut close = None;
        let mut tabs = Vec::with_capacity(self.docs.len());
        let mut drag_started = None;
        let view_tabs = self.view_tabs();
        let view_showing = view_tabs.iter().any(|t| t.2);
        let (mut view_select, mut view_close) = (None, None);
        let bar_height = ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body)) + 14.0;
        let buttons_width = if view_showing { 0.0 } else { self.file_buttons_ui(ui, bar_height) };
        egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .max_width((ui.available_width() - buttons_width).max(80.0))
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (i, doc) in self.docs.iter().enumerate() {
                        let active = i == self.active && !view_showing;
                        let fill = if active { theme::colors().tab_active_bg } else { theme::colors().tab_inactive_bg };
                        let resp = egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let color =
                                        if active { theme::colors().tab_active_fg } else { theme::colors().text_dim };
                                    let icon_size = ui.text_style_height(&egui::TextStyle::Body);
                                    let inside = doc.path.as_deref().zip(self.explorer.root.as_deref());
                                    let place = inside.and_then(|(p, r)| p.strip_prefix(r).ok());
                                    let place = place.map_or_else(|| doc.title(), |p| p.to_string_lossy().into_owned());
                                    let role = match &doc.path {
                                        Some(path) => self.explorer.role_of(path, &place),
                                        None => FileRole::of_name(&place),
                                    };
                                    icons::show_source(ui, &place, role, icon_size);
                                    ui.add_space(4.0);
                                    let label = ui.add(
                                        egui::Label::new(RichText::new(doc.title()).color(color))
                                            .sense(Sense::click_and_drag())
                                            .selectable(false),
                                    );
                                    if label.clicked() {
                                        select = Some(i);
                                    }
                                    if label.drag_started() {
                                        drag_started = Some(doc.uid);
                                    }
                                    if label.middle_clicked() {
                                        close = Some(i);
                                    }
                                    // Unsaved: a dot that turns into × on hover, like VS Code.
                                    let size = ui.text_style_height(&egui::TextStyle::Body);
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
                                    let hovered = resp.hovered();
                                    if hovered {
                                        ui.painter().rect_filled(rect, 3.0, theme::colors().hover_bg);
                                    }
                                    let (glyph, color) = match (doc.is_dirty(), hovered) {
                                        (true, false) => (Icon::Dot, theme::colors().modified),
                                        (_, true) => (Icon::Close, theme::colors().tab_active_fg),
                                        _ => (Icon::Close, theme::colors().text_dim),
                                    };
                                    icons::paint(ui.painter(), rect.shrink(size * 0.12), glyph, color);
                                    if resp.on_hover_text("Close (Ctrl+W)").clicked() {
                                        close = Some(i);
                                    }
                                });
                            })
                            .response;
                        tabs.push(resp.rect);
                        let r = resp.rect;
                        if active && theme::islands() {
                            // A line under the tab, that slides over from the last one.
                            let ctx = ui.ctx().clone();
                            let time = ctx.global_style().animation_time;
                            let left = ctx.animate_value_with_time(egui::Id::unique("tab_line_left"), r.left(), time);
                            let right =
                                ctx.animate_value_with_time(egui::Id::unique("tab_line_right"), r.right(), time);
                            let y = r.bottom() - 2.0;
                            let line = egui::Rect::from_x_y_ranges(left + 4.0..=right - 4.0, y..=y + 2.0);
                            ui.painter().rect_filled(line, 1.0, theme::colors().accent);
                        } else if active {
                            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, theme::colors().accent));
                        } else if !theme::islands() {
                            ui.painter().vline(r.right(), r.y_range(), Stroke::new(1.0, theme::colors().border));
                        }
                    }
                    for (tab, title, shown) in &view_tabs {
                        let pal = theme::colors();
                        let fill = if *shown { pal.tab_active_bg } else { pal.tab_inactive_bg };
                        let resp = egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let color = if *shown { pal.tab_active_fg } else { pal.text_dim };
                                    let size = ui.text_style_height(&egui::TextStyle::Body);
                                    icons::show_file(ui, title, size);
                                    ui.add_space(4.0);
                                    let label = ui.add(
                                        egui::Label::new(RichText::new(title).color(color))
                                            .sense(Sense::click())
                                            .selectable(false),
                                    );
                                    if label.clicked() {
                                        view_select = Some(*tab);
                                    }
                                    if label.middle_clicked() {
                                        view_close = Some(*tab);
                                    }
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
                                    let hovered = resp.hovered();
                                    if hovered {
                                        ui.painter().rect_filled(rect, 3.0, pal.hover_bg);
                                    }
                                    let glyph = if hovered { pal.tab_active_fg } else { pal.text_dim };
                                    icons::paint(ui.painter(), rect.shrink(size * 0.12), Icon::Close, glyph);
                                    if resp.on_hover_text("Close (Ctrl+W)").clicked() {
                                        view_close = Some(*tab);
                                    }
                                });
                            })
                            .response;
                        let r = resp.rect;
                        if *shown {
                            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.accent));
                        } else {
                            ui.painter().vline(r.right(), r.y_range(), Stroke::new(1.0, pal.border));
                        }
                    }
                });
            });
        if let Some(i) = select {
            self.hide_view_tabs();
            self.active = i;
            self.git.diff = None;
        }
        if let Some(tab) = view_close {
            self.close_view_tab(tab);
        } else if let Some(tab) = view_select {
            self.show_view_tab(tab);
        }
        if let Some(i) = close {
            self.request_close_tab(i);
        }
        self.tab_drag = drag_started.or(self.tab_drag);
        if let Some(uid) = self.tab_drag {
            let bar = ui.min_rect();
            self.drag_tab(ui, uid, &tabs, bar);
        }
    }

    pub(super) fn breadcrumbs(&mut self, ui: &mut egui::Ui) {
        let symbols = self.breadcrumb_symbols();
        let Some(doc) = self.docs.get(self.active) else { return };
        let text = match (&doc.path, &self.explorer.root) {
            (Some(p), Some(root)) => p
                .strip_prefix(root)
                .unwrap_or(p)
                .components()
                // Skip the root "\" component so absolute paths read "C: › Users › …".
                .filter(|c| !matches!(c, std::path::Component::RootDir))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("  ›  "),
            (Some(p), None) => p.display().to_string(),
            (None, _) => doc.title(),
        };
        let mut jump = None;
        let mut pick = false;
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let dim = theme::colors().text_dim;
            let size = small_size(ui);
            let file = ui.add(egui::Label::new(RichText::new(text).size(size).color(dim)).sense(Sense::click()));
            if file.on_hover_text("Go to Symbol in Editor (Ctrl+Shift+O)").clicked() {
                pick = true;
            }
            for (label, line) in symbols {
                ui.label(RichText::new("›").size(size).color(dim));
                let crumb = ui.add(egui::Label::new(RichText::new(label).size(size).color(dim)).sense(Sense::click()));
                if crumb.clicked() {
                    jump = Some(line);
                }
            }
        });
        if let Some(line) = jump {
            self.jump_to_symbol_line(line);
        }
        if pick {
            self.go_to_symbol();
        }
    }

    pub(super) fn find_bar(&mut self, ui: &mut egui::Ui) {
        if !self.find.open {
            return;
        }
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let fresh = self
            .find
            .cache
            .as_ref()
            .is_some_and(|c| (c.uid, c.version) == (uid, version) && c.options == self.find.options);
        if !fresh {
            let options = self.find.options.clone();
            let (matches, error) = match find::find_all(&doc.text(), &options, MAX_FIND_MATCHES) {
                Ok(m) => (m, None),
                Err(e) => (Vec::new(), Some(e)),
            };
            self.find.cache = Some(FindCache { uid, version, options, matches, error });
        }
        let matches = self.find.cache.as_ref().map(|c| c.matches.clone()).unwrap_or_default();
        let error = self.find.cache.as_ref().and_then(|c| c.error.clone());
        let sel_start = doc.buffer.primary().start();
        let current = matches.iter().position(|&(s, _)| s >= sel_start);

        let mut go_next = false;
        let mut go_prev = false;
        let mut replace_one = false;
        let mut replace_all = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(14.0);
                if icons::button(ui, Icon::Close, "Close (Escape)").clicked() {
                    self.find.open = false;
                }
                if icons::button(ui, Icon::ArrowDown, "Next Match (Enter)").clicked() {
                    go_next = true;
                }
                if icons::button(ui, Icon::ArrowUp, "Previous Match (Shift+Enter)").clicked() {
                    go_prev = true;
                }
                let count = if self.find.options.query.is_empty() {
                    String::new()
                } else if let Some(e) = &error {
                    e.clone()
                } else if matches.is_empty() {
                    "No results".to_owned()
                } else {
                    let shown = current.map_or(matches.len(), |c| c + 1);
                    format!("{shown} of {}", matches.len())
                };
                ui.add(egui::Label::new(RichText::new(count).small().color(theme::colors().text_dim)).truncate());
                let toggle = |ui: &mut egui::Ui, on: &mut bool, text: &str, tip: &str| {
                    if ui.selectable_label(*on, RichText::new(text).monospace()).on_hover_text(tip).clicked() {
                        *on = !*on;
                    }
                };
                toggle(ui, &mut self.find.options.regex, ".*", "Use Regular Expression");
                toggle(ui, &mut self.find.options.whole_word, "ab", "Match Whole Word");
                toggle(ui, &mut self.find.options.match_case, "Aa", "Match Case");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.find.options.query).hint_text("Find").desired_width(220.0),
                );
                if self.find.focus_requested {
                    edit.request_focus();
                    self.find.focus_requested = false;
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    if ui.input(|i| i.modifiers.shift) {
                        go_prev = true;
                    } else {
                        go_next = true;
                    }
                    edit.request_focus();
                }
                if edit.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.find.open = false;
                }
                let arrow = if self.find.show_replace { Icon::ChevronDown } else { Icon::ChevronRight };
                if icons::button(ui, arrow, "Toggle Replace (Ctrl+H)").clicked() {
                    self.find.show_replace = !self.find.show_replace;
                }
            });
        });
        if self.find.show_replace {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(14.0);
                    replace_all = ui.button("Replace All").clicked();
                    replace_one = ui.button("Replace").clicked();
                    let hint = if self.find.options.regex { "Replace ($1 for groups)" } else { "Replace" };
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.find.replacement).hint_text(hint).desired_width(220.0),
                    );
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        replace_one = true;
                        edit.request_focus();
                    }
                });
            });
        }
        if replace_all {
            let (options, with) = (self.find.options.clone(), self.find.replacement.clone());
            self.status = Some(match self.docs[self.active].replace_all(&options, &with) {
                Ok(n) => format!("Replaced {n} occurrences"),
                Err(e) => e,
            });
            return;
        }
        if replace_one {
            // The match under the selection gets replaced; otherwise go to the next.
            let (start, end) = {
                let p = doc.buffer.primary();
                (p.start(), p.end())
            };
            if matches.contains(&(start, end)) {
                let found = find::replacements(&doc.text(), &self.find.options, &self.find.replacement);
                if let Some(r) = found.ok().and_then(|f| f.into_iter().find(|r| r.start == start)) {
                    doc.buffer.insert(&r.text);
                    let after = start + r.text.chars().count();
                    let text = doc.text();
                    let next = find::find_all(&text, &self.find.options, MAX_FIND_MATCHES).unwrap_or_default();
                    let target = next.iter().find(|m| m.0 >= after).or(next.first()).copied();
                    if let Some((s, e)) = target {
                        doc.select(s, e);
                    }
                }
                return;
            }
            go_next = true;
        }

        if matches.is_empty() {
            return;
        }
        let target = if go_next {
            // First match strictly after the current selection start.
            let replacing = replace_one && sel_start < matches[matches.len() - 1].0 + 1;
            let found = if replacing { matches.iter().find(|&&(s, _)| s >= sel_start) } else { None };
            Some(found.or(matches.iter().find(|&&(s, _)| s > sel_start)).unwrap_or(&matches[0]))
        } else if go_prev {
            Some(matches.iter().rev().find(|&&(s, _)| s < sel_start).unwrap_or(&matches[matches.len() - 1]))
        } else {
            None
        };
        if let Some(&(s, e)) = target {
            // Keep focus in the find box (like VS Code); just move the selection.
            doc.buffer.set_single(crate::buffer::Selection::new(s, e));
            doc.reveal = true;
        }
    }

    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            ui.label(RichText::new("Rust Code").size(44.0).color(theme::colors().text_dim));
            ui.label(RichText::new("Editing evolved, in Rust").size(16.0).color(theme::colors().text_dim));
            ui.add_space(24.0);
            let keymap = &self.keymap;
            let rows: &[(&str, Command)] = &[
                ("New File", Command::NewFile),
                ("Open File...", Command::OpenFile),
                ("Open Folder...", Command::OpenFolder),
                ("Open Recent...", Command::OpenRecent),
                ("Show All Commands", Command::CommandPalette),
                ("Go to File...", Command::QuickOpen),
                ("Toggle Terminal", Command::ToggleTerminal),
            ];
            // Grids don't center themselves, so pad to roughly center a ~300px block.
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 300.0) / 2.0).max(0.0));
                egui::Grid::new("welcome").num_columns(2).spacing([40.0, 10.0]).show(ui, |ui| {
                    for (label, c) in rows {
                        if ui.link(*label).clicked() {
                            cmd = Some(*c);
                        }
                        ui.label(RichText::new(keymap.shortcut_text(*c)).monospace().color(theme::colors().text_dim));
                        ui.end_row();
                    }
                });
            });
        });
        cmd
    }

    /// Draws the status bar. Returns a command when a clickable item was clicked.
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut status_cmd = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let size = small_size(ui);
            let white = |s: String| RichText::new(s).size(size).color(theme::colors().status_fg);
            // The top bar says these in the island layout.
            let classic = !theme::islands();
            if let Some(root) = self.explorer.root.as_ref().filter(|_| classic) {
                let name =
                    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
                ui.label(white(name));
            }
            if let Some(branch) = self.git_branch_label().filter(|_| classic) {
                let resp = ui
                    .add(egui::Label::new(white(branch)).sense(Sense::click()).selectable(false))
                    .on_hover_text("Switch branch (Git: Checkout to...)");
                if resp.clicked() {
                    status_cmd = Some(Command::GitCheckout);
                }
            }
            let (errors, warnings) = self.problem_counts();
            let fg = theme::colors().status_fg;
            let problems = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    crate::lsp_ui::severity_icon(ui, 1, fg);
                    ui.label(white(errors.to_string()));
                    ui.add_space(4.0);
                    crate::lsp_ui::severity_icon(ui, 2, fg);
                    ui.label(white(warnings.to_string()));
                })
                .response
                .interact(Sense::click())
                .on_hover_text("Problems (Ctrl+Shift+M)");
            if problems.clicked() {
                status_cmd = Some(Command::ShowProblems);
            }
            if let Some(text) = self.debug_status() {
                ui.label(white(text));
            }
            if self.restricted_folder().is_some() {
                let resp = ui
                    .add(egui::Label::new(white("Restricted Mode".to_owned())).sense(Sense::click()).selectable(false))
                    .on_hover_text("This folder isn't trusted. Click to decide.");
                if resp.clicked() {
                    self.trust_prompt_hidden = false;
                }
            }
            self.plugin_status_items(ui, rust_code_core::plugin::manifest::Alignment::Left, size);
            if let Some(vim) = &self.vim {
                ui.label(white(vim.status()));
            }
            if let Some((text, tip)) = self.live_label() {
                let resp =
                    ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false)).on_hover_text(tip);
                if resp.clicked() {
                    status_cmd = Some(Command::ShowLive);
                }
            }
            if let Some((text, tip)) = self.coverage_label() {
                let resp =
                    ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false)).on_hover_text(tip);
                if resp.clicked() {
                    status_cmd = Some(Command::ShowCoverage);
                }
            }
            if let Some(text) = self.log_label() {
                let resp = ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false));
                if resp.clicked() {
                    status_cmd = Some(Command::ShowLogView);
                }
            }
            if let Some(text) = self.table_label() {
                let resp = ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false));
                if resp.clicked() {
                    status_cmd = Some(Command::ToggleTableView);
                }
            }
            if let Some((text, tip)) = self.cpp_label() {
                let resp =
                    ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false)).on_hover_text(tip);
                if resp.clicked() {
                    status_cmd = Some(Command::ShowCompileInfo);
                }
            }
            if let Some(text) = self.lsp_status_text() {
                let resp = ui
                    .add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false))
                    .on_hover_text("Language Servers: state, memory and controls");
                if resp.clicked() {
                    status_cmd = Some(Command::ShowLanguageServers);
                }
            }
            if let Some(msg) = &self.status {
                // Long messages, like git's errors, would run into the right side.
                const RIGHT_SIDE: f32 = 440.0;
                let room = (ui.available_width() - RIGHT_SIDE).max(0.0);
                if room > 40.0 {
                    let height = ui.available_height();
                    ui.allocate_ui(egui::vec2(room, height), |ui| {
                        ui.add(egui::Label::new(white(msg.clone())).truncate()).on_hover_text(msg);
                    });
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if self.shown_image().is_some() {
                    ui.label(white("Image".to_owned()));
                } else if let Some(doc) = self.docs.get(self.active) {
                    ui.label(white(doc.language_name().to_owned()));
                    if doc.buffer.read_only {
                        ui.label(white("Read-only".to_owned()))
                            .on_hover_text("Supplied by the language server; changes aren't possible");
                    }
                    if doc.large {
                        ui.label(white("Large file mode".to_owned()))
                            .on_hover_text("Syntax highlighting and word wrap are off for very large files");
                    }
                    let clickable = |ui: &mut egui::Ui, text: String, tip: &str| {
                        ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false))
                            .on_hover_text(tip)
                            .clicked()
                    };
                    if clickable(ui, doc.line_ending().to_owned(), "Select End of Line Sequence") {
                        status_cmd = Some(Command::ChangeLineEnding);
                    }
                    if clickable(ui, doc.encoding_name(), "Select Encoding") {
                        status_cmd = Some(Command::ChangeEncoding);
                    }
                    let indent = doc.buffer.indent;
                    let label = if indent.use_tabs {
                        format!("Tabs: {}", crate::buffer::tab_width())
                    } else {
                        format!("Spaces: {}", indent.size)
                    };
                    if clickable(ui, label, "Select Indentation") {
                        status_cmd = Some(Command::ChangeIndentation);
                    }
                    let (line, col) = doc.cursor();
                    let count = doc.buffer.selections().len();
                    let sel = match (count, doc.selection_len()) {
                        (1, 0) => String::new(),
                        (1, n) => format!(" ({n} selected)"),
                        (n, _) => format!(" ({n} selections)"),
                    };
                    ui.label(white(format!("Ln {line}, Col {col}{sel}")));
                }
                let zoom = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add(
                            egui::Label::new(white(format!("{:.0}%", self.zoom * 100.0)))
                                .sense(Sense::hover())
                                .selectable(false),
                        );
                        icons::show(ui, Icon::ZoomIn, size, theme::colors().status_fg);
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_text("Zoom");
                egui::Popup::menu(&zoom)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.zoom_menu(ui));
                if let Some((label, tip)) = self.python_label()
                    && ui
                        .add(egui::Label::new(white(label)).sense(Sense::click()).selectable(false))
                        .on_hover_text(tip)
                        .clicked()
                {
                    status_cmd = Some(Command::ChoosePython);
                }
                if let Some(url) = self.dev_url()
                    && ui
                        .add(egui::Label::new(white(format!("Open {url}"))).sense(Sense::click()).selectable(false))
                        .on_hover_text("Open the dev server in your browser")
                        .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
                if self.preview_running()
                    && ui
                        .add(egui::Label::new(white("Live Preview".to_owned())).sense(Sense::click()).selectable(false))
                        .on_hover_text("Open the page's console")
                        .clicked()
                {
                    self.panel_visible = true;
                    self.panel_tab = PanelTab::PageConsole;
                }
                self.plugin_status_items(ui, rust_code_core::plugin::manifest::Alignment::Right, size);
            });
        });
        status_cmd
    }

    pub(super) fn close_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_close else { return };
        let (message, index) = match pending {
            PendingClose::Tab(i) => {
                let title = self.docs.get(*i).map(Document::title).unwrap_or_default();
                (format!("Do you want to save the changes you made to {title}?"), Some(*i))
            }
            PendingClose::App => {
                let n = self.docs.iter().filter(|d| d.is_dirty()).count();
                (format!("Do you want to save the changes to {n} file(s) before exiting?"), None)
            }
        };

        let mut choice = None;
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(&message);
                ui.label(
                    RichText::new("Your changes will be lost if you don't save them.").color(theme::colors().text_dim),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        choice = Some(2);
                    }
                });
            });

        match (choice, index) {
            (Some(0), Some(i)) => {
                if self.save_doc(i) {
                    self.close_tab(i);
                }
                self.pending_close = None;
            }
            (Some(1), Some(i)) => {
                self.close_tab(i);
                self.pending_close = None;
            }
            (Some(0), None) => {
                let mut all_saved = true;
                for i in 0..self.docs.len() {
                    if self.docs[i].is_dirty() {
                        all_saved &= self.save_doc(i);
                    }
                }
                self.pending_close = None;
                if all_saved {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            (Some(1), None) => {
                self.pending_close = None;
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Some(_), _) => self.pending_close = None,
            (None, _) => {}
        }
    }

    pub(super) fn update_title(&mut self, ctx: &egui::Context) {
        let mut title = String::new();
        if let Some(doc) = self.docs.get(self.active) {
            if doc.is_dirty() {
                title.push_str("● ");
            }
            title.push_str(&doc.title());
            title.push_str(" - ");
        }
        if let Some(root) = &self.explorer.root
            && let Some(name) = root.file_name()
        {
            title.push_str(&name.to_string_lossy());
            title.push_str(" - ");
        }
        title.push_str("Rust Code");
        // Sending a viewport command schedules another frame, so only send changes;
        // otherwise the app repaints continuously.
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// Minimize (a dash), maximize (a square) or restore (two squares), close (an ×).
fn paint_window_button(
    painter: &egui::Painter,
    rect: egui::Rect,
    which: WindowButton,
    maximized: bool,
    color: Color32,
) {
    let c = rect.center();
    let s = 5.0;
    let stroke = Stroke::new(1.0, color);
    match which {
        WindowButton::Minimize => {
            painter.hline(egui::Rangef::new(c.x - s, c.x + s), c.y, stroke);
        }
        WindowButton::Maximize if maximized => {
            let front =
                egui::Rect::from_min_max(egui::pos2(c.x - s, c.y - s + 2.0), egui::pos2(c.x + s - 2.0, c.y + s));
            painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
            painter.line(
                vec![
                    egui::pos2(c.x - s + 2.0, c.y - s + 2.0),
                    egui::pos2(c.x - s + 2.0, c.y - s),
                    egui::pos2(c.x + s, c.y - s),
                    egui::pos2(c.x + s, c.y + s - 2.0),
                    egui::pos2(c.x + s - 2.0, c.y + s - 2.0),
                ],
                stroke,
            );
        }
        WindowButton::Maximize => {
            let r = egui::Rect::from_center_size(c, egui::vec2(2.0 * s, 2.0 * s));
            painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
        }
        WindowButton::Close => {
            painter.line_segment([egui::pos2(c.x - s, c.y - s), egui::pos2(c.x + s, c.y + s)], stroke);
            painter.line_segment([egui::pos2(c.x - s, c.y + s), egui::pos2(c.x + s, c.y - s)], stroke);
        }
    }
}

/// The app mark in the title bar: Rust Code's cog with code brackets.
fn paint_logo(painter: &egui::Painter, rect: egui::Rect) {
    let color = theme::colors().accent;
    let c = rect.center();
    let r = rect.width() / 2.0;
    painter.circle_stroke(c, r * 0.62, Stroke::new(1.5, color));
    for i in 0..8 {
        let (sin, cos) = (i as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
        let v = egui::vec2(cos, sin);
        painter.line_segment([c + v * r * 0.62, c + v * r * 0.98], Stroke::new(2.0, color));
    }
    let k = r * 0.28;
    let stroke = Stroke::new(1.3, color);
    painter.line(
        vec![egui::pos2(c.x - k * 0.2, c.y - k), egui::pos2(c.x - k * 1.1, c.y), egui::pos2(c.x - k * 0.2, c.y + k)],
        stroke,
    );
    painter.line(
        vec![egui::pos2(c.x + k * 0.2, c.y - k), egui::pos2(c.x + k * 1.1, c.y), egui::pos2(c.x + k * 0.2, c.y + k)],
        stroke,
    );
}

/// Size for secondary workbench text (breadcrumbs, status bar): 12/13 of the UI font.
const WINDOW_BUTTON: f32 = 46.0;

/// Minimize, maximize and close, at the right of the bar.
fn window_buttons(ui: &mut egui::Ui, bar: egui::Rect, maximized: bool) {
    let ctx = ui.ctx().clone();
    let pal = theme::colors();
    let buttons = [WindowButton::Minimize, WindowButton::Maximize, WindowButton::Close];
    for (i, which) in buttons.into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(bar.max.x - (3 - i) as f32 * WINDOW_BUTTON, bar.min.y),
            egui::vec2(WINDOW_BUTTON, bar.height()),
        );
        let resp = ui.interact(rect, egui::Id::unique(("window_button", i)), Sense::click());
        let close = which == WindowButton::Close;
        if resp.hovered() {
            let fill = if close { Color32::from_rgb(0xE8, 0x11, 0x23) } else { pal.hover_bg };
            ui.painter().rect_filled(rect, 0.0, fill);
        }
        let color = if close && resp.hovered() { Color32::WHITE } else { pal.text };
        paint_window_button(ui.painter(), rect, which, maximized, color);
        if resp.clicked() {
            match which {
                WindowButton::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                WindowButton::Maximize => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized)),
                WindowButton::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }
}

/// An icon button of the top bar.
fn tool_button(ui: &mut egui::Ui, icon: Icon, color: Color32, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 30.0), Sense::click());
    let hovered = theme::eased(ui.ctx(), response.id.with("hover"), response.hovered());
    if hovered > 0.0 {
        ui.painter().rect_filled(rect, 6.0, theme::colors().hover_bg.gamma_multiply(hovered));
    }
    icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)), icon, color);
    response.on_hover_text(tip)
}

/// A name with a drop-down mark, and `lead` wide of something before it, drawn by `paint`.
fn text_chip(
    ui: &mut egui::Ui,
    text: &str,
    lead: f32,
    paint: impl FnOnce(&egui::Painter, egui::Rect),
) -> egui::Response {
    let pal = theme::colors();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, pal.text);
    let text_width = galley.size().x.min(220.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(8.0 + lead + text_width + 24.0, 30.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal.hover_bg);
    }
    if lead > 0.0 {
        let side = lead - 8.0;
        let at = egui::Rect::from_center_size(
            egui::pos2(rect.min.x + 8.0 + side / 2.0, rect.center().y),
            egui::vec2(side, side),
        );
        paint(ui.painter(), at);
    }
    let left = rect.min.x + 8.0 + lead;
    let clip = rect.with_min_x(left).with_max_x(left + text_width);
    ui.painter().with_clip_rect(clip).galley(
        egui::pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        pal.text,
    );
    let (x, y) = (rect.max.x - 12.0, rect.center().y);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 3.0)],
        pal.text_dim,
        Stroke::NONE,
    ));
    response
}

fn small_size(ui: &egui::Ui) -> f32 {
    egui::TextStyle::Body.resolve(ui.style()).size * 12.0 / 13.0
}

/// All matches of `query` in `text`, as `[start, end)` char index ranges.
/// Most matches the find bar counts and steps through.
const MAX_FIND_MATCHES: usize = 10_000;
