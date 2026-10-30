//! Search in terminal text. Rows a long line wrapped over count as one line, so a
//! match can span them.

/// One screen row: each character with its column, and whether the line goes on
/// in the next row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub chars: Vec<(char, usize)>,
    pub wraps: bool,
}

/// The (row index, column) of each character of a match.
pub type Match = Vec<(usize, usize)>;

fn fold(c: char, match_case: bool) -> char {
    if match_case { c } else { c.to_lowercase().next().unwrap_or(c) }
}

/// Matches of `query`, top to bottom, without overlaps, at most `limit`.
pub fn find(rows: &[Row], query: &str, match_case: bool, limit: usize) -> Vec<Match> {
    let needle: Vec<char> = query.chars().map(|c| fold(c, match_case)).collect();
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut line: Vec<(char, usize, usize)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        line.extend(row.chars.iter().map(|(c, col)| (fold(*c, match_case), index, *col)));
        if row.wraps && index + 1 < rows.len() {
            continue;
        }
        let mut at = 0;
        while at + needle.len() <= line.len() {
            if line[at..at + needle.len()].iter().map(|c| c.0).eq(needle.iter().copied()) {
                out.push(line[at..at + needle.len()].iter().map(|c| (c.1, c.2)).collect());
                if out.len() >= limit {
                    return out;
                }
                at += needle.len();
            } else {
                at += 1;
            }
        }
        line.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str, wraps: bool) -> Row {
        Row { chars: text.chars().enumerate().map(|(i, c)| (c, i)).collect(), wraps }
    }

    #[test]
    fn finds_across_wrapped_rows() {
        let rows = [row("hello wor", true), row("ld hello", false), row("Hello", false)];
        let found = find(&rows, "world", true, 10);
        assert_eq!(found, [vec![(0, 6), (0, 7), (0, 8), (1, 0), (1, 1)]]);
        assert_eq!(find(&rows, "hello", true, 10).len(), 2);
        assert_eq!(find(&rows, "HELLO", false, 10).len(), 3);
        assert_eq!(find(&rows, "HELLO", false, 2).len(), 2);
        assert_eq!(find(&rows, "", false, 10), Vec::<Match>::new());
        assert_eq!(find(&[row("aaaa", false)], "aa", true, 10).len(), 2);
        // A last row that claims to wrap still ends the search.
        assert_eq!(find(&[row("end", true)], "end", true, 10).len(), 1);
        // Line ends aren't crossed.
        assert_eq!(find(&[row("ab", false), row("cd", false)], "bc", true, 10).len(), 0);
    }

    #[test]
    fn keeps_real_columns() {
        // A wide character takes two columns.
        let rows = [Row { chars: vec![('日', 0), ('x', 2)], wraps: false }];
        assert_eq!(find(&rows, "x", true, 10), [vec![(0, 2)]]);
    }
}
