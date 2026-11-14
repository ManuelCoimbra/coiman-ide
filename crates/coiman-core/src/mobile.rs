//! Running on a phone: Coiman IDE inside Termux on Android (shown through Termux:X11, often
//! in a proot-distro Linux). What's found about that, the places it adds, and the modes of
//! the on-screen keyboard.

use std::path::{Path, PathBuf};

/// Termux's own folder on Android.
const TERMUX_FILES: &str = "/data/data/com.termux/files";

/// This computer's phone side, looked at once.
pub fn phone() -> &'static Phone {
    static PHONE: std::sync::OnceLock<Phone> = std::sync::OnceLock::new();
    PHONE.get_or_init(Phone::here)
}

/// What's known about the phone side of things.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Phone {
    /// Running on Android (in Termux, or a Linux inside it).
    pub android: bool,
    /// Termux's home folder, when it can be reached from here.
    pub termux_home: Option<PathBuf>,
    /// Termux's own programs (`$PREFIX/bin`), when they can be reached from here.
    pub termux_bin: Option<PathBuf>,
    /// The phone's shared storage (Downloads, Documents…), when access was granted.
    pub storage: Option<PathBuf>,
}

impl Phone {
    /// Looks at this computer.
    pub fn here() -> Self {
        Self::at(Path::new("/"), |name| std::env::var(name).ok())
    }

    /// Looks under `root` (the file system's top, a folder in tests), with `env` for variables.
    pub fn at(root: &Path, env: impl Fn(&str) -> Option<String>) -> Self {
        let under = |path: &str| root.join(path.trim_start_matches('/'));
        let termux_files = under(TERMUX_FILES);
        // Termux sets these; a proot-distro Linux keeps Android's /system and its variables.
        let android = env("TERMUX_VERSION").is_some()
            || env("ANDROID_ROOT").is_some()
            || env("PREFIX").is_some_and(|p| p.contains("com.termux"))
            || under("system/build.prop").is_file()
            || termux_files.is_dir();
        if !android {
            return Self::default();
        }
        let home = env("HOME").map(PathBuf::from);
        let storage =
            [home.as_ref().map(|h| h.join("storage/shared")), Some(under("storage/emulated/0")), Some(under("sdcard"))]
                .into_iter()
                .flatten()
                // Listed only once access was granted (termux-setup-storage): it can be read.
                .find(|dir| std::fs::read_dir(dir).is_ok());
        Self {
            android,
            termux_home: Some(termux_files.join("home")).filter(|d| d.is_dir()),
            termux_bin: Some(termux_files.join("usr/bin")).filter(|d| d.is_dir()),
            storage,
        }
    }

    /// The places the File Manager adds: the phone's storage and Termux's home.
    pub fn places(&self) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        if let Some(dir) = &self.storage {
            out.push(("Phone storage".to_owned(), dir.clone()));
        }
        if let Some(dir) = &self.termux_home {
            out.push(("Termux home".to_owned(), dir.clone()));
        }
        out
    }
}

/// When the on-screen keyboard shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardMode {
    /// Never on its own: only when turned on.
    Off,
    /// Always, while there's something to type in.
    On,
    /// On a phone, while something takes typing and no hardware keyboard is being used.
    Auto,
}

impl KeyboardMode {
    /// As the `workbench.soft_keyboard` setting says it; anything else is Auto.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "on" => Self::On,
            _ => Self::Auto,
        }
    }

    pub fn setting(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Auto => "auto",
        }
    }

    /// What the keyboard button changes it to: Off, On, Auto, and round again.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Auto,
            Self::Auto => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::On => "ON",
            Self::Auto => "AUTO",
        }
    }

    /// Whether the keyboard shows: `typing` when something takes text, `phone` when on a
    /// phone, `hardware` when a real keyboard was used lately.
    pub fn shows(self, typing: bool, phone: bool, hardware: bool) -> bool {
        match self {
            Self::Off => false,
            Self::On => typing,
            Self::Auto => typing && phone && !hardware,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coiman_mobile_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_desktop_is_no_phone() {
        let root = temp("desktop");
        let phone = Phone::at(&root, |_| None);
        assert_eq!(phone, Phone::default());
        assert!(phone.places().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn termux_and_its_storage_are_found() {
        let root = temp("termux");
        let home = root.join("data/data/com.termux/files/home");
        std::fs::create_dir_all(home.join("storage/shared/Download")).unwrap();
        std::fs::create_dir_all(root.join("data/data/com.termux/files/usr/bin")).unwrap();
        let home_text = home.display().to_string();
        let phone = Phone::at(&root, |name| match name {
            "TERMUX_VERSION" => Some("0.118".to_owned()),
            "HOME" => Some(home_text.clone()),
            _ => None,
        });
        assert!(phone.android);
        assert_eq!(phone.termux_home.as_deref(), Some(home.as_path()));
        assert_eq!(phone.storage, Some(home.join("storage/shared")));
        assert!(phone.termux_bin.is_some());
        let names: Vec<String> = phone.places().into_iter().map(|p| p.0).collect();
        assert_eq!(names, ["Phone storage", "Termux home"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_linux_in_proot_finds_the_shared_storage() {
        let root = temp("proot");
        std::fs::create_dir_all(root.join("system")).unwrap();
        std::fs::write(root.join("system/build.prop"), "").unwrap();
        std::fs::create_dir_all(root.join("sdcard/Documents")).unwrap();
        let phone = Phone::at(&root, |_| None);
        assert!(phone.android);
        assert_eq!(phone.storage, Some(root.join("sdcard")));
        assert_eq!(phone.termux_home, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keyboard_modes_cycle_and_decide() {
        use KeyboardMode::*;
        assert_eq!([Off.next(), On.next(), Auto.next()], [On, Auto, Off]);
        assert_eq!(KeyboardMode::parse("ON"), On);
        assert_eq!(KeyboardMode::parse("whatever"), Auto);
        assert_eq!(KeyboardMode::parse(Off.setting()), Off);
        assert!(!Off.shows(true, true, false));
        assert!(On.shows(true, false, true));
        assert!(!On.shows(false, true, false));
        assert!(Auto.shows(true, true, false));
        assert!(!Auto.shows(true, false, false), "not on a desktop");
        assert!(!Auto.shows(true, true, true), "not with a hardware keyboard");
    }
}
