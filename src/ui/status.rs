//! How statuses look: text, icon and colour (colour and shape together, PRD section 12), and
//! the source shown in tooltips.

use eframe::egui::Color32;
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
        Status::NeedsInput => icons::BELL,
        Status::Failed(_) => icons::X_CIRCLE,
        Status::Done | Status::CommandDone => icons::CHECK_CIRCLE,
        Status::Unknown => icons::CIRCLE_HALF,
        Status::Ended => icons::STOP,
    }
}

pub fn color(s: Status, p: &Palette) -> Color32 {
    match s {
        Status::Running => p.info,
        Status::NeedsInput => p.warn,
        Status::Failed(_) => p.err,
        Status::Done | Status::CommandDone => p.ok,
        Status::Unknown => p.text_muted,
        Status::Idle | Status::Ended => p.text_faint,
    }
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
