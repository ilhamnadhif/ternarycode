//! Finds the addresses of local servers in program output (`npm run dev`, `rails s`,
//! `python -m http.server`): `http://localhost:5173/` and the like, so the app can offer to
//! open them.
//!
//! It runs over the PTY output before the parser and never changes the bytes. Between matches it
//! only jumps from `://` to `://` with `memchr`, so plain output costs almost nothing. An
//! address cut at a chunk boundary is found with the next chunk.

use std::sync::OnceLock;

use memchr::memmem::Finder;

fn scheme_end() -> &'static Finder<'static> {
    static FINDER: OnceLock<Finder<'static>> = OnceLock::new();
    FINDER.get_or_init(|| Finder::new(b"://"))
}

/// Bytes kept from the end of a chunk, for an address cut there.
const TAIL: usize = 96;

/// A local server's address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// What opens it in a browser (`0.0.0.0` becomes `localhost`).
    pub url: String,
    pub port: u16,
}

#[derive(Default)]
pub struct ServerScanner {
    tail: Vec<u8>,
}

impl ServerScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The local addresses in `bytes`, in order.
    pub fn scan(&mut self, bytes: &[u8], out: &mut Vec<Found>) {
        // An address cut at the end of the last chunk: its `://` was there, but it was not
        // complete without this one (the rest is found below).
        if !self.tail.is_empty() {
            let mut joint = std::mem::take(&mut self.tail);
            let from_tail = joint.len();
            joint.extend_from_slice(&bytes[..bytes.len().min(TAIL)]);
            for at in scheme_end().find_iter(&joint) {
                if at < from_tail
                    && parse(&joint[..from_tail], at).is_none()
                    && let Some(f) = parse(&joint, at)
                {
                    out.push(f);
                }
            }
        }
        for at in scheme_end().find_iter(bytes) {
            if let Some(f) = parse(bytes, at) {
                out.push(f);
            }
        }
        let keep = bytes.len().min(TAIL);
        self.tail = bytes[bytes.len() - keep..].to_vec();
    }
}

/// The address whose `://` is at `at`, when it is a local one with a port.
fn parse(b: &[u8], at: usize) -> Option<Found> {
    let scheme = if at >= 5 && b[at - 5..at].eq_ignore_ascii_case(b"https") {
        "https"
    } else if at >= 4 && b[at - 4..at].eq_ignore_ascii_case(b"http") {
        "http"
    } else {
        return None;
    };
    // Not the end of a longer word (`xhttp://`), though maybe right after a colour.
    let start = at - scheme.len();
    if start > 0 && b[start - 1].is_ascii_alphanumeric() && !ends_sgr(b, start) {
        return None;
    }
    let mut i = at + 3;
    let host = ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"]
        .into_iter()
        .find(|h| b.get(i..i + h.len()).is_some_and(|s| s.eq_ignore_ascii_case(h.as_bytes())))?;
    i += host.len();
    i = skip_sgr(b, i);
    if b.get(i) != Some(&b':') {
        return None;
    }
    i = skip_sgr(b, i + 1);
    let mut port: u32 = 0;
    let digits = i;
    while let Some(&d) = b.get(i).filter(|d| d.is_ascii_digit()) {
        port = port * 10 + u32::from(d - b'0');
        if port > 65535 {
            return None;
        }
        i = skip_sgr(b, i + 1);
    }
    if i == digits || port == 0 {
        return None;
    }
    // A digit cut at the chunk's end could be followed by more: wait for the next chunk.
    if i >= b.len() {
        return None;
    }
    // The path, up to what an address never has.
    let mut path = String::new();
    while let Some(&c) = b.get(i) {
        if c == 0x1b {
            let next = skip_sgr(b, i);
            if next == i {
                break;
            }
            i = next;
            continue;
        }
        if !(c.is_ascii_alphanumeric() || b"-._~/?#[]@!$&'()*+,;=%".contains(&c)) {
            break;
        }
        path.push(c as char);
        i += 1;
    }
    while path.ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '\'']) {
        path.pop();
    }
    let host = if host == "0.0.0.0" { "localhost" } else { host };
    let path = if path.is_empty() { "/".to_string() } else { path };
    Some(Found {
        url: format!("{scheme}://{}:{port}{path}", host.to_ascii_lowercase()),
        port: port as u16,
    })
}

/// An SGR sequence (`ESC [ … m`) ends right before `end`.
fn ends_sgr(b: &[u8], end: usize) -> bool {
    if end == 0 || b[end - 1] != b'm' {
        return false;
    }
    let mut j = end - 1;
    while j > 0 && (b[j - 1].is_ascii_digit() || b[j - 1] == b';' || b[j - 1] == b':') {
        j -= 1;
    }
    j >= 2 && b[j - 1] == b'[' && b[j - 2] == 0x1b
}

/// Past the SGR sequences (`ESC [ … m`) at `i`: a port printed in bold is still a port.
fn skip_sgr(b: &[u8], mut i: usize) -> usize {
    while b.get(i) == Some(&0x1b) && b.get(i + 1) == Some(&b'[') {
        let mut j = i + 2;
        while b.get(j).is_some_and(|c| c.is_ascii_digit() || *c == b';' || *c == b':') {
            j += 1;
        }
        if b.get(j) != Some(&b'm') {
            return i;
        }
        i = j + 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(chunks: &[&[u8]]) -> Vec<(String, u16)> {
        let mut s = ServerScanner::new();
        let mut out = Vec::new();
        for c in chunks {
            s.scan(c, &mut out);
        }
        out.into_iter().map(|f| (f.url, f.port)).collect()
    }

    #[test]
    fn dev_servers_say_where_they_listen() {
        // Vite, with its colours (the port in bold).
        assert_eq!(
            found(&[b"  \x1b[32m\xe2\x9e\x9c\x1b[39m  \x1b[1mLocal\x1b[22m:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/\x1b[39m\r\n"]),
            vec![("http://localhost:5173/".into(), 5173)]
        );
        // Next.js, Django, Rails, Python.
        assert_eq!(found(&[b"   - Local:        http://localhost:3000\r\n"]), vec![("http://localhost:3000/".into(), 3000)]);
        assert_eq!(
            found(&[b"Starting development server at http://127.0.0.1:8000/\r\nQuit with CONTROL-C.\r\n"]),
            vec![("http://127.0.0.1:8000/".into(), 8000)]
        );
        assert_eq!(found(&[b"* Listening on http://0.0.0.0:3000\r\n"]), vec![("http://localhost:3000/".into(), 3000)]);
        assert_eq!(
            found(&[b"Serving HTTP on 127.0.0.1 port 8765 (http://127.0.0.1:8765/) ...\r\n"]),
            vec![("http://127.0.0.1:8765/".into(), 8765)]
        );
        assert_eq!(found(&[b"at https://[::1]:8443/app.\r\n"]), vec![("https://[::1]:8443/app".into(), 8443)]);
    }

    #[test]
    fn an_address_cut_between_reads_is_found_once() {
        assert_eq!(found(&[b"Local: http://local", b"host:5173/\r\n"]), vec![("http://localhost:5173/".into(), 5173)]);
        assert_eq!(found(&[b"Local: http://localhost:51", b"73/\r\n"]), vec![("http://localhost:5173/".into(), 5173)]);
        assert_eq!(found(&[b"Local: http:/", b"/localhost:5173\r\n"]), vec![("http://localhost:5173/".into(), 5173)]);
    }

    #[test]
    fn other_addresses_are_not_servers_here() {
        assert!(found(&[b"see https://example.com:443/x and localhost:3000 and ftp://localhost:21\r\n"]).is_empty());
        assert!(found(&[b"http://localhost/ http://localhost:99999/ xhttp://localhost:80/\r\n"]).is_empty());
    }
}
