//! The agents' own logos (Claude Code, Codex, OpenCode, Antigravity CLI) where an
//! agent is shown in a small tile, in place of two letters. The Claude and OpenAI (for Codex)
//! marks are Simple Icons' (CC0), drawn white into `assets/agents/` and tinted here: Claude in
//! its brand colour, OpenAI in the text colour (its mark is black or white).
//! Antigravity's mark is in its own colours (a gradient), from antigravity.google's logo, scaled
//! to 64 pixels. OpenCode's mark is two rectangles, drawn as such.

use eframe::egui::{self, Color32, Rect, pos2, vec2};
use tc_core::agent::AgentKind;

const CLAUDE: &[u8] = include_bytes!("../../assets/agents/claude.png");
const OPENAI: &[u8] = include_bytes!("../../assets/agents/openai.png");
const ANTIGRAVITY: &[u8] = include_bytes!("../../assets/agents/antigravity.png");

/// Claude's orange (Simple Icons' brand colour).
const CLAUDE_COLOR: Color32 = Color32::from_rgb(0xD9, 0x77, 0x57);

/// Whether the agent has a logo (else its tile shows letters).
pub fn has(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Codex | AgentKind::OpenCode | AgentKind::Gemini | AgentKind::Antigravity)
}

/// Paints the agent's logo filling the square `r`, `ink` for the marks in the text colour;
/// dimmed (an agent that has ended). Returns false when it has none.
pub fn paint(painter: &egui::Painter, r: Rect, agent: AgentKind, ink: Color32, dim: bool) -> bool {
    let fade = |c: Color32| if dim { c.gamma_multiply(0.45) } else { c };
    let (name, png, tint) = match agent {
        AgentKind::Claude => ("claude", CLAUDE, CLAUDE_COLOR),
        AgentKind::Codex => ("openai", OPENAI, ink),
        // Antigravity CLI (`agy`) replaced Gemini CLI: render Antigravity's logo in its own colours.
        AgentKind::Antigravity | AgentKind::Gemini => ("antigravity", ANTIGRAVITY, Color32::WHITE),
        AgentKind::OpenCode => {
            opencode(painter, r, fade(ink));
            return true;
        }
        _ => return false,
    };
    let Some(tex) = texture(painter.ctx(), name, png) else { return false };
    painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), fade(tint));
    true
}

/// OpenCode's mark (240 × 300): a frame with a lighter block in its lower half, fitted to `r`.
fn opencode(painter: &egui::Painter, r: Rect, ink: Color32) {
    let h = r.height();
    let w = h * 240.0 / 300.0;
    let origin = pos2(r.center().x - w / 2.0, r.top());
    let s = h / 300.0;
    let at = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(origin + vec2(x0 * s, y0 * s), origin + vec2(x1 * s, y1 * s));
    for part in [at(0.0, 0.0, 240.0, 60.0), at(0.0, 240.0, 240.0, 300.0), at(0.0, 60.0, 60.0, 240.0), at(180.0, 60.0, 240.0, 240.0)] {
        painter.rect_filled(part, 0.0, ink);
    }
    painter.rect_filled(at(60.0, 120.0, 180.0, 240.0), 0.0, ink.gamma_multiply(0.28));
}

/// The logo's texture, made once per app run.
fn texture(ctx: &egui::Context, name: &'static str, png: &'static [u8]) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("agent-logo", name));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let img = tc_doc::img::decode(png, 256).ok()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    // Mipmaps: the 64-pixel mark is drawn at 11 to 18 points.
    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let t = ctx.load_texture(format!("agent-logo-{name}"), image, options);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logos_decode() {
        for png in [CLAUDE, OPENAI, ANTIGRAVITY] {
            let img = tc_doc::img::decode(png, 256).unwrap();
            assert_eq!((img.width, img.height), (64, 64));
            // A mark on a clear ground: some pixels drawn, some not.
            let alpha: Vec<u8> = img.pixels.chunks(4).map(|p| p[3]).collect();
            assert!(alpha.contains(&255) && alpha.contains(&0));
        }
        assert!(has(AgentKind::OpenCode) && has(AgentKind::Antigravity) && !has(AgentKind::Custom));
    }
}
