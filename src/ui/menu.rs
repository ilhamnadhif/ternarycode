//! Native macOS menu bar (muda). Items with key equivalents are handled by the menu, which gets
//! those keys before egui, so the same shortcuts are not handled again in egui on macOS.

use std::collections::HashMap;
use std::sync::mpsc;

use eframe::egui;
use muda::accelerator::Accelerator;
use muda::{AboutMetadata, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::actions::Action;
use crate::i18n::t;
use tc_core::model::View;

pub struct AppMenu {
    _menu: Menu,
    actions: HashMap<MenuId, Action>,
}

/// The menu's key equivalent for an action: its label in `kit::keys` read back (`⇧⌘D` is
/// Shift+Cmd+D), so the menu does what every hint says.
fn acc(action: &Action) -> Option<Accelerator> {
    let label = super::kit::keys(action)?;
    let mut spec = String::new();
    for c in label.chars() {
        match c {
            '⌃' => spec.push_str("Ctrl+"),
            '⌥' => spec.push_str("Alt+"),
            '⇧' => spec.push_str("Shift+"),
            '⌘' => spec.push_str("Cmd+"),
            c => spec.push(c),
        }
    }
    let parsed = spec.parse().ok();
    debug_assert!(parsed.is_some(), "no key equivalent for {label}");
    parsed
}

impl AppMenu {
    /// Builds and installs the main menu. Call on the main thread once the app has launched.
    pub fn install() -> Self {
        let mut actions = HashMap::new();
        let mut item = |text: &str, action: Action| -> MenuItem {
            let it = MenuItem::new(text, true, acc(&action));
            actions.insert(it.id().clone(), action);
            it
        };

        let about = PredefinedMenuItem::about(
            Some(t("About TernaryCode")),
            Some(AboutMetadata {
                name: Some("TernaryCode".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                comments: Some(t("Terminal workspace for AI coding agents").into()),
                ..Default::default()
            }),
        );
        let settings = item(t("Settings…"), Action::Settings);
        let updates = item(t("Check for Updates…"), Action::CheckForUpdates);
        let integrations = item(t("Agent Integrations…"), Action::Integrations);
        let quit = item(t("Quit TernaryCode"), Action::Quit);
        let app_menu = Submenu::with_items(
            "TernaryCode",
            true,
            &[
                &about,
                &updates,
                &PredefinedMenuItem::separator(),
                &settings,
                &integrations,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(Some(t("Services"))),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(Some(t("Hide TernaryCode"))),
                &PredefinedMenuItem::hide_others(Some(t("Hide Others"))),
                &PredefinedMenuItem::show_all(Some(t("Show All"))),
                &PredefinedMenuItem::separator(),
                &quit,
            ],
        )
        .expect("app menu");

        let add_project = item(t("Add Project…"), Action::AddProject);
        let open_file = item(t("Open File…"), Action::QuickOpen);
        let save = item(t("Save"), Action::SaveFile);
        let new_tab = item(t("New Tab"), Action::NewTab);
        let close = item(t("Close Pane"), Action::ClosePane);
        let file_menu = Submenu::with_items(
            t("File"),
            true,
            &[
                &add_project,
                &open_file,
                &save,
                &PredefinedMenuItem::separator(),
                &new_tab,
                &close,
            ],
        )
        .expect("file menu");

        let copy = item(t("Copy"), Action::Copy);
        let paste = item(t("Paste"), Action::Paste);
        let select_all = item(t("Select All"), Action::SelectAll);
        let find = item(t("Find in Pane…"), Action::Find);
        let find_all = item(t("Find in All Panes…"), Action::FindAll);
        let clear = item(t("Clear Scrollback"), Action::ClearScrollback);
        let edit_menu = Submenu::with_items(
            t("Edit"),
            true,
            &[
                &copy,
                &paste,
                &select_all,
                &PredefinedMenuItem::separator(),
                &find,
                &find_all,
                &PredefinedMenuItem::separator(),
                &clear,
            ],
        )
        .expect("edit menu");

        let palette = item(t("Command Palette…"), Action::Palette);
        let switch = item(t("Switch Project…"), Action::SwitchProject);
        let editor = item(t("Show/Hide Editor"), Action::ToggleEditor);
        let sidebar = item(t("Show/Hide Sidebar"), Action::ToggleSidebar);
        let focus = item(t("Focus View"), Action::SetView(View::Focus));
        let grid = item(t("Grid of All Projects"), Action::ToggleGrid);
        let split_right = item(t("Split Right"), Action::SplitRight);
        let split_down = item(t("Split Down"), Action::SplitDown);
        let next_pane = item(t("Next Pane"), Action::NextPane);
        let prev_pane = item(t("Previous Pane"), Action::PrevPane);
        let bigger = item(t("Bigger Text"), Action::FontBigger);
        let smaller = item(t("Smaller Text"), Action::FontSmaller);
        let reset = item(t("Actual Size"), Action::FontReset);
        let view_menu = Submenu::with_items(
            t("View"),
            true,
            &[
                &palette,
                &switch,
                &sidebar,
                &editor,
                &PredefinedMenuItem::separator(),
                &focus,
                &grid,
                &PredefinedMenuItem::separator(),
                &split_right,
                &split_down,
                &next_pane,
                &prev_pane,
                &PredefinedMenuItem::separator(),
                &bigger,
                &smaller,
                &reset,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::fullscreen(Some(t("Enter Full Screen"))),
            ],
        )
        .expect("view menu");

        let run_agent = item(t("Run Agent…"), Action::PickPreset);
        let next_attention = item(t("Go to Next Waiting Agent"), Action::NextAttention);
        let back = item(t("Back to Where I Was"), Action::BackAttention);
        let review = item(t("Review Changes…"), Action::Review);
        let attention = item(t("Show Attention List"), Action::ToggleAttention);
        let agent_menu = Submenu::with_items(
            t("Agent"),
            true,
            &[
                &run_agent,
                &review,
                &PredefinedMenuItem::separator(),
                &next_attention,
                &back,
                &attention,
            ],
        )
        .expect("agent menu");

        let next_tab = item(t("Show Next Tab"), Action::NextTab);
        let prev_tab = item(t("Show Previous Tab"), Action::PrevTab);
        let window_menu = Submenu::with_items(
            t("Window"),
            true,
            &[
                &PredefinedMenuItem::minimize(Some(t("Minimize"))),
                &PredefinedMenuItem::maximize(Some(t("Zoom"))),
                &PredefinedMenuItem::separator(),
                &next_tab,
                &prev_tab,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::bring_all_to_front(Some(t("Bring All to Front"))),
            ],
        )
        .expect("window menu");

        let menu = Menu::with_items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &agent_menu,
            &window_menu,
        ])
        .expect("menu bar");
        menu.init_for_nsapp();
        window_menu.set_as_windows_menu_for_nsapp();
        Self {
            _menu: menu,
            actions,
        }
    }

    pub fn action(&self, id: &MenuId) -> Option<Action> {
        self.actions.get(id).cloned()
    }
}

/// Forwards menu clicks into a channel and wakes egui.
pub fn event_channel(ctx: &egui::Context) -> mpsc::Receiver<MenuEvent> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = tx.send(event);
        ctx.request_repaint();
    }));
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use muda::accelerator::{Code, Modifiers};

    #[test]
    fn shortcut_labels_read_back_as_the_menus_keys() {
        let cmd = Modifiers::META;
        for (action, mods, code) in [
            (Action::SplitDown, cmd | Modifiers::SHIFT, Code::KeyD),
            (Action::ClearScrollback, cmd | Modifiers::ALT, Code::KeyK),
            (Action::Settings, cmd, Code::Comma),
            (Action::ToggleGrid, cmd, Code::Digit0),
            (Action::NextTab, cmd | Modifiers::SHIFT, Code::BracketRight),
            (Action::PrevPane, cmd, Code::BracketLeft),
            (Action::FontBigger, cmd, Code::Equal),
            (Action::FontSmaller, cmd, Code::Minus),
        ] {
            assert_eq!(acc(&action), Some(Accelerator::new(mods, code)), "{action:?}");
        }
        assert_eq!(acc(&Action::FontReset), None);
        assert_eq!(acc(&Action::CheckForUpdates), None);
    }
}
