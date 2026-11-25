//! What takes room in a program, a library or an object file (ELF, PE, Mach-O, COFF): its
//! sections, its symbols, and the symbols grouped by crate or namespace, read straight from the
//! file with no tool needed. For an embedded ELF program it also says what goes to flash and
//! what to RAM, the way `size` does.
//!
//! The reading goes in steps ([`Scan`]), so a large program never holds a call up for long.

use std::collections::HashMap;

use object::{Object, ObjectSection, ObjectSymbol, ReadRef, SectionKind, SymbolKind, SymbolSection};

/// What a section holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Code,
    ReadOnly,
    Data,
    /// Zero-filled at start (`.bss`): room in memory, none in the file.
    Zeroed,
    Debug,
    Other,
}

impl Part {
    pub fn label(self) -> &'static str {
        match self {
            Part::Code => "code",
            Part::ReadOnly => "read-only data",
            Part::Data => "data",
            Part::Zeroed => "zero-filled",
            Part::Debug => "debug info",
            Part::Other => "other",
        }
    }

    fn of(kind: SectionKind) -> Self {
        match kind {
            SectionKind::Text => Part::Code,
            SectionKind::ReadOnlyData | SectionKind::ReadOnlyDataWithRel | SectionKind::ReadOnlyString => {
                Part::ReadOnly
            }
            SectionKind::Data | SectionKind::Tls => Part::Data,
            SectionKind::UninitializedData | SectionKind::UninitializedTls => Part::Zeroed,
            SectionKind::Debug | SectionKind::DebugString => Part::Debug,
            _ => Part::Other,
        }
    }

    /// Whether it is loaded with the program (not debug info, notes and the like).
    pub fn loaded(self) -> bool {
        matches!(self, Part::Code | Part::ReadOnly | Part::Data | Part::Zeroed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub size: u64,
    pub part: Part,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// Demangled, without Rust's hash.
    pub name: String,
    pub size: u64,
    pub section: String,
    pub code: bool,
    /// The crate or namespace it belongs to (`core`, `std`, `my_app`), or `(no namespace)`.
    pub group: String,
}

/// The `size` view of an embedded program: flash holds code, read-only data and the start
/// values of data; RAM holds data and zero-filled data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub flash: u64,
    pub ram: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// As chosen (relative to the open folder).
    pub path: String,
    /// "ELF executable, Arm", "PE program, x86-64"…
    pub format: String,
    pub file_size: u64,
    pub sections: Vec<Section>,
    /// Biggest first.
    pub symbols: Vec<Symbol>,
    /// Set when the file has no symbols (stripped, or kept in a separate `.pdb`).
    pub no_symbols: Option<&'static str>,
    pub memory: Option<Memory>,
}

impl Report {
    /// Size of what is loaded with the program.
    pub fn loaded(&self) -> u64 {
        self.sections.iter().filter(|s| s.part.loaded()).map(|s| s.size).sum()
    }

    /// Size of each part (code, data…), biggest first, without empty ones.
    pub fn parts(&self) -> Vec<(Part, u64)> {
        let mut out: Vec<(Part, u64)> = Vec::new();
        for s in &self.sections {
            match out.iter_mut().find(|(p, _)| *p == s.part) {
                Some((_, n)) => *n += s.size,
                None => out.push((s.part, s.size)),
            }
        }
        out.retain(|(_, n)| *n > 0);
        out.sort_by_key(|a| std::cmp::Reverse(a.1));
        out
    }

    /// Symbols by group: name, total size, how many. Biggest first.
    pub fn groups(&self) -> Vec<(String, u64, usize)> {
        let mut by: HashMap<&str, (u64, usize)> = HashMap::new();
        for s in &self.symbols {
            let e = by.entry(&s.group).or_default();
            e.0 += s.size;
            e.1 += 1;
        }
        let mut out: Vec<_> = by.into_iter().map(|(g, (n, c))| (g.to_owned(), n, c)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// What's kept of it to compare a later report with.
    pub fn summary(&self) -> Summary {
        Summary { loaded: self.loaded(), groups: self.groups().into_iter().map(|(g, n, _)| (g, n)).collect() }
    }
}

/// An earlier report of the same file, as much as a comparison needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub loaded: u64,
    pub groups: HashMap<String, u64>,
}

/// What changed since an earlier report of the same file: each group's size then and now
/// (0 where it wasn't there), biggest change first, unchanged ones left out.
pub fn group_changes(before: &Summary, after: &Report) -> Vec<(String, u64, u64)> {
    let new: HashMap<String, u64> = after.groups().into_iter().map(|(g, n, _)| (g, n)).collect();
    let mut names: Vec<&String> = before.groups.keys().chain(new.keys()).collect();
    names.sort();
    names.dedup();
    let mut out: Vec<(String, u64, u64)> = names
        .into_iter()
        .map(|g| (g.clone(), before.groups.get(g).copied().unwrap_or(0), new.get(g).copied().unwrap_or(0)))
        .filter(|(_, a, b)| a != b)
        .collect();
    out.sort_by_key(|(_, a, b)| std::cmp::Reverse(a.abs_diff(*b)));
    out
}

/// A size as people read it: `812 B`, `14.2 KiB`, `3.10 MiB`.
pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KiB", b / 1024.0)
    } else {
        format!("{:.2} MiB", b / 1024.0 / 1024.0)
    }
}

/// A change in size: `+1.2 KiB`, `-300 B`.
pub fn human_change(before: u64, after: u64) -> String {
    if after >= before { format!("+{}", human(after - before)) } else { format!("-{}", human(before - after)) }
}

/// A symbol name as people read it: Rust (both manglings) and C++ demangled, Rust's hash dropped.
pub fn demangle(raw: &str) -> String {
    // Mach-O and 32-bit Windows put an underscore before every name.
    for name in [raw, raw.strip_prefix('_').unwrap_or(raw)] {
        if let Ok(d) = rustc_demangle::try_demangle(name) {
            return format!("{d:#}");
        }
    }
    if raw.starts_with("_Z") || raw.starts_with("__Z") {
        let name = raw.strip_prefix('_').filter(|n| n.starts_with("_Z")).unwrap_or(raw);
        if let Ok(sym) = cpp_demangle::Symbol::new(name)
            && let Ok(d) = sym.demangle()
        {
            return d;
        }
    }
    raw.to_owned()
}

/// The crate or namespace of a demangled name: the first path segment, looking inside
/// `<Type as Trait>` at the type. Names without one are `(no namespace)`.
pub fn group_of(name: &str) -> String {
    let mut s = name.trim_start_matches('<').trim_start_matches('&').trim_start_matches("mut ");
    s = s.trim_start_matches("dyn ");
    // A C++ function: drop its return type ("void ns::f()").
    if let Some((head, _)) = s.split_once('(')
        && let Some(space) = head.rfind(' ')
        && !head[..space].contains('<')
    {
        s = &s[space + 1..];
    }
    match s.find("::") {
        Some(i) if i > 0 && s[..i].chars().all(|c| c.is_alphanumeric() || c == '_') => s[..i].to_owned(),
        _ => "(no namespace)".to_owned(),
    }
}

fn format_name<'d, R: ReadRef<'d>>(file: &object::File<'d, R>) -> String {
    let format = match file.format() {
        object::BinaryFormat::Elf => "ELF",
        object::BinaryFormat::Pe => "PE",
        object::BinaryFormat::MachO => "Mach-O",
        object::BinaryFormat::Coff => "COFF",
        object::BinaryFormat::Wasm => "WebAssembly",
        object::BinaryFormat::Xcoff => "XCOFF",
        _ => "object",
    };
    let kind = match file.kind() {
        object::ObjectKind::Executable => "program",
        object::ObjectKind::Dynamic => "shared library",
        object::ObjectKind::Relocatable => "object file",
        object::ObjectKind::Core => "core dump",
        _ => "file",
    };
    let arch = match file.architecture() {
        object::Architecture::X86_64 | object::Architecture::X86_64_X32 => "x86-64",
        object::Architecture::I386 => "x86",
        object::Architecture::Aarch64 | object::Architecture::Aarch64_Ilp32 => "Arm64",
        object::Architecture::Arm => "Arm",
        object::Architecture::Riscv32 => "RISC-V 32",
        object::Architecture::Riscv64 => "RISC-V 64",
        object::Architecture::Avr => "AVR",
        object::Architecture::Xtensa => "Xtensa",
        object::Architecture::Msp430 => "MSP430",
        object::Architecture::Mips | object::Architecture::Mips64 => "MIPS",
        object::Architecture::Wasm32 | object::Architecture::Wasm64 => "wasm",
        other => return format!("{format} {kind}, {other:?}"),
    };
    format!("{format} {kind}, {arch}")
}

/// A defined symbol with a place, found so far.
struct Found {
    section: usize,
    address: u64,
    size: u64,
    /// Demangled.
    name: String,
    text: bool,
}

/// A file being read: its sections first, then its symbols a share at a time ([`Scan::step`]),
/// then [`Scan::finish`] makes the report. Each step parses the file again from `data` (a cache
/// of what was read keeps that cheap).
pub struct Scan {
    path: String,
    format: String,
    file_size: u64,
    pe: bool,
    elf_program: bool,
    sections: Vec<Section>,
    /// By section index: name, start, length, part.
    places: HashMap<usize, (String, u64, u64, Part)>,
    found: Vec<Found>,
    /// Symbols looked at so far.
    next: usize,
}

fn parse<'d, R: ReadRef<'d>>(data: R) -> Result<object::File<'d, R>, String> {
    if data.read_bytes_at(0, 8).is_ok_and(|b| b == b"!<arch>\n") {
        return Err("This is an archive (a static library): choose a program or an object file.".to_owned());
    }
    object::File::parse(data).map_err(|e| format!("Not a program or object file Coiman reads: {e}"))
}

impl Scan {
    /// Reads what the file is and its sections.
    pub fn start<'d, R: ReadRef<'d>>(path: &str, data: R, file_size: u64) -> Result<Scan, String> {
        let file = parse(data)?;
        let mut sections = Vec::new();
        let mut places = HashMap::new();
        for s in file.sections() {
            let name = s.name().unwrap_or("?").to_owned();
            // MinGW's DWARF sections in a PE file come out as "other".
            let part = if name.starts_with(".debug") || name.starts_with(".zdebug") || name.starts_with("__debug") {
                Part::Debug
            } else {
                Part::of(s.kind())
            };
            let size = s.size();
            places.insert(s.index().0, (name.clone(), s.address(), size, part));
            if size > 0 {
                sections.push(Section { name, size, part });
            }
        }
        sections.sort_by_key(|a| std::cmp::Reverse(a.size));
        Ok(Scan {
            path: path.to_owned(),
            format: format_name(&file),
            file_size,
            pe: file.format() == object::BinaryFormat::Pe,
            elf_program: file.format() == object::BinaryFormat::Elf && file.kind() == object::ObjectKind::Executable,
            sections,
            places,
            found: Vec::new(),
            next: 0,
        })
    }

    /// Looks at up to `count` more symbols; true once all have been.
    pub fn step<'d, R: ReadRef<'d>>(&mut self, data: R, count: usize) -> Result<bool, String> {
        let file = parse(data)?;
        let mut seen = 0;
        for sym in file.symbols().skip(self.next).take(count) {
            seen += 1;
            let SymbolSection::Section(index) = sym.section() else { continue };
            if !sym.is_definition() || matches!(sym.kind(), SymbolKind::Section | SymbolKind::File) {
                continue;
            }
            let Some((section, _, _, part)) = self.places.get(&index.0) else { continue };
            // Debug info and the like aren't counted, so their names needn't be read.
            if !part.loaded() {
                continue;
            }
            let Ok(raw) = sym.name() else { continue };
            // Local labels, mapping symbols, and a section's own name.
            if raw.is_empty() || raw.starts_with(".L") || raw.starts_with('$') || section == raw {
                continue;
            }
            self.found.push(Found {
                section: index.0,
                address: sym.address(),
                size: sym.size(),
                name: demangle(raw),
                text: sym.kind() == SymbolKind::Text,
            });
        }
        self.next += seen;
        Ok(seen < count)
    }

    /// Symbols looked at so far.
    pub fn progress(&self) -> usize {
        self.next
    }

    pub fn finish(self) -> Report {
        let Scan { path, format, file_size, pe, elf_program, sections, places, mut found, .. } = self;
        // One per address (aliases share one), the biggest kept.
        found.sort_by(|a, b| (a.section, a.address).cmp(&(b.section, b.address)).then(b.size.cmp(&a.size)));
        found.dedup_by(|b, a| a.section == b.section && a.address == b.address);
        // Formats without sizes (PE, COFF, Mach-O): a symbol runs to the next one in its section.
        let ends: Vec<Option<u64>> = (0..found.len())
            .map(|i| found.get(i + 1).filter(|n| n.section == found[i].section).map(|n| n.address))
            .collect();
        let mut symbols = Vec::with_capacity(found.len());
        for (f, next) in found.into_iter().zip(ends) {
            let Some((section, start, length, part)) = places.get(&f.section) else { continue };
            let size = if f.size > 0 { f.size } else { next.unwrap_or(start + length).saturating_sub(f.address) };
            if size == 0 {
                continue;
            }
            let group = group_of(&f.name);
            symbols.push(Symbol {
                name: f.name,
                size,
                section: section.clone(),
                code: f.text || *part == Part::Code,
                group,
            });
        }
        symbols.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        let no_symbols = symbols.is_empty().then_some(if pe {
            "No symbols in this file: Visual C++ and Rust's MSVC builds keep them in a .pdb beside it. Sections only."
        } else {
            "No symbols in this file (stripped). Sections only."
        });
        let memory = elf_program.then(|| {
            let sum = |parts: &[Part]| sections.iter().filter(|s| parts.contains(&s.part)).map(|s| s.size).sum();
            Memory { flash: sum(&[Part::Code, Part::ReadOnly, Part::Data]), ram: sum(&[Part::Data, Part::Zeroed]) }
        });
        Report { path, format, file_size, sections, symbols, no_symbols, memory }
    }
}

/// Reads a whole file held in memory, for tests and small files.
pub fn analyze(path: &str, data: &[u8]) -> Result<Report, String> {
    let mut scan = Scan::start(path, data, data.len() as u64)?;
    while !scan.step(data, 10_000)? {}
    Ok(scan.finish())
}

/// Whether a file name looks like a program, library or object file by its extension; `None`
/// when it has none (then its first bytes tell, see [`binary_start`]).
pub fn binary_name(name: &str) -> Option<bool> {
    if name.starts_with('.') {
        return Some(false);
    }
    name.rsplit_once('.').map(|(_, ext)| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "exe" | "dll" | "so" | "dylib" | "elf" | "axf" | "out" | "o" | "obj" | "wasm"
        )
    })
}

/// Whether a file starts like an ELF or Mach-O program.
pub fn binary_start(head: &[u8]) -> bool {
    head.starts_with(b"\x7fELF")
        || head.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || head.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
}

/// A project's build folders, where programs are looked for.
pub fn build_folder(name: &str) -> bool {
    matches!(name, "target" | "build" | "out" | "bin" | "Debug" | "Release") || name.starts_with("cmake-build-")
}

/// Folders inside a build folder that aren't looked in: Cargo's intermediate ones hold
/// thousands of files nobody picks.
pub fn skipped_folder(name: &str) -> bool {
    matches!(name, "deps" | "build" | "incremental" | ".fingerprint" | "examples" | "CMakeFiles")
        || name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_groups() {
        assert_eq!(demangle("_ZN4core3fmt5write17h0123456789abcdefE"), "core::fmt::write");
        assert_eq!(demangle("_RNvCs1234_7mycrate4main"), "mycrate::main");
        assert_eq!(
            demangle("_ZNSt6vectorIiSaIiEE9push_backERKi"),
            "std::vector<int, std::allocator<int> >::push_back(int const&)"
        );
        assert_eq!(demangle("main"), "main");
        assert_eq!(group_of("core::fmt::write"), "core");
        assert_eq!(group_of("<alloc::vec::Vec<T> as core::ops::drop::Drop>::drop"), "alloc");
        assert_eq!(group_of("<&mut serde_json::de::Deserializer<R> as x::Y>::f"), "serde_json");
        assert_eq!(group_of("std::vector<int>::push_back(int const&)"), "std");
        assert_eq!(group_of("void app::run(int)"), "app");
        assert_eq!(group_of("main"), "(no namespace)");
        assert_eq!(human(812), "812 B");
        assert_eq!(human(14_540), "14.2 KiB");
        assert_eq!(human_change(2048, 1024), "-1.0 KiB");
        assert_eq!(binary_name("app.EXE"), Some(true));
        assert_eq!(binary_name("app.d"), Some(false));
        assert_eq!(binary_name("firmware"), None);
        assert_eq!(binary_name(".hidden"), Some(false));
        assert!(binary_start(b"\x7fELF...") && !binary_start(b"MZ"));
        assert!(build_folder("cmake-build-debug") && !build_folder("src"));
    }

    /// Real object files from rustc: one for this computer (COFF on Windows, which has no
    /// symbol sizes, ELF on Linux, Mach-O on macOS), and an Arm64 ELF one when that target is
    /// installed. Each has a big function, a small one and a big table.
    #[test]
    fn reads_real_object_files() {
        let dir = std::env::temp_dir().join(format!("coiman-binsize-obj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("sizes.rs");
        std::fs::write(
            &src,
            "#![no_std]\n\
             pub mod sizes {\n\
               #[no_mangle] pub static TABLE: [u32; 1000] = [7; 1000];\n\
               #[inline(never)] pub fn big(x: &mut [u32]) { for i in 0..x.len() { x[i] = x[i].wrapping_mul(31).rotate_left(i as u32) ^ TABLE[i % 1000]; if x[i] % 3 == 0 { x[i] /= 7 } else if x[i] % 5 == 0 { x[i] = x[i].wrapping_sub(11) } } }\n\
               #[inline(never)] pub fn small() -> u32 { 1 }\n\
             }\n",
        )
        .unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let mut made = 0;
        for target in [None, Some("aarch64-linux-android")] {
            let out = dir.join(format!("sizes-{}.o", target.unwrap_or("host")));
            let mut cmd = std::process::Command::new(&rustc);
            cmd.args(["--crate-type", "lib", "--emit", "obj", "-C", "opt-level=1", "-C", "panic=abort", "-o"])
                .arg(&out)
                .arg(&src);
            if let Some(t) = target {
                cmd.args(["--target", t]);
            }
            if !cmd.output().is_ok_and(|o| o.status.success()) {
                assert!(target.is_some(), "rustc couldn't build for this computer");
                continue;
            }
            made += 1;
            let data = std::fs::read(&out).unwrap();
            // A step at a time, the way the panel reads.
            let mut scan = Scan::start("sizes.o", &*data, data.len() as u64).unwrap();
            while !scan.step(&*data, 3).unwrap() {}
            let report = scan.finish();
            assert_eq!(report, analyze("sizes.o", &data).unwrap());
            assert!(report.format.contains("object file"), "{}", report.format);
            assert_eq!(report.no_symbols, None, "{report:?}");
            let size = |name: &str| report.symbols.iter().find(|s| s.name.ends_with(name)).map(|s| s.size);
            let (big, small, table) = (size("sizes::big"), size("sizes::small"), size("TABLE"));
            assert!(big > small && small > Some(0), "{target:?}: big {big:?}, small {small:?}");
            assert_eq!(table, Some(4000), "{target:?}");
            assert!(report.symbols.windows(2).all(|w| w[0].size >= w[1].size));
            let total: u64 = report.groups().iter().map(|g| g.1).sum();
            assert_eq!(total, report.symbols.iter().map(|s| s.size).sum::<u64>());
            assert!(report.groups().iter().any(|(g, _, _)| g == "sizes"), "{:?}", report.groups());
            // Without the big function, that's what changed.
            let mut smaller = report.clone();
            smaller.symbols.retain(|s| !s.name.ends_with("sizes::big"));
            let changes = group_changes(&report.summary(), &smaller);
            assert_eq!(changes[0].0, "sizes");
            assert_eq!(changes[0].1 - changes[0].2, big.unwrap());
        }
        assert!(made >= 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn not_a_binary() {
        assert!(analyze("notes.txt", b"hello").is_err());
        assert!(analyze("lib.a", b"!<arch>\nxx").unwrap_err().contains("archive"));
    }
}

/// How long a large program takes, read the way the panel reads it (a cache over the file):
/// `cargo test --release timing -- --ignored --nocapture`.
#[cfg(test)]
mod timing {
    #[test]
    #[ignore]
    fn timing() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../target/release/coiman-ide.exe");
        let file = std::fs::File::open(path).unwrap();
        let len = file.metadata().unwrap().len();
        let cache = crate::pages::Pages::new(file, len);
        let t = std::time::Instant::now();
        let mut scan = super::Scan::start("coiman-ide.exe", &cache, len).unwrap();
        let mut steps = 0;
        while !scan.step(&cache, 2000).unwrap() {
            steps += 1;
        }
        let report = scan.finish();
        eprintln!(
            "{} symbols, {} groups, {steps} steps, {:?}, {} MB read",
            report.symbols.len(),
            report.groups().len(),
            t.elapsed(),
            cache.read_so_far() >> 20
        );
        assert!(report.symbols.len() > 1000);
    }
}
