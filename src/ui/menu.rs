//! Native macOS menu bar (muda). Items with key equivalents are handled by the menu, which gets
//! those keys before egui, so the same shortcuts are not handled again in egui on macOS.

use std::collections::HashMap;
use std::sync::mpsc;

use eframe::egui;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::actions::Action;
use crate::i18n::t;
use tc_core::model::View;

pub struct AppMenu {
    _menu: Menu,
    actions: HashMap<MenuId, Action>,
}

fn acc(mods: Modifiers, code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(mods, code))
}

const CMD: Modifiers = Modifiers::META;

impl AppMenu {
    /// Builds and installs the main menu. Call on the main thread once the app has launched.
    pub fn install() -> Self {
        let mut actions = HashMap::new();
        let mut item = |text: &str, accel: Option<Accelerator>, action: Action| -> MenuItem {
            let it = MenuItem::new(text, true, accel);
            actions.insert(it.id().clone(), action);
            it
        };
        let shift_cmd = CMD | Modifiers::SHIFT;
        let alt_cmd = CMD | Modifiers::ALT;

        let about = PredefinedMenuItem::about(
            Some(t("About TernaryCode")),
            Some(AboutMetadata {
                name: Some("TernaryCode".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                comments: Some(t("Terminal workspace for AI coding agents").into()),
                ..Default::default()
            }),
        );
        let settings = item(t("Settings…"), acc(CMD, Code::Comma), Action::Settings);
        let updates = item(t("Check for Updates…"), None, Action::CheckForUpdates);
        let integrations = item(t("Agent Integrations…"), None, Action::Integrations);
        let quit = item(t("Quit TernaryCode"), acc(CMD, Code::KeyQ), Action::Quit);
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

        let add_project = item(
            t("Add Project…"),
            acc(shift_cmd, Code::KeyO),
            Action::AddProject,
        );
        let open_file = item(t("Open File…"), acc(CMD, Code::KeyO), Action::QuickOpen);
        let save = item(t("Save"), acc(CMD, Code::KeyS), Action::SaveFile);
        let new_tab = item(t("New Tab"), acc(CMD, Code::KeyT), Action::NewTab);
        let close = item(t("Close Pane"), acc(CMD, Code::KeyW), Action::ClosePane);
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

        let copy = item(t("Copy"), acc(CMD, Code::KeyC), Action::Copy);
        let paste = item(t("Paste"), acc(CMD, Code::KeyV), Action::Paste);
        let select_all = item(t("Select All"), acc(CMD, Code::KeyA), Action::SelectAll);
        let find = item(t("Find in Pane…"), acc(CMD, Code::KeyF), Action::Find);
        let find_all = item(t("Find in All Panes…"), acc(shift_cmd, Code::KeyF), Action::FindAll);
        let clear = item(
            t("Clear Scrollback"),
            acc(alt_cmd, Code::KeyK),
            Action::ClearScrollback,
        );
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

        let palette = item(t("Command Palette…"), acc(CMD, Code::KeyK), Action::Palette);
        let switch = item(
            t("Switch Project…"),
            acc(CMD, Code::KeyP),
            Action::SwitchProject,
        );
        let editor = item(t("Show/Hide Editor"), acc(CMD, Code::KeyE), Action::ToggleEditor);
        let sidebar = item(
            t("Show/Hide Sidebar"),
            acc(CMD, Code::KeyB),
            Action::ToggleSidebar,
        );
        let focus = item(t("Focus View"), None, Action::SetView(View::Focus));
        let grid = item(t("Grid of All Projects"), acc(CMD, Code::Digit0), Action::ToggleGrid);
        let split_right = item(t("Split Right"), acc(CMD, Code::KeyD), Action::SplitRight);
        let split_down = item(
            t("Split Down"),
            acc(shift_cmd, Code::KeyD),
            Action::SplitDown,
        );
        let next_pane = item(
            t("Next Pane"),
            acc(CMD, Code::BracketRight),
            Action::NextPane,
        );
        let prev_pane = item(
            t("Previous Pane"),
            acc(CMD, Code::BracketLeft),
            Action::PrevPane,
        );
        let bigger = item(t("Bigger Text"), acc(CMD, Code::Equal), Action::FontBigger);
        let smaller = item(
            t("Smaller Text"),
            acc(CMD, Code::Minus),
            Action::FontSmaller,
        );
        let reset = item(t("Actual Size"), None, Action::FontReset);
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

        let run_agent = item(
            t("Run Agent…"),
            acc(shift_cmd, Code::KeyA),
            Action::PickPreset,
        );
        let next_attention = item(
            t("Go to Next Waiting Agent"),
            acc(CMD, Code::KeyJ),
            Action::NextAttention,
        );
        let back = item(
            t("Back to Where I Was"),
            acc(shift_cmd, Code::KeyJ),
            Action::BackAttention,
        );
        let review = item(t("Review Changes…"), acc(shift_cmd, Code::KeyR), Action::Review);
        let attention = item(
            t("Show Attention List"),
            acc(alt_cmd, Code::KeyJ),
            Action::ToggleAttention,
        );
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

        let next_tab = item(
            t("Show Next Tab"),
            acc(shift_cmd, Code::BracketRight),
            Action::NextTab,
        );
        let prev_tab = item(
            t("Show Previous Tab"),
            acc(shift_cmd, Code::BracketLeft),
            Action::PrevTab,
        );
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
