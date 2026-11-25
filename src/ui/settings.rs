//! Settings: one home for every setting. A modal with its pages at the left (General …
//! Notifications, Phone, then Updates and About), a search that narrows them, and each page as
//! groups of rows: what a setting is and one line about it at the left, its control at the
//! right. Saved as they change. Accounts (`profiles_ui.rs`) and Phone (`pairing_ui.rs`) draw their
//! pages with the rows and groups of this file.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Galley, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use tc_agent::integrate::InstallRecord;
use tc_core::agent::{AgentKind, Preset};
use tc_core::model::{CursorShape, Integration, ThemeMode};
use tc_core::status::Status;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Appearance,
    Terminal,
    /// Presets and how each agent reports to TernaryCode (its hooks).
    Agents,
    /// Agent accounts (profiles), their usage and the failover policy.
    Accounts,
    /// Browser panes: where web pages open, sleeping, logins, what agents may do.
    Browser,
    Editor,
    Notifications,
    /// The phone companion: its server, pairing and paired phones.
    Phone,
    Updates,
    About,
    /// Opens Agents (kept for "Agent Integrations…").
    Integrations,
}

/// The modal: fixed 960 × 640 (clamped cleanly on smaller windows) so switching tabs never
/// jumps or stretches the dialog; each page scrolls inside its body.
const W: f32 = 960.0;
const H: f32 = 640.0;
const NAV_W: f32 = 232.0;
/// The page's padding: 32 at the sides, 26 above, 30 below.
const PAD_X: f32 = 32.0;
const PAD_TOP: f32 = 26.0;
const PAD_BOTTOM: f32 = 30.0;
/// Between the head and the groups, and between groups.
const GAP: f32 = 24.0;

/// The nav, in order (None: the line before Updates), with each page's icon and name.
const NAV: [Option<(SettingsPage, &str, &str)>; 12] = [
    Some((SettingsPage::General, icons::GEAR_SIX, "General")),
    Some((SettingsPage::Appearance, icons::PALETTE, "Appearance")),
    Some((SettingsPage::Terminal, icons::TERMINAL_WINDOW, "Terminal")),
    Some((SettingsPage::Agents, icons::SPARKLE, "Agents")),
    Some((SettingsPage::Accounts, icons::USER_CIRCLE, "Accounts")),
    Some((SettingsPage::Browser, icons::GLOBE_SIMPLE, "Browser")),
    Some((SettingsPage::Editor, icons::CODE, "Editor")),
    Some((SettingsPage::Notifications, icons::BELL_SIMPLE, "Notifications")),
    Some((SettingsPage::Phone, icons::DEVICE_MOBILE, "Phone")),
    None,
    Some((SettingsPage::Updates, icons::ARROW_CIRCLE_UP, "Updates")),
    Some((SettingsPage::About, icons::INFO, "About")),
];

/// The rows of each page, for the search (their titles and labels; what the user adds, such as
/// accounts and presets, is matched by `App::page_matches`).
fn page_rows(page: SettingsPage) -> &'static [&'static str] {
    match page {
        SettingsPage::General => &["Command line", "Data folder", "Worktrees", "Where new worktrees go", "Branch names start with", "When a branch is merged"],
        SettingsPage::Appearance => &["Theme", "Terminal colours", "Dark theme", "Light theme", "Terminal text", "Font", "Size", "Cursor", "Blinking cursor", "Sidebar", "Text size", "Width"],
        SettingsPage::Terminal => {
            if cfg!(target_os = "macos") {
                &["Scrollback per pane", "Scrollback memory for all panes", "Option key as Meta", "Copy on select", "Programs may copy to the clipboard", "Shell integration"]
            } else {
                &["Scrollback per pane", "Scrollback memory for all panes", "Copy on select", "Programs may copy to the clipboard", "Shell integration"]
            }
        }
        SettingsPage::Agents | SettingsPage::Integrations => &["Command", "Resume", "Environment", "Status signals", "Browser tools", "Add preset", "Hook program"],
        SettingsPage::Accounts => &["Agents running", "Healthy accounts", "Rate-limited", "When an account hits its limit", "Switch", "Fall back to another provider", "Retry a limited account after", "Accounts, in failover order", "Add account", "Active sessions", "Recent events"],
        SettingsPage::Browser => &["Open web pages in TernaryCode", "When an agent opens a page", "Put pages to sleep after", "Pages in Grid tiles", "Logins & cookies", "Agents can use the browser", "Ask before agents open a site", "Sites agents may open without asking", "Agents may run JavaScript in pages", "Sites added for a project"],
        SettingsPage::Editor => &["Language features", "Start servers only when needed", "Completion while typing", "Format on save", "Found on this computer"],
        SettingsPage::Notifications => {
            if cfg!(target_os = "macos") {
                &["An agent needs you", "Something failed", "An agent finished its turn", "A long command finished", "Badge on the Dock icon"]
            } else {
                &["An agent needs you", "Something failed", "An agent finished its turn", "A long command finished"]
            }
        }
        SettingsPage::Phone => &["Companion server", "Remote access", "Port", "Tunnel address", "Scan with your phone's camera", "Paired phones"],
        SettingsPage::Updates => &["Check for updates automatically", "Install update", "Releases"],
        SettingsPage::About => &["MIT license", "Releases"],
    }
}

// ---------------------------------------------------------------------------------------------
// Rows and groups (also used by Accounts and Phone)
// ---------------------------------------------------------------------------------------------

/// Whether a row's label or note matches the search.
pub(super) fn hit(search: &str, label: &str, note: &str) -> bool {
    let q = search.trim().to_lowercase();
    q.is_empty() || label.to_lowercase().contains(&q) || note.to_lowercase().contains(&q)
}

/// Whether any of `labels` matches.
pub(super) fn any_hit(search: &str, labels: &[(&str, &str)]) -> bool {
    labels.iter().any(|(l, n)| hit(search, l, n))
}

/// `text` laid out in `font`, wrapped at `wrap` (one line when None), each line `line_h` high
/// (the type style's line height: Body 18, Small 16, Mono Small 14).
pub(super) fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, wrap: Option<f32>, line_h: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::default();
    let mut f = egui::TextFormat::simple(font, color);
    f.line_height = Some(line_h);
    job.append(text, 0.0, f);
    job.wrap.max_width = wrap.unwrap_or(f32::INFINITY);
    ui.painter().layout_job(job)
}

/// A label in a type style, as a widget (`wrap`: it wraps in the room left).
pub(super) fn text(ui: &mut Ui, text: &str, font: FontId, color: Color32, line_h: f32, wrap: bool) -> egui::Response {
    let g = galley(ui, text, font, color, wrap.then(|| ui.available_width()), line_h);
    let (r, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
    ui.painter().galley(r.min, g, color);
    resp
}

/// The room above a group: 24 unless it is the first thing on the page.
pub(super) fn gap(ui: &mut Ui) {
    if ui.min_rect().height() > 0.5 {
        ui.add_space(GAP);
    }
}

/// A group's title (Small Strong, secondary) with an optional note at its right, 8 above its card.
pub(super) fn group_head(ui: &mut Ui, title: &str, note: Option<&str>) {
    let p = style::pal(ui);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let g = galley(ui, title, style::medium(style::SMALL), p.text_muted, None, 16.0);
    ui.painter().galley(r.left_top(), g, p.text_muted);
    if let Some(n) = note {
        let g = galley(ui, n, FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
        ui.painter().galley(pos2(r.right() - g.size().x, r.top()), g, p.text_faint);
    }
    ui.add_space(8.0);
}

/// A group: its title (when it has one) over a card of rows, shown when the search matches one
/// of `labels` (or the title: then all its rows show). Returns whether it was drawn.
pub(super) fn section(ui: &mut Ui, search: &str, title: Option<&str>, note: Option<&str>, labels: &[(&str, &str)], add: impl FnOnce(&mut Card)) -> bool {
    let title_hit = title.is_some_and(|t| !search.trim().is_empty() && hit(search, t, ""));
    if !title_hit && !any_hit(search, labels) {
        return false;
    }
    gap(ui);
    if let Some(title) = title {
        group_head(ui, title, note);
    }
    card(ui, if title_hit { "" } else { search }, add);
    true
}

/// A card of rows (raised, a hairline, radius 10), the rows edge to edge with hairlines between.
pub(super) fn card(ui: &mut Ui, search: &str, add: impl FnOnce(&mut Card)) -> egui::Response {
    let p = style::pal(ui);
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS_CARD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut c = Card { ui, search: search.to_string(), first: true, row_idx: 0 };
            add(&mut c);
        })
        .response
}

/// A raised card of its own content (`pad` inside): the usage summary, the QR code, an update.
pub(super) fn raised(ui: &mut Ui, pad: i8, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let p = style::pal(ui);
    gap(ui);
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS_CARD))
        .inner_margin(egui::Margin::same(pad))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response
}

/// Rows in a card: each the setting's name and a line about it at the left, its control at the
/// right (padding 12 / 16, 14 between), a hairline between rows.
pub(super) struct Card<'a> {
    pub ui: &'a mut Ui,
    search: String,
    first: bool,
    row_idx: usize,
}

impl Card<'_> {
    pub fn row(&mut self, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
        self.row_full(None, label, note, None, control);
    }

    /// A row with a status icon before its name (Notifications).
    pub fn row_icon(&mut self, icon: &'static str, color: Color32, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
        self.row_full(Some((icon, color)), label, note, None, control);
    }

    /// A row whose note is in `color` (an error under a field).
    pub fn row_note(&mut self, label: &str, note: &str, color: Color32, control: impl FnOnce(&mut Ui)) {
        self.row_full(None, label, note, Some(color), control);
    }

    /// Content of its own, the card's width, with `pad` around it; shown when one of `labels`
    /// matches (always with none).
    pub fn block(&mut self, labels: &[(&str, &str)], pad: egui::Margin, add: impl FnOnce(&mut Ui)) {
        if !labels.is_empty() && !any_hit(&self.search, labels) {
            return;
        }
        self.divider();
        egui::Frame::NONE.inner_margin(pad).show(self.ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    }

    /// The hairline between rows (none before the first).
    pub fn divider(&mut self) {
        if !self.first {
            let p = style::pal(self.ui);
            let y = self.ui.cursor().top();
            let x = self.ui.max_rect().x_range();
            self.ui.painter().hline(x, y + 0.5, Stroke::new(1.0, p.border));
        }
        self.first = false;
    }

    /// The search this card filters by.
    pub fn search(&self) -> &str {
        &self.search
    }

    /// The row: the text at the left (as wide as the control leaves; the control's size is the
    /// one it had last frame), the control at the right, both centred in the row.
    fn row_full(&mut self, icon: Option<(&'static str, Color32)>, label: &str, note: &str, note_color: Option<Color32>, control: impl FnOnce(&mut Ui)) {
        let row_idx = self.row_idx;
        self.row_idx += 1;
        if !hit(&self.search, label, note) {
            return;
        }
        self.divider();
        let ui = &mut *self.ui;
        let p = style::pal(ui);
        let w = ui.available_width();
        let key = ui.id().with(("settings-row", row_idx, label));
        let ctrl: Vec2 = ui.data(|d| d.get_temp(key)).unwrap_or(Vec2::ZERO);
        let icon_w = if icon.is_some() { 16.0 + 14.0 } else { 0.0 };
        let ctrl_room = if ctrl.x > 0.0 { ctrl.x + 14.0 } else { 0.0 };
        let text_w = (w - 32.0 - icon_w - ctrl_room).max(80.0);
        let lg = galley(ui, label, FontId::proportional(style::BODY), p.text, Some(text_w), 18.0);
        let note_c = note_color.unwrap_or(p.text_faint);
        let ng = (!note.is_empty()).then(|| galley(ui, note, FontId::proportional(style::SMALL), note_c, Some(text_w), 16.0));
        let label_h = lg.size().y;
        let text_h = label_h + ng.as_ref().map_or(0.0, |g| 3.0 + g.size().y);
        let inner_h = text_h.max(ctrl.y);
        let (rect, _) = ui.allocate_exact_size(vec2(w, inner_h + 24.0), Sense::hover());
        let mut x = rect.left() + 16.0;
        if let Some((i, c)) = icon {
            ui.painter().text(pos2(x + 8.0, rect.center().y), Align2::CENTER_CENTER, i, style::icon_fill(16.0), c);
            x += icon_w;
        }
        let ty = rect.top() + 12.0 + (inner_h - text_h) / 2.0;
        ui.painter().galley(pos2(x, ty), lg, p.text);
        if let Some(g) = ng {
            ui.painter().galley(pos2(x, ty + label_h + 3.0), g, note_c);
        }
        let cr = Rect::from_min_max(pos2(rect.right() - 16.0 - ctrl.x, rect.top() + 12.0), pos2(rect.right() - 16.0, rect.bottom() - 12.0));
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("settings-ctrl", row_idx, label)).max_rect(cr).layout(egui::Layout::left_to_right(egui::Align::Center)));
        child.spacing_mut().item_spacing = vec2(10.0, 6.0);
        control(&mut child);
        let used = child.min_rect().size();
        if (used - ctrl).abs().max_elem() > 0.5 {
            ui.data_mut(|d| d.insert_temp(key, used));
            ui.ctx().request_repaint();
        }
    }
}

/// A text field in the data font (30 high, the inset fill, radius 7), `width` wide (the room
/// left when 0); `multi`: as many lines as it has.
pub(super) fn mono_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32, multi: bool) -> egui::Response {
    let p = style::pal(ui);
    let w = if width > 0.0 { width } else { ui.available_width() };
    let lines = if multi { text.lines().count().max(1) + usize::from(text.ends_with('\n')) } else { 1 };
    let h = 30.0 + (lines.saturating_sub(1)) as f32 * 16.0;
    let (rect, outer) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if outer.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.memory(|m| m.has_focus(id));
    if focused {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(10), Stroke::new(3.0, style::soft(p.accent, 0.25)), StrokeKind::Inside);
    }
    let line = if focused { p.accent } else { p.border_strong };
    ui.painter().rect(rect, CornerRadius::same(7), p.inset, Stroke::new(1.0, line), StrokeKind::Inside);
    let inner = Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + 7.0), pos2(rect.right() - 10.0, rect.bottom() - 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Min)));
    let edit = if multi { egui::TextEdit::multiline(text).desired_rows(1) } else { egui::TextEdit::singleline(text) };
    child.add_sized(
        inner.size(),
        edit.id(id)
            .char_limit(16_384)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(style::mono(12.0))
            .hint_text(RichText::new(hint).font(style::mono(12.0)).color(p.text_faint)),
    )
}

/// What a page says when nothing on it matches the search.
fn no_match(ui: &mut Ui, search: &str) {
    let p = style::pal(ui);
    let msg = tf("Nothing on this page matches “{}”.", &[search.trim()]);
    text(ui, &msg, FontId::proportional(style::SMALL), p.text_faint, 16.0, true);
}

/// A site in the list agents may open without asking, with × to take it out. Returns whether ×
/// was clicked.
fn site_chip(ui: &mut Ui, site: &str) -> bool {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(site.to_string(), style::mono(11.0), p.text);
    let (rect, _) = ui.allocate_exact_size(vec2(8.0 + g.size().x + 6.0 + 10.0 + 8.0, 24.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(6), p.subtle);
    ui.painter().galley(pos2(rect.left() + 8.0, rect.center().y - g.size().y / 2.0), g, p.text);
    let x = Rect::from_center_size(pos2(rect.right() - 8.0 - 5.0, rect.center().y), vec2(16.0, 16.0));
    let r = ui.interact(x, ui.id().with(("site-x", site)), Sense::click()).on_hover_text(t("Remove"));
    let c = if r.hovered() { p.text } else { p.text_faint };
    ui.painter().text(x.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(10.0), c);
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The title and the line under it of each page.
fn page_head_text(page: SettingsPage) -> (&'static str, String) {
    match page {
        SettingsPage::General => (t("General"), t("The app itself.").into()),
        SettingsPage::Appearance => (t("Appearance"), t("How TernaryCode and its terminals look.").into()),
        SettingsPage::Terminal => (t("Terminal"), t("How panes behave.").into()),
        SettingsPage::Agents | SettingsPage::Integrations => {
            (t("Agents"), tf("Presets you start with {}, and how each agent tells TernaryCode it needs you.", &[kit::keys(&Action::PickPreset).unwrap_or("")]))
        }
        SettingsPage::Accounts => (t("Accounts & usage"), t("Several sign-ins per agent, their limits, and what happens when one runs out.").into()),
        SettingsPage::Browser => (t("Browser"), t("Web pages beside your terminals, for you and your agents.").into()),
        SettingsPage::Editor => (t("Editor"), t("For reading code, and small edits.").into()),
        SettingsPage::Notifications => (t("Notifications"), t("Only when you are not already looking. The keyboard never moves by itself.").into()),
        SettingsPage::Phone => (t("Phone"), t("Watch agents, answer them and switch accounts from your phone on the same Wi-Fi.").into()),
        SettingsPage::Updates => (t("Updates"), t("TernaryCode updates itself from its GitHub releases.").into()),
        SettingsPage::About => (t("About"), t("Every project and its agents, in one window.").into()),
    }
}

// ---------------------------------------------------------------------------------------------
// The modal
// ---------------------------------------------------------------------------------------------

impl App {
    /// Opens Settings at `page` (the entry for Accounts, Phone, Agents, About…): what was
    /// floating closes first.
    pub fn open_settings_at(&mut self, page: SettingsPage) {
        self.monitor_popover = false;
        self.attention_open = false;
        self.palette = None;
        if page == SettingsPage::Accounts {
            // Fresh usage numbers for the page.
            self.usage_scan_at = None;
        }
        self.dialog = Some(crate::app::Dialog::Settings(page));
    }

    /// Whether the search matches something on `page`, its rows or what the user added there
    /// (accounts, presets, phones, sites).
    fn page_matches(&self, page: SettingsPage, search: &str) -> bool {
        if search.trim().is_empty() {
            return false;
        }
        let (title, _) = page_head_text(page);
        if hit(search, title, "") || page_rows(page).iter().any(|r| hit(search, t(r), "")) {
            return true;
        }
        match page {
            SettingsPage::Accounts => self.effective_profiles().iter().any(|p| hit(search, &p.id, &p.name) || p.account_hint.as_deref().is_some_and(|h| hit(search, h, ""))),
            SettingsPage::Agents => self.settings.presets().iter().any(|p| hit(search, &p.name, &p.command)),
            SettingsPage::Phone => self.settings.companion.devices.iter().any(|d| hit(search, &d.name, &d.ip)),
            SettingsPage::Browser => self.settings.browser_agent_sites.iter().any(|s| hit(search, s, "")),
            _ => false,
        }
    }

    /// Draws the settings dialog; returns false when it should close.
    pub fn settings_ui(&mut self, ui: &mut Ui, page: &mut SettingsPage) -> bool {
        if *page == SettingsPage::Integrations {
            *page = SettingsPage::Agents;
        }
        let screen = ui.ctx().content_rect();
        let w = W.min((screen.width() - 48.0).max(360.0));
        let h = H.min((screen.height() - 64.0).max(360.0));
        let nav_w = if w < 760.0 { 184.0 } else { NAV_W };
        ui.set_width(w);
        ui.set_height(h);
        ui.set_max_size(vec2(w, h));
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let mut keep = true;
        let p = style::pal(ui);
        let full = Rect::from_min_size(ui.max_rect().min, vec2(w, h));

        // ----- the nav: "Settings", the search, the pages, the version ----------------------
        let nav = Rect::from_min_max(full.min, pos2(full.left() + nav_w, full.bottom()));
        let r = style::RADIUS_DIALOG;
        ui.painter().rect_filled(nav, CornerRadius { nw: r, ne: 0, sw: r, se: 0 }, p.hover);
        ui.painter().vline(nav.right() + 0.5, nav.y_range(), Stroke::new(1.0, p.border));
        let inner = Rect::from_min_max(pos2(nav.left() + 12.0, nav.top() + 18.0), pos2(nav.right() - 12.0, nav.bottom() - 14.0));
        let g = galley(ui, t("Settings"), style::semibold(style::TITLE), p.text, None, 24.0);
        ui.painter().galley(inner.left_top(), g, p.text);
        let search_rect = Rect::from_min_size(pos2(inner.left(), inner.top() + 24.0 + 14.0), vec2(inner.width(), 30.0));
        let sid = egui::Id::new("settings-search");
        if ui.memory(|m| m.has_focus(sid)) {
            ui.memory_mut(|m| m.set_focus_lock_filter(sid, egui::EventFilter { escape: true, ..Default::default() }));
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.settings_search.clear();
                ui.memory_mut(|m| m.surrender_focus(sid));
            }
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            ui.memory_mut(|m| m.request_focus(sid));
        }
        let mut search = self.settings_search.clone();
        nav_search(ui, search_rect, sid, &mut search);
        self.settings_search = search.clone();

        // The version at the bottom: the logo, "TernaryCode", the version (and the one that is out).
        let ver_h = 16.0 + 1.0 + 14.0;
        let ver = Rect::from_min_max(pos2(inner.left(), inner.bottom() - ver_h), inner.max);
        let logo = Rect::from_min_size(pos2(ver.left(), ver.center().y - 12.0), vec2(24.0, 24.0));
        paint_logo(ui.painter(), logo, &p);
        let vx = logo.right() + 10.0;
        let g = galley(ui, "TernaryCode", style::medium(style::SMALL), p.text, None, 16.0);
        ui.painter().galley(pos2(vx, ver.top()), g, p.text);
        let (version, vc) = match self.updater.available_version() {
            Some(v) => (format!("{} · {}", crate::update::CURRENT, tf("{} is out", &[&v])), p.accent_text),
            None => (crate::update::CURRENT.to_string(), p.text_faint),
        };
        let mut job = egui::text::LayoutJob::simple_singleline(version, style::mono(style::DATA), vc);
        job.wrap = egui::text::TextWrapping::truncate_at_width((ver.right() - vx).max(20.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(vx, ver.top() + 17.0), g, vc);

        let list = Rect::from_min_max(pos2(inner.left(), search_rect.bottom() + 14.0), pos2(inner.right(), (ver.top() - 14.0).max(search_rect.bottom() + 14.0)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt("settings-nav-scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for entry in NAV {
                        let Some((pg, icon, name)) = entry else {
                            let (sr, _) = ui.allocate_exact_size(vec2(list.width(), 13.0), Sense::hover());
                            ui.painter().hline((sr.left() + 8.0)..=(sr.right() - 8.0), sr.center().y, Stroke::new(1.0, p.border));
                            continue;
                        };
                        let (item, resp) = ui.allocate_exact_size(vec2(list.width(), 30.0), Sense::click());
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("settings:{name}"), item);
                        let on = *page == pg;
                        if on {
                            ui.painter().rect(item, CornerRadius::same(style::RADIUS_ROW), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(item, CornerRadius::same(style::RADIUS_ROW), p.hover);
                        }
                        if resp.hovered() && !on {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        let (ic, tc) = if on || resp.hovered() { (p.text, p.text) } else { (p.text_muted, p.text_muted) };
                        ui.painter().text(pos2(item.left() + 10.0 + 8.0, item.center().y), Align2::CENTER_CENTER, icon, style::icon_font(16.0), ic);
                        let font = if on { style::medium(style::BODY) } else { FontId::proportional(style::BODY) };
                        ui.painter().text(pos2(item.left() + 10.0 + 16.0 + 10.0, item.center().y), Align2::LEFT_CENTER, t(name), font, tc);
                        // At the right: what a page has for the user (agents without hooks, an
                        // update), and a dot when the search matches something there.
                        let mut right = item.right() - 10.0;
                        if pg == SettingsPage::Agents {
                            let n = self.agents_without_hooks();
                            if n > 0 {
                                let g = ui.painter().layout_no_wrap(n.to_string(), style::mono_semibold(style::DATA), p.warn);
                                let bw = (g.size().x + 8.0).max(16.0);
                                let b = Rect::from_min_size(pos2(right - bw, item.center().y - 8.0), vec2(bw, 16.0));
                                ui.painter().rect_filled(b, CornerRadius::same(8), style::tint(p.warn, ui.visuals().dark_mode));
                                ui.painter().galley(b.center() - g.size() / 2.0, g, p.warn);
                                right = b.left() - 6.0;
                            }
                        }
                        if pg == SettingsPage::Updates && self.updater.available_version().is_some() {
                            ui.painter().circle_filled(pos2(right - 3.5, item.center().y), 3.5, p.accent);
                            right -= 7.0 + 6.0;
                        }
                        if self.page_matches(pg, &search) {
                            ui.painter().circle_filled(pos2(right - 3.0, item.center().y), 3.0, p.accent_text);
                        }
                        if resp.clicked() && !on {
                            *page = pg;
                            if pg == SettingsPage::Accounts {
                                self.usage_scan_at = None;
                            }
                        }
                    }
                });
        });

        // ----- the page's head: its title and line, "Saved as you change them", × -----------
        let body = Rect::from_min_max(pos2(nav.right() + 1.0, full.top()), full.max);
        let hx = body.left() + PAD_X;
        let close = Rect::from_min_size(pos2(body.right() - PAD_X - 24.0, body.top() + PAD_TOP + 3.0), vec2(24.0, 24.0));
        if ui.scope_builder(egui::UiBuilder::new().max_rect(close), |ui| kit::icon_button_ex(ui, icons::X, t("Close"), false, 24.0)).inner.clicked() {
            keep = false;
        }
        let mut saved_x = close.left() - 12.0;
        if body.width() >= 480.0 {
            let g = galley(ui, t("Saved as you change them"), FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
            let cy = close.center().y;
            saved_x -= g.size().x;
            ui.painter().galley(pos2(saved_x, cy - g.size().y / 2.0), g, p.text_faint);
            ui.painter().text(pos2(saved_x - 5.0 - 6.0, cy), Align2::CENTER_CENTER, icons::CHECK, style::icon_font(12.0), p.ok);
            saved_x -= 5.0 + 12.0 + 12.0;
        }
        let (title, lead) = page_head_text(*page);
        let mut job = egui::text::LayoutJob::default();
        let mut f = egui::TextFormat::simple(style::semibold(style::DISPLAY), p.text);
        f.line_height = Some(30.0);
        f.extra_letter_spacing = -0.3;
        job.append(title, 0.0, f);
        job.wrap = egui::text::TextWrapping::truncate_at_width((saved_x - hx).max(60.0));
        let tg = ui.painter().layout_job(job);
        let title_h = tg.size().y;
        ui.painter().galley(pos2(hx, body.top() + PAD_TOP), tg, p.text);
        let lg = galley(ui, &lead, FontId::proportional(style::BODY), p.text_muted, Some((saved_x - hx).max(120.0)), 18.0);
        let lead_h = lg.size().y;
        ui.painter().galley(pos2(hx, body.top() + PAD_TOP + title_h + 4.0), lg, p.text_muted);
        let head_h = title_h + 4.0 + lead_h;

        // ----- the page -----------------------------------------------------------------------
        let main = Rect::from_min_max(pos2(body.left(), body.top() + PAD_TOP + head_h + GAP), body.max);
        #[cfg(debug_assertions)]
        crate::devtest::record("settings-body", main);
        let before = self.settings.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            ui.set_clip_rect(main.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(("settings-page", *page as u8))
                .max_height(main.height())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let m = egui::Margin { left: PAD_X as i8, right: PAD_X as i8, top: 2, bottom: PAD_BOTTOM as i8 };
                    egui::Frame::NONE.inner_margin(m).show(ui, |ui| {
                        ui.set_width((main.width() - PAD_X * 2.0).max(160.0));
                        ui.spacing_mut().item_spacing.y = 0.0;
                        match *page {
                            SettingsPage::General => self.general_page(ui, &search),
                            SettingsPage::Appearance => self.appearance_page(ui, &search),
                            SettingsPage::Terminal => self.terminal_page(ui, &search),
                            SettingsPage::Agents | SettingsPage::Integrations => self.agents_page(ui, &search),
                            SettingsPage::Accounts => self.accounts_page(ui, &search),
                            SettingsPage::Browser => self.browser_page(ui, &search),
                            SettingsPage::Editor => self.editor_page(ui, &search),
                            SettingsPage::Notifications => self.notifications_page(ui, &search),
                            SettingsPage::Phone => self.phone_page(ui, &search),
                            SettingsPage::Updates => self.updates_page(ui, &search),
                            SettingsPage::About => self.about_page(ui, &search),
                        }
                        if ui.min_rect().height() < 0.5 && !search.trim().is_empty() {
                            no_match(ui, &search);
                        }
                    });
                });
        });

        ui.allocate_rect(full, Sense::hover());
        if self.settings != before {
            if self.settings.theme != before.theme {
                style::apply(ui.ctx(), self.settings.theme);
            }
            if self.settings.font_name != before.font_name {
                crate::app::apply_fonts(ui.ctx(), &self.settings);
            }
            if self.settings.lsp != before.lsp || self.settings.lsp_on_demand != before.lsp_on_demand {
                // On demand, open files wait for an edit or a language feature to start theirs.
                if self.settings.lsp && !self.settings.lsp_on_demand {
                    let ctx = ui.ctx().clone();
                    let open: Vec<std::path::PathBuf> = self.editors.values().flat_map(|e| e.files.iter().map(|f| f.path.clone())).collect();
                    for path in open {
                        self.lsp_open(&ctx, &path);
                    }
                } else if !self.settings.lsp {
                    self.lsp_stop_all();
                }
            }
            if (self.settings.sidebar_width - before.sidebar_width).abs() > 0.5 {
                self.sidebar_width_sync = true;
            }
            self.settings_changed();
        }
        keep
    }

    /// Agents whose status cannot come from hooks as things are set (they show "Active").
    pub fn agents_without_hooks(&self) -> usize {
        [AgentKind::OpenCode]
            .into_iter()
            .filter(|a| self.installed(*a).is_none() && self.settings.integration(*a) != Integration::Off)
            .count()
    }

    // -----------------------------------------------------------------------------------------
    // General
    // -----------------------------------------------------------------------------------------

    fn general_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        let cli_note = tf("Type {} in any terminal to open that folder here.", &["ternarycode ."]);
        let dir = crate::paths::contract_tilde(crate::paths::data_dir());
        let labels = [(t("Command line"), cli_note.as_str()), (t("Data folder"), dir.as_str())];
        section(ui, search, None, None, &labels, |c| {
            c.row(t("Command line"), &cli_note, |ui| match crate::cli::installed() {
                Some(path) => {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    text(ui, &format!("→ {}", crate::paths::contract_tilde(&path)), style::mono(style::DATA), p.text_faint, 14.0, false);
                    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::CHECK_CIRCLE, style::icon_font(14.0), p.text_muted);
                    resp.on_hover_text(t("Type ternarycode <folder or project name> in any terminal to open it here."));
                }
                None => {
                    #[cfg(target_os = "macos")]
                    if kit::button_ex(ui, kit::Kind::Secondary, None, t("Install ternarycode Command…"), true).clicked() {
                        match crate::cli::install_with_admin() {
                            Ok(path) => self.toast_ok(tf("Installed: {}", &[&path.display().to_string()])),
                            Err(e) => self.toast_err(e),
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    text(ui, t("Not on PATH (works in TernaryCode's own panes)"), FontId::proportional(style::SMALL), p.text_muted, 16.0, false);
                }
            });
            c.row(t("Data folder"), &dir, |ui| {
                let label = if cfg!(target_os = "macos") {
                    t("Show in Finder")
                } else if cfg!(windows) {
                    t("Show in Explorer")
                } else {
                    t("Show in Files")
                };
                if kit::button_ex(ui, kit::Kind::Secondary, None, label, true).clicked() {
                    crate::platform::reveal(crate::paths::data_dir());
                }
            });
        });

        // Worktrees: where Run Agent → New worktree puts them, what their branches are called.
        let beside = tf("Beside the repository: {}", &["‹repo›.worktrees/‹name›"]);
        let prefix = self.settings.worktrees.prefix.clone();
        let examples = format!("{prefix}generics, {prefix}lexer-fix…");
        let place_note = match self.settings.worktrees.place {
            tc_core::model::WorktreePlace::Beside => beside.clone(),
            tc_core::model::WorktreePlace::DataFolder => tf("In TernaryCode's data folder: {}", &[&crate::paths::contract_tilde(&crate::paths::data_dir().join("worktrees"))]),
        };
        let labels = [
            (t("Where new worktrees go"), place_note.as_str()),
            (t("Branch names start with"), examples.as_str()),
            (t("When a branch is merged"), t("Offer to remove its worktree.")),
        ];
        section(ui, search, Some(t("Worktrees")), None, &labels, |c| {
            c.row(t("Where new worktrees go"), &place_note, |ui| {
                let options = [t("Beside the repository"), t("In TernaryCode's data folder")];
                let current = match self.settings.worktrees.place {
                    tc_core::model::WorktreePlace::Beside => 0,
                    tc_core::model::WorktreePlace::DataFolder => 1,
                };
                if let Some(i) = kit::select(ui, "worktree-place", options[current], &options, 220.0) {
                    self.settings.worktrees.place = if i == 0 { tc_core::model::WorktreePlace::Beside } else { tc_core::model::WorktreePlace::DataFolder };
                }
            });
            c.row(t("Branch names start with"), &examples, |ui| {
                let mut text = self.settings.worktrees.prefix.clone();
                let r = mono_field(ui, egui::Id::new("worktree-prefix"), &mut text, "tc/", 90.0, false);
                // Only what a branch name can hold (empty: no prefix).
                if r.changed() && (text.is_empty() || tc_git::worktree::valid_branch(&format!("{text}x"))) {
                    self.settings.worktrees.prefix = text;
                }
            });
            c.row(t("When a branch is merged"), t("Offer to remove its worktree."), |ui| {
                kit::switch(ui, &mut self.settings.worktrees.offer_remove);
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Appearance
    // -----------------------------------------------------------------------------------------

    fn appearance_page(&mut self, ui: &mut Ui, search: &str) {
        let labels = [(t("Theme"), "")];
        section(ui, search, Some(t("Theme")), None, &labels, |c| {
            c.block(&[(t("Theme"), "")], egui::Margin::same(16), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 20.0;
                    for (mode, name) in [(ThemeMode::System, t("System")), (ThemeMode::Light, t("Light")), (ThemeMode::Dark, t("Dark"))] {
                        let r = theme_card(ui, mode, name, self.settings.theme == mode);
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("theme:{mode:?}"), r.rect);
                        if r.clicked() {
                            self.settings.theme = mode;
                        }
                    }
                });
            });
        });

        let labels = [(t("Dark theme"), ""), (t("Light theme"), "")];
        section(ui, search, Some(t("Terminal colours")), None, &labels, |c| {
            for dark in [true, false] {
                let label = if dark { t("Dark theme") } else { t("Light theme") };
                c.block(&[(label, "")], egui::Margin::same(16), |ui| self.scheme_row(ui, label, dark));
            }
        });

        let labels = [(t("Font"), ""), (t("Size"), ""), (t("Cursor"), t("New panes take it.")), (t("Blinking cursor"), "")];
        section(ui, search, Some(t("Terminal text")), None, &labels, |c| {
            let search = c.search().to_string();
            // The settings at the left, a live preview at the right (in the terminal's font and
            // colours).
            c.block(&[], egui::Margin::ZERO, |ui| {
                let full = ui.available_width();
                let preview_w = if full > 600.0 { 300.0 + 32.0 } else { 0.0 };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let left = ui
                        .vertical(|ui| {
                            ui.set_width(full - preview_w);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let mut c = Card { ui, search: search.clone(), first: true, row_idx: 0 };
                            c.row(t("Font"), "", |ui| {
                                let fonts: Vec<&'static str> = tc_term::fonts::PRESETS.iter().filter(|f| tc_term::fonts::is_available(f)).map(|f| f.name).collect();
                                let current = tc_term::fonts::effective_choice(&tc_term::prefs::FontChoice { name: self.settings.font_name.clone(), path: self.settings.font_path.clone(), index: 0 }).name;
                                if let Some(i) = kit::select(ui, "font", &current, &fonts, 180.0) {
                                    self.settings.font_name = fonts[i].to_string();
                                    self.settings.font_path = None;
                                }
                            });
                            c.row(t("Size"), "", |ui| {
                                let size = self.settings.font_size;
                                match kit::number_stepper(ui, &format!("{} pt", size_text(size)), 70.0) {
                                    -1 => self.settings.font_size = (size - 0.5).max(9.0),
                                    1 => self.settings.font_size = (size + 0.5).min(24.0),
                                    _ => {}
                                }
                            });
                            c.row(t("Cursor"), t("New panes take it."), |ui| {
                                let options = [kit::Seg::new(t("Block"), ""), kit::Seg::new(t("Beam"), ""), kit::Seg::new(t("Underline"), "")];
                                let on = self.settings.cursor_shape as usize;
                                if let Some(i) = kit::segmented(ui, &options, on) {
                                    self.settings.cursor_shape = [CursorShape::Block, CursorShape::Beam, CursorShape::Underline][i];
                                }
                            });
                            c.row(t("Blinking cursor"), "", |ui| {
                                kit::switch(ui, &mut self.settings.cursor_blink);
                            });
                        })
                        .response
                        .rect;
                    if preview_w > 0.0 {
                        let (r, _) = ui.allocate_exact_size(vec2(preview_w, left.height().max(150.0)), Sense::hover());
                        let r = Rect::from_min_size(r.min + vec2(16.0, 16.0), vec2(300.0, r.height() - 32.0));
                        let dark = ui.visuals().dark_mode;
                        terminal_preview(ui, r, self.scheme(dark), self.settings.font_size, self.settings.cursor_shape);
                    }
                });
            });
        });

        let labels = [
            (t("Text size"), t("Size of projects and controls in the sidebar.")),
            (t("Width"), t("Or drag the sidebar's right edge.")),
        ];
        section(ui, search, Some(t("Sidebar")), None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                let size = self.settings.sidebar_font_size;
                match kit::number_stepper(ui, &format!("{} pt", size_text(size)), 70.0) {
                    -1 => self.settings.sidebar_font_size = (size - 0.5).max(11.0),
                    1 => self.settings.sidebar_font_size = (size + 0.5).min(18.0),
                    _ => {}
                }
            });
            c.row(labels[1].0, labels[1].1, |ui| {
                let w = self.settings.sidebar_width.round();
                match kit::number_stepper(ui, &format!("{} px", w as i32), 70.0) {
                    -1 => self.settings.sidebar_width = (w - 16.0).max(200.0),
                    1 => self.settings.sidebar_width = (w + 16.0).min(420.0),
                    _ => {}
                }
            });
        });
    }

    /// One theme's terminal schemes: the row's name, four tiles and "+N More" (all of them when
    /// opened).
    fn scheme_row(&mut self, ui: &mut Ui, label: &str, dark: bool) {
        let p = style::pal(ui);
        let current = self.scheme(dark).name;
        // TernaryCode's own first, then Dracula, Nord and Gruvbox (as the design lists them),
        // then the rest.
        const FIRST: [&str; 4] = ["Dracula", "Nord", "Gruvbox Dark", "Tomorrow Night"];
        let mut schemes: Vec<&'static tc_term::scheme::Scheme> = tc_term::scheme::SCHEMES.iter().filter(|s| s.dark == dark).collect();
        schemes.sort_by_key(|s| if s.name.starts_with("TernaryCode") { 0 } else { FIRST.iter().position(|f| *f == s.name).map_or(10, |i| i + 1) });
        let open_id = egui::Id::new(("schemes-open", dark));
        let open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        let w = ui.available_width();
        let label_w = 120.0;
        let fit = (((w - label_w - 12.0 - 72.0) / (104.0 + 12.0)).floor() as usize).max(1);
        let mut shown: Vec<&'static tc_term::scheme::Scheme> = if open { schemes.clone() } else { schemes.iter().take(fit).copied().collect() };
        if !open
            && !shown.iter().any(|s| s.name == current)
            && let Some(s) = schemes.iter().find(|s| s.name == current)
        {
            shown.pop();
            shown.push(s);
        }
        let more = schemes.len().saturating_sub(fit);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (lr, _) = ui.allocate_exact_size(vec2(label_w, 18.0), Sense::hover());
            let g = galley(ui, label, FontId::proportional(style::BODY), p.text, None, 18.0);
            ui.painter().galley(lr.min, g, p.text);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                for s in &shown {
                    if scheme_tile(ui, s, s.name == current).clicked() {
                        if dark {
                            self.settings.scheme_dark = s.name.to_string();
                        } else {
                            self.settings.scheme_light = s.name.to_string();
                        }
                    }
                }
                if more > 0 && more_tile(ui, open_id, open, more).clicked() {
                    ui.data_mut(|d| d.insert_temp(open_id, !open));
                }
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Terminal
    // -----------------------------------------------------------------------------------------

    fn terminal_page(&mut self, ui: &mut Ui, search: &str) {
        let mut labels = vec![
            (t("Scrollback per pane"), t("Applies to new panes.")),
            (t("Scrollback memory for all panes"), t("Older lines are dropped first when it fills.")),
            (t("Copy on select"), ""),
            (t("Programs may copy to the clipboard"), t("vim, tmux and agents copy through OSC 52. They can never read it.")),
            (t("Shell integration"), t("Marks prompts and commands (zsh, bash, fish, PowerShell) so failures and long commands are noticed.")),
        ];
        if cfg!(target_os = "macos") {
            labels.push((t("Option key as Meta"), ""));
        }
        section(ui, search, None, None, &labels, |c| {
            c.row(t("Scrollback per pane"), t("Applies to new panes."), |ui| {
                let n = self.settings.scrollback;
                match kit::number_stepper(ui, &group_digits(n), 70.0) {
                    -1 => self.settings.scrollback = n.saturating_sub(1000).max(1000),
                    1 => self.settings.scrollback = (n + 1000).min(100_000),
                    _ => {}
                }
            });
            c.row(t("Scrollback memory for all panes"), t("Older lines are dropped first when it fills."), |ui| {
                let mb = self.settings.scrollback_budget_mb;
                match kit::number_stepper(ui, &format!("{mb} MB"), 70.0) {
                    -1 => self.settings.scrollback_budget_mb = mb.saturating_sub(64).max(64),
                    1 => self.settings.scrollback_budget_mb = (mb + 64).min(16_384),
                    _ => {}
                }
            });
            if cfg!(target_os = "macos") {
                c.row(t("Option key as Meta"), "", |ui| {
                    kit::switch(ui, &mut self.settings.option_as_meta);
                });
            }
            c.row(t("Copy on select"), "", |ui| {
                kit::switch(ui, &mut self.settings.copy_on_select);
            });
            c.row(t("Programs may copy to the clipboard"), t("vim, tmux and agents copy through OSC 52. They can never read it."), |ui| {
                kit::switch(ui, &mut self.settings.program_copy);
            });
            c.row(t("Shell integration"), t("Marks prompts and commands (zsh, bash, fish, PowerShell) so failures and long commands are noticed."), |ui| {
                kit::switch(ui, &mut self.settings.shell_integration);
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Browser
    // -----------------------------------------------------------------------------------------

    fn browser_page(&mut self, ui: &mut Ui, search: &str) {
        let labels = [
            (t("Open web pages in TernaryCode"), t("Server links, ⌘-clicked URLs and programs that open a browser. Hold ⌥ for your default browser.")),
            (t("When an agent opens a page"), t("Its page goes in the project's Browser panel. The keyboard never moves.")),
            (t("Put pages to sleep after"), t("Never while they play sound, use the camera, have a form being filled in, or an agent uses them.")),
            (t("Pages in Grid tiles"), t("Smaller, a page keeps the layout it has in Focus. Fitting the tile, a site may switch to its phone layout.")),
            (t("Logins & cookies"), t("One profile for every project: sign in once.")),
        ];
        section(ui, search, None, None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                kit::switch(ui, &mut self.settings.browser_here);
            });
            c.row(labels[1].0, labels[1].1, |ui| {
                let options = [t("Show the panel"), t("Light the Browser button")];
                let current = options[usize::from(!self.settings.browser_agent_beside)];
                if let Some(i) = kit::select(ui, "browser-agent-place", current, &options, 170.0) {
                    self.settings.browser_agent_beside = i == 0;
                }
            });
            c.row(labels[2].0, labels[2].1, |ui| {
                const STEPS: [u32; 7] = [1, 5, 10, 15, 30, 60, 120];
                let n = self.settings.browser_sleep_minutes;
                let at = STEPS.iter().position(|s| *s >= n).unwrap_or(STEPS.len() - 1);
                match kit::number_stepper(ui, &tf("{} min", &[&n.to_string()]), 70.0) {
                    -1 => self.settings.browser_sleep_minutes = STEPS[at.saturating_sub(1)],
                    1 => self.settings.browser_sleep_minutes = STEPS[(at + 1).min(STEPS.len() - 1)],
                    _ => {}
                }
            });
            c.row(labels[3].0, labels[3].1, |ui| {
                let options = [t("Keep their layout, smaller"), t("Fit the tile")];
                let current = options[usize::from(!self.settings.browser_grid_zoom)];
                if let Some(i) = kit::select(ui, "browser-grid-zoom", current, &options, 190.0) {
                    self.settings.browser_grid_zoom = i == 0;
                }
            });
            c.row(labels[4].0, labels[4].1, |ui| {
                if kit::button_ex(ui, kit::Kind::Secondary, None, t("Clear…"), true).clicked() {
                    self.dialog = Some(crate::app::Dialog::ConfirmClearBrowsing);
                }
            });
        });

        // Agents: whether they get the tools, whether they ask before a site (and which sites
        // they open without asking then), scripts.
        let labels = [
            (t("Agents can use the browser"), t("Only agents in TernaryCode panes, each in its own project's browser.")),
            (t("Ask before agents open a site"), t("Sites not on the list wait for Allow once, Always or Block.")),
            (t("Sites agents may open without asking"), ""),
            (t("Agents may run JavaScript in pages"), t("browser_evaluate can do anything the page's own scripts can.")),
        ];
        let mut tools_changed = false;
        let mut remove_site = None;
        let mut add_site = None;
        section(ui, search, Some(t("Agents")), None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                tools_changed = kit::switch(ui, &mut self.settings.browser_agents).changed();
            });
            c.row(labels[1].0, labels[1].1, |ui| {
                kit::switch(ui, &mut self.settings.browser_agent_ask);
            });
            if self.settings.browser_agent_ask {
                let sites_hit = self.settings.browser_agent_sites.iter().any(|s| hit(c.search(), s, ""));
                let wanted: &[(&str, &str)] = if sites_hit { &[] } else { &[(labels[2].0, "")] };
                c.block(wanted, egui::Margin::symmetric(16, 12), |ui| {
                    let p = style::pal(ui);
                    text(ui, labels[2].0, FontId::proportional(style::BODY), p.text, 18.0, false);
                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for site in &self.settings.browser_agent_sites {
                            if site_chip(ui, site) {
                                remove_site = Some(site.clone());
                            }
                        }
                        let id = egui::Id::new("browser-add-site");
                        let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
                        let r = kit::field(ui, id.with("field"), &mut text, None, t("+ Add site"), 150.0, false);
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if enter && !text.trim().is_empty() {
                            add_site = Some(text.trim().to_string());
                            text.clear();
                        }
                        ui.data_mut(|d| d.insert_temp(id, text));
                    });
                });
            }
            c.row(labels[3].0, labels[3].1, |ui| {
                kit::switch(ui, &mut self.settings.browser_agent_js);
            });
        });
        if tools_changed {
            // The next agent run and any new pane get (or no longer get) the browser tools.
            self.refresh_browser_pane_env();
        }
        if let Some(site) = remove_site {
            self.settings.browser_agent_sites.retain(|s| *s != site);
        }
        if let Some(site) = add_site {
            let site = crate::browser::host_of(&site).trim_start_matches("*.").to_string();
            if !site.is_empty() && !self.settings.browser_agent_sites.contains(&site) {
                self.settings.browser_agent_sites.push(site);
            }
        }

        // The sites one project's agents may open (added from the question in its browser).
        let sites: Vec<(tc_core::ProjectId, String, String)> =
            self.projects.iter().flat_map(|p| p.sites.iter().map(|s| (p.id, p.name.clone(), s.clone()))).collect();
        if sites.is_empty() || !self.settings.browser_agent_ask {
            return;
        }
        let labels: Vec<(&str, &str)> = sites.iter().map(|(_, project, site)| (site.as_str(), project.as_str())).collect();
        let mut remove = None;
        section(ui, search, Some(t("Sites added for a project")), None, &labels, |c| {
            for (id, project, site) in &sites {
                c.row(site, project, |ui| {
                    if kit::button_ex(ui, kit::Kind::Ghost, None, t("Remove"), true).clicked() {
                        remove = Some((*id, site.clone()));
                    }
                });
            }
        });
        if let Some((id, site)) = remove
            && let Some(p) = self.projects.iter_mut().find(|p| p.id == id)
        {
            p.sites.retain(|s| *s != site);
            self.mark_dirty();
        }
    }

    /// Whether `agent` gets the browser tools, and how that reads (Settings → Agents).
    fn browser_tools_status(&self, agent: AgentKind) -> (bool, &'static str) {
        let mode = self.settings.integration(agent);
        let installed = self.installed(agent);
        let has_server = |rec: &Option<InstallRecord>| {
            rec.as_ref().is_some_and(|r| {
                if r.agent == AgentKind::Antigravity {
                    return r
                        .path
                        .parent()
                        .and_then(|d| std::fs::read_to_string(d.join("mcp_config.json")).ok())
                        .is_some_and(|text| text.contains("ternarycode-hook") && text.contains("\"mcpServers\""));
                }
                std::fs::read_to_string(&r.path).ok().is_some_and(|text| {
                    text.contains("ternarycode-hook")
                        && (text.contains("\"mcpServers\"")
                            // Codex asks before each call of a server not approved.
                            || (text.contains("mcp_servers.ternarycode") && text.contains("default_tools_approval_mode")))
                })
            })
        };
        match (agent, mode) {
            (AgentKind::Claude | AgentKind::Codex, Integration::PerLaunch) => (true, t("Added to each run")),
            (AgentKind::Claude, Integration::Installed) => (true, t("Added to each run")),
            (AgentKind::Claude, _) => (false, t("Hooks off")),
            // In every pane (`OPENCODE_CONFIG_CONTENT`), whatever its hooks.
            (AgentKind::OpenCode, _) => (true, t("In every TernaryCode pane, typed by hand too")),
            (_, Integration::Installed) if has_server(&installed) => (true, t("Browser tools installed")),
            (_, Integration::Installed) => (false, t("Installed before the browser tools: install again")),
            _ => (false, t("Hooks not installed")),
        }
    }

    // -----------------------------------------------------------------------------------------
    // Editor
    // -----------------------------------------------------------------------------------------

    fn editor_page(&mut self, ui: &mut Ui, search: &str) {
        let labels = [
            (t("Language features"), t("Uses the language servers installed on this computer.")),
            (t("Start servers only when needed"), t("A file you only read costs nothing.")),
            (t("Completion while typing"), ""),
            (t("Format on save"), ""),
        ];
        section(ui, search, None, None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                kit::switch(ui, &mut self.settings.lsp);
            });
            let on = self.settings.lsp;
            c.row(labels[1].0, labels[1].1, |ui| {
                ui.add_enabled_ui(on, |ui| kit::switch(ui, &mut self.settings.lsp_on_demand));
            });
            c.row(labels[2].0, "", |ui| {
                ui.add_enabled_ui(on, |ui| kit::switch(ui, &mut self.settings.auto_complete));
            });
            c.row(labels[3].0, "", |ui| {
                ui.add_enabled_ui(on, |ui| kit::switch(ui, &mut self.settings.format_on_save));
            });
        });

        let path = self.lsp.path_env.clone().or_else(|| std::env::var_os("PATH")).unwrap_or_default();
        let servers: Vec<(&'static tc_lsp::servers::Spec, bool, Option<bool>)> = tc_lsp::servers::SERVERS
            .iter()
            .map(|spec| {
                let found = tc_lsp::servers::resolve(spec.program, &path).is_some();
                let running = self.lsp.servers.values().find(|s| s.spec.name == spec.name).map(|s| s.state == crate::lsp::ServerState::Ready);
                (spec, found, running)
            })
            .collect();
        let labels: Vec<(&str, &str)> = servers.iter().map(|(s, _, _)| (s.name, "")).collect();
        let mut copied = false;
        section(ui, search, Some(t("Found on this computer")), None, &labels, |c| {
            for (spec, found, running) in &servers {
                c.block(&[(spec.name, "")], egui::Margin::ZERO, |ui| {
                    let p = style::pal(ui);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0 + 20.0), Sense::hover());
                    let (dot, note) = match (found, running) {
                        (_, Some(true)) => (Some(p.ok), t("running")),
                        (true, _) => (None, t("ready")),
                        (false, _) => (Some(p.warn), t("not installed")),
                    };
                    let cy = r.center().y;
                    let dc = pos2(r.left() + 16.0 + 4.0, cy);
                    match dot {
                        Some(c) => {
                            ui.painter().circle_filled(dc, 4.0, c);
                        }
                        None => {
                            ui.painter().circle_stroke(dc, 3.5, Stroke::new(1.0, p.border_heavy));
                        }
                    }
                    let ng = galley(ui, spec.name, FontId::proportional(style::BODY), p.text, None, 18.0);
                    let nx = r.left() + 16.0 + 8.0 + 10.0;
                    let nw = ng.size().x;
                    ui.painter().galley(pos2(nx, cy - 9.0), ng, p.text);
                    let sg = galley(ui, note, FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
                    ui.painter().galley(pos2(nx + nw + 10.0, cy - 8.0), sg, p.text_faint);
                    if !found {
                        let g = galley(ui, t("Copy install command"), style::medium(style::SMALL), p.accent_text, None, 16.0);
                        let lr = Rect::from_min_size(pos2(r.right() - 16.0 - g.size().x, cy - 8.0), g.size());
                        ui.painter().galley(lr.min, g, p.accent_text);
                        let resp = ui.interact(lr, egui::Id::new(("copy-install", spec.name)), Sense::click()).on_hover_text(spec.install);
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            ui.ctx().copy_text(spec.install.to_string());
                            copied = true;
                        }
                    }
                });
            }
        });
        if copied {
            self.toast_ok(t("Install command copied.").into());
        }
    }

    // -----------------------------------------------------------------------------------------
    // Notifications
    // -----------------------------------------------------------------------------------------

    fn notifications_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        let mut labels = vec![
            (t("An agent needs you"), t("A permission request, a question or a plan.")),
            (t("Something failed"), t("An agent, a command that ran long, or a shell.")),
            (t("An agent finished its turn"), ""),
            (t("A long command finished"), t("Long means at least this long.")),
        ];
        if cfg!(target_os = "macos") {
            labels.push((t("Badge on the Dock icon"), t("Counts what waits for you.")));
        }
        section(ui, search, None, None, &labels, |c| {
            let n = &mut self.settings.notifications;
            c.row_icon(icons::CHAT_CIRCLE_DOTS, p.warn, labels[0].0, labels[0].1, |ui| {
                kit::switch(ui, &mut n.needs_input);
            });
            c.row_icon(icons::X_CIRCLE, p.err, labels[1].0, labels[1].1, |ui| {
                kit::switch(ui, &mut n.failed);
            });
            c.row_icon(icons::CHECK_CIRCLE, p.ok, labels[2].0, "", |ui| {
                kit::switch(ui, &mut n.done);
            });
            let secs = self.settings.long_command_secs;
            let mut long_on = self.settings.notifications.command_done;
            let mut new_secs = secs;
            c.row_icon(icons::CLOCK, p.text_muted, labels[3].0, labels[3].1, |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let step = ui.add_enabled_ui(long_on, |ui| kit::number_stepper(ui, &format!("{secs} s"), 70.0)).inner;
                match step {
                    -1 => new_secs = secs.saturating_sub(1).max(3),
                    1 => new_secs = (secs + 1).min(600),
                    _ => {}
                }
                kit::switch(ui, &mut long_on);
            });
            self.settings.long_command_secs = new_secs;
            self.settings.notifications.command_done = long_on;
            if cfg!(target_os = "macos") {
                c.row_icon(icons::APP_WINDOW, p.text_muted, t("Badge on the Dock icon"), t("Counts what waits for you."), |ui| {
                    kit::switch(ui, &mut self.settings.notifications.dock_badge);
                });
            }
        });
    }

    // -----------------------------------------------------------------------------------------
    // Updates and About
    // -----------------------------------------------------------------------------------------

    fn updates_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        if hit(search, t("Install update"), "") || hit(search, t("Updates"), "") {
            let release = self.updater.available_release().cloned();
            raised(ui, 16, |ui| {
                ui.spacing_mut().item_spacing.y = 14.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    paint_logo(ui.painter(), r, &p);
                    let (title, sub) = match &release {
                        Some(r) => {
                            let mut sub = tf("you have {}", &[crate::update::CURRENT]);
                            if let Some(d) = &r.date {
                                sub.push_str(&format!(" · {}", tf("released {}", &[d.get(..10).unwrap_or(d)])));
                            }
                            if let Some(a) = &r.download {
                                sub.push_str(&format!(" · {}", super::update_ui::human_size(a.size)));
                            }
                            (tf("TernaryCode {} is available", &[&r.version]), sub)
                        }
                        None => {
                            let checked = match self.settings.last_update_check {
                                Some(at) => tf("checked {}", &[&ago(at)]),
                                None => t("not checked yet").to_string(),
                            };
                            (tf("TernaryCode {}", &[crate::update::CURRENT]), checked)
                        }
                    };
                    let bw = if release.is_some() {
                        kit::button_width(ui, None, &self.update_button_label(), None, false)
                    } else {
                        kit::button_width(ui, None, t("Check for updates"), None, false)
                    };
                    let tw = (ui.available_width() - bw - 14.0).max(80.0);
                    ui.vertical(|ui| {
                        ui.set_width(tw);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        text(ui, &title, style::semibold(style::HEADING), p.text, 20.0, true);
                        text(ui, &sub, style::mono(style::DATA), p.text_faint, 14.0, true);
                    });
                    let clicked = if release.is_some() {
                        let label = self.update_button_label();
                        kit::button_ex(ui, kit::Kind::Primary, None, &label, false).clicked()
                    } else {
                        kit::button_ex(ui, kit::Kind::Secondary, None, t("Check for updates"), false).clicked()
                    };
                    if clicked {
                        self.actions.push(if release.is_some() { Action::ShowUpdate } else { Action::CheckForUpdates });
                    }
                });
                if let Some(r) = &release {
                    let notes = note_lines(&r.notes);
                    if !notes.is_empty() {
                        egui::Frame::NONE.fill(p.inset).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 8.0;
                            for (tag, line) in notes.iter().take(8) {
                                release_note(ui, tag, line, &p);
                            }
                        });
                    }
                    let promise = if crate::update::restarts_in_place() {
                        t("Agents and programs keep running through the update: TernaryCode restarts in place.")
                    } else {
                        t("Your projects and tabs come back after the update.")
                    };
                    egui::Frame::NONE.fill(p.accent_tint).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                            ui.painter().text(ir.center(), Align2::CENTER_CENTER, icons::LIGHTNING, style::icon_font(16.0), p.accent_text);
                            text(ui, promise, FontId::proportional(style::SMALL), p.text, 16.0, true);
                        });
                    });
                }
            });
        }
        let labels = [(t("Check for updates automatically"), t("At launch and every few hours. The only connection TernaryCode makes on its own."))];
        section(ui, search, None, None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                kit::switch(ui, &mut self.settings.check_updates);
            });
        });
        if hit(search, t("Releases"), t("MIT license · Inter (OFL) · JetBrains Mono (OFL) · Phosphor icons (MIT)")) {
            credits(ui, &p);
        }
    }

    /// The update card's button: what Install update does next.
    fn update_button_label(&self) -> String {
        if self.updater.prepared().is_some() {
            t("Restart to update").to_string()
        } else if self.updater.downloading() {
            self.updater.status_text().unwrap_or_else(|| t("Updating…").to_string())
        } else {
            t("Install update").to_string()
        }
    }

    fn about_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        if !search.trim().is_empty() && !hit(search, "TernaryCode", t("MIT license · Inter (OFL) · JetBrains Mono (OFL) · Phosphor icons (MIT)")) && !hit(search, t("Releases"), "") {
            return;
        }
        raised(ui, 16, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                paint_logo(ui.painter(), r, &p);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add_space(2.0);
                    text(ui, "TernaryCode", style::semibold(style::HEADING), p.text, 20.0, false);
                    text(ui, &tf("Version {}", &[crate::update::CURRENT]), style::mono(style::DATA), p.text_faint, 14.0, false);
                });
            });
        });
        credits(ui, &p);
    }

    // -----------------------------------------------------------------------------------------
    // Agents
    // -----------------------------------------------------------------------------------------

    fn agents_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let mut presets = self.settings.presets();
        let mut changed = false;
        let mut remove = None;
        let open_id = egui::Id::new("settings-agent-open");
        let confirm_id = egui::Id::new("settings-preset-delete");
        let mut confirm: Option<String> = ui.data(|d| d.get_temp(confirm_id));
        let mut open: Option<String> = ui.data(|d| d.get_temp(open_id)).unwrap_or(Some("claude".to_string()));
        let rows_hit = page_rows(SettingsPage::Agents).iter().any(|r| hit(search, t(r), ""));
        let mut drawn = 0;
        for (i, preset) in presets.iter_mut().enumerate() {
            if !hit(search, &preset.name, &preset.command) && !rows_hit {
                continue;
            }
            if drawn > 0 {
                ui.add_space(8.0);
            }
            drawn += 1;
            let agent = preset.agent.unwrap_or(AgentKind::Custom);
            let expanded = open.as_deref() == Some(preset.id.as_str());
            let hooks = self.agent_hooks(agent);
            egui::Frame::NONE
                .fill(p.surface)
                .stroke(Stroke::new(1.0, if expanded { p.border_strong } else { p.border }))
                .corner_radius(CornerRadius::same(style::RADIUS_CARD))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // Head: the agent, its name and how it reports, its commands; at the right
                    // how many panes run it (or Install / Uninstall), and the caret. A click
                    // opens it.
                    let (head, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0 + 35.0 + 12.0), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("agent-card:{}", preset.id), head);
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let cy = head.center().y;
                    let avatar = Rect::from_center_size(pos2(head.left() + 16.0 + 14.0, cy), vec2(28.0, 28.0));
                    super::profiles_ui::paint_agent_tile(ui.painter(), avatar, preset.agent, &agent_mark(preset), &p);
                    let caret_x = head.right() - 16.0 - 6.0;
                    ui.painter().text(pos2(caret_x, cy), Align2::CENTER_CENTER, if expanded { icons::CARET_UP } else { icons::CARET_DOWN }, style::icon_font(12.0), p.text_faint);
                    let mut right = caret_x - 6.0 - 12.0;
                    if !hooks.hooked && hooks.can_install && self.installed(agent).is_none() && !hooks.off {
                        let w = kit::button_width(ui, None, t("Install…"), None, true);
                        let br = Rect::from_min_size(pos2(right - w, cy - 12.0), vec2(w, 24.0));
                        if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Primary, None, t("Install…"), true)).inner.clicked() {
                            self.actions.push(Action::PlanInstall(agent));
                        }
                        right = br.left() - 12.0;
                    } else if hooks.installed && !matches!(agent, AgentKind::Claude | AgentKind::Codex) && !expanded {
                        // An agent whose hooks are only ever installed: Uninstall right here.
                        let w = kit::button_width(ui, None, t("Uninstall"), None, true);
                        let br = Rect::from_min_size(pos2(right - w, cy - 12.0), vec2(w, 24.0));
                        if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Ghost, None, t("Uninstall"), true)).inner.clicked() {
                            self.actions.push(Action::Uninstall(agent));
                        }
                        right = br.left() - 12.0;
                    } else if hooks.active > 0 {
                        let text = crate::i18n::count(hooks.active, t("Active in 1 pane"), t("Active in {} panes"));
                        let ag = galley(ui, &text, FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
                        let aw = ag.size().x;
                        ui.painter().galley(pos2(right - aw, cy - 8.0), ag, p.text_faint);
                        right -= aw + 12.0;
                    }
                    let x = avatar.right() + 12.0;
                    let room = (right - x).max(40.0);
                    let (status, pill_text) = hooks.pill();
                    let pill_w = kit::state_pill_width(ui.painter(), status, &pill_text);
                    let name_max = if room > pill_w + 68.0 { room - pill_w - 8.0 } else { room };
                    let mut njob = egui::text::LayoutJob::simple_singleline(preset.name.clone(), style::medium(style::BODY), p.text);
                    njob.wrap = egui::text::TextWrapping::truncate_at_width(name_max);
                    let g = ui.painter().layout_job(njob);
                    let nw = g.size().x;
                    let name_cy = cy - 17.5 + 9.0;
                    ui.painter().galley(pos2(x, name_cy - g.size().y / 2.0), g, p.text);
                    if x + nw + 8.0 + pill_w <= right {
                        kit::paint_state_pill(ui.painter(), pos2(x + nw + 8.0, name_cy), status, &pill_text, &p, dark);
                    }
                    let sub = match &preset.resume {
                        Some(r) if !r.is_empty() => format!("{} · {} {r}", preset.command, t("resume:")),
                        _ => format!("{} · {} —", preset.command, t("resume:")),
                    };
                    let mut sjob = egui::text::LayoutJob::simple_singleline(sub, style::mono(style::DATA), p.text_faint);
                    sjob.wrap = egui::text::TextWrapping::truncate_at_width(room);
                    let sg = ui.painter().layout_job(sjob);
                    ui.painter().galley(pos2(x, cy - 17.5 + 18.0 + 3.0), sg, p.text_faint);
                    if resp.clicked() {
                        open = if expanded { None } else { Some(preset.id.clone()) };
                    }
                    if !expanded {
                        return;
                    }
                    // Opened: the commands, the environment, the hooks and what they report, the
                    // browser tools.
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, p.border));
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 16 }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 12.0;
                        const LABEL_W: f32 = 110.0;
                        let field_row = |ui: &mut Ui, label: &str, top: bool, add: &mut dyn FnMut(&mut Ui)| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, if top { 16.0 } else { 30.0 }), Sense::hover());
                                let g = galley(ui, label, FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
                                ui.painter().galley(pos2(r.left(), if top { r.top() + 6.0 } else { r.center().y - 8.0 }), g, p.text_faint);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 8.0;
                                    add(ui);
                                });
                            });
                        };
                        if !preset.builtin {
                            field_row(ui, t("Name"), false, &mut |ui| {
                                let id = egui::Id::new(("preset-name", i));
                                changed |= mono_field(ui, id, &mut preset.name, "", 0.0, false).changed();
                            });
                        }
                        field_row(ui, t("Command"), false, &mut |ui| {
                            let id = egui::Id::new(("preset-command", i));
                            changed |= mono_field(ui, id, &mut preset.command, "", 0.0, false).changed();
                        });
                        field_row(ui, t("Resume"), false, &mut |ui| {
                            let id = egui::Id::new(("preset-resume", i));
                            let mut resume = preset.resume.clone().unwrap_or_default();
                            if mono_field(ui, id, &mut resume, "", 0.0, false).changed() {
                                preset.resume = Some(resume).filter(|r| !r.trim().is_empty());
                                changed = true;
                            }
                        });
                        field_row(ui, t("Environment"), false, &mut |ui| {
                            // KEY=value, one per line (kept as typed while the field has the
                            // keyboard; read into the preset as it changes).
                            let id = egui::Id::new(("preset-env", i));
                            let typed_id = id.with("typed");
                            let focused = ui.memory(|m| m.has_focus(id));
                            let mut text: String = match ui.data(|d| d.get_temp::<String>(typed_id)).filter(|_| focused) {
                                Some(t) => t,
                                None => preset.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n"),
                            };
                            if mono_field(ui, id, &mut text, t("KEY=value, one per line"), 0.0, true).changed() {
                                preset.env = text
                                    .lines()
                                    .filter_map(|kv| kv.trim().split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                                    .filter(|(k, _)| !k.is_empty())
                                    .collect();
                                changed = true;
                            }
                            ui.data_mut(|d| d.insert_temp(typed_id, text));
                        });
                        if agent != AgentKind::Custom {
                            field_row(ui, t("Status signals"), true, &mut |ui| {
                                self.hooks_controls(ui, agent);
                                text(ui, hooks_note(agent), FontId::proportional(style::SMALL), p.text_faint, 16.0, true);
                                // What the hooks report, and the version they were checked with.
                                health_strip(ui, &hooks, agent, &p, dark);
                                if hooks.unconnected > 0 {
                                    let msg = crate::i18n::count(
                                        hooks.unconnected,
                                        t("Typed by hand in 1 pane, it is not connected: Install connects it (then restart it there)."),
                                        t("Typed by hand in {} panes, it is not connected: Install connects it (then restart it there)."),
                                    );
                                    text(ui, &msg, FontId::proportional(style::SMALL), p.warn, 16.0, true);
                                }
                            });
                        } else {
                            field_row(ui, "", true, &mut |ui| {
                                let note = t("A custom agent reports through ternarycode-hook with start, working, needs-input, done and end; without it, its status comes from the terminal.");
                                text(ui, note, FontId::proportional(style::SMALL), p.text_faint, 16.0, true);
                                if confirm.as_deref() == Some(preset.id.as_str()) {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        text(ui, &tf("Delete the preset {}?", &[&preset.name]), FontId::proportional(style::SMALL), p.text, 16.0, false);
                                        if kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), true).clicked() {
                                            confirm = None;
                                        }
                                        if kit::button_ex(ui, kit::Kind::DangerFilled, Some(icons::TRASH), t("Delete"), true).clicked() {
                                            remove = Some(i);
                                            confirm = None;
                                        }
                                    });
                                } else if kit::button_ex(ui, kit::Kind::Danger, Some(icons::TRASH), t("Delete Preset"), true).clicked() {
                                    confirm = Some(preset.id.clone());
                                }
                            });
                        }
                        // The browser tools: what this agent gets in its runs.
                        if agent != AgentKind::Custom {
                            let (ok, status_text) = self.browser_tools_status(agent);
                            let per_launch_note = matches!(agent, AgentKind::Claude | AgentKind::Codex) && self.settings.integration(agent) == Integration::PerLaunch;
                            field_row(ui, t("Browser tools"), false, &mut |ui| {
                                ui.horizontal(|ui| {
                                    ui.set_min_height(30.0);
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    let (st, label) = if ok { (Status::Done, status_text) } else { (Status::Idle, t("Not added")) };
                                    let w = kit::state_pill_width(ui.painter(), st, label);
                                    let (r, _) = ui.allocate_exact_size(vec2(w, kit::PILL_H), Sense::hover());
                                    kit::paint_state_pill(ui.painter(), r.left_center(), st, label, &p, dark);
                                    let note = if per_launch_note {
                                        let n = tc_agent::mcp::tools().as_array().map_or(0, |a| a.len());
                                        tf("{} tools, from presets started with Per launch", &[&n.to_string()])
                                    } else if ok {
                                        String::new()
                                    } else {
                                        status_text.to_string()
                                    };
                                    if !note.is_empty() {
                                        text(ui, &note, FontId::proportional(style::SMALL), p.text_faint, 16.0, true);
                                    }
                                });
                            });
                        }
                    });
                });
        }
        ui.data_mut(|d| {
            d.insert_temp(open_id, open);
            match &confirm {
                Some(c) => {
                    d.insert_temp(confirm_id, c.clone());
                }
                None => d.remove::<String>(confirm_id),
            }
        });
        if let Some(i) = remove {
            presets.remove(i);
            changed = true;
        }
        if drawn > 0 || hit(search, t("Add preset"), "") || hit(search, t("Hook program"), "") {
            if drawn > 0 {
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUS), t("Add preset"), true).clicked() {
                    let n = presets.iter().filter(|p| !p.builtin).count() + 1;
                    // An id no preset has (counting would repeat one after a deletion).
                    let mut k = n;
                    while presets.iter().any(|p| p.id == format!("custom-{k}")) {
                        k += 1;
                    }
                    let id = format!("custom-{k}");
                    presets.push(Preset {
                        id: id.clone(),
                        name: tf("My agent {}", &[&n.to_string()]),
                        agent: Some(AgentKind::Custom),
                        command: String::new(),
                        resume: None,
                        env: Default::default(),
                        builtin: false,
                    });
                    ui.data_mut(|d| d.insert_temp(open_id, Some(id)));
                    changed = true;
                }
                // At the right: the hook program the agents report through (its path cut from
                // the left when it is long).
                let exists = self.hook_bin.bin.exists();
                let (icon, ic) = if exists { (icons::CHECK_CIRCLE, p.ok) } else { (icons::WARNING, p.err) };
                let path = format!("{}{}", crate::paths::contract_tilde(&self.hook_bin.bin), if exists { "" } else { t(" (missing)") });
                let lg = galley(ui, t("Hook program"), FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                let path_room = (r.width() - 16.0 - 13.0 - 6.0 - lg.size().x - 6.0).max(40.0);
                let chars: Vec<char> = path.chars().collect();
                let mut shown = path.clone();
                let mut cut = 0;
                while ui.painter().layout_no_wrap(shown.clone(), style::mono(style::DATA), p.text_faint).size().x > path_room && cut + 8 < chars.len() {
                    cut += 4;
                    shown = format!("…{}", chars[cut..].iter().collect::<String>());
                }
                let pc = if exists { p.text_faint } else { p.err };
                let pg = ui.painter().layout_no_wrap(shown, style::mono(style::DATA), pc);
                let mut x = r.right() - pg.size().x;
                ui.painter().galley(pos2(x, r.center().y - pg.size().y / 2.0), pg, pc);
                x -= 6.0 + lg.size().x;
                ui.painter().galley(pos2(x, r.center().y - 8.0), lg, p.text_faint);
                x -= 6.0 + 6.5;
                ui.painter().text(pos2(x, r.center().y), Align2::CENTER_CENTER, icon, style::icon_fill(13.0), ic);
            });
        }
        if changed {
            // Saved: every user preset, and built-ins only where they differ from the default.
            let defaults = tc_core::agent::builtin_presets();
            self.settings.presets = presets.into_iter().filter(|p| !p.builtin || !defaults.contains(p)).collect();
        }
    }

    /// Per launch | Installed | Off, and Install… or Uninstall.
    fn hooks_controls(&mut self, ui: &mut Ui, agent: AgentKind) {
        let per_launch = matches!(agent, AgentKind::Claude | AgentKind::Codex);
        let installed = self.installed(agent);
        // Per launch is the default (and what Uninstall leaves), but an agent without it gets no
        // hooks then: that is Off. Nothing is saved unless a choice is made.
        let shown = match self.settings.integration(agent) {
            Integration::PerLaunch if !per_launch => Integration::Off,
            m => m,
        };
        let mut modes = Vec::new();
        if per_launch {
            modes.push((Integration::PerLaunch, t("Per launch")));
        }
        modes.push((Integration::Installed, t("Installed")));
        modes.push((Integration::Off, t("Off")));
        let options: Vec<kit::Seg> = modes.iter().map(|(_, l)| kit::Seg::new(l, "")).collect();
        let on = modes.iter().position(|(m, _)| *m == shown).unwrap_or(0);
        let mut action = None;
        let mut mode = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if let Some(i) = kit::segmented(ui, &options, on) {
                let m = modes[i].0;
                if m == Integration::Installed && installed.is_none() {
                    action = Some(Action::PlanInstall(agent));
                } else {
                    mode = Some(m);
                }
            }
            if installed.is_some() {
                if kit::button_ex(ui, kit::Kind::Secondary, None, t("Uninstall"), true).clicked() {
                    action = Some(Action::Uninstall(agent));
                }
            } else {
                let r = kit::button_ex(ui, kit::Kind::Secondary, None, t("Install…"), true);
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("install:{}", agent.slug()), r.rect);
                if r.clicked() {
                    action = Some(Action::PlanInstall(agent));
                }
            }
        });
        if let Some(m) = mode {
            self.settings.integrations.insert(agent.slug().into(), m);
        }
        if let Some(a) = action {
            self.actions.push(a);
        }
    }

    /// How an agent reports to TernaryCode as things are set, and in how many panes it runs with
    /// its hooks now.
    fn agent_hooks(&self, agent: AgentKind) -> Hooks {
        let installed = self.installed(agent).is_some();
        let per_launch = matches!(agent, AgentKind::Claude | AgentKind::Codex) && self.settings.integration(agent) == Integration::PerLaunch;
        let active = self.panes.values().filter(|p| p.tracker.hooks_active && p.tracker.agent == Some(agent)).count();
        let unconnected = self.unconnected_agents().iter().filter(|x| x.1 == agent).count();
        Hooks {
            agent,
            hooked: installed || per_launch,
            installed,
            can_install: agent != AgentKind::Custom,
            off: self.settings.integration(agent) == Integration::Off,
            active,
            unconnected,
        }
    }
}

/// The page's nav search: a magnifier, the field, ⌘F at its right while it is empty.
fn nav_search(ui: &mut Ui, rect: Rect, id: egui::Id, search: &mut String) {
    let p = style::pal(ui);
    let resp = ui.interact(rect, id.with("box"), Sense::click());
    if resp.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.memory(|m| m.has_focus(id));
    if focused {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(11), Stroke::new(3.0, style::soft(p.accent, 0.25)), StrokeKind::Inside);
    }
    ui.painter().rect(rect, CornerRadius::same(8), p.inset, Stroke::new(1.0, if focused { p.accent } else { p.border }), StrokeKind::Inside);
    ui.painter().text(pos2(rect.left() + 10.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(14.0), p.text_faint);
    let mut right = rect.right() - 6.0;
    if search.is_empty() && !focused {
        let keys = kit::keys(&Action::Find).unwrap_or("");
        if !keys.is_empty() && keys.chars().count() <= 4 {
            let w = kit::kbd_width(ui.painter(), keys);
            kit::paint_kbd_text(ui.painter(), pos2(right - w, rect.center().y), keys, &p);
            right -= w + 8.0;
        }
    }
    let inner = Rect::from_min_max(pos2(rect.left() + 10.0 + 14.0 + 8.0, rect.top() + 1.0), pos2(right, rect.bottom() - 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(search)
            .id(id)
            .char_limit(1_024)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(FontId::proportional(style::SMALL))
            .hint_text(RichText::new(t("Search settings")).size(style::SMALL).color(p.text_faint))
            .vertical_align(egui::Align::Center),
    );
}

/// The licences, and the releases page.
fn credits(ui: &mut Ui, p: &style::Palette) {
    gap(ui);
    ui.horizontal(|ui| {
        let w = kit::button_width(ui, Some(icons::ARROW_SQUARE_OUT), t("Releases"), None, true);
        let room = (ui.available_width() - w - 12.0).max(80.0);
        ui.scope(|ui| {
            ui.set_width(room);
            text(ui, t("MIT license · Inter (OFL) · JetBrains Mono (OFL) · Phosphor icons (MIT)"), FontId::proportional(style::SMALL), p.text_faint, 16.0, true);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_SQUARE_OUT), t("Releases"), true).clicked() {
                crate::update::open_page(&format!("https://github.com/{}/releases/latest", crate::update::REPO));
            }
        });
    });
}

/// "2 h ago" for a Unix time.
fn ago(at: i64) -> String {
    let now = crate::profiles::now_unix_secs();
    let s = (now - at).max(0);
    if s < 60 {
        t("just now").to_string()
    } else if s < 3600 {
        tf("{} min ago", &[&(s / 60).to_string()])
    } else if s < 86_400 {
        tf("{} h ago", &[&(s / 3600).to_string()])
    } else {
        tf("{} d ago", &[&(s / 86_400).to_string()])
    }
}

/// The release notes as tagged lines: each bullet under its heading's tag (NEW, IMPROVED…),
/// without Markdown marks. They stop where the notes end for the app ("## Install").
fn note_lines(notes: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut tag = "NEW".to_string();
    for line in notes.lines() {
        let l = line.trim();
        if l.contains("ternarycode:end-of-notes") || l.eq_ignore_ascii_case("## install") {
            break;
        }
        if let Some(h) = l.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim();
            if !h.is_empty() {
                tag = h.split_whitespace().next().unwrap_or(h).to_uppercase();
            }
            continue;
        }
        let Some(item) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).or_else(|| l.strip_prefix("• ")) else {
            continue;
        };
        let mut s = item.replace("**", "").replace('`', "");
        // [text](url) → text.
        while let (Some(a), Some(b)) = (s.find('['), s.find("](")) {
            let Some(c) = s[b..].find(')') else { break };
            if b < a {
                break;
            }
            s = format!("{}{}{}", &s[..a], &s[a + 1..b], &s[b + c + 1..]);
        }
        out.push((tag.clone(), s));
    }
    out
}

/// One release note: its tag outlined in its colour, the line beside it.
fn release_note(ui: &mut Ui, tag: &str, line: &str, p: &style::Palette) {
    let c = match tag {
        "NEW" | "ADDED" => p.accent_text,
        "IMPROVED" | "CHANGED" => p.ok,
        "FIXED" => p.warn,
        _ => p.text_faint,
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let g = ui.painter().layout_no_wrap(tag.to_string(), style::mono(style::DATA), c);
        let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 12.0, 18.0), Sense::hover());
        ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, c), StrokeKind::Inside);
        ui.painter().galley(r.center() - g.size() / 2.0, g, c);
        ui.vertical(|ui| {
            ui.add_space(1.0);
            text(ui, line, FontId::proportional(style::SMALL), p.text, 16.0, true);
        });
    });
}

struct Hooks {
    agent: AgentKind,
    hooked: bool,
    installed: bool,
    can_install: bool,
    off: bool,
    active: usize,
    /// Panes where it runs without the hooks (typed by hand).
    unconnected: usize,
}

impl Hooks {
    /// The pill after the agent's name: how it reports, as a status (Done: hooks; Needs input:
    /// no hooks, its status reads Active; Idle: off).
    fn pill(&self) -> (Status, String) {
        if self.agent == AgentKind::Custom {
            return (Status::Idle, t("Custom").to_string());
        }
        // Per launch covers what Run Agent starts, not the agent typed by hand.
        if self.hooked && !self.installed && self.unconnected > 0 {
            return (Status::NeedsInput, t("Hooks · per launch only").to_string());
        }
        if self.hooked && self.installed && matches!(self.agent, AgentKind::OpenCode | AgentKind::Antigravity) {
            return (Status::Done, t("Plugin installed").to_string());
        }
        if self.hooked {
            let how = if self.installed { t("installed") } else { t("per launch") };
            return (Status::Done, tf("Hooks · {}", &[how]));
        }
        if self.off {
            return (Status::Idle, t("Hooks off").to_string());
        }
        (Status::NeedsInput, t("No hooks · status shows Active").to_string())
    }
}

/// What each agent's hooks report, and how they are added.
fn hooks_note(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => t("Per launch adds the hooks and browser tools to each run, without touching your files. Installed writes them into ~/.claude/settings.json, with a backup."),
        AgentKind::Codex => t("Per launch adds the hooks, notify and browser tools to each run. Codex asks you once to trust TernaryCode's hooks (its own security check)."),
        AgentKind::Gemini => t("Install adds hooks to ~/.gemini/settings.json (tool permissions, finished turns). Gemini CLI has no per-launch option."),
        AgentKind::Antigravity => t("Install adds a TernaryCode plugin to ~/.gemini/config/plugins/ternarycode (status hooks and built-in browser tools; inactive outside TernaryCode)."),
        _ => t("Install adds a small plugin file to OpenCode's plugins folder (permission requests, finished turns). OpenCode has no per-launch option."),
    }
}

/// Two letters for the tile of an agent with no logo.
fn agent_mark(preset: &Preset) -> String {
    match preset.agent {
        Some(a) if a != AgentKind::Custom => a.mark().into(),
        _ => preset.name.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>(),
    }
}

/// Under an agent's status signals: what its hooks report (on the green tint when they are in
/// use), and the version they were checked with.
fn health_strip(ui: &mut Ui, hooks: &Hooks, agent: AgentKind, p: &style::Palette, dark: bool) {
    let (icon, color, text) = if hooks.hooked {
        (icons::CHECK_CIRCLE, p.ok, t("Reports permission requests, questions and finished turns"))
    } else if hooks.off {
        (icons::MINUS_CIRCLE, p.text_faint, t("Off: its status comes from the terminal"))
    } else {
        (icons::WARNING, p.warn, t("Not connected: its status reads “Active” only"))
    };
    let checked = match agent {
        AgentKind::Claude => Some("2.1.319"),
        AgentKind::Codex => Some("0.162.1"),
        AgentKind::OpenCode => Some("1.19.5"),
        AgentKind::Antigravity => Some("1.5.15"),
        AgentKind::Gemini => Some("0.43.0"),
        _ => None,
    };
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let painter = ui.painter();
    let fill = if hooks.hooked { style::tint(p.ok, dark) } else { p.hover };
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    painter.text(pos2(rect.left() + 10.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_fill(14.0), color);
    let mut right = rect.right() - 10.0;
    if let Some(v) = checked {
        let g = painter.layout_no_wrap(tf("checked with {}", &[v]), style::mono(style::DATA), p.text_faint);
        right -= g.size().x;
        painter.galley(pos2(right, rect.center().y - g.size().y / 2.0), g, p.text_faint);
        right -= 12.0;
    }
    let x = rect.left() + 10.0 + 14.0 + 8.0;
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), FontId::proportional(style::SMALL), if hooks.hooked { p.text } else { p.text_muted });
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, p.text);
}

/// "10,000".
fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "13" or "13.5".
fn size_text(size: f32) -> String {
    if size.fract() == 0.0 { format!("{size:.0}") } else { format!("{size:.1}") }
}

/// A theme's card: a miniature window (its sidebar, a sheet with a line of a terminal) in its
/// colours, 152 × 74, then a radio and its name.
fn theme_card(ui: &mut Ui, mode: ThemeMode, name: &str, on: bool) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(152.0, 74.0 + 10.0 + 16.0), Sense::click());
    if resp.hovered() && !on {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let card = Rect::from_min_size(rect.min, vec2(152.0, 74.0));
    let painter = ui.painter();
    // The design's miniature: the window, its sidebar, a sheet, two rows (one picked), a
    // terminal line.
    let mini = |painter: &egui::Painter, origin: egui::Pos2, dark: bool, corners: CornerRadius| {
        let hex = |c: u32| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
        let (bg, sheet, side) = if dark { (0x09090B, 0x151518, 0x0F0F12) } else { (0xEDEDEF, 0xFFFFFF, 0xE4E4E7) };
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(origin + vec2(x, y), vec2(w, h));
        painter.rect_filled(at(0.0, 0.0, 152.0, 74.0), corners, hex(bg));
        painter.rect_filled(at(0.0, 0.0, 44.0, 74.0), CornerRadius { nw: corners.nw, sw: corners.sw, ne: 0, se: 0 }, hex(side));
        painter.rect_filled(at(50.0, 8.0, 98.0, 60.0), CornerRadius::same(5), hex(sheet));
        painter.rect_filled(at(8.0, 14.0, 30.0, 6.0), CornerRadius::same(3), hex(0xF2B14B));
        painter.rect_filled(at(8.0, 26.0, 26.0, 5.0), CornerRadius::same(2), hex(0x80808A));
        painter.rect_filled(at(58.0, 18.0, 60.0, 4.0), CornerRadius::same(2), hex(if dark { 0x7FD1A0 } else { 0x1C7A4A }));
    };
    match mode {
        ThemeMode::Dark | ThemeMode::Light => mini(painter, card.min, mode == ThemeMode::Dark, CornerRadius::same(8)),
        ThemeMode::System => {
            let (l, r) = card.split_left_right_at_x(card.left() + 76.0);
            mini(&painter.with_clip_rect(l.intersect(painter.clip_rect())), card.min, false, CornerRadius::same(8));
            mini(&painter.with_clip_rect(r.intersect(painter.clip_rect())), pos2(card.left() + 76.0, card.top()), true, CornerRadius { nw: 0, sw: 0, ne: 8, se: 8 });
        }
    }
    let stroke = if on {
        Stroke::new(2.0, p.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, p.border_heavy)
    } else {
        Stroke::new(1.0, p.border_strong)
    };
    painter.rect_stroke(card, CornerRadius::same(8), stroke, StrokeKind::Inside);
    // A radio and the name beside it.
    let rc = pos2(card.left() + 7.0, card.bottom() + 10.0 + 8.0);
    if on {
        painter.circle_filled(rc, 7.0, p.accent);
        painter.circle_filled(rc, 3.0, Color32::WHITE);
    } else {
        painter.circle_stroke(rc, 6.25, Stroke::new(1.5, p.border_heavy));
    }
    let font = if on { style::medium(style::SMALL) } else { FontId::proportional(style::SMALL) };
    let g = painter.layout_no_wrap(name.to_string(), font, p.text);
    painter.galley(pos2(rc.x + 7.0 + 8.0, rc.y - g.size().y / 2.0), g, p.text);
    let os = if cfg!(target_os = "macos") { t("follows macOS") } else if cfg!(windows) { t("follows Windows") } else { t("follows the system") };
    if mode == ThemeMode::System { resp.on_hover_text(os) } else { resp }
}

/// A terminal scheme's tile: its background with a prompt and five of its colours, its name.
fn scheme_tile(ui: &mut Ui, s: &tc_term::scheme::Scheme, on: bool) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(104.0, 52.0 + 6.0 + 16.0), Sense::click());
    if resp.hovered() && !on {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    let top = Rect::from_min_size(rect.min, vec2(104.0, 52.0));
    painter.rect_filled(top, CornerRadius::same(7), s.background);
    let stroke = if on {
        Stroke::new(2.0, p.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, p.border_heavy)
    } else {
        Stroke::new(1.0, p.border_strong)
    };
    painter.rect_stroke(top, CornerRadius::same(7), stroke, StrokeKind::Inside);
    let g = painter.layout_no_wrap("❯ ls src".to_string(), style::mono(style::DATA), s.foreground);
    painter.galley(pos2(top.left() + 8.0, top.top() + 16.0 - g.size().y / 2.0), g, s.foreground);
    for (i, c) in [1, 2, 3, 4, 5].iter().map(|k| s.ansi[*k]).enumerate() {
        let r = Rect::from_min_size(top.min + vec2(8.0 + i as f32 * 13.0, 34.0), vec2(10.0, 10.0));
        painter.rect_filled(r, CornerRadius::same(2), c);
    }
    // Its name without " Dark" / " Light" (the row says which).
    let name = s.name.trim_end_matches(" Dark").trim_end_matches(" Light");
    let c = if on { p.text } else { p.text_muted };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), FontId::proportional(style::SMALL), c);
    job.wrap = egui::text::TextWrapping::truncate_at_width(104.0);
    let g = painter.layout_job(job);
    painter.galley(pos2(top.left(), top.bottom() + 6.0), g, c);
    resp
}

/// "+N More" (or "Fewer"): opens the rest of the schemes.
fn more_tile(ui: &mut Ui, id: egui::Id, open: bool, more: usize) -> egui::Response {
    let p = style::pal(ui);
    let (full, _) = ui.allocate_exact_size(vec2(72.0, 52.0 + 6.0 + 16.0), Sense::hover());
    let rect = Rect::from_min_size(full.min, vec2(72.0, 52.0));
    let resp = ui.interact(rect, id.with("tile"), Sense::click());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(7), if resp.hovered() { p.hover } else { Color32::TRANSPARENT }, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    let big = if open { "−".to_string() } else { format!("+{more}") };
    painter.text(rect.center(), Align2::CENTER_CENTER, big, style::medium(style::SMALL), p.text_muted);
    let label = if open { t("Fewer") } else { t("More") };
    painter.text(pos2(rect.left(), rect.bottom() + 6.0), Align2::LEFT_TOP, label, FontId::proportional(style::SMALL), p.text_faint);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What the terminal settings beside it look like: a few lines in the chosen scheme, the
/// terminal's own font and size, the cursor after the command.
fn terminal_preview(ui: &mut Ui, r: Rect, scheme: &tc_term::scheme::Scheme, size: f32, cursor: CursorShape) {
    let p = style::pal(ui);
    let painter = ui.painter_at(r.expand(1.0));
    let head = format!("{} · {} · {} pt", t("Preview"), scheme.name, size_text(size));
    painter.text(pos2(r.left() + 6.0, r.top() + 8.0), Align2::CENTER_CENTER, icons::EYE, style::icon_font(12.0), p.text_faint);
    let hg = galley(ui, &head, FontId::proportional(style::SMALL), p.text_faint, Some(r.width() - 18.0), 16.0);
    painter.galley(pos2(r.left() + 12.0 + 6.0, r.top()), hg, p.text_faint);
    let font = egui::FontId::new(size, egui::FontFamily::Name(tc_term::fonts::TERM_REGULAR.into()));
    let a = scheme.ansi;
    let fg = scheme.foreground;
    let lines: [Vec<(&str, Color32)>; 4] = [
        vec![("storefront ", a[4]), ("main ", a[5]), ("❯ ", a[2]), ("pnpm test", fg)],
        vec![(" ✓ ", a[2]), ("auth/callback.test.ts (12)", fg)],
        vec![(" ✕ ", a[1]), ("cart/voucher.test.ts (1)", fg)],
        vec![("   Tests  13 passed, 1 failed", a[8])],
    ];
    let line_h = (size * 1.54).round();
    let box_r = Rect::from_min_size(pos2(r.left(), r.top() + 16.0 + 8.0), vec2(r.width(), 12.0 * 2.0 + line_h * 4.0));
    painter.rect(box_r, CornerRadius::same(8), scheme.background, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let x0 = box_r.left() + 12.0;
    let mut y = box_r.top() + 12.0;
    let mut first_end = None;
    for line in &lines {
        let mut job = egui::text::LayoutJob::default();
        for (text, c) in line {
            job.append(text, 0.0, egui::TextFormat::simple(font.clone(), *c));
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width(box_r.width() - 24.0);
        let g = painter.layout_job(job);
        first_end.get_or_insert(pos2(x0 + g.size().x, y));
        painter.galley(pos2(x0, y + (line_h - g.size().y) / 2.0), g, fg);
        y += line_h;
    }
    // The cursor after the command, as it is set.
    if let Some(end) = first_end {
        let cw = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
        let cell = Rect::from_min_size(end + vec2(cw * 0.5, (line_h - size * 1.2) / 2.0), vec2(cw, size * 1.2));
        if cell.right() < box_r.right() - 6.0 {
            match cursor {
                CursorShape::Block => painter.rect_filled(cell, CornerRadius::ZERO, scheme.cursor),
                CursorShape::Beam => painter.rect_filled(Rect::from_min_size(cell.min, vec2(2.0, cell.height())), CornerRadius::ZERO, scheme.cursor),
                CursorShape::Underline => painter.rect_filled(Rect::from_min_max(pos2(cell.left(), cell.bottom() - 2.0), cell.max), CornerRadius::ZERO, scheme.cursor),
            };
        }
    }
}

/// The app's mark: a 3 × 3 lattice on the accent.
pub fn paint_logo(painter: &egui::Painter, r: Rect, _p: &style::Palette) {
    // The app icon (assets/icon.svg), drawn: a blue square, the "</" path through a 3 × 3
    // lattice in cream, the lattice's other points faint.
    let s = r.width() / 1024.0;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    let square = Rect::from_min_max(at(72.0, 72.0), at(952.0, 952.0));
    painter.rect_filled(square, CornerRadius::same((196.0 * s).round().min(255.0) as u8), Color32::from_rgb(0x1f, 0x5f, 0xbf));
    let cream = Color32::from_rgb(0xff, 0xf6, 0xea);
    let stroke = Stroke::new((72.0 * s).max(1.0), cream);
    painter.line_segment([at(512.0, 312.0), at(312.0, 512.0)], stroke);
    painter.line_segment([at(312.0, 512.0), at(512.0, 712.0)], stroke);
    painter.line_segment([at(712.0, 312.0), at(512.0, 712.0)], stroke);
    for (x, y) in [(512.0, 312.0), (712.0, 312.0), (312.0, 512.0), (512.0, 712.0)] {
        painter.circle_filled(at(x, y), 52.0 * s, cream);
    }
    for (x, y) in [(312.0, 312.0), (512.0, 512.0), (712.0, 512.0), (312.0, 712.0), (712.0, 712.0)] {
        painter.circle_filled(at(x, y), 30.0 * s, cream.gamma_multiply(0.32));
    }
}
