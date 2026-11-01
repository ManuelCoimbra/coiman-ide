//! Tabs for what isn't text in the editor: log views, the byte view, and images. They
//! show in the editor's place, and go on existing behind another tab.

use super::CodeApp;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewTab {
    Log(usize),
    Bytes,
    Image,
}

impl CodeApp {
    /// Every such tab: what it is, its title, and whether it's showing.
    pub(super) fn view_tabs(&self) -> Vec<(ViewTab, String, bool)> {
        let logs = self.log_tabs().into_iter().enumerate();
        let mut tabs: Vec<_> = logs.map(|(i, (title, shown))| (ViewTab::Log(i), title, shown)).collect();
        tabs.extend(self.bytes_tab().map(|(title, shown)| (ViewTab::Bytes, title, shown)));
        tabs.extend(self.image_tab().map(|(title, shown)| (ViewTab::Image, title, shown)));
        tabs
    }

    pub(super) fn hide_view_tabs(&mut self) {
        self.hide_logs();
        self.hide_bytes();
        self.hide_image();
    }

    pub(super) fn show_view_tab(&mut self, tab: ViewTab) {
        self.hide_view_tabs();
        match tab {
            ViewTab::Log(i) => self.show_log_tab(i),
            ViewTab::Bytes => self.show_bytes_tab(),
            ViewTab::Image => self.show_image_tab(),
        }
    }

    pub(super) fn close_view_tab(&mut self, tab: ViewTab) {
        match tab {
            ViewTab::Log(i) => self.close_log_tab(i),
            ViewTab::Bytes => self.close_bytes_tab(),
            ViewTab::Image => self.close_image_tab(),
        }
    }

    /// Ctrl+W: closes what's showing, if that's one of these. Says whether it did.
    pub(super) fn close_shown_view_tab(&mut self) -> bool {
        let shown = self.view_tabs().into_iter().find(|t| t.2);
        if let Some((tab, ..)) = shown {
            self.close_view_tab(tab);
        }
        shown.is_some()
    }
}
