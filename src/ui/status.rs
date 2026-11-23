//! How statuses look: text, icon and colour (colour and shape together, PRD section 12), and
//! the source shown in tooltips.

use eframe::egui::{self, Color32};
use tc_core::status::{Source, Status, Tracker};

use super::icons;
use super::style::Palette;
use crate::i18n::{t, tf};

pub fn text(s: Status) -> String {
    match s {
        Status::Idle => t("Idle").into(),
        Status::Running => t("Running").into(),
        Status::NeedsInput => t("Needs input").into(),
        Status::Failed(c) => tf("Failed (code {})", &[&c.to_string()]),
        Status::Done => t("Done").into(),
        Status::CommandDone => t("Command finished").into(),
        // An agent runs, and says nothing about what it does (no hooks): it is just active.
        Status::Unknown => t("Active").into(),
        Status::Ended => t("Ended").into(),
    }
}

pub fn icon(s: Status) -> &'static str {
    match s {
        Status::Idle => icons::CIRCLE,
        Status::Running => icons::CIRCLE_NOTCH,
        Status::NeedsInput => icons::CHAT_CIRCLE_DOTS,
        Status::Failed(_) => icons::X_CIRCLE,
        Status::Done | Status::CommandDone => icons::CHECK_CIRCLE,
        Status::Unknown => icons::CIRCLE,
        Status::Ended => icons::STOP,
    }
}

/// Whether the status's icon is drawn filled (Needs input, Done, Failed).
pub fn filled(s: Status) -> bool {
    matches!(s, Status::NeedsInput | Status::Failed(_) | Status::Done | Status::CommandDone)
}

pub fn color(s: Status, p: &Palette) -> Color32 {
    match s {
        Status::Running => p.info,
        Status::NeedsInput => p.warn,
        Status::Failed(_) => p.err,
        Status::Done | Status::CommandDone => p.ok,
        Status::Unknown => p.idle,
        Status::Idle | Status::Ended => p.idle,
    }
}

/// Whether the status has a glyph of its own (Ended has none).
pub fn has_glyph(s: Status) -> bool {
    !matches!(s, Status::Ended)
}

/// Paints a status's glyph centred at `center`, `size` across (12 small, 14 medium): the Slate
/// status icons. Running is a ring with a gap that turns (the only thing that moves); Idle and
/// Active a hollow circle; Needs input a chat bubble, Done a check, Failed a cross (filled).
pub fn paint_glyph(painter: &egui::Painter, center: egui::Pos2, s: Status, p: &Palette, size: f32) {
    let c = color(s, p);
    let size = if size < 13.0 { 12.0 } else { 14.0 };
    match s {
        Status::Running => paint_spinner(painter, center, size, c),
        Status::Ended => {}
        _ => {
            let font = if filled(s) { super::style::icon_fill(size) } else { super::style::icon_font(size) };
            painter.text(center, egui::Align2::CENTER_CENTER, icon(s), font, c);
        }
    }
}

/// The running glyph: Phosphor's circle-notch, drawn as an arc that turns once a second.
pub fn paint_spinner(painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
    let t = painter.ctx().input(|i| i.time) as f32;
    let r = size * 0.36;
    let start = t * std::f32::consts::TAU;
    let sweep = std::f32::consts::TAU * 0.72;
    let n = 24;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = start + sweep * i as f32 / n as f32;
            egui::pos2(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(pts, egui::Stroke::new((size * 0.11).max(1.3), color)));
    painter.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

pub fn source(src: Source) -> String {
    match src {
        Source::None => String::new(),
        Source::Process => t("foreground process").into(),
        Source::Hook(a) => tf("{} hook", &[a.label()]),
        Source::Osc => t("terminal notification (OSC)").into(),
        Source::Bell => t("terminal bell").into(),
        Source::Shell => t("shell integration").into(),
        Source::Input => t("your input").into(),
    }
}

/// A pane's status and where it came from, for tooltips: "Needs input · from Claude Code hook,
/// 3 s ago" (only the status when nothing said it).
pub fn described(tracker: &Tracker) -> String {
    let src = source(tracker.source);
    if src.is_empty() {
        text(tracker.status)
    } else {
        tf("{} · from {}, {} ago", &[&text(tracker.status), &src, &age(tracker.since)])
    }
}

/// "12s", "4m", "2h" since `at`: the short age of lists (the inbox, the attention list).
pub fn short_age(at: std::time::Instant) -> String {
    let s = at.elapsed().as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// The colour of a pane's dot in a project's row (its status; quiet panes faint).
pub fn dot_color(s: Status, p: &Palette) -> Color32 {
    match s {
        Status::Running | Status::NeedsInput | Status::Failed(_) | Status::Done | Status::CommandDone => color(s, p),
        _ => super::style::soft(p.text_faint, 0.55),
    }
}

/// "3 s", "5 min", "2 h" since `at`.
pub fn age(at: std::time::Instant) -> String {
    let s = at.elapsed().as_secs();
    if s < 60 {
        tf("{} s", &[&s.to_string()])
    } else if s < 3600 {
        tf("{} min", &[&(s / 60).to_string()])
    } else {
        tf("{} h", &[&(s / 3600).to_string()])
    }
}
