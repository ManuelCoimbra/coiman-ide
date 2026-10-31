//! Bytes as people read them, from a file or a debugger, and what debug adapters
//! say about memory and instructions.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;

pub const BYTES_PER_ROW: u64 = 16;
/// Memory is read this many bytes at a time.
pub const PAGE: u64 = 4096;

/// A file that's better shown as bytes: a zero byte early on means it isn't text,
/// unless a byte order mark says it's UTF-16 or UTF-32, where those are normal.
pub fn looks_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut head = vec![0; 8192];
    let Ok(got) = file.read(&mut head) else { return false };
    head.truncate(got);
    let marked = [&[0xFF, 0xFE][..], &[0xFE, 0xFF][..]].iter().any(|bom| head.starts_with(bom))
        || head.starts_with(&[0, 0, 0xFE, 0xFF]);
    !marked && head.contains(&0)
}

/// A page of a file.
pub fn read_page(path: &Path, start: u64) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; PAGE as usize];
    let mut filled = 0;
    while filled < bytes.len() {
        let got = file.read(&mut bytes[filled..])?;
        if got == 0 {
            break;
        }
        filled += got;
    }
    bytes.truncate(filled);
    Ok(bytes)
}

/// A byte as a letter, or a dot when it isn't printable.
pub fn printable(byte: u8) -> char {
    if (0x20..0x7F).contains(&byte) { byte as char } else { '.' }
}

/// `0x1F`, `1f` or a plain number with `#`: what someone types as an address.
/// Hex is the default, as it's how addresses are read everywhere around them.
pub fn parse_address(text: &str) -> Option<u64> {
    let text = text.trim().replace('_', "");
    if let Some(decimal) = text.strip_prefix('#') {
        return decimal.parse().ok();
    }
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(&text);
    u64::from_str_radix(hex, 16).ok()
}

/// What an adapter says it can do, from its `initialize` response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub read_memory: bool,
    pub disassemble: bool,
    pub instruction_stepping: bool,
}

impl Capabilities {
    pub fn parse(body: &Value) -> Self {
        let on = |name: &str| body[name].as_bool().unwrap_or(false);
        Self {
            read_memory: on("supportsReadMemoryRequest"),
            disassemble: on("supportsDisassembleRequest"),
            instruction_stepping: on("supportsSteppingGranularity"),
        }
    }
}

/// A `readMemory` answer: bytes from where it says they start, then what couldn't be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryBlock {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub unreadable: u64,
}

pub fn parse_read_memory(body: &Value) -> Option<MemoryBlock> {
    let address = parse_address(body["address"].as_str()?)?;
    let bytes = match body["data"].as_str() {
        Some(data) => B64.decode(data).ok()?,
        None => Vec::new(),
    };
    Some(MemoryBlock { address, bytes, unreadable: body["unreadableBytes"].as_u64().unwrap_or(0) })
}

/// One line of disassembly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instruction {
    pub address: u64,
    pub address_text: String,
    pub bytes: String,
    pub text: String,
    pub symbol: String,
    pub path: Option<PathBuf>,
    pub line: Option<u32>,
}

pub fn parse_disassembly(body: &Value) -> Vec<Instruction> {
    let list = body["instructions"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out: Vec<Instruction> = Vec::with_capacity(list.len());
    let mut last_path = None;
    for item in list {
        let address_text = item["address"].as_str().unwrap_or("").to_owned();
        // A location holds until another is given, the protocol says.
        let path = item["location"]["path"].as_str().map(PathBuf::from);
        if path.is_some() {
            last_path.clone_from(&path);
        }
        out.push(Instruction {
            address: parse_address(&address_text).unwrap_or(0),
            address_text,
            bytes: item["instructionBytes"].as_str().unwrap_or("").to_owned(),
            text: item["instruction"].as_str().unwrap_or("").to_owned(),
            symbol: item["symbol"].as_str().unwrap_or("").to_owned(),
            path: last_path.clone(),
            line: item["line"].as_u64().map(|l| l as u32),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("rc-hex-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn tells_binary_from_text() {
        let cases: [(&str, &[u8], bool); 5] = [
            ("text", b"plain text\n", false),
            ("empty", b"", false),
            ("zero", b"MZ\x00\x01", true),
            ("utf16", b"\xFF\xFEa\x00b\x00", false),
            ("high", "caf\u{e9}".as_bytes(), false),
        ];
        for (name, bytes, binary) in cases {
            let path = temp(name, bytes);
            assert_eq!(looks_binary(&path), binary, "{name}");
            std::fs::remove_file(path).unwrap();
        }
        assert!(!looks_binary(Path::new("/nope/none")));
    }

    #[test]
    fn reads_pages_of_a_file() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let path = temp("pages", &bytes);
        assert_eq!(read_page(&path, 0).unwrap().len(), 4096);
        let second = read_page(&path, 4096).unwrap();
        assert_eq!(second.len(), 904);
        assert_eq!(second[0], bytes[4096]);
        assert!(read_page(&path, 8192).unwrap().is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_addresses_as_typed() {
        assert_eq!(parse_address("0x1F"), Some(31));
        assert_eq!(parse_address(" 1f "), Some(31));
        assert_eq!(parse_address("0000_00FF"), Some(255));
        assert_eq!(parse_address("#10"), Some(10));
        assert_eq!(parse_address("zz"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn shows_only_printable_bytes() {
        assert_eq!(printable(b'A'), 'A');
        assert_eq!(printable(0), '.');
        assert_eq!(printable(0x7F), '.');
        assert_eq!(printable(0xE9), '.');
    }

    #[test]
    fn reads_capabilities() {
        let all = json!({"supportsReadMemoryRequest": true, "supportsDisassembleRequest": true, "supportsSteppingGranularity": true});
        assert!(Capabilities::parse(&all).read_memory && Capabilities::parse(&all).instruction_stepping);
        assert_eq!(Capabilities::parse(&json!({})), Capabilities::default());
    }

    #[test]
    fn reads_memory_answers() {
        let block = parse_read_memory(&json!({"address": "0x1000", "data": "AQID", "unreadableBytes": 8})).unwrap();
        assert_eq!(block, MemoryBlock { address: 0x1000, bytes: vec![1, 2, 3], unreadable: 8 });
        let none = parse_read_memory(&json!({"address": "0x10", "unreadableBytes": 4})).unwrap();
        assert!(none.bytes.is_empty());
        assert!(parse_read_memory(&json!({"data": "AQID"})).is_none());
        assert!(parse_read_memory(&json!({"address": "0x1", "data": "!!!"})).is_none());
    }

    #[test]
    fn reads_disassembly_and_carries_locations() {
        let body = json!({"instructions": [
            {"address": "0x10", "instructionBytes": "48 89", "instruction": "mov rax, rbx", "symbol": "main", "location": {"path": "/s/a.rs"}, "line": 3},
            {"address": "0x12", "instruction": "ret"},
        ]});
        let list = parse_disassembly(&body);
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].address, list[0].line), (0x10, Some(3)));
        assert_eq!(list[1].path, Some(PathBuf::from("/s/a.rs")));
        assert_eq!(list[1].line, None);
        assert!(parse_disassembly(&json!({})).is_empty());
    }
}
