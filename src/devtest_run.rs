//! Debug builds: runs `TERNARYCODE_DEV_SCRIPT` steps against the real app (see `devtest.rs` for
//! input injection). Results go to stderr as `DEVTEST PASS|FAIL …`, then `DEVTEST DONE`.
//!
//! Steps (separated by `;;`; the ones with an argument format are explained where they run):
//! - input: `sleep MS`, `key SPEC`, `keys TEXT`, `click NAME`, `mouse down|drag|up NAME[@FX,FY]`,
//!   `scroll NAME[@FX,FY] DY`, `focus-pane N`, `action NAME [ARG]`, `goto PATH LINE COL`,
//!   `open-file PATH[:LINE]`, `close-window`, `clear-line`, `free-editor`, `shot NAME`, `quit`;
//! - files: `write-file PATH TEXT`, `rm-file PATH`, `expect-file PATH TEXT`,
//!   `expect-not-file PATH TEXT`;
//! - waits (until true, or fail after a while): `wait-screen TEXT`, `wait-status STATUS`,
//!   `wait-queue N`, `wait-toast TEXT`, `wait-editor TEXT`, `wait-conflict`, `wait-review KINDS`,
//!   `wait-snapshot`, `wait-tree|wait-git|wait-open PATH [LETTER]`, `wait-tabs SPEC`,
//!   `wait-find N`, `wait-caret LINE`, `wait-active NAME`, `wait-diag ERRORS WARNINGS`,
//!   `wait-completion`, `wait-hover`, `wait-signature`, `wait-rename`, `wait-locations N`,
//!   `wait-lsp STATE`, `wait-doc FACTS`;
//! - checks: `expect-project NAME`, `expect-alive N`, `expect-started N`, `expect-fill PANE`,
//!   `expect-resizes PANE N`, `expect-width NAME MIN MAX`, `expect-rect NAME`,
//!   `expect-typing PROJECT`, `expect-screen-count TEXT N`, `expect-no-line TEXT`,
//!   `expect-shown NAME`, `expect-hidden NAME`, `expect-dialog open|closed`;
//! - timing and logs: `hook-latency AGENT EVENT STATUS [KIND]`, `hook PANE AGENT EVENT [KIND
//!   [MESSAGE]]`, `echo-latency`, `frames N LABEL`, `resizes LABEL`, `log-mem LABEL`,
//!   `explorer-reads LABEL`, `dump-pane PANE`, `log-review`, `log TEXT`.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use eframe::egui;
use tc_core::agent::AgentKind;
use tc_core::status::Status;
use tc_term::alacritty_terminal::grid::Dimensions;
use tc_term::alacritty_terminal::index::{Column, Line};

use crate::actions::Action;
use crate::app::App;
use crate::devtest;

struct Wait {
    step: String,
    deadline: Instant,
    started: Instant,
}

thread_local! {
    static WAIT: RefCell<Option<Wait>> = const { RefCell::new(None) };
    static SHOT: RefCell<Option<String>> = const { RefCell::new(None) };
    /// `frames N label`: how long building the UI takes, over the next N frames.
    static FRAMES: RefCell<Option<(usize, Vec<std::time::Duration>, String)>> = const { RefCell::new(None) };
}

/// Called after each frame's UI is built: collects times for `frames` and keeps frames coming
/// until enough are measured.
pub(crate) fn frame_drawn(ctx: &egui::Context, took: std::time::Duration) {
    FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        let Some((want, times, label)) = f.as_mut() else { return };
        times.push(took);
        if times.len() < *want {
            ctx.request_repaint();
            return;
        }
        let mut ms: Vec<f64> = times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms.sort_by(f64::total_cmp);
        let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
        eprintln!(
            "DEVTEST FRAMES {label}: n={} p50={:.2} ms p95={:.2} ms max={:.2} ms",
            ms.len(),
            at(0.5),
            at(0.95),
            ms[ms.len() - 1]
        );
        *f = None;
    });
}

/// A minimal PNG writer (stored deflate blocks, no compression), enough for test screenshots.
fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    let [w, h] = image.size;
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let c = image.pixels[y * w + x];
            raw.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
        }
    }
    // zlib stream of stored blocks.
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

const WAIT_LIMIT: Duration = Duration::from_secs(8);

fn status_slug(s: Status) -> &'static str {
    match s {
        Status::Idle => "idle",
        Status::Running => "running",
        Status::NeedsInput => "needs-input",
        Status::Failed(_) => "failed",
        Status::Done => "done",
        Status::CommandDone => "command-done",
        Status::Unknown => "unknown",
        Status::Ended => "ended",
    }
}

pub fn screen_text(shared: &tc_term::core::TermShared) -> String {
    let term = shared.term.lock();
    (0..term.screen_lines() as i32)
        .map(|l| {
            let row = &term.grid()[Line(l)];
            (0..term.columns())
                .map(|c| row[Column(c)].c)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl App {
    fn current_screen(&self) -> String {
        self.current_pane()
            .and_then(|id| self.panes.get(&id))
            .map(|p| screen_text(&p.shared))
            .unwrap_or_default()
    }

    fn current_status(&self) -> Option<Status> {
        self.current_pane()
            .and_then(|id| self.panes.get(&id))
            .map(|p| p.tracker.status)
    }

    /// Checks a waiting condition; `Some(ok)` once decided.
    fn wait_check(&self, step: &str) -> Option<bool> {
        let (cmd, arg) = step.split_once(' ').unwrap_or((step, ""));
        match cmd {
            "wait-screen" => Some(self.current_screen().contains(arg)).filter(|ok| *ok),
            // A widget drawn in the last frame, or not drawn any more.
            "wait-shown" => Some(devtest::shown(arg.trim())).filter(|ok| *ok),
            "wait-hidden" => Some(!devtest::shown(arg.trim())).filter(|ok| *ok),
            "wait-status" | "hook-latency" => {
                let want = if cmd == "hook-latency" {
                    arg.split_whitespace().nth(2).unwrap_or("")
                } else {
                    arg.trim()
                };
                let now = self.current_status().map(status_slug).unwrap_or("");
                Some(want.split('|').any(|w| w == now)).filter(|ok| *ok)
            }
            "wait-queue" => {
                Some(self.attention_queue().len().to_string() == arg.trim()).filter(|ok| *ok)
            }
            "wait-toast" => {
                Some(self.toast.as_ref().is_some_and(|t| arg.trim().split('|').any(|w| t.text.contains(w)))).filter(|ok| *ok)
            }
            "wait-editor" => {
                let text = self.active.and_then(|id| self.editors.get(&id)).and_then(|e| e.file()).map(|f| f.text.clone()).unwrap_or_default();
                Some(text.contains(&devtest::unescape(arg))).filter(|ok| *ok)
            }
            "wait-review" => {
                let r = self.review.as_ref();
                let kinds: String = r
                    .map(|r| {
                        let mut v: Vec<String> = r.files.iter().map(|f| format!("{:?}:{}", f.kind, f.path)).collect();
                        v.sort();
                        v.join(",")
                    })
                    .unwrap_or_default();
                Some(r.is_some_and(|r| !r.loading) && kinds == arg.trim()).filter(|ok| *ok)
            }
            "wait-snapshot" => {
                let done = self.current_pane().and_then(|id| self.panes.get(&id)).is_some_and(|p| p.run_snapshot.is_some());
                Some(done).filter(|ok| *ok)
            }
            // `wait-doc kind=pdf pages=4 rendered=1`: the editor's document, as `DocView::facts`
            // tells it (every word given must be among them).
            "wait-doc" => {
                let f = self.editors.get(&self.active?)?.file()?;
                let facts = f.doc.as_ref()?.facts();
                // `rendered>0`: a number above; else the fact as written.
                let holds = |w: &str| match w.split_once('>') {
                    Some((k, v)) => facts.split(' ').filter_map(|x| x.split_once('=')).any(|(fk, fv)| {
                        fk == k && fv.parse::<i64>().ok().zip(v.parse::<i64>().ok()).is_some_and(|(a, b)| a > b)
                    }),
                    None => facts.split(' ').any(|x| x == w),
                };
                Some(arg.split_whitespace().all(holds)).filter(|ok| *ok)
            }
            "wait-conflict" => {
                let c = self.active.and_then(|id| self.editors.get(&id)).and_then(|e| e.file()).is_some_and(|f| f.conflict);
                Some(c).filter(|ok| *ok)
            }
            // `wait-active main.rs`: the name of the active editor file.
            "wait-active" => {
                let f = self.editors.get(&self.active?)?.file()?;
                Some(f.name() == arg.trim()).filter(|ok| *ok)
            }
            // `wait-diag 1 0`: errors and warnings in the active file.
            "wait-diag" => {
                let f = self.editors.get(&self.active?)?.file()?;
                let got = self.lsp.counts(&f.path);
                let want: Vec<usize> = arg.split_whitespace().filter_map(|n| n.parse().ok()).collect();
                Some(want == [got.0, got.1]).filter(|ok| *ok)
            }
            "wait-completion" => Some(self.lsp.ide.completion.as_ref().is_some_and(|c| !c.shown.is_empty())).filter(|ok| *ok),
            "wait-hover" => Some(self.lsp.ide.hover.is_some()).filter(|ok| *ok),
            "wait-signature" => Some(self.lsp.ide.signature.is_some()).filter(|ok| *ok),
            "wait-rename" => Some(self.lsp.ide.rename.is_some()).filter(|ok| *ok),
            // `wait-locations 2`: places listed in the palette (references, symbols…).
            "wait-locations" => {
                let n: usize = arg.trim().parse().ok()?;
                Some(self.palette.as_ref().is_some_and(|p| p.locations.len() == n)).filter(|ok| *ok)
            }
            // `wait-lsp ready|starting|missing|failed`: the active file's language server.
            "wait-lsp" => {
                let f = self.editors.get(&self.active?)?.file()?;
                let state = match &self.lsp.server_for(&f.path)?.state {
                    crate::lsp::ServerState::Ready => "ready",
                    crate::lsp::ServerState::Starting => "starting",
                    crate::lsp::ServerState::Missing => "missing",
                    crate::lsp::ServerState::Failed(_) => "failed",
                };
                Some(state == arg.trim()).filter(|ok| *ok)
            }
            // `wait-find 3`: matches of the editor's find bar; `wait-caret 2`: the caret's line.
            "wait-find" | "wait-caret" => {
                let e = self.editors.get(&self.active?)?;
                let n: usize = arg.trim().parse().ok()?;
                let ok = if cmd == "wait-find" {
                    e.find.as_ref().is_some_and(|f| f.matches.len() == n)
                } else {
                    e.file().is_some_and(|f| f.caret.0 == n)
                };
                Some(ok).filter(|ok| *ok)
            }
            // `wait-tabs 0:2,4 1:3`: the active project's tabs and the panes in each.
            "wait-tabs" => {
                let p = self.active_project()?;
                let now: Vec<String> = p
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let ids: Vec<String> = t.layout.panes().iter().map(|x| x.to_string()).collect();
                        format!("{i}:{}", ids.join(","))
                    })
                    .collect();
                Some(now.join(" ") == arg.trim()).filter(|ok| *ok)
            }
            // File tree, git and editor of the active project, by path relative to it.
            "wait-tree" | "wait-git" | "wait-open" => {
                let id = self.active?;
                let root = self.project(id)?.path.clone();
                let (rel, extra) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
                let path = root.join(rel);
                let ok = match cmd {
                    "wait-tree" => self.explorers.get(&id).is_some_and(|e| e.shows(&path)),
                    "wait-git" => self
                        .project(id)
                        .and_then(|p| p.git.as_ref())
                        .and_then(|g| g.changes.get(&path))
                        .is_some_and(|c| c.letter() == extra),
                    _ => self
                        .editors
                        .get(&id)
                        .is_some_and(|e| e.visible && e.file().is_some_and(|f| f.path == path)),
                };
                Some(ok).filter(|ok| *ok)
            }
            _ => Some(true),
        }
    }

    /// Saves a screenshot asked for by `shot` (it arrives as an input event of a drawn frame).
    pub(crate) fn devtest_screenshot(&mut self, ctx: &egui::Context) {
        if let Some(name) = SHOT.with(|s| s.borrow().clone()) {
            let image = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            match image {
                Some(image) => {
                    let dir = std::env::var_os("TERNARYCODE_SHOT_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(std::env::temp_dir);
                    let path = dir.join(format!("{name}.png"));
                    let ok = write_png(&path, &image).is_ok();
                    self.devtest.check(ok, &format!("shot {}", path.display()));
                    SHOT.with(|s| *s.borrow_mut() = None);
                }
                None => ctx.request_repaint_after(Duration::from_millis(20)),
            }
        }
    }

    pub(crate) fn run_devtest(&mut self, ctx: &egui::Context) {
        if SHOT.with(|s| s.borrow().is_some()) {
            return;
        }
        // A condition being waited for.
        let waiting = WAIT.with(|w| w.borrow().as_ref().map(|w| (w.step.clone(), w.deadline, w.started)));
        if let Some((step, deadline, started)) = waiting {
            ctx.request_repaint_after(Duration::from_millis(20));
            match self.wait_check(&step) {
                Some(true) => {
                    let ms = started.elapsed().as_secs_f64() * 1000.0;
                    self.devtest.check(true, &format!("{step} ({ms:.0} ms)"));
                    WAIT.with(|w| *w.borrow_mut() = None);
                }
                _ if Instant::now() > deadline => {
                    let detail = format!(
                        "{step} (status {:?}, screen tail: {:?})",
                        self.current_status(),
                        self.current_screen().lines().rfind(|l| !l.trim().is_empty()).unwrap_or("")
                    );
                    self.devtest.check(false, &detail);
                    WAIT.with(|w| *w.borrow_mut() = None);
                }
                _ => return,
            }
        }
        let Some(step) = self.devtest.next(ctx) else {
            return;
        };
        let (cmd, arg) = step.split_once(' ').unwrap_or((step.as_str(), ""));
        match cmd {
            "key" => devtest::key(arg),
            "keys" => devtest::type_text(&devtest::unescape(arg)),
            // `mouse down|drag|up <widget>[@fx,fy]`: one event at a point of a widget (fractions
            // of its size; the centre by default). A drag is several steps a frame apart.
            "mouse" => {
                let (kind, target) = arg.trim().split_once(' ').unwrap_or(("up", arg.trim()));
                let (name, at) = target.split_once('@').unwrap_or((target, "0.5,0.5"));
                let (fx, fy) = at
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)))
                    .unwrap_or((0.5, 0.5));
                match devtest::rect_of(name) {
                    Some(r) => devtest::mouse(kind, r.lerp_inside(egui::vec2(fx, fy))),
                    None => self.devtest.check(false, &format!("mouse: no widget named {name}")),
                }
            }
            // `drop <widget>[@fx,fy] <path>`: files dropped at a point of a widget (an OS drag
            // can't be injected): typed into the terminal there, or added as a project.
            "drop" => {
                let (target, path) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
                let (name, at) = target.split_once('@').unwrap_or((target, "0.5,0.5"));
                let (fx, fy) = at
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)))
                    .unwrap_or((0.5, 0.5));
                match devtest::rect_of(name) {
                    Some(r) => {
                        let pos = r.lerp_inside(egui::vec2(fx, fy));
                        let pane = self.term_rects.iter().find(|(_, r)| r.contains(pos)).map(|(id, _)| *id);
                        self.route_drop(pane, vec![std::path::PathBuf::from(devtest::unescape(path))]);
                        ctx.request_repaint();
                    }
                    None => self.devtest.check(false, &format!("drop: no widget named {name}")),
                }
            }
            // `scroll <widget>[@fx,fy] <dy>`: the mouse wheel over a widget (negative: down).
            "scroll" => {
                let (target, dy) = arg.trim().rsplit_once(' ').unwrap_or((arg.trim(), "0"));
                let (name, at) = target.split_once('@').unwrap_or((target, "0.5,0.5"));
                let (fx, fy) = at
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)))
                    .unwrap_or((0.5, 0.5));
                match devtest::rect_of(name) {
                    Some(r) => {
                        devtest::wheel(r.lerp_inside(egui::vec2(fx, fy)), dy.parse().unwrap_or(0.0));
                        ctx.request_repaint();
                    }
                    None => self.devtest.check(false, &format!("scroll: no widget named {name}")),
                }
            }
            "click" => match devtest::rect_of(arg) {
                Some(r) => devtest::click(r.center(), 1, false),
                None => self.devtest.check(false, &format!("click: no widget named {arg}")),
            },
            "focus-pane" => {
                let n: usize = arg.trim().parse().unwrap_or(0);
                let pane = self
                    .active_project()
                    .and_then(|p| p.tab())
                    .and_then(|t| t.layout.panes().get(n).copied());
                match pane {
                    Some(p) => self.actions.push(Action::FocusPane(p)),
                    None => self.devtest.check(false, &format!("focus-pane {n}: no such pane")),
                }
            }
            "action" => {
                let (name, rest) = arg.split_once(' ').unwrap_or((arg, ""));
                let action = match name {
                    "add-project" => Some(Action::AddProjectPath(crate::app::absolute(rest.trim()))),
                    "split-right" => Some(Action::SplitRight),
                    "split-down" => Some(Action::SplitDown),
                    "new-tab" => Some(Action::NewTab),
                    "select-tab" => rest.trim().parse().ok().map(Action::SelectTab),
                    "close-pane" => Some(Action::ClosePane),
                    "next-attention" => Some(Action::NextAttention),
                    "back-attention" => Some(Action::BackAttention),
                    "run" => Some(Action::RunPreset(rest.trim().to_string())),
                    "select" => rest.trim().parse().ok().map(Action::SelectProjectIndex),
                    "palette" => Some(Action::Palette),
                    "pick" => Some(Action::PickPreset),
                    "files-scope" => Some(Action::QuickOpen),
                    "grid" => Some(Action::SetView(tc_core::model::View::Grid)),
                    "grid-all" => Some(Action::GridFilter(tc_core::model::GridFilter::All)),
                    "grid-waiting" => Some(Action::GridFilter(tc_core::model::GridFilter::Waiting)),
                    "ide" => {
                        use crate::lsp::{Goto, IdeCmd};
                        let cmd = match rest.trim() {
                            "def" => Some(IdeCmd::Goto(Goto::Definition)),
                            "typedef" => Some(IdeCmd::Goto(Goto::TypeDefinition)),
                            "impl" => Some(IdeCmd::Goto(Goto::Implementation)),
                            "refs" => Some(IdeCmd::References),
                            "rename" => Some(IdeCmd::Rename),
                            "actions" => Some(IdeCmd::CodeActions),
                            "run-action" => Some(IdeCmd::RunAction(0)),
                            "format" => Some(IdeCmd::Format),
                            "symbols" => Some(IdeCmd::Symbols),
                            "problems" => Some(IdeCmd::Problems),
                            "back" => Some(IdeCmd::Back),
                            "forward" => Some(IdeCmd::Forward),
                            "complete" => Some(IdeCmd::Complete(true)),
                            "signature" => Some(IdeCmd::Signature),
                            "undo" => Some(IdeCmd::UndoRefactor),
                            _ => None,
                        };
                        cmd.map(Action::Ide)
                    }
                    "grid-active" => Some(Action::GridFilter(tc_core::model::GridFilter::Active)),
                    "focus" => Some(Action::SetView(tc_core::model::View::Focus)),
                    "save" => Some(Action::SaveFile),
                    "find" => Some(Action::Find),
                    "find-all" => Some(Action::FindAll),
                    "review" => Some(Action::Review),
                    "force-save" => Some(Action::ForceSave),
                    "reload" => Some(Action::ReloadFile),
                    "editor" => Some(Action::ToggleEditor),
                    "files" => Some(Action::ToggleFiles),
                    "sidebar" => Some(Action::ToggleSidebar),
                    "attention" => Some(Action::ToggleAttention),
                    "rename" => self.project(self.active.unwrap_or_default()).map(|p| Action::RenamePath(p.path.join(rest.trim()))),
                    "settings" => Some(Action::Settings),
                    "add-dialog" => Some(Action::AddProject),
                    "integrations" => Some(Action::Integrations),
                    "install" => AgentKind::from_slug(rest.trim()).map(Action::PlanInstall),
                    "confirm-install" => Some(Action::ConfirmInstall),
                    "uninstall" => AgentKind::from_slug(rest.trim()).map(Action::UninstallNow),
                    "quit" => Some(Action::Quit),
                    _ => None,
                };
                match action {
                    Some(a) => self.actions.push(a),
                    None => self.devtest.check(false, &format!("unknown action {arg}")),
                }
            }
            // `goto path line col` (0-based): the caret to a place, as a jump would put it.
            "goto" => {
                let mut parts = arg.split_whitespace();
                let path = crate::app::absolute(parts.next().unwrap_or(""));
                let line = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                let col = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                self.actions.push(Action::GotoLocation(path, line, col));
            }
            "open-file" => {
                let (path, line) = arg.trim().rsplit_once(':').and_then(|(p, l)| l.parse().ok().map(|l| (p, Some(l)))).unwrap_or((arg.trim(), None));
                self.actions.push(Action::OpenFile(crate::app::absolute(path), line));
            }
            "write-file" => {
                let (path, content) = arg.split_once(' ').unwrap_or((arg, ""));
                let ok = std::fs::write(crate::app::absolute(path), devtest::unescape(content)).is_ok();
                self.devtest.check(ok, &format!("write-file {path}"));
            }
            "rm-file" => {
                let ok = std::fs::remove_file(crate::app::absolute(arg.trim())).is_ok();
                self.devtest.check(ok, &format!("rm-file {arg}"));
            }
            "expect-file" => {
                let (path, content) = arg.split_once(' ').unwrap_or((arg, ""));
                let disk = std::fs::read_to_string(crate::app::absolute(path)).unwrap_or_default();
                let want = devtest::unescape(content);
                self.devtest.check(disk.contains(&want), &format!("expect-file {path} contains {want:?} (disk: {disk:?})"));
            }
            "expect-not-file" => {
                let (path, content) = arg.split_once(' ').unwrap_or((arg, ""));
                let disk = std::fs::read_to_string(crate::app::absolute(path)).unwrap_or_default();
                let want = devtest::unescape(content);
                self.devtest.check(!disk.contains(&want), &format!("expect-not-file {path} lacks {want:?}"));
            }
            "wait-screen" | "wait-status" | "wait-queue" | "wait-editor" | "wait-conflict"
            | "wait-review" | "wait-snapshot" | "wait-tree" | "wait-git" | "wait-open" | "wait-tabs" | "wait-find" | "wait-caret"
            | "wait-active" | "wait-diag" | "wait-completion" | "wait-hover" | "wait-signature" | "wait-rename" | "wait-locations"
            | "wait-lsp" | "wait-doc" | "wait-toast" | "wait-shown" | "wait-hidden" => {
                WAIT.with(|w| {
                    *w.borrow_mut() = Some(Wait {
                        step: step.clone(),
                        deadline: Instant::now() + WAIT_LIMIT,
                        started: Instant::now(),
                    })
                });
            }
            "hook-latency" => {
                // Runs the real hook program with the current pane's environment and waits for
                // the pane's status: program start, socket, server, UI frame, tracker.
                let mut parts = arg.split_whitespace();
                let (agent, event, _status, kind) =
                    (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next(), parts.next());
                let env = self
                    .current_pane()
                    .and_then(|id| self.panes.get(&id))
                    .and_then(|p| p.hook_env.clone().map(|(s, t)| (p.id, s, t)));
                match env {
                    Some((pane, sock, token)) => {
                        let payload = match kind {
                            Some(k) => format!(r#"{{"notification_type":"{k}","message":"devtest {event}"}}"#),
                            None => r#"{"message":"devtest"}"#.to_string(),
                        };
                        let started = Instant::now();
                        let result = std::process::Command::new(&self.hook_bin.bin)
                            .args([agent, event, &payload])
                            .env(tc_agent::ENV_SOCK, sock)
                            .env(tc_agent::ENV_PANE, pane.to_string())
                            .env(tc_agent::ENV_TOKEN, token)
                            .status();
                        if result.is_err() {
                            self.devtest.check(false, &format!("hook program missing: {}", self.hook_bin.bin.display()));
                        } else {
                            WAIT.with(|w| {
                                *w.borrow_mut() = Some(Wait {
                                    step: step.clone(),
                                    deadline: started + Duration::from_secs(3),
                                    started,
                                })
                            });
                        }
                    }
                    None => self.devtest.check(false, "hook-latency: no pane with a hook environment"),
                }
            }
            "hook" => {
                // `hook PANE AGENT EVENT [KIND [MESSAGE…]]`: the real hook program, for any pane.
                let mut parts = arg.split_whitespace();
                let pane: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
                let (agent, event, kind) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next());
                let message = parts.collect::<Vec<_>>().join(" ");
                let message = if message.is_empty() { "Allow Bash: cargo test --workspace".to_string() } else { message };
                let Some((sock, token)) = self.panes.get(&pane).and_then(|p| p.hook_env.clone()) else {
                    self.devtest.check(false, &format!("hook: no pane {pane}"));
                    return;
                };
                let payload = match kind {
                    Some(k) => serde_json::json!({"notification_type": k, "message": message}).to_string(),
                    None => r#"{"message":"devtest"}"#.to_string(),
                };
                let _ = std::process::Command::new(&self.hook_bin.bin)
                    .args([agent, event, &payload])
                    .env(tc_agent::ENV_SOCK, sock)
                    .env(tc_agent::ENV_PANE, pane.to_string())
                    .env(tc_agent::ENV_TOKEN, token)
                    .status();
            }
            // `expect-no-lsp`: no language server serves the active file (servers on demand
            // start at an edit or a language feature, not when a file is opened).
            "expect-no-lsp" => {
                let file = self.active.and_then(|id| self.editors.get(&id)).and_then(|e| e.file()).map(|f| f.path.clone());
                let served = file.as_ref().is_some_and(|f| self.lsp.server_for(f).is_some());
                self.devtest.check(file.is_some() && !served, "expect-no-lsp");
            }
            "expect-project" => {
                let name = self.active_project().map(|p| p.name.clone()).unwrap_or_default();
                self.devtest.check(name == arg.trim(), &format!("expect-project {arg} (got {name})"));
            }
            "close-window" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "echo-latency" => {
                // Types a marker at the shell prompt and waits until the echo is on screen:
                // PTY write, shell echo, read, parse, wake-up. A new one each time, so one still
                // on screen from an earlier step does not count (digits only: measure-gate-b.sh
                // reads `tc\d+`; the count leads, the time has a fixed width, so none is part of
                // an earlier one).
                static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                let marker = format!("tc{n}{:06}", now.as_millis() % 1_000_000);
                if let Some(p) = self.current_pane().and_then(|id| self.panes.get(&id)) {
                    p.write(marker.clone().into_bytes());
                    WAIT.with(|w| {
                        *w.borrow_mut() = Some(Wait {
                            step: format!("wait-screen {marker}"),
                            deadline: Instant::now() + Duration::from_secs(2),
                            started: Instant::now(),
                        })
                    });
                }
            }
            "clear-line" => {
                if let Some(p) = self.current_pane().and_then(|id| self.panes.get(&id)) {
                    p.write(vec![0x15]);
                }
            }
            "log-mem" => {
                let mb = tc_agent::proc::own_resident_bytes().unwrap_or(0) as f64 / (1024.0 * 1024.0);
                eprintln!("DEVTEST MEM {arg}: {mb:.1} MB, {} panes", self.panes.len());
            }
            "expect-alive" => {
                let alive = self.panes.values().filter(|p| p.alive()).count();
                self.devtest.check(
                    alive.to_string() == arg.trim(),
                    &format!("expect-alive {arg} (got {alive})"),
                );
            }
            "resizes" => {
                let n: u32 = self.panes.values().map(|p| p.resizes).sum();
                eprintln!("DEVTEST RESIZES {arg}: {n}");
            }
            // `expect-fill 3`: pane 3's terminal has the size of where it is drawn (its Grid tile,
            // or its place in the Focus view).
            "expect-fill" => {
                let id: u64 = arg.trim().parse().unwrap_or(0);
                let want = if self.view == tc_core::model::View::Grid {
                    let cell = tc_term::widget::cell_size(&self.ctx, self.settings.font_size);
                    devtest::rect_of(&format!("tile-pane:{id}")).map(|r| tc_term::widget::grid_for(r, cell))
                } else {
                    self.panes.get(&id).and_then(|p| p.view.grid)
                };
                let got = self.panes.get(&id).map(|p| {
                    use tc_term::alacritty_terminal::grid::Dimensions;
                    let term = p.shared.term.lock();
                    tc_term::core::GridSize { cols: term.columns(), rows: term.screen_lines() }
                });
                self.devtest.check(want.is_some() && got == want, &format!("expect-fill {id}: terminal {got:?}, place {want:?}"));
            }
            // `dump-pane 3`: pane 3's screen, size, cursor and prompt marks, in the log.
            "dump-pane" => {
                if let Some(p) = arg.trim().parse::<u64>().ok().and_then(|id| self.panes.get(&id)) {
                    let text = screen_text(&p.shared);
                    let (cols, rows, cursor, history) = {
                        use tc_term::alacritty_terminal::grid::Dimensions;
                        let term = p.shared.term.lock();
                        (term.columns(), term.screen_lines(), term.grid().cursor.point, term.grid().history_size())
                    };
                    let prompt = p.shared.prompt.lock();
                    eprintln!(
                        "DEVTEST DUMP pane {arg}: {cols}x{rows} cursor {cursor:?} history {history} at_prompt {} redraw {:?} start {:?} input {:?} resizes {}\n{text}\n---- end",
                        prompt.at_prompt, prompt.redraw, prompt.start, prompt.input, p.resizes
                    );
                }
            }
            // `expect-resizes 3 0`: how often pane 3's PTY was resized.
            "expect-resizes" => {
                let (pane, n) = arg.trim().split_once(' ').unwrap_or((arg.trim(), "0"));
                let got = pane.parse::<u64>().ok().and_then(|id| self.panes.get(&id)).map(|p| p.resizes);
                self.devtest.check(got == n.parse().ok(), &format!("expect-resizes {arg} (got {got:?})"));
            }
            // `expect-width <widget> <min> <max>`: a widget's width, in points.
            "expect-width" => {
                let mut it = arg.split_whitespace();
                let (name, lo, hi) = (it.next().unwrap_or(""), it.next(), it.next());
                let (lo, hi): (f32, f32) = (
                    lo.and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    hi.and_then(|v| v.parse().ok()).unwrap_or(f32::MAX),
                );
                let w = devtest::rect_of(name).map(|r| r.width());
                self.devtest.check(
                    w.is_some_and(|w| (lo..=hi).contains(&w)),
                    &format!("expect-width {name} in {lo}..{hi}: {w:?}"),
                );
            }
            "expect-rect" => {
                let found = devtest::rect_of(arg.trim()).is_some();
                self.devtest.check(found, &format!("expect-rect {arg}"));
            }
            // `expect-typing g1`: the Grid tile taking the keyboard belongs to this project.
            "expect-typing" => {
                let got = self
                    .grid_focus
                    .and_then(|id| self.panes.get(&id))
                    .and_then(|p| self.project(p.project))
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.devtest.check(got == arg.trim(), &format!("expect-typing {arg} (got {got:?})"));
            }
            // `expect-screen-count alpha % 2`: lines of the current pane's screen containing the
            // text (a prompt left behind by a resize would add one).
            "expect-screen-count" => {
                let (text, n) = arg.trim().rsplit_once(' ').unwrap_or((arg.trim(), "0"));
                let screen = self.current_screen();
                let got = screen.lines().filter(|l| l.contains(text)).count();
                self.devtest.check(
                    got.to_string() == n,
                    &format!("expect-screen-count {arg} (got {got}; screen {screen:?})"),
                );
            }
            // `expect-no-line %`: no line of the current pane's screen is just this text.
            "expect-no-line" => {
                let screen = self.current_screen();
                let found = screen.lines().any(|l| l.trim() == arg.trim());
                self.devtest.check(!found, &format!("expect-no-line {arg} (screen {screen:?})"));
            }
            // The editor's colouring memory, let go now (as a minute after it was hidden).
            "free-editor" => self.free_editor_memory(),
            // Drawn (or not) in the last frame.
            "expect-shown" => self.devtest.check(devtest::shown(arg.trim()), &format!("expect-shown {arg}")),
            "expect-hidden" => self.devtest.check(!devtest::shown(arg.trim()), &format!("expect-hidden {arg}")),
            // How many panes have started their shell (the others sleep until shown).
            "expect-started" => {
                let started = self.panes.values().filter(|p| !p.dormant()).count();
                self.devtest.check(
                    started.to_string() == arg.trim(),
                    &format!("expect-started {arg} (got {started})"),
                );
            }
            "log-review" => {
                if let Some(r) = &self.review {
                    let v: Vec<String> = r.files.iter().map(|f| format!("{:?}:{}", f.kind, f.path)).collect();
                    eprintln!("DEVTEST REVIEW {:?} loading={} files={v:?} error={:?}", r.base, r.loading, r.error);
                }
            }
            "expect-dialog" => {
                let open = self.dialog.is_some();
                self.devtest.check(open == (arg.trim() == "open"), &format!("expect-dialog {arg}"));
            }
            "shot" => {
                SHOT.with(|s| *s.borrow_mut() = Some(arg.trim().to_string()));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            "frames" => {
                let (n, label) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
                let n = n.parse().unwrap_or(200);
                FRAMES.with(|f| *f.borrow_mut() = Some((n, Vec::with_capacity(n), label.to_string())));
                ctx.request_repaint();
            }
            "explorer-reads" => {
                let reads: u64 = self.explorers.values().map(|e| e.reads).sum();
                eprintln!("DEVTEST EXPLORER-READS {arg}: {reads}");
            }
            "log" => eprintln!("DEVTEST LOG: {arg}"),
            "quit" => {
                eprintln!(
                    "DEVTEST DONE passed={} failed={}",
                    self.devtest.passed, self.devtest.failed
                );
                self.actions.push(Action::QuitNow);
            }
            other => self.devtest.check(false, &format!("unknown step {other}")),
        }
    }
}
