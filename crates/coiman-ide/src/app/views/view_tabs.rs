//! Tabs for what isn't text in the editor: log views, the byte view, and images. They
//! show in the editor's place, and go on existing behind another tab.

use crate::app::CodeApp;

/// Which non-file tab is in front. The others stay open behind it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ActiveView {
    Settings,
    Extension(String),
    Webview(String),
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ViewTab {
    Log(usize),
    Bytes,
    Image,
    Files,
    Settings,
    /// An extension's detail page, open as an editor tab.
    Extension(String),
    /// A webview panel opened by an extension.
    Webview(String),
}

impl CodeApp {
    /// Every such tab: what it is, its title, and whether it's showing.
    pub(crate) fn view_tabs(&self) -> Vec<(ViewTab, String, bool)> {
        let logs = self.log_tabs().into_iter().enumerate();
        let mut tabs: Vec<_> = logs.map(|(i, (title, shown))| (ViewTab::Log(i), title, shown)).collect();
        tabs.extend(self.bytes_tab().map(|(title, shown)| (ViewTab::Bytes, title, shown)));
        tabs.extend(self.image_tab().map(|(title, shown)| (ViewTab::Image, title, shown)));
        tabs.extend(self.file_manager_tab().map(|(title, shown)| (ViewTab::Files, title, shown)));
        if self.settings_tab_open() {
            tabs.push((ViewTab::Settings, "Settings".to_owned(), self.showing_settings_tab()));
        }
        if let Some(found) = &self.marketplace.detail_tab {
            let title = if found.display_name.is_empty() { found.id() } else { found.display_name.clone() };
            tabs.push((ViewTab::Extension(found.id()), format!("Extension: {title}"), self.showing_extension_tab()));
        }
        for (id, (title, _)) in &self.ext_webviews {
            let shown = matches!(&self.active_view, Some(ActiveView::Webview(open)) if open == id);
            tabs.push((ViewTab::Webview(id.clone()), title.clone(), shown));
        }
        tabs
    }

    pub(crate) fn hide_view_tabs(&mut self) {
        self.hide_logs();
        self.hide_bytes();
        self.hide_image();
        self.hide_file_manager();
        // Settings, extension, and webview tabs stay open. Only what is in front changes,
        // so switching back to a file gives the editor its space back without
        // losing those tabs.
        self.active_view = None;
    }

    pub(crate) fn show_view_tab(&mut self, ctx: &egui::Context, tab: ViewTab) {
        self.hide_view_tabs();
        match tab {
            ViewTab::Log(i) => self.show_log_tab(i),
            ViewTab::Bytes => self.show_bytes_tab(),
            ViewTab::Image => self.show_image_tab(),
            ViewTab::Files => self.show_file_manager_tab(),
            ViewTab::Settings => self.open_settings_tab(),
            ViewTab::Extension(id) => {
                if let Some(found) = self.marketplace.detail_tab.clone().filter(|f| f.id() == id) {
                    self.show_extension_tab(ctx, found);
                }
            }
            ViewTab::Webview(id) => {
                self.active_view = Some(ActiveView::Webview(id));
            }
        }
    }

    pub(crate) fn close_view_tab(&mut self, tab: ViewTab) {
        match tab {
            ViewTab::Log(i) => self.close_log_tab(i),
            ViewTab::Bytes => self.close_bytes_tab(),
            ViewTab::Image => self.close_image_tab(),
            ViewTab::Files => self.close_file_manager_tab(),
            ViewTab::Settings => self.close_settings_tab(),
            ViewTab::Extension(id) => {
                self.marketplace.detail_tab = None;
                if matches!(&self.active_view, Some(ActiveView::Extension(open)) if open.as_str() == id.as_str()) {
                    self.active_view = None;
                }
            }
            ViewTab::Webview(id) => {
                self.ext_webviews.remove(&id);
                if matches!(&self.active_view, Some(ActiveView::Webview(open)) if open.as_str() == id.as_str()) {
                    self.active_view = None;
                }
            }
        }
    }

    /// Ctrl+W: closes what's showing, if that's one of these. Says whether it did.
    pub(crate) fn close_shown_view_tab(&mut self) -> bool {
        let shown = self.view_tabs().into_iter().find(|t| t.2);
        if let Some((tab, ..)) = &shown {
            self.close_view_tab(tab.clone());
        }
        shown.is_some()
    }
}
