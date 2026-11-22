//! Packs in the Extensions view: each kind of work's extras, installed or removed at once.
//! Installing one turns its features on (written to `[packs] installed` in the user's
//! settings) and offers its extensions that come with Coiman; removing it turns them off
//! and uninstalls those extensions.

use std::path::PathBuf;

use coiman_core::extensions::plugin::registry;
use coiman_core::packs::{PACKS, Pack};
use egui::{Align, RichText};

use super::{Consent, SHIPPED_SOURCE};
use crate::app::CodeApp;
use crate::ui::theme;

impl CodeApp {
    /// The packs installed now, by id ("all" spelled out).
    fn packs_installed(&self) -> Vec<String> {
        PACKS.iter().filter(|p| self.settings.pack_on(p.id)).map(|p| p.id.to_owned()).collect()
    }

    /// Writes the user's `[packs] installed` and takes it in at once.
    fn write_packs(&mut self, ids: &[String]) {
        let Some(path) = coiman_core::settings::user_settings_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let list = format!("[{}]", ids.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", "));
        let text = coiman_core::settings::set_value_in_text(&text, "packs", "installed", &list);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, text) {
            Ok(()) => {
                self.settings.packs.installed = ids.to_vec();
                self.settings_reload = true;
            }
            Err(e) => self.status = Some(format!("Couldn't save the packs: {e}")),
        }
    }

    /// Where an extension that comes with Coiman is, if it is there.
    fn shipped_folder(&mut self, id: &str) -> Option<PathBuf> {
        let shipped = self.plugins.shipped.get_or_insert_with(super::shipped_extensions);
        shipped.iter().find(|(_, m)| m.id == id).map(|(folder, _)| folder.clone())
    }

    /// Installed from the copy that comes with Coiman, which has changed since: rebuilt for
    /// a newer plugin API, or a new version (same version number or not).
    pub(super) fn shipped_differs(&mut self, id: &str) -> bool {
        let Some(l) = self.plugins.lock.get(id) else { return false };
        if !l.source.starts_with(SHIPPED_SOURCE) {
            return false;
        }
        let (wasm, toml) = (l.wasm_sha256.clone(), l.manifest_sha256.clone());
        let Some(folder) = self.shipped_folder(id) else { return false };
        let (w, t) = self
            .plugins
            .shipped_hashes
            .entry(folder.clone())
            .or_insert_with(|| {
                let hash = |name: &str| std::fs::read(folder.join(name)).map(|b| registry::sha256_hex(&b));
                (hash("plugin.wasm").unwrap_or_default(), hash("plugin.toml").unwrap_or_default())
            })
            .clone();
        !w.is_empty() && (w != wasm || t != toml)
    }

    /// Asks to install an extension that comes with Coiman (the usual consent prompt).
    pub(super) fn offer_shipped(&mut self, id: &str) {
        let Some(folder) = self.shipped_folder(id) else {
            self.status = Some(format!("{id} isn't beside this copy of Coiman"));
            return;
        };
        match registry::read_folder(&folder) {
            Ok(pkg) => {
                // An update keeps what was granted.
                let locked = self.plugins.lock.get(&pkg.manifest.id);
                let granted = locked.map_or_else(|| pkg.manifest.permissions.clone(), |l| l.granted.clone());
                let programs = locked.map(|l| l.granted_programs.clone()).unwrap_or_default();
                self.plugins.consent = Some(Consent {
                    programs,
                    granted,
                    source: format!("{SHIPPED_SOURCE}{}", folder.display()),
                    pkg,
                    key: String::new(),
                    trusted: false,
                    diff: None,
                });
            }
            Err(e) => self.status = Some(format!("Could not read {}: {e}", folder.display())),
        }
    }

    fn install_pack(&mut self, pack: &Pack) {
        let mut ids = self.packs_installed();
        if !ids.iter().any(|i| i == pack.id) {
            ids.push(pack.id.to_owned());
        }
        self.write_packs(&ids);
        // Its first extension not installed yet is offered; each accepted one offers the next.
        if let Some(ext) = pack.extensions.iter().find(|e| self.plugins.lock.get(e).is_none()) {
            self.offer_shipped(ext);
        }
        self.status = Some(format!("Installed the {} pack", pack.name));
    }

    fn remove_pack(&mut self, pack: &Pack) {
        let ids: Vec<String> = self.packs_installed().into_iter().filter(|i| i != pack.id).collect();
        self.write_packs(&ids);
        for ext in pack.extensions {
            if self.plugins.lock.get(ext).is_some_and(|l| l.source.starts_with(SHIPPED_SOURCE)) {
                self.uninstall_plugin(ext);
            }
        }
        self.status = Some(format!("Removed the {} pack", pack.name));
    }

    /// The packs, each with what it brings and Install or Remove.
    pub(super) fn packs_section(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        ui.add_space(8.0);
        ui.label(RichText::new("PACKS").small().strong());
        ui.label(
            RichText::new(
                "Extras by kind of work. One that isn't installed adds nothing: no commands, windows or work.",
            )
            .small()
            .color(pal.text_dim),
        );
        let mut install = None;
        let mut remove = None;
        let mut offer = None;
        for pack in &PACKS {
            let on = self.settings.pack_on(pack.id);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(pack.name).strong());
                if on {
                    ui.label(RichText::new("installed").small().color(pal.text_dim));
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if on {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(pack);
                        }
                    } else if ui.small_button("Install").clicked() {
                        install = Some(pack);
                    }
                });
            });
            ui.label(RichText::new(pack.description).small().color(pal.text_dim));
            for f in pack.features {
                ui.label(RichText::new(format!("• {f}")).small().color(pal.text_dim));
            }
            if on {
                for ext in pack.extensions {
                    if self.plugins.lock.get(ext).is_none() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Its extension {ext} isn't installed")).small());
                            if ui.small_button("Install…").clicked() {
                                offer = Some(*ext);
                            }
                        });
                    }
                }
            }
        }
        if let Some(p) = install {
            self.install_pack(p);
        }
        if let Some(p) = remove {
            self.remove_pack(p);
        }
        if let Some(ext) = offer {
            self.offer_shipped(ext);
        }
    }
}

/// A new install starts with no packs: the user picks the extras they want. (Settings from
/// before packs mean every pack, so an update takes nothing away.)
pub(crate) fn first_run_packs() {
    let Some(path) = coiman_core::settings::user_settings_path() else { return };
    if path.exists() {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, "[packs]\ninstalled = []\n");
}
