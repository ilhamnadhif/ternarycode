//! "Update available": background checks, the status bar item and the update dialog.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{self, CornerRadius, Id, Margin, RichText, Stroke};
use parking_lot::Mutex;
use tc_core::model::Settings;

use super::{dialogs, icons, kit, style};
use crate::i18n::{t, tf};
use crate::update::{self, CURRENT, Prepared, Release, Stage};

/// "12.4 MB".
pub(super) fn human_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 { format!("{mb:.1} MB") } else { format!("{} KB", bytes / 1024) }
}

/// Automatic checks: right after launch, where a new version opens the update dialog by
/// itself, then every few hours while TernaryCode stays open, where it only lights up the status
/// bar (a dialog popping up while you type would take your keys).
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(2);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

enum State {
    Idle,
    Checking,
    UpToDate,
    Error(String),
    Available(Release),
    Downloading(Release),
    Ready(Release, Prepared),
}

enum Msg {
    Checked(Result<Option<Release>, String>),
    /// The download goes for this release instead: it came out after the one offered.
    Newer(Release),
    Downloaded(Result<Prepared, String>),
}

/// What the app should do after drawing the dialog.
pub enum UpdateEvent {
    None,
    /// The update is ready: quit (after the usual confirmation) and let it install.
    Restart,
}

impl Updater {
    /// The version an update is available for (or downloading, or ready), if any.
    /// An update is being downloaded.
    pub fn downloading(&self) -> bool {
        matches!(self.state, State::Downloading(_))
    }

    pub fn available_version(&self) -> Option<String> {
        match &self.state {
            State::Available(r) | State::Downloading(r) | State::Ready(r, _) => Some(r.version.clone()),
            _ => None,
        }
    }

    /// The release an update is available for (or downloading, or ready): Settings → Updates
    /// shows its notes.
    pub fn available_release(&self) -> Option<&Release> {
        match &self.state {
            State::Available(r) | State::Downloading(r) | State::Ready(r, _) => Some(r),
            _ => None,
        }
    }
}

pub struct Updater {
    state: State,
    rx: Option<mpsc::Receiver<Msg>>,
    stage: Arc<Mutex<Stage>>,
    pub dialog_open: bool,
    next_check: Instant,
    /// No check has answered yet in this run: an update it finds is shown in the dialog.
    launch_check: bool,
    /// The dialog opens for a new version as soon as no other dialog is on screen.
    popup: bool,
    /// The version already announced (dialog or toast).
    announced: Option<String>,
    /// "Don't show again for this version" is ticked in the dialog.
    skip_ticked: bool,
    /// A version to skip from now on, for the settings.
    skipped: Option<String>,
    /// The previous run's update was reported and its leftovers cleaned (first frame).
    started: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: State::Idle,
            rx: None,
            stage: Arc::default(),
            dialog_open: false,
            next_check: Instant::now() + FIRST_CHECK_AFTER,
            launch_check: true,
            popup: false,
            announced: None,
            skip_ticked: false,
            skipped: None,
            started: false,
        }
    }
}

/// Automatic checks: when turned on, and in development builds only against a test feed (the
/// end-to-end checks start the app many times).
fn auto_checks(settings: &Settings) -> bool {
    settings.check_updates && (!cfg!(debug_assertions) || std::env::var_os("TERNARYCODE_UPDATE_FEED").is_some())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Updater {
    fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Starts a check unless one is running; whoever is waiting when it ends gets its answer.
    fn start_check(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Checked(update::check()));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    /// "Check for Updates…" from the menu.
    pub fn check_now(&mut self, ctx: &egui::Context) {
        self.dialog_open = true;
        match self.state {
            State::Downloading(_) => {}
            // Shown as it is, while a check looks for an even newer version.
            State::Available(_) | State::Ready(..) => self.start_check(ctx),
            // An automatic check that is already running answers this one too.
            _ => {
                self.state = State::Checking;
                self.start_check(ctx);
            }
        }
    }

    /// Closes the dialog. A check still running carries on like an automatic one. A version
    /// the user chose not to see again is dropped (no toolbar button either).
    pub fn close_dialog(&mut self) {
        self.dialog_open = false;
        if std::mem::take(&mut self.skip_ticked)
            && let State::Available(release) = &self.state
        {
            self.skipped = Some(release.version.clone());
            self.state = State::Idle;
        }
        if matches!(
            self.state,
            State::UpToDate | State::Error(_) | State::Checking
        ) {
            self.state = State::Idle;
        }
    }

    /// Shows the dialog with an error.
    fn show_error(&mut self, message: String) {
        self.state = State::Error(message);
        self.dialog_open = true;
    }

    /// First frame: reports how the update the previous run installed went, and removes what
    /// an interrupted one left behind.
    fn report_last_update(&mut self) -> Option<String> {
        std::thread::spawn(update::remove_leftovers);
        match update::take_outcome()? {
            update::Outcome::Installed(version) => (version == CURRENT)
                .then(|| tf("TernaryCode was updated to version {}.", &[CURRENT])),
            update::Outcome::Cancelled => {
                self.show_error(
                    t("The update was not installed because no administrator password was entered.")
                        .to_string(),
                );
                None
            }
            update::Outcome::Failed(reason) => {
                self.show_error(tf("The update could not be installed: {}", &[&reason]));
                None
            }
        }
    }

    /// Runs every frame: schedules automatic checks and collects results. Returns a message to
    /// show as a toast. `can_popup`: no other dialog is open, so the update dialog may appear.
    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        can_popup: bool,
    ) -> Option<String> {
        let mut toast = None;
        if let Some(version) = self.skipped.take() {
            settings.skipped_version = Some(version);
        }
        if !self.started {
            self.started = true;
            toast = self.report_last_update();
        }
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(msg) => {
                    if !matches!(msg, Msg::Newer(_)) {
                        self.rx = None;
                    }
                    match msg {
                        Msg::Newer(release) => {
                            if let State::Downloading(r) = &mut self.state {
                                self.announced = Some(release.version.clone());
                                *r = release;
                            }
                        }
                        Msg::Checked(result) => {
                            // Someone is looking at "Checking for updates…" (whichever check it was).
                            let manual =
                                self.dialog_open && matches!(self.state, State::Idle | State::Checking);
                            let at_launch = std::mem::take(&mut self.launch_check);
                            settings.last_update_check = Some(now_secs());
                            match result {
                                Ok(Some(release)) => {
                                    let skipped = settings.skipped_version.as_deref()
                                        == Some(release.version.as_str());
                                    // A version already offered (or downloaded) is only replaced by a
                                    // newer one.
                                    let held = match &self.state {
                                        State::Available(r)
                                        | State::Downloading(r)
                                        | State::Ready(r, _) => Some(r.version.as_str()),
                                        _ => None,
                                    };
                                    let newer = held.is_none_or(|v| update::is_newer(&release.version, v));
                                    if newer && (manual || !skipped) {
                                        if let State::Ready(_, prepared) = &self.state {
                                            let dir = prepared.dir.clone();
                                            std::thread::spawn(move || std::fs::remove_dir_all(dir));
                                        }
                                        if self.announced.as_deref() != Some(&release.version) && !manual {
                                            if at_launch {
                                                self.popup = true;
                                            } else if !self.dialog_open {
                                                toast = Some(tf(
                                                    "TernaryCode {} is available: click Update in the status bar to install it.",
                                                    &[&release.version],
                                                ));
                                            }
                                        }
                                        self.announced = Some(release.version.clone());
                                        self.state = State::Available(release);
                                    }
                                }
                                Ok(None) if manual => self.state = State::UpToDate,
                                Err(e) if manual => self.state = State::Error(e),
                                // Automatic checks fail silently.
                                Ok(None) | Err(_) => {}
                            }
                        }
                        Msg::Downloaded(result) => {
                            match (std::mem::replace(&mut self.state, State::Idle), result) {
                                (State::Downloading(release), Ok(prepared)) => {
                                    self.state = State::Ready(release, prepared);
                                    self.dialog_open = true;
                                }
                                (_, Err(e)) => {
                                    self.state =
                                        State::Error(tf("The update could not be installed: {}", &[&e]));
                                    self.dialog_open = true;
                                }
                                (other, Ok(_)) => self.state = other,
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    if matches!(self.state, State::Checking | State::Downloading(_)) {
                        self.state = State::Error(t("The update task stopped unexpectedly.").to_string());
                    }
                }
            }
        }
        // A new version found at launch: shown once nothing else is on screen.
        if self.popup && can_popup {
            self.popup = false;
            if matches!(self.state, State::Available(_)) {
                self.dialog_open = true;
            }
        }
        // Automatic checks.
        let now = Instant::now();
        // They go on while an update waits, so a newer release replaces it.
        if auto_checks(settings)
            && now >= self.next_check
            && !self.busy()
            && matches!(
                self.state,
                State::Idle | State::Available(_) | State::Ready(..)
            )
        {
            self.next_check = now + CHECK_EVERY;
            self.start_check(ctx);
        }
        if matches!(self.state, State::Downloading(_)) {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else if auto_checks(settings) && !self.busy() {
            // Wake up for the next scheduled check even when nothing else happens.
            let wait = self.next_check.saturating_duration_since(now);
            ctx.request_repaint_after(
                wait.clamp(Duration::from_millis(50), Duration::from_secs(600)),
            );
        }
        toast
    }

    /// What the status bar shows while an update is available, downloading or ready: its
    /// text. None otherwise.
    pub fn status_text(&self) -> Option<String> {
        Some(match &self.state {
            State::Available(r) => tf("Update to {}", &[&r.version]),
            State::Downloading(_) => match *self.stage.lock() {
                Stage::Downloading { done, total } if total > 0 => {
                    tf("Updating… {}%", &[&format!("{}", (done as f64 / total as f64 * 100.0).floor())])
                }
                _ => t("Updating…").to_string(),
            },
            State::Ready(..) => t("Restart to Update").to_string(),
            _ => return None,
        })
    }

    /// Opens the dialog (the status bar item was clicked).
    pub fn open_dialog(&mut self) {
        self.dialog_open = true;
    }

    fn start_download(&mut self, ctx: &egui::Context, release: Release) {
        let (tx, rx) = mpsc::channel();
        let stage = self.stage.clone();
        *stage.lock() = Stage::Starting;
        let ctx = ctx.clone();
        let offered = release.clone();
        std::thread::spawn(move || {
            // Always the latest: a release that came out since this one was found (TernaryCode
            // open for days) is installed instead, not after another update.
            let release = match update::check() {
                Ok(Some(latest)) if update::is_newer(&latest.version, &offered.version) => {
                    let _ = tx.send(Msg::Newer(latest.clone()));
                    latest
                }
                _ => offered,
            };
            let _ = tx.send(Msg::Downloaded(update::download(&release, &stage)));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
        self.state = State::Downloading(release);
    }

    /// The downloaded update, ready to install.
    pub fn prepared(&self) -> Option<Prepared> {
        match &self.state {
            State::Ready(_, prepared) => Some(prepared.clone()),
            _ => None,
        }
    }

    /// Hands the downloaded app to the installer helper; the app must quit right after.
    pub fn launch_installer(&self) -> Result<(), String> {
        let State::Ready(_, prepared) = &self.state else {
            return Err("no update is ready".into());
        };
        update::launch_installer(prepared)
    }

    /// `running_count` / `running`: how many agents work now and their projects, for the promise
    /// that they keep running.
    pub fn show_dialog(&mut self, ctx: &egui::Context, settings: &mut Settings, running_count: usize, running: &[String]) -> UpdateEvent {
        if !self.dialog_open {
            return UpdateEvent::None;
        }
        let mut event = UpdateEvent::None;
        let mut close = false;
        let mut retry = false;
        let mut install_release = None;
        let screen = ctx.content_rect();
        let dark = ctx.global_style().visuals.dark_mode;
        let p = style::palette(dark);
        let width = if matches!(self.state, State::Available(_)) {
            (screen.width() - 32.0).clamp(360.0, 560.0)
        } else {
            (screen.width() - 32.0).clamp(340.0, 460.0)
        };
        // Enter answers only the dialog asked for (up to date). The others can open by
        // themselves (found at launch, downloaded, failed) while the user types into a
        // terminal, and an Enter meant for it must not install or restart: they take a click
        // (Esc still closes them).
        let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let modal = egui::Modal::new(Id::new("update-dialog"))
            .frame(dialogs::frame(ctx))
            .backdrop_color(dialogs::scrim(dark))
            .show(ctx, |ui| {
                ui.set_width(width);
                match &self.state {
                    State::Idle | State::Checking => {
                        if dialogs::header(ui, icons::ARROW_CIRCLE_UP, p.accent, t("Software Update"), Some(&tf("TernaryCode {} is installed.", &[CURRENT]))) {
                            close = true;
                        }
                        dialogs::body(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new(t("Checking for updates…")).color(p.text_muted));
                            });
                        });
                        dialogs::footer_with(ui, None, |ui| {
                            if kit::secondary(ui, None, t("Cancel")).clicked() {
                                close = true;
                            }
                        });
                    }
                    State::UpToDate => {
                        if dialogs::header(ui, icons::CHECK_CIRCLE, p.ok, t("TernaryCode is up to date"), Some(&tf("Version {} is the latest.", &[CURRENT]))) {
                            close = true;
                        }
                        ui.add_space(12.0);
                        dialogs::footer_with(ui, Some(&tf("↵ {}", &[t("OK")])), |ui| {
                            if kit::button_ex(ui, kit::Kind::Primary, None, t("OK"), false).clicked() || enter {
                                close = true;
                            }
                        });
                    }
                    State::Error(e) => {
                        let sub = format!("{}. {}", e.trim_end_matches('.'), t("Nothing was changed."));
                        if dialogs::header(ui, icons::WARNING, p.err, t("Software Update Failed"), Some(&sub)) {
                            close = true;
                        }
                        ui.add_space(12.0);
                        dialogs::footer_left(
                            ui,
                            |ui| {
                                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_SQUARE_OUT), t("Open Releases Page"), false).clicked() {
                                    update::open_page(&format!("https://github.com/{}/releases/latest", update::REPO));
                                }
                            },
                            |ui| {
                                if kit::button_ex(ui, kit::Kind::Primary, None, t("OK"), false).clicked() {
                                    close = true;
                                }
                                if kit::secondary(ui, None, t("Try Again")).clicked() {
                                    retry = true;
                                }
                            },
                        );
                    }
                    State::Available(release) => {
                        let mut sub = vec![tf("you have {}", &[CURRENT])];
                        if let Some(date) = &release.date {
                            sub.push(tf("released {}", &[date]));
                        }
                        if let Some(size) = release.download.as_ref().map(|a| a.size).filter(|s| *s > 0) {
                            sub.push(human_size(size));
                        }
                        if update_head(ui, &tf("TernaryCode {} is available", &[&release.version]), &sub.join(" · ")) {
                            close = true;
                        }
                        let installable = update::installable(release);
                        dialogs::body(ui, |ui| {
                            let notes_h = (screen.height() - 470.0).clamp(110.0, 300.0);
                            release_notes(ui, &release.notes, notes_h);
                            if !installable {
                                ui.add_space(6.0);
                                ui.add(egui::Label::new(RichText::new(update::manual_reason()).size(style::SMALL).color(p.text_muted)).wrap());
                            } else {
                                ui.add_space(12.0);
                                promise(ui, running_count, running);
                                ui.add_space(10.0);
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    ui.label(RichText::new(icons::SHIELD_CHECK).font(style::icon_fill(13.0)).color(p.ok));
                                    let note = if cfg!(target_os = "macos") {
                                        t("Downloaded from GitHub, then its size, SHA-256 and code signature are checked before anything is replaced.")
                                    } else {
                                        t("Downloaded from GitHub, then its size and SHA-256 are checked before anything is replaced.")
                                    };
                                    ui.add(egui::Label::new(RichText::new(note).size(12.0).color(p.text_faint)).wrap());
                                });
                            }
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let skip = kit::switch(ui, &mut self.skip_ticked);
                                #[cfg(debug_assertions)]
                                crate::devtest::record("update-skip", skip.rect);
                                let _ = skip;
                                ui.label(RichText::new(tf("Don’t remind me about {}", &[&release.version])).size(style::SMALL).color(p.text_muted));
                            });
                        });
                        dialogs::footer_with(ui, Some(t("Later keeps it in the status bar.")), |ui| {
                            if installable {
                                let install = kit::button_keys(ui, kit::Kind::Primary, None, t("Install Update"), None, false);
                                #[cfg(debug_assertions)]
                                crate::devtest::record("update-install", install.rect);
                                if install.clicked() {
                                    install_release = Some(release.clone());
                                }
                            } else if kit::button_ex(ui, kit::Kind::Primary, None, t("Download…"), false).clicked() {
                                update::open_page(&release.page);
                                close = true;
                            }
                            let later = kit::secondary(ui, None, t("Later"));
                            #[cfg(debug_assertions)]
                            crate::devtest::record("update-close", later.rect);
                            if later.clicked() {
                                close = true;
                            }
                            if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_SQUARE_OUT), t("View on GitHub"), false).clicked() {
                                update::open_page(&release.page);
                            }
                        });
                    }
                    State::Downloading(release) => {
                        if dialogs::header(ui, icons::DOWNLOAD_SIMPLE, p.accent, &tf("Updating to TernaryCode {}", &[&release.version]), None) {
                            close = true;
                        }
                        let stage = self.stage.lock().clone();
                        let (fraction, text) = match stage {
                            Stage::Starting => (0.0, t("Starting…").to_string()),
                            Stage::Downloading { done, total } => (
                                if total > 0 { done as f32 / total as f32 } else { 0.0 },
                                tf("Downloading… {} of {}", &[&human_size(done), &human_size(total)]),
                            ),
                            Stage::Verifying => (1.0, t("Verifying…").to_string()),
                        };
                        let verifying = matches!(stage, Stage::Verifying);
                        dialogs::body(ui, |ui| {
                            let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
                            ui.painter().rect_filled(bar, egui::CornerRadius::same(3), p.border_strong);
                            ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * fraction.clamp(0.0, 1.0), 6.0)), egui::CornerRadius::same(3), p.accent);
                            ui.add_space(12.0);
                            // The steps: each running (the lattice), done (a check) or to come.
                            let step = |ui: &mut egui::Ui, state: u8, label: &str, data: &str| {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
                                let cy = r.center().y;
                                let c = egui::pos2(r.left() + 7.0, cy);
                                match state {
                                    2 => {
                                        ui.painter().text(c, egui::Align2::CENTER_CENTER, icons::CHECK_CIRCLE, style::icon_fill(14.0), p.ok);
                                    }
                                    1 => kit::paint_lattice(ui.painter(), c, 10.0, p.info, true),
                                    _ => {
                                        ui.painter().circle_stroke(c, 5.5, Stroke::new(1.0, p.text_ghost));
                                    }
                                }
                                let tc = if state == 0 { p.text_ghost } else { p.text };
                                ui.painter().text(egui::pos2(r.left() + 22.0, cy), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), tc);
                                if !data.is_empty() {
                                    ui.painter().text(egui::pos2(r.right(), cy), egui::Align2::RIGHT_CENTER, data, style::mono(11.0), p.text_faint);
                                }
                            };
                            let pct = format!("{}%", (fraction * 100.0).round() as i32);
                            step(ui, if verifying { 2 } else { 1 }, &text, if verifying { "" } else { &pct });
                            let verify_label = if cfg!(target_os = "macos") {
                                t("Verify size, SHA-256, signature")
                            } else {
                                t("Verify size and SHA-256")
                            };
                            step(ui, if verifying { 1 } else { 0 }, verify_label, "");
                            let finish_label = if update::restarts_in_place() {
                                t("Restart in place")
                            } else {
                                t("Install and reopen")
                            };
                            step(ui, 0, finish_label, "");
                        });
                        dialogs::footer_with(ui, Some(t("It keeps going in the background.")), |ui| {
                            if kit::secondary(ui, None, t("Hide")).clicked() {
                                close = true;
                            }
                        });
                    }
                    State::Ready(release, _) => {
                        let ready_sub = if update::restarts_in_place() {
                            t("TernaryCode restarts in place: your projects, tabs and running agents stay open.")
                        } else {
                            t("TernaryCode restarts with your projects and tabs. Agents that support it can be resumed.")
                        };
                        if dialogs::header(
                            ui,
                            icons::CHECK_CIRCLE,
                            p.ok,
                            &tf("TernaryCode {} is ready", &[&release.version]),
                            Some(ready_sub),
                        ) {
                            close = true;
                        }
                        ui.add_space(12.0);
                        dialogs::footer_with(ui, None, |ui| {
                            let restart = kit::button_ex(ui, kit::Kind::Primary, None, t("Restart Now"), false);
                            #[cfg(debug_assertions)]
                            crate::devtest::record("update-restart", restart.rect);
                            if restart.clicked() {
                                event = UpdateEvent::Restart;
                                close = true;
                            }
                            if kit::secondary(ui, None, t("Later")).clicked() {
                                close = true;
                            }
                        });
                    }
                }
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(release) = install_release {
            self.skip_ticked = false;
            self.start_download(ctx, release);
        }
        if retry {
            self.check_now(ctx);
        }
        if close {
            self.close_dialog();
            if let Some(version) = self.skipped.take() {
                settings.skipped_version = Some(version);
            }
        }
        event
    }
}

impl crate::app::App {
    /// Runs every frame after the dialogs: the updater's checks, its toasts and its dialog.
    pub fn updates(&mut self, ctx: &egui::Context) {
        let before = (self.settings.skipped_version.clone(), self.settings.last_update_check);
        let can_popup = self.dialog.is_none() && !self.review_full() && self.palette.is_none();
        if let Some(text) = self.updater.tick(ctx, &mut self.settings, can_popup) {
            self.toast(text);
        }
        // The projects where an agent works now (they keep running through a restart in place).
        let mut running: Vec<String> = self.panes.values().filter(|x| x.tracker.agent.is_some() && x.alive()).map(|x| self.project_label(x.project)).collect();
        let running_count = running.len();
        running.sort();
        running.dedup();
        if let UpdateEvent::Restart = self.updater.show_dialog(ctx, &mut self.settings, running_count, &running) {
            self.actions.push(crate::actions::Action::RestartToUpdate);
        }
        if (self.settings.skipped_version.clone(), self.settings.last_update_check) != before {
            self.settings_changed();
        }
    }

    /// The status bar's update item, while there is an update to install.
    pub fn update_status_item(&mut self, ui: &mut egui::Ui) {
        let Some(text) = self.updater.status_text() else { return };
        let p = style::pal(ui);
        let font = style::medium(style::SMALL);
        let glyph = if self.updater.downloading() { icons::DOWNLOAD_SIMPLE } else { icons::ARROW_CIRCLE_UP };
        let icon = ui.painter().layout_no_wrap(glyph.to_string(), style::icon_font(13.0), p.accent_text);
        let label = ui.painter().layout_no_wrap(text, font, p.accent_text);
        let size = egui::vec2(7.0 + icon.size().x + 5.0 + label.size().x + 9.0, 20.0);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
        let fill = if resp.hovered() { p.accent_soft.gamma_multiply(1.4) } else { p.accent_soft };
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        let mut x = rect.left() + 7.0;
        ui.painter().galley(egui::pos2(x, rect.center().y - icon.size().y / 2.0), icon.clone(), p.accent_text);
        x += icon.size().x + 5.0;
        ui.painter().galley(egui::pos2(x, rect.center().y - label.size().y / 2.0), label, p.accent_text);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        #[cfg(debug_assertions)]
        crate::devtest::record("update-button", resp.rect);
        if resp.on_hover_text(t("A new version of TernaryCode is available")).clicked() {
            self.actions.push(crate::actions::Action::ShowUpdate);
        }
        ui.add_space(4.0);
    }
}

/// Marks where the release notes end and the web-only part (how to install) begins.
const NOTES_END: &str = "<!-- ternarycode:end-of-notes -->";

/// One piece of the release notes.
#[derive(Debug, PartialEq)]
enum Block {
    Heading(String),
    /// A list item, with its nesting level.
    Item(usize, String),
    Paragraph(String),
    Code(String),
}

/// A small Markdown reader for release notes: headings, (nested) lists, paragraphs and code
/// blocks; lines that continue a list item or paragraph are joined to it. Stops where the
/// install instructions begin.
fn parse_notes(notes: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut in_code = false;
    // The previous line continues: a list item or paragraph that a next line may extend.
    let mut open = false;
    for line in notes.lines() {
        let line = line.trim_end();
        if line.trim() == NOTES_END || line == "## Install" {
            break;
        }
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            open = false;
            continue;
        }
        if in_code {
            blocks.push(Block::Code(line.to_string()));
            continue;
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.is_empty() || trimmed.starts_with("<!--") {
            open = false;
        } else if trimmed.starts_with('#') {
            blocks.push(Block::Heading(
                trimmed.trim_start_matches('#').trim().to_string(),
            ));
            open = false;
        } else if let Some(text) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            blocks.push(Block::Item(indent / 2, text.trim().to_string()));
            open = true;
        } else if open
            && let Some(Block::Item(_, text) | Block::Paragraph(text)) = blocks.last_mut()
        {
            text.push(' ');
            text.push_str(trimmed);
        } else {
            blocks.push(Block::Paragraph(trimmed.to_string()));
            open = true;
        }
    }
    blocks
}

/// Inline Markdown: **bold** stands out, `code` is monospace, links show their text.
fn inline(ui: &egui::Ui, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let normal = ui.visuals().text_color();
    let strong = ui.visuals().strong_text_color();
    // [text](url) → text
    let mut plain = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        match after.find("](").and_then(|close| {
            after[close + 2..]
                .find(')')
                .map(|end| (close, close + 2 + end))
        }) {
            Some((close, end)) => {
                plain.push_str(&rest[..open]);
                plain.push_str(&after[..close]);
                rest = &after[end + 1..];
            }
            None => {
                plain.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    plain.push_str(rest);
    let (mut bold, mut code) = (false, false);
    let mut chunk = String::new();
    let mut chars = plain.chars().peekable();
    let flush = |job: &mut egui::text::LayoutJob, chunk: &mut String, bold: bool, code: bool| {
        if chunk.is_empty() {
            return;
        }
        let format = egui::TextFormat {
            font_id: if code { mono.clone() } else { body.clone() },
            color: if bold { strong } else { normal },
            ..Default::default()
        };
        job.append(&std::mem::take(chunk), 0.0, format);
    };
    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'*') && !code {
            chars.next();
            flush(&mut job, &mut chunk, bold, code);
            bold = !bold;
        } else if c == '`' {
            flush(&mut job, &mut chunk, bold, code);
            code = !code;
        } else {
            chunk.push(c);
        }
    }
    flush(&mut job, &mut chunk, bold, code);
    job
}

/// Release notes from GitHub (Markdown), in a scrolling box.
/// The update window's head: the app's icon, the title and what it says under it, ×.
fn update_head(ui: &mut egui::Ui, title: &str, sub: &str) -> bool {
    let p = style::pal(ui);
    let top = ui.max_rect();
    kit::paint_highlight(ui.painter(), egui::Rect::from_min_size(top.min, egui::vec2(top.width(), 1.0)), style::RADIUS_DIALOG, &p);
    let mut close = false;
    egui::Frame::NONE.inner_margin(Margin { left: 20, right: 14, top: 18, bottom: 6 }).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
            super::settings::paint_logo(ui.painter(), r, &p);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(2.0);
                ui.label(RichText::new(title).font(style::semibold(15.0)).color(p.text));
                ui.label(RichText::new(sub).font(style::mono(11.0)).color(p.text_faint));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                close = kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0).clicked();
            });
        });
    });
    close
}

/// The promise of a restart in place: what runs keeps running (where TernaryCode can hand its
/// panes over), else that the panes and tabs come back.
fn promise(ui: &mut egui::Ui, running_count: usize, running: &[String]) {
    let p = style::pal(ui);
    let in_place = update::restarts_in_place();
    let r = egui::Frame::NONE
        .fill(style::soft(p.accent, 0.08))
        .stroke(Stroke::new(1.0, style::soft(p.accent, 0.25)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin { left: 14, right: 14, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (g, _) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover());
                kit::paint_lattice(ui.painter(), g.center(), 14.0, p.info, true);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let title = if in_place { t("Agents and programs keep running through the update") } else { t("Your projects and tabs come back after the update") };
                    ui.label(RichText::new(title).font(style::medium(13.0)).color(p.text));
                    let mut text = String::new();
                    if running_count > 0 && in_place {
                        let list = match running {
                            [one] => one.clone(),
                            [rest @ .., last] => format!("{} {} {last}", rest.join(", "), t("and")),
                            [] => String::new(),
                        };
                        text.push_str(&crate::i18n::count(running_count, &tf("1 agent is working now, in {}.", &[&list]), &tf("{} agents are working now, in {}.", &["{}", &list])));
                        text.push(' ');
                    }
                    text.push_str(if in_place {
                        t("TernaryCode restarts in place: your panes, tabs and pages come back as they are.")
                    } else {
                        t("Agents that support it can be resumed in their panes.")
                    });
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.text_muted).line_height(Some(17.5))).wrap());
                });
            });
        });
    let _ = r;
}

fn release_notes(ui: &mut egui::Ui, notes: &str, max_height: f32) {
    let blocks = parse_notes(notes);
    let p = style::pal(ui);
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(0, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .min_scrolled_height(max_height.min(140.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if blocks.is_empty() {
                        ui.label(RichText::new(t("No release notes.")).color(p.text_muted));
                    }
                    for (i, block) in blocks.iter().enumerate() {
                        match block {
                            Block::Heading(text) => {
                                if i > 0 {
                                    ui.add_space(10.0);
                                }
                                // A heading as a tag: NEW in the accent, IMPROVED in green, the rest
                                // quiet.
                                let upper = text.to_uppercase();
                                let c = if upper.contains("NEW") || upper.contains("ADDED") { p.accent_text } else if upper.contains("IMPROV") || upper.contains("CHANGED") { p.ok } else { p.text_faint };
                                let g = ui.painter().layout_no_wrap(upper, style::mono_semibold(10.0), c);
                                let (r, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 12.0, 18.0), egui::Sense::hover());
                                ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                                ui.painter().galley(r.center() - g.size() / 2.0, g, c);
                                ui.add_space(4.0);
                            }
                            Block::Item(level, text) => {
                                ui.horizontal_top(|ui| {
                                    ui.add_space(4.0 + *level as f32 * 16.0);
                                    ui.label(if *level == 0 { "•" } else { "◦" });
                                    ui.vertical(|ui| {
                                        let job = inline(ui, text);
                                        ui.add(egui::Label::new(job).wrap());
                                    });
                                });
                            }
                            Block::Paragraph(text) => {
                                let job = inline(ui, text);
                                ui.add(egui::Label::new(job).wrap());
                            }
                            Block::Code(text) => {
                                ui.label(RichText::new(text).monospace().small());
                            }
                        }
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            notes: String::new(),
            date: None,
            page: String::new(),
            download: None,
        }
    }

    /// An updater with an automatic check running: the one right after launch, or a later
    /// one. (`started` skips the first-frame report, which reads the real data folder.)
    fn checking_in_background(at_launch: bool) -> (Updater, mpsc::Sender<Msg>) {
        let (tx, rx) = mpsc::channel();
        let updater = Updater {
            rx: Some(rx),
            started: true,
            launch_check: at_launch,
            ..Default::default()
        };
        (updater, tx)
    }

    /// "Check for Updates…" while the automatic check runs, which then answers. Returns the
    /// state the dialog shows.
    fn manual_check_during_automatic_check(answer: Result<Option<Release>, String>) -> State {
        let ctx = egui::Context::default();
        // Shown even when skipped: the user asked.
        let mut settings = Settings {
            skipped_version: Some("9.0.0".into()),
            ..Default::default()
        };
        let (mut updater, tx) = checking_in_background(true);
        updater.check_now(&ctx);
        assert!(matches!(updater.state, State::Checking) && updater.dialog_open);
        tx.send(Msg::Checked(answer)).unwrap();
        assert_eq!(updater.tick(&ctx, &mut settings, false), None);
        assert!(updater.dialog_open && !updater.busy());
        updater.state
    }

    #[test]
    fn manual_check_during_automatic_check_gets_an_answer() {
        assert!(matches!(
            manual_check_during_automatic_check(Ok(None)),
            State::UpToDate
        ));
        assert!(matches!(
            manual_check_during_automatic_check(Err("offline".into())),
            State::Error(e) if e == "offline"
        ));
        assert!(matches!(
            manual_check_during_automatic_check(Ok(Some(release("9.0.0")))),
            State::Available(_)
        ));
    }

    /// Closing the dialog while checking turns the check into an automatic one.
    #[test]
    fn closed_dialog_gets_the_automatic_behavior() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let (mut updater, tx) = checking_in_background(false);
        updater.check_now(&ctx);
        updater.close_dialog();
        assert!(matches!(updater.state, State::Idle) && !updater.dialog_open);
        tx.send(Msg::Checked(Ok(Some(release("9.0.0"))))).unwrap();
        assert!(
            updater.tick(&ctx, &mut settings, true).is_some(),
            "announced"
        );
        assert!(matches!(updater.state, State::Available(_)) && !updater.dialog_open);

        let (mut updater, tx) = checking_in_background(false);
        tx.send(Msg::Checked(Err("offline".into()))).unwrap();
        assert_eq!(updater.tick(&ctx, &mut settings, true), None);
        assert!(matches!(updater.state, State::Idle) && !updater.dialog_open);
    }

    fn prepared(version: &str) -> Prepared {
        Prepared {
            version: version.into(),
            dir: std::path::PathBuf::from("/nonexistent/ternarycode-update-test"),
            file: std::path::PathBuf::new(),
        }
    }

    fn version(state: &State) -> Option<(&'static str, &str)> {
        match state {
            State::Available(r) => Some(("available", &r.version)),
            State::Downloading(r) => Some(("downloading", &r.version)),
            State::Ready(r, _) => Some(("ready", &r.version)),
            _ => None,
        }
    }

    /// TernaryCode stays open while two releases come out: the second replaces the first, even one
    /// already downloaded, and finding the same version again changes nothing.
    #[test]
    fn a_newer_release_replaces_the_one_offered() {
        let ctx = egui::Context::default();
        let check = |state: State, found: &str| {
            let (mut updater, tx) = checking_in_background(false);
            updater.state = state;
            tx.send(Msg::Checked(Ok(Some(release(found))))).unwrap();
            let toast = updater.tick(&ctx, &mut Settings::default(), true);
            (updater, toast)
        };
        let (updater, toast) = check(State::Available(release("9.0.0")), "9.0.1");
        assert_eq!(version(&updater.state), Some(("available", "9.0.1")));
        assert!(toast.is_some(), "announced");
        let (updater, _) = check(State::Ready(release("9.0.0"), prepared("9.0.0")), "9.0.1");
        assert_eq!(version(&updater.state), Some(("available", "9.0.1")));
        let (updater, toast) = check(State::Ready(release("9.0.1"), prepared("9.0.1")), "9.0.1");
        assert_eq!(version(&updater.state), Some(("ready", "9.0.1")));
        assert_eq!(toast, None);
        let (updater, _) = check(State::Available(release("9.0.1")), "9.0.0");
        assert_eq!(
            version(&updater.state),
            Some(("available", "9.0.1")),
            "never older"
        );
    }

    /// Installing an update goes for the release that came out after the one offered.
    #[test]
    fn the_download_goes_for_the_newest_release() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let (mut updater, tx) = checking_in_background(false);
        updater.state = State::Downloading(release("9.0.0"));
        tx.send(Msg::Newer(release("9.0.1"))).unwrap();
        updater.tick(&ctx, &mut settings, true);
        assert!(updater.busy(), "still downloading");
        assert_eq!(version(&updater.state), Some(("downloading", "9.0.1")));
        tx.send(Msg::Downloaded(Ok(prepared("9.0.1")))).unwrap();
        updater.tick(&ctx, &mut settings, true);
        assert_eq!(version(&updater.state), Some(("ready", "9.0.1")));
        assert!(updater.dialog_open && !updater.busy());
    }

    #[test]
    fn notes_are_read_like_markdown() {
        let notes = "## Fixes\n\n- **TernaryCode no longer quits** when you close\n  a tab.\n- Dialogs\n  - Esc closes `Export` again.\n\nSee [the page](https://x) for more.\n\n<!-- ternarycode:end-of-notes -->\n\n## Install\n\ncurl …";
        assert_eq!(
            parse_notes(notes),
            [
                Block::Heading("Fixes".into()),
                Block::Item(
                    0,
                    "**TernaryCode no longer quits** when you close a tab.".into()
                ),
                Block::Item(0, "Dialogs".into()),
                Block::Item(1, "Esc closes `Export` again.".into()),
                Block::Paragraph("See [the page](https://x) for more.".into()),
            ]
        );
        // Releases made before the marker existed stop at their install section.
        assert_eq!(
            parse_notes("- One\n\n## Install\n\n- curl"),
            [Block::Item(0, "One".into())]
        );
    }

    /// A new version found right after launch opens the dialog, once no other dialog is up;
    /// a skipped version doesn't, and later checks only announce it.
    #[test]
    fn launch_check_opens_the_dialog() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let (mut updater, tx) = checking_in_background(true);
        tx.send(Msg::Checked(Ok(Some(release("9.0.0"))))).unwrap();
        assert_eq!(updater.tick(&ctx, &mut settings, false), None, "no toast");
        assert!(!updater.dialog_open, "waits for the other dialog");
        assert_eq!(updater.tick(&ctx, &mut settings, true), None);
        assert!(updater.dialog_open && matches!(updater.state, State::Available(_)));
        updater.close_dialog();
        assert_eq!(updater.tick(&ctx, &mut settings, true), None);
        assert!(!updater.dialog_open, "Later: not shown again in this run");

        let mut settings = Settings {
            skipped_version: Some("9.0.0".into()),
            ..Default::default()
        };
        let (mut updater, tx) = checking_in_background(true);
        tx.send(Msg::Checked(Ok(Some(release("9.0.0"))))).unwrap();
        assert_eq!(updater.tick(&ctx, &mut settings, true), None);
        assert!(!updater.dialog_open && matches!(updater.state, State::Idle));
    }
}
