//! Tabs for what isn't text in the editor: log views, the byte view, and images. They
//! show in the editor's place, and go on existing behind another tab.

use crate::app::CodeApp;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewTab {
    Log(usize),
    Bytes,
    Image,
    Files,
}

impl CodeApp {
    /// Every such tab: what it is, its title, and whether it's showing.
    pub(crate) fn view_tabs(&self) -> Vec<(ViewTab, String, bool)> {
        let logs = self.log_tabs().into_iter().enumerate();
        let mut tabs: Vec<_> = logs.map(|(i, (title, shown))| (ViewTab::Log(i), title, shown)).collect();
        tabs.extend(self.bytes_tab().map(|(title, shown)| (ViewTab::Bytes, title, shown)));
        tabs.extend(self.image_tab().map(|(title, shown)| (ViewTab::Image, title, shown)));
        tabs.extend(self.file_manager_tab().map(|(title, shown)| (ViewTab::Files, title, shown)));
        tabs
    }

    pub(crate) fn hide_view_tabs(&mut self) {
        self.hide_logs();
        self.hide_bytes();
        self.hide_image();
        self.hide_file_manager();
    }

    pub(crate) fn show_view_tab(&mut self, tab: ViewTab) {
        self.hide_view_tabs();
        match tab {
            ViewTab::Log(i) => self.show_log_tab(i),
            ViewTab::Bytes => self.show_bytes_tab(),
            ViewTab::Image => self.show_image_tab(),
            ViewTab::Files => self.show_file_manager_tab(),
        }
    }

    pub(crate) fn close_view_tab(&mut self, tab: ViewTab) {
        match tab {
            ViewTab::Log(i) => self.close_log_tab(i),
            ViewTab::Bytes => self.close_bytes_tab(),
            ViewTab::Image => self.close_image_tab(),
            ViewTab::Files => self.close_file_manager_tab(),
        }
    }

    /// Ctrl+W: closes what's showing, if that's one of these. Says whether it did.
    pub(crate) fn close_shown_view_tab(&mut self) -> bool {
        let shown = self.view_tabs().into_iter().find(|t| t.2);
        if let Some((tab, ..)) = shown {
            self.close_view_tab(tab);
        }
        shown.is_some()
    }
}
