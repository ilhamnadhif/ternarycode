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

/// Smooth display-synced running spinner: subtle circular track ring with a continuously rotating,
/// single-mesh anti-aliased tapered comet arc and rounded leading cap (no stepped jumps, no
/// segment seams, no flat square stroke ends).
pub fn paint_spinner(painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
    let ctx = painter.ctx();
    let t = ctx.input(|i| i.time);
    let ppp = ctx.pixels_per_point().max(1.0);
    let r = (size * 0.37).max(3.5);
    let w = (size * 0.145).clamp(1.6, 2.3);

    // Faint full track ring anchors the circle cleanly with zero visual wobble.
    painter.circle_stroke(center, r, egui::Stroke::new(w, super::style::soft(color, 0.18)));

    // Continuous smooth rotation (~0.88 s per turn), synced to display refresh (60/120 Hz).
    let head = ((t * 1.14).fract() as f32) * std::f32::consts::TAU;
    let sweep = std::f32::consts::TAU * 0.78;
    let steps = 36usize;

    // One continuous anti-aliased triangle strip with smooth per-vertex alpha tapering from an
    // invisible tail (0%) to a crisp leading edge (100%) — zero segment joints or double-blending.
    let aa = 0.85 / ppp;
    let hw = ((w - aa) * 0.5).max(0.3);
    let r_in_aa = (r - hw - aa).max(0.0);
    let r_in = (r - hw).max(0.0);
    let r_out = r + hw;
    let r_out_aa = r + hw + aa;
    let clear = Color32::TRANSPARENT;

    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve((steps + 1) * 4 + 20);
    mesh.indices.reserve(steps * 18 + 48);

    for i in 0..=steps {
        let frac = i as f32 / steps as f32;
        let a = head - sweep * (1.0 - frac);
        let (sin, cos) = a.sin_cos();
        let c = super::style::soft(color, frac.powf(1.12));
        let p = |rad: f32| egui::pos2(center.x + rad * cos, center.y + rad * sin);
        mesh.colored_vertex(p(r_in_aa), clear);
        mesh.colored_vertex(p(r_in), c);
        mesh.colored_vertex(p(r_out), c);
        mesh.colored_vertex(p(r_out_aa), clear);
        if i > 0 {
            let cur = (i * 4) as u32;
            let prev = cur - 4;
            for band in 0..3u32 {
                mesh.add_triangle(prev + band, cur + band, cur + band + 1);
                mesh.add_triangle(prev + band, cur + band + 1, prev + band + 1);
            }
        }
    }

    // Semicircular rounded leading cap extending smoothly past `head` with zero overlap against
    // the arc strip (so both opaque and translucent colors stay completely seam-free).
    let (hsin, hcos) = head.sin_cos();
    let tip = egui::pos2(center.x + r * hcos, center.y + r * hsin);
    let cap_steps = 8usize;
    let tip_idx = mesh.vertices.len() as u32;
    mesh.colored_vertex(tip, color);
    let base_idx = mesh.vertices.len() as u32;
    for j in 0..=cap_steps {
        let phi = head + std::f32::consts::PI * (j as f32 / cap_steps as f32);
        let (psin, pcos) = phi.sin_cos();
        mesh.colored_vertex(egui::pos2(tip.x + hw * pcos, tip.y + hw * psin), color);
        mesh.colored_vertex(egui::pos2(tip.x + (hw + aa) * pcos, tip.y + (hw + aa) * psin), clear);
        if j > 0 {
            let c0 = base_idx + ((j - 1) * 2) as u32;
            let c1 = base_idx + (j * 2) as u32;
            mesh.add_triangle(tip_idx, c0, c1);
            mesh.add_triangle(c0, c0 + 1, c1 + 1);
            mesh.add_triangle(c0, c1 + 1, c1);
        }
    }

    painter.add(egui::Shape::mesh(mesh));
    ctx.request_repaint();
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
