//! Whether the computer is set to dark mode, for a window that follows it.
//!
//! The window system usually says (winit asks Windows and macOS, and listens for
//! changes). These are the hints for when it can't: the Windows registry before the
//! first frame, and on Linux GTK's and KDE's own settings.

/// `COIMAN_SYSTEM_THEME=dark` or `light` stands in for the system's mode, to try a
/// theme that follows it without changing the computer's settings.
pub fn forced() -> Option<bool> {
    let value = std::env::var("COIMAN_SYSTEM_THEME").ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "dark" => Some(true),
        "light" => Some(false),
        _ => None,
    }
}

/// Asks the computer directly: `Some(true)` for dark mode, `Some(false)` for light, `None`
/// when it can't tell. Starts small programs on Linux, so it's for now and then, not
/// every frame.
pub fn os_prefers_dark() -> Option<bool> {
    forced().or_else(platform::prefers_dark)
}

/// A theme named like `Adwaita-dark`, `Yaru-dark` or `Breeze Dark` is a dark one.
pub fn name_is_dark(name: &str) -> bool {
    name.to_ascii_lowercase().contains("dark")
}

/// The answer to `gsettings get org.gnome.desktop.interface color-scheme`:
/// `'prefer-dark'` or `'prefer-light'`; `'default'` leaves it to the GTK theme.
pub fn from_color_scheme(output: &str) -> Option<bool> {
    match output.trim().trim_matches('\'') {
        "prefer-dark" => Some(true),
        "prefer-light" => Some(false),
        _ => None,
    }
}

/// GTK's `settings.ini` (`~/.config/gtk-3.0` or `gtk-4.0`): asks for dark with
/// `gtk-application-prefer-dark-theme`, or names a dark theme.
pub fn from_gtk_ini(text: &str) -> Option<bool> {
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
        })
    };
    if value("gtk-application-prefer-dark-theme").is_some_and(|p| matches!(p.as_str(), "1" | "true")) {
        return Some(true);
    }
    value("gtk-theme-name").map(|name| name_is_dark(&name))
}

/// KDE's `~/.config/kdeglobals`: the color scheme's name, else how bright its window
/// background is.
pub fn from_kdeglobals(text: &str) -> Option<bool> {
    let mut table = "";
    let mut background = None;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match (table, key.trim()) {
            ("General", "ColorScheme") => return Some(name_is_dark(value)),
            ("Colors:Window", "BackgroundNormal") => {
                let rgb: Vec<u32> = value.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                if let [r, g, b, ..] = rgb[..] {
                    background = Some((r * 299 + g * 587 + b * 114) / 1000 < 128);
                }
            }
            _ => {}
        }
    }
    background
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

    /// `AppsUseLightTheme` under Personalize: 0 when apps are set to dark.
    pub fn prefers_dark() -> Option<bool> {
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let name = wide("AppsUseLightTheme");
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the strings end in a zero, and `data` holds the `size` bytes a DWORD takes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut data).cast(),
                &raw mut size,
            )
        };
        (status == 0).then_some(data == 0)
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod platform {
    use super::{from_color_scheme, from_gtk_ini, from_kdeglobals, name_is_dark};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn gsettings(key: &str) -> Option<String> {
        let out = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", key])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn config_home() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }

    fn read(name: &str) -> Option<String> {
        std::fs::read_to_string(config_home()?.join(name)).ok()
    }

    pub fn prefers_dark() -> Option<bool> {
        // A theme picked for GTK programs by hand, as in `Adwaita:dark`.
        if let Some(theme) = std::env::var("GTK_THEME").ok().filter(|t| !t.trim().is_empty()) {
            return Some(name_is_dark(&theme));
        }
        let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"));
        if kde && let Some(dark) = read("kdeglobals").as_deref().and_then(from_kdeglobals) {
            return Some(dark);
        }
        // GNOME and the desktops on GTK: the style switch, else the theme's name.
        if let Some(dark) = gsettings("color-scheme").as_deref().and_then(from_color_scheme) {
            return Some(dark);
        }
        if let Some(theme) = gsettings("gtk-theme") {
            return Some(name_is_dark(&theme));
        }
        ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
            .into_iter()
            .find_map(|f| read(f).as_deref().and_then(from_gtk_ini))
            .or_else(|| read("kdeglobals").as_deref().and_then(from_kdeglobals))
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "android")))))]
mod platform {
    /// The window system knows here (macOS, Android).
    pub fn prefers_dark() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_style_switch() {
        assert_eq!(from_color_scheme("'prefer-dark'\n"), Some(true));
        assert_eq!(from_color_scheme("'prefer-light'"), Some(false));
        assert_eq!(from_color_scheme("'default'"), None);
        assert!(name_is_dark("'Adwaita-dark'") && name_is_dark("Breeze Dark") && !name_is_dark("Yaru"));
    }

    #[test]
    fn gtk_settings_file() {
        assert_eq!(from_gtk_ini("[Settings]\ngtk-application-prefer-dark-theme=1\n"), Some(true));
        assert_eq!(from_gtk_ini("[Settings]\ngtk-application-prefer-dark-theme = true\n"), Some(true));
        assert_eq!(from_gtk_ini("[Settings]\ngtk-theme-name=Arc-Dark\n"), Some(true));
        assert_eq!(
            from_gtk_ini("[Settings]\ngtk-application-prefer-dark-theme=0\ngtk-theme-name=Adwaita\n"),
            Some(false)
        );
        assert_eq!(from_gtk_ini("[Settings]\ngtk-font-name=Cantarell 11\n"), None);
    }

    #[test]
    fn kde_globals() {
        assert_eq!(from_kdeglobals("[General]\nColorScheme=BreezeDark\n"), Some(true));
        assert_eq!(from_kdeglobals("[General]\nColorScheme=BreezeLight\n"), Some(false));
        assert_eq!(from_kdeglobals("[Colors:Window]\nBackgroundNormal=32,35,38\n"), Some(true));
        assert_eq!(from_kdeglobals("[Colors:Window]\nBackgroundNormal=239,240,241\n"), Some(false));
        assert_eq!(from_kdeglobals("[KDE]\nSingleClick=false\n"), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_registry_answers() {
        // Every Windows 10 and 11 account has the value.
        assert!(platform::prefers_dark().is_some());
    }
}
