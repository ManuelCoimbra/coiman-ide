//! Finds fonts installed on the system by reading just the headers of their
//! files: family, style and whether they are fixed-width.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub style: String,
    pub path: PathBuf,
    /// Which font in a collection file.
    pub index: u32,
    pub monospace: bool,
}

fn be16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn be32(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?)))
}

fn read_at(file: &mut File, at: u64, len: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(at)).ok()?;
    let mut buf = vec![0; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// A name from a font's `name` table, in English when there is a choice.
fn font_name(table: &[u8], id: usize) -> Option<String> {
    let (count, strings) = (be16(table, 2)?, be16(table, 4)?);
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = 6 + i * 12;
        let (platform, language) = (be16(table, r)?, be16(table, r + 4)?);
        if be16(table, r + 6)? != id {
            continue;
        }
        let (len, offset) = (be16(table, r + 8)?, be16(table, r + 10)?);
        let bytes = table.get(strings + offset..strings + offset + len)?;
        let (rank, text) = match platform {
            3 => {
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                (if language == 0x409 { 3 } else { 2 }, String::from_utf16_lossy(&units))
            }
            1 => (1, bytes.iter().map(|b| *b as char).collect()),
            _ => continue,
        };
        if best.as_ref().is_none_or(|b| rank > b.0) {
            best = Some((rank, text));
        }
    }
    best.map(|b| b.1)
}

fn read_face(file: &mut File, path: &Path, offset: u64, index: u32) -> Option<Face> {
    let header = read_at(file, offset, 12)?;
    let tables = be16(&header, 4)?;
    let records = read_at(file, offset + 12, tables * 16)?;
    let find = |tag: &[u8; 4]| {
        records.as_chunks::<16>().0.iter().find(|r| &r[..4] == tag).and_then(|r| Some((be32(r, 8)?, be32(r, 12)?)))
    };
    let (name_at, name_len) = find(b"name")?;
    let (post_at, _) = find(b"post")?;
    let post = read_at(file, post_at, 16)?;
    let names = read_at(file, name_at, usize::try_from(name_len).ok()?.min(1 << 20))?;
    // The typographic names group a family's styles; older fonts only have these.
    let family = font_name(&names, 16).or_else(|| font_name(&names, 1))?;
    let style = font_name(&names, 17).or_else(|| font_name(&names, 2)).unwrap_or_default();
    Some(Face { family, style, path: path.to_path_buf(), index, monospace: be32(&post, 12)? != 0 })
}

/// The fonts in one file. A file that isn't a font gives none.
pub fn read_faces(path: &Path) -> Vec<Face> {
    let read = || -> Option<Vec<Face>> {
        let mut file = File::open(path).ok()?;
        let head = read_at(&mut file, 0, 12)?;
        if &head[..4] != b"ttcf" {
            return Some(read_face(&mut file, path, 0, 0).into_iter().collect());
        }
        let count = usize::try_from(be32(&head, 8)?).ok()?.min(256);
        let offsets = read_at(&mut file, 12, count * 4)?;
        let faces = (0..count).filter_map(|i| {
            let at = be32(&offsets, i * 4)?;
            read_face(&mut file, path, at, i as u32)
        });
        Some(faces.collect())
    };
    read().unwrap_or_default()
}

fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(windows) {
        dirs.extend(std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")));
        dirs.extend(std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Microsoft/Windows/Fonts")));
    } else if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        dirs.extend(home.map(|h| h.join("Library/Fonts")));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(h) = home {
            dirs.extend([h.join(".fonts"), h.join(".local/share/fonts")]);
        }
    }
    dirs
}

fn scan_dir(dir: &Path, faces: &mut Vec<Face>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, faces);
        } else if path
            .extension()
            .is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x)))
        {
            faces.extend(read_faces(&path));
        }
    }
}

/// Every font installed. Reads many files: call it off the UI thread.
pub fn installed() -> Vec<Face> {
    let mut faces = Vec::new();
    for dir in font_dirs() {
        scan_dir(&dir, &mut faces);
    }
    faces
}

/// A font with the math and APL symbols the built-in one lacks, if this system
/// has one where these usually are.
pub fn load_symbol_font() -> Option<(Vec<u8>, u32)> {
    let windows = std::env::var_os("WINDIR").map(PathBuf::from).map(|w| w.join("Fonts"));
    let mut places = Vec::new();
    if let Some(fonts) = windows {
        places.push(fonts.join("seguisym.ttf"));
    }
    places.extend(
        [
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
        ]
        .map(PathBuf::from),
    );
    places.into_iter().find_map(|p| std::fs::read(p).ok()).map(|bytes| (bytes, 0))
}

/// Sorted family names of the fixed-width fonts, the only ones code lines up in.
pub fn monospace_families(faces: &[Face]) -> Vec<String> {
    // `@` names are for vertical text.
    let mut names: Vec<String> =
        faces.iter().filter(|f| f.monospace && !f.family.starts_with('@')).map(|f| f.family.clone()).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// The upright, normal-weight face of a family, or else its first.
pub fn pick_face<'a>(faces: &'a [Face], family: &str) -> Option<&'a Face> {
    let mut matching = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).peekable();
    let first = *matching.peek()?;
    Some(
        matching
            .find(|f| matches!(f.style.to_lowercase().as_str(), "regular" | "book" | "normal" | "roman"))
            .unwrap_or(first),
    )
}

/// A font's bytes and which font in the file, or why it couldn't be loaded.
pub type Loaded = Result<(Vec<u8>, u32), String>;

/// The bytes of a font, by family name or file path, and which font in the file.
pub fn load(family: &str) -> Loaded {
    let path = Path::new(family);
    let (path, index) = if path.is_file() {
        (path.to_path_buf(), 0)
    } else {
        let faces = installed();
        let face = pick_face(&faces, family).ok_or_else(|| format!("Font \"{family}\" isn't installed"))?;
        (face.path.clone(), face.index)
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((bytes, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal font file: `name` and `post` tables only.
    fn font(family: &str, style: &str, fixed: bool) -> Vec<u8> {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let (f, s) = (utf16(family), utf16(style));
        let mut name = Vec::new();
        name.extend([0, 0, 0, 2, 0, 30]); // format, 2 records, strings at 6 + 24
        for (id, len, at) in [(1u16, f.len(), 0), (2, s.len(), f.len())] {
            for v in [3u16, 1, 0x409, id, len as u16, at as u16] {
                name.extend(v.to_be_bytes());
            }
        }
        name.extend(&f);
        name.extend(&s);
        let mut post = vec![0u8; 16];
        post[15] = u8::from(fixed);

        let dir_end = 12 + 2 * 16;
        let mut out = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        for (tag, at, len) in [(b"name", dir_end, name.len()), (b"post", dir_end + name.len(), post.len())] {
            out.extend(tag);
            out.extend([0; 4]);
            out.extend((at as u32).to_be_bytes());
            out.extend((len as u32).to_be_bytes());
        }
        out.extend(name);
        out.extend(post);
        out
    }

    #[test]
    fn reads_fonts_and_picks_a_regular_face() {
        let dir = std::env::temp_dir().join(format!("rust-code-fonts-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("a.ttf"), font("Code Mono", "Bold", true)).unwrap();
        std::fs::write(dir.join("nested/b.TTF"), font("Code Mono", "Regular", true)).unwrap();
        std::fs::write(dir.join("c.otf"), font("Prose", "Regular", false)).unwrap();
        std::fs::write(dir.join("broken.ttf"), b"not a font at all").unwrap();
        std::fs::write(dir.join("notes.txt"), font("Ignored", "Regular", true)).unwrap();

        let mut faces = Vec::new();
        scan_dir(&dir, &mut faces);
        assert_eq!(faces.len(), 3);
        assert_eq!(monospace_families(&faces), ["Code Mono"]);
        let regular = pick_face(&faces, "code mono").unwrap();
        assert_eq!(regular.style, "Regular");
        assert!(regular.path.ends_with("b.TTF"));
        assert!(pick_face(&faces, "Nope").is_none());

        // A path works as well as a name.
        let file = dir.join("c.otf");
        assert_eq!(load(file.to_str().unwrap()).unwrap().1, 0);
        assert!(load("Surely No Such Font Family").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_a_font_collection() {
        let dir = std::env::temp_dir().join(format!("rust-code-ttc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Table offsets in a collection count from the start of the file.
        let inner = font("Inner", "Regular", true);
        let shift = 16u32;
        let mut shifted = inner.clone();
        for record in [12usize, 28] {
            let at = u32::from_be_bytes(shifted[record + 8..record + 12].try_into().unwrap()) + shift;
            shifted[record + 8..record + 12].copy_from_slice(&at.to_be_bytes());
        }
        let mut file = b"ttcf\0\x01\0\0\0\0\0\x01".to_vec();
        file.extend(shift.to_be_bytes());
        file.extend(shifted);
        let path = dir.join("set.ttc");
        std::fs::write(&path, file).unwrap();
        let faces = read_faces(&path);
        assert_eq!(faces.len(), 1);
        assert_eq!((faces[0].family.as_str(), faces[0].index, faces[0].monospace), ("Inner", 0, true));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
