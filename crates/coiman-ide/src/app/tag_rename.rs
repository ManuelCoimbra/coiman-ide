//! Renaming a tag renames its partner, as VS Code's linked editing and the Auto Rename
//! Tag extension do: typing in `<div` changes its `</div>` too, and the other way round.

use coiman_core::auto_tags::{self, TagMode};

use super::CodeApp;

/// Files larger than this aren't watched: reading the whole text each edit costs too much.
const LIMIT: usize = 1024 * 1024;

/// The tag name the cursor is in, and its partner, as they were.
#[derive(Clone, Debug)]
pub(super) struct TagWatch {
    uid: u64,
    version: u64,
    head: usize,
    /// Char ranges: the name being edited, and its partner's.
    own: (usize, usize),
    partner: (usize, usize),
    name: String,
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

/// The tag pair at `head`, with the one holding the cursor first.
fn watch_at(uid: u64, version: u64, mode: TagMode, chars: &[char], head: usize) -> Option<TagWatch> {
    let [open, close] = auto_tags::matching_tag(mode, chars, head)?;
    let (own, partner) = if (open.0..=open.1).contains(&head) { (open, close) } else { (close, open) };
    let name: String = chars[own.0..own.1].iter().collect();
    Some(TagWatch { uid, version, head, own, partner, name })
}

impl CodeApp {
    /// After an edit in a tag's name, the same change in its partner.
    pub(super) fn pump_tag_rename(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let mode = TagMode::for_extension(&doc.extension());
        if !self.settings.editor.rename_tags
            || mode == TagMode::Off
            || doc.buffer.len_chars() > LIMIT
            || doc.buffer.read_only
        {
            self.tag_watch = None;
            return;
        }
        let (uid, version) = (doc.uid, doc.buffer.version());
        let sel = doc.buffer.primary();
        let head = sel.head;
        let unchanged = self.tag_watch.as_ref().is_some_and(|w| w.uid == uid && w.version == version && w.head == head);
        if unchanged {
            return;
        }
        let chars: Vec<char> = doc.buffer.rope().chars().collect();
        let mut rename = None;
        // Still in the name being edited, though it may not parse as a tag for now.
        let mut editing = false;
        if let Some(w) = self.tag_watch.as_ref().filter(|w| w.uid == uid && w.version != version && sel.anchor == head)
        {
            // The name now, read from where it starts, if the cursor is still in it.
            let start = w.own.0;
            let before = start.checked_sub(1).and_then(|i| chars.get(i)).copied();
            if matches!(before, Some('<' | '/')) {
                let end = (start..chars.len()).find(|&i| !is_name_char(chars[i])).unwrap_or(chars.len());
                let name: String = chars[start..end].iter().collect();
                editing = (start..=end).contains(&head);
                if editing && name != w.name && !name.is_empty() {
                    let delta = (end - start) as isize - (w.own.1 - w.own.0) as isize;
                    let p = if w.partner.0 > w.own.0 { (w.partner.0 as isize + delta) as usize } else { w.partner.0 };
                    let old_len = w.partner.1 - w.partner.0;
                    let partner_now: String = chars.get(p..p + old_len).map(|s| s.iter().collect()).unwrap_or_default();
                    let ends = chars.get(p + old_len).is_none_or(|c| !is_name_char(*c));
                    if partner_now == w.name && ends {
                        rename = Some((p, p + old_len, name));
                    }
                }
            }
        }
        if let Some((a, b, name)) = rename {
            let doc = &mut self.docs[self.active];
            doc.buffer.apply_edits_joined(vec![(a, b, name)]);
            let chars: Vec<char> = doc.buffer.rope().chars().collect();
            let (version, head) = (doc.buffer.version(), doc.buffer.primary().head);
            self.tag_watch = watch_at(uid, version, mode, &chars, head);
            return;
        }
        // While the name is being typed over (all of it deleted, say), the pair is still
        // the one from before: what's typed next goes to the partner.
        if editing && let Some(w) = self.tag_watch.as_mut() {
            w.version = version;
            w.head = head;
            return;
        }
        self.tag_watch = watch_at(uid, version, mode, &chars, head);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pair_with_the_cursor_first() {
        let chars: Vec<char> = "<div><p>x</p></div>".chars().collect();
        let w = watch_at(1, 1, TagMode::Html, &chars, 2).unwrap();
        assert_eq!(w.name, "div");
        assert_eq!(w.own, (1, 4));
        assert_eq!(w.partner, (15, 18));
        let w = watch_at(1, 1, TagMode::Html, &chars, 16).unwrap();
        assert_eq!(w.own, (15, 18));
        assert_eq!(w.partner, (1, 4));
        assert!(watch_at(1, 1, TagMode::Html, &chars, 9).is_none());
    }
}
