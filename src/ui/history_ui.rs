//! History, in the code panel (`Changes | Files | History`): the forge (GitHub / GitLab) with its
//! pull requests and Fetch, the commits on a rail, and the selected commit: its message, files
//! and diff. Side by side once the panel is expanded, the commits over the commit otherwise.

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_git::ChangeKind;
use tc_git::ship::{Checks, Forge, PrState};

use super::{icons, kit, review, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

/// A commit's row in the list.
const COMMIT_ROW_H: f32 = 56.0;
/// The list's column once the panel is expanded.
const LIST_W: f32 = 420.0;
/// The narrowest commit side that offers Unified | Split.
const SPLIT_MIN_W: f32 = 760.0;

/// What a frame of History asked for (done once drawing is over).
#[derive(Default)]
struct Asks {
    fetch: bool,
    reload: bool,
    load_more: bool,
    select_commit: Option<usize>,
    select_file: Option<usize>,
    toggle_prs: bool,
    split: Option<bool>,
    open_url: Option<String>,
    open_file: Option<std::path::PathBuf>,
    copy_sha: Option<String>,
    query: Option<String>,
    run_in_term: Option<String>,
    to_terminal: bool,
}

impl App {
    /// History in the code panel: the forge card, the filter and the commits; then the selected
    /// commit (its message, files and diff), beside them once the panel is expanded
    /// ([`review::WIDE_W`] and wider), under them otherwise.
    pub fn history_drawer(&mut self, ui: &mut Ui) {
        let body = ui.max_rect();
        let p = style::pal(ui);
        let Some(h) = self.history.as_ref() else { return };
        let pid = h.project;
        let wide = body.width() >= review::WIDE_W;
        let (list_col, detail) = if wide {
            let col = Rect::from_min_max(body.min, pos2(body.left() + LIST_W, body.bottom()));
            ui.painter().vline(col.right() - 0.5, col.y_range(), Stroke::new(1.0, p.border));
            (col, Rect::from_min_max(pos2(col.right(), body.top()), body.max))
        } else {
            let y = body.top() + stacked_split_h(body.height());
            ui.painter().hline(body.x_range(), y + 0.5, Stroke::new(1.0, p.border));
            (Rect::from_min_max(body.min, pos2(body.right(), y)), Rect::from_min_max(pos2(body.left(), y + 1.0), body.max))
        };
        let mut asks = Asks::default();
        let keys_id = egui::Id::new("history-keys");
        let filter_id = egui::Id::new(("history-filter", pid));

        // ----- the forge, the filter --------------------------------------------------------
        let pad = 12.0;
        let x0 = list_col.left() + pad;
        let w = list_col.width() - pad * 2.0;
        let card_bottom = self.history_forge_card(ui, pos2(x0, list_col.top() + pad), w, &mut asks);
        let filter_rect = Rect::from_min_size(pos2(x0, card_bottom + 10.0), vec2(w, 28.0));
        let Some(h) = self.history.as_ref() else { return };
        let mut q = h.query.clone();
        let filter_resp = ui.scope_builder(egui::UiBuilder::new().max_rect(filter_rect), |ui| kit::filter_field(ui, filter_id, &mut q, Some(icons::MAGNIFYING_GLASS), t("Filter commits"), filter_rect.width())).inner;
        if q != h.query {
            let cleared = q.trim().is_empty() && !h.active_query.is_empty();
            asks.query = Some(q.clone());
            if cleared {
                asks.reload = true;
            }
        }
        // `TextEdit::singleline` consumes Enter and gives up the keyboard in the same frame.
        if filter_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            asks.reload = true;
        }
        if filter_resp.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(filter_id, egui::EventFilter { escape: true, ..Default::default() }));
        }
        if filter_resp.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            if !q.is_empty() {
                let cleared = !h.active_query.is_empty();
                q.clear();
                asks.query = Some(String::new());
                if cleared {
                    asks.reload = true;
                }
            } else {
                ui.memory_mut(|m| m.request_focus(keys_id));
            }
        }

        // ----- the commits ------------------------------------------------------------------
        let list_rect = Rect::from_min_max(pos2(list_col.left() + pad, filter_rect.bottom() + 10.0), pos2(list_col.right() - pad, list_col.bottom() - 4.0));
        let q_trim = q.trim().to_string();
        let q_low = q_trim.to_ascii_lowercase();
        let server_filtered = !h.active_query.is_empty() && q_trim == h.active_query;
        let visible: Vec<usize> = h
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                q_low.is_empty()
                    || server_filtered
                    || c.subject.to_ascii_lowercase().contains(&q_low)
                    || c.body.to_ascii_lowercase().contains(&q_low)
                    || c.author.to_ascii_lowercase().contains(&q_low)
                    || c.hash.to_ascii_lowercase().starts_with(&q_low)
                    || c.refs.iter().any(|r| r.to_ascii_lowercase().contains(&q_low))
            })
            .map(|(i, _)| i)
            .collect();
        // The selection follows the filter.
        if !visible.is_empty() && !visible.contains(&h.selected) {
            asks.select_commit = Some(visible[0]);
        }

        // Keys (j / k / ↑ / ↓ / Esc) once History has been clicked.
        let click_in_body = ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|pt| body.contains(pt) && !filter_rect.contains(pt)));
        if click_in_body && !self.overlay_open() {
            ui.memory_mut(|m| m.request_focus(keys_id));
        }
        let mut scroll_selected = false;
        if ui.memory(|m| m.has_focus(keys_id)) && !filter_resp.has_focus() && !self.overlay_open() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(keys_id, egui::EventFilter { escape: true, vertical_arrows: true, ..Default::default() });
            });
            let mut step = 0i32;
            let mut esc = false;
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::J) || i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                    step += 1;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::K) || i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                    step -= 1;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    esc = true;
                }
            });
            if esc {
                ui.memory_mut(|m| m.surrender_focus(keys_id));
                asks.to_terminal = true;
            } else if step != 0 && !visible.is_empty() {
                let at = visible.iter().position(|&idx| idx == h.selected);
                let next = match at {
                    Some(pos) => (pos as i32 + step).clamp(0, visible.len() as i32 - 1) as usize,
                    None => 0,
                };
                if visible[next] != h.selected {
                    asks.select_commit = Some(visible[next]);
                    scroll_selected = true;
                }
            }
        }

        let dark = ui.visuals().dark_mode;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect.intersect(ui.clip_rect()));
            if h.loading && h.commits.is_empty() {
                centered(ui.painter(), list_rect, t("Loading commit history…"), p.text_muted);
            } else if let Some(err) = &h.error {
                centered(ui.painter(), list_rect, err, p.err);
            } else if h.commits.is_empty() && h.active_query.is_empty() && q_trim.is_empty() {
                kit::empty_state_at(ui, list_rect, icons::GIT_COMMIT, t("No commits yet"), t("Make the first one from Changes."), None);
            } else if visible.is_empty() {
                let msg = t("No matching commits.");
                ui.vertical_centered(|ui| {
                    ui.add_space(((list_rect.height() - 48.0) / 2.0).max(16.0));
                    ui.label(RichText::new(msg).size(style::BODY).color(p.text_muted));
                    if !q_trim.is_empty() && q_trim != h.active_query {
                        ui.add_space(6.0);
                        if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::MAGNIFYING_GLASS), t("Search full history"), true).clicked() {
                            asks.reload = true;
                        }
                    }
                });
            } else {
                egui::ScrollArea::vertical().id_salt(("history-commits", pid)).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let count = visible.len();
                    let sel = asks.select_commit.unwrap_or(h.selected);
                    for (pos, &idx) in visible.iter().enumerate() {
                        let c = &h.commits[idx];
                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), COMMIT_ROW_H), Sense::click());
                        if scroll_selected && idx == sel {
                            ui.scroll_to_rect(row, Some(egui::Align::Center));
                        }
                        if !ui.is_rect_visible(row) {
                            continue;
                        }
                        let selected = idx == sel;
                        let last = pos + 1 == count && !h.has_more;
                        paint_commit_row(ui.painter(), row, c, selected, resp.hovered(), pos == 0, last, &p, dark);
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() && !selected {
                            asks.select_commit = Some(idx);
                        }
                    }
                    ui.add_space(6.0);
                    if !q_trim.is_empty() && q_trim != h.active_query {
                        if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::MAGNIFYING_GLASS), t("Search full history"), true).clicked() {
                            asks.reload = true;
                        }
                    } else if h.has_more {
                        if h.loading_more {
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                ui.label(RichText::new(t("Loading more commits…")).size(style::SMALL).color(p.text_faint));
                            });
                        } else if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CARET_DOWN), t("Load more commits"), true).clicked() {
                            asks.load_more = true;
                        }
                    }
                    ui.add_space(8.0);
                });
            }
        });

        // ----- the commit ---------------------------------------------------------------------
        let active = asks.select_commit.unwrap_or(h.selected);
        let commit = if visible.is_empty() && !q_low.is_empty() { None } else { h.commits.get(active) };
        match commit {
            Some(_) => self.history_commit(ui, detail, active, wide, &mut asks),
            None => {
                if !h.loading && h.error.is_none() {
                    let msg = if !q_low.is_empty() { t("No matching commits.") } else { t("Select a commit to inspect its changes.") };
                    centered(ui.painter(), detail, msg, p.text_muted);
                }
            }
        }
        self.apply_history_asks(asks);
    }

    /// The forge card at `at`, `width` wide: GitHub or GitLab and the repository, ahead / behind,
    /// Fetch; its pull requests and pipelines, the open ones (listed when unfolded). A remote of
    /// another kind, or none: its name and Fetch. Returns its bottom.
    fn history_forge_card(&mut self, ui: &mut Ui, at: egui::Pos2, width: f32, asks: &mut Asks) -> f32 {
        let p = style::pal(ui);
        let Some(h) = self.history.as_ref() else { return at.y };
        let pid = h.project;
        let facts = self.ship.get(&pid).and_then(|s| s.facts.clone());
        let remote = facts.as_ref().and_then(|f| f.remote.clone());
        let web = remote.as_ref().and_then(|r| r.web.clone());
        let branch_pr = self.ship.get(&pid).and_then(|s| s.pr.clone());
        let ahead = facts.as_ref().map_or(0, |f| f.unpushed);
        let behind = facts.as_ref().map_or(0, |f| f.behind);
        let has_cli = self.ship.get(&pid).is_some_and(|s| crate::ship::cli_for(web.as_ref(), &s.gh, &s.glab).is_some());
        let prs_open = h.prs_expanded && !h.open_prs.is_empty() && web.is_some();
        let list_h = if prs_open { 8.0 + (h.open_prs.len() as f32 * 26.0).min(132.0) } else { 0.0 };
        let height = if web.is_some() { 10.0 + 24.0 + 8.0 + 24.0 + list_h + 10.0 } else { 10.0 + 24.0 + 10.0 };
        let card = Rect::from_min_size(at, vec2(width, height));
        ui.painter().rect(card, CornerRadius::same(style::RADIUS_CARD), p.inset, Stroke::new(1.0, p.border), StrokeKind::Inside);
        let row1 = Rect::from_min_size(pos2(card.left() + 10.0, card.top() + 10.0), vec2(card.width() - 20.0, 24.0));
        let painter = ui.painter().clone();
        let cy = row1.center().y;

        // Row 1, from the right: Fetch (a spinner while it fetches), ahead / behind.
        let fetch_r = Rect::from_min_size(pos2(row1.right() - 24.0, row1.top()), vec2(24.0, 24.0));
        let has_remote = remote.is_some();
        if has_remote {
            if h.fetching {
                super::status::paint_spinner(&painter, fetch_r.center(), 14.0, p.info);
            } else if place(ui, fetch_r, |ui| kit::icon_button_ex(ui, icons::ARROWS_CLOCKWISE, t("Fetch from remote"), false, 24.0)).clicked() {
                asks.fetch = true;
            }
        }
        let mut right = if has_remote { fetch_r.left() - 8.0 } else { row1.right() };
        if web.is_some() && (ahead > 0 || behind > 0) {
            let c = if ahead > 0 { p.warn } else { p.text_faint };
            let g = painter.layout_no_wrap(format!("↑{ahead} ↓{behind}"), style::mono(style::DATA), c);
            right -= g.size().x;
            painter.galley(pos2(right, cy - g.size().y / 2.0), g, c);
            right -= 8.0;
        }
        match &web {
            Some(web) => {
                let is_gitlab = web.forge == Forge::GitLab;
                // The forge's badge, then the repository (a click opens it).
                let label = web.forge.label();
                let g = painter.layout_no_wrap(label.to_string(), style::medium(style::SMALL), p.text);
                let icon_w = if is_gitlab { 0.0 } else { 13.0 + 5.0 };
                let badge = Rect::from_min_size(pos2(row1.left(), cy - 10.0), vec2(7.0 + icon_w + g.size().x + 7.0, 20.0));
                painter.rect_filled(badge, CornerRadius::same(5), p.subtle);
                if !is_gitlab {
                    painter.text(pos2(badge.left() + 7.0 + 6.5, cy), Align2::CENTER_CENTER, icons::GITHUB_LOGO, style::icon_font(13.0), p.text);
                }
                painter.galley(pos2(badge.left() + 7.0 + icon_w, cy - g.size().y / 2.0), g, p.text);
                let sx = badge.right() + 8.0;
                let mut job = egui::text::LayoutJob::simple_singleline(web.repo.clone(), style::mono(style::MONO), p.text_muted);
                job.wrap = egui::text::TextWrapping::truncate_at_width((right - sx).max(20.0));
                let sg = painter.layout_job(job);
                let slug = Rect::from_min_size(pos2(sx, cy - sg.size().y / 2.0), sg.size());
                let resp = ui.interact(slug.expand(2.0), egui::Id::new(("history-repo", pid)), Sense::click());
                painter.galley(slug.min, sg, if resp.hovered() { p.text } else { p.text_muted });
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.on_hover_text(tf("Open repository on {}", &[web.forge.label()])).clicked() {
                    asks.open_url = Some(web.url());
                }

                // Row 2: the forge's pull requests and pipelines; at the right the open ones (or
                // how to see them).
                let row2 = Rect::from_min_size(pos2(card.left() + 10.0, row1.bottom() + 8.0), vec2(card.width() - 20.0, 24.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row2).layout(egui::Layout::left_to_right(egui::Align::Center)));
                child.spacing_mut().item_spacing.x = 4.0;
                let (prs_label, prs_tip) = if is_gitlab {
                    (t("Merge requests"), t("Open Merge Requests on GitLab"))
                } else {
                    (t("Pull requests"), t("Open Pull Requests on GitHub"))
                };
                if kit::button_ex(&mut child, kit::Kind::Ghost, Some(icons::GIT_PULL_REQUEST), prs_label, true).on_hover_text(prs_tip).clicked() {
                    asks.open_url = Some(web.prs_url());
                }
                let ci_label = if is_gitlab { t("Pipelines") } else { t("Actions") };
                if kit::button_ex(&mut child, kit::Kind::Ghost, Some(icons::LIGHTNING), ci_label, true).on_hover_text(tf("Open {} on {}", &[ci_label, web.forge.label()])).clicked() {
                    asks.open_url = Some(web.actions_url());
                }
                child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !h.open_prs.is_empty() {
                        let lbl = tf("{} open", &[&h.open_prs.len().to_string()]);
                        if kit::button_full(ui, kit::Kind::Secondary, None, &lbl, true, h.prs_expanded).clicked() {
                            asks.toggle_prs = true;
                        }
                    } else if h.prs_signed_out {
                        let cli = web.forge.cli_name();
                        let cmd = if is_gitlab && web.host != "gitlab.com" {
                            format!("glab auth login --hostname {}", crate::ship::quote(&web.host))
                        } else if !is_gitlab && web.host != "github.com" {
                            format!("gh auth login --hostname {}", crate::ship::quote(&web.host))
                        } else {
                            format!("{cli} auth login")
                        };
                        if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::KEY), &tf("Sign in ({})", &[cli]), true).on_hover_text(&cmd).clicked() {
                            asks.run_in_term = Some(cmd);
                        }
                    } else if let Some(pr) = &branch_pr {
                        let badge = format!("{}{}", web.forge.request_prefix(), pr.number);
                        if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::GIT_PULL_REQUEST), &badge, true).clicked() {
                            asks.open_url = Some(pr.url.clone());
                        }
                    } else if !has_cli {
                        ui.label(RichText::new(tf("Install {} CLI for live list", &[web.forge.cli_name()])).size(style::SMALL).color(p.text_faint));
                    }
                });

                // The open pull requests, unfolded.
                if prs_open {
                    let list = Rect::from_min_max(pos2(card.left() + 6.0, row2.bottom() + 8.0), pos2(card.right() - 6.0, card.bottom() - 8.0));
                    painter.hline(card.x_range().shrink(10.0), row2.bottom() + 4.5, Stroke::new(1.0, p.border));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(egui::Layout::top_down(egui::Align::Min)));
                    child.set_clip_rect(list.intersect(child.clip_rect()));
                    egui::ScrollArea::vertical().id_salt(("history-open-prs", pid)).auto_shrink([false, false]).show(&mut child, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for pr in &h.open_prs {
                            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rr, CornerRadius::same(5), p.hover);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let cy = rr.center().y;
                            let mut rx = rr.left() + 6.0;
                            let (icon, color, fill) = match pr.checks {
                                Checks::Passed => (icons::CHECK_CIRCLE, p.ok, true),
                                Checks::Failed => (icons::X_CIRCLE, p.err, true),
                                Checks::Pending => (icons::CIRCLE_NOTCH, p.info, false),
                                Checks::None => (icons::GIT_PULL_REQUEST, if pr.state == PrState::Draft { p.text_faint } else { p.accent_text }, false),
                            };
                            ui.painter().text(pos2(rx + 6.0, cy), Align2::CENTER_CENTER, icon, if fill { style::icon_fill(12.0) } else { style::icon_font(12.0) }, color);
                            rx += 18.0;
                            let ng = ui.painter().layout_no_wrap(format!("{}{}", web.forge.request_prefix(), pr.number), style::mono(style::DATA), p.text_muted);
                            let nw = ng.size().x;
                            ui.painter().galley(pos2(rx, cy - ng.size().y / 2.0), ng, p.text_muted);
                            rx += nw + 6.0;
                            let mut tj = egui::text::LayoutJob::simple_singleline(pr.title.clone(), egui::FontId::proportional(style::SMALL), p.text);
                            tj.wrap = egui::text::TextWrapping::truncate_at_width((rr.right() - 6.0 - rx).max(24.0));
                            let tg = ui.painter().layout_job(tj);
                            ui.painter().galley(pos2(rx, cy - tg.size().y / 2.0), tg, p.text);
                            if resp.on_hover_text(format!("{} · {} ({})", pr.title, pr.branch, pr.author)).clicked() {
                                asks.open_url = Some(pr.url.clone());
                            }
                        }
                    });
                }
            }
            None => {
                // A local repository, or a remote of another kind: its name.
                painter.text(pos2(row1.left() + 6.5, cy), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(13.0), p.text_faint);
                let label = remote.as_ref().map(|r| r.name.clone()).unwrap_or_else(|| t("Local repository").to_string());
                let mut job = egui::text::LayoutJob::simple_singleline(label, style::medium(style::SMALL), p.text_muted);
                job.wrap = egui::text::TextWrapping::truncate_at_width((right - row1.left() - 21.0).max(20.0));
                let g = painter.layout_job(job);
                painter.galley(pos2(row1.left() + 21.0, cy - g.size().y / 2.0), g, p.text_muted);
            }
        }
        card.bottom()
    }

    /// The selected commit in `rect`: its card (subject, message, who and when, where it is),
    /// the files it changed and the selected file's diff.
    fn history_commit(&mut self, ui: &mut Ui, rect: Rect, idx: usize, wide: bool, asks: &mut Asks) {
        let p = style::pal(ui);
        let Some(h) = self.history.as_ref() else { return };
        let Some(commit) = h.commits.get(idx) else { return };
        let pid = h.project;
        let web = self.ship.get(&pid).and_then(|s| s.facts.as_ref()).and_then(|f| f.remote.as_ref()).and_then(|r| r.web.clone());

        // The card, at most a part of the height (a long message scrolls).
        let max_h = (rect.height() * if wide { 0.36 } else { 0.45 }).max(96.0);
        let area = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + max_h).min(rect.bottom())));
        let card_bottom = ui
            .scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                ui.set_clip_rect(area.intersect(ui.clip_rect()));
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width((area.width() - 32.0).max(80.0));
                        ui.spacing_mut().item_spacing.y = 8.0;
                        let mut buttons = |ui: &mut Ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            // Right to left: the commit's id (copies it), then the forge.
                            if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::COPY), &commit.short, true).on_hover_text(t("Copy full commit hash")).clicked() {
                                asks.copy_sha = Some(commit.hash.clone());
                            }
                            if let Some(web) = &web
                                && kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), web.forge.label(), true).on_hover_text(tf("Open commit on {}", &[web.forge.label()])).clicked()
                            {
                                asks.open_url = Some(web.commit_url(&commit.hash));
                            }
                        };
                        let title = RichText::new(&commit.subject).font(style::semibold(style::TITLE)).color(p.text);
                        if area.width() >= 380.0 {
                            egui::Sides::new().spacing(10.0).shrink_left().show(
                                ui,
                                |ui| {
                                    ui.add(egui::Label::new(title).wrap());
                                },
                                |ui| buttons(ui),
                            );
                        } else {
                            ui.add(egui::Label::new(title).wrap());
                            ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui)));
                        }
                        if !commit.body.is_empty() {
                            egui::ScrollArea::vertical().id_salt(("history-commit-body", pid, idx)).max_height(64.0).auto_shrink([false, true]).show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(&commit.body).size(style::SMALL).color(p.text_muted)).wrap());
                            });
                        }
                        // Who and when, a merge, where it is, not pushed.
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
                            if !commit.author.is_empty() {
                                ui.label(RichText::new(&commit.author).font(style::medium(style::SMALL)).color(p.text));
                            }
                            if commit.time > 0 {
                                ui.label(RichText::new(format_commit_time(commit.time)).font(style::mono(style::DATA)).color(p.text_faint));
                            }
                            if commit.parents.len() > 1 {
                                outline_pill(ui, t("Merge"), p.info);
                            }
                            for raw in filtered_refs(&commit.refs, None, false) {
                                let (label, c) = format_ref_pill(raw, &p);
                                outline_pill(ui, &label, c);
                            }
                            if commit.unpushed {
                                outline_pill(ui, t("Not pushed"), p.warn);
                            }
                        });
                    })
                    .response
                    .rect
                    .bottom()
            })
            .inner;
        ui.painter().hline(rect.x_range(), card_bottom + 0.5, Stroke::new(1.0, p.border));

        // The files bar: how many changed, + −; Open in editor, Unified | Split.
        let content = Rect::from_min_max(pos2(rect.left(), (card_bottom + 1.0).min(rect.bottom())), rect.max);
        let (adds, dels): (u32, u32) = h.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
        let bar = Rect::from_min_size(content.min, vec2(content.width(), 40.0_f32.min(content.height())));
        let split_ok = content.width() >= SPLIT_MIN_W;
        let split = h.split && split_ok;
        let selected_f = h.files.get(h.selected_file);
        let inner = Rect::from_min_max(pos2(bar.left() + 12.0, bar.top()), pos2((bar.right() - 12.0).max(bar.left() + 12.0), bar.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner.intersect(ui.clip_rect()));
            egui::Sides::new().spacing(8.0).shrink_left().show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if h.files_loading {
                        ui.label(RichText::new(t("Loading changed files…")).size(style::SMALL).color(p.text_faint));
                    } else {
                        let count = crate::i18n::count(h.files.len(), t("1 file changed"), t("{} files changed"));
                        ui.label(RichText::new(count).font(style::medium(style::SMALL)).color(p.text));
                        ui.label(RichText::new(format!("+{adds}")).font(style::mono(style::DATA)).color(p.ok));
                        ui.label(RichText::new(format!("−{dels}")).font(style::mono(style::DATA)).color(p.err));
                    }
                },
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(f) = selected_f
                        && f.kind != ChangeKind::Deleted
                        && kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in editor"), true).on_hover_text(t("Open selected file in editor")).clicked()
                    {
                        asks.open_file = Some(h.root.join(&f.path));
                    }
                    if split_ok {
                        let options = [kit::Seg::new(t("Unified"), ""), kit::Seg::new(t("Split"), "")];
                        let w = kit::segmented_width(ui, &options);
                        let (sr, _) = ui.allocate_exact_size(vec2(w, kit::SEGMENTED_H), Sense::hover());
                        if let Some(i) = kit::segmented_in(ui, sr, egui::Id::new("history-split"), &options, h.split as usize) {
                            asks.split = Some(i == 1);
                        }
                    }
                },
            );
        });

        // The files.
        let avail = (content.bottom() - bar.bottom()).max(0.0);
        let row_h = review::FILE_ROW_H + 1.0;
        let files_h = if h.files.is_empty() || avail < 24.0 {
            0.0
        } else {
            (h.files.len() as f32 * row_h + 4.0).min((avail * 0.3).clamp(row_h * 2.0 + 4.0, 180.0)).min(avail * 0.5)
        };
        let files = Rect::from_min_size(pos2(content.left() + 6.0, bar.bottom()), vec2(content.width() - 12.0, files_h));
        let diff_rect = Rect::from_min_max(pos2(content.left(), files.bottom() + if files_h > 0.0 { 6.0 } else { 0.0 }), content.max);
        if files_h > 0.0 {
            ui.scope_builder(egui::UiBuilder::new().max_rect(files), |ui| {
                ui.set_clip_rect(files.intersect(ui.clip_rect()));
                egui::ScrollArea::vertical().id_salt(("history-commit-files", pid, h.selected)).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (fi, f) in h.files.iter().enumerate() {
                        let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), review::FILE_ROW_H), Sense::click());
                        if !ui.is_rect_visible(rr) {
                            continue;
                        }
                        let on = fi == h.selected_file;
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        let (folder, name) = match f.path.rsplit_once('/') {
                            Some((d, n)) => (d.to_string(), n.to_string()),
                            None => (String::new(), f.path.clone()),
                        };
                        let sub = match &f.old_path {
                            Some(old) => format!("← {old}"),
                            None => folder,
                        };
                        review::paint_file_row(
                            ui.painter(),
                            rr,
                            &review::FileRow {
                                kind: f.kind,
                                name: &name,
                                sub: &sub,
                                adds: f.additions,
                                dels: f.deletions,
                                binary: f.binary,
                                check: None,
                                box_slot: false,
                                agent: None,
                                selected: on,
                                hovered: resp.hovered(),
                                loose: false,
                                indent: 0.0,
                            },
                            &p,
                        );
                        if resp.clicked() && !on {
                            asks.select_file = Some(fi);
                        }
                    }
                });
            });
            ui.painter().hline(content.x_range(), files.bottom() + 5.5, Stroke::new(1.0, p.border));
        }

        // The diff.
        if let Some(err) = &h.files_error {
            centered(ui.painter(), diff_rect, err, p.err);
        } else if h.files_loading {
            centered(ui.painter(), diff_rect, t("Loading the diff…"), p.text_muted);
        } else if h.files.is_empty() {
            centered(ui.painter(), diff_rect, t("No file changes in this commit."), p.text_muted);
        } else if let Some(f) = selected_f {
            match (&h.diff, &h.diff_error) {
                (_, Some(e)) => centered(ui.painter(), diff_rect, e, p.err),
                (None, None) => centered(ui.painter(), diff_rect, t("Loading the diff…"), p.text_muted),
                (Some(d), None) if d.binary => centered(ui.painter(), diff_rect, t("Binary file: no text diff."), p.text_muted),
                (Some(d), None) if d.hunks.is_empty() => centered(ui.painter(), diff_rect, t("No text changes in this file."), p.text_muted),
                (Some(d), None) => {
                    review::diff_view(ui, diff_rect, ("history-diff", pid, h.selected, h.selected_file), d, &f.path, split, None, None);
                }
            }
        }
    }

    fn apply_history_asks(&mut self, a: Asks) {
        if let Some(q) = a.query
            && let Some(h) = self.history.as_mut()
        {
            h.query = q;
        }
        if a.toggle_prs
            && let Some(h) = self.history.as_mut()
        {
            h.prs_expanded = !h.prs_expanded;
        }
        if let Some(s) = a.split {
            self.review_split = s;
            if let Some(h) = self.history.as_mut() {
                h.split = s;
            }
        }
        if a.fetch {
            self.history_fetch();
        }
        if a.reload {
            self.history_reload(false);
        }
        if a.load_more {
            self.history_reload(true);
        }
        if let Some(idx) = a.select_commit {
            self.history_select_commit(idx);
        }
        if let Some(idx) = a.select_file {
            self.history_select_file(idx);
        }
        if let Some(url) = a.open_url {
            self.actions.push(Action::OpenWeb(url));
        }
        if let Some(cmd) = a.run_in_term {
            self.actions.push(Action::TypeInTerminal(cmd));
        }
        if let Some(path) = a.open_file {
            if path.is_file() {
                self.history = None;
                self.actions.push(Action::OpenFile(path, None));
            } else {
                self.toast(t("File no longer exists in the working tree.").to_string());
            }
        }
        if let Some(sha) = a.copy_sha {
            self.ctx.copy_text(sha);
            self.toast(t("Copied.").to_string());
        }
        if a.to_terminal {
            self.focus_request = self.current_pane();
        }
    }
}

/// One commit on the rail (56 high): its dot (HEAD filled in the accent, not pushed an amber ring,
/// others a quiet ring), the subject and how long ago, then its id, where it is, "local" and who.
#[allow(clippy::too_many_arguments)]
fn paint_commit_row(painter: &egui::Painter, row: Rect, c: &tc_git::history::CommitEntry, selected: bool, hovered: bool, first: bool, last: bool, p: &style::Palette, dark: bool) {
    if selected {
        painter.rect_filled(row, CornerRadius::same(8), p.accent_tint);
    } else if hovered {
        painter.rect_filled(row, CornerRadius::same(8), p.hover);
    }
    // The rail, broken around the dot.
    let rx = row.left() + 6.0 + 7.5;
    let dot = pos2(rx, row.top() + 18.5);
    let stroke = Stroke::new(1.0, p.border_strong);
    if !first {
        painter.vline(rx, row.top()..=(dot.y - 6.5), stroke);
    }
    if !last {
        painter.vline(rx, (dot.y + 6.5)..=row.bottom(), stroke);
    }
    let head = c.refs.iter().any(|r| r == "HEAD" || r.starts_with("HEAD -> "));
    match (head, c.unpushed) {
        (true, true) => {
            painter.circle_filled(dot, 4.5, p.accent);
            painter.circle_stroke(dot, 4.5, Stroke::new(2.0, p.warn));
        }
        (true, false) => {
            painter.circle_filled(dot, 4.5, p.accent);
        }
        (false, true) => {
            painter.circle_stroke(dot, 3.5, Stroke::new(2.0, p.warn));
        }
        (false, false) => {
            painter.circle_stroke(dot, 3.75, Stroke::new(1.5, p.border_heavy));
        }
    }
    // Line 1: the subject, how long ago.
    let x = row.left() + 6.0 + 16.0 + 8.0;
    let right = row.right() - 10.0;
    let y1 = row.top() + 10.0 + 8.0;
    let age = painter.layout_no_wrap(short_age(c.time), style::mono(style::DATA), p.text_faint);
    let aw = age.size().x;
    painter.galley(pos2(right - aw, y1 - age.size().y / 2.0), age, p.text_faint);
    let font = if selected { style::medium(style::SMALL) } else { egui::FontId::proportional(style::SMALL) };
    let mut job = egui::text::LayoutJob::simple_singleline(c.subject.clone(), font, p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - aw - 8.0 - x).max(20.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, y1 - g.size().y / 2.0), g, p.text);
    // Line 2: the id, where it is (two at most), local, who.
    let y2 = y1 + 16.0 + 4.0;
    let mono = style::mono(style::DATA);
    let sha = painter.layout_no_wrap(c.short.clone(), mono.clone(), p.accent_text);
    let mut bx = x + sha.size().x + 6.0;
    painter.galley(pos2(x, y2 - sha.size().y / 2.0), sha, p.accent_text);
    let head_branch = c.refs.iter().find_map(|r| r.strip_prefix("HEAD -> "));
    let mut pills: Vec<(String, Color32)> = filtered_refs(&c.refs, head_branch, true).into_iter().take(2).map(|r| format_ref_pill(r, p)).collect();
    if c.unpushed {
        pills.push((t("local").to_string(), p.warn));
    }
    for (label, col) in pills {
        let max_w = (right - 24.0 - bx).min(120.0);
        if max_w < 28.0 {
            break;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(label, mono.clone(), col);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w - 10.0);
        let g = painter.layout_job(job);
        let r = Rect::from_min_size(pos2(bx, y2 - 8.0), vec2(g.size().x + 10.0, 16.0));
        painter.rect_filled(r, CornerRadius::same(4), style::tint(col, dark));
        painter.galley(pos2(r.left() + 5.0, y2 - g.size().y / 2.0), g, col);
        bx = r.right() + 6.0;
    }
    if right - bx > 24.0 && !c.author.is_empty() {
        let mut job = egui::text::LayoutJob::simple_singleline(c.author.clone(), egui::FontId::proportional(style::SMALL), p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width(right - bx);
        let g = painter.layout_job(job);
        painter.galley(pos2(bx, y2 - g.size().y / 2.0), g, p.text_faint);
    }
}

/// A word in an outline the colour of its text (Not pushed, Merge, a branch): 18 high.
fn outline_pill(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), style::mono(style::DATA), color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 12.0, 18.0), Sense::hover());
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, color), StrokeKind::Inside);
    ui.painter().galley(pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

/// `add` in a child of `ui` over `rect`, a place already taken (the parent's layout is left
/// alone).
fn place<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    add(&mut child)
}

/// "45s", "9m", "2h", "3d" since a Unix time: the short age of the commit list.
fn short_age(t: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let s = (now - t).max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// Leaves out the noisy `origin/HEAD` / `*/HEAD -> ...` refs (and in a row, `origin/<head>`).
fn filtered_refs<'a>(refs: &'a [String], head_branch: Option<&str>, compact: bool) -> Vec<&'a str> {
    refs.iter()
        .map(String::as_str)
        .filter(|r| {
            if r.ends_with("/HEAD") || r.contains("/HEAD -> ") {
                return false;
            }
            if compact
                && let Some(b) = head_branch
                && let Some((_, tail)) = r.split_once('/')
                && tail == b
            {
                return false;
            }
            true
        })
        .collect()
}

/// A wrapped message in the middle of `rect`.
fn centered(painter: &egui::Painter, rect: Rect, text: &str, color: Color32) {
    let g = painter.layout(text.to_string(), egui::FontId::proportional(style::BODY), color, (rect.width() - 28.0).max(60.0));
    let sz = g.size();
    painter.with_clip_rect(rect.intersect(painter.clip_rect())).galley(pos2(rect.center().x - sz.x / 2.0, rect.center().y - sz.y / 2.0), g, color);
}

/// A git ref (`HEAD -> main`, `HEAD`, `origin/main`, `tag: v0.2.3`) as a pill's word and colour:
/// branches in the accent, remotes blue, tags amber.
fn format_ref_pill(raw: &str, p: &style::Palette) -> (String, Color32) {
    if raw == "HEAD" {
        ("HEAD".to_string(), p.accent_text)
    } else if let Some(branch) = raw.strip_prefix("HEAD -> ") {
        (branch.to_string(), p.accent_text)
    } else if let Some(tag) = raw.strip_prefix("tag: ") {
        (tag.to_string(), p.warn)
    } else if raw.contains('/') {
        (raw.to_string(), p.info)
    } else {
        (raw.to_string(), p.accent_text)
    }
}

/// When a commit was made: "Today 14:03 · 2 min ago", "Yesterday 09:12 · 1 d ago", else
/// "2026-11-20 14:03 · 3 d ago".
fn format_commit_time(unix: i64) -> String {
    let age = tf("{} ago", &[&review::unix_age(unix)]);
    let Some(dt) = chrono::DateTime::from_timestamp(unix, 0) else { return age };
    let local = dt.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let day = local.date_naive();
    let when = if day == today {
        tf("Today {}", &[&local.format("%H:%M").to_string()])
    } else if today.pred_opt() == Some(day) {
        tf("Yesterday {}", &[&local.format("%H:%M").to_string()])
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    };
    format!("{when} · {age}")
}

/// Height of the top commit list when History is stacked vertically in a narrow panel.
fn stacked_split_h(body_h: f32) -> f32 {
    let body_h = body_h.max(0.0);
    let hi = (body_h - 180.0).max(120.0).min(body_h);
    let lo = 240.0_f32.min(body_h * 0.6).min(hi);
    (body_h * 0.46).clamp(lo, hi)
}

#[cfg(test)]
mod tests {
    use super::stacked_split_h;

    #[test]
    fn stacked_split_never_panics_across_heights() {
        for h in [0.0, 1.0, 50.0, 120.0, 180.0, 200.0, 250.0, 350.0, 400.0, 420.0, 600.0, 1200.0] {
            let y = stacked_split_h(h);
            assert!(y >= 0.0 && y <= h, "height={h} produced y={y}");
        }
    }
}
