//! Automations view & Create/Edit modal (Slate design system).
//!
//! Master-detail workspace for scheduled background agent runs:
//! - Left column: grouped automation list (Running & Needs review, Scheduled, Paused) + Templates launcher.
//! - Right column: 4-cell summary card + 3 tabs (`Runs & live session`, `Configuration & prompt`, `Templates`).
//! - Fixed-size centered modal supporting both `Configure manually` and `Create with prompt` (NL draft).

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use tc_core::agent::AgentKind;
use tc_core::model::View;

use super::{dialogs, focus, icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::automations::{
    Automation, AutomationDetailTab, AutomationDraft, AutomationFilter, AutomationRun, AutomationStatus, CadenceKind,
    MissedPolicy, OverlapPolicy, RunStatus, RunTrigger, STARTER_TEMPLATES, SessionMode, WorkspaceMode,
    automation_agent_label, format_countdown, format_local_short, now_epoch_secs,
};
use crate::i18n::{t, tf};

const LIST_W: f32 = 348.0;
const FOOT_H: f32 = 38.0;
const ROW_H: f32 = 66.0;
const ROW_GAP: f32 = 2.0;

impl App {
    /// Where Automations goes back to on Escape or toggling `⌥⌘A` again.
    pub fn automations_back(&self) -> View {
        if crate::ade::is_management_view(self.prev_view) || self.prev_view == View::Automations {
            View::Focus
        } else {
            self.prev_view
        }
    }

    fn automations_keys_free(&self, ui: &Ui) -> bool {
        self.dialog.is_none()
            && self.automation_draft.is_none()
            && self.palette.is_none()
            && !self.updater.dialog_open
            && !self.monitor_popover
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.ctx().egui_wants_keyboard_input()
    }

    /// Opens the New Automation modal, optionally prefilled from starter template `template_ix`.
    pub fn open_new_automation_modal(&mut self, template_ix: Option<usize>) {
        let (path, name) = self.default_automation_project();
        let mut draft = AutomationDraft::new_for(path, name);
        if let Some(ix) = template_ix
            && let Some(tpl) = STARTER_TEMPLATES.get(ix)
        {
            draft.apply_template(tpl);
        }
        self.automation_modal_err = None;
        self.automation_modal_seq = self.automation_modal_seq.wrapping_add(1);
        self.automation_draft = Some(draft);
    }

    fn default_automation_project(&self) -> (std::path::PathBuf, String) {
        let root = self
            .active
            .and_then(|id| self.root_project(id))
            .or_else(|| self.projects.iter().find(|p| !p.is_worktree()))
            .or_else(|| self.projects.first());
        match root {
            Some(p) => (p.path.clone(), p.name.clone()),
            None => (std::path::PathBuf::new(), String::new()),
        }
    }

    /// Top bar right-side controls when `View::Automations` is active (laid out right-to-left).
    pub(super) fn automations_toolbar(&mut self, ui: &mut Ui) {
        let new_btn = kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLUS), t("New automation"), false);
        #[cfg(debug_assertions)]
        crate::devtest::record("auto-new", new_btn.rect);
        if new_btn.on_hover_text(t("Create a scheduled background automation (N)")).clicked() {
            self.open_new_automation_modal(None);
        }

        ui.add_space(6.0);
        let tpl_active = self.automation_tab == AutomationDetailTab::Templates;
        let tpl_btn = kit::button_full(ui, kit::Kind::Ghost, Some(icons::SPARKLE), t("Templates"), false, tpl_active);
        #[cfg(debug_assertions)]
        crate::devtest::record("auto-templates-btn", tpl_btn.rect);
        if tpl_btn.on_hover_text(t("Browse ready-to-use automation templates")).clicked() {
            self.automation_tab = if tpl_active {
                AutomationDetailTab::Runs
            } else {
                AutomationDetailTab::Templates
            };
        }

        if self.automations.is_empty() {
            return;
        }

        ui.add_space(10.0);
        let total = self.automations.len();
        let active_n = self.automations.iter().filter(|a| a.enabled).count();
        let review_n = self
            .automations
            .iter()
            .filter(|a| {
                matches!(
                    a.status(self.active_automation_runs.contains_key(&a.id)),
                    AutomationStatus::NeedsReview | AutomationStatus::Failed
                )
            })
            .count();
        let paused_n = self.automations.iter().filter(|a| !a.enabled).count();

        let labels = [
            format!("{} {total}", t("All")),
            format!("{} {active_n}", t("Active")),
            format!("{} {review_n}", t("Needs review")),
            format!("{} {paused_n}", t("Paused")),
        ];
        let options = [
            kit::Seg::new(&labels[0], t("Show all automations")),
            kit::Seg::new(&labels[1], t("Enabled automations")),
            kit::Seg::new(&labels[2], t("Automations with unreviewed findings or failures")),
            kit::Seg::new(&labels[3], t("Paused automations")),
        ];
        let filters = [
            AutomationFilter::All,
            AutomationFilter::Active,
            AutomationFilter::NeedsReview,
            AutomationFilter::Paused,
        ];
        let on = filters.iter().position(|f| *f == self.automation_filter).unwrap_or(0);
        let r = ui.push_id("auto-filter", |ui| kit::segmented(ui, &options, on));
        #[cfg(debug_assertions)]
        for (rect, name) in super::grid::word_segments(ui, r.response.rect, &labels)
            .into_iter()
            .zip(["auto-filter:all", "auto-filter:active", "auto-filter:review", "auto-filter:paused"])
        {
            crate::devtest::record(name, rect);
        }
        if let Some(i) = r.inner {
            self.automation_filter = filters[i];
        }
    }

    /// Main Automations view (top bar + Slate sheet).
    pub fn automations_view(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let full = ui.max_rect();

        let running_n = self.running_automations_count();
        let review_n = self.unreviewed_automation_alerts_count();
        let active_n = self.automations.iter().filter(|a| a.enabled).count();
        let subtitle = if self.automations.is_empty() {
            t("Scheduled background runs in isolated worktrees").to_string()
        } else {
            let mut parts = vec![tf("{} active", &[&active_n.to_string()])];
            if running_n > 0 {
                parts.push(tf("{} running", &[&running_n.to_string()]));
            }
            if review_n > 0 {
                parts.push(tf("{} needs review", &[&review_n.to_string()]));
            }
            parts.join(" · ")
        };

        let body = self.top_bar(
            ui,
            full,
            |_, ui| {
                ui.add(
                    egui::Label::new(RichText::new(t("Automations")).font(style::semibold(style::TITLE)).color(p.text))
                        .selectable(false),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(subtitle).font(FontId::proportional(style::SMALL)).color(p.text_faint),
                    )
                    .selectable(false),
                );
            },
            None,
        );

        let sheet = focus::sheets_area(body, self.settings.sidebar_hidden);
        #[cfg(debug_assertions)]
        crate::devtest::record("automations", sheet);
        focus::paint_sheet(ui.painter(), sheet, &p, focus::SheetLook::Plain);

        let keys_free = self.automations_keys_free(ui);

        // Filtered ordered list of automation IDs.
        let ordered_ids = self.filtered_automation_ids();
        if self.automation_sel.as_ref().is_none_or(|id| !ordered_ids.contains(id)) {
            self.automation_sel = ordered_ids.first().cloned().or_else(|| self.automations.first().map(|a| a.id.clone()));
            self.automation_run_ix = 0;
        }

        // Keyboard shortcuts when no modal/field owns focus.
        let mut back = false;
        let mut open_new = false;
        let mut key_run_id: Option<String> = None;
        let mut key_toggle_id: Option<String> = None;
        let mut key_open_run: Option<(String, String)> = None;

        if keys_free {
            let cur_ix = self
                .automation_sel
                .as_ref()
                .and_then(|id| ordered_ids.iter().position(|x| x == id))
                .unwrap_or(0);
            let cur_id = ordered_ids.get(cur_ix).cloned();
            ui.input_mut(|inp| {
                use egui::Modifiers as M;
                if !ordered_ids.is_empty() && inp.consume_key(M::NONE, Key::ArrowDown) {
                    let next = (cur_ix + 1).min(ordered_ids.len() - 1);
                    self.automation_sel = Some(ordered_ids[next].clone());
                    self.automation_run_ix = 0;
                } else if !ordered_ids.is_empty() && inp.consume_key(M::NONE, Key::ArrowUp) {
                    let prev = cur_ix.saturating_sub(1);
                    self.automation_sel = Some(ordered_ids[prev].clone());
                    self.automation_run_ix = 0;
                } else if inp.consume_key(M::NONE, Key::N) {
                    open_new = true;
                } else if let Some(id) = cur_id.as_ref()
                    && inp.consume_key(M::NONE, Key::R)
                {
                    key_run_id = Some(id.clone());
                } else if let Some(id) = cur_id.as_ref()
                    && inp.consume_key(M::NONE, Key::Space)
                {
                    key_toggle_id = Some(id.clone());
                } else if let Some(id) = cur_id.as_ref()
                    && inp.consume_key(M::NONE, Key::O)
                    && let Some(run_id) = self
                        .automations
                        .iter()
                        .find(|a| a.id == *id)
                        .and_then(|a| a.runs.get(self.automation_run_ix).or_else(|| a.runs.first()))
                        .map(|r| r.id.clone())
                {
                    key_open_run = Some((id.clone(), run_id));
                } else if inp.consume_key(M::NONE, Key::Escape) {
                    back = true;
                }
            });
        }

        let foot = Rect::from_min_max(pos2(sheet.left(), sheet.bottom() - FOOT_H), sheet.max);
        let main = Rect::from_min_max(sheet.min, pos2(sheet.right(), foot.top()));

        if self.automations.is_empty() {
            self.automations_empty_canvas(ui, main);
        } else {
            let with_detail = main.width() >= 720.0;
            let list_w = if with_detail {
                LIST_W.min((main.width() * 0.34).max(276.0))
            } else {
                main.width()
            };
            let list_rect = Rect::from_min_max(main.min, pos2(main.left() + list_w, main.bottom()));
            if with_detail {
                ui.painter().vline(list_rect.right() + 0.5, main.y_range(), Stroke::new(1.0, p.border));
            }
            self.automations_list_pane(ui, list_rect, &ordered_ids);
            if with_detail {
                let detail_rect = Rect::from_min_max(pos2(list_rect.right() + 1.0, main.top()), main.max);
                self.automations_detail_pane(ui, detail_rect);
            }
        }

        // Bottom keyboard hints strip.
        ui.painter().hline(sheet.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
        self.automations_footer_hints(ui, foot);

        // Apply deferred keyboard actions.
        if open_new {
            self.open_new_automation_modal(None);
        }
        if let Some(id) = key_run_id {
            if self.active_automation_runs.contains_key(&id) {
                self.stop_automation_run(&id);
            } else {
                self.start_automation_run(&id, RunTrigger::Manual);
            }
        }
        if let Some(id) = key_toggle_id {
            self.toggle_automation_enabled(&id);
        }
        if let Some((aid, rid)) = key_open_run {
            let ctx = ui.ctx().clone();
            self.open_automation_run_workspace(&ctx, &aid, &rid, false);
        }
        if back {
            self.actions.push(Action::SetView(self.automations_back()));
        }
    }

    fn filtered_automation_ids(&self) -> Vec<String> {
        let mut attention = Vec::new();
        let mut scheduled = Vec::new();
        let mut paused = Vec::new();
        for a in &self.automations {
            let running = self.active_automation_runs.contains_key(&a.id);
            let st = a.status(running);
            let keep = match self.automation_filter {
                AutomationFilter::All => true,
                AutomationFilter::Active => a.enabled,
                AutomationFilter::NeedsReview => matches!(st, AutomationStatus::NeedsReview | AutomationStatus::Failed),
                AutomationFilter::Paused => !a.enabled,
            };
            if !keep {
                continue;
            }
            match st {
                AutomationStatus::Running | AutomationStatus::NeedsReview | AutomationStatus::Failed => {
                    attention.push(a.id.clone())
                }
                AutomationStatus::Paused => paused.push(a.id.clone()),
                AutomationStatus::Clean => scheduled.push(a.id.clone()),
            }
        }
        attention.extend(scheduled);
        attention.extend(paused);
        attention
    }

    /// Empty state with starter templates gallery so first-time users can launch immediately.
    fn automations_empty_canvas(&mut self, ui: &mut Ui, area: Rect) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let mut open_blank = false;
        let mut open_nl = false;
        let mut use_tpl: Option<usize> = None;

        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.set_clip_rect(area.shrink(1.0).intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt("auto-empty-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let avail = ui.available_width().max(480.0);
                    let w = (avail - 64.0).clamp(440.0, 920.0);
                    let pad_x = ((avail - w) * 0.5).max(20.0);

                    egui::Frame::NONE
                        .inner_margin(Margin {
                            left: pad_x.round() as i8,
                            right: pad_x.round() as i8,
                            top: 28,
                            bottom: 24,
                        })
                        .show(ui, |ui| {
                            ui.set_width(w);
                            // Hero card.
                            let hero = egui::Frame::NONE
                                .fill(p.surface)
                                .stroke(Stroke::new(1.0, p.border))
                                .corner_radius(CornerRadius::same(style::RADIUS_CARD))
                                .inner_margin(Margin::same(22));
                            hero.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 14.0;
                                    let (tile, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                                    ui.painter().rect_filled(tile, CornerRadius::same(11), p.accent_tint);
                                    ui.painter().text(
                                        tile.center(),
                                        Align2::CENTER_CENTER,
                                        icons::CLOCK,
                                        style::icon_font(22.0),
                                        p.accent_text,
                                    );
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        ui.label(
                                            RichText::new(t("Put agents on autopilot, safely"))
                                                .font(style::semibold(style::TITLE))
                                                .color(p.text),
                                        );
                                        ui.label(
                                            RichText::new(t(
                                                "Each scheduled run spins up a fresh session in an isolated Git worktree, stays quiet when everything is clean, and lands in your Inbox only when diffs or failures need review.",
                                            ))
                                            .font(FontId::proportional(style::BODY))
                                            .color(p.text_muted),
                                        );
                                        ui.add_space(8.0);
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 8.0;
                                            let b1 = kit::button_ex(
                                                ui,
                                                kit::Kind::Primary,
                                                Some(icons::PLUS),
                                                t("New automation"),
                                                false,
                                            );
                                            #[cfg(debug_assertions)]
                                            crate::devtest::record("auto-empty-new", b1.rect);
                                            if b1.clicked() {
                                                open_blank = true;
                                            }
                                            let b2 = kit::button_ex(
                                                ui,
                                                kit::Kind::Secondary,
                                                Some(icons::SPARKLE),
                                                t("Create with prompt"),
                                                false,
                                            );
                                            #[cfg(debug_assertions)]
                                            crate::devtest::record("auto-empty-nl", b2.rect);
                                            if b2.clicked() {
                                                open_nl = true;
                                            }
                                            if self.projects.is_empty() {
                                                ui.add_space(6.0);
                                                ui.label(
                                                    RichText::new(t("Add a project first to schedule runs."))
                                                        .font(FontId::proportional(style::SMALL))
                                                        .color(p.warn),
                                                );
                                            }
                                        });
                                    });
                                });
                            });

                            ui.add_space(22.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(t("Starter templates"))
                                        .font(style::semibold(style::HEADING))
                                        .color(p.text),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(t("Click any template to customize and run"))
                                        .font(FontId::proportional(style::SMALL))
                                        .color(p.text_faint),
                                );
                            });
                            ui.add_space(10.0);

                            if let Some(ix) = draw_templates_grid(ui, w, &p, dark) {
                                use_tpl = Some(ix);
                            }
                        });
                });
        });

        if open_blank {
            self.open_new_automation_modal(None);
        } else if open_nl {
            self.open_new_automation_modal(None);
            if let Some(d) = self.automation_draft.as_mut() {
                d.nl_mode = true;
            }
        } else if let Some(ix) = use_tpl {
            self.open_new_automation_modal(Some(ix));
        }
    }

    /// Left column: grouped automation list + pinned starter templates footer.
    fn automations_list_pane(&mut self, ui: &mut Ui, rect: Rect, ordered_ids: &[String]) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let now = now_epoch_secs();
        let bottom_h = 44.0;
        let list_area = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - bottom_h));
        let foot_area = Rect::from_min_max(pos2(rect.left(), list_area.bottom()), rect.max);

        // Snapshot rows to avoid borrowing self while drawing.
        struct RowData {
            id: String,
            name: String,
            project: String,
            agent: AgentKind,
            cadence: String,
            enabled: bool,
            status: AutomationStatus,
            next_label: String,
            files_changed: usize,
            group: u8, // 0 = attention/running, 1 = scheduled, 2 = paused
        }
        let rows: Vec<RowData> = ordered_ids
            .iter()
            .filter_map(|id| {
                let a = self.automations.iter().find(|x| &x.id == id)?;
                let running = self.active_automation_runs.contains_key(&a.id);
                let st = a.status(running);
                let group = match st {
                    AutomationStatus::Running | AutomationStatus::NeedsReview | AutomationStatus::Failed => 0,
                    AutomationStatus::Paused => 2,
                    _ => 1,
                };
                let next_label = match st {
                    AutomationStatus::Running => t("Running…").to_string(),
                    AutomationStatus::NeedsReview => t("Needs review").to_string(),
                    AutomationStatus::Failed => t("Failed").to_string(),
                    AutomationStatus::Paused => t("Paused").to_string(),
                    _ => a
                        .next_run_at
                        .map(|n| format_countdown(n, now))
                        .unwrap_or_else(|| t("Manual").to_string()),
                };
                let files_changed = a.latest_run().map_or(0, |r| r.files_changed);
                Some(RowData {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    project: a.project_name.clone(),
                    agent: a.agent,
                    cadence: a.schedule.short_label(),
                    enabled: a.enabled,
                    status: st,
                    next_label,
                    files_changed,
                    group,
                })
            })
            .collect();

        let mut pick_id: Option<String> = None;
        let mut quick_run: Option<String> = None;
        let mut quick_stop: Option<String> = None;

        ui.scope_builder(egui::UiBuilder::new().max_rect(list_area), |ui| {
            ui.set_clip_rect(list_area.shrink(1.0).intersect(ui.clip_rect()));
            if rows.is_empty() {
                kit::empty_state_at(
                    ui,
                    list_area,
                    icons::FUNNEL,
                    t("No automations match"),
                    t("Switch the filter above to see all automations."),
                    None,
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("auto-list-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = ROW_GAP;
                    egui::Frame::NONE.inner_margin(Margin::symmetric(8, 8)).show(ui, |ui| {
                        let mut last_group: Option<u8> = None;
                        for (ix, row) in rows.iter().enumerate() {
                            let _ = ix;
                            if last_group != Some(row.group) {
                                if last_group.is_some() {
                                    ui.add_space(6.0);
                                }
                                let g_title = match row.group {
                                    0 => t("RUNNING & NEEDS REVIEW"),
                                    1 => t("SCHEDULED"),
                                    _ => t("PAUSED"),
                                };
                                let (gr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                                ui.painter().text(
                                    pos2(gr.left() + 8.0, gr.center().y),
                                    Align2::LEFT_CENTER,
                                    g_title,
                                    style::semibold(10.5),
                                    p.text_faint,
                                );
                                last_group = Some(row.group);
                            }

                            let selected = self.automation_sel.as_deref() == Some(row.id.as_str());
                            let (r, resp) =
                                ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                            #[cfg(debug_assertions)]
                            {
                                crate::devtest::record(format!("auto-row:{}", row.id), r);
                                crate::devtest::record(format!("auto-row-ix:{ix}"), r);
                            }
                            let hovered = resp.hovered();
                            if hovered {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let painter = ui.painter_at(r);
                            let fill = if selected {
                                p.subtle
                            } else if hovered {
                                p.hover
                            } else {
                                Color32::TRANSPARENT
                            };
                            if fill != Color32::TRANSPARENT {
                                painter.rect_filled(r, CornerRadius::same(8), fill);
                            }
                            if selected {
                                painter.rect_stroke(
                                    r,
                                    CornerRadius::same(8),
                                    Stroke::new(1.0, p.border_strong),
                                    StrokeKind::Inside,
                                );
                            }

                            // Status indicator dot / live lattice.
                            let dot_at = pos2(r.left() + 14.0, r.top() + 22.0);
                            match row.status {
                                AutomationStatus::Running => {
                                    kit::paint_lattice(&painter, dot_at, 9.0, p.info, true);
                                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
                                }
                                AutomationStatus::NeedsReview => {
                                    painter.circle_filled(dot_at, 4.0, p.warn);
                                }
                                AutomationStatus::Failed => {
                                    painter.circle_filled(dot_at, 4.0, p.err);
                                }
                                AutomationStatus::Clean => {
                                    painter.circle_filled(dot_at, 3.5, p.ok);
                                }
                                AutomationStatus::Paused => {
                                    painter.circle_stroke(dot_at, 3.5, Stroke::new(1.3, p.text_ghost));
                                }
                            }

                            // Quick play/stop button at top right (visible on hover or when running).
                            let btn_r = Rect::from_center_size(pos2(r.right() - 18.0, r.top() + 21.0), vec2(24.0, 24.0));
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("auto-row-run:{ix}"), btn_r);
                            let is_running = row.status == AutomationStatus::Running;
                            let show_quick = hovered || selected || is_running;
                            let mut quick_clicked = false;
                            if show_quick {
                                let qb = ui.interact(btn_r, ui.id().with(("auto-qrun", &row.id)), Sense::click());
                                if qb.hovered() {
                                    painter.rect_filled(btn_r, CornerRadius::same(6), p.hover);
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                let (qicon, qcol) = if is_running {
                                    (icons::STOP, p.err)
                                } else {
                                    (icons::PLAY, if qb.hovered() { p.accent_text } else { p.text_muted })
                                };
                                painter.text(btn_r.center(), Align2::CENTER_CENTER, qicon, style::icon_font(12.0), qcol);
                                if qb
                                    .on_hover_text(if is_running { t("Stop run") } else { t("Run now") })
                                    .clicked()
                                {
                                    quick_clicked = true;
                                    if is_running {
                                        quick_stop = Some(row.id.clone());
                                    } else {
                                        quick_run = Some(row.id.clone());
                                    }
                                }
                            }

                            // Title (line 1).
                            let text_l = r.left() + 28.0;
                            let title_max = (btn_r.left() - 6.0 - text_l).max(40.0);
                            let mut tj = egui::text::LayoutJob::simple_singleline(
                                row.name.clone(),
                                style::medium(style::BODY),
                                if row.enabled { p.text } else { p.text_muted },
                            );
                            tj.wrap = egui::text::TextWrapping::truncate_at_width(title_max);
                            let tg = painter.layout_job(tj);
                            painter.galley(pos2(text_l, r.top() + 13.0), tg, p.text);

                            // Meta (line 2): project · cadence | right status chip.
                            let badge_col = match row.status {
                                AutomationStatus::Running => p.info,
                                AutomationStatus::NeedsReview => p.warn,
                                AutomationStatus::Failed => p.err,
                                AutomationStatus::Paused => p.text_faint,
                                _ => p.text_faint,
                            };
                            let status_txt = if row.status == AutomationStatus::NeedsReview && row.files_changed > 0 {
                                format!("{} · {}f", row.next_label, row.files_changed)
                            } else {
                                row.next_label.clone()
                            };
                            let sg = painter.layout_no_wrap(status_txt, style::mono(10.5), badge_col);
                            let sg_w = sg.size().x;
                            if matches!(
                                row.status,
                                AutomationStatus::Running | AutomationStatus::NeedsReview | AutomationStatus::Failed
                            ) {
                                let pill_r = Rect::from_min_size(
                                    pos2(r.right() - 10.0 - sg_w - 10.0, r.bottom() - 25.0),
                                    vec2(sg_w + 10.0, 17.0),
                                );
                                painter.rect_filled(pill_r, CornerRadius::same(9), style::tint(badge_col, dark));
                                painter.galley(pill_r.center() - sg.size() / 2.0, sg, badge_col);
                            } else {
                                painter.galley(
                                    pos2(r.right() - 12.0 - sg_w, r.bottom() - 23.0),
                                    sg,
                                    badge_col,
                                );
                            }

                            let meta_max = (r.right() - 26.0 - sg_w - text_l).max(40.0);
                            let meta = format!(
                                "{} · {} · {}",
                                row.project,
                                automation_agent_label(row.agent),
                                row.cadence
                            );
                            let mut mj = egui::text::LayoutJob::simple_singleline(
                                meta,
                                FontId::proportional(style::SMALL),
                                p.text_faint,
                            );
                            mj.wrap = egui::text::TextWrapping::truncate_at_width(meta_max);
                            let mg = painter.layout_job(mj);
                            painter.galley(pos2(text_l, r.bottom() - 24.0), mg, p.text_faint);

                            if resp.clicked() && !quick_clicked {
                                pick_id = Some(row.id.clone());
                            }
                        }
                    });
                });
        });

        // Pinned templates footer in left column.
        ui.painter().hline(rect.x_range(), foot_area.top() + 0.5, Stroke::new(1.0, p.border));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(foot_area.shrink2(vec2(10.0, 6.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let on = self.automation_tab == AutomationDetailTab::Templates;
                let r = kit::button_full(
                    ui,
                    kit::Kind::Ghost,
                    Some(icons::SPARKLE),
                    t("Browse starter templates"),
                    true,
                    on,
                );
                #[cfg(debug_assertions)]
                crate::devtest::record("auto-browse-templates", r.rect);
                if r.clicked() {
                    self.automation_tab = AutomationDetailTab::Templates;
                }
            },
        );

        if let Some(id) = pick_id {
            if self.automation_sel.as_deref() != Some(id.as_str()) {
                self.automation_sel = Some(id);
                self.automation_run_ix = 0;
            }
            if self.automation_tab == AutomationDetailTab::Templates {
                self.automation_tab = AutomationDetailTab::Runs;
            }
        }
        if let Some(id) = quick_run {
            self.automation_sel = Some(id.clone());
            self.automation_run_ix = 0;
            self.automation_tab = AutomationDetailTab::Runs;
            self.start_automation_run(&id, RunTrigger::Manual);
        }
        if let Some(id) = quick_stop {
            self.stop_automation_run(&id);
        }
    }

    /// Right detail column: selected automation header, summary strip, tabs, and content.
    fn automations_detail_pane(&mut self, ui: &mut Ui, rect: Rect) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;

        if self.automation_tab == AutomationDetailTab::Templates {
            let mut use_tpl = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
                egui::ScrollArea::vertical()
                    .id_salt("auto-templates-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(Margin::same(22)).show(ui, |ui| {
                            let w = ui.available_width();
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(t("Starter templates"))
                                        .font(style::semibold(style::TITLE))
                                        .color(p.text),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(t("Pre-configured workflows with isolated worktrees and quiet-when-clean defaults."))
                                        .font(FontId::proportional(style::SMALL))
                                        .color(p.text_faint),
                                );
                            });
                            ui.add_space(14.0);
                            if let Some(ix) = draw_templates_grid(ui, w, &p, dark) {
                                use_tpl = Some(ix);
                            }
                        });
                    });
            });
            if let Some(ix) = use_tpl {
                self.open_new_automation_modal(Some(ix));
            }
            return;
        }

        let Some(auto) = self
            .automation_sel
            .as_ref()
            .and_then(|id| self.automations.iter().find(|a| &a.id == id))
            .cloned()
        else {
            kit::empty_state_at(
                ui,
                rect,
                icons::CLOCK,
                t("Select an automation"),
                t("Choose an automation from the list or create a new one."),
                None,
            );
            return;
        };

        let is_running = self.active_automation_runs.contains_key(&auto.id);
        let status = auto.status(is_running);
        let now = now_epoch_secs();

        let confirm_del_id = egui::Id::new("auto-confirm-delete");
        let mut confirm_del: Option<String> = ui.data(|d| d.get_temp(confirm_del_id));
        if confirm_del.as_deref() != Some(auto.id.as_str()) {
            confirm_del = None;
        }

        let mut act_run = false;
        let mut act_stop = false;
        let mut act_toggle = false;
        let mut act_edit = false;
        let mut act_delete = false;
        let mut act_review_run: Option<String> = None;
        let mut act_open_run: Option<String> = None;
        let mut act_mark_reviewed: Option<String> = None;

        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(("auto-detail-scroll", &auto.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::NONE.inner_margin(Margin::same(22)).show(ui, |ui| {
                        let content_w = ui.available_width();

                        // ----- Header row -----
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            let (tile, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                            if auto.agent == AgentKind::Custom {
                                ui.painter().rect_filled(tile, CornerRadius::same(9), p.hover);
                                ui.painter().rect_stroke(
                                    tile,
                                    CornerRadius::same(9),
                                    Stroke::new(1.0, p.border),
                                    StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    tile.center(),
                                    Align2::CENTER_CENTER,
                                    icons::TERMINAL,
                                    style::icon_font(18.0),
                                    p.text_muted,
                                );
                            } else {
                                kit::paint_agent_tile(ui.painter(), tile, Some(auto.agent), &p);
                            }

                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    ui.label(
                                        RichText::new(&auto.name)
                                            .font(style::semibold(style::TITLE))
                                            .color(p.text),
                                    );
                                    let (sp_label, sp_col) = status_badge(status, &p);
                                    let pw = kit::pill_width(ui.painter(), None, sp_label);
                                    let (pr, _) = ui.allocate_exact_size(vec2(pw, 18.0), Sense::hover());
                                    kit::paint_pill(
                                        ui.painter(),
                                        pr.left_center(),
                                        None,
                                        sp_label,
                                        sp_col,
                                        style::tint(sp_col, dark),
                                    );
                                });
                                let meta = format!(
                                    "{} · {} · {}",
                                    auto.project_name,
                                    automation_agent_label(auto.agent),
                                    auto.schedule.short_label()
                                );
                                ui.label(
                                    RichText::new(meta)
                                        .font(FontId::proportional(style::SMALL))
                                        .color(p.text_faint),
                                );
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if confirm_del.as_deref() == Some(auto.id.as_str()) {
                                    let del_yes = kit::button_ex(
                                        ui,
                                        kit::Kind::DangerFilled,
                                        Some(icons::TRASH),
                                        t("Confirm delete"),
                                        false,
                                    );
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("auto-delete-confirm", del_yes.rect);
                                    if del_yes.clicked() {
                                        act_delete = true;
                                        confirm_del = None;
                                    }
                                    if kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), false).clicked() {
                                        confirm_del = None;
                                    }
                                } else {
                                    let del_b = kit::icon_button_ex(ui, icons::TRASH, t("Delete automation"), false, 28.0);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("auto-delete", del_b.rect);
                                    if del_b.clicked() {
                                        confirm_del = Some(auto.id.clone());
                                    }
                                }

                                let edit_b = kit::button_ex(
                                    ui,
                                    kit::Kind::Secondary,
                                    Some(icons::PENCIL_SIMPLE),
                                    t("Edit"),
                                    false,
                                );
                                #[cfg(debug_assertions)]
                                crate::devtest::record("auto-edit", edit_b.rect);
                                if edit_b.clicked() {
                                    act_edit = true;
                                }

                                if is_running {
                                    let stop_b =
                                        kit::button_ex(ui, kit::Kind::Danger, Some(icons::STOP), t("Stop"), false);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("auto-stop-now", stop_b.rect);
                                    if stop_b.clicked() {
                                        act_stop = true;
                                    }
                                } else {
                                    let run_b =
                                        kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLAY), t("Run now"), false);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("auto-run-now", run_b.rect);
                                    if run_b.clicked() {
                                        act_run = true;
                                    }
                                }

                                ui.add_space(4.0);
                                let mut en = auto.enabled;
                                let sw = kit::switch(ui, &mut en);
                                #[cfg(debug_assertions)]
                                crate::devtest::record("auto-toggle-enabled", sw.rect);
                                if sw.changed() {
                                    act_toggle = true;
                                }
                                ui.label(
                                    RichText::new(if auto.enabled { t("Active") } else { t("Paused") })
                                        .font(FontId::proportional(style::SMALL))
                                        .color(if auto.enabled { p.text_muted } else { p.text_faint }),
                                );
                            });
                        });

                        ui.add_space(16.0);

                        // ----- 4-cell Summary Metrics Card -----
                        let (next_val, next_sub) = if is_running {
                            (t("Running now").to_string(), t("Live session in progress").to_string())
                        } else if !auto.enabled {
                            (t("Paused").to_string(), t("Toggle Active to resume schedule").to_string())
                        } else if let Some(next_at) = auto.next_run_at {
                            (format_countdown(next_at, now), format_local_short(next_at))
                        } else {
                            (t("Manual only").to_string(), auto.schedule.short_label())
                        };

                        let finished_runs: Vec<&AutomationRun> =
                            auto.runs.iter().filter(|r| r.status != RunStatus::Running).collect();
                        let ok_runs = finished_runs
                            .iter()
                            .filter(|r| matches!(r.status, RunStatus::Clean | RunStatus::NeedsReview))
                            .count();
                        let (rate_val, rate_sub) = if finished_runs.is_empty() {
                            ("—".to_string(), t("No completed runs yet").to_string())
                        } else {
                            let pct = (ok_runs * 100) / finished_runs.len();
                            (
                                format!("{pct}%"),
                                tf("{} of {} succeeded", &[&ok_runs.to_string(), &finished_runs.len().to_string()]),
                            )
                        };

                        let (ws_val, ws_sub) = match auto.workspace_mode {
                            WorkspaceMode::Worktree => (
                                t("Isolated worktree").to_string(),
                                t("Zero interference with main repo").to_string(),
                            ),
                            WorkspaceMode::ProjectDir => (
                                t("Project folder").to_string(),
                                t("Read-only / direct workspace").to_string(),
                            ),
                        };
                        let (sess_val, sess_sub) = match auto.session_mode {
                            SessionMode::Fresh => (
                                t("Fresh per run").to_string(),
                                if auto.quiet_when_clean {
                                    t("Quiet when clean · clean context").to_string()
                                } else {
                                    t("Clean context · always notifies").to_string()
                                },
                            ),
                            SessionMode::Continue => (
                                t("Continue last").to_string(),
                                if auto.quiet_when_clean {
                                    t("Resumes previous session · quiet").to_string()
                                } else {
                                    t("Resumes previous session").to_string()
                                },
                            ),
                        };

                        paint_metrics_strip(
                            ui,
                            content_w,
                            &[
                                (t("NEXT TRIGGER"), &next_val, &next_sub),
                                (t("SUCCESS RATE"), &rate_val, &rate_sub),
                                (t("WORKSPACE"), &ws_val, &ws_sub),
                                (t("SESSION POLICY"), &sess_val, &sess_sub),
                            ],
                            &p,
                        );

                        ui.add_space(16.0);

                        // ----- Detail Tabs -----
                        let tabs = [
                            (AutomationDetailTab::Runs, t("Runs & live session")),
                            (AutomationDetailTab::Config, t("Configuration & prompt")),
                            (AutomationDetailTab::Templates, t("Templates")),
                        ];
                        let resps = kit::tab_strip(ui, &mut self.automation_tab, &tabs);
                        #[cfg(debug_assertions)]
                        for (resp, name) in resps.iter().zip(["auto-tab:runs", "auto-tab:config", "auto-tab:templates"]) {
                            crate::devtest::record(name, resp.rect);
                        }
                        let _ = resps;
                        ui.add_space(16.0);

                        match self.automation_tab {
                            AutomationDetailTab::Runs => {
                                let (rev, open, mark) = self.draw_runs_tab(ui, &auto, content_w);
                                act_review_run = rev;
                                act_open_run = open;
                                act_mark_reviewed = mark;
                            }
                            AutomationDetailTab::Config => {
                                if self.draw_config_tab(ui, &auto) {
                                    act_edit = true;
                                }
                            }
                            AutomationDetailTab::Templates => {}
                        }
                    });
                });
        });

        ui.data_mut(|d| {
            match &confirm_del {
                Some(v) => {
                    d.insert_temp(confirm_del_id, v.clone());
                }
                None => {
                    d.remove::<String>(confirm_del_id);
                }
            }
        });

        // Dispatch mutations cleanly after UI closures.
        if act_run {
            self.automation_run_ix = 0;
            self.start_automation_run(&auto.id, RunTrigger::Manual);
        }
        if act_stop {
            self.stop_automation_run(&auto.id);
        }
        if act_toggle {
            self.toggle_automation_enabled(&auto.id);
        }
        if act_edit {
            self.automation_modal_err = None;
            self.automation_modal_seq = self.automation_modal_seq.wrapping_add(1);
            self.automation_draft = Some(AutomationDraft::from_automation(&auto));
        }
        if act_delete {
            self.delete_automation(&auto.id);
        }
        if let Some(run_id) = act_review_run {
            let ctx = ui.ctx().clone();
            self.open_automation_run_workspace(&ctx, &auto.id, &run_id, true);
        }
        if let Some(run_id) = act_open_run {
            let ctx = ui.ctx().clone();
            self.open_automation_run_workspace(&ctx, &auto.id, &run_id, false);
        }
        if let Some(run_id) = act_mark_reviewed {
            self.mark_automation_run_reviewed(&auto.id, &run_id);
        }
    }

    /// Runs & live session tab: run timeline + selected run inspector + live log output.
    fn draw_runs_tab(
        &mut self,
        ui: &mut Ui,
        auto: &Automation,
        content_w: f32,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let mut act_review = None;
        let mut act_open = None;
        let mut act_mark = None;

        if auto.runs.is_empty() {
            let (box_r, _) = ui.allocate_exact_size(vec2(content_w, 190.0), Sense::hover());
            ui.painter().rect(
                box_r,
                CornerRadius::same(style::RADIUS_CARD),
                p.surface,
                Stroke::new(1.0, p.border),
                StrokeKind::Inside,
            );
            kit::empty_state_at(
                ui,
                box_r,
                icons::PLAY,
                t("No runs yet"),
                t("Click Run now (or press R) to test this automation right away, or wait for its next scheduled trigger."),
                None,
            );
            return (None, None, None);
        }

        if self.automation_run_ix >= auto.runs.len() {
            self.automation_run_ix = 0;
        }
        let sel_run = &auto.runs[self.automation_run_ix];

        // Run timeline horizontal/vertical list (compact rows, max 6 visible with scroll).
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(t("RUN HISTORY"))
                    .font(style::semibold(10.5))
                    .color(p.text_faint),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(tf("{} recorded", &[&auto.runs.len().to_string()]))
                    .font(style::mono(style::DATA))
                    .color(p.text_ghost),
            );
        });
        ui.add_space(6.0);

        let list_h = (auto.runs.len().min(4) as f32 * 40.0 + 4.0).min(168.0);
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(style::RADIUS_CARD))
            .inner_margin(Margin::symmetric(6, 4))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("auto-runs-list", &auto.id))
                    .max_height(list_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (ix, run) in auto.runs.iter().enumerate() {
                            let selected = ix == self.automation_run_ix;
                            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("auto-run-item:{ix}"), rr);
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let rp = ui.painter_at(rr);
                            if selected {
                                rp.rect_filled(rr, CornerRadius::same(6), p.subtle);
                            } else if resp.hovered() {
                                rp.rect_filled(rr, CornerRadius::same(6), p.hover);
                            }
                            let (r_lbl, r_col) = run_status_badge(run.status, &p);
                            let dot_at = pos2(rr.left() + 14.0, rr.center().y);
                            if run.status == RunStatus::Running {
                                kit::paint_lattice(&rp, dot_at, 8.0, p.info, true);
                                ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
                            } else {
                                rp.circle_filled(dot_at, 3.5, r_col);
                            }
                            let mut x = rr.left() + 28.0;
                            let num_g = rp.layout_no_wrap(
                                format!("#{}", run.run_number),
                                style::mono_semibold(style::DATA),
                                p.text,
                            );
                            let nw = num_g.size().x;
                            rp.galley(pos2(x, rr.center().y - num_g.size().y / 2.0), num_g, p.text);
                            x += nw + 8.0;

                            let pw = kit::pill_width(&rp, None, r_lbl);
                            kit::paint_pill(&rp, pos2(x, rr.center().y), None, r_lbl, r_col, style::tint(r_col, dark));
                            x += pw + 10.0;

                            // Right side: timestamp + duration + diff.
                            let dur = if run.status == RunStatus::Running {
                                t("running").to_string()
                            } else {
                                format_duration(run.duration_secs)
                            };
                            let right_txt = if run.files_changed > 0 {
                                format!(
                                    "{}f +{} -{} · {} · {}",
                                    run.files_changed,
                                    run.additions,
                                    run.deletions,
                                    dur,
                                    format_local_short(run.started_at)
                                )
                            } else {
                                format!("{} · {} · {}", run.trigger.label(), dur, format_local_short(run.started_at))
                            };
                            let rg = rp.layout_no_wrap(right_txt, style::mono(11.0), p.text_faint);
                            let r_left = rr.right() - 10.0 - rg.size().x;
                            rp.galley(pos2(r_left, rr.center().y - rg.size().y / 2.0), rg, p.text_faint);

                            let sum_room = (r_left - 10.0 - x).max(20.0);
                            let mut sj = egui::text::LayoutJob::simple_singleline(
                                run.summary_title.clone(),
                                FontId::proportional(style::SMALL),
                                p.text_muted,
                            );
                            sj.wrap = egui::text::TextWrapping::truncate_at_width(sum_room);
                            let sg = rp.layout_job(sj);
                            rp.galley(pos2(x, rr.center().y - sg.size().y / 2.0), sg, p.text_muted);

                            if resp.clicked() {
                                self.automation_run_ix = ix;
                            }
                        }
                    });
            });

        ui.add_space(14.0);

        // ----- Selected Run Inspector Card -----
        let (r_lbl, r_col) = run_status_badge(sel_run.status, &p);
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(style::RADIUS_CARD))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                // Header row.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    ui.label(
                        RichText::new(tf("Run #{}", &[&sel_run.run_number.to_string()]))
                            .font(style::semibold(style::HEADING))
                            .color(p.text),
                    );
                    let pw = kit::pill_width(ui.painter(), None, r_lbl);
                    let (pr, _) = ui.allocate_exact_size(vec2(pw, 18.0), Sense::hover());
                    kit::paint_pill(ui.painter(), pr.left_center(), None, r_lbl, r_col, style::tint(r_col, dark));

                    if let Some(br) = &sel_run.worktree_branch {
                        paint_mono_chip(ui, icons::GIT_BRANCH, br, p.text_muted, p.hover, &p);
                    }
                    if sel_run.files_changed > 0 {
                        let diff_txt = format!(
                            "{} {}  +{} -{}",
                            sel_run.files_changed,
                            if sel_run.files_changed == 1 { t("file") } else { t("files") },
                            sel_run.additions,
                            sel_run.deletions
                        );
                        paint_mono_chip(ui, icons::GIT_DIFF, &diff_txt, p.ok, style::tint(p.ok, dark), &p);
                    }
                    if !sel_run.account_used.is_empty() {
                        paint_mono_chip(ui, icons::USER_CIRCLE, &sel_run.account_used, p.text_muted, p.hover, &p);
                    }
                    if auto.agent != AgentKind::Custom
                        && let Some(sid) = &sel_run.session_id
                    {
                        let short = if sid.len() > 14 { format!("{}…", &sid[..14]) } else { sid.clone() };
                        paint_mono_chip(ui, icons::SPARKLE, &short, p.text_faint, p.hover, &p);
                    }
                });

                ui.add_space(10.0);

                // Summary callout.
                let callout_bg = style::tint(r_col, dark);
                egui::Frame::NONE
                    .fill(callout_bg)
                    .stroke(Stroke::new(1.0, style::soft(r_col, 0.30)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(&sel_run.summary_title)
                                .font(style::medium(style::BODY))
                                .color(p.text),
                        );
                        if !sel_run.summary_body.is_empty() {
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(&sel_run.summary_body)
                                    .font(FontId::proportional(style::SMALL))
                                    .color(p.text_muted),
                            );
                        }
                    });

                ui.add_space(12.0);

                // Action buttons row.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    if sel_run.files_changed > 0 || sel_run.worktree_branch.is_some() {
                        let b = kit::button_ex(
                            ui,
                            kit::Kind::Primary,
                            Some(icons::GIT_DIFF),
                            t("Review diff"),
                            false,
                        );
                        #[cfg(debug_assertions)]
                        crate::devtest::record("auto-run-review", b.rect);
                        if b.clicked() {
                            act_review = Some(sel_run.id.clone());
                        }
                    }
                    let open_kind = if sel_run.files_changed > 0 || sel_run.worktree_branch.is_some() {
                        kit::Kind::Secondary
                    } else {
                        kit::Kind::Primary
                    };
                    let open_b = kit::button_ex(
                        ui,
                        open_kind,
                        Some(icons::ARROW_SQUARE_OUT),
                        t("Open in Focus"),
                        false,
                    );
                    #[cfg(debug_assertions)]
                    crate::devtest::record("auto-run-open", open_b.rect);
                    if open_b.clicked() {
                        act_open = Some(sel_run.id.clone());
                    }

                    if sel_run.unreviewed_alert() {
                        let mark_b = kit::button_ex(
                            ui,
                            kit::Kind::Secondary,
                            Some(icons::CHECK),
                            t("Mark reviewed"),
                            false,
                        );
                        #[cfg(debug_assertions)]
                        crate::devtest::record("auto-run-mark-reviewed", mark_b.rect);
                        if mark_b.clicked() {
                            act_mark = Some(sel_run.id.clone());
                        }
                    }

                    if !sel_run.log_lines.is_empty() {
                        let copy_b = kit::button_ex(ui, kit::Kind::Ghost, Some(icons::COPY), t("Copy log"), false);
                        if copy_b.clicked() {
                            ui.ctx().copy_text(sel_run.log_lines.join("\n"));
                            self.toast_ok(t("Copied run log.").into());
                        }
                    }
                });

                ui.add_space(12.0);

                // Output / Live terminal transcript box.
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(t("SESSION OUTPUT"))
                            .font(style::semibold(10.5))
                            .color(p.text_faint),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if sel_run.status == RunStatus::Running {
                            ui.label(RichText::new(t("● LIVE")).font(style::mono_semibold(10.5)).color(p.info));
                        } else {
                            ui.label(
                                RichText::new(tf("{} lines", &[&sel_run.log_lines.len().to_string()]))
                                    .font(style::mono(10.5))
                                    .color(p.text_ghost),
                            );
                        }
                    });
                });
                ui.add_space(6.0);

                let log_frame = egui::Frame::NONE
                    .fill(p.inset)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(12));
                let log_resp = log_frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt(("auto-run-log-scroll", &sel_run.id))
                        .min_scrolled_height(150.0)
                        .max_height(260.0)
                        .stick_to_bottom(sel_run.status == RunStatus::Running)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if sel_run.log_lines.is_empty() {
                                ui.label(
                                    RichText::new(if sel_run.status == RunStatus::Running {
                                        t("Starting background session…")
                                    } else {
                                        t("No stdout/stderr output recorded.")
                                    })
                                    .font(style::mono(style::DATA))
                                    .color(p.text_faint),
                                );
                            } else {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for line in &sel_run.log_lines {
                                    let col = if line.starts_with('[') && line.contains(']') {
                                        p.info
                                    } else if line.to_ascii_lowercase().contains("error")
                                        || line.to_ascii_lowercase().contains("failed")
                                    {
                                        p.err
                                    } else {
                                        p.text_muted
                                    };
                                    ui.add(
                                        egui::Label::new(RichText::new(line).font(style::mono(style::DATA)).color(col))
                                            .wrap(),
                                    );
                                }
                            }
                        });
                });
                #[cfg(debug_assertions)]
                crate::devtest::record("auto-run-log", log_resp.response.rect);
                let _ = log_resp;
            });

        (act_review, act_open, act_mark)
    }

    /// Configuration & prompt tab (read-only overview + Edit CTA).
    fn draw_config_tab(&mut self, ui: &mut Ui, auto: &Automation) -> bool {
        let p = style::pal(ui);
        let mut edit_clicked = false;

        // Prompt card.
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(style::RADIUS_CARD))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if auto.agent == AgentKind::Custom {
                            t("COMMAND")
                        } else {
                            t("SELF-CONTAINED PROMPT")
                        })
                        .font(style::semibold(10.5))
                        .color(p.text_faint),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let eb = kit::button_ex(
                            ui,
                            kit::Kind::Secondary,
                            Some(icons::PENCIL_SIMPLE),
                            t("Edit configuration"),
                            true,
                        );
                        #[cfg(debug_assertions)]
                        crate::devtest::record("auto-config-edit", eb.rect);
                        if eb.clicked() {
                            edit_clicked = true;
                        }
                        if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::COPY), t("Copy"), true).clicked() {
                            ui.ctx().copy_text(auto.prompt.clone());
                            self.toast_ok(t("Copied prompt.").into());
                        }
                    });
                });
                ui.add_space(8.0);
                egui::Frame::NONE
                    .fill(p.inset)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(&auto.prompt)
                                    .font(if auto.agent == AgentKind::Custom {
                                        style::mono(style::MONO)
                                    } else {
                                        FontId::proportional(style::BODY)
                                    })
                                    .color(p.text),
                            )
                            .wrap(),
                        );
                    });
            });

        ui.add_space(12.0);

        // Settings summary table card.
        let account_val = auto
            .profile_id
            .clone()
            .unwrap_or_else(|| t("Auto-failover (healthy pool)").to_string());
        let rows: [(&str, String); 8] = [
            (t("Schedule"), auto.schedule.preview_sentence()),
            (t("Project"), format!("{}  ({})", auto.project_name, crate::paths::contract_tilde(&auto.project_path))),
            (t("Agent"), automation_agent_label(auto.agent).to_string()),
            (t("Account"), if auto.agent == AgentKind::Custom { t("N/A (shell)").to_string() } else { account_val }),
            (t("Workspace"), match auto.workspace_mode {
                WorkspaceMode::Worktree => t("Isolated Git worktree (cleaned automatically if no diff)").to_string(),
                WorkspaceMode::ProjectDir => t("Project folder directly").to_string(),
            }),
            (t("Session"), match auto.session_mode {
                SessionMode::Fresh => t("Fresh session per run").to_string(),
                SessionMode::Continue => t("Continue last session").to_string(),
            }),
            (t("Quiet when clean"), if auto.quiet_when_clean { t("On — notify Inbox only on changes or failures").to_string() } else { t("Off — notify Inbox on every run").to_string() }),
            (t("Guardrails"), format!(
                "{}{} · {} min timeout · {}",
                if auto.agent != AgentKind::Custom && auto.auto_approve {
                    "Auto-approve tools · "
                } else {
                    ""
                },
                auto.overlap.label(),
                auto.timeout_mins,
                auto.missed.label()
            )),
        ];

        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(style::RADIUS_CARD))
            .inner_margin(Margin::symmetric(16, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (i, (k, v)) in rows.iter().enumerate() {
                    if i > 0 {
                        let (lr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().hline(lr.x_range(), lr.center().y, Stroke::new(1.0, p.border));
                    }
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            vec2(140.0, 26.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(*k).font(style::medium(style::SMALL)).color(p.text_faint),
                                    )
                                    .selectable(false),
                                );
                            },
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(v).font(FontId::proportional(style::BODY)).color(p.text),
                            )
                            .wrap(),
                        );
                    });
                }
            });

        edit_clicked
    }

    fn automations_footer_hints(&self, ui: &mut Ui, foot: Rect) {
        let p = style::pal(ui);
        let hints = [
            ("↑↓", t("choose")),
            ("R", t("run / stop")),
            ("Space", t("pause / resume")),
            ("O", t("open workspace")),
            ("N", t("new")),
            ("Esc", t("back")),
        ];
        kit::paint_hints(ui.painter(), pos2(foot.left() + 18.0, foot.center().y), &hints, &p);
    }

    /// Centered fixed-size Slate modal for creating or editing an automation.
    pub(super) fn automation_editor_modal(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.automation_draft.clone() else {
            return;
        };
        let dark = ctx.global_style().visuals.dark_mode;
        let p = style::palette(dark);

        // Root projects available for selection.
        let project_choices: Vec<(std::path::PathBuf, String)> = self
            .projects
            .iter()
            .filter(|pr| !pr.is_worktree())
            .map(|pr| (pr.path.clone(), pr.name.clone()))
            .collect();
        if draft.project_path.as_os_str().is_empty()
            && let Some((path, name)) = project_choices.first()
        {
            draft.project_path = path.clone();
            draft.project_name = name.clone();
        }

        let title = if draft.editing_id.is_some() {
            t("Edit automation")
        } else {
            t("New automation")
        };
        let sub = if draft.nl_mode {
            t("Describe what should run and when — we'll fill the schedule, isolation, and prompt.")
        } else {
            t("Schedule a background agent or command with isolated sessions and worktrees.")
        };

        let mut cancel = false;
        let mut save_action: Option<bool> = None; // Some(run_now)
        let mut apply_nl = false;
        let mut modal_rect = Rect::NOTHING;

        // Fixed body height so switching NL/Manual or cadence never jumps the modal height!
        let body_h = (ctx.content_rect().height() - 190.0).clamp(320.0, 560.0);
        let now = now_epoch_secs();
        let sched_preview = draft.build_schedule(now);

        let modal = egui::Modal::new(egui::Id::new("automation-editor-modal"))
            .frame(dialogs::frame(ctx))
            .backdrop_color(dialogs::scrim(dark))
            .show(ctx, |ui| {
                dialogs::begin(ui, 628.0);
                if dialogs::header(ui, icons::CLOCK, p.accent, title, Some(sub)) {
                    cancel = true;
                }

                dialogs::body(ui, |ui| {
                    let scroll_out = egui::ScrollArea::vertical()
                        .id_salt(("auto-modal-scroll", self.automation_modal_seq))
                        .min_scrolled_height(body_h)
                        .max_height(body_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let w = ui.available_width();

                            // Mode switch (only when creating or anytime for quick prompt drafting).
                            let modes = [
                                kit::Seg::new(t("Configure manually"), t("Set schedule, agent, prompt, and isolation")),
                                kit::Seg::new(t("Create with prompt"), t("Describe your automation in plain language"))
                                    .icon(icons::SPARKLE),
                            ];
                            let mode_on = usize::from(draft.nl_mode);
                            let (mr, _) = ui.allocate_exact_size(vec2(w, kit::SEGMENTED_H), Sense::hover());
                            let seg_rects = kit::segment_rects(mr, 2);
                            #[cfg(debug_assertions)]
                            {
                                crate::devtest::record("auto-modal-mode:manual", seg_rects[0]);
                                crate::devtest::record("auto-modal-mode:nl", seg_rects[1]);
                            }
                            let _ = seg_rects;
                            if let Some(i) = kit::segmented_in(ui, mr, ui.id().with("auto-mode-seg"), &modes, mode_on) {
                                draft.nl_mode = i == 1;
                                self.automation_modal_err = None;
                            }

                            ui.add_space(12.0);

                            if draft.nl_mode {
                                modal_card(ui, t("Describe your automation"), |ui| {
                                    ui.label(
                                        RichText::new(t(
                                            "Include cadence, agent, and what to check or fix. Example: \"Every weekday at 09:00 check open PRs and fix clippy lints in a worktree using claude\"",
                                        ))
                                        .font(FontId::proportional(style::SMALL))
                                        .color(p.text_muted),
                                    );
                                    ui.add_space(8.0);
                                    let nl_id = egui::Id::new("auto-nl-input");
                                    let nl_r = multiline_field(
                                        ui,
                                        nl_id,
                                        &mut draft.nl_input,
                                        t("Every morning at 09:00 run tests and summarize failures in a worktree…"),
                                        120.0,
                                        false,
                                    );
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("auto-nl-input", nl_r.rect);
                                    let _ = nl_r;
                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        let gen_btn = kit::button_ex(
                                            ui,
                                            kit::Kind::Primary,
                                            Some(icons::SPARKLE),
                                            t("Generate configuration"),
                                            false,
                                        );
                                        #[cfg(debug_assertions)]
                                        crate::devtest::record("auto-nl-apply", gen_btn.rect);
                                        if gen_btn.clicked() && !draft.nl_input.trim().is_empty() {
                                            apply_nl = true;
                                        }
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(t("You can inspect and tweak every field before saving."))
                                                .font(FontId::proportional(style::SMALL))
                                                .color(p.text_faint),
                                        );
                                    });
                                });
                            } else {
                                // ----- Card 1: Name, Project, Agent, Account -----
                                modal_card(ui, t("General"), |ui| {
                                    form_row(ui, t("Name"), t("Short title shown in Automations and Inbox"), |ui| {
                                        let r = kit::field(
                                            ui,
                                            egui::Id::new("auto-field-name"),
                                            &mut draft.name,
                                            None,
                                            t("Nightly Bug Sweep"),
                                            260.0,
                                            false,
                                        );
                                        #[cfg(debug_assertions)]
                                        crate::devtest::record("auto-field-name", r.rect);
                                        let _ = r;
                                    });

                                    form_divider(ui);
                                    form_row(ui, t("Project"), t("Repository where this automation runs"), |ui| {
                                        if project_choices.is_empty() {
                                            ui.label(
                                                RichText::new(t("No projects in workspace"))
                                                    .font(FontId::proportional(style::SMALL))
                                                    .color(p.warn),
                                            );
                                        } else {
                                            let names: Vec<&str> = project_choices.iter().map(|(_, n)| n.as_str()).collect();
                                            let cur = project_choices
                                                .iter()
                                                .find(|(pp, _)| *pp == draft.project_path)
                                                .map(|(_, n)| n.as_str())
                                                .unwrap_or(names[0]);
                                            if let Some(ix) = kit::select(ui, "auto-proj-sel", cur, &names, 200.0) {
                                                draft.project_path = project_choices[ix].0.clone();
                                                draft.project_name = project_choices[ix].1.clone();
                                            }
                                        }
                                    });

                                    form_divider(ui);
                                    // Agent selector as clickable segmented pills (one click, zero dropdown friction!).
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(t("Agent")).font(style::medium(style::BODY)).color(p.text));
                                    ui.add_space(6.0);
                                    let agents = [
                                        (AgentKind::Claude, "Claude", "auto-agent:claude"),
                                        (AgentKind::Codex, "Codex", "auto-agent:codex"),
                                        (AgentKind::Antigravity, "Antigravity", "auto-agent:antigravity"),
                                        (AgentKind::OpenCode, "OpenCode", "auto-agent:opencode"),
                                        (AgentKind::Custom, "Shell", "auto-agent:shell"),
                                    ];
                                    let agent_segs: Vec<kit::Seg> = agents
                                        .iter()
                                        .map(|(k, lbl, _)| {
                                            let s = kit::Seg::new(lbl, automation_agent_label(*k));
                                            if *k == AgentKind::Custom {
                                                s.icon(icons::TERMINAL)
                                            } else {
                                                s.agent(*k)
                                            }
                                        })
                                        .collect();
                                    let cur_a = agents.iter().position(|(k, _, _)| *k == draft.agent).unwrap_or(0);
                                    let (ar, _) = ui.allocate_exact_size(vec2(ui.available_width(), kit::SEGMENTED_H), Sense::hover());
                                    #[cfg(debug_assertions)]
                                    for (sr, (_, _, marker)) in kit::segment_rects(ar, agents.len()).into_iter().zip(&agents) {
                                        crate::devtest::record(*marker, sr);
                                    }
                                    if let Some(ix) = kit::segmented_in(ui, ar, ui.id().with("auto-agent-seg"), &agent_segs, cur_a) {
                                        draft.agent = agents[ix].0;
                                        draft.profile_id = None;
                                    }
                                    ui.add_space(4.0);

                                    if draft.agent != AgentKind::Custom {
                                        form_divider(ui);
                                        let matching: Vec<String> = self
                                            .effective_profiles()
                                            .into_iter()
                                            .filter(|pr| pr.agent == draft.agent && pr.enabled)
                                            .map(|pr| pr.id)
                                            .collect();
                                        let mut acc_opts = vec![t("Auto-failover (recommended)").to_string()];
                                        acc_opts.extend(matching.iter().cloned());
                                        let acc_refs: Vec<&str> = acc_opts.iter().map(String::as_str).collect();
                                        let cur_acc = draft
                                            .profile_id
                                            .as_deref()
                                            .filter(|id| matching.iter().any(|m| m == id))
                                            .unwrap_or(acc_refs[0])
                                            .to_string();
                                        form_row(
                                            ui,
                                            t("Account"),
                                            t("Automatically switches on rate limit when Auto-failover is chosen"),
                                            |ui| {
                                                if let Some(ix) = kit::select(ui, "auto-acc-sel", &cur_acc, &acc_refs, 210.0) {
                                                    draft.profile_id = if ix == 0 {
                                                        None
                                                    } else {
                                                        matching.get(ix - 1).cloned()
                                                    };
                                                }
                                            },
                                        );
                                    }
                                });

                                ui.add_space(12.0);

                                // ----- Card 2: Schedule -----
                                modal_card(ui, t("Schedule"), |ui| {
                                    let cads = [
                                        (CadenceKind::Interval, t("Interval"), "auto-cadence:interval"),
                                        (CadenceKind::Daily, t("Daily"), "auto-cadence:daily"),
                                        (CadenceKind::Weekdays, t("Weekdays"), "auto-cadence:weekdays"),
                                        (CadenceKind::Weekly, t("Weekly"), "auto-cadence:weekly"),
                                        (CadenceKind::Cron, t("Cron"), "auto-cadence:cron"),
                                        (CadenceKind::Once, t("One-shot"), "auto-cadence:once"),
                                    ];
                                    let cad_segs: Vec<kit::Seg> = cads.iter().map(|(_, lbl, _)| kit::Seg::new(lbl, "")).collect();
                                    let cur_c = cads.iter().position(|(k, _, _)| *k == draft.cadence_kind).unwrap_or(1);
                                    let (cr, _) = ui.allocate_exact_size(vec2(ui.available_width(), kit::SEGMENTED_H), Sense::hover());
                                    #[cfg(debug_assertions)]
                                    for (sr, (_, _, marker)) in kit::segment_rects(cr, cads.len()).into_iter().zip(&cads) {
                                        crate::devtest::record(*marker, sr);
                                    }
                                    if let Some(ix) = kit::segmented_in(ui, cr, ui.id().with("auto-cad-seg"), &cad_segs, cur_c) {
                                        draft.cadence_kind = cads[ix].0;
                                        match draft.cadence_kind {
                                            CadenceKind::Interval => draft.time_text = "60".into(),
                                            CadenceKind::Daily | CadenceKind::Weekdays | CadenceKind::Weekly => {
                                                if !draft.time_text.contains(':') {
                                                    draft.time_text = "09:00".into();
                                                }
                                            }
                                            CadenceKind::Cron => {
                                                if draft.time_text.split_whitespace().count() != 5 {
                                                    draft.time_text = "0 9 * * 1-5".into();
                                                }
                                            }
                                            CadenceKind::Once => draft.time_text = "30m".into(),
                                        }
                                    }
                                    ui.add_space(10.0);

                                    match draft.cadence_kind {
                                        CadenceKind::Interval => {
                                            form_row(ui, t("Repeat every"), t("Minutes or hours (e.g. 30, 60, 4h)"), |ui| {
                                                let r = kit::field(
                                                    ui,
                                                    egui::Id::new("auto-field-time"),
                                                    &mut draft.time_text,
                                                    Some(icons::TIMER),
                                                    "60",
                                                    140.0,
                                                    false,
                                                );
                                                #[cfg(debug_assertions)]
                                                crate::devtest::record("auto-field-time", r.rect);
                                                let _ = r;
                                            });
                                        }
                                        CadenceKind::Daily | CadenceKind::Weekdays => {
                                            form_row(ui, t("Time (local)"), t("24-hour HH:MM in your local timezone"), |ui| {
                                                let r = kit::field(
                                                    ui,
                                                    egui::Id::new("auto-field-time"),
                                                    &mut draft.time_text,
                                                    Some(icons::CLOCK),
                                                    "09:00",
                                                    130.0,
                                                    false,
                                                );
                                                #[cfg(debug_assertions)]
                                                crate::devtest::record("auto-field-time", r.rect);
                                                let _ = r;
                                            });
                                        }
                                        CadenceKind::Weekly => {
                                            let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
                                            form_row(ui, t("Day & time"), t("Local day and 24-hour HH:MM"), |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 6.0;
                                                    let cur_d = days[(draft.weekday as usize) % 7];
                                                    if let Some(ix) = kit::select(ui, "auto-weekday-sel", cur_d, &days, 116.0) {
                                                        draft.weekday = ix as u8;
                                                    }
                                                    let r = kit::field(
                                                        ui,
                                                        egui::Id::new("auto-field-time"),
                                                        &mut draft.time_text,
                                                        Some(icons::CLOCK),
                                                        "08:30",
                                                        100.0,
                                                        false,
                                                    );
                                                    #[cfg(debug_assertions)]
                                                    crate::devtest::record("auto-field-time", r.rect);
                                                    let _ = r;
                                                });
                                            });
                                        }
                                        CadenceKind::Cron => {
                                            form_row(ui, t("Cron expression"), t("5-field local cron: min hour dom month dow"), |ui| {
                                                let r = kit::field(
                                                    ui,
                                                    egui::Id::new("auto-field-time"),
                                                    &mut draft.time_text,
                                                    Some(icons::CLOCK),
                                                    "0 9 * * 1-5",
                                                    190.0,
                                                    false,
                                                );
                                                #[cfg(debug_assertions)]
                                                crate::devtest::record("auto-field-time", r.rect);
                                                let _ = r;
                                            });
                                        }
                                        CadenceKind::Once => {
                                            form_row(ui, t("Run once at"), t("Relative (30m, 2h) or local HH:MM"), |ui| {
                                                let r = kit::field(
                                                    ui,
                                                    egui::Id::new("auto-field-time"),
                                                    &mut draft.time_text,
                                                    Some(icons::CLOCK),
                                                    "30m",
                                                    130.0,
                                                    false,
                                                );
                                                #[cfg(debug_assertions)]
                                                crate::devtest::record("auto-field-time", r.rect);
                                                let _ = r;
                                            });
                                        }
                                    }

                                    ui.add_space(6.0);
                                    match &sched_preview {
                                        Ok(sched) => {
                                            let next_txt = sched
                                                .next_after(now)
                                                .map(|n| format!("{} · {}", format_countdown(n, now), format_local_short(n)))
                                                .unwrap_or_else(|| t("One-shot time is in the past").to_string());
                                            egui::Frame::NONE
                                                .fill(style::tint(p.ok, dark))
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(10, 6))
                                                .show(ui, |ui| {
                                                    ui.set_width(ui.available_width());
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{} {} · Next: {}",
                                                            icons::CHECK_CIRCLE,
                                                            sched.preview_sentence(),
                                                            next_txt
                                                        ))
                                                        .font(FontId::proportional(style::SMALL))
                                                        .color(p.ok),
                                                    );
                                                });
                                        }
                                        Err(err) => {
                                            egui::Frame::NONE
                                                .fill(style::tint(p.err, dark))
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(10, 6))
                                                .show(ui, |ui| {
                                                    ui.set_width(ui.available_width());
                                                    ui.label(
                                                        RichText::new(format!("{} {err}", icons::WARNING_CIRCLE))
                                                            .font(FontId::proportional(style::SMALL))
                                                            .color(p.err),
                                                    );
                                                });
                                        }
                                    }
                                });

                                ui.add_space(12.0);

                                // ----- Card 3: Prompt / Command -----
                                let prompt_title = if draft.agent == AgentKind::Custom {
                                    t("Shell command")
                                } else {
                                    t("Self-contained prompt")
                                };
                                modal_card(ui, prompt_title, |ui| {
                                    let hint = if draft.agent == AgentKind::Custom {
                                        t("cargo test --workspace && echo all-ok")
                                    } else {
                                        t("Write a self-contained instruction. The agent starts fresh without chat context.")
                                    };
                                    let pr = multiline_field(
                                        ui,
                                        egui::Id::new("auto-field-prompt"),
                                        &mut draft.prompt,
                                        hint,
                                        76.0,
                                        draft.agent == AgentKind::Custom,
                                    );
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("auto-field-prompt", pr.rect);
                                    let _ = pr;
                                });

                                ui.add_space(12.0);

                                // ----- Card 4: Isolation, Session & Guardrails -----
                                modal_card(ui, t("Isolation & guardrails"), |ui| {
                                    // Session mode.
                                    form_row(
                                        ui,
                                        t("Session isolation"),
                                        t("Fresh per run avoids context bloat; Continue resumes the previous thread"),
                                        |ui| {
                                            let opts = [
                                                kit::Seg::new(t("Fresh per run"), t("Clean isolated session each run")),
                                                kit::Seg::new(t("Continue last"), t("Resume prior session when supported")),
                                            ];
                                            let on = usize::from(draft.session_mode == SessionMode::Continue);
                                            let (sr, _) = ui.allocate_exact_size(vec2(236.0, kit::SEGMENTED_H), Sense::hover());
                                            #[cfg(debug_assertions)]
                                            {
                                                let segs = kit::segment_rects(sr, 2);
                                                crate::devtest::record("auto-session:fresh", segs[0]);
                                                crate::devtest::record("auto-session:continue", segs[1]);
                                            }
                                            if let Some(i) = kit::segmented_in(ui, sr, ui.id().with("auto-sess-seg"), &opts, on) {
                                                draft.session_mode = if i == 0 {
                                                    SessionMode::Fresh
                                                } else {
                                                    SessionMode::Continue
                                                };
                                            }
                                        },
                                    );

                                    form_divider(ui);

                                    // Workspace mode.
                                    form_row(
                                        ui,
                                        t("Workspace"),
                                        t("Isolated Git worktree protects your working tree and cleans up automatically when empty"),
                                        |ui| {
                                            let opts = [
                                                kit::Seg::new(t("Worktree"), t("Run in an isolated Git worktree")).icon(icons::GIT_BRANCH),
                                                kit::Seg::new(t("Project dir"), t("Run directly in the project folder")).icon(icons::FOLDER),
                                            ];
                                            let on = usize::from(draft.workspace_mode == WorkspaceMode::ProjectDir);
                                            let (wr, _) = ui.allocate_exact_size(vec2(236.0, kit::SEGMENTED_H), Sense::hover());
                                            #[cfg(debug_assertions)]
                                            {
                                                let segs = kit::segment_rects(wr, 2);
                                                crate::devtest::record("auto-ws:worktree", segs[0]);
                                                crate::devtest::record("auto-ws:project", segs[1]);
                                            }
                                            if let Some(i) = kit::segmented_in(ui, wr, ui.id().with("auto-ws-seg"), &opts, on) {
                                                draft.workspace_mode = if i == 0 {
                                                    WorkspaceMode::Worktree
                                                } else {
                                                    WorkspaceMode::ProjectDir
                                                };
                                            }
                                        },
                                    );

                                    form_divider(ui);

                                    // Quiet when clean.
                                    form_row(
                                        ui,
                                        t("Quiet when clean"),
                                        t("Only alert Inbox when files changed or a run fails"),
                                        |ui| {
                                            let sw = kit::switch(ui, &mut draft.quiet_when_clean);
                                            #[cfg(debug_assertions)]
                                            crate::devtest::record("auto-switch-quiet", sw.rect);
                                            let _ = sw;
                                        },
                                    );

                                    if draft.agent != AgentKind::Custom {
                                        form_divider(ui);
                                        form_row(
                                            ui,
                                            t("Auto-approve tools"),
                                            t("Skip interactive permission prompts so headless runs can execute commands and edits"),
                                            |ui| {
                                                let sw = kit::switch(ui, &mut draft.auto_approve);
                                                #[cfg(debug_assertions)]
                                                crate::devtest::record("auto-switch-approve", sw.rect);
                                                let _ = sw;
                                            },
                                        );
                                    }

                                    form_divider(ui);

                                    // Overlap & Timeout.
                                    form_row(ui, t("If still running"), t("What happens when the next trigger fires"), |ui| {
                                        let opts = [
                                            OverlapPolicy::Skip.label(),
                                            OverlapPolicy::Queue.label(),
                                            OverlapPolicy::CancelPrevious.label(),
                                        ];
                                        if let Some(i) = kit::select(ui, "auto-overlap-sel", draft.overlap.label(), &opts, 190.0) {
                                            draft.overlap = match i {
                                                0 => OverlapPolicy::Skip,
                                                1 => OverlapPolicy::Queue,
                                                _ => OverlapPolicy::CancelPrevious,
                                            };
                                        }
                                    });

                                    form_divider(ui);

                                    form_row(ui, t("Run timeout"), t("Automatically stop hung runs after this many minutes"), |ui| {
                                        let step = kit::number_stepper(ui, &format!("{} min", draft.timeout_mins), 64.0);
                                        if step != 0 {
                                            let next = (draft.timeout_mins as i32 + step * 5).clamp(1, 240) as u32;
                                            draft.timeout_mins = next;
                                        }
                                    });

                                    form_divider(ui);

                                    form_row(ui, t("If computer slept"), t("Behavior for triggers missed while asleep"), |ui| {
                                        let opts = [MissedPolicy::Skip.label(), MissedPolicy::RunOnWake.label()];
                                        if let Some(i) = kit::select(ui, "auto-missed-sel", draft.missed.label(), &opts, 190.0) {
                                            draft.missed = if i == 0 {
                                                MissedPolicy::Skip
                                            } else {
                                                MissedPolicy::RunOnWake
                                            };
                                        }
                                    });
                                });
                            }

                            if let Some(err) = &self.automation_modal_err {
                                ui.add_space(10.0);
                                egui::Frame::NONE
                                    .fill(style::tint(p.err, dark))
                                    .stroke(Stroke::new(1.0, style::soft(p.err, 0.4)))
                                    .corner_radius(CornerRadius::same(8))
                                    .inner_margin(Margin::symmetric(12, 8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new(format!("{} {err}", icons::WARNING_CIRCLE))
                                                .font(FontId::proportional(style::SMALL))
                                                .color(p.err),
                                        );
                                    });
                            }
                        });
                    #[cfg(debug_assertions)]
                    crate::devtest::record("auto-modal-scroll", scroll_out.inner_rect);
                    let _ = scroll_out;
                });

                // Footer buttons.
                let can_save = !draft.nl_mode
                    && !draft.name.trim().is_empty()
                    && !draft.prompt.trim().is_empty()
                    && !draft.project_path.as_os_str().is_empty()
                    && sched_preview.is_ok();
                let save_label = if draft.editing_id.is_some() {
                    t("Save changes")
                } else {
                    t("Create automation")
                };
                dialogs::footer_left(
                    ui,
                    dialogs::esc_hint,
                    |ui| {
                        let save_b = ui
                            .add_enabled_ui(can_save, |ui| {
                                kit::button_ex(ui, kit::Kind::Primary, None, save_label, false)
                            })
                            .inner;
                        #[cfg(debug_assertions)]
                        crate::devtest::record("auto-modal-save", save_b.rect);
                        if save_b.clicked() {
                            save_action = Some(false);
                        }

                        let test_b = ui
                            .add_enabled_ui(can_save, |ui| {
                                kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLAY), t("Save & test run"), false)
                            })
                            .inner;
                        #[cfg(debug_assertions)]
                        crate::devtest::record("auto-modal-save-run", test_b.rect);
                        if test_b.clicked() {
                            save_action = Some(true);
                        }

                        let cancel_b = kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), false);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("auto-modal-cancel", cancel_b.rect);
                        if cancel_b.clicked() {
                            cancel = true;
                        }
                    },
                );

                modal_rect = ui.min_rect();
            });

        #[cfg(debug_assertions)]
        crate::devtest::record("auto-modal", modal_rect);
        let _ = modal_rect;

        if modal.should_close() || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            cancel = true;
        }

        if cancel {
            self.automation_draft = None;
            self.automation_modal_err = None;
            return;
        }

        if apply_nl {
            draft.apply_natural_language();
            self.automation_draft = Some(draft);
            self.automation_modal_err = None;
            return;
        }

        if let Some(run_now) = save_action {
            match self.save_automation_draft(draft.clone(), run_now) {
                Ok(_) => {
                    self.automation_draft = None;
                    self.automation_modal_err = None;
                }
                Err(err) => {
                    self.automation_modal_err = Some(err);
                    self.automation_draft = Some(draft);
                }
            }
            return;
        }

        self.automation_draft = Some(draft);
    }
}

// -----------------------------------------------------------------------------------------------
// Shared Slate helpers for Automations
// -----------------------------------------------------------------------------------------------

fn status_badge(st: AutomationStatus, p: &style::Palette) -> (&'static str, Color32) {
    match st {
        AutomationStatus::Running => (t("Running"), p.info),
        AutomationStatus::NeedsReview => (t("Needs review"), p.warn),
        AutomationStatus::Failed => (t("Failed"), p.err),
        AutomationStatus::Clean => (t("Clean"), p.ok),
        AutomationStatus::Paused => (t("Paused"), p.text_faint),
    }
}

fn run_status_badge(st: RunStatus, p: &style::Palette) -> (&'static str, Color32) {
    match st {
        RunStatus::Running => (t("Running"), p.info),
        RunStatus::NeedsReview => (t("Needs review"), p.warn),
        RunStatus::Clean => (t("Clean"), p.ok),
        RunStatus::Failed => (t("Failed"), p.err),
        RunStatus::Cancelled => (t("Cancelled"), p.text_faint),
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn paint_metrics_strip(ui: &mut Ui, width: f32, cells: &[(&str, &str, &str); 4], p: &style::Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 68.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(style::RADIUS_CARD),
        p.surface,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    let cw = rect.width() / 4.0;
    for (i, (label, val, sub)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(rect.left() + i as f32 * cw, rect.top()), vec2(cw, rect.height()));
        if i > 0 {
            painter.vline(cell.left(), rect.y_range().shrink(10.0), Stroke::new(1.0, p.border));
        }
        let max_w = (cell.width() - 24.0).max(40.0);
        painter.text(
            pos2(cell.left() + 14.0, cell.top() + 12.0),
            Align2::LEFT_TOP,
            *label,
            style::semibold(10.0),
            p.text_faint,
        );
        let mut vj = egui::text::LayoutJob::simple_singleline((*val).to_string(), style::semibold(style::HEADING), p.text);
        vj.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let vg = painter.layout_job(vj);
        painter.galley(pos2(cell.left() + 14.0, cell.top() + 28.0), vg, p.text);

        let mut sj = egui::text::LayoutJob::simple_singleline(
            (*sub).to_string(),
            FontId::proportional(11.0),
            p.text_faint,
        );
        sj.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let sg = painter.layout_job(sj);
        painter.galley(pos2(cell.left() + 14.0, cell.top() + 48.0), sg, p.text_faint);
    }
}

fn paint_mono_chip(ui: &mut Ui, icon: &str, text: &str, fg: Color32, bg: Color32, p: &style::Palette) {
    let g = ui.painter().layout_no_wrap(text.to_string(), style::mono(11.0), fg);
    let w = 8.0 + 12.0 + 5.0 + g.size().x + 8.0;
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    ui.painter().rect(
        r,
        CornerRadius::same(6),
        bg,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(12.0), fg);
    ui.painter().galley(pos2(r.left() + 25.0, r.center().y - g.size().y / 2.0), g, fg);
}

fn draw_templates_grid(ui: &mut Ui, width: f32, p: &style::Palette, dark: bool) -> Option<usize> {
    let cols = if width >= 620.0 { 2 } else { 1 };
    let gap = 12.0;
    let card_w = ((width - gap * (cols - 1) as f32) / cols as f32).max(260.0);
    let mut picked = None;

    for chunk in STARTER_TEMPLATES.chunks(cols).enumerate() {
        let (row_ix, items) = chunk;
        if row_ix > 0 {
            ui.add_space(gap);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (c_ix, tpl) in items.iter().enumerate() {
                let idx = row_ix * cols + c_ix;
                    let frame = egui::Frame::NONE
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(CornerRadius::same(style::RADIUS_CARD))
                        .inner_margin(Margin::same(14));
                    frame.show(ui, |ui| {
                        ui.vertical(|ui| {
                            let inner_w = (card_w - 28.0).max(200.0);
                            ui.set_width(inner_w);
                            ui.set_min_height(106.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(tpl.name)
                                        .font(style::semibold(style::BODY + 0.5))
                                        .color(p.text),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let ws_txt = if tpl.workspace_mode == WorkspaceMode::Worktree {
                                        t("Worktree")
                                    } else {
                                        t("Project dir")
                                    };
                                    let col = if tpl.workspace_mode == WorkspaceMode::Worktree {
                                        p.accent_text
                                    } else {
                                        p.text_muted
                                    };
                                    let bg = if tpl.workspace_mode == WorkspaceMode::Worktree {
                                        p.accent_tint
                                    } else {
                                        p.hover
                                    };
                                    let pw = kit::pill_width(ui.painter(), None, ws_txt);
                                    let (pr, _) = ui.allocate_exact_size(vec2(pw, 18.0), Sense::hover());
                                    kit::paint_pill(ui.painter(), pr.left_center(), None, ws_txt, col, bg);
                                });
                            });
                            ui.add_space(6.0);
                            ui.allocate_ui_with_layout(
                                vec2(inner_w, 34.0),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(tpl.desc)
                                                .font(FontId::proportional(style::SMALL))
                                                .color(p.text_muted),
                                        )
                                        .wrap(),
                                    );
                                },
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} {} · {}",
                                        icons::CLOCK,
                                        tpl.cadence_summary(),
                                        automation_agent_label(tpl.agent)
                                    ))
                                    .font(style::mono(11.0))
                                    .color(p.text_faint),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let b = kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUS), t("Use template"), true);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record(format!("auto-template:{idx}"), b.rect);
                                    if b.clicked() {
                                        picked = Some(idx);
                                    }
                                });
                            });
                        });
                        let _ = dark;
                    });
            }
        });
    }
    picked
}

fn modal_card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    if !title.is_empty() {
        ui.label(RichText::new(title).font(style::semibold(style::SMALL)).color(p.text_muted));
        ui.add_space(4.0);
    }
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS_CARD))
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            body(ui);
        });
}

fn form_divider(ui: &mut Ui) {
    let p = style::pal(ui);
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
    ui.add_space(4.0);
}

fn form_row(ui: &mut Ui, label: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 250.0).max(160.0));
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(label).font(style::medium(style::BODY)).color(p.text));
            if !desc.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(desc).font(FontId::proportional(11.0)).color(p.text_faint),
                    )
                    .wrap(),
                );
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            control(ui);
        });
    });
}

fn multiline_field(
    ui: &mut Ui,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    height: f32,
    mono: bool,
) -> egui::Response {
    let p = style::pal(ui);
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let frame = egui::Frame::NONE
        .fill(p.inset)
        .stroke(Stroke::new(1.0, if focused { p.accent } else { p.border_strong }))
        .corner_radius(CornerRadius::same(style::RADIUS + 1))
        .inner_margin(Margin::same(10));
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_sized(
                vec2(ui.available_width(), height),
                egui::TextEdit::multiline(text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .font(if mono {
                        style::mono(style::MONO)
                    } else {
                        FontId::proportional(style::BODY)
                    })
                    .hint_text(RichText::new(hint).color(p.text_faint)),
            )
        })
        .inner
}
