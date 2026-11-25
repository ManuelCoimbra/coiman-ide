//! What `adb` says, read: the devices attached, logcat's lines in their parts, process ids, and
//! the Java and Kotlin stack frames in log lines with the project file each one means.

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
    /// The project file and line a stack frame in it means, when it's one.
    pub place: Option<(String, u32)>,
}

impl LogLine {
    /// As the panel shows it: the time without the date.
    pub fn shown(&self) -> String {
        let time = self.time.get(6..).unwrap_or(&self.time);
        format!("{time} {:>5} {} {}: {}", self.pid, self.level, self.tag, self.message)
    }
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

/// The levels offered, by name.
pub const LEVELS: [(char, &str); 5] =
    [('V', "Verbose"), ('D', "Debug"), ('I', "Info"), ('W', "Warning"), ('E', "Error")];

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
        place: None,
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

/// A Java or Kotlin frame: `at com.example.app.Main.onCreate(Main.kt:42)`. Its file as a path
/// under the package's folders (`com/example/app/Main.kt`), and the line.
pub fn parse_frame(message: &str) -> Option<(String, u32)> {
    let rest = message.trim_start().strip_prefix("at ")?;
    let (symbol, place) = rest.split_once('(')?;
    let (file, line) = place.split_once(')')?.0.rsplit_once(':')?;
    let line: u32 = line.parse().ok()?;
    if !(file.ends_with(".java") || file.ends_with(".kt")) || file.contains('/') {
        return None;
    }
    // The class's package: the symbol up to its class (the part starting with a capital).
    let mut folders: Vec<&str> = Vec::new();
    for part in symbol.trim().split('.') {
        if part.chars().next().is_some_and(char::is_uppercase) {
            break;
        }
        folders.push(part);
    }
    folders.push(file);
    Some((folders.join("/"), line))
}

/// Which of the project's files (relative paths) a frame's path means: the one sharing the
/// most folders from the end, at least the package's last folder.
pub fn locate(wanted: &str, files: &[String]) -> Option<String> {
    let wanted: Vec<&str> = wanted.split('/').collect();
    let name = *wanted.last()?;
    let mut best: Option<(usize, &String)> = None;
    let mut same_name = 0;
    for file in files {
        let have: Vec<&str> = file.split(['/', '\\']).collect();
        if have.last() != Some(&name) {
            continue;
        }
        same_name += 1;
        let shared = have.iter().rev().zip(wanted.iter().rev()).take_while(|(a, b)| a == b).count();
        if best.is_none_or(|(most, _)| shared > most) {
            best = Some((shared, file));
        }
    }
    let (shared, file) = best?;
    // A folder in common, or the frame named no folders to disagree with.
    (shared >= 2 || wanted.len() == 1 && same_name == 1).then(|| file.clone())
}

/// Built APKs under a module's `build/outputs/apk` (paths relative to the open folder).
pub fn is_apk(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".apk")
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
        assert_eq!(l.shown(), "16:30:01.123  1234 E AndroidRuntime: FATAL EXCEPTION: main");
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

    #[test]
    fn frames_found_in_the_project() {
        let frame = parse_frame("\tat com.example.app.MainActivity.onCreate(MainActivity.kt:42)").unwrap();
        assert_eq!(frame, ("com/example/app/MainActivity.kt".to_owned(), 42));
        let inner = parse_frame("at com.example.app.Main$Inner.run(Main.java:7)").unwrap();
        assert_eq!(inner.0, "com/example/app/Main.java");
        assert!(parse_frame("at android.os.Handler.dispatchMessage(Handler.java:106)").is_some());
        assert!(parse_frame("at java.lang.Thread.run(Native Method)").is_none());
        assert!(parse_frame("FATAL EXCEPTION: main").is_none());
        let files = vec![
            "app/src/main/java/com/example/app/MainActivity.kt".to_owned(),
            "lib/src/main/java/com/other/MainActivity.kt".to_owned(),
            "app/src/main/java/com/example/app/Main.java".to_owned(),
        ];
        assert_eq!(locate(&frame.0, &files).as_deref(), Some("app/src/main/java/com/example/app/MainActivity.kt"));
        assert_eq!(locate(&inner.0, &files).as_deref(), Some("app/src/main/java/com/example/app/Main.java"));
        assert_eq!(locate("android/os/Handler.java", &files), None);
    }
}
