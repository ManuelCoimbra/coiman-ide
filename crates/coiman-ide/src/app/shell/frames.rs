//! Frame pacing. The window draws only when something changes (egui repaints on demand).
//! While something keeps changing (a flood of terminal output, an animation) frames are
//! held to the display's refresh rate: vsync alone didn't (231 frames a second on a
//! 120 Hz display). `workbench.max_fps` sets a lower limit, to save power; 0 follows the
//! display. `RUST_CODE_REPAINT_LOG=1` logs how often, and why, the window is drawn.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::CodeApp;

#[derive(Default)]
pub(crate) struct Frames {
    last_end: Option<Instant>,
    /// The window's middle, in physical pixels: its display sets the pace.
    at: (i32, i32),
    /// For the log: since when, how many frames, and their causes.
    window: Option<Instant>,
    count: u32,
    causes: HashMap<String, u32>,
}

/// How long to wait before the next frame may start, to keep to `max_fps`. Nothing when
/// there's no cap, or the last frame ended long enough ago (after idling, say).
pub(crate) fn wait_for(max_fps: u32, since_last_end: Duration, frame_took: Duration) -> Option<Duration> {
    if max_fps == 0 {
        return None;
    }
    let interval = Duration::from_secs_f64(1.0 / f64::from(max_fps));
    let spent = since_last_end.saturating_add(frame_took);
    (spent < interval).then(|| interval - spent)
}

/// The refresh rate of the display holding a point (in physical pixels), read once per
/// display.
pub(crate) fn display_hz_at(x: i32, y: i32) -> Option<u32> {
    use std::sync::Mutex;
    static RATES: Mutex<Vec<(isize, Option<u32>)>> = Mutex::new(Vec::new());
    let monitor = monitor_at(x, y);
    let mut rates = RATES.lock().ok()?;
    if let Some((_, hz)) = rates.iter().find(|(m, _)| *m == monitor) {
        return *hz;
    }
    let hz = read_display_hz(monitor);
    coiman_core::applog::info(&format!("display under the window: {hz:?} Hz"));
    rates.push((monitor, hz));
    hz
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn MonitorFromPoint(point: Point, flags: u32) -> isize;
        pub fn GetMonitorInfoW(monitor: isize, info: *mut u8) -> i32;
        pub fn EnumDisplaySettingsW(device: *const u16, mode: u32, devmode: *mut u8) -> i32;
    }
}

#[cfg(windows)]
fn monitor_at(x: i32, y: i32) -> isize {
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    // SAFETY: a plain call with a point by value.
    unsafe { win::MonitorFromPoint(win::Point { x, y }, MONITOR_DEFAULTTONEAREST) }
}

#[cfg(not(windows))]
fn monitor_at(_x: i32, _y: i32) -> isize {
    0
}

#[cfg(windows)]
fn read_display_hz(monitor: isize) -> Option<u32> {
    // MONITORINFOEXW, as bytes: cbSize at 0, the device's name (32 UTF-16 units) at 40.
    let mut info = [0u8; 104];
    info[0..4].copy_from_slice(&104u32.to_le_bytes());
    // SAFETY: the buffer is a MONITORINFOEXW's size, with its size field set.
    let named = unsafe { win::GetMonitorInfoW(monitor, info.as_mut_ptr()) } != 0;
    let mut device: Vec<u16> =
        info[40..104].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
    device.push(0);
    // DEVMODEW, as bytes: dmSize at 68, dmDisplayFrequency at 184, 220 in all.
    const ENUM_CURRENT_SETTINGS: u32 = u32::MAX;
    let mut devmode = [0u8; 220];
    devmode[68..70].copy_from_slice(&220u16.to_le_bytes());
    let name = if named && device.len() > 1 { device.as_ptr() } else { std::ptr::null() };
    // SAFETY: a NUL-ended name (or null for the main display), and a DEVMODEW-sized buffer.
    let ok = unsafe { win::EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, devmode.as_mut_ptr()) };
    let hz = u32::from_le_bytes(devmode[184..188].try_into().ok()?);
    // 0 and 1 mean "the hardware's default".
    (ok != 0 && hz > 1).then_some(hz)
}

#[cfg(not(windows))]
fn read_display_hz(_monitor: isize) -> Option<u32> {
    None
}

/// The limit frames are held to: the setting, else the rate of the display at `at`.
pub(crate) fn frame_limit(max_fps: u32, at: (i32, i32)) -> u32 {
    if max_fps > 0 { max_fps } else { display_hz_at(at.0, at.1).unwrap_or(0) }
}

impl CodeApp {
    /// Called as a frame starts: notes why it's drawn, for the log.
    pub(crate) fn frame_started(&mut self, ctx: &egui::Context) {
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            let scale = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
            let c = rect.center();
            self.frames.at = ((c.x * scale) as i32, (c.y * scale) as i32);
        }
        if std::env::var_os("RUST_CODE_REPAINT_LOG").is_none() {
            return;
        }
        let now = Instant::now();
        let f = &mut self.frames;
        let start = *f.window.get_or_insert(now);
        f.count += 1;
        for cause in ctx.repaint_causes() {
            *f.causes.entry(format!("{}:{} {}", cause.file, cause.line, cause.reason)).or_default() += 1;
        }
        if now - start >= Duration::from_secs(5) {
            let mut top: Vec<(String, u32)> = f.causes.drain().collect();
            top.sort_by_key(|c| std::cmp::Reverse(c.1));
            let top: Vec<String> = top.into_iter().take(4).map(|(c, n)| format!("{n}x {c}")).collect();
            coiman_core::applog::info(&format!(
                "frames: {:.1}/s over {:.0}s; causes: {}",
                f64::from(f.count) / (now - start).as_secs_f64(),
                (now - start).as_secs_f64(),
                top.join("; ")
            ));
            f.count = 0;
            f.window = Some(now);
        }
    }

    /// Called as a frame ends: holds to the frame rate limit, if there is one.
    pub(crate) fn frame_ended(&mut self, frame_started: Instant) {
        let now = Instant::now();
        let since = self.frames.last_end.map_or(Duration::MAX, |e| frame_started.saturating_duration_since(e));
        let limit = frame_limit(self.settings.workbench.max_fps, self.frames.at);
        if let Some(wait) = wait_for(limit, since, now - frame_started) {
            std::thread::sleep(wait);
        }
        self.frames.last_end = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_wait_only_when_drawn_back_to_back() {
        let ms = Duration::from_millis;
        assert_eq!(wait_for(0, ms(0), ms(1)), None, "no cap");
        // 60 per second: a frame of 4 ms right after another waits about 12.7 ms.
        let wait = wait_for(60, ms(0), ms(4)).unwrap();
        assert!(wait > ms(12) && wait < ms(13), "{wait:?}");
        // After idling, the next frame starts at once.
        assert_eq!(wait_for(60, ms(500), ms(4)), None);
        assert_eq!(wait_for(60, Duration::MAX, ms(4)), None);
        // The setting wins; without it, the display's rate (on Windows, read from it).
        assert_eq!(frame_limit(30, (0, 0)), 30);
        if cfg!(windows) {
            let hz = display_hz_at(0, 0);
            assert!(hz.is_some_and(|hz| (24..=1000).contains(&hz)), "{hz:?}");
            assert_eq!(frame_limit(0, (0, 0)), hz.unwrap());
        }
    }
}
