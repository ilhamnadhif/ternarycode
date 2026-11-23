//! Commit History & GitHub/GitLab Forge integration view (`Files | Changes | History`),
//! drawn inside the Code Editor stage.

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_git::ChangeKind;
use tc_git::ship::{Checks, Forge, PrState};

use super::{icons, kit, review, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

/// Row height for one commit item in the timeline column.
const COMMIT_ROW_H: f32 = 52.0;
/// Row height for one changed file in the commit inspector.
const FILE_ROW_H: f32 = 30.0;
/// Minimum diff pane width to show the Unified | Split switch.
const SPLIT_MIN_W: f32 = 540.0;

impl App {
    /// Renders Commit History + GitHub/GitLab Forge panel inside the Code Editor stage.
    pub fn history_drawer(&mut self, ui: &mut Ui) {
        let body = ui.max_rect();
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let Some(h) = self.history.as_ref() else { return };
        let pid = h.project;

        // Read remote & forge metadata from `self.ship`.
        let facts = self.ship.get(&pid).and_then(|s| s.facts.clone());
        let remote = facts.as_ref().and_then(|f| f.remote.clone());
        let web = remote.as_ref().and_then(|r| r.web.clone());
        let branch_pr = self.ship.get(&pid).and_then(|s| s.pr.clone());
        let ahead = facts.as_ref().map_or(0, |f| f.unpushed);
        let behind = facts.as_ref().map_or(0, |f| f.behind);
        let has_cli = self
            .ship
            .get(&pid)
            .is_some_and(|s| crate::ship::cli_for(web.as_ref(), &s.gh, &s.glab).is_some());

        let left_w = (body.width() * 0.40).clamp(245.0, 360.0).min(body.width() * 0.48);
        let left_rect = Rect::from_min_max(body.min, pos2(body.left() + left_w, body.bottom()));
        let right_rect = Rect::from_min_max(pos2(left_rect.right(), body.top()), body.max);
        ui.painter().vline(left_rect.right() - 0.5, left_rect.y_range(), Stroke::new(1.0, p.border));

        // Actions deferred until borrows end.
        let mut do_fetch = false;
        let mut do_reload = false;
        let mut do_load_more = false;
        let mut select_commit: Option<usize> = None;
        let mut select_file: Option<usize> = None;
        let mut toggle_prs = false;
        let mut toggle_split: Option<bool> = None;
        let mut open_url: Option<String> = None;
        let mut open_file: Option<std::path::PathBuf> = None;
        let mut copy_sha: Option<String> = None;
        let mut new_query: Option<String> = None;

        // ===== LEFT COLUMN: Forge Card + Search + Commit Timeline =====
        let mut y = left_rect.top() + 8.0;

        // 1. GitHub / GitLab Forge Integration Card
        let card_rect = Rect::from_min_max(pos2(left_rect.left() + 8.0, y), pos2(left_rect.right() - 8.0, left_rect.bottom()));
        let card_bottom = ui
            .scope_builder(egui::UiBuilder::new().max_rect(card_rect), |ui| {
                ui.set_clip_rect(card_rect);
                egui::Frame::NONE
                    .fill(if dark { p.inset } else { Color32::from_rgb(0xf6, 0xf6, 0xf5) })
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width((card_rect.width() - 20.0).max(120.0));
                        ui.spacing_mut().item_spacing.y = 6.0;

                        if let Some(web) = &web {
                            let is_gitlab = web.forge == Forge::GitLab;
                            let forge_c = if is_gitlab { p.warn } else { p.accent_text };
                            // Row 1: Forge badge + clickable repo slug + Fetch button
                            egui::Sides::new().spacing(6.0).shrink_left().show(
                                ui,
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    kit::status_badge(ui, web.forge.label(), forge_c, true);
                                    let repo_lbl = ui.add(
                                        egui::Label::new(RichText::new(&web.repo).font(style::mono(11.0)).color(p.text))
                                            .truncate()
                                            .sense(Sense::click()),
                                    );
                                    if repo_lbl.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if repo_lbl
                                        .on_hover_text(tf("Open repository on {}", &[web.forge.label()]))
                                        .clicked()
                                    {
                                        open_url = Some(web.url());
                                    }
                                },
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if h.fetching {
                                        let (lr, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                                        kit::paint_lattice(ui.painter(), lr.center(), 10.0, p.info, true);
                                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                                    } else if kit::icon_button_ex(ui, icons::ARROWS_CLOCKWISE, t("Fetch from remote"), false, 22.0).clicked() {
                                        do_fetch = true;
                                    }
                                },
                            );

                            // Row 2: Quick Forge Actions (PRs/MRs, CI, sync counts, open PR/MR drawer)
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                                let prs_label = if is_gitlab { t("MRs") } else { t("PRs") };
                                let prs_tip = if is_gitlab {
                                    t("Open Merge Requests on GitLab")
                                } else {
                                    t("Open Pull Requests on GitHub")
                                };
                                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::GIT_PULL_REQUEST), prs_label, true)
                                    .on_hover_text(prs_tip)
                                    .clicked()
                                {
                                    open_url = Some(web.prs_url());
                                }
                                let ci_label = if is_gitlab { t("Pipelines") } else { t("Actions") };
                                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::LIGHTNING), ci_label, true)
                                    .on_hover_text(tf("Open {} on {}", &[ci_label, web.forge.label()]))
                                    .clicked()
                                {
                                    open_url = Some(web.actions_url());
                                }
                                if ahead > 0 || behind > 0 {
                                    let sync = format!("↑{ahead} ↓{behind}");
                                    ui.label(RichText::new(sync).font(style::mono(10.5)).color(if ahead > 0 { p.warn } else { p.text_faint }));
                                }
                                if !h.open_prs.is_empty() {
                                    let caret = if h.prs_expanded { icons::CARET_UP } else { icons::CARET_DOWN };
                                    let count_lbl = tf("{} open", &[&h.open_prs.len().to_string()]);
                                    if kit::button_ex(ui, kit::Kind::Secondary, Some(caret), &count_lbl, true).clicked() {
                                        toggle_prs = true;
                                    }
                                } else if let Some(pr) = &branch_pr {
                                    let badge = format!("{}{}", web.forge.request_prefix(), pr.number);
                                    if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::GIT_PULL_REQUEST), &badge, true).clicked() {
                                        open_url = Some(pr.url.clone());
                                    }
                                } else if !has_cli {
                                    let hint = tf("Install {} CLI for live list", &[web.forge.cli_name()]);
                                    ui.label(RichText::new(hint).size(10.0).color(p.text_ghost));
                                }
                            });

                            // Collapsible live open PRs / MRs list from `gh` or `glab`
                            if h.prs_expanded && !h.open_prs.is_empty() {
                                ui.add_space(2.0);
                                let sep = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover()).0;
                                ui.painter().hline(sep.x_range(), sep.center().y, Stroke::new(1.0, p.border));
                                let max_h = 132.0_f32.min(h.open_prs.len() as f32 * 26.0 + 4.0);
                                egui::ScrollArea::vertical()
                                    .id_salt(("history-open-prs", pid))
                                    .max_height(max_h)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        for pr in &h.open_prs {
                                            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                                            if resp.hovered() {
                                                ui.painter().rect_filled(rr, CornerRadius::same(5), p.hover);
                                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                            }
                                            let cy = rr.center().y;
                                            let mut rx = rr.left() + 6.0;
                                            let (c_icon, c_col) = match pr.checks {
                                                Checks::Passed => (icons::CHECK_CIRCLE, p.ok),
                                                Checks::Failed => (icons::X_CIRCLE, p.err),
                                                Checks::Pending => (icons::CIRCLE_NOTCH, p.info),
                                                Checks::None => (
                                                    icons::GIT_PULL_REQUEST,
                                                    if pr.state == PrState::Draft { p.text_faint } else { p.accent_text },
                                                ),
                                            };
                                            ui.painter().text(pos2(rx + 6.0, cy), Align2::CENTER_CENTER, c_icon, style::icon_font(12.0), c_col);
                                            rx += 16.0;
                                            let num = format!("{}{}", web.forge.request_prefix(), pr.number);
                                            let ng = ui.painter().layout_no_wrap(num, style::mono_medium(10.5), p.text_muted);
                                            let nw = ng.size().x;
                                            ui.painter().galley(pos2(rx, cy - ng.size().y / 2.0), ng, p.text_muted);
                                            rx += nw + 6.0;
                                            let mut tj = egui::text::LayoutJob::simple_singleline(pr.title.clone(), egui::FontId::proportional(11.5), p.text);
                                            tj.wrap = egui::text::TextWrapping::truncate_at_width((rr.right() - 6.0 - rx).max(24.0));
                                            let tg = ui.painter().layout_job(tj);
                                            ui.painter().galley(pos2(rx, cy - tg.size().y / 2.0), tg, p.text);
                                            let tip = format!("{} · {} ({})", pr.title, pr.branch, pr.author);
                                            if resp.on_hover_text(tip).clicked() {
                                                open_url = Some(pr.url.clone());
                                            }
                                        }
                                    });
                            }
                        } else {
                            // Local-only repository or non-GitHub/GitLab remote
                            egui::Sides::new().spacing(6.0).shrink_left().show(
                                ui,
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    ui.label(RichText::new(icons::GIT_BRANCH).font(style::icon_font(12.5)).color(p.text_faint));
                                    let label = remote
                                        .as_ref()
                                        .map(|r| r.name.clone())
                                        .unwrap_or_else(|| t("Local repository").to_string());
                                    ui.label(RichText::new(label).font(style::medium(11.5)).color(p.text_muted));
                                },
                                |ui| {
                                    if remote.is_some() {
                                        if h.fetching {
                                            let (lr, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                                            kit::paint_lattice(ui.painter(), lr.center(), 10.0, p.info, true);
                                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                                        } else if kit::icon_button_ex(ui, icons::ARROWS_CLOCKWISE, t("Fetch from remote"), false, 22.0).clicked() {
                                            do_fetch = true;
                                        }
                                    }
                                },
                            );
                        }
                    })
                    .response
                    .rect
                    .bottom()
            })
            .inner;
        y = card_bottom + 8.0;

        // 2. Commit Search / Filter Input
        let filter_rect = Rect::from_min_size(pos2(left_rect.left() + 8.0, y), vec2(left_rect.width() - 16.0, 26.0));
        let mut q = h.query.clone();
        let filter_resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(filter_rect), |ui| {
                kit::field(ui, egui::Id::new(("history-filter", pid)), &mut q, Some(icons::MAGNIFYING_GLASS), t("Filter commits…"), filter_rect.width(), true)
            })
            .inner;
        if q != h.query {
            let cleared = q.trim().is_empty() && !h.active_query.is_empty();
            new_query = Some(q.clone());
            if cleared {
                do_reload = true;
            }
        }
        if filter_resp.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
            do_reload = true;
        }
        y = filter_rect.bottom() + 8.0;

        // 3. Scrollable Commit Timeline List
        let list_rect = Rect::from_min_max(pos2(left_rect.left() + 6.0, y), pos2(left_rect.right() - 6.0, left_rect.bottom() - 4.0));
        let q_low = q.trim().to_ascii_lowercase();
        let visible_commits: Vec<usize> = h
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                q_low.is_empty()
                    || c.subject.to_ascii_lowercase().contains(&q_low)
                    || c.author.to_ascii_lowercase().contains(&q_low)
                    || c.hash.to_ascii_lowercase().starts_with(&q_low)
                    || c.refs.iter().any(|r| r.to_ascii_lowercase().contains(&q_low))
            })
            .map(|(i, _)| i)
            .collect();

        ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect);
            if h.loading && h.commits.is_empty() {
                ui.painter().text(list_rect.center(), Align2::CENTER_CENTER, t("Loading commit history…"), egui::FontId::proportional(12.5), p.text_muted);
            } else if let Some(err) = &h.error {
                ui.painter().text(list_rect.center(), Align2::CENTER_CENTER, err, egui::FontId::proportional(12.5), p.text_muted);
            } else if visible_commits.is_empty() {
                let msg = if h.commits.is_empty() { t("No commits yet.") } else { t("No matching commits.") };
                ui.painter().text(list_rect.center(), Align2::CENTER_CENTER, msg, egui::FontId::proportional(12.5), p.text_muted);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("history-commits", pid))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let count = visible_commits.len();
                        for (pos_i, &idx) in visible_commits.iter().enumerate() {
                            let c = &h.commits[idx];
                            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), COMMIT_ROW_H), Sense::click());
                            if !ui.is_rect_visible(row) {
                                continue;
                            }
                            let selected = idx == h.selected;
                            let painter = ui.painter_at(row);
                            if selected {
                                painter.rect(
                                    row.shrink2(vec2(0.0, 1.0)),
                                    CornerRadius::same(7),
                                    p.accent_tint,
                                    Stroke::new(1.0, style::soft(p.accent, 0.28)),
                                    StrokeKind::Inside,
                                );
                            } else if resp.hovered() {
                                painter.rect_filled(row.shrink2(vec2(0.0, 1.0)), CornerRadius::same(7), p.hover);
                            }
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            // Continuous vertical commit timeline rail + status dot
                            let dx = row.left() + 14.0;
                            let top = if pos_i == 0 { row.center().y } else { row.top() };
                            let bot = if pos_i + 1 == count && !h.has_more { row.center().y } else { row.bottom() };
                            painter.vline(dx, top..=bot, Stroke::new(1.0, p.border_heavy));
                            let is_head = c.refs.iter().any(|r| r.starts_with("HEAD"));
                            if is_head {
                                painter.circle_filled(pos2(dx, row.center().y), 4.5, p.accent);
                            } else if c.unpushed {
                                painter.circle(pos2(dx, row.center().y), 4.5, p.canvas, Stroke::new(2.0, p.warn));
                            } else {
                                painter.circle(pos2(dx, row.center().y), 4.0, p.canvas, Stroke::new(1.8, p.text_faint));
                            }

                            let tx = dx + 14.0;
                            let right_pad = row.right() - 8.0;

                            // Line 1: Commit subject + right-aligned relative time
                            let age = review::unix_age(c.time);
                            let ag = painter.layout_no_wrap(age, style::mono(10.0), p.text_ghost);
                            let aw = ag.size().x;
                            let top_y = row.top() + 8.0;
                            painter.galley(pos2(right_pad - aw, top_y + 1.0), ag, p.text_ghost);

                            let subj_w = (right_pad - aw - 8.0 - tx).max(20.0);
                            let mut sj = egui::text::LayoutJob::simple_singleline(
                                c.subject.clone(),
                                if selected { style::medium(12.0) } else { egui::FontId::proportional(12.0) },
                                if selected { p.text } else { p.text_muted },
                            );
                            sj.wrap = egui::text::TextWrapping::truncate_at_width(subj_w);
                            let sg = painter.layout_job(sj);
                            painter.galley(pos2(tx, top_y), sg, p.text);

                            // Line 2: short SHA + author + ref pills / unpushed marker
                            let bot_y = row.top() + 28.0;
                            let mut bx = tx;
                            let sha_g = painter.layout_no_wrap(c.short.clone(), style::mono(10.5), p.accent_text);
                            let sha_w = sha_g.size().x;
                            painter.galley(pos2(bx, bot_y + 1.0), sha_g, p.accent_text);
                            bx += sha_w + 6.0;

                            // Draw up to 2 concise ref pills (HEAD branch / tag / remote)
                            for raw_ref in c.refs.iter().take(2) {
                                let (lbl, col) = format_ref_pill(raw_ref, &p);
                                let rg = painter.layout_no_wrap(lbl, style::mono_medium(9.5), col);
                                let pw = rg.size().x + 8.0;
                                if bx + pw > right_pad - 24.0 {
                                    break;
                                }
                                let pr = Rect::from_min_size(pos2(bx, bot_y), vec2(pw, 15.0));
                                painter.rect_filled(pr, CornerRadius::same(4), style::tint(col, dark));
                                painter.galley(pos2(pr.left() + 4.0, pr.center().y - rg.size().y / 2.0), rg, col);
                                bx += pw + 4.0;
                            }

                            if c.unpushed && bx + 42.0 < right_pad {
                                let ug = painter.layout_no_wrap(t("local").to_string(), style::mono_medium(9.5), p.warn);
                                let pw = ug.size().x + 8.0;
                                let pr = Rect::from_min_size(pos2(bx, bot_y), vec2(pw, 15.0));
                                painter.rect_filled(pr, CornerRadius::same(4), style::tint(p.warn, dark));
                                painter.galley(pos2(pr.left() + 4.0, pr.center().y - ug.size().y / 2.0), ug, p.warn);
                                bx += pw + 4.0;
                            }

                            let author_w = (right_pad - bx).max(0.0);
                            if author_w > 24.0 && !c.author.is_empty() {
                                let mut aj = egui::text::LayoutJob::simple_singleline(c.author.clone(), egui::FontId::proportional(10.5), p.text_faint);
                                aj.wrap = egui::text::TextWrapping::truncate_at_width(author_w);
                                let ag = painter.layout_job(aj);
                                painter.galley(pos2(bx, bot_y + 1.0), ag, p.text_faint);
                            }

                            if resp.clicked() && !selected {
                                select_commit = Some(idx);
                            }
                        }

                        if h.has_more {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.add_space(24.0);
                                if h.loading_more {
                                    ui.label(RichText::new(t("Loading more commits…")).size(11.5).color(p.text_faint));
                                } else if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CARET_DOWN), t("Load more commits"), true).clicked() {
                                    do_load_more = true;
                                }
                            });
                            ui.add_space(8.0);
                        }
                    });
            }
        });

        // ===== RIGHT PANE: Commit Metadata + Changed Files + Syntax-Highlighted Diff =====
        let Some(commit) = h.commits.get(h.selected) else {
            if !h.loading && h.error.is_none() {
                ui.painter().text(
                    right_rect.center(),
                    Align2::CENTER_CENTER,
                    t("Select a commit to inspect its changes."),
                    egui::FontId::proportional(13.0),
                    p.text_muted,
                );
            }
            self.apply_history_actions(
                do_fetch,
                do_reload,
                do_load_more,
                select_commit,
                select_file,
                toggle_prs,
                toggle_split,
                open_url,
                open_file,
                copy_sha,
                new_query,
            );
            return;
        };

        // 1. Commit Metadata Header Card
        let head_area = Rect::from_min_max(right_rect.min, pos2(right_rect.right(), right_rect.bottom()));
        let head_bottom = ui
            .scope_builder(egui::UiBuilder::new().max_rect(head_area), |ui| {
                ui.set_clip_rect(head_area);
                egui::Frame::NONE
                    .fill(style::over(p.canvas, p.chrome, 0.22))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width((head_area.width() - 28.0).max(160.0));
                        ui.spacing_mut().item_spacing.y = 6.0;

                        // Subject + Copy SHA + Open on GitHub/GitLab
                        egui::Sides::new().spacing(10.0).shrink_left().wrap().show(
                            ui,
                            |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&commit.subject).font(style::semibold(13.5)).color(p.text))
                                        .wrap(),
                                );
                            },
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if let Some(web) = &web {
                                    let btn_lbl = web.forge.label();
                                    let tip = tf("Open commit on {}", &[web.forge.label()]);
                                    if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), btn_lbl, true)
                                        .on_hover_text(tip)
                                        .clicked()
                                    {
                                        open_url = Some(web.commit_url(&commit.hash));
                                    }
                                }
                                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::COPY), &commit.short, true)
                                    .on_hover_text(t("Copy full commit hash"))
                                    .clicked()
                                {
                                    copy_sha = Some(commit.hash.clone());
                                }
                            },
                        );

                        if !commit.body.is_empty() {
                            let body_preview: String = commit.body.lines().take(4).collect::<Vec<_>>().join("\n");
                            ui.add(
                                egui::Label::new(RichText::new(body_preview).size(11.5).color(p.text_muted))
                                    .wrap(),
                            );
                        }

                        // Metadata strip: author, date, refs, unpushed badge
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                            if !commit.author.is_empty() {
                                ui.label(RichText::new(&commit.author).font(style::medium(11.5)).color(p.text_muted));
                            }
                            if commit.time > 0 {
                                let formatted = format_commit_time(commit.time);
                                ui.label(RichText::new(formatted).font(style::mono(10.5)).color(p.text_faint));
                            }
                            for raw_ref in &commit.refs {
                                let (lbl, col) = format_ref_pill(raw_ref, &p);
                                kit::status_badge(ui, &lbl, col, true);
                            }
                            if commit.unpushed {
                                kit::status_badge(ui, t("Unpushed"), p.warn, false);
                            }
                        });
                    })
                    .response
                    .rect
                    .bottom()
            })
            .inner;
        ui.painter().hline(right_rect.x_range(), head_bottom + 0.5, Stroke::new(1.0, p.border));

        // 2. Changed Files Strip + Diff Area
        let content_top = head_bottom + 1.0;
        let content_rect = Rect::from_min_max(pos2(right_rect.left(), content_top), right_rect.max);
        let (total_adds, total_dels): (u32, u32) = h.files.iter().fold((0, 0), |(a, d), f| {
            (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0))
        });

        let bar_h = 34.0;
        let bar_rect = Rect::from_min_size(content_rect.min, vec2(content_rect.width(), bar_h));
        ui.painter().hline(bar_rect.x_range(), bar_rect.bottom() - 0.5, Stroke::new(1.0, p.border));

        let split_ok = content_rect.width() >= SPLIT_MIN_W;
        let split = h.split && split_ok;
        let selected_f = h.files.get(h.selected_file);

        // Files header bar: count + total adds/dels on left, Unified|Split & Open on right
        let bar_inner = Rect::from_min_max(pos2(bar_rect.left() + 12.0, bar_rect.top()), pos2(bar_rect.right() - 10.0, bar_rect.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar_inner), |ui| {
            egui::Sides::new().spacing(8.0).shrink_left().show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if h.files_loading {
                        ui.label(RichText::new(t("Loading changed files…")).size(11.5).color(p.text_faint));
                    } else {
                        let count_str = crate::i18n::count(h.files.len(), t("1 file changed"), t("{} files changed"));
                        ui.label(RichText::new(count_str).font(style::medium(11.5)).color(p.text_muted));
                        if total_adds > 0 {
                            ui.label(RichText::new(format!("+{total_adds}")).font(style::mono(11.0)).color(p.ok));
                        }
                        if total_dels > 0 {
                            ui.label(RichText::new(format!("−{total_dels}")).font(style::mono(11.0)).color(p.err));
                        }
                    }
                },
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if let Some(f) = selected_f
                        && f.kind != ChangeKind::Deleted
                        && kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open"), true).clicked()
                    {
                        open_file = Some(h.root.join(&f.path));
                    }
                    if split_ok {
                        let options = [kit::Seg::new(t("Unified"), ""), kit::Seg::new(t("Split"), "")];
                        let w = kit::segmented_width(ui, &options);
                        let (sr, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                        if let Some(i) = kit::segmented_in(ui, sr, egui::Id::new("history-split"), &options, h.split as usize) {
                            toggle_split = Some(i == 1);
                        }
                    }
                },
            );
        });

        let avail_h = (content_rect.bottom() - bar_rect.bottom()).max(0.0);
        let files_list_h = if h.files.is_empty() {
            0.0
        } else {
            let natural = h.files.len() as f32 * FILE_ROW_H + 8.0;
            natural.min((avail_h * 0.28).clamp(68.0, 172.0))
        };
        let files_rect = Rect::from_min_size(pos2(content_rect.left(), bar_rect.bottom()), vec2(content_rect.width(), files_list_h));
        let diff_rect = Rect::from_min_max(pos2(content_rect.left(), files_rect.bottom()), content_rect.max);

        if files_list_h > 0.0 {
            let inner = files_rect.shrink2(vec2(6.0, 4.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.set_clip_rect(inner);
                egui::ScrollArea::vertical()
                    .id_salt(("history-commit-files", pid, h.selected))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (fi, f) in h.files.iter().enumerate() {
                            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
                            if !ui.is_rect_visible(rr) {
                                continue;
                            }
                            let on = fi == h.selected_file;
                            let painter = ui.painter_at(rr);
                            if on {
                                painter.rect_filled(rr.shrink2(vec2(0.0, 1.0)), CornerRadius::same(6), p.accent_tint);
                            } else if resp.hovered() {
                                painter.rect_filled(rr.shrink2(vec2(0.0, 1.0)), CornerRadius::same(6), p.hover);
                            }
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let cy = rr.center().y;
                            let (letter, lcol) = review::change_letter(f.kind, &p);
                            painter.text(pos2(rr.left() + 14.0, cy), Align2::CENTER_CENTER, letter, style::mono_semibold(10.5), lcol);

                            let a = f.additions.unwrap_or(0);
                            let d = f.deletions.unwrap_or(0);
                            let mut cjob = egui::text::LayoutJob::default();
                            if a > 0 {
                                cjob.append(&format!("+{a}"), 0.0, egui::TextFormat::simple(style::mono(10.5), p.ok));
                            }
                            if d > 0 {
                                cjob.append(&format!("−{d}"), if a > 0 { 6.0 } else { 0.0 }, egui::TextFormat::simple(style::mono(10.5), p.err));
                            }
                            let cg = painter.layout_job(cjob);
                            let cw = cg.size().x;
                            let bars_w = if f.binary { 0.0 } else { review::BARS_W + 10.0 };
                            let right_x = rr.right() - 10.0;
                            if !f.binary {
                                review::paint_bars(&painter, pos2(right_x - review::BARS_W, cy), a, d, &p);
                            }
                            painter.galley(pos2(right_x - bars_w - cw, cy - cg.size().y / 2.0), cg, p.text_faint);

                            let path_label = match &f.old_path {
                                Some(old) => format!("{old} → {}", f.path),
                                None => f.path.clone(),
                            };
                            let mut pj = egui::text::LayoutJob::simple_singleline(
                                path_label,
                                if on { style::medium(12.0) } else { egui::FontId::proportional(12.0) },
                                if on { p.text } else { p.text_muted },
                            );
                            pj.wrap = egui::text::TextWrapping::truncate_at_width((right_x - bars_w - cw - 12.0 - (rr.left() + 26.0)).max(24.0));
                            let pg = painter.layout_job(pj);
                            painter.galley(pos2(rr.left() + 26.0, cy - pg.size().y / 2.0), pg, p.text);

                            if resp.clicked() && !on {
                                select_file = Some(fi);
                            }
                        }
                    });
            });
            ui.painter().hline(files_rect.x_range(), files_rect.bottom() - 0.5, Stroke::new(1.0, p.border));
        }

        // 3. Syntax-Highlighted Diff Viewer
        if let Some(err) = &h.files_error {
            ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, err, egui::FontId::proportional(12.5), p.err);
        } else if h.files_loading {
            ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, t("Loading the diff…"), egui::FontId::proportional(12.5), p.text_muted);
        } else if h.files.is_empty() {
            ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, t("No file changes in this commit."), egui::FontId::proportional(12.5), p.text_muted);
        } else if let Some(f) = selected_f {
            let syntax = crate::editor::syntax_for(std::path::Path::new(&f.path));
            let look = review::LineLook {
                h: if split { 21.0 } else { 20.0 },
                font: if split { 11.5 } else { 11.0 },
                num_w: 36.0,
            };
            match (&h.diff, &h.diff_error) {
                (_, Some(e)) => {
                    ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, e, egui::FontId::proportional(12.5), p.err);
                }
                (None, None) => {
                    ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, t("Loading the diff…"), egui::FontId::proportional(12.5), p.text_muted);
                }
                (Some(d), None) if d.binary => {
                    ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, t("Binary file: no text diff."), egui::FontId::proportional(12.5), p.text_muted);
                }
                (Some(d), None) => {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(diff_rect), |ui| {
                        ui.set_clip_rect(diff_rect);
                        egui::ScrollArea::both()
                            .id_salt(("history-diff", pid, h.selected, h.selected_file))
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let w = ui.available_width().max(diff_rect.width());
                                for (hi, hunk) in d.hunks.iter().enumerate() {
                                    let (hr, _) = ui.allocate_exact_size(vec2(w, review::HUNK_H), Sense::hover());
                                    let _ = review::paint_hunk_header(ui, hr, &hunk.header, &p, hi == 0, None);
                                    if split {
                                        review::paint_split(ui, w, &hunk.lines, &syntax, dark, &p, &look);
                                    } else {
                                        let pairs = review::word_pairs(&hunk.lines);
                                        for (li, line) in hunk.lines.iter().enumerate() {
                                            let (lr, _) = ui.allocate_exact_size(vec2(w, look.h), Sense::hover());
                                            if ui.is_rect_visible(lr) {
                                                let hl = pairs[li].and_then(|o| review::changed_part(&line.text, &hunk.lines[o].text));
                                                review::paint_line(ui, lr, line.kind, line.old_no, line.new_no, &line.text, &syntax, dark, &p, true, hl, &look);
                                            }
                                        }
                                    }
                                }
                                ui.add_space(12.0);
                            });
                    });
                }
            }
        }

        self.apply_history_actions(
            do_fetch,
            do_reload,
            do_load_more,
            select_commit,
            select_file,
            toggle_prs,
            toggle_split,
            open_url,
            open_file,
            copy_sha,
            new_query,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_history_actions(
        &mut self,
        do_fetch: bool,
        do_reload: bool,
        do_load_more: bool,
        select_commit: Option<usize>,
        select_file: Option<usize>,
        toggle_prs: bool,
        toggle_split: Option<bool>,
        open_url: Option<String>,
        open_file: Option<std::path::PathBuf>,
        copy_sha: Option<String>,
        new_query: Option<String>,
    ) {
        if let Some(q) = new_query
            && let Some(h) = self.history.as_mut()
        {
            h.query = q;
        }
        if toggle_prs
            && let Some(h) = self.history.as_mut()
        {
            h.prs_expanded = !h.prs_expanded;
        }
        if let Some(s) = toggle_split
            && let Some(h) = self.history.as_mut()
        {
            h.split = s;
        }
        if do_fetch {
            self.history_fetch();
        }
        if do_reload {
            self.history_reload(false);
        }
        if do_load_more {
            self.history_reload(true);
        }
        if let Some(idx) = select_commit {
            self.history_select_commit(idx);
        }
        if let Some(idx) = select_file {
            self.history_select_file(idx);
        }
        if let Some(url) = open_url {
            self.actions.push(Action::OpenWeb(url));
        }
        if let Some(path) = open_file {
            self.history = None;
            self.actions.push(Action::OpenFile(path, None));
        }
        if let Some(sha) = copy_sha {
            self.ctx.copy_text(sha);
            self.toast(t("Copied.").to_string());
        }
    }
}

/// Formats one git ref label (`HEAD -> main`, `origin/main`, `tag: v0.2.3`) into concise pill text + color.
fn format_ref_pill(raw: &str, p: &style::Palette) -> (String, Color32) {
    if let Some(branch) = raw.strip_prefix("HEAD -> ") {
        (branch.to_string(), p.accent_text)
    } else if let Some(tag) = raw.strip_prefix("tag: ") {
        (tag.to_string(), p.warn)
    } else if raw.contains('/') {
        (raw.to_string(), p.info)
    } else {
        (raw.to_string(), p.text_muted)
    }
}

/// Formats a Unix commit timestamp into local `YYYY-MM-DD HH:MM · <age>`.
fn format_commit_time(unix: i64) -> String {
    let age = review::unix_age(unix);
    if let Some(dt) = chrono::DateTime::from_timestamp(unix, 0) {
        let local = dt.with_timezone(&chrono::Local);
        format!("{} · {age}", local.format("%Y-%m-%d %H:%M"))
    } else {
        age
    }
}
