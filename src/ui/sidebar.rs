//! The sidebar: the project list (FR-P3) on top, each project with its most urgent status and
//! pane count (drag to reorder, right-click for the rest), and below it the file tree of the
//! active project, split by a divider that can be dragged.

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, Dialog};
use crate::i18n::t;
use crate::project::COLORS;
use tc_core::status::Status;

const ROW_H: f32 = 30.0;
/// A project: its icon and name over a line about it.
const PROJECT_H: f32 = 46.0;
/// Centre of a project's icon, and where its text starts (from the row's left).
const ICON_X: f32 = 19.0;
const TEXT_X: f32 = 36.0;
/// Height of the row at the top of the sidebar and of the project header beside it.
pub const TOP_H: f32 = 40.0;
/// Height of a section title ("PROJECTS", "FILES").
pub const SECTION_H: f32 = 28.0;
const PAD: f32 = 8.0;

impl App {
    pub fn sidebar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let full = ui.max_rect();

        // ----- top row: room for the window buttons, the window drag area, the toggle ----------
        let top = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
        super::view::window_drag_area(ui, top, "sidebar-top");
        let toggle = Rect::from_center_size(pos2(top.right() - PAD - 13.0, top.center().y), vec2(26.0, 26.0));
        let r = ui
            .scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
                kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, &kit::tip(t("Hide Sidebar"), Action::ToggleSidebar), false, 26.0)
            })
            .inner;
        if r.clicked() {
            self.actions.push(Action::ToggleSidebar);
        }

        let body = Rect::from_min_max(pos2(full.left(), top.bottom()), full.max);
        // The file tree belongs to the project in the Focus view: the Grid shows every project,
        // and the project list takes the whole sidebar.
        let files_shown = self.active.is_some() && self.view == tc_core::model::View::Focus;
        let files_open = !self.settings.files_hidden && files_shown;
        let content_h = SECTION_H + self.projects.len() as f32 * (PROJECT_H + 2.0) + ROW_H + 8.0;
        let files_h = if !files_shown {
            0.0
        } else if files_open {
            let max = (body.height() - SECTION_H - ROW_H).max(SECTION_H * 2.0);
            let want = if self.settings.files_height > 0.0 {
                self.settings.files_height
            } else {
                body.height() - content_h.min(body.height() * 0.45)
            };
            want.clamp(SECTION_H * 3.0, max)
        } else {
            SECTION_H
        };
        let split_y = (body.bottom() - files_h).round();
        let projects_rect = Rect::from_min_max(body.min, pos2(body.right(), split_y));
        let files_rect = Rect::from_min_max(pos2(body.left(), split_y), body.max);

        ui.scope_builder(egui::UiBuilder::new().max_rect(projects_rect), |ui| {
            ui.set_clip_rect(projects_rect);
            self.project_list(ui)
        });

        // The divider between the two sections.
        if files_shown {
            ui.painter()
                .hline(body.x_range(), split_y + 0.5, Stroke::new(1.0, p.border));
        }
        if files_open {
            let handle = Rect::from_min_max(pos2(body.left(), split_y - 3.0), pos2(body.right(), split_y + 3.0));
            let dr = ui.interact(handle, egui::Id::new("sidebar-split"), Sense::drag());
            if dr.hovered() || dr.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            if dr.dragged()
                && let Some(pos) = dr.interact_pointer_pos()
            {
                self.settings.files_height = (body.bottom() - pos.y).max(SECTION_H * 3.0);
            }
            if dr.drag_stopped() {
                self.settings_changed();
            }
        }
        if files_shown {
            ui.scope_builder(egui::UiBuilder::new().max_rect(files_rect), |ui| {
                ui.set_clip_rect(files_rect);
                self.files_section(ui, files_open)
            });
        }
    }

    fn project_list(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let area = ui.max_rect();
        let title = Rect::from_min_size(area.min, vec2(area.width(), SECTION_H));
        section_title(ui, title, t("PROJECTS"));
        let add = Rect::from_center_size(pos2(title.right() - PAD - 11.0, title.center().y), vec2(22.0, 22.0));
        if ui
            .scope_builder(egui::UiBuilder::new().max_rect(add), |ui| {
                kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("Add Project…"), Action::AddProject), false, 22.0)
            })
            .inner
            .clicked()
        {
            self.actions.push(Action::AddProject);
        }

        let rows: Vec<ProjectRow> = self.projects.iter().map(|pr| self.project_row(pr, &p)).collect();
        let list = Rect::from_min_max(pos2(area.left(), title.bottom()), area.max);
        let pointer = ui.ctx().pointer_interact_pos();
        let mut row_rects = Vec::with_capacity(rows.len());
        ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink2(vec2(PAD - 2.0, 0.0))), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("project-list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, row) in rows.iter().enumerate() {
                        let ProjectRow { id, name, color, kind, status, waiting, asleep, split, branch, place, chip } = row;
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(ui.available_width(), PROJECT_H), Sense::click_and_drag());
                        row_rects.push(rect);
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("project:{name}"), rect);
                        let active = self.active == Some(*id);
                        let hovered = response.hovered();
                        let painter = ui.painter_at(rect);
                        let fill = if active {
                            p.accent_soft
                        } else if hovered {
                            p.faint_row()
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        painter.rect_filled(rect, CornerRadius::same(style::RADIUS + 2), fill);

                        // The project's colour: its icon, and a bar at the left of the open one.
                        let (r, g, b) = COLORS[*color as usize % COLORS.len()];
                        let mut tint = egui::Color32::from_rgb(r, g, b);
                        if *asleep {
                            tint = tint.gamma_multiply(0.55);
                        }
                        if active {
                            let bar = Rect::from_min_max(
                                pos2(rect.left() + 1.0, rect.top() + 11.0),
                                pos2(rect.left() + 4.0, rect.bottom() - 11.0),
                            );
                            painter.rect_filled(bar, CornerRadius::same(2), tint);
                        }
                        let top_y = rect.top() + 14.5;
                        let bottom_y = rect.top() + 31.0;
                        painter.text(
                            pos2(rect.left() + ICON_X, top_y),
                            Align2::CENTER_CENTER,
                            kind_icon(*kind),
                            style::icon_font(16.0),
                            tint,
                        );

                        // Right of the name: what waits (a count in the status colour); else,
                        // hovered, the shortcut; else a split mark when the project is split.
                        let mut right = rect.right() - 8.0;
                        let attention = *waiting > 0;
                        if attention {
                            let c = status.map_or(p.warn, |s| status::color(s, &p));
                            let text = waiting.to_string();
                            let g = painter.layout_no_wrap(text, style::medium(style::SMALL - 0.5), p.on_accent);
                            let w = (g.size().x + 9.0).max(17.0);
                            let badge = Rect::from_center_size(pos2(right - w / 2.0, top_y), vec2(w, 16.0));
                            painter.rect_filled(badge, CornerRadius::same(8), c);
                            painter.galley(badge.center() - g.size() / 2.0, g, p.on_accent);
                            right -= w + 6.0;
                        } else if hovered && i < 9 {
                            let keys = format!("{}{}", if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+Shift+" }, i + 1);
                            let g = painter.layout_no_wrap(keys, egui::FontId::proportional(style::SMALL), p.text_faint);
                            right -= g.size().x;
                            painter.galley(pos2(right, top_y - g.size().y / 2.0), g, p.text_faint);
                            right -= 6.0;
                        } else if *split {
                            let g = painter.layout_no_wrap(icons::SQUARE_SPLIT_HORIZONTAL.to_string(), style::icon_font(12.0), p.text_faint);
                            right -= g.size().x;
                            painter.galley(pos2(right, top_y - g.size().y / 2.0), g, p.text_faint);
                            right -= 6.0;
                        }

                        // The name.
                        let x = rect.left() + TEXT_X;
                        let name_color = if *asleep {
                            p.text_muted
                        } else if active || attention {
                            p.text
                        } else {
                            p.text.gamma_multiply(0.9)
                        };
                        let font = if active { style::semibold(style::BODY) } else { style::medium(style::BODY) };
                        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), font, name_color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(10.0));
                        let g = painter.layout_job(job);
                        painter.galley(pos2(x, top_y - g.size().y / 2.0), g, name_color);

                        // Under it: what is at work or waits, at the right; the branch (and how
                        // many files changed) or where the project lives, in what is left.
                        let small = egui::FontId::proportional(style::SMALL);
                        let mut line_right = rect.right() - 8.0;
                        if let Some((icon, text, c)) = chip {
                            let max = (rect.right() - 8.0 - x) * 0.62;
                            let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), small.clone(), *c);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((max - 15.0).max(20.0));
                            let g = painter.layout_job(job);
                            line_right -= g.size().x;
                            painter.galley(pos2(line_right, bottom_y - g.size().y / 2.0), g, *c);
                            line_right -= 13.0;
                            painter.text(
                                pos2(line_right + 5.0, bottom_y),
                                Align2::CENTER_CENTER,
                                icon,
                                style::icon_font(11.0),
                                *c,
                            );
                            line_right -= 8.0;
                        }
                        let room = (line_right - x).max(0.0);
                        if let Some((name, changed)) = branch {
                            painter.text(
                                pos2(x + 5.0, bottom_y),
                                Align2::CENTER_CENTER,
                                icons::GIT_BRANCH,
                                style::icon_font(11.0),
                                p.text_faint,
                            );
                            let mut job = egui::text::LayoutJob::default();
                            job.append(name, 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
                            if *changed > 0 {
                                job.append(
                                    &format!("+{changed}"),
                                    6.0,
                                    egui::TextFormat::simple(small.clone(), p.warn.gamma_multiply(0.85)),
                                );
                            }
                            job.wrap = egui::text::TextWrapping::truncate_at_width((room - 14.0).max(0.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(x + 13.0, bottom_y - g.size().y / 2.0), g, p.text_faint);
                        } else {
                            let mut job = egui::text::LayoutJob::simple_singleline(place.clone(), small.clone(), p.text_faint);
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(10.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(x, bottom_y - g.size().y / 2.0), g, p.text_faint);
                        }

                        if response.clicked() {
                            self.actions.push(Action::SelectProject(*id));
                        }
                        if response.drag_started() {
                            self.sidebar_drag = Some(i);
                        }
                        let shortcut = if i < 9 {
                            format!(
                                "  ({}{})",
                                if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+Shift+" },
                                i + 1
                            )
                        } else {
                            String::new()
                        };
                        let mut tip = self
                            .project(*id)
                            .map(|pr| {
                                let mut s = crate::paths::contract_tilde(&pr.path);
                                if let Some(g) = &pr.git {
                                    s = format!("{s}\n{} {}", icons::GIT_BRANCH, branch_text(g));
                                }
                                if !pr.kind.label().is_empty() {
                                    s = format!("{s}\n{}", pr.kind.label());
                                }
                                // What the chip shows, and where its status came from.
                                if chip.is_some()
                                    && let Some(pane) = self.project_pane(pr.id).and_then(|x| self.panes.get(&x))
                                {
                                    s = format!("{s}\n{} · {}", pane.label(), status::described(&pane.tracker));
                                }
                                s
                            })
                            .unwrap_or_default();
                        tip.push_str(&shortcut);
                        if *asleep {
                            tip = format!("{tip}\n{}", t("Shell not started yet (starts when the project is shown)"));
                        }
                        let response = response.on_hover_text(tip);
                        response.context_menu(|ui| self.project_menu(ui, *id, name));
                    }
                    // The last row adds a project.
                    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    let hovered = response.hovered();
                    if hovered {
                        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.faint_row());
                    }
                    let c = if hovered { p.text } else { p.text_faint };
                    ui.painter().text(
                        pos2(rect.left() + ICON_X, rect.center().y),
                        Align2::CENTER_CENTER,
                        icons::PLUS,
                        style::icon_font(14.0),
                        c,
                    );
                    ui.painter().text(
                        pos2(rect.left() + TEXT_X, rect.center().y),
                        Align2::LEFT_CENTER,
                        t("Add Project…"),
                        egui::FontId::proportional(style::BODY - 0.5),
                        c,
                    );
                    if response.clicked() {
                        self.actions.push(Action::AddProject);
                    }
                });
        });

        // Reordering: a line where the dragged project would land.
        if let Some(from) = self.sidebar_drag {
            let released = ui.input(|i| i.pointer.any_released());
            if let Some(pos) = pointer {
                let to = row_rects
                    .iter()
                    .position(|r| pos.y < r.center().y)
                    .unwrap_or(row_rects.len());
                let y = row_rects
                    .get(to)
                    .map(|r| r.top() - 1.0)
                    .or_else(|| row_rects.last().map(|r| r.bottom() + 1.0));
                if let Some(y) = y
                    && to != from
                    && to != from + 1
                {
                    ui.painter().hline(list.x_range().shrink(PAD), y, Stroke::new(2.0, p.accent));
                }
                if released {
                    self.actions.push(Action::MoveProject { from, to });
                }
            }
            if released {
                self.sidebar_drag = None;
            }
        }
    }

    fn project_menu(&mut self, ui: &mut Ui, id: u64, name: &str) {
        if ui.button(t("Rename…")).clicked() {
            self.dialog = Some(Dialog::Rename(id, name.to_string()));
            ui.close();
        }
        ui.menu_button(t("Color"), |ui| {
            ui.horizontal(|ui| {
                for (ci, (r, g, b)) in COLORS.iter().enumerate() {
                    let c = egui::Color32::from_rgb(*r, *g, *b);
                    if kit::dot(ui, c, 7.0).clicked() {
                        self.actions.push(Action::SetProjectColor(id, ci as u8));
                        ui.close();
                    }
                }
            });
        });
        let reveal = if cfg!(target_os = "macos") {
            t("Show in Finder")
        } else {
            t("Show in Folder")
        };
        if ui.button(reveal).clicked() {
            self.actions.push(Action::RevealProject(id));
            ui.close();
        }
        ui.separator();
        if ui.button(t("Remove from Workspace")).clicked() {
            self.actions.push(Action::RemoveProject(id));
            ui.close();
        }
    }
}

/// What a project's sidebar row shows.
struct ProjectRow {
    id: u64,
    name: String,
    color: u8,
    kind: crate::project::Kind,
    /// What happens there (none when every shell idles).
    status: Option<Status>,
    waiting: usize,
    /// No shell started yet.
    asleep: bool,
    split: bool,
    /// Under the name: the branch and how many files changed, else where it lives.
    branch: Option<(String, usize)>,
    place: String,
    /// Under the name, at the right: what is at work or waits (icon, text, colour).
    chip: Option<(&'static str, String, egui::Color32)>,
}

/// The icon of a kind of project.
pub fn kind_icon(kind: crate::project::Kind) -> &'static str {
    use crate::project::Kind;
    match kind {
        Kind::Folder => icons::FOLDER_SIMPLE,
        Kind::Rust => icons::GEAR_SIX,
        Kind::Go => icons::CUBE,
        Kind::TypeScript => icons::BRACKETS_CURLY,
        Kind::JavaScript => icons::HEXAGON,
        Kind::React => icons::ATOM,
        Kind::Flutter | Kind::Dart => icons::DEVICE_MOBILE,
        Kind::Python => icons::FILE_PY,
        Kind::Swift => icons::BIRD,
        Kind::Java => icons::COFFEE_BEAN,
        Kind::Cpp => icons::FILE_CPP,
        Kind::Web => icons::GLOBE,
    }
}

/// Where a project lives, short: its parent folder (`~/Developments`), or its last two names
/// (`…/RUST/tools`).
fn folder_line(path: &std::path::Path) -> String {
    let parent = path.parent().unwrap_or(path);
    let short = crate::paths::contract_tilde(parent);
    let parts: Vec<&str> = short.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        short
    } else {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    }
}

impl App {
    fn project_row(&self, pr: &crate::project::Project, p: &style::Palette) -> ProjectRow {
        let panes = pr.panes();
        let waiting = panes
            .iter()
            .filter_map(|id| self.panes.get(id))
            .filter(|pane| pane.tracker.wants_attention())
            .count();
        let asleep = panes.iter().all(|id| self.panes.get(id).is_none_or(crate::pane::Pane::dormant));
        let status = self.project_status(pr).map(|(s, _)| s);
        // The chip: what waits; else the agent or program at work.
        let pane = self.project_pane(pr.id).and_then(|id| self.panes.get(&id));
        let busy = pane.filter(|x| x.tracker.agent.is_some() || matches!(x.tracker.fg, tc_core::status::Foreground::Program { .. }));
        let chip = match (pane, status) {
            (Some(x), Some(s)) if x.tracker.wants_attention() => Some((status::icon(s), status::text(s), status::color(s, p))),
            _ => busy.map(|x| {
                let s = x.tracker.status;
                let c = match s {
                    Status::Unknown => p.text_muted,
                    Status::Idle | Status::Ended => p.text_faint,
                    _ => status::color(s, p),
                };
                (status::icon(s), x.label(), c)
            }),
        };
        ProjectRow {
            id: pr.id,
            name: pr.name.clone(),
            color: pr.color,
            kind: pr.kind,
            status,
            waiting,
            asleep,
            split: pr.tab().is_some_and(|t| t.layout.panes().len() > 1),
            branch: pr.git.as_ref().map(|g| (g.branch.clone(), g.changed)),
            place: if asleep { t("Not started").to_string() } else { folder_line(&pr.path) },
            chip,
        }
    }
}

/// "main · 3 changed".
pub fn branch_text(g: &crate::project::GitInfo) -> String {
    if g.changed > 0 {
        crate::i18n::tf("{} · {} changed", &[&g.branch, &g.changed.to_string()])
    } else {
        g.branch.clone()
    }
}

/// A small upper-case section title at the left of `rect`.
pub fn section_title(ui: &Ui, rect: Rect, text: &str) {
    let p = style::pal(ui);
    ui.painter().text(
        pos2(rect.left() + PAD + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        style::semibold(style::SMALL - 0.5),
        p.text_faint,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_project_line_is_short() {
        let home = crate::paths::home_dir();
        assert_eq!(super::folder_line(&home.join("Developments/webapp")), "~/Developments");
        assert_eq!(super::folder_line(&home.join("Developments/RUST/tools/x")), "…/RUST/tools");
    }
}
