//! Debug builds only: a scripted end-to-end test driver.
//!
//! `TERNARYCODE_DEV_SCRIPT="sleep 800;; key cmd+d;; keys echo hi\r;; expect-screen hi;; quit"` runs
//! steps one after another. Keyboard and mouse steps inject real `NSEvent`s into the window, so
//! they travel the same path as user input (menu key equivalents, winit, egui, the widgets).
//! Results are printed to stderr as `DEVTEST PASS|FAIL …` lines and a final `DEVTEST DONE`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use eframe::egui::{self, Rect};

// ----- script ---------------------------------------------------------------------------------

#[derive(Default)]
pub struct Script {
    steps: VecDeque<String>,
    wait_until: Option<Instant>,
    pub passed: usize,
    pub failed: usize,
}

impl Script {
    pub fn from_env() -> Self {
        let steps = std::env::var("TERNARYCODE_DEV_SCRIPT")
            .map(|s| {
                s.split(";;")
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            steps,
            ..Default::default()
        }
    }

    /// The next step to run now, if any.
    pub fn next(&mut self, ctx: &egui::Context) -> Option<String> {
        if self.steps.is_empty() {
            return None;
        }
        // A `sleep` step wakes the app when it ends (not every 30 ms: that would make CPU
        // measurements of the app measure the script); other steps poll.
        match self.wait_until {
            Some(t) if Instant::now() < t => {
                ctx.request_repaint_after(t - Instant::now());
                return None;
            }
            _ => ctx.request_repaint_after(Duration::from_millis(30)),
        }
        self.wait_until = None;
        let step = self.steps.pop_front()?;
        if let Some(ms) = step.strip_prefix("sleep ") {
            self.wait_until =
                Some(Instant::now() + Duration::from_millis(ms.trim().parse().unwrap_or(500)));
            return None;
        }
        // Give injected events a few frames to be processed before the next step.
        self.wait_until = Some(Instant::now() + Duration::from_millis(120));
        Some(step)
    }

    /// Runs `steps` next, before the rest of the script.
    pub fn push_front(&mut self, steps: Vec<String>) {
        for step in steps.into_iter().rev() {
            self.steps.push_front(step);
        }
    }

    pub fn check(&mut self, ok: bool, what: &str) {
        if ok {
            self.passed += 1;
            eprintln!("DEVTEST PASS: {what}");
        } else {
            self.failed += 1;
            eprintln!("DEVTEST FAIL: {what}");
        }
    }
}

pub fn unescape(s: &str) -> String {
    s.replace("\\r", "\r")
        .replace("\\n", "\n")
        .replace("\\e", "\x1b")
        .replace("\\t", "\t")
}

// ----- named rectangles (widgets the script can click) ------------------------------------------

/// Each widget's last rectangle and the frame it was drawn in.
fn rects() -> &'static Mutex<HashMap<String, (Rect, u64)>> {
    static RECTS: OnceLock<Mutex<HashMap<String, (Rect, u64)>>> = OnceLock::new();
    RECTS.get_or_init(|| Mutex::new(HashMap::new()))
}

static FRAME: AtomicU64 = AtomicU64::new(0);

/// A new frame is being drawn (widgets recorded from now on belong to it).
pub fn begin_frame() {
    FRAME.fetch_add(1, Ordering::Relaxed);
}

/// Remembers where a named widget was drawn this frame.
pub fn record(name: impl Into<String>, rect: Rect) {
    rects().lock().unwrap().insert(name.into(), (rect, FRAME.load(Ordering::Relaxed)));
}

pub fn rect_of(name: &str) -> Option<Rect> {
    rects().lock().unwrap().get(name).map(|(r, _)| *r)
}

/// Input events for the next frame, added to what the window delivers (the mouse wheel: AppKit
/// has no way to make a scroll event for a window).
fn injected() -> &'static Mutex<Vec<egui::Event>> {
    static EVENTS: OnceLock<Mutex<Vec<egui::Event>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// The mouse wheel turned at `pos` (egui points): positive `dy` scrolls up.
pub fn wheel(pos: egui::Pos2, dy: f32) {
    injected().lock().unwrap().extend([
        egui::Event::PointerMoved(pos),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        },
    ]);
}

pub fn take_injected() -> Vec<egui::Event> {
    std::mem::take(&mut *injected().lock().unwrap())
}

/// The widget was drawn in the last frame.
pub fn shown(name: &str) -> bool {
    rects().lock().unwrap().get(name).is_some_and(|(_, f)| *f == FRAME.load(Ordering::Relaxed))
}

// ----- event injection --------------------------------------------------------------------------
// Real AppKit events on macOS. Elsewhere the steps that inject input only report that they
// were skipped; the rest of the script (actions, checks) runs everywhere.

#[cfg(target_os = "macos")]
pub use native::{click, key, mouse, type_text};

#[cfg(target_os = "macos")]
mod native {
    use eframe::egui;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    use objc2_foundation::{NSPoint, NSString};

    fn window() -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        let windows = app.windows();
        (0..windows.count())
            .map(|i| windows.objectAtIndex(i))
            .max_by(|a, b| {
                let (ra, rb) = (a.contentLayoutRect(), b.contentLayoutRect());
                (ra.size.width * ra.size.height).total_cmp(&(rb.size.width * rb.size.height))
            })
    }

    fn key_event(
        down: bool,
        flags: NSEventModifierFlags,
        chars: &str,
        unmod: &str,
        code: u16,
        wn: isize,
    ) -> Option<Retained<NSEvent>> {
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            if down { NSEventType::KeyDown } else { NSEventType::KeyUp },
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            wn,
            None,
            &NSString::from_str(chars),
            &NSString::from_str(unmod),
            false,
            code,
        )
    }

    /// US-layout key code for a character, and whether Shift is needed.
    pub(super) fn key_code(c: char) -> (u16, bool) {
        // Index = macOS virtual key code ('§'/'±' fill code 10, unused here).
        const PLAIN: &str = "asdfhgzxcv§bqweryt123465=97-80]ou[ip\nlj'k;\\,/nm.\t `";
        const SHIFTED: &str = "ASDFHGZXCV±BQWERYT!@#$^%+(&_*)}OU{IP\nLJ\"K:|<?NM>\t ~";
        if let Some(i) = PLAIN.chars().position(|x| x == c) {
            return (i as u16, false);
        }
        if let Some(i) = SHIFTED.chars().position(|x| x == c) {
            return (i as u16, true);
        }
        match c {
            '\r' => (36, false),
            '\x1b' => (53, false),
            _ => (0, false),
        }
    }

    /// Sends a key press. Command shortcuts go through the main menu first, like real key presses.
    fn press(flags: NSEventModifierFlags, chars: &str, unmod: &str, code: u16) {
        let Some(w) = window() else { return };
        let wn = w.windowNumber();
        let (Some(down), Some(up)) = (
            key_event(true, flags, chars, unmod, code, wn),
            key_event(false, flags, chars, unmod, code, wn),
        ) else {
            return;
        };
        if flags.contains(NSEventModifierFlags::Command)
            && let Some(mtm) = MainThreadMarker::new()
            && let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu()
            && menu.performKeyEquivalent(&down)
        {
            return;
        }
        w.sendEvent(&down);
        w.sendEvent(&up);
    }

    /// Types text as individual key presses.
    pub fn type_text(text: &str) {
        for c in text.chars() {
            let (code, shift) = key_code(c);
            let flags = if shift {
                NSEventModifierFlags::Shift
            } else {
                NSEventModifierFlags::empty()
            };
            let s = c.to_string();
            press(flags, &s, &s.to_lowercase(), code);
        }
    }

    /// A key combination such as `cmd+shift+d`, `ctrl+c`, `enter`, `esc`, `up`, `cmd+1`.
    pub fn key(spec: &str) {
        let mut flags = NSEventModifierFlags::empty();
        let mut name = "";
        for part in spec.split('+') {
            match part {
                "cmd" => flags |= NSEventModifierFlags::Command,
                "shift" => flags |= NSEventModifierFlags::Shift,
                "alt" | "opt" => flags |= NSEventModifierFlags::Option,
                "ctrl" => flags |= NSEventModifierFlags::Control,
                other => name = other,
            }
        }
        let arrow = |code: u16, ch: u32| (code, char::from_u32(ch).unwrap().to_string());
        let (code, chars) = match name {
            "enter" | "return" => (36, "\r".to_string()),
            "tab" => (48, "\t".to_string()),
            "esc" | "escape" => (53, "\x1b".to_string()),
            "backspace" => (51, "\x7f".to_string()),
            "space" => (49, " ".to_string()),
            "up" => arrow(126, 0xF700),
            "down" => arrow(125, 0xF701),
            "left" => arrow(123, 0xF702),
            "right" => arrow(124, 0xF703),
            "home" => arrow(115, 0xF729),
            "end" => arrow(119, 0xF72B),
            "pageup" => arrow(116, 0xF72C),
            "pagedown" => arrow(121, 0xF72D),
            "delete" => arrow(117, 0xF728),
            n if n.chars().count() == 1 => {
                let c = n.chars().next().unwrap();
                (key_code(c).0, n.to_string())
            }
            _ => return,
        };
        let unmod = chars.clone();
        let chars = if flags.contains(NSEventModifierFlags::Control)
            && chars.len() == 1
            && chars.as_bytes()[0].is_ascii_alphabetic()
        {
            ((chars.as_bytes()[0].to_ascii_lowercase() - b'a' + 1) as char).to_string()
        } else {
            chars
        };
        press(flags, &chars, &unmod, code);
    }

    /// The content view's height: with the content under the title bar it is the whole window,
    /// not just the area below the title bar.
    fn content_height(w: &NSWindow) -> f64 {
        w.contentView()
            .map(|v| v.frame().size.height)
            .unwrap_or_else(|| w.contentLayoutRect().size.height)
    }

    /// Clicks at a point in egui coordinates (points from the content's top-left).
    pub fn click(pos: egui::Pos2, count: isize, right: bool) {
        let Some(w) = window() else { return };
        // The content view's height: with the content under the title bar it is the whole
        // window, not just the area below the title bar.
        let height = content_height(&w);
        let loc = NSPoint::new(pos.x as f64, height - pos.y as f64);
        let wn = w.windowNumber();
        let mk = |ty, clicks| {
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                ty,
                loc,
                NSEventModifierFlags::empty(),
                0.0,
                wn,
                None,
                0,
                clicks,
                1.0,
            )
        };
        if let Some(m) = mk(NSEventType::MouseMoved, 0) {
            w.sendEvent(&m);
        }
        let (down, up) = if right {
            (NSEventType::RightMouseDown, NSEventType::RightMouseUp)
        } else {
            (NSEventType::LeftMouseDown, NSEventType::LeftMouseUp)
        };
        for n in 1..=count {
            if let (Some(d), Some(u)) = (mk(down, n), mk(up, n)) {
                w.sendEvent(&d);
                w.sendEvent(&u);
            }
        }
    }

    /// One left-button mouse event (`down`, `drag` or `up`) at a point in egui coordinates. A drag
    /// is a series of these in separate frames, as with a real mouse.
    pub fn mouse(kind: &str, pos: egui::Pos2) {
        let Some(w) = window() else { return };
        let height = content_height(&w);
        // `cmd-down`, `cmd-up`…: with ⌘ held.
        let (flags, kind) = match kind.strip_prefix("cmd-") {
            Some(k) => (NSEventModifierFlags::Command, k),
            None => (NSEventModifierFlags::empty(), kind),
        };
        let ty = match kind {
            "down" => NSEventType::LeftMouseDown,
            "drag" => NSEventType::LeftMouseDragged,
            "move" => NSEventType::MouseMoved,
            _ => NSEventType::LeftMouseUp,
        };
        if let Some(e) =
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                ty,
                NSPoint::new(pos.x as f64, height - pos.y as f64),
                flags,
                0.0,
                w.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
        {
            w.sendEvent(&e);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn skipped(step: &str) {
    eprintln!("DEVTEST LOG: {step}: input injection is macOS-only, skipped");
}

#[cfg(not(target_os = "macos"))]
pub fn type_text(_text: &str) {
    skipped("keys");
}

#[cfg(not(target_os = "macos"))]
pub fn key(_spec: &str) {
    skipped("key");
}

#[cfg(not(target_os = "macos"))]
pub fn click(_pos: egui::Pos2, _count: isize, _right: bool) {
    skipped("click");
}

#[cfg(not(target_os = "macos"))]
pub fn mouse(_kind: &str, _pos: egui::Pos2) {
    skipped("mouse");
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn key_codes() {
        assert_eq!(super::native::key_code('a'), (0, false));
        assert_eq!(super::native::key_code('t'), (17, false));
        assert_eq!(super::native::key_code(' '), (49, false));
        assert_eq!(super::native::key_code('\n'), (36, false));
        assert_eq!(super::native::key_code('L'), (37, true));
        assert_eq!(super::native::key_code('~'), (50, true));
    }
}
