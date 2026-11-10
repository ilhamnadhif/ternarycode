//! Input encoding: keys (xterm's legacy encoding, or the kitty keyboard protocol when the
//! program asks for it), mouse reports, paste and focus events → bytes for the program.

use alacritty_terminal::term::TermMode;
use egui::Key;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    /// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }

    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }
}

/// The kitty keyboard protocol's progressive enhancements, as a program sets them with
/// `CSI > flags u` (alacritty keeps their stacks; the keys are encoded here). Esc, and keys with
/// Alt or Ctrl, as `CSI u` escape codes.
pub const DISAMBIGUATE: u8 = 1;
/// Repeat and release events.
pub const EVENT_TYPES: u8 = 2;
/// The shifted key and the key's place on a US layout, along with the key.
pub const ALTERNATE_KEYS: u8 = 4;
/// Every key as an escape code, text keys and Enter, Tab and Backspace too.
pub const ALL_KEYS: u8 = 8;
/// The text a key typed, in its escape code (with `ALL_KEYS` only).
pub const ASSOCIATED_TEXT: u8 = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: cursor keys send SS3 sequences.
    pub app_cursor: bool,
    /// The kitty keyboard protocol's flags in effect; none is the legacy encoding.
    pub kitty: u8,
}

impl KeyModes {
    /// The key modes a terminal is in.
    pub fn of(mode: TermMode) -> Self {
        const KITTY: [(TermMode, u8); 5] = [
            (TermMode::DISAMBIGUATE_ESC_CODES, DISAMBIGUATE),
            (TermMode::REPORT_EVENT_TYPES, EVENT_TYPES),
            (TermMode::REPORT_ALTERNATE_KEYS, ALTERNATE_KEYS),
            (TermMode::REPORT_ALL_KEYS_AS_ESC, ALL_KEYS),
            (TermMode::REPORT_ASSOCIATED_TEXT, ASSOCIATED_TEXT),
        ];
        Self {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            kitty: KITTY.iter().filter(|(m, _)| mode.contains(*m)).fold(0, |a, (_, f)| a | f),
        }
    }
}

const ESC: u8 = 0x1b;

/// Press, repeat or release: the kitty protocol's event types 1, 2 and 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

/// A key event as egui reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    /// The key in the same place on a US layout (egui's `physical_key`).
    pub physical: Option<Key>,
    pub mods: Mods,
    pub action: KeyAction,
}

impl KeyEvent {
    pub fn press(key: Key, mods: Mods) -> Self {
        Self {
            key,
            physical: None,
            mods,
            action: KeyAction::Press,
        }
    }
}

/// Encodes a key event as the program asked: in the legacy encoding (`encode_key`), or the kitty
/// keyboard protocol's (https://sw.kovidgoyal.net/kitty/keyboard-protocol/). `text` is what the
/// key typed (egui's text event right after it). `None`: the key is not sent as a key, its text
/// is typed as it is. Otherwise the bytes stand for the key, and its text is not typed again.
///
/// What egui does not tell is left out: modifier keys pressed on their own, Caps Lock and Num
/// Lock, the keypad (its keys are the main ones), the Super key (⌘ belongs to the app).
pub fn encode_event(ev: &KeyEvent, text: Option<&str>, modes: KeyModes) -> Option<Vec<u8>> {
    let flags = modes.kitty;
    let all = flags & ALL_KEYS != 0;
    let disambiguate = all || flags & DISAMBIGUATE != 0;
    let events = flags & EVENT_TYPES != 0;
    let released = ev.action == KeyAction::Release;
    if released && !events {
        return Some(Vec::new());
    }
    // The event type (a press is the default); without event types a repeat is a press.
    let event = match ev.action {
        KeyAction::Repeat if events => Some(2),
        KeyAction::Release => Some(3),
        _ => None,
    };
    let mods = ev.mods;
    // The legacy encoding, which has no releases.
    let legacy = || {
        if released {
            Some(Vec::new())
        } else {
            encode_key(ev.key, mods, modes)
        }
    };
    // Alternate keys and associated text only add to keys sent as escape codes.
    if flags & (DISAMBIGUATE | EVENT_TYPES | ALL_KEYS) == 0 {
        return legacy();
    }

    if let Some(f) = functional(ev.key) {
        let kept = match f {
            // Enter, Tab and Backspace without modifiers stay as they were, so a shell is still
            // usable after a program left the protocol on; nor are they released.
            Functional::U(13 | 9 | 127) if !all => !disambiguate || !mods.any() || released,
            Functional::U(27) => !disambiguate,
            _ => false,
        };
        if kept {
            return legacy();
        }
        let plain = !mods.any() && event.is_none();
        return Some(match f {
            Functional::U(n) => kitty_csi(&n.to_string(), mods, event, None, 'u'),
            Functional::Tilde(n) => kitty_csi(&n.to_string(), mods, event, None, '~'),
            // The 1 is left out with nothing after it: `CSI A`, also in cursor key mode.
            Functional::Letter(c) => kitty_csi(if plain { "" } else { "1" }, mods, event, None, c as char),
        });
    }

    let Some(named) = key_char(ev.key) else {
        return legacy();
    };
    // Text keys are typed as text, plain or with Shift: Alt and Ctrl make shortcuts of them.
    if !all && !(disambiguate && (mods.alt || mods.ctrl)) {
        return legacy();
    }
    let shortcut = mods.alt || mods.ctrl;
    let typed = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
    // A text key that typed nothing is a dead key, or the input method composes with it: what
    // that makes arrives as text.
    if typed.is_none() && !shortcut && !released {
        return Some(Vec::new());
    }
    let typed_char = typed.and_then(|t| {
        let mut chars = t.chars();
        chars.next().filter(|_| chars.as_str().is_empty())
    });
    let lower = |c: char| {
        let mut l = c.to_lowercase();
        match (l.next(), l.next()) {
            (Some(l), None) => l,
            _ => c,
        }
    };
    // The key's code is its unshifted character. What it typed names it on any layout (egui
    // names keys it has no name for by their place on a US layout), unless Shift changed more
    // than its case; then egui's name is the shifted character, and the US place tells the key.
    let code = match typed_char {
        Some(c) if !shortcut && (!mods.shift || lower(c) != c) => lower(c),
        _ if letter(ev.key).is_some() || !mods.shift => named,
        _ => ev.physical.and_then(key_char).unwrap_or(named),
    };
    let mut key = (code as u32).to_string();
    if flags & ALTERNATE_KEYS != 0 {
        let shifted = match typed_char {
            _ if !mods.shift => None,
            Some(c) if !shortcut && c != code => Some(c),
            _ if letter(ev.key).is_some() => Some(code.to_ascii_uppercase()),
            _ => Some(named),
        }
        .filter(|s| *s != code);
        let base = ev.physical.and_then(key_char).filter(|b| *b != code);
        match (shifted, base) {
            (Some(s), Some(b)) => key += &format!(":{}:{}", s as u32, b as u32),
            (Some(s), None) => key += &format!(":{}", s as u32),
            (None, Some(b)) => key += &format!("::{}", b as u32),
            (None, None) => {}
        }
    }
    let text = typed.filter(|_| all && flags & ASSOCIATED_TEXT != 0 && !shortcut && !released);
    Some(kitty_csi(&key, mods, event, text, 'u'))
}

/// How the kitty protocol writes a key that types no text.
#[derive(Clone, Copy)]
enum Functional {
    /// `CSI number u`.
    U(u32),
    /// `CSI number ~`.
    Tilde(u8),
    /// `CSI 1 letter`.
    Letter(u8),
}

fn functional(key: Key) -> Option<Functional> {
    use Functional::*;
    const F13_TO_F35: [Key; 23] = [
        Key::F13, Key::F14, Key::F15, Key::F16, Key::F17, Key::F18, Key::F19, Key::F20, Key::F21,
        Key::F22, Key::F23, Key::F24, Key::F25, Key::F26, Key::F27, Key::F28, Key::F29, Key::F30,
        Key::F31, Key::F32, Key::F33, Key::F34, Key::F35,
    ];
    Some(match key {
        Key::Escape => U(27),
        Key::Enter => U(13),
        Key::Tab => U(9),
        Key::Backspace => U(127),
        Key::Insert => Tilde(2),
        Key::Delete => Tilde(3),
        Key::PageUp => Tilde(5),
        Key::PageDown => Tilde(6),
        Key::ArrowUp => Letter(b'A'),
        Key::ArrowDown => Letter(b'B'),
        Key::ArrowRight => Letter(b'C'),
        Key::ArrowLeft => Letter(b'D'),
        Key::Home => Letter(b'H'),
        Key::End => Letter(b'F'),
        Key::F1 => Letter(b'P'),
        Key::F2 => Letter(b'Q'),
        // Not `CSI R`, which is also a cursor position report.
        Key::F3 => Tilde(13),
        Key::F4 => Letter(b'S'),
        Key::F5 => Tilde(15),
        Key::F6 => Tilde(17),
        Key::F7 => Tilde(18),
        Key::F8 => Tilde(19),
        Key::F9 => Tilde(20),
        Key::F10 => Tilde(21),
        Key::F11 => Tilde(23),
        Key::F12 => Tilde(24),
        // The protocol's own numbers (the legacy encoding has none).
        _ => U(57376 + F13_TO_F35.iter().position(|k| *k == key)? as u32),
    })
}

/// `CSI key ; modifiers:event ; text terminator`, leaving out the defaults at the end.
fn kitty_csi(key: &str, mods: Mods, event: Option<u8>, text: Option<&str>, terminator: char) -> Vec<u8> {
    let mut out = format!("\x1b[{key}");
    if mods.any() || event.is_some() || text.is_some() {
        out += &format!(";{}", mods.param());
        if let Some(e) = event {
            out += &format!(":{e}");
        }
    }
    if let Some(text) = text {
        let codes: Vec<String> = text.chars().map(|c| (c as u32).to_string()).collect();
        out += &format!(";{}", codes.join(":"));
    }
    out.push(terminator);
    out.into_bytes()
}

/// Whether a key types text (a letter, digit, punctuation or Space).
pub fn is_text_key(key: Key) -> bool {
    key_char(key).is_some()
}

/// The character egui names a text key by (a letter in lower case).
fn key_char(key: Key) -> Option<char> {
    use Key::*;
    const DIGITS: [Key; 10] = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
    if let Some(c) = letter(key) {
        return Some(c as char);
    }
    if let Some(d) = DIGITS.iter().position(|k| *k == key) {
        return Some((b'0' + d as u8) as char);
    }
    Some(match key {
        Space => ' ',
        Colon => ':',
        Comma => ',',
        Backslash => '\\',
        Slash => '/',
        Pipe => '|',
        Questionmark => '?',
        Exclamationmark => '!',
        OpenBracket => '[',
        CloseBracket => ']',
        OpenCurlyBracket => '{',
        CloseCurlyBracket => '}',
        Backtick => '`',
        Minus => '-',
        Period => '.',
        Plus => '+',
        Equals => '=',
        Semicolon => ';',
        Quote => '\'',
        _ => return None,
    })
}

/// Encodes a key press in the legacy (xterm) encoding. Returns `None` for keys that should be
/// handled as text input instead (plain printable characters arrive through text events, which
/// respect the keyboard layout).
pub fn encode_key(key: Key, mods: Mods, modes: KeyModes) -> Option<Vec<u8>> {
    let csi_mod = |final_byte: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{}", mods.param(), final_byte as char).into_bytes()
        } else if modes.app_cursor && matches!(final_byte, b'A'..=b'D' | b'H' | b'F') {
            vec![ESC, b'O', final_byte]
        } else {
            vec![ESC, b'[', final_byte]
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[{code};{}~", mods.param()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let ss3_fn = |final_byte: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{}", mods.param(), final_byte as char).into_bytes()
        } else {
            vec![ESC, b'O', final_byte]
        }
    };
    let alt_prefix = |mut bytes: Vec<u8>| -> Vec<u8> {
        if mods.alt {
            bytes.insert(0, ESC);
        }
        bytes
    };

    let out = match key {
        Key::ArrowUp => csi_mod(b'A'),
        Key::ArrowDown => csi_mod(b'B'),
        Key::ArrowRight => csi_mod(b'C'),
        Key::ArrowLeft => csi_mod(b'D'),
        Key::Home => csi_mod(b'H'),
        Key::End => csi_mod(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => ss3_fn(b'P'),
        Key::F2 => ss3_fn(b'Q'),
        Key::F3 => ss3_fn(b'R'),
        Key::F4 => ss3_fn(b'S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Backspace => alt_prefix(vec![if mods.ctrl { 0x08 } else { 0x7f }]),
        Key::Enter => alt_prefix(vec![b'\r']),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt_prefix(vec![b'\t']),
        Key::Escape => alt_prefix(vec![ESC]),
        Key::Space if mods.ctrl => alt_prefix(vec![0]),
        _ => {
            // Option as Meta for printable keys is applied to the text event instead, so it
            // follows the keyboard layout.
            if mods.ctrl {
                return ctrl_byte(key).map(|b| alt_prefix(vec![b]));
            }
            return None;
        }
    };
    Some(out)
}

/// Ctrl+key control bytes (Ctrl+A = 0x01 … Ctrl+Z = 0x1a, plus the punctuation classics).
fn ctrl_byte(key: Key) -> Option<u8> {
    if let Some(c) = letter(key) {
        return Some(c - b'a' + 1);
    }
    Some(match key {
        Key::Num2 | Key::Space => 0x00,
        Key::OpenBracket | Key::Num3 => 0x1b,
        Key::Backslash | Key::Num4 => 0x1c,
        Key::CloseBracket | Key::Num5 => 0x1d,
        Key::Num6 => 0x1e,
        Key::Minus | Key::Slash | Key::Num7 => 0x1f,
        Key::Num8 => 0x7f,
        _ => return None,
    })
}

fn letter(key: Key) -> Option<u8> {
    use Key::*;
    const LETTERS: [Key; 26] = [
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    ];
    LETTERS
        .iter()
        .position(|k| *k == key)
        .map(|i| b'a' + i as u8)
}

/// Wraps pasted text for the remote: normalizes newlines to CR and adds bracketed-paste markers
/// when the application asked for them. In bracketed mode ESC and ^C are removed (as alacritty
/// does), so pasted text can neither end bracketed mode early nor interrupt the program.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let clean: String = normalized
            .chars()
            .filter(|c| *c != '\x1b' && *c != '\x03')
            .collect();
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Motion with no button held.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// Encodes a mouse report for a cell (0-based col/row). SGR (1006) encoding when `sgr`,
/// otherwise the legacy X10 form (limited to 223 columns/rows).
pub fn encode_mouse(
    button: MouseButton,
    action: MouseAction,
    mods: Mods,
    col: usize,
    row: usize,
    sgr: bool,
) -> Option<Vec<u8>> {
    let mut code: u32 = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if action == MouseAction::Motion {
        code += 32;
    }
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    if sgr {
        let suffix = if action == MouseAction::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{};{}{suffix}", col + 1, row + 1).into_bytes());
    }
    if action == MouseAction::Release {
        code = (code & !3) | 3; // X10 can't say which button was released
    }
    if col > 222 || row > 222 {
        return None;
    }
    Some(vec![
        ESC,
        b'[',
        b'M',
        32 + code as u8,
        33 + col as u8,
        33 + row as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: false,
    };
    const CTRL: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: true,
    };

    fn k(key: Key, mods: Mods) -> Vec<u8> {
        encode_key(key, mods, KeyModes::default()).unwrap()
    }

    #[test]
    fn cursor_keys_and_modes() {
        assert_eq!(k(Key::ArrowUp, NONE), b"\x1b[A");
        let app = KeyModes {
            app_cursor: true,
            ..Default::default()
        };
        assert_eq!(encode_key(Key::ArrowUp, NONE, app).unwrap(), b"\x1bOA");
        let shift = Mods {
            shift: true,
            ..NONE
        };
        assert_eq!(encode_key(Key::ArrowUp, shift, app).unwrap(), b"\x1b[1;2A");
        assert_eq!(k(Key::ArrowLeft, CTRL), b"\x1b[1;5D");
        assert_eq!(k(Key::Home, NONE), b"\x1b[H");
        assert_eq!(k(Key::Delete, NONE), b"\x1b[3~");
        assert_eq!(k(Key::PageUp, Mods { alt: true, ..NONE }), b"\x1b[5;3~");
        assert_eq!(k(Key::F1, NONE), b"\x1bOP");
        assert_eq!(k(Key::F5, NONE), b"\x1b[15~");
        assert_eq!(k(Key::F12, shift), b"\x1b[24;2~");
    }

    #[test]
    fn control_and_meta() {
        assert_eq!(k(Key::C, CTRL), [0x03]);
        assert_eq!(k(Key::OpenBracket, CTRL), [0x1b]);
        assert_eq!(k(Key::Space, CTRL), [0x00]);
        assert_eq!(k(Key::Backspace, NONE), [0x7f]);
        assert_eq!(
            k(
                Key::Tab,
                Mods {
                    shift: true,
                    ..NONE
                }
            ),
            b"\x1b[Z"
        );
        assert_eq!(k(Key::Enter, NONE), b"\r");
        assert!(
            encode_key(Key::A, NONE, KeyModes::default()).is_none(),
            "plain text goes via text events"
        );
        // Option+letter is text (prefixed with ESC by the terminal in Meta mode); keys that
        // are not text carry the Alt modifier themselves.
        let alt = Mods { alt: true, ..NONE };
        assert!(encode_key(Key::B, alt, KeyModes::default()).is_none());
        assert_eq!(k(Key::Enter, alt), b"\x1b\r");
        assert_eq!(k(Key::ArrowLeft, alt), b"\x1b[1;3D");
    }

    const SHIFT: Mods = Mods {
        shift: true,
        ..NONE
    };
    const ALT: Mods = Mods { alt: true, ..NONE };

    fn ev(key: Key, mods: Mods, action: KeyAction) -> KeyEvent {
        KeyEvent {
            key,
            physical: Some(key),
            mods,
            action,
        }
    }

    /// A key event under the kitty `flags`; `None` when its text is typed instead.
    fn kitty(flags: u8, e: KeyEvent, text: Option<&str>) -> Option<Vec<u8>> {
        let modes = KeyModes {
            kitty: flags,
            ..Default::default()
        };
        encode_event(&e, text, modes)
    }

    fn press(flags: u8, key: Key, mods: Mods) -> Vec<u8> {
        kitty(flags, ev(key, mods, KeyAction::Press), None).expect("sent as a key")
    }

    #[test]
    fn kitty_disambiguates_keys_with_modifiers() {
        let d = DISAMBIGUATE;
        let ctrl_shift = Mods { ctrl: true, ..SHIFT };
        for (key, mods, want) in [
            (Key::Enter, SHIFT, &b"\x1b[13;2u"[..]),
            (Key::Enter, CTRL, b"\x1b[13;5u"),
            (Key::Enter, ALT, b"\x1b[13;3u"),
            (Key::Escape, NONE, b"\x1b[27u"),
            (Key::Escape, SHIFT, b"\x1b[27;2u"),
            (Key::C, CTRL, b"\x1b[99;5u"),
            (Key::Tab, SHIFT, b"\x1b[9;2u"),
            (Key::Backspace, CTRL, b"\x1b[127;5u"),
            (Key::A, ctrl_shift, b"\x1b[97;6u"),
            (Key::B, ALT, b"\x1b[98;3u"),
            (Key::Space, CTRL, b"\x1b[32;5u"),
            (Key::OpenBracket, CTRL, b"\x1b[91;5u"),
            // Functional keys keep their legacy CSI forms.
            (Key::ArrowUp, NONE, b"\x1b[A"),
            (Key::ArrowUp, SHIFT, b"\x1b[1;2A"),
            (Key::ArrowLeft, CTRL, b"\x1b[1;5D"),
            (Key::End, ALT, b"\x1b[1;3F"),
            (Key::Delete, NONE, b"\x1b[3~"),
            (Key::PageUp, ALT, b"\x1b[5;3~"),
            (Key::F1, NONE, b"\x1b[P"),
            (Key::F3, NONE, b"\x1b[13~"),
            (Key::F3, SHIFT, b"\x1b[13;2~"),
            (Key::F5, NONE, b"\x1b[15~"),
            (Key::F12, CTRL, b"\x1b[24;5~"),
            (Key::F13, NONE, b"\x1b[57376u"),
            // Without modifiers Enter, Tab and Backspace stay as they were.
            (Key::Enter, NONE, b"\r"),
            (Key::Tab, NONE, b"\t"),
            (Key::Backspace, NONE, b"\x7f"),
        ] {
            assert_eq!(press(d, key, mods), want, "{key:?} {mods:?}");
        }
        // Cursor key mode makes no SS3 of them.
        let app = KeyModes {
            app_cursor: true,
            kitty: d,
        };
        assert_eq!(encode_event(&ev(Key::ArrowUp, NONE, KeyAction::Press), None, app).unwrap(), b"\x1b[A");
        // Plain and shifted text is typed as text.
        assert_eq!(kitty(d, ev(Key::A, NONE, KeyAction::Press), Some("a")), None);
        assert_eq!(kitty(d, ev(Key::A, SHIFT, KeyAction::Press), Some("A")), None);
        assert_eq!(kitty(d, ev(Key::Space, SHIFT, KeyAction::Press), Some(" ")), None);
    }

    #[test]
    fn without_flags_keys_are_sent_as_before() {
        for (key, mods) in [
            (Key::Enter, SHIFT),
            (Key::Enter, ALT),
            (Key::Escape, NONE),
            (Key::C, CTRL),
            (Key::Tab, SHIFT),
            (Key::ArrowUp, SHIFT),
            (Key::F1, NONE),
            (Key::F5, NONE),
            (Key::A, NONE),
            (Key::B, ALT),
        ] {
            let modes = KeyModes::default();
            let event = encode_event(&ev(key, mods, KeyAction::Press), Some("x"), modes);
            assert_eq!(event, encode_key(key, mods, modes), "{key:?} {mods:?}");
        }
        assert_eq!(press(0, Key::Enter, SHIFT), b"\r");
        // Repeats are presses; releases are not sent.
        assert_eq!(kitty(0, ev(Key::Enter, NONE, KeyAction::Repeat), None).unwrap(), b"\r");
        assert_eq!(kitty(0, ev(Key::Enter, NONE, KeyAction::Release), None).unwrap(), b"");
        // Alternate keys and associated text alone change nothing.
        assert_eq!(press(ALTERNATE_KEYS | ASSOCIATED_TEXT, Key::Enter, SHIFT), b"\r");
        assert_eq!(press(ALTERNATE_KEYS, Key::A, CTRL), [0x01]);
    }

    #[test]
    fn kitty_event_types() {
        let de = DISAMBIGUATE | EVENT_TYPES;
        let release = |flags, key, mods| kitty(flags, ev(key, mods, KeyAction::Release), None).unwrap();
        let repeat = |flags, key, mods| kitty(flags, ev(key, mods, KeyAction::Repeat), None);
        assert_eq!(release(de, Key::A, CTRL), b"\x1b[97;5:3u");
        assert_eq!(release(de, Key::Escape, NONE), b"\x1b[27;1:3u");
        assert_eq!(release(de, Key::ArrowUp, NONE), b"\x1b[1;1:3A");
        assert_eq!(release(de, Key::F5, SHIFT), b"\x1b[15;2:3~");
        assert_eq!(repeat(de, Key::ArrowUp, NONE).unwrap(), b"\x1b[1;1:2A");
        assert_eq!(repeat(de, Key::Enter, SHIFT).unwrap(), b"\x1b[13;2:2u");
        // Enter, Tab and Backspace are released only with every key as an escape code, and text
        // is typed (and not released) as text.
        assert_eq!(release(de, Key::Enter, SHIFT), b"");
        assert_eq!(release(de, Key::A, NONE), b"");
        assert_eq!(repeat(de, Key::A, NONE), None);
        assert_eq!(repeat(de, Key::Enter, NONE).unwrap(), b"\r");
        // Without event types: a repeat is a press, a release is not sent.
        assert_eq!(repeat(DISAMBIGUATE, Key::A, CTRL).unwrap(), b"\x1b[97;5u");
        assert_eq!(release(DISAMBIGUATE, Key::A, CTRL), b"");
        // Event types alone: the CSI forms get them, legacy bytes stay.
        assert_eq!(release(EVENT_TYPES, Key::ArrowUp, NONE), b"\x1b[1;1:3A");
        assert_eq!(press(EVENT_TYPES, Key::A, CTRL), [0x01]);
        assert_eq!(release(EVENT_TYPES, Key::A, CTRL), b"");
        assert_eq!(press(EVENT_TYPES, Key::Escape, NONE), [0x1b]);
    }

    #[test]
    fn kitty_alternate_keys() {
        let da = DISAMBIGUATE | ALTERNATE_KEYS;
        let ctrl_shift = Mods { ctrl: true, ..SHIFT };
        assert_eq!(press(da, Key::A, ctrl_shift), b"\x1b[97:65;6u");
        assert_eq!(press(da, Key::A, CTRL), b"\x1b[97;5u");
        // egui names a shifted punctuation key by what it types: the key is the unshifted one.
        let bang = KeyEvent {
            physical: Some(Key::Num1),
            ..ev(Key::Exclamationmark, ctrl_shift, KeyAction::Press)
        };
        assert_eq!(kitty(da, bang, None).unwrap(), b"\x1b[49:33;6u");
        // Dvorak's O sits where a US layout has S: that is its base layout key.
        let dvorak = KeyEvent {
            physical: Some(Key::S),
            ..ev(Key::O, CTRL, KeyAction::Press)
        };
        assert_eq!(kitty(da, dvorak, None).unwrap(), b"\x1b[111::115;5u");
    }

    #[test]
    fn kitty_all_keys_and_associated_text() {
        let all = ALL_KEYS;
        let typed = |flags, e: KeyEvent, text| kitty(flags, e, Some(text)).unwrap();
        let a = ev(Key::A, NONE, KeyAction::Press);
        let shift_a = ev(Key::A, SHIFT, KeyAction::Press);
        assert_eq!(typed(all, a, "a"), b"\x1b[97u");
        assert_eq!(typed(all, shift_a, "A"), b"\x1b[97;2u");
        assert_eq!(typed(all, ev(Key::Space, SHIFT, KeyAction::Press), " "), b"\x1b[32;2u");
        assert_eq!(press(all, Key::Enter, NONE), b"\x1b[13u");
        assert_eq!(press(all, Key::Tab, NONE), b"\x1b[9u");
        assert_eq!(press(all, Key::Backspace, NONE), b"\x1b[127u");
        assert_eq!(press(all, Key::A, CTRL), b"\x1b[97;5u");
        // A layout egui has no names for (Cyrillic): the text names the key, its place on a US
        // layout is the base layout key.
        assert_eq!(typed(all, a, "ф"), b"\x1b[1092u");
        assert_eq!(typed(all | ALTERNATE_KEYS, a, "ф"), b"\x1b[1092::97u");
        assert_eq!(typed(all | ALTERNATE_KEYS, shift_a, "A"), b"\x1b[97:65;2u");
        // Shift+1 on a US layout.
        let bang = KeyEvent {
            physical: Some(Key::Num1),
            ..ev(Key::Exclamationmark, SHIFT, KeyAction::Press)
        };
        assert_eq!(typed(all | ALTERNATE_KEYS, bang, "!"), b"\x1b[49:33;2u");
        // Associated text.
        let text = all | ASSOCIATED_TEXT;
        assert_eq!(typed(text, a, "a"), b"\x1b[97;1;97u");
        assert_eq!(typed(text, shift_a, "A"), b"\x1b[97;2;65u");
        assert_eq!(press(text, Key::A, CTRL), b"\x1b[97;5u");
        // Releases (with event types), Enter's too.
        let released = |key, mods| kitty(all | EVENT_TYPES, ev(key, mods, KeyAction::Release), None).unwrap();
        assert_eq!(released(Key::A, NONE), b"\x1b[97;1:3u");
        assert_eq!(released(Key::Enter, NONE), b"\x1b[13;1:3u");
        // A dead key types nothing: nothing is sent for it.
        assert_eq!(kitty(all, a, None).unwrap(), b"");
        // Associated text alone is undefined: text stays text.
        assert_eq!(kitty(ASSOCIATED_TEXT, a, Some("a")), None);
    }

    #[test]
    fn paste_and_mouse() {
        assert_eq!(encode_paste("a\nb", false), b"a\rb");
        assert_eq!(
            encode_paste("x\x1b[201~y\x03", true),
            b"\x1b[200~x[201~y\x1b[201~"
        );
        assert_eq!(
            encode_mouse(MouseButton::Left, MouseAction::Press, NONE, 0, 0, true).unwrap(),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(
            encode_mouse(MouseButton::Left, MouseAction::Release, NONE, 9, 4, true).unwrap(),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(
            encode_mouse(
                MouseButton::WheelDown,
                MouseAction::Press,
                NONE,
                1,
                1,
                false
            )
            .unwrap(),
            [0x1b, b'[', b'M', 32 + 65, 34, 34]
        );
        assert!(encode_mouse(MouseButton::Left, MouseAction::Press, NONE, 300, 1, false).is_none());
    }
}
