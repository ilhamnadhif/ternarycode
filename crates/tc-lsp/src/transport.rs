//! The LSP base protocol: each message is `Content-Length: N\r\n` (other headers allowed, then
//! an empty line) followed by N bytes of JSON.

use serde_json::Value;

/// Messages larger than this are refused (a broken server must not exhaust memory).
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// One message, framed.
pub fn encode(msg: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(msg).unwrap_or_default();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Splits a byte stream into messages, whatever the reads cut it into.
#[derive(Default)]
pub struct Framer {
    buf: Vec<u8>,
}

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `bytes`; returns the messages now complete (a broken one as an error, after which
    /// the stream continues with the next header).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Value, String>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(end) = find(&self.buf, b"\r\n\r\n") else {
                // A header never ends: drop what cannot be one.
                if self.buf.len() > 64 * 1024 {
                    self.buf.clear();
                    out.push(Err("header too long".into()));
                }
                break;
            };
            let header = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            let len = header.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<usize>().ok())?
            });
            let Some(len) = len else {
                self.buf.drain(..end + 4);
                out.push(Err(format!("no Content-Length in {header:?}")));
                continue;
            };
            if len > MAX_MESSAGE {
                self.buf.clear();
                out.push(Err(format!("message of {len} bytes")));
                break;
            }
            let start = end + 4;
            if self.buf.len() < start + len {
                break;
            }
            let body: Vec<u8> = self.buf.drain(..start + len).skip(start).collect();
            out.push(serde_json::from_slice(&body).map_err(|e| e.to_string()));
        }
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_come_out_whole_however_they_are_cut() {
        let a = encode(&json!({"jsonrpc": "2.0", "id": 1, "result": "héllo"}));
        let b = encode(&json!({"jsonrpc": "2.0", "method": "x"}));
        let all: Vec<u8> = [a.clone(), b.clone()].concat();
        // Byte by byte.
        let mut f = Framer::new();
        let mut got = Vec::new();
        for byte in &all {
            got.extend(f.push(std::slice::from_ref(byte)));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].as_ref().unwrap()["result"], "héllo");
        assert_eq!(got[1].as_ref().unwrap()["method"], "x");
        // Both at once, with another header.
        let mut f = Framer::new();
        let mut extra = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n".to_vec();
        extra.extend_from_slice(&a);
        let got = f.push(&[extra, b].concat());
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(Result::is_ok));
    }

    #[test]
    fn a_broken_message_is_reported_and_skipped() {
        let mut f = Framer::new();
        let mut bytes = b"Content-Length: 5\r\n\r\n{nop}".to_vec();
        bytes.extend(encode(&json!({"id": 2})));
        let got = f.push(&bytes);
        assert!(got[0].is_err());
        assert_eq!(got[1].as_ref().unwrap()["id"], 2);
    }
}
