//! Running commands: the command switch, keyboard shortcuts and line edits.

use crate::commands::Command;
use crate::document::Document;
use crate::edit_ops::{self, CaseTransform, Edit, Sel};
use crate::editor_view::{ZoomPart, editor_id};
use crate::keymap::Keymap;
use crate::palette::PickerKind;
use crate::settings;
use crate::theme;
use egui::{Key, Modifiers};
use rust_code_core::encoding;

use super::lsp_extra::Hierarchy;
use super::outline::FoldAction;
use super::{CodeApp, PanelTab, SidebarView};
use rust_code_core::structure::TextObject;
use rust_code_core::tree_edit;

impl CodeApp {
    /// Applies a text edit to the active document and moves its selection.
    /// Runs a line command on the active document as a small, incremental edit.
    pub(super) fn apply_edit(&mut self, f: impl FnOnce(&str, Sel) -> Option<Edit>) {
        self.apply_line_op(false, f);
    }

    /// Like [`Self::apply_edit`], but with an empty selection the command
    /// applies to the whole document (sort, reverse, remove duplicates).
    pub(super) fn apply_edit_whole_if_empty(&mut self, f: impl FnOnce(&str, Sel) -> Option<Edit>) {
        self.apply_line_op(true, f);
    }

    pub(super) fn apply_line_op(&mut self, whole_if_empty: bool, f: impl FnOnce(&str, Sel) -> Option<Edit>) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let whole = whole_if_empty && doc.buffer.primary().is_empty();
        if doc.buffer.apply_line_op(whole, f) {
            doc.reveal = true;
        }
    }

    pub(super) fn editor_focused(&self, ctx: &egui::Context) -> bool {
        let pane = self.active_pane();
        self.docs.get(self.active).is_some_and(|d| ctx.memory(|m| m.has_focus(editor_id(d.uid, pane))))
    }

    pub(super) fn run_command(&mut self, ctx: &egui::Context, cmd: Command) {
        rust_code_core::applog::info(&format!("command {cmd:?}"));
        match cmd {
            Command::OpenLog | Command::OpenCrashLog => {
                let Some(dir) = rust_code_core::applog::dir() else { return };
                let path = if cmd == Command::OpenLog {
                    rust_code_core::applog::log_path(&dir)
                } else {
                    rust_code_core::applog::crash_path(&dir)
                };
                if path.exists() {
                    self.open_file(&path);
                } else {
                    self.status = Some("Nothing has been written there yet: no crash so far".to_owned());
                }
            }
            Command::CommandPalette => self.open_palette(">"),
            Command::QuickOpen => self.open_palette(""),
            Command::SearchEverywhere => self.open_everywhere(),
            Command::NewFile => self.new_file(),
            Command::OpenFile => {
                if let Some(path) = self.file_dialog("open_file").pick_file() {
                    self.dialog_chose("open_file", &path);
                    self.open_file(&path);
                }
            }
            Command::OpenFolder => {
                if let Some(path) = self.file_dialog("open_folder").pick_folder() {
                    self.dialog_chose("open_folder", &path);
                    self.open_folder(path);
                }
            }
            Command::GitClone | Command::GitInit | Command::GitSetAuthor | Command::GitPublishGitHub => {
                self.git_setup_command(ctx, cmd);
            }
            Command::ExplorerFind => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Explorer;
                self.explorer.open_filter();
            }
            Command::Save => {
                // A project's own formatter owns its files: the server would fight it.
                let owner = self.settings.editor.format_on_save.then(|| self.formatter_owner()).flatten();
                // Format first if asked to; the save happens when formatting is done.
                if !(self.settings.editor.format_on_save
                    && owner.is_none()
                    && self.request_format(ctx, self.active, true))
                {
                    self.save_doc(self.active);
                    if let Some(owner) = owner {
                        self.status = Some(format!("Saved without formatting: this project formats with {owner}"));
                    }
                }
            }
            Command::SaveAs => {
                self.save_doc_as(self.active);
            }
            Command::SaveAll => {
                for i in 0..self.docs.len() {
                    if self.docs[i].is_dirty() {
                        self.save_doc(i);
                    }
                }
            }
            Command::CloseTab => {
                if !self.close_shown_view_tab() {
                    self.request_close_tab(self.active);
                }
            }
            Command::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Command::ToggleTerminal => {
                if self.terminal.detached {
                    self.terminal.focus_requested = true;
                } else {
                    self.toggle_panel(PanelTab::Terminal);
                    self.terminal.focus_requested = self.panel_visible;
                }
                if self.panel_visible && self.panel_tab == PanelTab::Terminal && self.terminal.is_empty() {
                    self.new_terminal(ctx);
                }
            }
            Command::NewTerminal => self.new_terminal(ctx),
            Command::KillTerminal => self.terminal.kill_active(),
            Command::ClearTerminal => self.terminal.clear_active(),
            Command::DetachTerminal => {
                if self.terminal.is_empty() {
                    self.new_terminal(ctx);
                }
                self.terminal.detached = true;
                self.terminal.focus_requested = true;
                if self.panel_tab == PanelTab::Terminal {
                    self.panel_visible = false;
                }
            }
            Command::ShowProblems => self.toggle_panel(PanelTab::Problems),
            Command::TriggerSuggest => self.request_completion(None),
            Command::ShowHover => self.show_hover_at_cursor(),
            Command::GoToDefinition => self.request_locations("textDocument/definition", "Definitions", None),
            Command::GoToTypeDefinition => {
                self.request_locations("textDocument/typeDefinition", "Type Definitions", None)
            }
            Command::GoToWorkspaceSymbol => self.open_palette("#"),
            Command::TriggerSignatureHelp => self.request_signature(),
            Command::GoToImplementation => {
                self.request_locations("textDocument/implementation", "Implementations", None);
            }
            Command::FindReferences => self.request_locations("textDocument/references", "References", None),
            Command::ShowIncomingCalls => self.request_hierarchy(Hierarchy::IncomingCalls),
            Command::ShowOutgoingCalls => self.request_hierarchy(Hierarchy::OutgoingCalls),
            Command::ShowSupertypes => self.request_hierarchy(Hierarchy::Supertypes),
            Command::ShowSubtypes => self.request_hierarchy(Hierarchy::Subtypes),
            Command::ShowCodeLens => self.show_code_lens(),
            Command::RenameSymbol => self.start_rename(),
            Command::QuickFix => self.request_code_actions(),
            Command::FormatDocument => {
                if !self.request_format(ctx, self.active, false) && !self.format_json_active() {
                    self.status = Some("No formatter available for this file".to_owned());
                }
            }
            Command::NextProblem | Command::PreviousProblem => self.next_problem(cmd == Command::NextProblem),
            Command::ShowSourceControl => self.show_source_control(),
            Command::LocalHistory => self.history_picker(ctx),
            Command::StartDebugging => self.start_debugging(ctx),
            Command::StopDebugging => self.debug_stop(),
            Command::RestartDebugging => self.debug_restart(ctx),
            Command::StepOver => self.debug_step("next"),
            Command::StepInto => self.debug_step("stepIn"),
            Command::StepOut => self.debug_step("stepOut"),
            Command::PauseDebugging => self.debug_pause(),
            Command::ToggleBreakpoint => self.toggle_breakpoint_at_cursor(),
            Command::ConditionalBreakpoint => self.edit_breakpoint(false),
            Command::AddLogpoint => self.edit_breakpoint(true),
            Command::ShowDebug => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Debug;
            }
            Command::ShowDebugConsole => self.toggle_panel(PanelTab::DebugConsole),
            Command::ManageWorkspaceTrust => self.toggle_workspace_trust(ctx),
            Command::ShowExtensions => self.show_extensions(),
            Command::CheckPluginUpdates => self.check_registry(ctx),
            Command::InstallPluginFromFolder => self.install_plugin_from_folder(ctx),
            Command::DebugTestAtCursor => match self.test_item_at_cursor() {
                Some((target, item)) => self.debug_test(ctx, target, item),
                None => self.status = Some("No test at the cursor".to_owned()),
            },
            Command::ToggleVimMode => {
                let next = if self.settings.editor.keyboard_mode == "vim" { "default" } else { "vim" };
                match self.write_user_setting(ctx, "editor", "keyboard_mode", &format!("\"{next}\"")) {
                    Ok(()) => self.status = Some(format!("Keyboard mode: {next}")),
                    Err(e) => self.status = Some(e),
                }
            }
            Command::ShowTimeline => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Explorer;
                self.timeline_open = true;
            }
            Command::Fold => self.fold_command(FoldAction::Fold),
            Command::Unfold => self.fold_command(FoldAction::Unfold),
            Command::ToggleFold => self.fold_command(FoldAction::Toggle),
            Command::FoldAll => self.fold_command(FoldAction::FoldAll),
            Command::UnfoldAll => self.fold_command(FoldAction::UnfoldAll),
            Command::GoToSymbol => self.go_to_symbol(),
            Command::ExpandSelection => self.expand_selection(),
            Command::ShrinkSelection => self.shrink_selection(),
            Command::SelectFunction => self.select_text_object(TextObject::Function),
            Command::SelectFunctionBody => self.select_text_object(TextObject::FunctionBody),
            Command::SelectType => self.select_text_object(TextObject::Type),
            Command::SelectArgument => self.select_text_object(TextObject::Argument),
            Command::SelectComment => self.select_text_object(TextObject::Comment),
            Command::NextFunction | Command::PreviousFunction => self.adjacent_function(cmd == Command::NextFunction),
            Command::RunWithoutDebugging => self.run_selected_config(ctx),
            Command::RunProgram => self.run_program(ctx),
            Command::RunCurrentFile => self.run_current_file(ctx),
            Command::PreviewFontFamily => self.preview_font_at_cursor(ctx),
            Command::SelectRunConfig => self.select_run_config(ctx),
            Command::ChooseRunTarget => self.choose_run_target(),
            Command::OpenLivePreview => self.open_live_preview(ctx),
            Command::ShowLanguageDoctor => self.show_doctor(ctx),
            Command::SendToRepl => self.send_to_repl(ctx),
            Command::ChoosePython => self.choose_python(),
            Command::OpenLanguagesFile => self.open_languages_file(),
            Command::StopLivePreview => self.stop_live_preview(),
            Command::ConstantPreview => self.toggle_constant_preview(ctx),
            Command::OpenRunConfigs => self.open_run_configs(),
            Command::ShowTesting => self.show_testing(ctx),
            Command::TestAtCursor => self.test_at_cursor(ctx),
            Command::TestCurrentFile => {
                // Cargo's own view knows Rust's; the rest run in a terminal.
                let doc = self.docs.get(self.active);
                let other = doc.and_then(|d| d.path.clone().filter(|_| d.extension() != "rs").map(|p| (p, d.text())));
                let plan = other.and_then(|(file, text)| {
                    rust_code_core::run::tests_of(&file, &text, self.explorer.root.as_deref())
                });
                match plan {
                    Some(plan) => {
                        for i in 0..self.docs.len() {
                            if self.docs[i].is_dirty() && self.docs[i].path.is_some() {
                                self.save_doc(i);
                            }
                        }
                        self.start_run(ctx, &plan);
                    }
                    None => self.test_current_file(ctx),
                }
            }
            Command::TestAll => self.test_all(ctx),
            Command::CancelTests => self.cancel_job(),
            Command::CargoCheck => self.run_cargo_check(ctx, "check"),
            Command::MavenRunGoal => self.maven_goal_picker(None),
            Command::MavenCustomGoal => self.maven_custom_goal(None),
            Command::MavenEffectivePom => self.maven_effective_pom(ctx, None),
            Command::MavenAddDependency => self.maven_add_dependency(None),
            Command::MavenResolveDependencies => self.maven_resolve(ctx, None),
            Command::MavenNewProject => self.maven_new_project(),
            Command::MavenRefresh => self.maven_refresh(ctx),
            Command::ShowMaven => self.show_maven(),
            Command::CargoClippy => self.run_cargo_check(ctx, "clippy"),
            Command::ShowOutput => self.toggle_panel(PanelTab::Output),
            Command::GitCommit => self.git_commit(ctx),
            Command::GitRefresh => self.git_refresh_soon(ctx, 0.0),
            Command::GitCheckout => self.git_checkout(ctx),
            Command::GitCreateBranch => {
                self.palette.show_input(crate::palette::InputKind::NewBranch, "", "New branch name, then Enter");
            }
            Command::GitStageHunk => self.hunk_at_cursor(ctx, true),
            Command::GitRevertHunk => self.hunk_at_cursor(ctx, false),
            Command::GitOpenChanges => {
                if let Some(path) = self.docs.get(self.active).and_then(|d| d.path.clone()) {
                    self.open_diff(ctx, &path, false);
                }
            }
            Command::NextChange | Command::PreviousChange => self.next_change(cmd == Command::NextChange),
            Command::ToggleBlame => {
                let on = !self.settings.git.blame_inline;
                if let Err(e) = self.write_user_setting(ctx, "git", "blame_inline", if on { "true" } else { "false" }) {
                    self.status = Some(e);
                }
            }
            Command::RestartLanguageServer => {
                self.restart_lsp();
                self.status = Some("Restarting language servers…".to_owned());
            }
            Command::ShowExplorer => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Explorer;
            }
            Command::ShowSearch => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Search;
                self.search.focus_requested = true;
            }
            Command::Find => {
                self.find.open = true;
                self.find.focus_requested = true;
            }
            Command::Replace => {
                self.find.open = true;
                self.find.show_replace = true;
                self.find.focus_requested = true;
            }
            Command::GoToLine => self.palette.show_with(":"),
            Command::ZoomIn => self.change_zoom(ctx, self.zoom + super::window::ZOOM_STEP),
            Command::ZoomOut => self.change_zoom(ctx, self.zoom - super::window::ZOOM_STEP),
            Command::ResetZoom => self.change_zoom(ctx, rust_code_core::settings::DEFAULT_ZOOM),
            Command::CodeZoomIn => self.change_font_zoom(ctx, ZoomPart::Code, self.font_size + 1.0),
            Command::CodeZoomOut => self.change_font_zoom(ctx, ZoomPart::Code, self.font_size - 1.0),
            Command::ResetCodeZoom => {
                let size = rust_code_core::settings::EditorSettings::default().font_size;
                self.change_font_zoom(ctx, ZoomPart::Code, size);
            }
            Command::RefreshExplorer => {
                self.explorer.refresh();
                self.finder.mark_stale();
            }
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),

            Command::ToggleLineComment => {
                let ext = self.docs.get(self.active).map(Document::extension).unwrap_or_default();
                match edit_ops::comment_token(&ext) {
                    Some(token) => self.apply_edit(|t, s| Some(edit_ops::toggle_line_comment(t, s, token))),
                    None => self.status = Some("No line comment syntax for this language".to_owned()),
                }
            }
            Command::MoveLinesUp => self.apply_edit(|t, s| edit_ops::move_lines(t, s, false)),
            Command::MoveLinesDown => self.apply_edit(|t, s| edit_ops::move_lines(t, s, true)),
            Command::CopyLinesUp => self.apply_edit(|t, s| Some(edit_ops::copy_lines(t, s, false))),
            Command::CopyLinesDown => self.apply_edit(|t, s| Some(edit_ops::copy_lines(t, s, true))),
            Command::DeleteLines => self.apply_edit(|t, s| Some(edit_ops::delete_lines(t, s))),
            Command::IndentLines | Command::OutdentLines => {
                let unit = self.docs.get(self.active).map(|d| d.buffer.indent.unit()).unwrap_or_default();
                let outdent = cmd == Command::OutdentLines;
                self.apply_edit(|t, s| Some(edit_ops::indent_lines(t, s, outdent, &unit)));
            }
            Command::SelectNextOccurrence => {
                if let Some(doc) = self.active_doc() {
                    doc.buffer.add_next_occurrence();
                    doc.reveal = true;
                }
            }
            Command::SelectAllOccurrences => {
                if let Some(doc) = self.active_doc() {
                    doc.buffer.select_all_occurrences();
                }
            }
            Command::AddCursorAbove | Command::AddCursorBelow => {
                if let Some(doc) = self.active_doc() {
                    doc.buffer.add_cursor_vertical(cmd == Command::AddCursorBelow);
                    doc.reveal = true;
                }
            }
            Command::ExplorerOpen => self.explorer.open_keyboard_row(true),
            Command::ExplorerOpenPreserveFocus => self.explorer.open_keyboard_row(false),
            Command::ExplorerRename => self.explorer.rename_picked(),
            Command::ExplorerDelete => self.explorer.delete_picked(false),
            Command::ExplorerDeletePermanently => self.explorer.delete_picked(true),
            Command::ExplorerCopy => self.explorer.copy_picked(ctx, false),
            Command::ExplorerCut => self.explorer.copy_picked(ctx, true),
            Command::ExplorerPaste => self.explorer.paste_here(),
            Command::ExplorerCopyPath => self.explorer.copy_paths(ctx, false),
            Command::ExplorerCopyRelativePath => self.explorer.copy_paths(ctx, true),
            Command::ExplorerSelectAll => self.explorer.select_all(),
            Command::JumpToBracket => {
                // Jump to the partner of the bracket next to the cursor.
                if let Some(doc) = self.active_doc() {
                    let head = doc.buffer.primary().head;
                    if let Some((open, close)) = doc.matching_bracket(head, 1_000_000) {
                        let target = if head == open || head == open + 1 { close } else { open };
                        doc.select(target, target);
                    }
                }
            }
            Command::JoinLines => self.apply_edit(edit_ops::join_lines),
            Command::DuplicateSelection => self.apply_edit(|t, s| Some(edit_ops::duplicate_selection(t, s))),
            Command::SortLinesAscending => {
                self.apply_edit_whole_if_empty(|t, s| Some(edit_ops::sort_lines(t, s, false)))
            }
            Command::SortLinesDescending => {
                self.apply_edit_whole_if_empty(|t, s| Some(edit_ops::sort_lines(t, s, true)))
            }
            Command::RemoveDuplicateLines => {
                self.apply_edit_whole_if_empty(|t, s| Some(edit_ops::remove_duplicate_lines(t, s)))
            }
            Command::ReverseLines => self.apply_edit_whole_if_empty(|t, s| Some(edit_ops::reverse_lines(t, s))),
            Command::TransformToUppercase => {
                self.apply_edit(|t, s| Some(edit_ops::transform_case(t, s, CaseTransform::Upper)));
            }
            Command::TransformToLowercase => {
                self.apply_edit(|t, s| Some(edit_ops::transform_case(t, s, CaseTransform::Lower)));
            }
            Command::TransformToTitlecase => {
                self.apply_edit(|t, s| Some(edit_ops::transform_case(t, s, CaseTransform::Title)));
            }

            Command::ToggleWordWrap => self.word_wrap = !self.word_wrap,
            Command::ToggleZenMode => {
                self.zen_mode = !self.zen_mode;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.zen_mode));
            }
            Command::NextEditor | Command::PreviousEditor => {
                let n = self.docs.len();
                if n > 0 {
                    let step = if cmd == Command::NextEditor { 1 } else { n - 1 };
                    self.active = (self.active + step) % n;
                    self.docs[self.active].request_focus = true;
                }
            }
            Command::OpenUserSettings => {
                self.settings_ui.workspace = false;
                self.settings_ui.open = true;
            }
            Command::OpenSettingsFile => {
                self.open_config_file(settings::user_settings_path(), settings::default_file_contents());
            }
            Command::OpenWorkspaceSettings => {
                if self.explorer.root.is_some() {
                    self.settings_ui.workspace = true;
                    self.settings_ui.open = true;
                } else {
                    self.status = Some("Open a folder to use workspace settings".to_owned());
                }
            }
            Command::OpenKeybindings => self.keys_ui.open = true,
            Command::OpenKeybindingsFile => {
                let contents = Keymap::defaults().default_file_contents();
                self.open_config_file(settings::user_keybindings_path(), contents);
            }
            Command::SelectTheme => {
                if self.themes.is_empty() {
                    self.themes = theme::discover();
                }
                let mut labels: Vec<String> = self.themes.iter().map(|t| t.label.clone()).collect();
                if !labels.iter().any(|l| l == "Dark Modern") {
                    labels.insert(0, "Dark Modern".to_owned());
                }
                let current = self.settings.workbench.color_theme.clone();
                self.palette.show_picker(PickerKind::Theme, labels, &current, "Select Color Theme");
            }
            Command::NextHiddenCharacter => self.next_hidden_char(),
            Command::ShowLanguageServers => self.show_lsp_room(),
            Command::GitPush => self.git_remote(ctx, Command::GitPush),
            Command::GitPull => self.git_remote(ctx, Command::GitPull),
            Command::GitFetch => self.git_remote(ctx, Command::GitFetch),
            Command::GitStash => self.git_stash(ctx),
            Command::GitShowHistory => self.git_show_history(),
            Command::MarkdownPreview => self.toggle_markdown_preview(),
            Command::ToggleSvgPreview => self.toggle_svg_preview(),
            Command::ImportFromVsCode => self.import_from_vscode(),
            Command::VsCodeCompatibility => self.vscode_compatibility(ctx),
            Command::TakeTour => self.start_tour(),
            Command::NewSampleProject => self.new_sample_project(),
            Command::ChooseBackgroundImage => {
                let filter = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
                if let Some(file) = self.file_dialog("background_picture").add_filter("Pictures", &filter).pick_file() {
                    self.dialog_chose("background_picture", &file);
                    self.set_background(ctx, &file.to_string_lossy());
                }
            }
            Command::ChooseBackgroundFolder => {
                if let Some(folder) = self.file_dialog("background_folder").pick_folder() {
                    self.dialog_chose("background_folder", &folder);
                    self.set_background(ctx, &folder.to_string_lossy());
                }
            }
            Command::ClearBackground => self.set_background(ctx, ""),
            Command::ToggleDataTree => self.data_tree.open = !self.data_tree.open,
            Command::EmmetExpand => {
                self.expand_emmet(false);
            }
            Command::PickColor => self.pick_color(),
            Command::ShowLive => self.show_live(),
            Command::CursorInfoBeside => self.at_cursor_toggle_beside(),
            Command::TreeSlurp => self.tree_move(tree_edit::Move::SlurpForward),
            Command::TreeSlurpBack => self.tree_move(tree_edit::Move::SlurpBackward),
            Command::TreeBarf => self.tree_move(tree_edit::Move::BarfForward),
            Command::TreeBarfBack => self.tree_move(tree_edit::Move::BarfBackward),
            Command::TreeRaise => self.tree_move(tree_edit::Move::Raise),
            Command::TreeSplice => self.tree_move(tree_edit::Move::Splice),
            Command::TreeWrapParens => self.tree_wrap("(", ")"),
            Command::TreeWrapBrackets => self.tree_wrap("[", "]"),
            Command::TreeWrapBraces => self.tree_wrap("{", "}"),
            Command::ShowTreeSearch => self.show_tree_search(),
            Command::RulesCheckProject => self.rules_check_project(ctx),
            Command::RulesFixFile => self.rules_fix_file(),
            Command::RulesFixHere => self.rules_fix_here(),
            Command::ReplRestart => self.restart_repl(),
            Command::ShowCursorInfo => self.show_at_cursor(),
            Command::CursorInfoPause => self.at_cursor_toggle_pause(),
            Command::CursorInfoPin => self.at_cursor_pin(),
            Command::LiveLoadFile => self.live_load_file(),
            Command::ToggleCoverage => self.toggle_coverage(),
            Command::ShowCoverage => self.show_coverage_panel(),
            Command::LoadCoverage => self.pick_coverage_report(ctx),
            Command::OpenHexView => self.hex_command(),
            Command::ShowMemory => self.show_memory(None),
            Command::ShowDisassembly => self.show_disassembly(),
            Command::OpenLogView => self.log_view_command(ctx),
            Command::ShowLogView => self.show_log_view(),
            Command::ToggleTableView => self.toggle_table_view(),
            Command::InsertSymbol => self.pick_glyph(),
            Command::GoToInclude => self.open_include(),
            Command::AnalyzeStackTrace => self.analyze_stack_trace(),
            Command::ShowCompileInfo => self.show_compile_info(),
            Command::SwitchHeaderSource => self.switch_header_source(),
            Command::ToggleBookmark => self.toggle_bookmark(),
            Command::NextBookmark => self.next_bookmark(true),
            Command::PreviousBookmark => self.next_bookmark(false),
            Command::ListBookmarks => self.list_bookmarks(),
            Command::ShowDbgOutput => self.toggle_panel(PanelTab::Prints),
            Command::RemoveDbgFromFile => self.remove_dbg_here(),
            Command::RemoveDbgFromWorkspace => self.plan_remove_dbg(),
            Command::InsertSnippet => self.pick_snippet(),
            Command::ManageSnippets => self.snippets_ui.show(),
            Command::TerminalFind => {
                if self.terminal.is_empty() {
                    self.status = Some("No terminal is running".to_owned());
                } else {
                    self.panel_visible = true;
                    self.panel_tab = PanelTab::Terminal;
                    self.terminal.open_search();
                }
            }
            Command::SplitEditor => self.split_editor(),
            Command::CloseSplit => self.close_split(),
            Command::ToggleSplitLayout => self.toggle_split_layout(),
            Command::FocusFirstPane => self.focus_pane(false),
            Command::FocusSecondPane => self.focus_pane(true),
            Command::ChangeIndentation => {
                if let Some(doc) = self.docs.get(self.active) {
                    let indent = doc.buffer.indent;
                    let current = if indent.use_tabs { "Tabs".to_owned() } else { format!("Spaces: {}", indent.size) };
                    let items = ["Spaces: 2", "Spaces: 4", "Spaces: 8", "Tabs", "Detect from Content"];
                    let items = items.map(str::to_owned).to_vec();
                    self.palette.show_picker(PickerKind::Indentation, items, &current, "Select Indentation");
                }
            }
            Command::OpenRecent => self.show_recent(),
            Command::ClearRecentlyOpened => self.ask_clear_recent(),
            Command::RemoveMissingRecent => self.remove_missing_recent(),
            Command::ChangeLineEnding => {
                if let Some(doc) = self.docs.get(self.active) {
                    let current = doc.line_ending();
                    let items = vec!["LF".to_owned(), "CRLF".to_owned()];
                    self.palette.show_picker(PickerKind::LineEnding, items, current, "Select End of Line Sequence");
                }
            }
            Command::ChangeEncoding => {
                if let Some(doc) = self.docs.get(self.active) {
                    if doc.path.is_some() {
                        let items = vec!["Reopen with Encoding".to_owned(), "Save with Encoding".to_owned()];
                        self.palette.show_picker(PickerKind::EncodingAction, items, "", "Select Action");
                    } else {
                        self.pick_encoding(PickerKind::SaveWithEncoding);
                    }
                }
            }
            Command::ReopenClosedEditor => {
                if let Some(path) = self.closed_editors.pop() {
                    self.open_file(&path);
                }
            }
        }
    }

    /// Carries out what a Vim command line asked for.
    pub(super) fn vim_effect(&mut self, ctx: &egui::Context, effect: rust_code_core::vim::Effect) {
        use rust_code_core::vim::Effect;
        match effect {
            Effect::Save => self.run_command(ctx, Command::Save),
            Effect::Quit { force: true } => self.close_tab(self.active),
            Effect::Quit { force: false } => self.request_close_tab(self.active),
            Effect::SaveQuit => {
                if self.save_doc(self.active) {
                    self.close_tab(self.active);
                }
            }
            Effect::Clipboard(text) => ctx.copy_text(text),
        }
    }

    /// Starts a terminal with the configured shell and shows it.
    pub(super) fn new_terminal(&mut self, ctx: &egui::Context) {
        let (shell, scrollback) = (self.settings.terminal.shell.clone(), self.settings.terminal.scrollback);
        let env = self.environment_variables();
        match self.terminal.new_tab(ctx, &shell, &env, scrollback) {
            Ok(()) => {
                if !self.terminal.detached {
                    self.panel_visible = true;
                    self.panel_tab = PanelTab::Terminal;
                }
            }
            Err(e) => self.status = Some(format!("Terminal: {e}")),
        }
    }

    /// Shows the bottom panel on `tab`, or hides it if it already shows `tab`.
    fn toggle_panel(&mut self, tab: PanelTab) {
        if self.panel_visible && self.panel_tab == tab {
            self.panel_visible = false;
        } else {
            self.panel_visible = true;
            self.panel_tab = tab;
        }
    }

    fn open_everywhere(&mut self) {
        self.finder.refresh_if_stale();
        self.palette.show_everywhere(self.explorer.root.clone(), super::settings_ui::searchable());
    }

    pub(super) fn open_palette(&mut self, prefix: &str) {
        self.finder.refresh_if_stale();
        self.palette.show_with(prefix);
    }

    /// Opens the encoding list for reopening or saving the active file.
    fn pick_encoding(&mut self, kind: PickerKind) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let current = doc.encoding_name();
        let items = encoding::CHOICES.iter().map(|(name, ..)| (*name).to_owned()).collect();
        let placeholder = if kind == PickerKind::ReopenWithEncoding {
            "Select File Encoding to Reopen File"
        } else {
            "Select File Encoding to Save with"
        };
        self.palette.show_picker(kind, items, &current, placeholder);
    }

    /// Applies an item chosen in a palette picker.
    pub(super) fn handle_pick(&mut self, ctx: &egui::Context, kind: PickerKind, label: &str) {
        match kind {
            PickerKind::Theme => self.set_color_theme(ctx, label),
            PickerKind::CodeAction => self.pick_code_action(label),
            PickerKind::CodeLens => self.pick_code_lens(ctx, label),
            PickerKind::Branch => self.pick_branch(ctx, label),
            PickerKind::RunConfig => self.picked_run_config(ctx, label),
            PickerKind::RunMode => self.picked_run_mode(label),
            PickerKind::Python => self.picked_python(ctx, label),
            PickerKind::Glyph => self.picked_glyph(label),
            PickerKind::Symbol => self.picked_symbol(label),
            PickerKind::History => self.picked_history(label),
            PickerKind::RunTarget => self.picked_run_target(ctx, label),
            PickerKind::DebugTarget => self.picked_debug_target(ctx, label),
            PickerKind::Recent => self.open_recent(label),
            PickerKind::GitHubVisibility => self.git_publish_visibility(ctx, label),
            PickerKind::ClearRecent => {
                if label != "Cancel" {
                    self.clear_recent();
                }
            }
            PickerKind::VsCodeImport => self.picked_vscode_import(ctx, label),
            PickerKind::MavenGoal => self.picked_maven_goal(ctx, label),
            PickerKind::MavenFound => self.picked_maven_found(label),
            PickerKind::MavenScope => self.picked_maven_scope(label),
            PickerKind::MavenArchetype => self.picked_maven_archetype(label),
            PickerKind::SampleProject => self.picked_sample(label),
            PickerKind::Indentation => {
                let size = label.strip_prefix("Spaces: ").and_then(|n| n.parse().ok());
                let mut docs = std::mem::take(&mut self.docs);
                if let Some(doc) = docs.get_mut(self.active) {
                    match (label, size) {
                        ("Tabs", _) => doc.buffer.indent.use_tabs = true,
                        (_, Some(size)) => {
                            doc.buffer.indent.use_tabs = false;
                            doc.buffer.indent.size = size;
                        }
                        _ => {}
                    }
                    if label == "Tabs" || size.is_some() {
                        self.indent_chosen.insert(doc.uid, doc.buffer.indent);
                    } else {
                        self.indent_chosen.remove(&doc.uid);
                        self.apply_file_rules(doc);
                    }
                }
                self.docs = docs;
            }
            PickerKind::Bookmark => self.picked_bookmark(label),
            PickerKind::Snippet => self.picked_snippet(label),
            PickerKind::LineEnding => {
                if let Some(doc) = self.active_doc() {
                    doc.set_line_ending(if label == "CRLF" { "\r\n" } else { "\n" });
                }
            }
            PickerKind::EncodingAction => self.pick_encoding(if label.starts_with("Reopen") {
                PickerKind::ReopenWithEncoding
            } else {
                PickerKind::SaveWithEncoding
            }),
            PickerKind::ReopenWithEncoding | PickerKind::SaveWithEncoding => {
                let Some((enc, bom)) = encoding::from_display_name(label) else { return };
                let Some(doc) = self.docs.get_mut(self.active) else { return };
                if kind == PickerKind::ReopenWithEncoding {
                    if let Err(e) = doc.reopen_with_encoding(enc) {
                        self.status = Some(e);
                    }
                } else {
                    let previous = (doc.encoding, doc.bom);
                    (doc.encoding, doc.bom) = (enc, bom);
                    if !self.save_doc(self.active)
                        && let Some(doc) = self.docs.get_mut(self.active)
                    {
                        // Keep the old encoding if saving failed (e.g. unrepresentable characters).
                        (doc.encoding, doc.bom) = previous;
                    }
                }
            }
        }
    }

    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (shift, other, now) = ctx.input(|i| {
            let mods = i.modifiers;
            // Shift's own press arrives as a key too.
            let pressed = i.events.iter().any(|e| match e {
                egui::Event::Key { key, pressed, .. } => *pressed && !matches!(key, Key::ShiftLeft | Key::ShiftRight),
                egui::Event::Text(_) | egui::Event::PointerButton { pressed: true, .. } => true,
                _ => false,
            });
            (mods.shift, pressed || mods.ctrl || mods.alt || mods.command, i.time)
        });
        if self.double_shift.update(shift, other, now) && self.settings.workbench.double_shift {
            rust_code_core::applog::info("double Shift");
            self.open_everywhere();
        }
        if self.zen_mode
            && !self.find.open
            && self.chord.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.run_command(ctx, Command::ToggleZenMode);
            return;
        }
        let editor_focused = self.editor_focused(ctx);
        // Inside a snippet, Tab goes on to its next place.
        if editor_focused && self.chord.is_none() && self.vim.is_none() && !self.completion_open() {
            let forward = ctx.input(|i| i.key_pressed(Key::Tab) && i.modifiers.is_none());
            let back = ctx.input(|i| i.key_pressed(Key::Tab) && i.modifiers == Modifiers::SHIFT);
            if (forward || back) && self.snippet_step(back) {
                let modifiers = if back { Modifiers::SHIFT } else { Modifiers::NONE };
                ctx.input_mut(|i| i.consume_key(modifiers, Key::Tab));
                return;
            }
        }
        // Tab after a snippet's prefix inserts the snippet; modal editing has its own keys.
        if editor_focused
            && self.chord.is_none()
            && self.vim.is_none()
            && ctx.input(|i| i.key_pressed(Key::Tab) && i.modifiers.is_none())
            && self.expand_snippet_prefix()
        {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
            return;
        }
        // Tab after a backslash name makes its symbol.
        self.pasted = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(text) if editor_focused => Some(text.clone()),
                _ => None,
            })
        });
        // Enter ends a name as a space would, and then does what it always does.
        if editor_focused
            && self.vim.is_none()
            && !self.completion_open()
            && ctx.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none())
        {
            self.symbol_on_tab();
        }
        if editor_focused
            && self.chord.is_none()
            && self.vim.is_none()
            && !self.completion_open()
            && ctx.input(|i| i.key_pressed(Key::Tab) && i.modifiers.is_none())
            && self.symbol_on_tab()
        {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
            return;
        }
        // Tab after an Emmet abbreviation grows it, unless a completion is on offer.
        if editor_focused
            && self.chord.is_none()
            && self.vim.is_none()
            && self.settings.editor.emmet
            && !self.completion_open()
            && ctx.input(|i| i.key_pressed(Key::Tab) && i.modifiers.is_none())
            && self.expand_emmet(true)
        {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
            return;
        }
        // Tab with a multi-line selection indents the lines instead of replacing them.
        if editor_focused && self.chord.is_none() {
            let multi_line = self.docs.get(self.active).is_some_and(|d| {
                let p = d.buffer.primary();
                d.buffer.char_to_line(p.start()) != d.buffer.char_to_line(p.end())
            });
            if multi_line && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                self.run_command(ctx, Command::IndentLines);
                return;
            }
        }
        let terminal_focused = self.terminal.has_focus(ctx);
        // While the delete prompt is up, Enter and Escape are its.
        let explorer_focused = crate::explorer::Explorer::has_focus(ctx) && self.delete_pending.is_none();
        if let Some(cmd) = self.keymap.process(ctx, &mut self.chord, editor_focused, terminal_focused, explorer_focused)
        {
            self.run_command(ctx, cmd);
        }
    }
}
