//! A streaming scanner for the OSC sequences `vte` 0.15 drops without an event: desktop
//! notifications (OSC 9, OSC 99, OSC 777 `notify`), shell integration (OSC 133), the working
//! directory (OSC 7) and TernaryCode's own hook messages (OSC 777 `ternarycode`).
//!
//! It runs over the PTY output before the parser and never changes the bytes. Between escape
//! sequences it only jumps from ESC to ESC with `memchr`, so plain output costs almost nothing.
//! A sequence cut at a chunk boundary is picked up in the next call.

use std::sync::OnceLock;

use memchr::memchr2;
use memchr::memmem::Finder;

/// Finder for `ESC ]`, built once.
fn osc_start() -> &'static Finder<'static> {
    static FINDER: OnceLock<Finder<'static>> = OnceLock::new();
    FINDER.get_or_init(|| Finder::new(b"\x1b]"))
}

/// Longest payload kept; longer sequences are skipped (nothing legitimate needs more, and an
/// endless sequence must not grow memory).
pub const MAX_PAYLOAD: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptMark {
    /// OSC 133;A: the prompt starts.
    PromptStart,
    /// OSC 133;B: the user's input starts (end of the prompt).
    CommandStart,
    /// OSC 133;C: the command was entered and runs.
    CommandExecuted,
    /// OSC 133;D[;code]: the command finished.
    CommandFinished(Option<i32>),
}

/// How a shell redraws its prompt after the terminal is resized (`OSC 133;A;redraw=…`, the
/// option kitty and Ghostty use): the terminal may clear the prompt first so no stale copy is
/// left where the old width put it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Redraw {
    /// Not said (or `redraw=0`): leave the prompt alone.
    #[default]
    Unknown,
    /// The whole prompt is drawn again (zsh, fish): clear from the prompt's start.
    All,
    /// Only its last line is (bash/readline): clear from the input line.
    LastLine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscSignal {
    /// A desktop notification request (OSC 9, OSC 99, OSC 777;notify).
    Notify { title: Option<String>, body: String },
    /// ConEmu / Windows Terminal progress (OSC 9;4;state;value).
    Progress { state: u8, value: Option<u8> },
    /// Shell integration mark (OSC 133).
    Prompt(PromptMark),
    /// The `redraw=` option of the prompt start that follows (OSC 133;A;redraw=…).
    PromptRedraw(Redraw),
    /// The shell's working directory (OSC 7, `file://host/path`).
    Cwd(String),
    /// A TernaryCode hook message written to the terminal (OSC 777;ternarycode;payload), used
    /// by panes whose agent runs on another machine.
    Hook(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    OscNum,
    OscPayload,
    /// ESC seen inside an OSC: `\` ends it (ST), anything else aborts it.
    OscEsc,
}

pub struct OscScanner {
    state: State,
    num: u32,
    digits: u8,
    interesting: bool,
    overflow: bool,
    payload: Vec<u8>,
}

impl Default for OscScanner {
    fn default() -> Self {
        Self::new()
    }
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            num: 0,
            digits: 0,
            interesting: false,
            overflow: false,
            payload: Vec::new(),
        }
    }

    /// Forgets a half-received sequence (the session ended).
    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.payload.clear();
    }

    /// Scans `bytes`, appending the complete signals found to `out`.
    pub fn scan(&mut self, bytes: &[u8], out: &mut Vec<OscSignal>) {
        let mut found = Vec::new();
        self.scan_at(bytes, &mut found);
        out.extend(found.into_iter().map(|(_, s)| s));
    }

    /// As `scan`, with the offset in `bytes` just past each signal's sequence (where the parser
    /// has seen all of it).
    pub fn scan_at(&mut self, bytes: &[u8], out: &mut Vec<(usize, OscSignal)>) {
        let mut i = 0;
        let len = bytes.len();
        while i < len {
            match self.state {
                // Jump straight to the next `ESC ]` (SIMD search), skipping every CSI/SGR
                // sequence on the way without looking at it. An ESC as the very last byte may
                // start an OSC in the next chunk.
                State::Ground => match osc_start().find(&bytes[i..]) {
                    Some(p) => {
                        i += p + 2;
                        self.begin_osc();
                    }
                    None => {
                        if bytes[len - 1] == ESC {
                            self.state = State::Esc;
                        }
                        return;
                    }
                },
                // Only reached when a chunk ended with ESC.
                State::Esc => {
                    let b = bytes[i];
                    if b == b']' {
                        self.begin_osc();
                        i += 1;
                    } else if b == ESC {
                        i += 1;
                    } else {
                        self.state = State::Ground;
                    }
                }
                State::OscNum => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b'0'..=b'9' if self.digits < 6 => {
                            self.num = self.num * 10 + (b - b'0') as u32;
                            self.digits += 1;
                        }
                        b';' => {
                            self.interesting =
                                self.digits > 0 && matches!(self.num, 7 | 9 | 99 | 133 | 777);
                            self.state = State::OscPayload;
                        }
                        BEL => {
                            self.state = State::Ground;
                        }
                        ESC => self.state = State::OscEsc,
                        _ => {
                            // Not a numbered OSC we know (e.g. `ESC ] L`): skip to its end.
                            self.interesting = false;
                            self.state = State::OscPayload;
                        }
                    }
                }
                State::OscPayload => {
                    let rest = &bytes[i..];
                    let found = memchr2(BEL, ESC, rest);
                    let chunk = &rest[..found.unwrap_or(rest.len())];
                    if self.interesting && !self.overflow {
                        if self.payload.len() + chunk.len() > MAX_PAYLOAD {
                            self.overflow = true;
                            self.payload.clear();
                        } else {
                            self.payload.extend_from_slice(chunk);
                        }
                    }
                    match found {
                        Some(p) => {
                            let term = rest[p];
                            i += p + 1;
                            if term == BEL {
                                self.finish(out, i);
                                self.state = State::Ground;
                            } else {
                                self.state = State::OscEsc;
                            }
                        }
                        None => i = len,
                    }
                }
                State::OscEsc => {
                    if bytes[i] == b'\\' {
                        i += 1;
                        self.finish(out, i);
                        self.state = State::Ground;
                    } else {
                        // A new escape sequence interrupted the OSC: drop it and look at this
                        // byte as the start of the new one.
                        self.payload.clear();
                        self.state = State::Esc;
                    }
                }
            }
        }
    }

    fn begin_osc(&mut self) {
        self.state = State::OscNum;
        self.num = 0;
        self.digits = 0;
        self.interesting = false;
        self.overflow = false;
        self.payload.clear();
    }

    fn finish(&mut self, out: &mut Vec<(usize, OscSignal)>, end: usize) {
        if !self.interesting || self.overflow {
            self.payload.clear();
            return;
        }
        let text = String::from_utf8_lossy(&self.payload).into_owned();
        self.payload.clear();
        if self.num == 133
            && let Some(redraw) = redraw_option(&text)
        {
            out.push((end, OscSignal::PromptRedraw(redraw)));
        }
        if let Some(signal) = parse(self.num, &text) {
            out.push((end, signal));
        }
    }
}

/// The `redraw=` option of a prompt start mark (`A;redraw=1;…`), when given.
fn redraw_option(text: &str) -> Option<Redraw> {
    let mut parts = text.split(';');
    if parts.next()? != "A" {
        return None;
    }
    parts.find_map(|kv| kv.strip_prefix("redraw=")).map(|v| match v {
        "1" => Redraw::All,
        "last" => Redraw::LastLine,
        _ => Redraw::Unknown,
    })
}

fn parse(num: u32, text: &str) -> Option<OscSignal> {
    match num {
        7 => {
            let rest = text.strip_prefix("file://")?;
            // file://host/path → /path (the host part may be empty).
            let path = percent_decode(&rest[rest.find('/')?..]);
            // Windows: file:///C:/dir → C:/dir.
            #[cfg(windows)]
            let path = match path.as_bytes() {
                [b'/', d, b':', ..] if d.is_ascii_alphabetic() => path[1..].to_string(),
                _ => path,
            };
            Some(OscSignal::Cwd(path))
        }
        9 => {
            // ConEmu sub-commands are `9;<n>` and `9;<n>;…` for n from 1 to 12; only progress
            // (4) matters. Any other text, numbers too, is a notification.
            let digits = text.bytes().take_while(u8::is_ascii_digit).count();
            let sub = matches!(
                &text[..digits],
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "11" | "12"
            );
            if sub && (text.len() == digits || text.as_bytes()[digits] == b';') {
                if &text[..digits] != "4" {
                    return None;
                }
                let mut parts = text.split(';').skip(1);
                let state = parts.next()?.parse().ok()?;
                let value = parts.next().and_then(|v| v.parse().ok());
                return Some(OscSignal::Progress { state, value });
            }
            if text.is_empty() {
                return None;
            }
            Some(OscSignal::Notify {
                title: None,
                body: text.to_string(),
            })
        }
        99 => {
            // Kitty: `99;metadata;payload`; metadata `key=value` pairs joined by `:`.
            let (meta, payload) = text.split_once(';')?;
            if payload.is_empty() {
                return None;
            }
            let is_body = meta.split(':').any(|kv| kv == "p=body");
            Some(if is_body {
                OscSignal::Notify {
                    title: None,
                    body: payload.to_string(),
                }
            } else {
                OscSignal::Notify {
                    title: Some(payload.to_string()),
                    body: String::new(),
                }
            })
        }
        133 => {
            let mut parts = text.split(';');
            let mark = match parts.next()? {
                "A" => PromptMark::PromptStart,
                "B" => PromptMark::CommandStart,
                "C" => PromptMark::CommandExecuted,
                "D" => PromptMark::CommandFinished(parts.next().and_then(|c| c.parse().ok())),
                _ => return None,
            };
            Some(OscSignal::Prompt(mark))
        }
        777 => {
            let (kind, rest) = text.split_once(';').unwrap_or((text, ""));
            match kind {
                "notify" => {
                    let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
                    Some(OscSignal::Notify {
                        title: Some(title.to_string()).filter(|t| !t.is_empty()),
                        body: body.to_string(),
                    })
                }
                "ternarycode" => Some(OscSignal::Hook(rest.to_string())),
                _ => None,
            }
        }
        _ => None,
    }
}

fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        (b as char).to_digit(16).map(|d| d as u8)
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(input: &[u8]) -> Vec<OscSignal> {
        let mut s = OscScanner::new();
        let mut out = Vec::new();
        s.scan(input, &mut out);
        out
    }

    /// The same input split at every possible position gives the same signals.
    fn scan_split_everywhere(input: &[u8]) -> Vec<OscSignal> {
        let whole = scan_all(input);
        for cut in 0..=input.len() {
            let mut s = OscScanner::new();
            let mut out = Vec::new();
            s.scan(&input[..cut], &mut out);
            s.scan(&input[cut..], &mut out);
            assert_eq!(out, whole, "cut at {cut}");
        }
        // Byte by byte.
        let mut s = OscScanner::new();
        let mut out = Vec::new();
        for b in input {
            s.scan(std::slice::from_ref(b), &mut out);
        }
        assert_eq!(out, whole, "byte by byte");
        whole
    }

    #[test]
    fn notifications_all_three_kinds() {
        let input = b"hello\x1b]9;Claude needs your permission\x07world\
            \x1b]777;notify;Codex;Approval requested\x1b\\\
            \x1b]99;i=1:p=body;Done\x1b\\";
        assert_eq!(
            scan_split_everywhere(input),
            vec![
                OscSignal::Notify {
                    title: None,
                    body: "Claude needs your permission".into()
                },
                OscSignal::Notify {
                    title: Some("Codex".into()),
                    body: "Approval requested".into()
                },
                OscSignal::Notify {
                    title: None,
                    body: "Done".into()
                },
            ]
        );
    }

    #[test]
    fn shell_integration_marks() {
        let input = b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;1\x07\
            \x1b]133;D\x07";
        assert_eq!(
            scan_split_everywhere(input),
            vec![
                OscSignal::Prompt(PromptMark::PromptStart),
                OscSignal::Prompt(PromptMark::CommandStart),
                OscSignal::Prompt(PromptMark::CommandExecuted),
                OscSignal::Prompt(PromptMark::CommandFinished(Some(1))),
                OscSignal::Prompt(PromptMark::CommandFinished(None)),
            ]
        );
    }

    #[test]
    fn progress_is_not_a_notification() {
        assert_eq!(
            scan_split_everywhere(b"\x1b]9;4;1;40\x07\x1b]9;4;0\x07\x1b]9;1;x\x07"),
            vec![
                OscSignal::Progress {
                    state: 1,
                    value: Some(40)
                },
                OscSignal::Progress {
                    state: 0,
                    value: None
                },
            ]
        );
    }

    #[test]
    fn notifications_may_be_numbers() {
        let notify = |body: &str| OscSignal::Notify {
            title: None,
            body: body.into(),
        };
        // Not a ConEmu sub-command: shown. ConEmu's own (a cwd, a tab title, a bare 12) are not.
        assert_eq!(
            scan_split_everywhere(b"\x1b]9;42\x07\x1b]9;2024;release done\x07\x1b]9;0;x\x07\x1b]9;13;y\x07\
                \x1b]9;9;C:\\dir\x07\x1b]9;3;tab\x07\x1b]9;12\x07"),
            vec![notify("42"), notify("2024;release done"), notify("0;x"), notify("13;y")]
        );
    }

    #[test]
    fn other_sequences_are_ignored() {
        // Titles, colours, hyperlinks, CSI, DCS-like noise and an unknown OSC.
        let input = b"\x1b]0;title\x07\x1b]11;?\x1b\\\x1b]8;;http://x\x07link\x1b]8;;\x07\
            \x1b[31mred\x1b[0m\x1b]L\x07\x1bPq#0\x1b\\";
        assert!(scan_split_everywhere(input).is_empty());
    }

    #[test]
    fn interrupted_sequence_is_dropped_and_the_next_one_counts() {
        // ESC [ inside the OSC aborts it; the following OSC 9 is still seen.
        let input = b"\x1b]9;half\x1b[0m\x1b]9;whole\x07";
        assert_eq!(
            scan_split_everywhere(input),
            vec![OscSignal::Notify {
                title: None,
                body: "whole".into()
            }]
        );
    }

    #[test]
    fn oversized_payload_is_skipped() {
        let mut input = b"\x1b]9;".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_PAYLOAD + 10));
        input.extend_from_slice(b"\x07\x1b]9;after\x07");
        assert_eq!(
            scan_all(&input),
            vec![OscSignal::Notify {
                title: None,
                body: "after".into()
            }]
        );
    }

    #[test]
    fn hook_and_cwd() {
        let input = b"\x1b]777;ternarycode;{\"event\":\"stop\"}\x07\
            \x1b]7;file://mac.local/Users/me/My%20Project\x07";
        assert_eq!(
            scan_split_everywhere(input),
            vec![
                OscSignal::Hook("{\"event\":\"stop\"}".into()),
                OscSignal::Cwd("/Users/me/My Project".into()),
            ]
        );
        // PowerShell on Windows (System.Uri): file:///C:/…
        let win = scan_all(b"\x1b]7;file:///C:/Users/me/My%20Project\x07");
        let want = if cfg!(windows) { "C:/Users/me/My Project" } else { "/C:/Users/me/My Project" };
        assert_eq!(win, vec![OscSignal::Cwd(want.into())]);
    }

    #[test]
    fn utf8_payload_survives_splits() {
        let input = "\x1b]777;notify;Agén;Selesai ✓\x07".as_bytes();
        assert_eq!(
            scan_split_everywhere(input),
            vec![OscSignal::Notify {
                title: Some("Agén".into()),
                body: "Selesai ✓".into()
            }]
        );
    }
}
