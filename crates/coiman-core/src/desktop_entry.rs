//! The launcher entry Linux desktops find a window's icon through. Under Wayland a window
//! can't hand the desktop its icon: the taskbar looks for a `.desktop` file named after the
//! window's app id and shows that file's icon, or a default one when there is none.
//!
//! The packages (deb, rpm, Arch, Flatpak, Snap) install that file. Coiman run from the
//! tar.gz, an AppImage or a build of its own has none, so it writes one, with its icons, into
//! the user's own folders (`~/.local/share`). It never touches a file it didn't write.

use std::path::{Path, PathBuf};

/// The app id: the window's Wayland app id and X11 class, and the launcher file's name.
pub const APP_ID: &str = "io.github.manuelcoimbra.CoimanIde";
/// The line that marks a launcher file as written by the app, so it may be rewritten.
const MARK: &str = "X-Coiman-Written-By-App=true";

/// The app id for this run. Snapd installs the launcher as `<snap>_<app>.desktop`, so in a
/// snap the window takes that name for the desktop to match it.
pub fn app_id(snap_instance: Option<&str>) -> String {
    match snap_instance.filter(|s| !s.is_empty()) {
        Some(snap) => format!("{snap}_coiman-ide"),
        None => APP_ID.to_owned(),
    }
}

/// Where launcher files are looked for, as the XDG base directory rules say.
#[derive(Clone, Debug)]
pub struct DataDirs {
    /// `$XDG_DATA_HOME`, or `~/.local/share`: where the app writes.
    pub home: PathBuf,
    /// `$XDG_DATA_DIRS`, or `/usr/local/share` and `/usr/share`.
    pub system: Vec<PathBuf>,
}

impl DataDirs {
    /// From the environment; `None` without a home folder.
    pub fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let home = match var("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(var("HOME")?).join(".local/share"),
        };
        let system = match var("XDG_DATA_DIRS") {
            Some(dirs) => std::env::split_paths(&dirs).collect(),
            None => vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")],
        };
        Some(Self { home, system })
    }
}

/// What the launcher folders hold for the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A package's file (or one the user wrote): left alone.
    Installed(PathBuf),
    /// One the app wrote, with its `Exec=` line.
    Ours {
        path: PathBuf,
        exec: String,
    },
    Nothing,
}

pub fn find(dirs: &DataDirs) -> Found {
    let name = format!("{APP_ID}.desktop");
    for dir in std::iter::once(&dirs.home).chain(&dirs.system) {
        let path = dir.join("applications").join(&name);
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if !text.lines().any(|l| l.trim() == MARK) {
            return Found::Installed(path);
        }
        let exec = text.lines().find_map(|l| l.strip_prefix("Exec=")).unwrap_or_default().to_owned();
        return Found::Ours { path, exec };
    }
    Found::Nothing
}

/// The `Exec=` value starting `program`: quoted, with the characters the desktop entry
/// rules reserve escaped, then the files to open. The rules escape twice: a backslash for the
/// quoting, then every backslash doubled for the file's string values.
pub fn exec_line(program: &Path) -> String {
    let mut quoted = String::from("\"");
    for c in program.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted.push_str(" %F");
    quoted
}

/// The launcher file's text.
pub fn entry_text(program: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Coiman IDE\n\
         GenericName=Code Editor\n\
         Comment=Edit, run, test and debug code\n\
         Exec={}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Development;IDE;TextEditor;\n\
         StartupNotify=true\n\
         StartupWMClass={APP_ID}\n\
         {MARK}\n",
        exec_line(program)
    )
}

/// The app's icons: PNGs by size, and the SVG.
pub struct Icons<'a> {
    pub png: &'a [(u32, &'a [u8])],
    pub svg: &'a [u8],
}

/// Writes the launcher file and icons into the user's folders, unless a package installed
/// them or the app already wrote them for this program. Returns whether it wrote anything.
pub fn ensure(dirs: &DataDirs, program: &Path, icons: &Icons) -> std::io::Result<bool> {
    let wanted = exec_line(program);
    match find(dirs) {
        Found::Installed(_) => return Ok(false),
        Found::Ours { exec, .. } if exec == wanted => return Ok(false),
        _ => {}
    }
    let hicolor = dirs.home.join("icons/hicolor");
    for (size, bytes) in icons.png {
        let dir = hicolor.join(format!("{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{APP_ID}.png")), bytes)?;
    }
    let scalable = hicolor.join("scalable/apps");
    std::fs::create_dir_all(&scalable)?;
    std::fs::write(scalable.join(format!("{APP_ID}.svg")), icons.svg)?;
    let apps = dirs.home.join("applications");
    std::fs::create_dir_all(&apps)?;
    std::fs::write(apps.join(format!("{APP_ID}.desktop")), entry_text(program))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_take_the_name_snapd_gives_their_launcher() {
        assert_eq!(app_id(None), APP_ID);
        assert_eq!(app_id(Some("")), APP_ID);
        assert_eq!(app_id(Some("coiman-ide")), "coiman-ide_coiman-ide");
    }

    #[test]
    fn program_paths_are_quoted_as_the_desktop_rules_say() {
        assert_eq!(exec_line(Path::new("/opt/coiman/bin/coiman-ide")), "\"/opt/coiman/bin/coiman-ide\" %F");
        assert_eq!(
            exec_line(Path::new("/home/a b/$x/100%/Coiman.AppImage")),
            r#""/home/a b/\\$x/100%%/Coiman.AppImage" %F"#
        );
        assert_eq!(exec_line(Path::new(r#"/a\b"c"#)), r#""/a\\\\b\\"c" %F"#);
    }

    #[test]
    fn writes_its_own_launcher_and_leaves_a_packaged_one_alone() {
        let root = std::env::temp_dir().join(format!("coiman-desktop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = DataDirs { home: root.join("home"), system: vec![root.join("usr")] };
        let icons = Icons { png: &[(48, b"png48"), (256, b"png256")], svg: b"<svg/>" };
        let program = Path::new("/opt/Coiman.AppImage");

        assert_eq!(find(&dirs), Found::Nothing);
        assert!(ensure(&dirs, program, &icons).unwrap(), "nothing there: written");
        let file = dirs.home.join(format!("applications/{APP_ID}.desktop"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("Exec=\"/opt/Coiman.AppImage\" %F\n") && text.contains(&format!("Icon={APP_ID}\n")));
        assert_eq!(
            std::fs::read(dirs.home.join(format!("icons/hicolor/256x256/apps/{APP_ID}.png"))).unwrap(),
            b"png256"
        );
        assert!(dirs.home.join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg")).is_file());
        assert!(!ensure(&dirs, program, &icons).unwrap(), "same program: left as it is");
        assert!(ensure(&dirs, Path::new("/new/place/Coiman.AppImage"), &icons).unwrap(), "moved: rewritten");
        assert!(std::fs::read_to_string(&file).unwrap().contains("/new/place/"));

        // A package's file wins, even with the app's own still in the home folder.
        std::fs::remove_file(&file).unwrap();
        let packaged = root.join(format!("usr/applications/{APP_ID}.desktop"));
        std::fs::create_dir_all(packaged.parent().unwrap()).unwrap();
        std::fs::write(&packaged, "[Desktop Entry]\nExec=coiman-ide %F\n").unwrap();
        assert_eq!(find(&dirs), Found::Installed(packaged));
        assert!(!ensure(&dirs, program, &icons).unwrap());
        assert!(!file.exists(), "nothing written beside a package");
        let _ = std::fs::remove_dir_all(&root);
    }
}
