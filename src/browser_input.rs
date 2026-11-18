//! An agent's clicks and keys as a person's: real mouse and keyboard events (NSEvent) handed to
//! the page's web view, not events made up by a script in the page. WebKit takes them as the
//! user's own, so they reach into frames of other sites (a card form, a captcha), count as a
//! user action (a file chooser, the clipboard, a pop-up) and type into any editor.
//!
//! The events go to the web view itself, not through the app (`NSApplication sendEvent`): the
//! app's own click watch (`browser::clicked_in`) does not see them, so the agent's click is not
//! taken for the user taking the page over, and the user's typing elsewhere is not disturbed.
//!
//! Points are CSS pixels from the top left of the page's viewport (what `getBoundingClientRect`
//! gives), drawn at the page's zoom.

use std::collections::VecDeque;

use objc2::rc::Retained;
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSPoint, NSProcessInfo, NSString};
use objc2_web_kit::WKWebView;
use parking_lot::Mutex;

/// The agent's last key downs, by their time (it names an event): WebKit hands a key its page
/// did not take on to the app (`NSApp sendEvent`), which drops it (`agents_key`), so an agent's
/// typing never lands in a terminal.
static AGENT_KEYS: Mutex<VecDeque<u64>> = Mutex::new(VecDeque::new());

/// Whether a key event is one an agent pressed in a page.
pub fn agents_key(ev: &NSEvent) -> bool {
    AGENT_KEYS.lock().contains(&ev.timestamp().to_bits())
}

/// A point of the page (CSS pixels) in its window.
fn in_window(wk: &WKWebView, x: f64, y: f64) -> NSPoint {
    // SAFETY: a plain WKWebView getter on the main thread.
    let z = unsafe { wk.pageZoom() };
    let (px, py) = (x * z, y * z);
    let p = if wk.isFlipped() { NSPoint::new(px, py) } else { NSPoint::new(px, wk.bounds().size.height - py) };
    wk.convertPoint_toView(p, None)
}

fn now() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

fn mouse(wk: &WKWebView, ty: NSEventType, at: NSPoint, count: isize) -> Option<Retained<NSEvent>> {
    let window = wk.window()?;
    let pressure = if matches!(ty, NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged) { 1.0 } else { 0.0 };
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        ty,
        at,
        NSEventModifierFlags::empty(),
        now(),
        window.windowNumber(),
        None,
        0,
        count,
        pressure,
    )
}

/// The pointer comes into the page and moves to (`x`, `y`) of it (hover). WebKit takes moves
/// only once the pointer has come in (its own tracking of the mouse), so it is told so first.
pub fn hover(wk: &WKWebView, x: f64, y: f64) -> bool {
    let at = in_window(wk, x, y);
    let Some(window) = wk.window() else { return false };
    // SAFETY: an entered event for the web view's window, on the main thread.
    let entered = unsafe {
        NSEvent::enterExitEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_trackingNumber_userData(
            NSEventType::MouseEntered,
            at,
            NSEventModifierFlags::empty(),
            now(),
            window.windowNumber(),
            None,
            0,
            0,
            std::ptr::null_mut(),
        )
    };
    if let Some(e) = entered {
        wk.mouseEntered(&e);
    }
    for p in [NSPoint::new(at.x - 1.0, at.y), at] {
        if let Some(e) = mouse(wk, NSEventType::MouseMoved, p, 0) {
            wk.mouseMoved(&e);
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                objc2_app_kit::NSApplication::sharedApplication(mtm).postEvent_atStart(&e, false);
            }
        }
    }
    true
}

/// A click (`count` 2: a double click) at (`x`, `y`) of the page.
pub fn click(wk: &WKWebView, x: f64, y: f64, count: isize) -> bool {
    let at = in_window(wk, x, y);
    if let Some(e) = mouse(wk, NSEventType::MouseMoved, at, 0) {
        wk.mouseMoved(&e);
    }
    for n in 1..=count.max(1) {
        let (Some(down), Some(up)) = (mouse(wk, NSEventType::LeftMouseDown, at, n), mouse(wk, NSEventType::LeftMouseUp, at, n)) else { return false };
        wk.mouseDown(&down);
        wk.mouseUp(&up);
    }
    true
}

/// A drag with the button held from one point of the page to another, in steps.
pub fn drag(wk: &WKWebView, from: (f64, f64), to: (f64, f64)) -> bool {
    let a = in_window(wk, from.0, from.1);
    let Some(down) = mouse(wk, NSEventType::LeftMouseDown, a, 1) else { return false };
    if let Some(e) = mouse(wk, NSEventType::MouseMoved, a, 0) {
        wk.mouseMoved(&e);
    }
    wk.mouseDown(&down);
    const STEPS: usize = 12;
    for i in 1..=STEPS {
        let f = i as f64 / STEPS as f64;
        let p = in_window(wk, from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
        if let Some(e) = mouse(wk, NSEventType::LeftMouseDragged, p, 1) {
            wk.mouseDragged(&e);
        }
    }
    if let Some(up) = mouse(wk, NSEventType::LeftMouseUp, in_window(wk, to.0, to.1), 1) {
        wk.mouseUp(&up);
    }
    true
}

/// A named key as the page knows it (`Enter`, `Tab`, `ArrowDown`…), or one character: its key
/// code, its characters and whether ⇧ is held. `None`: not a key this knows.
pub fn key_of(key: &str) -> Option<(u16, String, bool)> {
    let f = |c: u32| char::from_u32(c).map(String::from).unwrap_or_default();
    Some(match key {
        "Enter" => (36, "\r".into(), false),
        "Tab" => (48, "\t".into(), false),
        "Escape" | "Esc" => (53, "\u{1b}".into(), false),
        "Backspace" => (51, "\u{7f}".into(), false),
        "Delete" => (117, f(0xF728), false),
        " " | "Space" => (49, " ".into(), false),
        "ArrowUp" => (126, f(0xF700), false),
        "ArrowDown" => (125, f(0xF701), false),
        "ArrowLeft" => (123, f(0xF702), false),
        "ArrowRight" => (124, f(0xF703), false),
        "Home" => (115, f(0xF729), false),
        "End" => (119, f(0xF72B), false),
        "PageUp" => (116, f(0xF72C), false),
        "PageDown" => (121, f(0xF72D), false),
        k if k.chars().count() == 1 => {
            let c = k.chars().next()?;
            let (code, shift) = char_code(c);
            (code, k.to_string(), shift)
        }
        _ => return None,
    })
}

/// The US key code of a character, and whether it takes ⇧ (0 for one not on that keyboard: the
/// character still goes in).
fn char_code(c: char) -> (u16, bool) {
    const LETTERS: &[(char, u16)] = &[
        ('a', 0), ('s', 1), ('d', 2), ('f', 3), ('h', 4), ('g', 5), ('z', 6), ('x', 7), ('c', 8), ('v', 9), ('b', 11), ('q', 12), ('w', 13), ('e', 14), ('r', 15),
        ('y', 16), ('t', 17), ('o', 31), ('u', 32), ('i', 34), ('p', 35), ('l', 37), ('j', 38), ('k', 40), ('n', 45), ('m', 46),
    ];
    const OTHERS: &[(char, char, u16)] = &[
        ('1', '!', 18), ('2', '@', 19), ('3', '#', 20), ('4', '$', 21), ('6', '^', 22), ('5', '%', 23), ('=', '+', 24), ('9', '(', 25), ('7', '&', 26), ('-', '_', 27),
        ('8', '*', 28), ('0', ')', 29), (']', '}', 30), ('[', '{', 33), ('\'', '"', 39), (';', ':', 41), ('\\', '|', 42), (',', '<', 43), ('/', '?', 44), ('.', '>', 47), ('`', '~', 50),
    ];
    if let Some((_, code)) = LETTERS.iter().find(|(l, _)| *l == c.to_ascii_lowercase()) {
        return (*code, c.is_ascii_uppercase());
    }
    if let Some((plain, _, code)) = OTHERS.iter().find(|(p, s, _)| *p == c || *s == c) {
        return (*code, *plain != c);
    }
    if c == ' ' {
        return (49, false);
    }
    (0, false)
}

/// A key pressed and let go in the page (its focused element), the web view taking the keyboard
/// for it.
fn press(wk: &WKWebView, code: u16, chars: &str, shift: bool) -> bool {
    let Some(window) = wk.window() else { return false };
    window.makeFirstResponder(Some(wk));
    let flags = if shift { NSEventModifierFlags::Shift } else { NSEventModifierFlags::empty() };
    let unmod = if shift { chars.to_lowercase() } else { chars.to_string() };
    let (keys, ukeys) = (NSString::from_str(chars), NSString::from_str(&unmod));
    let at = now();
    {
        let mut mine = AGENT_KEYS.lock();
        if mine.len() >= 64 {
            mine.pop_front();
        }
        mine.push_back(at.to_bits());
    }
    let mk = |ty| {
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty,
            NSPoint::new(0.0, 0.0),
            flags,
            at,
            window.windowNumber(),
            None,
            &keys,
            &ukeys,
            false,
            code,
        )
    };
    let (Some(down), Some(up)) = (mk(NSEventType::KeyDown), mk(NSEventType::KeyUp)) else { return false };
    wk.keyDown(&down);
    wk.keyUp(&up);
    true
}

/// Presses a key (`Enter`, `Tab`, `a`…) in the page. Err: a key this does not know.
pub fn key(wk: &WKWebView, key: &str) -> Result<(), String> {
    let (code, chars, shift) = key_of(key).ok_or_else(|| format!("Unknown key {key:?}: use Enter, Tab, Escape, Backspace, Delete, Space, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, or one character."))?;
    if press(wk, code, &chars, shift) { Ok(()) } else { Err("The page has no window.".into()) }
}

/// The page takes the keyboard of its window, as when the user clicks in it. False: it had it
/// (or has no window).
pub fn take_keyboard(wk: &WKWebView) -> bool {
    if crate::browser::view_has_keyboard(wk) {
        return false;
    }
    let Some(window) = wk.window() else { return false };
    window.makeFirstResponder(Some(wk));
    true
}

/// Puts `text` in at once where the page has its selection (replacing it), as the system's
/// input does (dictation, the emoji picker): the page hears it as typed (`beforeinput`,
/// `textInput`), which editors such as x.com's want.
pub fn insert_text(wk: &WKWebView, text: &str) -> bool {
    if wk.window().is_none() {
        return false;
    }
    let s = NSString::from_str(text);
    let none = objc2_foundation::NSRange::new(objc2_foundation::NSNotFound as usize, 0);
    // SAFETY: WKWebView's NSTextInputClient method (macOS 10.5+) with an NSString and "no range"
    // (the selection), on the main thread.
    unsafe {
        let _: () = objc2::msg_send![wk, insertText: &*s, replacementRange: none];
    }
    true
}

/// Types `text` into what has the keyboard in the page, a key at a time (a line break is Enter).
pub fn type_text(wk: &WKWebView, text: &str) -> bool {
    for c in text.chars() {
        let ok = match c {
            '\n' => press(wk, 36, "\r", false),
            '\t' => press(wk, 48, "\t", false),
            c => {
                let (code, shift) = char_code(c);
                press(wk, code, &c.to_string(), shift)
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_as_a_us_keyboard() {
        assert_eq!(key_of("Enter"), Some((36, "\r".to_string(), false)));
        assert_eq!(key_of("a"), Some((0, "a".to_string(), false)));
        assert_eq!(key_of("A"), Some((0, "A".to_string(), true)));
        assert_eq!(key_of("?"), Some((44, "?".to_string(), true)));
        assert_eq!(key_of("é"), Some((0, "é".to_string(), false)));
        assert_eq!(key_of("F13"), None);
    }
}
