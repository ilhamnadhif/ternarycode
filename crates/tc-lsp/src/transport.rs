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
    /// Bytes of a refused message still to come: dropped as they arrive, so the next header is
    /// found where it starts.
    skip: usize,
}

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `bytes`; returns the messages now complete (a broken one as an error, after which
    /// the stream continues with the next header).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Value, String>> {
        let skipped = self.skip.min(bytes.len());
        self.skip -= skipped;
        self.buf.extend_from_slice(&bytes[skipped..]);
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
            let start = end + 4;
            if len > MAX_MESSAGE {
                // Drop exactly its body (what has come of it now, the rest as it comes).
                let have = (self.buf.len() - start).min(len);
                self.buf.drain(..start + have);
                self.skip = len - have;
                out.push(Err(format!("message of {len} bytes")));
                continue;
            }
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
    fn a_message_too_big_is_skipped_to_the_next() {
        let len = MAX_MESSAGE + 100;
        let mut f = Framer::new();
        // Its body looks like messages: none of them may come out.
        let fake = encode(&json!({"id": 666}));
        let mut first = format!("Content-Length: {len}\r\n\r\n").into_bytes();
        first.extend_from_slice(&fake);
        let got = f.push(&first);
        assert_eq!(got.len(), 1);
        assert!(got[0].is_err());
        let mut left = len - fake.len();
        let chunk: Vec<u8> = fake.iter().copied().cycle().take(1 << 20).collect();
        while left > 10 {
            let n = chunk.len().min(left - 10);
            assert!(f.push(&chunk[..n]).is_empty());
            left -= n;
        }
        // The body's end and the next message's start come in one read, its end in another.
        let next = encode(&json!({"id": 2}));
        let mut tail = chunk[..left].to_vec();
        tail.extend_from_slice(&next[..5]);
        assert!(f.push(&tail).is_empty());
        let got = f.push(&next[5..]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref().unwrap()["id"], 2);
        // A body already in the buffer whole is dropped at once.
        let mut f = Framer::new();
        let mut bytes = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE + 1).into_bytes();
        bytes.resize(bytes.len() + MAX_MESSAGE + 1, b'x');
        bytes.extend_from_slice(&next);
        let got = f.push(&bytes);
        assert!(got[0].is_err());
        assert_eq!(got[1].as_ref().unwrap()["id"], 2);
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
