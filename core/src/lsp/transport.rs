//! JSON-RPC message framing over a byte stream (`Content-Length` headers),
//! as used by language servers on stdio.

use std::io::{self, BufRead};

use serde_json::Value;

/// Frames one message: `Content-Length: N\r\n\r\n` followed by the JSON body.
pub fn encode(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Reads the next message. Returns `Ok(None)` at end of stream.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            // Tolerate stray blank lines between messages.
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            let n = value.trim().parse::<usize>().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            length = Some(n);
        }
    }
    let mut body = vec![0; length.unwrap_or(0)];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_messages_and_stops_at_eof() {
        let a = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"text": "é\r\n"}});
        let b = json!({"jsonrpc": "2.0", "method": "exit"});
        let mut bytes = encode(&a);
        bytes.extend(b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n");
        bytes.extend(encode(&b));
        let mut r = io::BufReader::new(&bytes[..]);
        assert_eq!(read_message(&mut r).unwrap(), Some(a));
        assert_eq!(read_message(&mut r).unwrap(), Some(b));
        assert_eq!(read_message(&mut r).unwrap(), None);
    }
}
