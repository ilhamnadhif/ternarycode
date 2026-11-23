//! The worktree parts of the UI: Run Agent's new-worktree box, the worktree menu (the project
//! title, the sidebar, the palette), and the Merge, Remove and Setup dialogs.

use eframe::egui::{self, Align2, Color32, CornerRadius, Margin, RichText, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_core::agent::AgentKind;
use tc_core::model::WorktreeSetup;

use super::dialogs::{ago, body, field_label, footer_left, footer_with, header};
use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::worktree::{Facts, Form};

/// Width of the labels at the left of Run Agent's worktree box.
const LABEL_W: f32 = 72.0;
/// Run Agent's body: the dialog (560) without its sides.
const RUN_AGENT_BODY: f32 = 780.0 - 250.0 - 40.0;

/// A line of a dialog's checks: an icon in its colour and what it says, wrapped.
fn check(ui: &mut Ui, icon: &str, color: Color32, text: &str, muted: bool) {
    let p = style::pal(ui);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(icon).font(style::icon_font(14.0)).color(color));
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(if muted { p.text_muted } else { p.text }).line_height(Some(18.0))).wrap());
    });
}

/// A quiet box holding lines (programs to stop, a branch's state).
fn boxed(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS + 1))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

/// One line of a [`boxed`] list.
fn boxed_line(ui: &mut Ui, icon: &str, color: Color32, text: &str) {
    let p = style::pal(ui);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 6.5, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), color);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(12.5), p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 21.0).max(20.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(r.left() + 21.0, r.center().y - g.size().y / 2.0), g, p.text);
}

/// A switch with its label after it.
fn switch_line(ui: &mut Ui, on: &mut bool, label: &str, muted: bool) {
    let p = style::pal(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        kit::switch(ui, on);
        ui.label(RichText::new(label).size(style::SMALL).color(if muted { p.text_muted } else { p.text }));
    });
}

impl App {
    /// Run Agent's "Skip permission prompts": the agent's own flag, remembered per agent.
    pub(super) fn skip_prompts_row(&mut self, ui: &mut Ui, agent: Option<AgentKind>) {
        let p = style::pal(ui);
        let flag = agent.and_then(tc_agent::sessions::skip_permissions_flag);
        let mut skip = agent.is_some_and(|a| self.skips_permissions(a)) && flag.is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let r = ui.add_enabled_ui(flag.is_some(), |ui| kit::switch(ui, &mut skip)).inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("skip-permissions", r.rect);
            if r.changed()
                && let Some(a) = agent
            {
                self.set_skips_permissions(a, skip);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(t("Skip permission prompts")).size(13.0).color(p.text));
                let (note, c) = match flag {
                    Some(f) if skip => (tf("Adds {}: the agent runs every tool without asking you.", &[f]), p.warn),
                    Some(_) => (t("The agent runs every tool without asking you.").to_string(), p.text_faint),
                    None => (t("This agent has no option for this.").to_string(), p.text_faint),
                };
                ui.label(RichText::new(note).size(11.5).color(c));
            });
        });
    }

    /// Run Agent's new worktree: its name (and so its branch), the branch it starts from, its
    /// folder, the project's setup (Edit… opens it), and a word on changes not committed in the
    /// project (they stay there). Returns whether Edit… was clicked.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn worktree_box(
        &mut self,
        ui: &mut Ui,
        root: ProjectId,
        root_name: &str,
        changed: usize,
        wt: &mut Form,
        base: &str,
        facts: Option<&Facts>,
        problem: Option<&str>,
        name_id: egui::Id,
    ) -> bool {
        let p = style::pal(ui);
        let mut edit = false;
        let folder = self.project(root).map(|r| crate::paths::contract_tilde(&self.worktree_dir(r, wt.name.trim()))).unwrap_or_default();
        let branch = self.worktree_branch(&wt.name);
        let setup = crate::worktree::setup_summary(&self.worktree_setup_of(root));
        // The dialog's width, not what its longest line would like (a long folder is cut).
        let inner = (ui.available_width().min(RUN_AGENT_BODY) - 28.0).max(200.0);
        egui::Frame::NONE
            .fill(p.canvas)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin { left: 14, right: 14, top: 12, bottom: 12 })
            .show(ui, |ui| {
                ui.set_width(inner);
                ui.set_max_width(inner);
                ui.spacing_mut().item_spacing.y = 10.0;
                let label = |ui: &mut Ui, text: &str| {
                    let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 26.0), Sense::hover());
                    ui.painter().text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, text, egui::FontId::proportional(style::SMALL), p.text_muted);
                };
                // The name, and the branch it makes (or why it can't be).
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    label(ui, t("Name"));
                    let mut name = wt.name.clone();
                    let r = kit::field(ui, name_id, &mut name, None, "", 200.0, true);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("worktree-name", r.rect);
                    if r.changed() {
                        wt.name = name.replace(char::is_whitespace, "-");
                        wt.typed = true;
                    }
                    ui.add_space(10.0);
                    match problem {
                        Some(why) => {
                            ui.label(RichText::new(why).size(style::SMALL).color(p.err));
                        }
                        None => {
                            ui.label(RichText::new(t("branch")).size(style::SMALL).color(p.text_faint));
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(&branch).font(egui::FontId::monospace(11.5)).color(p.text_muted)).truncate());
                        }
                    }
                });
                // The branch it starts from.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    label(ui, t("From"));
                    let branches = facts.map(|f| f.branches.clone()).unwrap_or_default();
                    let names: Vec<&str> = branches.iter().map(String::as_str).collect();
                    if let Some(i) = kit::select(ui, "worktree-base", base, &names, 120.0) {
                        wt.base = branches.get(i).cloned();
                    }
                    ui.add_space(10.0);
                    let note = match facts.and_then(|f| f.last.get(base)) {
                        Some((short, time)) => {
                            let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs((*time).max(0) as u64);
                            tf("last commit {} · {}", &[short, &tf("{} ago", &[&ago(at)])])
                        }
                        None if facts.is_none() => t("Reading the branches…").to_string(),
                        None => String::new(),
                    };
                    ui.add(egui::Label::new(RichText::new(note).size(style::SMALL).color(p.text_faint)).truncate());
                });
                // Where it goes.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    label(ui, t("Folder"));
                    let mut job = egui::text::LayoutJob::simple_singleline(folder.clone(), egui::FontId::monospace(11.5), p.text_faint);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((inner - LABEL_W).max(40.0));
                    ui.label(job).on_hover_text(&folder);
                });
                // What it gets from the project.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    label(ui, t("Setup"));
                    ui.add(egui::Label::new(RichText::new(&setup).size(style::SMALL).color(p.text)).truncate());
                    ui.add_space(8.0);
                    let link = ui.add(egui::Label::new(RichText::new(t("Edit…")).size(style::SMALL).color(p.accent_text)).sense(Sense::click()));
                    #[cfg(debug_assertions)]
                    crate::devtest::record("worktree-setup-edit", link.rect);
                    if link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("What a new worktree of this project gets")).clicked() {
                        edit = true;
                    }
                });
                // Changes not committed in the project stay there.
                if changed > 0 {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_space(LABEL_W - 8.0);
                        ui.label(RichText::new(icons::WARNING).font(style::icon_font(13.0)).color(p.warn));
                        ui.set_max_width(inner - LABEL_W);
                        ui.add(
                            egui::Label::new(
                                RichText::new(tf(
                                    "{} has {} uncommitted changes. They stay there: the worktree starts from {}’s last commit.",
                                    &[root_name, &changed.to_string(), base],
                                ))
                                .size(12.5)
                                .color(p.warn)
                                .line_height(Some(18.0)),
                            )
                            .wrap(),
                        );
                    });
                }
            });
        edit
    }

    /// A worktree's menu: Review against its base, Merge, its path, its project, Remove. On its
    /// title in the project header, its row in the sidebar (right-click) and its Grid tile.
    pub fn worktree_menu(&mut self, ui: &mut Ui, id: ProjectId) {
        kit::menu_body(ui);
        let Some(w) = self.project(id) else { return };
        let Some(wt) = w.worktree.clone() else { return };
        let (missing, parent) = (w.missing, w.parent);
        let parent_name = parent.and_then(|x| self.project(x)).map(|x| x.name.clone()).unwrap_or_default();
        kit::menu_header(ui, &tf("Worktree · {}", &[&wt.branch]));
        if missing {
            if kit::menu_item(ui, Some(icons::TRASH), t("Remove from List"), None, true).clicked() {
                self.actions.push(Action::RemoveWorktreeNow(id, false));
                ui.close();
            }
            return;
        }
        if kit::menu_item(ui, Some(icons::GIT_DIFF), &tf("Review Against {}", &[&wt.base]), kit::keys(&Action::Review), false).clicked() {
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::Review);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::GIT_MERGE), &tf("Merge into {}…", &[&wt.base]), None, false).clicked() {
            self.actions.push(Action::MergeWorktree(id));
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::COPY), t("Copy Path"), None, false).clicked() {
            if let Some(path) = self.project(id).map(|x| x.path.to_string_lossy().into_owned()) {
                ui.ctx().copy_text(path);
            }
            ui.close();
        }
        let reveal = if cfg!(target_os = "macos") { t("Show in Finder") } else { t("Show in Folder") };
        if kit::menu_item(ui, Some(icons::FOLDER_OPEN), reveal, None, false).clicked() {
            self.actions.push(Action::RevealProject(id));
            ui.close();
        }
        if let Some(parent) = parent
            && kit::menu_item(ui, Some(icons::FOLDER_SIMPLE), &tf("Open {}", &[&parent_name]), None, false).clicked()
        {
            self.actions.push(Action::SelectProject(parent));
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::TRASH), t("Remove Worktree…"), None, true).clicked() {
            self.actions.push(Action::RemoveWorktree(id));
            ui.close();
        }
    }

    /// Merging a worktree's branch: the command TernaryCode will type in a new tab of its
    /// project, and what is known before (the project's branch, commits and files to merge,
    /// changes not committed on either side).
    pub(super) fn merge_worktree_dialog(&mut self, ui: &mut Ui, id: ProjectId, enter: bool) -> bool {
        let p = style::pal(ui);
        let w_cap = (ui.ctx().content_rect().width() - 32.0).clamp(320.0, 480.0);
        ui.set_width(w_cap);
        let Some(w) = self.project(id) else { return false };
        let Some(wt) = w.worktree.clone() else { return false };
        let (name, git) = (w.name.clone(), w.git.clone());
        let parent = self.root_project(id).filter(|r| r.id != id);
        let parent_name = parent.map(|r| r.name.clone()).unwrap_or_default();
        let parent_git = parent.and_then(|r| r.git.clone());
        let mut keep = true;
        let path = w.path.clone();
        let sub = tf("TernaryCode types the command in a new tab of {}, so you see git and any conflict.", &[&parent_name]);
        if header(ui, icons::GIT_MERGE, p.accent, &tf("Merge {} into {}", &[&name, &wt.base]), Some(&sub)) {
            keep = false;
        }
        // What the merge brings: commits, files, lines (read once, when the dialog opens).
        let stat_id = egui::Id::new(("merge-stat", id));
        let stat = match ui.data(|d| d.get_temp::<(usize, u32, u32)>(stat_id)) {
            Some(s) => s,
            None => {
                let s = tc_git::worktree::diff_stat(&path, &wt.base, "HEAD").unwrap_or((0, 0, 0));
                ui.data_mut(|d| d.insert_temp(stat_id, s));
                s
            }
        };
        body(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            let (ahead, files) = git.as_ref().map_or((0, 0), |g| (g.ahead, g.branch_files.max(stat.0)));
            if ahead > 0 {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                let painter = ui.painter();
                let mut job = egui::text::LayoutJob::default();
                let small = egui::FontId::proportional(12.0);
                job.append(&crate::i18n::count(ahead, t("1 commit"), t("{} commits")), 0.0, egui::TextFormat::simple(small.clone(), p.text_muted));
                job.append("·", 10.0, egui::TextFormat::simple(small.clone(), p.text_ghost));
                job.append(&crate::i18n::count(files, t("1 file"), t("{} files")), 10.0, egui::TextFormat::simple(small.clone(), p.text_muted));
                job.append("·", 10.0, egui::TextFormat::simple(small, p.text_ghost));
                job.append(&format!("+{}", stat.1), 10.0, egui::TextFormat::simple(style::mono(11.0), p.ok));
                job.append(&format!("−{}", stat.2), 6.0, egui::TextFormat::simple(style::mono(11.0), p.err));
                let g = painter.layout_job(job);
                painter.galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, p.text_muted);
                super::review::paint_share(painter, pos2(rect.right() - super::review::SHARE_W, rect.center().y), stat.1, stat.2, &p);
            } else {
                check(ui, icons::INFO, p.text_faint, &tf("{} has no commits of its own yet: there is nothing to merge.", &[&wt.branch]), true);
            }
            // The command it types.
            egui::Frame::NONE
                .fill(if ui.visuals().dark_mode { p.inset } else { Color32::WHITE })
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.label(RichText::new("$").font(style::mono(12.0)).color(p.text_ghost));
                        let cmd = format!("git merge --no-ff --no-edit {}", wt.branch);
                        ui.add(egui::Label::new(RichText::new(&cmd).font(style::mono(12.0)).color(p.text)).truncate())
                            .on_hover_text(cmd);
                    });
                });
            match parent_git.as_ref().map(|g| g.branch.clone()) {
                Some(b) if b != wt.base => check(ui, icons::WARNING, p.warn, &tf("{} is on {}, not {}: the merge goes into {}.", &[&parent_name, &b, &wt.base, &b]), false),
                _ => {}
            }
            let parent_changed = parent_git.as_ref().map_or(0, |g| g.changed);
            if parent_changed > 0 {
                let changes = crate::i18n::count(parent_changed, t("1 uncommitted change"), t("{} uncommitted changes"));
                check(ui, icons::WARNING, p.warn, &tf("{} has {}. git stops if the merge touches it.", &[&parent_name, &changes]), false);
            }
            let changed = self.worktree_dirty_files(id).len();
            if changed > 0 {
                let files = crate::i18n::count(changed, t("1 file"), t("{} files"));
                check(ui, icons::INFO, p.text_faint, &tf("{} in {} are not committed: they are not merged.", &[&files, &name]), true);
            }
        });
        let mut ok = false;
        footer_with(ui, Some(&tf("Opens a new tab in {}.", &[&parent_name])), |ui| {
            let r = kit::button_keys(ui, kit::Kind::Primary, Some(icons::TERMINAL_WINDOW), t("Merge in Terminal"), Some("↵"), false);
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", r.rect);
            ok = r.clicked();
            if kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), false).clicked() {
                keep = false;
            }
        });
        if ok || enter {
            ui.data_mut(|d| d.remove::<(usize, u32, u32)>(stat_id));
            self.actions.push(Action::MergeWorktreeNow(id));
            keep = false;
        }
        if !keep {
            ui.data_mut(|d| d.remove::<(usize, u32, u32)>(stat_id));
        }
        keep
    }

    /// Removing a worktree. With nothing to lose (its branch merged or without commits, nothing
    /// uncommitted): its folder goes, and its branch too unless kept. Else what would be lost is
    /// named first, and its branch is kept unless chosen otherwise.
    pub(super) fn remove_worktree_dialog(&mut self, ui: &mut Ui, id: ProjectId, branch: &mut Option<bool>) -> bool {
        let p = style::pal(ui);
        let w_cap = (ui.ctx().content_rect().width() - 32.0).clamp(320.0, 500.0);
        ui.set_width(w_cap);
        let Some(w) = self.project(id) else { return false };
        let Some(wt) = w.worktree.clone() else { return false };
        let name = w.name.clone();
        let w_path = w.path.clone();
        let place = crate::paths::contract_tilde(&w.path);
        let (merged, ahead) = w.git.as_ref().map_or((false, 0), |g| (g.merged, g.ahead));
        let uncommitted = self.worktree_dirty_files(id);
        let running: Vec<String> = w.panes().iter().filter_map(|x| self.panes.get(x)).filter_map(crate::pane::Pane::busy_with).collect();
        let unmerged = ahead > 0 && !merged;
        let safe = uncommitted.is_empty() && !unmerged;
        // Until the user flips it, the switch follows what git says now (its numbers may come
        // after the dialog opened): a branch with commits not merged is kept.
        let mut choice = branch.unwrap_or(!unmerged);
        let delete = &mut choice;
        let mut keep = true;
        let (mut remove, mut review) = (false, false);
        if safe {
            if header(ui, icons::TRASH, p.err, &tf("Remove the worktree {}?", &[&name]), Some(&tf("Its folder is deleted: {}", &[&place]))) {
                keep = false;
            }
            body(ui, |ui| {
                boxed(ui, |ui| {
                    for r in &running {
                        boxed_line(ui, icons::TERMINAL_WINDOW, p.text_faint, &tf("{} · it is stopped", &[r]));
                    }
                    if merged {
                        boxed_line(ui, icons::CHECK_CIRCLE, p.ok, &tf("{} is merged into {}", &[&wt.branch, &wt.base]));
                    } else {
                        boxed_line(ui, icons::INFO, p.text_faint, &tf("{} has no commits of its own", &[&wt.branch]));
                    }
                });
                ui.add_space(6.0);
                switch_line(ui, delete, &tf("Delete the branch {} too", &[&wt.branch]), false);
            });
            footer_with(ui, None, |ui| {
                let r = kit::button_ex(ui, kit::Kind::DangerFilled, None, t("Remove Worktree"), false);
                #[cfg(debug_assertions)]
                crate::devtest::record("dialog-ok", r.rect);
                remove = r.clicked();
                if kit::secondary(ui, None, t("Cancel")).clicked() {
                    keep = false;
                }
            });
        } else {
            let sub = tf("{} has work that is not in {}. Removing the worktree deletes its folder; the branch is kept, so you can merge it later.", &[&wt.branch, &wt.base]);
            if header(ui, icons::TRASH, p.err, &tf("Remove worktree {}?", &[&name]), Some(&sub)) {
                keep = false;
            }
            // Its commits not merged (read once, when the dialog opens).
            let commits_id = egui::Id::new(("remove-commits", id));
            let commits: Vec<(String, String)> = match ui.data(|d| d.get_temp::<Vec<(String, String)>>(commits_id)) {
                Some(c) => c,
                None => {
                    let c: Vec<(String, String)> = tc_git::worktree::merge_base(&w_path, &wt.base, "HEAD")
                        .and_then(|fork| tc_git::worktree::commits(&w_path, &fork, "HEAD"))
                        .map(|cs| cs.into_iter().map(|c| (c.short, c.subject)).collect())
                        .unwrap_or_default();
                    ui.data_mut(|d| d.insert_temp(commits_id, c.clone()));
                    c
                }
            };
            let git_changes = self.project(id).and_then(|x| x.git.clone());
            let max_list_h = (ui.ctx().content_rect().height() - 280.0).clamp(160.0, 380.0);
            body(ui, |ui| {
                // What would be lost or left behind, in one scrollable list so short windows never clip the footer.
                egui::ScrollArea::vertical()
                    .id_salt(("remove-wt-scroll", id))
                    .max_height(max_list_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                egui::Frame::NONE
                    .fill(p.canvas)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let full = ui.available_width();
                        let section = |ui: &mut Ui, icon: &str, text: &str, first: bool| {
                            let (r, _) = ui.allocate_exact_size(vec2(full, 30.0), Sense::hover());
                            if !first {
                                ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border));
                            }
                            ui.painter().rect_filled(r.shrink(1.0), CornerRadius::ZERO, p.hover);
                            ui.painter().text(pos2(r.left() + 12.0 + 6.0, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(12.0), p.text_faint);
                            ui.painter().text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, text, style::medium(12.0), p.text);
                        };
                        let line = |ui: &mut Ui, mark: (&str, Color32), text: &str, mono: bool| {
                            let (r, _) = ui.allocate_exact_size(vec2(full, 30.0), Sense::hover());
                            ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border));
                            ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, mark.0, style::mono(10.5), mark.1);
                            let x = r.left() + if mark.0.len() > 2 { 66.0 } else { 34.0 };
                            let font = if mono { style::mono(11.5) } else { egui::FontId::proportional(12.5) };
                            let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, p.text);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - 12.0 - x).max(20.0));
                            let g = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, p.text);
                        };
                        let mut first = true;
                        if unmerged {
                            section(ui, icons::GIT_COMMIT, &crate::i18n::count(ahead, t("1 commit to merge"), t("{} commits to merge")), first);
                            first = false;
                            for (short, subject) in commits.iter().take(5) {
                                line(ui, (short, p.accent_text), subject, false);
                            }
                            if commits.len() > 5 {
                                line(ui, ("", p.text_faint), &tf("{} more", &[&(commits.len() - 5).to_string()]), false);
                            }
                        }
                        if !uncommitted.is_empty() {
                            section(ui, icons::FILE, &crate::i18n::count(uncommitted.len(), t("1 file is not committed"), t("{} files are not committed")), first);
                            first = false;
                            for f in uncommitted.iter().take(5) {
                                let change = git_changes.as_ref().and_then(|g| g.changes.get(&w_path.join(f)).copied());
                                let (letter, color) = change.map_or(("M", p.accent_text), |c| (c.letter(), super::explorer_ui::change_color(c, &p)));
                                line(ui, (letter, color), f, true);
                            }
                            if uncommitted.len() > 5 {
                                line(ui, ("", p.text_faint), &tf("{} more", &[&(uncommitted.len() - 5).to_string()]), false);
                            }
                        }
                        if !running.is_empty() {
                            section(ui, icons::TERMINAL_WINDOW, t("Stopped"), first);
                            for r in &running {
                                line(ui, ("", p.text_faint), r, false);
                            }
                        }
                    });
                    });
                ui.add_space(4.0);
                switch_line(ui, delete, &tf("Delete the branch {} too", &[&wt.branch]), true);
            });
            footer_left(
                ui,
                |ui| {
                    review = kit::button_ex(ui, kit::Kind::Ghost, Some(icons::GIT_DIFF), t("Review First"), false).clicked();
                },
                |ui| {
                    let r = kit::button_ex(ui, kit::Kind::DangerFilled, None, t("Remove Anyway"), false);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("dialog-ok", r.rect);
                    remove = r.clicked();
                    if kit::secondary(ui, None, t("Cancel")).clicked() {
                        keep = false;
                    }
                },
            );
            if !keep || remove || review {
                ui.data_mut(|d| d.remove::<Vec<(String, String)>>(commits_id));
            }
        }
        if choice != branch.unwrap_or(!unmerged) {
            *branch = Some(choice);
        }
        if remove {
            self.actions.push(Action::RemoveWorktreeNow(id, choice));
            keep = false;
        }
        if review {
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::Review);
            keep = false;
        }
        keep
    }

    /// What a new worktree of a project gets: files git ignores copied from it (chips; the
    /// ignored names at its top can be added with a click), and a command run in it first.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn worktree_setup_dialog(
        &mut self,
        ui: &mut Ui,
        id: ProjectId,
        copy: &mut Vec<String>,
        adding: &mut Option<String>,
        run: &mut String,
        fresh: &mut bool,
        enter: bool,
    ) -> bool {
        let p = style::pal(ui);
        ui.set_width(480.0);
        let Some(w) = self.project(id) else { return false };
        let (name, path) = (w.name.clone(), w.path.clone());
        let facts = self.worktree_facts(&path).cloned();
        let ignored = facts.as_ref().map(|f| f.ignored.clone()).unwrap_or_default();
        // Never set for this project: its defaults, once the repository is read.
        if *fresh && facts.is_some() {
            *fresh = false;
            if copy.is_empty() {
                *copy = self.worktree_setup_of(id).copy;
            }
        }
        let mut keep = true;
        if header(ui, icons::GIT_FORK, p.accent, &tf("Worktree setup · {}", &[&name]), Some(t("A new worktree starts from a commit: files git ignores are not in it."))) {
            keep = false;
        }
        let add_id = egui::Id::new("worktree-setup-add");
        let mut typing = false;
        body(ui, |ui| {
            field_label(ui, &tf("Copy from {}", &[&name]));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let mut gone = None;
                for (i, item) in copy.iter().enumerate() {
                    let font = egui::FontId::monospace(12.0);
                    let g = ui.painter().layout_no_wrap(item.clone(), font, p.text);
                    let (r, _) = ui.allocate_exact_size(vec2(8.0 + g.size().x + 6.0 + 14.0 + 4.0, 22.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), p.subtle);
                    ui.painter().galley(pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, p.text);
                    let x = egui::Rect::from_center_size(pos2(r.right() - 4.0 - 7.0, r.center().y), vec2(14.0, 14.0));
                    let xr = ui.interact(x, egui::Id::new(("setup-chip", i)), Sense::click()).on_hover_text(t("Don’t copy it"));
                    ui.painter().text(x.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(11.0), if xr.hovered() { p.text } else { p.text_faint });
                    if xr.clicked() {
                        gone = Some(i);
                    }
                }
                if let Some(i) = gone {
                    copy.remove(i);
                }
                match adding {
                    Some(text) => {
                        let r = kit::field(ui, add_id, text, None, ".env.local", 140.0, true);
                        typing = r.has_focus() || r.lost_focus();
                        if !r.has_focus() && !r.lost_focus() && text.is_empty() {
                            r.request_focus();
                        }
                        if r.lost_focus() {
                            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            if !esc {
                                let item = text.trim().to_string();
                                if !item.is_empty() && !copy.contains(&item) {
                                    copy.push(item);
                                }
                            }
                            *adding = None;
                        }
                    }
                    None => {
                        if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::PLUS), t("Add…"), true).clicked() {
                            *adding = Some(String::new());
                        }
                    }
                }
            });
            // What git ignores here: a click adds it.
            let others: Vec<&String> = ignored.iter().filter(|n| !copy.contains(n)).collect();
            if !others.is_empty() {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                    ui.label(RichText::new(t("Ignored here:")).size(style::SMALL).color(p.text_faint));
                    for (i, n) in others.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("·").size(style::SMALL).color(p.text_faint));
                        }
                        let r = ui.add(egui::Label::new(RichText::new(n.as_str()).size(style::SMALL).color(p.text_faint)).sense(Sense::click()));
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Copy it into new worktrees")).clicked() {
                            copy.push((*n).clone());
                        }
                    }
                });
            }
            ui.add_space(10.0);
            field_label(ui, t("Then run, in the new worktree"));
            let w = ui.available_width();
            let r = kit::field(ui, egui::Id::new("worktree-setup-run"), run, None, "npm install", w, false);
            let _ = r;
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    RichText::new(t("Runs in the worktree’s first tab before the agent starts; you see its output. Saved in TernaryCode for this project, not in the repository."))
                        .size(style::SMALL)
                        .color(p.text_faint)
                        .line_height(Some(17.0)),
                )
                .wrap(),
            );
        });
        let mut save = enter && !typing;
        footer_with(ui, Some(&tf("↵ {}", &[t("Save")])), |ui| {
            let r = kit::button_ex(ui, kit::Kind::Primary, None, t("Save"), false);
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", r.rect);
            save |= r.clicked();
            if kit::secondary(ui, None, t("Cancel")).clicked() {
                keep = false;
            }
        });
        if save {
            let setup = WorktreeSetup { copy: copy.clone(), run: run.trim().to_string() };
            self.actions.push(Action::SetWorktreeSetup(id, setup));
            keep = false;
        }
        keep
    }
}
