//! A large file read a page (1 MiB) at a time, as `object` asks for its parts: only the pages
//! holding the headers, the section table and the symbol names are ever read, and each page
//! once. (`object`'s own `ReadCache` reads each symbol name separately, which through the
//! editor means one call per name.)

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use object::ReadRef;

const PAGE: u64 = 1 << 20;

pub struct Pages<R: Read + Seek> {
    len: u64,
    inner: RefCell<Inner<R>>,
}

struct Inner<R> {
    read: R,
    /// Read pages, by number. Never removed: what was handed out points into them.
    pages: HashMap<u64, Box<[u8]>>,
    /// Pieces that cross pages, by start and size; also never removed.
    pieces: HashMap<(u64, u64), Box<[u8]>>,
}

impl<R: Read + Seek> Pages<R> {
    pub fn new(read: R, len: u64) -> Self {
        Pages { len, inner: RefCell::new(Inner { read, pages: HashMap::new(), pieces: HashMap::new() }) }
    }

    /// How many bytes have been read so far.
    pub fn read_so_far(&self) -> u64 {
        let inner = self.inner.borrow();
        inner.pages.values().chain(inner.pieces.values()).map(|p| p.len() as u64).sum()
    }

    fn page(&self, number: u64) -> Result<&[u8], ()> {
        let mut inner = self.inner.borrow_mut();
        if !inner.pages.contains_key(&number) {
            let start = number * PAGE;
            let size = PAGE.min(self.len.saturating_sub(start)) as usize;
            let data = read_exact(&mut inner.read, start, size)?;
            inner.pages.insert(number, data);
        }
        let page: *const [u8] = &**inner.pages.get(&number).ok_or(())?;
        // SAFETY: the boxed page is never dropped, replaced or changed while `self` lives (the
        // map may move the box, not what it points to), so the slice stays valid for `&self`.
        Ok(unsafe { &*page })
    }

    fn piece(&self, start: u64, size: u64) -> Result<&[u8], ()> {
        let mut inner = self.inner.borrow_mut();
        if !inner.pieces.contains_key(&(start, size)) {
            let data = read_exact(&mut inner.read, start, size as usize)?;
            inner.pieces.insert((start, size), data);
        }
        let piece: *const [u8] = &**inner.pieces.get(&(start, size)).ok_or(())?;
        // SAFETY: as for pages.
        Ok(unsafe { &*piece })
    }
}

fn read_exact(read: &mut impl ReadSeek, start: u64, size: usize) -> Result<Box<[u8]>, ()> {
    let mut data = vec![0u8; size].into_boxed_slice();
    read.seek(SeekFrom::Start(start)).map_err(|_| ())?;
    read.read_exact(&mut data).map_err(|_| ())?;
    Ok(data)
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

impl<'a, R: Read + Seek> ReadRef<'a> for &'a Pages<R> {
    fn len(self) -> Result<u64, ()> {
        Ok(self.len)
    }

    fn read_bytes_at(self, offset: u64, size: u64) -> Result<&'a [u8], ()> {
        if size == 0 {
            return Ok(&[]);
        }
        let end = offset.checked_add(size).ok_or(())?;
        if end > self.len {
            return Err(());
        }
        let number = offset / PAGE;
        if (end - 1) / PAGE == number {
            let page = self.page(number)?;
            let at = (offset - number * PAGE) as usize;
            return Ok(&page[at..at + size as usize]);
        }
        self.piece(offset, size)
    }

    fn read_bytes_at_until(self, range: Range<u64>, delimiter: u8) -> Result<&'a [u8], ()> {
        let end = range.end.min(self.len);
        if range.start >= end {
            return Err(());
        }
        // Mostly inside one page.
        let number = range.start / PAGE;
        let page = self.page(number)?;
        let at = (range.start - number * PAGE) as usize;
        let stop = ((end - number * PAGE) as usize).min(page.len());
        if let Some(i) = page[at..stop].iter().position(|&b| b == delimiter) {
            return Ok(&page[at..at + i]);
        }
        // It runs on into the following pages.
        let mut size = (stop - at) as u64;
        let mut next = number + 1;
        while next * PAGE < end {
            let page = self.page(next)?;
            let stop = ((end - next * PAGE) as usize).min(page.len());
            if let Some(i) = page[..stop].iter().position(|&b| b == delimiter) {
                return self.piece(range.start, size + i as u64);
            }
            size += stop as u64;
            next += 1;
        }
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_and_pieces() {
        let data: Vec<u8> =
            (0..(PAGE * 2 + 100)).map(|i| if i % 1000 == 999 { 0 } else { (i % 251) as u8 + 1 }).collect();
        let pages = Pages::new(std::io::Cursor::new(data.clone()), data.len() as u64);
        let p = &pages;
        assert_eq!(p.read_bytes_at(10, 20).unwrap(), &data[10..30]);
        // Across the first page's end.
        let at = PAGE - 5;
        assert_eq!(p.read_bytes_at(at, 10).unwrap(), &data[at as usize..at as usize + 10]);
        assert!(p.read_bytes_at(data.len() as u64 - 1, 2).is_err());
        // Up to the next 0.
        assert_eq!(p.read_bytes_at_until(0..500_000, 0).unwrap(), &data[..999]);
        let start = PAGE - 300;
        let zero = data[start as usize..].iter().position(|&b| b == 0).unwrap();
        assert_eq!(p.read_bytes_at_until(start..PAGE * 2, 0).unwrap(), &data[start as usize..start as usize + zero]);
        assert!(p.read_bytes_at_until(5..10, 0).is_err());
        assert!(pages.read_so_far() <= PAGE * 2 + 1000);
    }
}
