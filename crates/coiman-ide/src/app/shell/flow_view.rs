//! Architecture: Show Flow of Function at Cursor: the function's control flow as a Mermaid
//! flowchart, opened with its preview.

use crate::app::CodeApp;

impl CodeApp {
    /// Architecture: Show Flow of Function at Cursor: a Mermaid flowchart of its ifs and loops.
    pub(crate) fn show_function_flow(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let path = doc.path.clone();
        let ext =
            path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some(lang) = coiman_core::language::syntax::Lang::from_extension(&ext) else {
            self.status =
                Some("Flows are drawn for Rust, Python, JavaScript, TypeScript, Java, C, C++, C# and Go".to_owned());
            return;
        };
        let text = doc.buffer.text();
        let head = doc.buffer.primary().head;
        let offset = text.char_indices().nth(head).map_or(text.len(), |(i, _)| i);
        match coiman_core::analysis::flow::function_flow(lang, &text, offset) {
            Some((name, line, chart)) => {
                let file = path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let markdown = format!(
                    "# Flow of `{name}`

`{file}:{line}`: ifs (yes, no), loops (each, done), returns and breaks; plain statements in a row share one box.

```mermaid
{chart}```
"
                );
                self.open_markdown_page(&markdown);
            }
            None => self.status = Some("Put the cursor inside a function first".to_owned()),
        }
    }
}
