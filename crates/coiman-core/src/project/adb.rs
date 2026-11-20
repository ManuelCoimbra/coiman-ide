//! Android devices through `adb`: the devices attached, and logcat's lines read into their
//! parts. Coiman talks to no device itself; `adb` does. adb is looked for on PATH, in the
//! Android SDK, and in the config folder's `tools/platform-tools`.

use std::path::PathBuf;

/// A device `adb devices -l` lists.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Device {
    pub serial: String,
    /// `device` (ready), `unauthorized`, `offline`, `recovery`…
    pub state: String,
    pub model: String,
    pub product: String,
}

impl Device {
    pub fn ready(&self) -> bool {
        self.state == "device"
    }

    /// The model when known (`Pixel 8`), else the serial.
    pub fn title(&self) -> String {
        if self.model.is_empty() { self.serial.clone() } else { self.model.replace('_', " ") }
    }

    /// What its state means for the user, when it isn't ready.
    pub fn advice(&self) -> Option<&'static str> {
        match self.state.as_str() {
            "device" => None,
            "unauthorized" => Some("Accept the \"Allow USB debugging?\" prompt on the device, then refresh"),
            "offline" => Some("Offline: unplug and plug it in again, or restart adb"),
            "no permissions" => Some("No permission for the USB device: add a udev rule for it"),
            _ => Some("Not ready for debugging"),
        }
    }
}

/// `adb devices -l`.
pub fn parse_devices(out: &str) -> Vec<Device> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let serial = words.next()?.to_owned();
            // "no permissions" is two words.
            let mut state = words.next()?.to_owned();
            let rest: Vec<&str> = words.collect();
            let mut rest = rest.as_slice();
            if state == "no" && rest.first() == Some(&"permissions") {
                state = "no permissions".to_owned();
                rest = &rest[1..];
            }
            let field = |key: &str| {
                rest.iter().find_map(|w| w.strip_prefix(key).and_then(|v| v.strip_prefix(':'))).unwrap_or("").to_owned()
            };
            Some(Device { serial, state, model: field("model"), product: field("product") })
        })
        .collect()
}

/// A logcat line (`-v threadtime`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// `11-20 16:30:01.123`.
    pub time: String,
    pub pid: u32,
    pub level: char,
    pub tag: String,
    pub message: String,
}

/// How severe a level is: V 0, D 1, I 2, W 3, E 4, F/A 5.
pub fn severity(level: char) -> u8 {
    match level {
        'V' => 0,
        'D' => 1,
        'I' => 2,
        'W' => 3,
        'E' => 4,
        _ => 5,
    }
}

/// One `-v threadtime` line: `MM-DD HH:MM:SS.mmm  PID  TID L TAG: message`. Lines that
/// aren't (`--------- beginning of main`) give None.
pub fn parse_line(line: &str) -> Option<LogLine> {
    let mut parts = line.split_whitespace();
    let date = parts.next()?;
    let clock = parts.next()?;
    if date.len() != 5 || !date.contains('-') || !clock.contains(':') {
        return None;
    }
    let pid = parts.next()?.parse().ok()?;
    let _tid: u32 = parts.next()?.parse().ok()?;
    let level = parts.next()?.chars().next()?;
    // The tag runs to the first ": " after the level; it may hold spaces.
    let after_level = line.find(&format!(" {level} "))? + 3;
    let rest = &line[after_level..];
    let (tag, message) = rest.split_once(": ").unwrap_or((rest.trim_end_matches(':'), ""));
    Some(LogLine {
        time: format!("{date} {clock}"),
        pid,
        level,
        tag: tag.trim().to_owned(),
        message: message.to_owned(),
    })
}

/// What the logcat view shows: lines at least this severe, with this text (in the tag or
/// message, any case), from these processes (none: all).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub min_level: u8,
    pub text: String,
    pub pids: Vec<u32>,
}

impl Filter {
    pub fn keeps(&self, l: &LogLine) -> bool {
        if severity(l.level) < self.min_level {
            return false;
        }
        if !self.pids.is_empty() && !self.pids.contains(&l.pid) {
            return false;
        }
        let t = self.text.trim().to_lowercase();
        t.is_empty() || l.tag.to_lowercase().contains(&t) || l.message.to_lowercase().contains(&t)
    }
}

/// `adb shell pidof <package>`: the process ids.
pub fn parse_pids(out: &str) -> Vec<u32> {
    out.split_whitespace().filter_map(|p| p.parse().ok()).collect()
}

/// Where adb may be, besides PATH.
fn sdk_places() -> Vec<PathBuf> {
    let mut places = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = std::env::var_os(var) {
            places.push(PathBuf::from(sdk).join("platform-tools"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        places.push(PathBuf::from(local).join("Android").join("Sdk").join("platform-tools"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        places.push(PathBuf::from(&home).join("Android").join("Sdk").join("platform-tools"));
        places.push(PathBuf::from(home).join("Library").join("Android").join("sdk").join("platform-tools"));
    }
    if let Some(config) = crate::settings::config_dir() {
        places.push(config.join("tools").join("platform-tools"));
    }
    places
}

/// The adb program, if there is one.
pub fn find_adb() -> Option<PathBuf> {
    crate::language::lsp::servers::which("adb").or_else(|| {
        let name = if cfg!(windows) { "adb.exe" } else { "adb" };
        sdk_places().into_iter().map(|d| d.join(name)).find(|p| p.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_listed() {
        let out = "* daemon started successfully\nList of devices attached\n\
                   db425be55d15           unauthorized transport_id:1\n\
                   emulator-5554          device product:sdk_gphone64 model:sdk_gphone64_x86_64 device:emu64x transport_id:2\n\
                   0123ABC                no permissions (user in plugdev group) usb:1-1\n\n";
        let d = parse_devices(out);
        assert_eq!(d.len(), 3);
        assert!(!d[0].ready() && d[0].advice().unwrap().contains("Allow USB debugging"));
        assert!(d[1].ready() && d[1].advice().is_none());
        assert_eq!(d[1].title(), "sdk gphone64 x86 64");
        assert_eq!(d[0].title(), "db425be55d15");
        assert_eq!(d[2].state, "no permissions");
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }

    #[test]
    fn logcat_lines_and_filters() {
        let l = parse_line("11-20 16:30:01.123  1234  1250 E AndroidRuntime: FATAL EXCEPTION: main").unwrap();
        assert_eq!((l.pid, l.level, l.tag.as_str()), (1234, 'E', "AndroidRuntime"));
        assert_eq!(l.message, "FATAL EXCEPTION: main");
        let spaced = parse_line("11-20 16:30:01.200   900   900 I chatty  : uid=1000 expire 3 lines").unwrap();
        assert_eq!((spaced.tag.as_str(), spaced.message.as_str()), ("chatty", "uid=1000 expire 3 lines"));
        assert!(parse_line("--------- beginning of main").is_none());
        let mut f = Filter { min_level: severity('W'), ..Filter::default() };
        assert!(f.keeps(&l) && !f.keeps(&spaced));
        f.text = "fatal".into();
        assert!(f.keeps(&l));
        f.pids = vec![99];
        assert!(!f.keeps(&l));
        assert_eq!(parse_pids("1234 5678\n"), [1234, 5678]);
    }
}
