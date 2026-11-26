//! A firmware's memory map: the memory regions a linker script (`MEMORY { FLASH … RAM … }`,
//! as in `memory.x`) or a GNU linker map ("Memory Configuration") declares, and what of the
//! program sits in each. A section takes room where it runs (its VMA), and a section with
//! contents whose load address differs (`.data` copied from flash at start) takes room where
//! it's loaded (its LMA) too.

/// A memory region: `FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 256K`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub origin: u64,
    pub length: u64,
}

impl Region {
    fn holds(&self, address: u64) -> bool {
        address >= self.origin && address - self.origin < self.length
    }
}

/// A section of the program as it sits in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub name: String,
    /// Where it runs.
    pub vma: u64,
    /// Where it's loaded from (the same as `vma` unless copied at start).
    pub lma: u64,
    pub size: u64,
    /// It has contents in the file (not zero-filled, like `.bss`).
    pub contents: bool,
}

/// How a section takes room in a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// It runs there.
    Runs,
    /// Its start values are kept there and copied elsewhere at start.
    Loaded,
}

/// A region and what's in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub region: Region,
    pub used: u64,
    /// Section, size, how; biggest first.
    pub sections: Vec<(String, u64, How)>,
}

/// Places sections in regions; also returns the sections that fall in none.
pub fn place(regions: &[Region], sections: &[Placed]) -> (Vec<Use>, Vec<Placed>) {
    let mut uses: Vec<Use> = regions.iter().map(|r| Use { region: r.clone(), used: 0, sections: Vec::new() }).collect();
    let mut outside = Vec::new();
    for s in sections.iter().filter(|s| s.size > 0) {
        let mut placed = false;
        if let Some(u) = uses.iter_mut().find(|u| u.region.holds(s.vma)) {
            u.used += s.size;
            u.sections.push((s.name.clone(), s.size, How::Runs));
            placed = true;
        }
        if s.contents
            && s.lma != s.vma
            && let Some(u) = uses.iter_mut().find(|u| u.region.holds(s.lma))
        {
            u.used += s.size;
            u.sections.push((s.name.clone(), s.size, How::Loaded));
            placed = true;
        }
        if !placed {
            outside.push(s.clone());
        }
    }
    for u in &mut uses {
        u.sections.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }
    (uses, outside)
}

/// A number or a small sum in a linker script: `0x08000000`, `256K`, `1M`, `64 * 1024`,
/// `0x20000000 + 16K`.
pub fn eval(expr: &str) -> Option<u64> {
    let expr = expr.trim();
    if let Some((i, op)) = split_last(expr, &['+', '-']) {
        let (a, b) = (eval(&expr[..i])?, eval(&expr[i + 1..])?);
        return if op == '+' { a.checked_add(b) } else { a.checked_sub(b) };
    }
    if let Some((i, _)) = split_last(expr, &['*']) {
        return eval(&expr[..i])?.checked_mul(eval(&expr[i + 1..])?);
    }
    let expr = expr.trim_matches(|c| c == '(' || c == ')').trim();
    let (digits, scale) = match expr.char_indices().last() {
        Some((i, 'K' | 'k')) => (&expr[..i], 1024),
        Some((i, 'M' | 'm')) => (&expr[..i], 1024 * 1024),
        _ => (expr, 1),
    };
    let digits = digits.trim();
    let value = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    value.checked_mul(scale)
}

/// The last of `ops` outside parentheses (left to right evaluation), and which it was.
fn split_last(expr: &str, ops: &[char]) -> Option<(usize, char)> {
    let mut depth = 0i32;
    let mut found = None;
    for (i, c) in expr.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if depth == 0 && ops.contains(&c) && i > 0 => found = Some((i, c)),
            _ => {}
        }
    }
    found
}

/// The regions a linker script's `MEMORY { … }` declares.
pub fn regions_from_script(text: &str) -> Vec<Region> {
    // Comments out of the way.
    let mut clean = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("/*") {
        clean.push_str(&rest[..i]);
        rest = rest[i..].find("*/").map_or("", |j| &rest[i + j + 2..]);
    }
    clean.push_str(rest);
    let clean: String = clean.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let Some(start) = find_word(&clean, "MEMORY") else { return Vec::new() };
    let Some(open) = clean[start..].find('{').map(|i| start + i + 1) else { return Vec::new() };
    let Some(close) = clean[open..].find('}').map(|i| open + i) else { return Vec::new() };
    let body = &clean[open..close];
    let mut out = Vec::new();
    // Each region: NAME [(attrs)] : ORIGIN = …, LENGTH = …
    for entry in split_regions(body) {
        let Some((head, tail)) = entry.split_once(':') else { continue };
        let name = head.split('(').next().unwrap_or("").trim().to_owned();
        let (mut origin, mut length) = (None, None);
        for part in tail.split(',') {
            let Some((key, value)) = part.split_once('=') else { continue };
            match key.trim().to_ascii_lowercase().as_str() {
                "origin" | "org" | "o" => origin = eval(value),
                "length" | "len" | "l" => length = eval(value),
                _ => {}
            }
        }
        if let (false, Some(origin), Some(length)) = (name.is_empty(), origin, length) {
            out.push(Region { name, origin, length });
        }
    }
    out
}

/// The MEMORY body cut into one text per region: each starts at the name before a `:`
/// (`FLASH (rx) :`), so regions on one line split too.
fn split_regions(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut starts = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b':' {
            continue;
        }
        // Back over spaces and an `(attributes)` part, then over the name.
        let mut j = i;
        while j > 0 && bytes[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        if j > 0 && bytes[j - 1] == b')' {
            while j > 0 && bytes[j - 1] != b'(' {
                j -= 1;
            }
            j = j.saturating_sub(1);
            while j > 0 && bytes[j - 1].is_ascii_whitespace() {
                j -= 1;
            }
        }
        let end = j;
        while j > 0 && (bytes[j - 1].is_ascii_alphanumeric() || bytes[j - 1] == b'_') {
            j -= 1;
        }
        if j < end {
            starts.push(j);
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &s)| body[s..starts.get(k + 1).copied().unwrap_or(body.len())].to_owned())
        .collect()
}

fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| from + i) {
        let before = text[..i].chars().last().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after = text[i + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if before && after {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}

/// The regions a GNU linker map lists under "Memory Configuration".
pub fn regions_from_gnu_map(text: &str) -> Vec<Region> {
    let mut lines = text.lines().skip_while(|l| !l.trim_start().starts_with("Memory Configuration"));
    lines.next();
    let mut out = Vec::new();
    for line in lines {
        let t = line.trim();
        if t.starts_with("Linker script and memory map") {
            break;
        }
        let words: Vec<&str> = t.split_whitespace().collect();
        if words.len() < 3 || words[0] == "Name" || words[0] == "*default*" {
            continue;
        }
        if let (Some(origin), Some(length)) = (eval(words[1]), eval(words[2])) {
            out.push(Region { name: words[0].to_owned(), origin, length });
        }
    }
    out
}

/// The regions in a file: a GNU map's, or a linker script's.
pub fn regions_from(text: &str) -> Vec<Region> {
    let from_map = regions_from_gnu_map(text);
    if from_map.is_empty() { regions_from_script(text) } else { from_map }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(eval("0x08000000"), Some(0x0800_0000));
        assert_eq!(eval("256K"), Some(256 * 1024));
        assert_eq!(eval("2M"), Some(2 << 20));
        assert_eq!(eval("64 * 1024"), Some(65536));
        assert_eq!(eval("0x20000000 + 16K"), Some(0x2000_4000));
        assert_eq!(eval("(128K) - 4K"), Some(124 * 1024));
        assert_eq!(eval("ORIGIN(FLASH)"), None);
    }

    #[test]
    fn scripts_and_maps() {
        let script = "/* memory.x */\nMEMORY\n{\n  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 256K\n  RAM : org = 0x20000000, len = 64K /* SRAM1 */\n  CCMRAM (rw) : ORIGIN = 0x10000000, LENGTH = 64 * 1024\n}\n_stack_start = ORIGIN(RAM) + LENGTH(RAM);\n";
        let r = regions_from_script(script);
        assert_eq!(
            r,
            [
                Region { name: "FLASH".into(), origin: 0x0800_0000, length: 256 * 1024 },
                Region { name: "RAM".into(), origin: 0x2000_0000, length: 64 * 1024 },
                Region { name: "CCMRAM".into(), origin: 0x1000_0000, length: 65536 },
            ]
        );
        // One line holding everything.
        let one = "MEMORY { FLASH : ORIGIN = 0, LENGTH = 1K RAM : ORIGIN = 0x100, LENGTH = 512 }";
        assert_eq!(regions_from_script(one).len(), 2);
        assert!(regions_from_script("SECTIONS { }").is_empty());

        let map = "Memory Configuration\n\nName             Origin             Length             Attributes\nFLASH            0x0000000008000000 0x0000000000080000 xr\nRAM              0x0000000020000000 0x0000000000020000 xrw\n*default*        0x0000000000000000 0xffffffffffffffff\n\nLinker script and memory map\n";
        let r = regions_from(map);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].name.as_str(), r[0].length), ("FLASH", 512 * 1024));
        assert_eq!(regions_from(script).len(), 3);
    }

    #[test]
    fn data_counts_in_flash_and_ram() {
        let regions = regions_from_script(
            "MEMORY { FLASH : ORIGIN = 0x08000000, LENGTH = 64K\n RAM : ORIGIN = 0x20000000, LENGTH = 20K }",
        );
        let s = |name: &str, vma, lma, size, contents| Placed { name: name.into(), vma, lma, size, contents };
        let sections = vec![
            s(".text", 0x0800_0000, 0x0800_0000, 60, true),
            s(".rodata", 0x0800_003c, 0x0800_003c, 1024, true),
            s(".data", 0x2000_0000, 0x0800_0484, 4, true),
            s(".bss", 0x2000_0004, 0x2000_0004, 1024, false),
            s(".comment", 0, 0, 153, true),
        ];
        let (uses, outside) = place(&regions, &sections);
        assert_eq!(uses[0].used, 60 + 1024 + 4);
        assert_eq!(uses[1].used, 4 + 1024);
        assert!(uses[0].sections.contains(&(".data".to_owned(), 4, How::Loaded)));
        assert!(uses[1].sections.contains(&(".data".to_owned(), 4, How::Runs)));
        assert_eq!(uses[0].sections[0].0, ".rodata");
        assert_eq!(outside.len(), 1);
    }
}
