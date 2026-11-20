//! Jupyter notebooks (`.ipynb`, nbformat 4): cells read from and written back to the file.
//! Only what Coiman edits changes on save (a cell's source, outputs and run count, the cell
//! list); every other field of the file and of each cell is kept as it was.

use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    Code,
    Markdown,
    Raw,
}

impl CellKind {
    fn name(self) -> &'static str {
        match self {
            CellKind::Code => "code",
            CellKind::Markdown => "markdown",
            CellKind::Raw => "raw",
        }
    }
}

/// What a code cell showed when it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Printed text; `error` for stderr.
    Stream { text: String, error: bool },
    /// The cell's value as text (`text/plain`).
    Value(String),
    /// A picture (`image/png`), its bytes.
    Png(Vec<u8>),
    /// An exception: `name: value`.
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub kind: CellKind,
    pub source: String,
    pub outputs: Vec<Output>,
    pub count: Option<u64>,
    /// The cell as read, for the fields Coiman doesn't edit (metadata, ids, attachments).
    raw: Map<String, Value>,
    /// The outputs as read: written back as they were (tracebacks, HTML and the other kinds
    /// Coiman doesn't show) while the cell hasn't run again.
    raw_outputs: Option<(Vec<Output>, Vec<Value>)>,
}

impl Cell {
    pub fn new(kind: CellKind) -> Self {
        Self { kind, source: String::new(), outputs: Vec::new(), count: None, raw: Map::new(), raw_outputs: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notebook {
    pub cells: Vec<Cell>,
    /// The notebook as read, cells aside.
    raw: Map<String, Value>,
}

/// nbformat's multi-line strings: a string, or a list of lines.
fn joined(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// Text as nbformat writes it: a list of lines, each but the last with its `\n`.
fn lines(text: &str) -> Value {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('\n') {
        out.push(Value::String(rest[..=i].to_owned()));
        rest = &rest[i + 1..];
    }
    if !rest.is_empty() {
        out.push(Value::String(rest.to_owned()));
    }
    Value::Array(out)
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = T.iter().position(|&t| t == c)? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &b)| acc | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn read_output(o: &Value) -> Option<Output> {
    match o["output_type"].as_str()? {
        "stream" => Some(Output::Stream { text: joined(&o["text"]), error: o["name"] == "stderr" }),
        "execute_result" | "display_data" => {
            let data = &o["data"];
            if let Some(png) = data.get("image/png") {
                return base64_decode(&joined(png)).map(Output::Png);
            }
            data.get("text/plain").map(|t| Output::Value(joined(t)))
        }
        "error" => Some(Output::Error(format!(
            "{}: {}",
            o["ename"].as_str().unwrap_or("Error"),
            o["evalue"].as_str().unwrap_or("")
        ))),
        _ => None,
    }
}

fn write_output(o: &Output, count: Option<u64>) -> Value {
    match o {
        Output::Stream { text, error } => {
            json!({"output_type": "stream", "name": if *error { "stderr" } else { "stdout" }, "text": lines(text)})
        }
        Output::Value(text) => json!({
            "output_type": "execute_result", "execution_count": count, "metadata": {},
            "data": {"text/plain": lines(text)}
        }),
        Output::Png(bytes) => json!({
            "output_type": "display_data", "metadata": {},
            "data": {"image/png": base64_encode(bytes), "text/plain": ["<Figure>"]}
        }),
        Output::Error(text) => {
            let (name, value) = text.split_once(": ").unwrap_or((text.as_str(), ""));
            json!({"output_type": "error", "ename": name, "evalue": value, "traceback": [text]})
        }
    }
}

impl Notebook {
    /// A new notebook with one empty code cell, for Python.
    pub fn empty() -> Self {
        let raw = json!({
            "metadata": {
                "kernelspec": {"display_name": "Python 3", "language": "python", "name": "python3"},
                "language_info": {"name": "python"}
            },
            "nbformat": 4,
            "nbformat_minor": 5
        });
        Self { cells: vec![Cell::new(CellKind::Code)], raw: raw.as_object().cloned().unwrap_or_default() }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not a notebook: {e}"))?;
        let mut raw = v.as_object().cloned().ok_or("not a notebook: expected an object")?;
        if raw.get("nbformat").and_then(Value::as_u64).is_some_and(|n| n < 4) {
            return Err("nbformat 3 and older aren't read: save it with Jupyter as nbformat 4".to_owned());
        }
        let cells = raw.remove("cells").and_then(|c| c.as_array().cloned()).unwrap_or_default();
        let cells = cells
            .into_iter()
            .filter_map(|c| {
                let mut raw = c.as_object()?.clone();
                let kind = match raw.get("cell_type")?.as_str()? {
                    "code" => CellKind::Code,
                    "markdown" => CellKind::Markdown,
                    _ => CellKind::Raw,
                };
                let listed = raw.remove("outputs").and_then(|o| o.as_array().cloned()).unwrap_or_default();
                let outputs: Vec<Output> = listed.iter().filter_map(read_output).collect();
                let source = raw.remove("source").map(|s| joined(&s)).unwrap_or_default();
                let count = raw.remove("execution_count").and_then(|c| c.as_u64());
                let raw_outputs = (!listed.is_empty()).then(|| (outputs.clone(), listed));
                Some(Cell { kind, source, outputs, count, raw, raw_outputs })
            })
            .collect();
        Ok(Self { cells, raw })
    }

    /// The notebook as JSON, indented as Jupyter writes it.
    pub fn to_json(&self) -> String {
        let cells: Vec<Value> = self
            .cells
            .iter()
            .map(|c| {
                let mut m = c.raw.clone();
                m.insert("cell_type".into(), json!(c.kind.name()));
                m.insert("source".into(), lines(&c.source));
                m.entry("metadata").or_insert_with(|| json!({}));
                if c.kind == CellKind::Code {
                    m.insert("execution_count".into(), json!(c.count));
                    let outs: Vec<Value> = match &c.raw_outputs {
                        Some((read, as_read)) if *read == c.outputs => as_read.clone(),
                        _ => c.outputs.iter().map(|o| write_output(o, c.count)).collect(),
                    };
                    m.insert("outputs".into(), Value::Array(outs));
                } else {
                    m.remove("outputs");
                    m.remove("execution_count");
                }
                Value::Object(m)
            })
            .collect();
        let mut doc = self.raw.clone();
        doc.insert("cells".into(), Value::Array(cells));
        let mut out = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
        let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
        let _ = serde::Serialize::serialize(&Value::Object(doc), &mut ser);
        let mut text = String::from_utf8(out).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The kernel's language, as the file names it.
    pub fn language(&self) -> String {
        let meta = &self.raw.get("metadata").cloned().unwrap_or_default();
        meta["kernelspec"]["language"]
            .as_str()
            .or_else(|| meta["language_info"]["name"].as_str())
            .unwrap_or("python")
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
 "cells": [
  {"cell_type": "markdown", "id": "m1", "metadata": {"tags": ["intro"]}, "source": ["# Title\n", "Some text"]},
  {"cell_type": "code", "id": "c1", "execution_count": 3, "metadata": {"collapsed": false},
   "source": "print(1)\n1 + 1",
   "outputs": [
    {"output_type": "stream", "name": "stdout", "text": ["1\n"]},
    {"output_type": "execute_result", "execution_count": 3, "metadata": {}, "data": {"text/plain": ["2"]}},
    {"output_type": "display_data", "metadata": {}, "data": {"image/png": "iVBORw0KGgo=", "text/plain": ["<Figure>"]}},
    {"output_type": "error", "ename": "ValueError", "evalue": "bad", "traceback": ["..."]}
   ]}
 ],
 "metadata": {"kernelspec": {"display_name": "Python 3", "language": "python", "name": "python3"}, "custom": 7},
 "nbformat": 4,
 "nbformat_minor": 5
}"##;

    #[test]
    fn reads_and_writes_back() {
        let nb = Notebook::parse(SAMPLE).unwrap();
        assert_eq!(nb.cells.len(), 2);
        assert_eq!(nb.cells[0].source, "# Title\nSome text");
        let c = &nb.cells[1];
        assert_eq!((c.kind, c.count), (CellKind::Code, Some(3)));
        assert_eq!(
            c.outputs,
            [
                Output::Stream { text: "1\n".into(), error: false },
                Output::Value("2".into()),
                Output::Png(vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10]),
                Output::Error("ValueError: bad".into()),
            ]
        );
        assert_eq!(nb.language(), "python");
        // Saved and read again: the same, and what Coiman doesn't edit is kept.
        let text = nb.to_json();
        assert_eq!(Notebook::parse(&text).unwrap(), nb);
        assert!(text.contains("\"custom\": 7") && text.contains("\"tags\"") && text.contains("\"id\": \"c1\""));
        // Outputs untouched keep the traceback; run again, they're written anew.
        assert!(text.contains("\"...\""));
        let mut ran = nb.clone();
        ran.cells[1].outputs = vec![Output::Value("3".into())];
        let text = ran.to_json();
        assert!(!text.contains("\"...\"") && text.contains("\"3\""));
        assert!(text.contains("\"source\": [\n    \"print(1)\\n\",\n    \"1 + 1\"\n   ]"), "{text}");
    }

    #[test]
    fn new_cells_old_formats_and_base64() {
        let mut nb = Notebook::empty();
        nb.cells.push(Cell::new(CellKind::Markdown));
        nb.cells[1].source = "Hi".into();
        let again = Notebook::parse(&nb.to_json()).unwrap();
        assert_eq!(again.cells.len(), 2);
        assert_eq!(again.cells[1].kind, CellKind::Markdown);
        assert!(Notebook::parse("{\"nbformat\": 3}").unwrap_err().contains("nbformat 3"));
        assert!(Notebook::parse("[]").is_err());
        for data in [&b""[..], b"a", b"ab", b"abc", b"hello world"] {
            assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }
}
