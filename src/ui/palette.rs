//! The command palette (⌘K), the project switcher (⌘P) and the agent picker (⌘⇧A).

use eframe::egui::{self, Align2, Key, RichText, Sense, pos2, vec2};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::{App, PaletteMode};
use crate::i18n::t;

/// Results kept for the list (the rest by typing more).
const MAX_ROWS: usize = 200;
const ROW_H: f32 = 32.0;

struct Item {
    icon: &'static str,
    label: String,
    detail: String,
    action: Action,
}

/// Subsequence match: higher is better, `None` when `query` doesn't match.
fn score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text = text.to_lowercase();
    // Char positions, not byte offsets: letters after an "é" are still next to each other.
    let mut chars = text.chars().enumerate();
    let mut score = 0;
    let mut last = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let (i, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        score += match last {
            Some(l) if i == l + 1 => 3,
            _ => 1,
        };
        if i == 0 {
            score += 2;
        }
        last = Some(i);
    }
    Some(score * 100 - text.chars().count() as i32)
}

impl App {
    fn palette_items(&self, mode: PaletteMode) -> Vec<Item> {
        let mut items = Vec::new();
        if mode == PaletteMode::Search {
            if let Some(pal) = &self.palette {
                for (pane, m, text) in &pal.results {
                    let Some(p) = self.panes.get(pane) else { continue };
                    let project = self.project(p.project).map(|x| x.name.clone()).unwrap_or_default();
                    items.push(Item {
                        icon: icons::TERMINAL,
                        label: text.clone(),
                        detail: format!("{project} · {}", p.label()),
                        action: Action::RevealMatch(*pane, m.clone()),
                    });
                }
            }
            return items;
        }
        if mode == PaletteMode::Locations {
            if let Some(pal) = &self.palette {
                for l in &pal.locations {
                    items.push(Item {
                        icon: if l.error { icons::X_CIRCLE } else { icons::FILE_CODE },
                        label: l.label.clone(),
                        detail: l.detail.clone(),
                        action: Action::GotoLocation(l.path.clone(), l.line, l.col),
                    });
                }
            }
            return items;
        }
        if mode == PaletteMode::Files {
            let root = self.active_project().map(|p| p.path.clone()).unwrap_or_default();
            if let Some(pal) = &self.palette {
                for f in &pal.files {
                    items.push(Item {
                        icon: icons::FILE,
                        label: f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        detail: f.parent().map(|d| d.display().to_string()).unwrap_or_default(),
                        action: Action::OpenFile(root.join(f), None),
                    });
                }
            }
            return items;
        }
        if mode != PaletteMode::Presets {
            for (i, p) in self.projects.iter().enumerate() {
                items.push(Item {
                    icon: icons::FOLDER,
                    label: p.name.clone(),
                    detail: match kit::keys(&Action::SelectProjectIndex(i)) {
                        Some(k) => format!("{}  {k}", crate::paths::contract_tilde(&p.path)),
                        None => crate::paths::contract_tilde(&p.path),
                    },
                    action: Action::SelectProject(p.id),
                });
            }
        }
        if mode != PaletteMode::Projects {
            for preset in self.presets() {
                items.push(Item {
                    icon: icons::PLAY,
                    label: if mode == PaletteMode::Presets {
                        preset.name.clone()
                    } else {
                        format!("{} {}", t("Run"), preset.name)
                    },
                    detail: preset.command.clone(),
                    action: Action::RunPreset(preset.id.clone()),
                });
            }
        }
        if mode == PaletteMode::All {
            // The editor's language commands, with a file open in it.
            let code = self.settings.lsp && self.active.and_then(|id| self.editors.get(&id)).and_then(|e| e.file()).is_some();
            let undo = self.lsp.ide.undo.is_some();
            // Each with its shortcut, from the one table.
            let mut add = |icon, label: &str, action| {
                items.push(Item {
                    icon,
                    label: label.to_string(),
                    detail: kit::keys(&action).unwrap_or_default().to_string(),
                    action,
                })
            };
            add(icons::BELL, t("Go to Next Waiting Agent"), Action::NextAttention);
            add(icons::ARROW_LEFT, t("Back to Where I Was"), Action::BackAttention);
            add(icons::LIST, t("Show Attention List"), Action::ToggleAttention);
            add(icons::SQUARE_SPLIT_HORIZONTAL, t("Grid of All Projects"), Action::ToggleGrid);
            add(icons::TERMINAL_WINDOW, t("Focus View"), Action::SetView(tc_core::model::View::Focus));
            add(icons::FOLDER_PLUS, t("Add Project…"), Action::AddProject);
            add(icons::MAGNIFYING_GLASS, t("Find in Pane…"), Action::Find);
            add(icons::MAGNIFYING_GLASS, t("Find in All Panes…"), Action::FindAll);
            add(icons::CHECK_CIRCLE, t("Review Changes…"), Action::Review);
            add(icons::FILE, t("Open File…"), Action::QuickOpen);
            add(icons::CODE, t("Show/Hide Editor"), Action::ToggleEditor);
            add(icons::PLUS, t("New Tab"), Action::NewTab);
            add(icons::SQUARE_SPLIT_HORIZONTAL, t("Split Right"), Action::SplitRight);
            add(icons::SQUARE_SPLIT_VERTICAL, t("Split Down"), Action::SplitDown);
            add(icons::X, t("Close Pane"), Action::ClosePane);
            add(icons::ARROW_RIGHT, t("Show Next Tab"), Action::NextTab);
            add(icons::ARROW_LEFT, t("Show Previous Tab"), Action::PrevTab);
            add(icons::ARROW_RIGHT, t("Next Pane"), Action::NextPane);
            add(icons::ARROW_LEFT, t("Previous Pane"), Action::PrevPane);
            add(icons::TEXT_AA, t("Bigger Text"), Action::FontBigger);
            add(icons::TEXT_AA, t("Smaller Text"), Action::FontSmaller);
            add(icons::TEXT_AA, t("Actual Size"), Action::FontReset);
            add(icons::TERMINAL, t("Clear Scrollback"), Action::ClearScrollback);
            add(icons::SIDEBAR_SIMPLE, t("Show/Hide Sidebar"), Action::ToggleSidebar);
            add(icons::PLUG, t("Agent Integrations…"), Action::Integrations);
            add(icons::GEAR, t("Settings…"), Action::Settings);
            add(icons::ARROW_CIRCLE_UP, t("Check for Updates…"), Action::CheckForUpdates);
            if code {
                use crate::lsp::{Goto, IdeCmd};
                add(icons::ARROW_SQUARE_OUT, t("Go to Definition"), Action::Ide(IdeCmd::Goto(Goto::Definition)));
                add(icons::ARROW_SQUARE_OUT, t("Go to Type Definition"), Action::Ide(IdeCmd::Goto(Goto::TypeDefinition)));
                add(icons::ARROW_SQUARE_OUT, t("Go to Implementation"), Action::Ide(IdeCmd::Goto(Goto::Implementation)));
                add(icons::LIST, t("Find References"), Action::Ide(IdeCmd::References));
                add(icons::PENCIL_SIMPLE, t("Rename Symbol"), Action::Ide(IdeCmd::Rename));
                add(icons::LIGHTNING, t("Code Actions…"), Action::Ide(IdeCmd::CodeActions));
                add(icons::TEXT_AA, t("Format Document"), Action::Ide(IdeCmd::Format));
                add(icons::LIST, t("Go to Symbol in File…"), Action::Ide(IdeCmd::Symbols));
                add(icons::MAGNIFYING_GLASS, t("Go to Symbol in Project…"), Action::Ide(IdeCmd::WorkspaceSymbols));
                add(icons::WARNING, t("Problems"), Action::Ide(IdeCmd::Problems));
                add(icons::ARROW_LEFT, t("Back"), Action::Ide(IdeCmd::Back));
                add(icons::ARROW_RIGHT, t("Forward"), Action::Ide(IdeCmd::Forward));
                add(icons::ARROWS_CLOCKWISE, t("Restart Language Server"), Action::Ide(IdeCmd::RestartServer));
            }
            if undo {
                add(icons::CLOCK_COUNTER_CLOCKWISE, t("Undo Refactor"), Action::Ide(crate::lsp::IdeCmd::UndoRefactor));
            }
        }
        items
    }

    pub fn palette_ui(&mut self, ctx: &egui::Context) {
        // Search across panes: only when the query changed.
        if let Some(pal) = self.palette.as_mut()
            && pal.mode == PaletteMode::Search
            && pal.query != pal.searched
        {
            pal.searched = pal.query.clone();
            pal.results.clear();
            if pal.query.chars().count() >= 2 {
                let mut ids: Vec<_> = self.panes.keys().copied().collect();
                ids.sort_unstable();
                for id in ids {
                    let shared = self.panes[&id].shared.clone();
                    for (m, text) in tc_term::search::find_all(&shared, &pal.query, 5) {
                        pal.results.push((id, m, text));
                    }
                    if pal.results.len() >= 60 {
                        break;
                    }
                }
            }
        }
        let Some(pal_ref) = self.palette.as_ref() else { return };
        let all = self.palette_items(pal_ref.mode);
        let Some(mut pal) = self.palette.take() else { return };
        let mut items: Vec<(i32, Item)> = all
            .into_iter()
            .filter_map(|it| {
                if pal.mode == PaletteMode::Search {
                    return Some((0, it));
                }
                score(&pal.query, &format!("{} {}", it.label, it.detail)).map(|s| (s, it))
            })
            .collect();
        if !pal.query.is_empty() {
            items.sort_by_key(|x| std::cmp::Reverse(x.0));
        }
        items.truncate(MAX_ROWS);
        pal.selected = pal.selected.min(items.len().saturating_sub(1));
        let mut chosen: Option<Action> = None;
        let mut close = false;
        let title = format!("{} — {}", pal.title, t("type to filter"));
        let hint = match pal.mode {
            PaletteMode::All => t("Type a command, project or agent…"),
            PaletteMode::Projects => t("Switch to project…"),
            PaletteMode::Presets => t("Start which agent?"),
            PaletteMode::Files => t("Open which file of this project?"),
            PaletteMode::Search => t("Search the output of every pane…"),
            PaletteMode::Locations => title.as_str(),
        };
        let area = egui::Area::new(egui::Id::new("palette-area")).anchor(Align2::CENTER_TOP, vec2(0.0, 80.0));
        let modal = egui::Modal::new(egui::Id::new("palette"))
            .area(area)
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(egui::Margin::same(10))
                    .corner_radius(egui::CornerRadius::same(12)),
            )
            .show(ctx, |ui| {
                ui.set_width(560.0);
                let p = style::pal(ui);
                let r = ui
                    .horizontal(|ui| {
                        ui.add_space(6.0);
                        kit::icon(ui, icons::MAGNIFYING_GLASS, 18.0, p.text_muted);
                        ui.add(
                            egui::TextEdit::singleline(&mut pal.query)
                                .hint_text(RichText::new(hint).color(p.text_faint))
                                .font(egui::FontId::proportional(16.0))
                                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 10)))
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .inner;
                if pal.opened {
                    r.request_focus();
                    pal.opened = false;
                }
                // The selection is kept in view when the keyboard moves it (the wheel scrolls freely).
                let mut moved = r.changed();
                if r.changed() {
                    pal.selected = 0;
                }
                if r.has_focus() {
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            r.id,
                            egui::EventFilter {
                                vertical_arrows: true,
                                ..Default::default()
                            },
                        )
                    });
                }
                let (down, up, enter) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.key_pressed(Key::Enter),
                    )
                });
                if down {
                    pal.selected = (pal.selected + 1).min(items.len().saturating_sub(1));
                }
                if up {
                    pal.selected = pal.selected.saturating_sub(1);
                }
                moved |= down || up;
                ui.add_space(6.0);
                if items.is_empty() {
                    let text = if pal.loading { t("Listing the project's files…") } else { t("Nothing matches.") };
                    ui.label(RichText::new(text).color(p.text_muted));
                }
                // A dozen rows show at a time (fewer in a low window), the rest scroll; only the
                // rows in view are laid out.
                let step = ROW_H + ui.spacing().item_spacing.y;
                let max_h = (ctx.content_rect().height() - 220.0).clamp(3.0 * step, 12.0 * step);
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, (_, item)) in items.iter().enumerate() {
                        let selected = i == pal.selected;
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                        if selected && moved {
                            ui.scroll_to_rect(rect, None);
                        }
                        if resp.clicked() || (enter && selected) {
                            chosen = Some(item.action.clone());
                        }
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        if selected || resp.hovered() {
                            ui.painter().rect_filled(rect, egui::CornerRadius::same(6), if selected { p.accent_soft } else { p.faint_row() });
                        }
                        ui.painter().text(rect.left_center() + vec2(14.0, 0.0), Align2::CENTER_CENTER, item.icon, style::icon_font(15.0), p.text_muted);
                        // The detail at the right (at most 45% of the row), the label cut short before it.
                        let mut job = egui::text::LayoutJob::simple_singleline(item.detail.clone(), egui::FontId::proportional(style::SMALL), p.text_faint);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() * 0.45);
                        let detail = ui.painter().layout_job(job);
                        let detail_x = rect.right() - 10.0 - detail.size().x;
                        ui.painter().galley(pos2(detail_x, rect.center().y - detail.size().y / 2.0), detail, p.text_faint);
                        let label_x = rect.left() + 32.0;
                        let mut job = egui::text::LayoutJob::simple_singleline(item.label.clone(), style::medium(13.5), p.text);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((detail_x - 12.0 - label_x).max(10.0));
                        let label = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(label_x, rect.center().y - label.size().y / 2.0), label, p.text);
                    }
                });
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(a) = chosen {
            self.actions.push(a);
            close = true;
        }
        if close {
            // Back where the keyboard was: the editor (an IDE command came from there) or the
            // terminal.
            if pal.from_editor {
                if let Some(e) = self.active.and_then(|id| self.editors.get_mut(&id)) {
                    e.refocus = true;
                }
            } else {
                self.focus_request = self.current_pane();
            }
        } else {
            self.palette = Some(pal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn fuzzy_scores_prefer_compact_early_matches() {
        assert!(score("cla", "Claude Code").unwrap() > score("cla", "OpenCode claw").unwrap_or(i32::MIN));
        assert!(score("xyz", "Claude").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("cc", "Claude Code").is_some());
    }

    #[test]
    fn letters_beyond_ascii_count_once() {
        // "é" is two bytes: the "f" after it is still the next letter, and the text two long.
        assert_eq!(score("éf", "éf"), score("ab", "ab"));
    }
}
