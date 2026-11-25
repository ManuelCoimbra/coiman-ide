//! What the monitor shows: lines with their times, bytes in hex, and numbers to plot; and the
//! port's format and line endings.

use std::collections::VecDeque;

/// The speeds offered (any other can be typed).
pub const BAUDS: [u32; 13] =
    [300, 1200, 2400, 4800, 9600, 19_200, 38_400, 57_600, 74_880, 115_200, 230_400, 460_800, 921_600];
pub const FORMATS: [&str; 5] = ["8N1", "8E1", "8O1", "7E1", "8N2"];

/// How the bytes are framed, from `8N1`: data bits, parity (`N`, `E`, `O`), stop bits.
pub fn parse_format(format: &str) -> Option<(u8, char, bool)> {
    let f = format.trim().to_ascii_uppercase();
    let mut chars = f.chars();
    let bits = chars.next()?.to_digit(10)? as u8;
    let parity = chars.next()?;
    let stop = chars.next()?.to_digit(10)?;
    if chars.next().is_some()
        || !(5..=8).contains(&bits)
        || !matches!(parity, 'N' | 'E' | 'O')
        || !(1..=2).contains(&stop)
    {
        return None;
    }
    Some((bits, parity, stop == 2))
}

/// What's added to each line sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ending {
    None,
    #[default]
    Lf,
    Cr,
    CrLf,
}

impl Ending {
    pub const ALL: [Ending; 4] = [Ending::None, Ending::Lf, Ending::Cr, Ending::CrLf];

    pub fn label(self) -> &'static str {
        match self {
            Ending::None => "No line ending",
            Ending::Lf => "Newline (LF)",
            Ending::Cr => "Carriage return (CR)",
            Ending::CrLf => "Both (CR LF)",
        }
    }

    pub fn from_label(label: &str) -> Ending {
        Ending::ALL.into_iter().find(|e| e.label() == label).unwrap_or_default()
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            Ending::None => b"",
            Ending::Lf => b"\n",
            Ending::Cr => b"\r",
            Ending::CrLf => b"\r\n",
        }
    }
}

/// The text received, as lines with the time each started (seconds since the port opened),
/// and the last bytes for the hex view.
#[derive(Default)]
pub struct Received {
    pub lines: Vec<(f64, String)>,
    /// The line still coming (no newline yet).
    partial: Vec<u8>,
    partial_since: f64,
    pub bytes: VecDeque<u8>,
    pub total: u64,
}

/// How many lines and bytes are kept.
pub const KEEP_LINES: usize = 20_000;
pub const KEEP_BYTES: usize = 64 * 1024;

impl Received {
    /// Adds what came; returns how many old lines were dropped to make room.
    pub fn push(&mut self, data: &[u8], now: f64) -> usize {
        self.total += data.len() as u64;
        self.bytes.extend(data);
        while self.bytes.len() > KEEP_BYTES {
            self.bytes.pop_front();
        }
        for &b in data {
            if self.partial.is_empty() {
                self.partial_since = now;
            }
            if b == b'\n' {
                let mut line = std::mem::take(&mut self.partial);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.lines.push((self.partial_since, String::from_utf8_lossy(&line).into_owned()));
            } else {
                self.partial.push(b);
            }
        }
        let extra = self.lines.len().saturating_sub(KEEP_LINES);
        self.lines.drain(..extra);
        extra
    }

    /// The line still coming, if any.
    pub fn partial(&self) -> Option<(f64, String)> {
        let text = String::from_utf8_lossy(&self.partial);
        let text = text.trim_end_matches('\r');
        (!text.is_empty()).then(|| (self.partial_since, text.to_owned()))
    }

    pub fn clear(&mut self) {
        *self = Received::default();
    }

    /// Each line (and the one still coming), with its time first when asked.
    pub fn shown(&self, times: bool) -> Vec<String> {
        self.lines
            .iter()
            .chain(self.partial().iter())
            .map(|(t, line)| if times { format!("{} {line}", clock(*t)) } else { line.clone() })
            .collect()
    }

    /// All of it as text, for saving.
    pub fn text(&self, times: bool) -> String {
        let mut out = String::new();
        for line in self.shown(times) {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

/// Seconds since opening as `mm:ss.mmm` (or `h:mm:ss.mmm`).
pub fn clock(t: f64) -> String {
    let ms = (t * 1000.0).max(0.0) as u64;
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{ms:03}") } else { format!("{m:02}:{s:02}.{ms:03}") }
}

/// Bytes as hex rows of 16, with their text beside.
pub fn hex_rows(bytes: &[u8], offset: u64) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, row)| {
            let hex: Vec<String> = row.iter().map(|b| format!("{b:02X}")).collect();
            let text: String = row.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
            format!("{:08X}  {:<47}  {text}", offset + i as u64 * 16, hex.join(" "))
        })
        .collect()
}

/// The numbers on a line, as the Arduino plotter reads them: separated by commas, spaces or
/// tabs, each maybe named (`temp:21.5`). Words without a number are skipped.
pub fn plot_values(line: &str) -> Vec<(Option<String>, f64)> {
    line.split([',', ' ', '\t', ';'])
        .filter(|p| !p.trim().is_empty())
        .filter_map(|p| match p.split_once([':', '=']) {
            Some((name, v)) => v.trim().parse().ok().map(|v| (Some(name.trim().to_owned()), v)),
            None => p.trim().parse().ok().map(|v| (None, v)),
        })
        .filter(|(_, v): &(Option<String>, f64)| v.is_finite())
        .collect()
}

/// How many readings of each series the plot keeps.
pub const PLOT_POINTS: usize = 500;

/// Plot series: name, values; and how many lines have been read into them.
#[derive(Default)]
pub struct Plot {
    pub series: Vec<(String, VecDeque<f64>)>,
    plotted: usize,
}

impl Plot {
    /// Takes the numbers of each new whole line.
    pub fn update(&mut self, received: &Received, dropped: usize) {
        // Lines dropped from the front (only the last ones are kept) shift the count.
        self.plotted = self.plotted.saturating_sub(dropped).min(received.lines.len());
        for (_, line) in &received.lines[self.plotted..] {
            for (i, (name, value)) in plot_values(line).into_iter().enumerate() {
                let name = name.unwrap_or_else(|| format!("value {}", i + 1));
                let at = match self.series.iter().position(|(n, _)| *n == name) {
                    Some(at) => at,
                    None => {
                        self.series.push((name, VecDeque::new()));
                        self.series.len() - 1
                    }
                };
                let values = &mut self.series[at].1;
                values.push_back(value);
                if values.len() > PLOT_POINTS {
                    values.pop_front();
                }
            }
        }
        self.plotted = received.lines.len();
    }

    pub fn clear(&mut self) {
        *self = Plot::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_endings() {
        assert_eq!(parse_format("8N1"), Some((8, 'N', false)));
        assert_eq!(parse_format("7e1"), Some((7, 'E', false)));
        assert_eq!(parse_format("8O2"), Some((8, 'O', true)));
        assert!(parse_format("9N1").is_none() && parse_format("8X1").is_none() && parse_format("8N1x").is_none());
        assert_eq!(Ending::CrLf.bytes(), b"\r\n");
        assert_eq!(Ending::from_label("Both (CR LF)"), Ending::CrLf);
        assert_eq!(Ending::from_label("?"), Ending::Lf);
    }

    #[test]
    fn lines_hex_and_plots() {
        let mut r = Received::default();
        r.push(b"hel", 1.0);
        assert_eq!(r.partial(), Some((1.0, "hel".to_owned())));
        r.push(b"lo\r\nworld\nnext", 2.0);
        assert_eq!(r.lines, [(1.0, "hello".to_owned()), (2.0, "world".to_owned())]);
        assert_eq!(r.partial(), Some((2.0, "next".to_owned())));
        assert_eq!(r.total, 17);
        assert_eq!(r.text(false), "hello\nworld\nnext\n");
        assert!(r.text(true).starts_with("00:01.000 hello"));
        assert_eq!(clock(3725.5), "1:02:05.500");
        let rows = hex_rows(b"AB\n\x00xyz", 0);
        assert_eq!(rows, ["00000000  41 42 0A 00 78 79 7A                             AB..xyz"]);
        assert_eq!(plot_values("21.5,40"), [(None, 21.5), (None, 40.0)]);
        assert_eq!(
            plot_values("temp:21.5 hum=40\tx"),
            [(Some("temp".to_owned()), 21.5), (Some("hum".to_owned()), 40.0)]
        );
        assert!(plot_values("Booting… done").is_empty());
        let mut plot = Plot::default();
        let mut r = Received::default();
        r.push(b"temp:20 hum:40\ntemp:21\n7\n", 0.0);
        plot.update(&r, 0);
        let names: Vec<&str> = plot.series.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["temp", "hum", "value 1"]);
        assert_eq!(plot.series[0].1, [20.0, 21.0]);
        // Only the new lines are read the next time.
        r.push(b"temp:22\n", 1.0);
        plot.update(&r, 0);
        assert_eq!(plot.series[0].1, [20.0, 21.0, 22.0]);
    }
}
