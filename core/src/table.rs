//! CSV and TSV as a table, without holding every cell: a record's place in the
//! text is kept, and its cells are read when they're needed.

use std::cmp::Ordering;
use std::sync::Arc;

const DELIMITERS: [u8; 4] = *b",;\t|";
/// Records looked at for column widths.
const WIDTH_SAMPLE: usize = 1000;

pub fn is_table_file(extension: &str) -> bool {
    matches!(extension, "csv" | "tsv" | "tab")
}

/// Fields on a line, outside quotes.
fn count_on_line(line: &str, delimiter: u8) -> usize {
    let mut quoted = false;
    let mut count = 0;
    for b in line.bytes() {
        if b == b'"' {
            quoted = !quoted;
        } else if b == delimiter && !quoted {
            count += 1;
        }
    }
    count
}

/// The delimiter that splits the first lines most evenly.
pub fn detect_delimiter(text: &str, extension: &str) -> u8 {
    if matches!(extension, "tsv" | "tab") {
        return b'\t';
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(50).collect();
    let score = |d: u8| {
        let counts: Vec<usize> = lines.iter().map(|l| count_on_line(l, d)).collect();
        let first = counts.first().copied().unwrap_or(0);
        if first == 0 {
            return (0, 0);
        }
        (counts.iter().filter(|c| **c == first).count(), first)
    };
    DELIMITERS.into_iter().max_by_key(|d| score(*d)).filter(|d| score(*d).0 > 0).unwrap_or(b',')
}

/// The name of a delimiter, for people.
pub fn delimiter_name(delimiter: u8) -> &'static str {
    match delimiter {
        b',' => "comma",
        b';' => "semicolon",
        b'\t' => "tab",
        b'|' => "pipe",
        _ => "?",
    }
}

/// A CSV file's records.
#[derive(Debug)]
pub struct Table {
    text: Arc<str>,
    pub delimiter: u8,
    /// Byte ranges of the records.
    ranges: Vec<(usize, usize)>,
    /// The line each record starts on, from 0.
    lines: Vec<u32>,
    /// The most fields in a record.
    pub columns: usize,
    /// Records with a different number of fields than the first.
    pub uneven: usize,
    /// The text ends inside a quote, so the last record may have swallowed others.
    pub unclosed_quote: bool,
}

impl Table {
    pub fn parse(text: impl Into<Arc<str>>, delimiter: u8) -> Self {
        let text: Arc<str> = text.into();
        let bytes = text.as_bytes();
        let (mut ranges, mut lines) = (Vec::new(), Vec::new());
        let (mut columns, mut uneven, mut expected) = (0, 0, None);
        let (mut start, mut line, mut start_line, mut fields, mut quoted) = (0, 0u32, 0u32, 1, false);
        let mut end_record = |from: usize, to: usize, first_line: u32, fields: usize| {
            let to = if to > from && bytes[to - 1] == b'\r' { to - 1 } else { to };
            if to == from {
                return;
            }
            ranges.push((from, to));
            lines.push(first_line);
            columns = fields.max(columns);
            match expected {
                None => expected = Some(fields),
                Some(e) if e != fields => uneven += 1,
                Some(_) => {}
            }
        };
        for (at, b) in bytes.iter().enumerate() {
            match *b {
                b'"' => quoted = !quoted,
                b'\n' => {
                    line += 1;
                    if !quoted {
                        end_record(start, at, start_line, fields);
                        start = at + 1;
                        start_line = line;
                        fields = 1;
                    }
                }
                d if d == delimiter && !quoted => fields += 1,
                _ => {}
            }
        }
        end_record(start, bytes.len(), start_line, fields);
        Self { text, delimiter, ranges, lines, columns, uneven, unclosed_quote: quoted }
    }

    /// All records, the first row included.
    pub fn records(&self) -> usize {
        self.ranges.len()
    }

    pub fn line_of(&self, record: usize) -> Option<usize> {
        self.lines.get(record).map(|l| *l as usize)
    }

    /// A record's fields, unquoted.
    pub fn cells(&self, record: usize) -> Vec<String> {
        let Some(&(a, b)) = self.ranges.get(record) else { return Vec::new() };
        split_record(&self.text[a..b], self.delimiter)
    }

    /// Whether the first record reads as column names: none of it is a number,
    /// and there's something for it to name.
    pub fn first_row_is_header(&self) -> bool {
        self.records() > 1 && self.cells(0).iter().all(|c| !c.is_empty() && c.trim().parse::<f64>().is_err())
    }

    /// Widths in characters that suit each column, from the first records.
    pub fn column_widths(&self) -> Vec<usize> {
        let mut widths = vec![4; self.columns];
        // Both ends: numbers and ids tend to grow down a file.
        let tail = self.records().saturating_sub(WIDTH_SAMPLE).max(WIDTH_SAMPLE.min(self.records()));
        for record in (0..self.records().min(WIDTH_SAMPLE)).chain(tail..self.records()) {
            for (i, cell) in self.cells(record).iter().enumerate() {
                let width = cell.chars().count().min(40);
                if width > widths[i] {
                    widths[i] = width;
                }
            }
        }
        widths
    }

    /// The records to show, in order: those from `first` on that have `filter` in a
    /// cell, ignoring case, and sorted by a column if asked.
    pub fn view(&self, first: usize, filter: &str, sort: Option<(usize, bool)>) -> Vec<u32> {
        let needle = filter.trim().to_lowercase();
        let mut kept: Vec<u32> = (first..self.records())
            .filter(|r| {
                needle.is_empty() || {
                    let (a, b) = self.ranges[*r];
                    self.text[a..b].to_lowercase().contains(&needle)
                        && self.cells(*r).iter().any(|c| c.to_lowercase().contains(&needle))
                }
            })
            .map(|r| r as u32)
            .collect();
        if let Some((column, descending)) = sort {
            let mut keyed: Vec<(SortKey, u32)> = kept
                .iter()
                .map(|r| (SortKey::of(self.cells(*r as usize).get(column).map_or("", String::as_str)), *r))
                .collect();
            keyed.sort_by(|a, b| compare(&a.0, &b.0, descending));
            kept = keyed.into_iter().map(|(_, r)| r).collect();
        }
        kept
    }
}

/// How a cell sorts: numbers by value, before words, with empty cells last.
enum SortKey {
    Number(f64),
    Word(String),
    Empty,
}

impl SortKey {
    fn of(cell: &str) -> Self {
        let cell = cell.trim();
        if cell.is_empty() {
            return Self::Empty;
        }
        match cell.parse::<f64>() {
            Ok(n) if n.is_finite() => Self::Number(n),
            _ => Self::Word(cell.to_lowercase()),
        }
    }
}

fn compare(a: &SortKey, b: &SortKey, descending: bool) -> Ordering {
    let order = match (a, b) {
        // Wherever the sort goes, gaps stay at the end.
        (SortKey::Empty, SortKey::Empty) => return Ordering::Equal,
        (SortKey::Empty, _) => return Ordering::Greater,
        (_, SortKey::Empty) => return Ordering::Less,
        (SortKey::Number(x), SortKey::Number(y)) => x.total_cmp(y),
        (SortKey::Number(_), SortKey::Word(_)) => Ordering::Less,
        (SortKey::Word(_), SortKey::Number(_)) => Ordering::Greater,
        (SortKey::Word(x), SortKey::Word(y)) => x.cmp(y),
    };
    if descending { order.reverse() } else { order }
}

/// A record's fields: quotes group, and a doubled quote is a quote.
fn split_record(record: &str, delimiter: u8) -> Vec<String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = record.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            '"' => quoted = !quoted,
            d if !quoted && d.is_ascii() && d as u8 == delimiter => out.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }
    out.push(cell);
    out
}

/// Char ranges of the fields on one line, delimiters left out, for coloring.
pub fn field_ranges(line: &str, delimiter: u8) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut at) = (0, false, 0);
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && c.is_ascii() && c as u8 == delimiter {
            out.push((start, at));
            start = at + 1;
        }
        at += 1;
    }
    out.push((start, at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        Table::parse(text, b',')
    }

    #[test]
    fn detects_the_delimiter() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3\n", "csv"), b',');
        assert_eq!(detect_delimiter("a;b;c\n1,5;2,5;3\n", "csv"), b';');
        assert_eq!(detect_delimiter("a\tb\n1\t2\n", "csv"), b'\t');
        assert_eq!(detect_delimiter("a|b\n1|2\n", "csv"), b'|');
        assert_eq!(detect_delimiter("a,b\n", "tsv"), b'\t');
        assert_eq!(detect_delimiter("\"a,b\";c\n\"d,e\";f\n", "csv"), b';');
        assert_eq!(detect_delimiter("just words\n", "csv"), b',');
        assert_eq!(detect_delimiter("", "csv"), b',');
    }

    #[test]
    fn reads_records_and_cells() {
        let t = table("a,b\r\n1,2\n\n3,4");
        assert_eq!(t.records(), 3);
        assert_eq!(t.columns, 2);
        assert_eq!(t.cells(0), ["a", "b"]);
        assert_eq!(t.cells(1), ["1", "2"]);
        assert_eq!(t.cells(2), ["3", "4"]);
        assert_eq!(t.line_of(2), Some(3));
        assert_eq!(t.cells(9), Vec::<String>::new());
    }

    #[test]
    fn quotes_group_and_escape() {
        let t = table("a,\"b,c\",\"say \"\"hi\"\"\"\n\"two\nlines\",x,y\nz,z,z\n");
        assert_eq!(t.records(), 3);
        assert_eq!(t.cells(0), ["a", "b,c", "say \"hi\""]);
        assert_eq!(t.cells(1), ["two\nlines", "x", "y"]);
        assert_eq!(t.line_of(2), Some(3));
        assert_eq!(t.uneven, 0);
        assert!(!t.unclosed_quote);
    }

    #[test]
    fn says_what_is_wrong() {
        let t = table("a,b\n1,2,3\n4\n5,6\n");
        assert_eq!((t.uneven, t.columns), (2, 3));
        assert!(table("a,\"b\nc,d\n").unclosed_quote);
        assert_eq!(table("").records(), 0);
    }

    #[test]
    fn guesses_a_header() {
        assert!(table("name,age\nAna,3\n").first_row_is_header());
        assert!(!table("1,2\n3,4\n").first_row_is_header());
        assert!(!table("name,age\n").first_row_is_header(), "nothing to name");
        assert!(!table("name,\nAna,3\n").first_row_is_header());
    }

    #[test]
    fn filters_ignoring_case() {
        let t = table("h,i\nApple,1\npear,2\nPINEAPPLE,3\n");
        assert_eq!(t.view(1, "", None), [1, 2, 3]);
        assert_eq!(t.view(1, "APPLE", None), [1, 3]);
        assert_eq!(t.view(0, "h", None), [0]);
        assert_eq!(t.view(1, "none", None), Vec::<u32>::new());
        // A delimiter isn't a value.
        assert_eq!(t.view(1, "e,1", None), Vec::<u32>::new());
    }

    #[test]
    fn sorts_numbers_by_value_with_gaps_last() {
        let t = table("h\n10\n9\n\"\"\nabc\n-1\n");
        assert_eq!(t.view(1, "", Some((0, false))), [5, 2, 1, 4, 3]);
        assert_eq!(t.view(1, "", Some((0, true))), [4, 1, 2, 5, 3]);
    }

    #[test]
    fn sorting_is_stable_and_a_missing_column_is_empty() {
        let t = table("h,x\nb,1\na,1\nc\n");
        assert_eq!(t.view(1, "", Some((1, false))), [1, 2, 3]);
    }

    #[test]
    fn widths_suit_the_text_within_reason() {
        let long = "x".repeat(99);
        let t = table(&format!("ab,{long}\nabcdefg,y\n"));
        assert_eq!(t.column_widths(), [7, 40]);
    }

    #[test]
    fn colors_go_by_field() {
        assert_eq!(field_ranges("ab,\"c,d\",", b','), [(0, 2), (3, 8), (9, 9)]);
        assert_eq!(field_ranges("é;x", b';'), [(0, 1), (2, 3)]);
    }

    #[test]
    fn handles_a_million_rows_in_reasonable_time() {
        let mut text = String::from("id,name,value\n");
        for i in 0..1_000_000 {
            text.push_str(&format!("{i},name{i},{}\n", i % 97));
        }
        let t = Table::parse(text, b',');
        assert_eq!(t.records(), 1_000_001);
        assert_eq!(t.cells(1_000_000)[1], "name999999");
        assert_eq!(t.view(1, "", Some((2, true))).len(), 1_000_000);
    }
}
