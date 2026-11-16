//! Agents using the browser of their project: the tools of `ternarycode-hook mcp`. A tool call
//! comes in from the hook socket ([`App::browser_call`]) and becomes a job: a few steps carried
//! out in the page, one after another, a frame at a time (open a page and wait for it, act in
//! it through the helper `browser_agent.js`, wait for what that loads, read the page), then the
//! answer goes back on the call's connection.
//!
//! The user sees it all: the pane says which agent uses it and what it does, a ring shows each
//! element acted on, and Take Over (or a click in the page) gives the browser back to the
//! user, after which the agent's calls are told so. Each agent pane has its own browser: beside
//! it when there is room, else in a tab of the project. Sites other than this machine's ask the
//! user first (Allow Once, Always for the project, Block), as a question in the pane that also
//! goes into the usual queue of things waiting for the user.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tc_agent::protocol::CallMsg;
use tc_agent::server::Reply;
use tc_core::PaneId;
use tc_core::agent::AgentKind;
use tc_core::status::{Event as StatusEvent, HookSignal};

use crate::app::App;
use crate::browser::{Answer, Ask, Driver};
use crate::i18n::{t, tf};

/// What the pages answered, by call number (filled on the main thread by WebKit's callbacks).
pub type Done = Arc<Mutex<HashMap<u64, Result<String, String>>>>;

/// After its last call, an agent is shown using the browser this long (unless its turn ends).
const DRIVER_LINGER: Duration = Duration::from_secs(30);
/// Longest wait for a page to finish loading.
const LOAD_LIMIT: Duration = Duration::from_secs(30);
/// Longest wait for the page to answer a step (`browser_wait_for` waits up to 60 s in it).
const STEP_LIMIT: Duration = Duration::from_secs(70);
/// The moment between typed characters.
const KEY_GAP: Duration = Duration::from_millis(12);

/// A step of a job.
#[derive(Clone, Debug)]
enum Op {
    /// Open an address: at once when the agent may, else after the user's OK.
    Go(String),
    Back,
    /// A function of the helper, with its arguments, and the line the answer gets for it.
    Act(&'static str, Value, String),
    /// The agent's own function, run in the page.
    Eval(String),
    /// Wait for a load the last step started (if it started one).
    Settle,
    /// The page's outline, for the answer.
    Read,
    /// A picture of the page, for the answer.
    Shot,
    /// A moment between keys.
    Pause(Duration),
    /// Answer the page's dialog: OK (with a prompt's text) or Cancel.
    Dialog(bool, Option<String>),
    /// Find an element (the helper's `aim`: into view, shown), then act on its middle with real
    /// mouse events; the line the answer gets for it.
    Aim(Value, Then, String),
    /// Real mouse or keyboard events, with the line the answer gets for them.
    Native(Native, String),
}

/// What real events follow the aim at an element.
#[derive(Clone, Debug)]
enum Then {
    Click,
    Hover,
    /// A click that opens a file chooser, answered with these files.
    Upload(Vec<std::path::PathBuf>),
    /// The start of a drag to the element of this ref.
    DragFrom(String),
    /// The end of a drag from this point.
    DragTo(f64, f64),
}

/// Real events not aimed at an element.
#[derive(Clone, Debug)]
enum Native {
    /// A click at a point of the page (CSS pixels).
    ClickAt(f64, f64),
    Key(String),
    /// Text typed into what has the keyboard; Enter after it.
    Type(String, bool),
    /// One character typed (text goes a character at a time, a moment apart: WebKit's input
    /// takes text from keys sent all at once only for the first few).
    Char(char),
}

/// What a job waits for.
enum Waiting {
    /// The page's answer to a step; `at`: where the page was, and whether it was loading,
    /// when the step began (a step whose page goes away gets no answer at all).
    Page { seq: u64, op: Op, since: Instant, at: (String, bool) },
    /// `quiet`: frames in a row it was seen not loading (a page replacing another, a blank
    /// one, is briefly not loading in between).
    Load { since: Instant, started: bool, grace: Duration, quiet: u8 },
    Picture { seq: u64, since: Instant },
    /// A moment, until then.
    Pause(Instant),
    /// The user's answer about a site; then the page goes there (or the click is made).
    User { then: Op },
}

pub struct Job {
    pub browser: PaneId,
    agent_pane: PaneId,
    reply: Option<Reply>,
    ops: VecDeque<Op>,
    waiting: Option<Waiting>,
    /// The answer so far (what was done, then the page).
    text: String,
    image: Option<String>,
    /// The page was read again after it went away once.
    reread: bool,
    seqs: Vec<u64>,
    /// The pop-up the last step acted in (none: the page), once a step has run.
    on_popup: Option<Option<usize>>,
    /// The web view a click (or key) of the job acts in now: a pop-up it opens to a site the
    /// agent may not open is asked about.
    acted: Option<usize>,
    /// When it came: the downloads begun since are reported.
    since: Instant,
    /// Files wait for a file chooser its click opens.
    uploading: bool,
    /// The real click (or key) just made, to make again once the user lets a pop-up it opened
    /// go to a site the agent may not open.
    redo: Option<Op>,
}

/// A file an agent may upload: one that exists, in the project's folder (a relative path is
/// taken from there) or the Downloads folder; nothing else of the user's leaves for a site.
fn upload_path(f: &str, root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(f.trim());
    let p = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let p = p.canonicalize().map_err(|_| format!("No such file: {f}"))?;
    if !p.is_file() {
        return Err(format!("Not a file: {f}"));
    }
    let mut places = vec![root.to_path_buf()];
    if let Some(home) = std::env::var_os("HOME") {
        places.push(std::path::PathBuf::from(home).join("Downloads"));
    }
    #[cfg(debug_assertions)]
    if let Some(d) = std::env::var_os("TERNARYCODE_DOWNLOAD_DIR") {
        places.push(d.into());
    }
    if places.iter().filter_map(|d| d.canonicalize().ok()).any(|d| p.starts_with(d)) {
        Ok(p)
    } else {
        Err(format!("{f} is outside the project's folder and Downloads: only files there can be uploaded. Ask the user to copy it into the project."))
    }
}

/// The helper's function `name` with `args`, as the body of an async function.
fn helper_call(name: &str, args: &Value) -> String {
    format!("{}\nreturn JSON.stringify(await window.__tc.run({}, {}));", crate::browser::AGENT_JS, json!(name), args)
}

/// The agent's function (or expression) run in the page, its result as JSON.
fn eval_call(src: &str) -> String {
    let s = src.trim();
    let function = s.starts_with("function") || s.starts_with("async") || s.contains("=>");
    let f = if function { s.to_string() } else { format!("async () => ({s})") };
    format!(
        "try {{ const r = await ({f})(); let s; try {{ s = JSON.stringify(r === undefined ? null : r); }} catch (e) {{ s = JSON.stringify(String(r)); }} return JSON.stringify({{ result: s }}); }} catch (e) {{ return JSON.stringify({{ error: String(e && e.stack || e) }}); }}"
    )
}

/// The agent's status for a site question: asking (with the question), or working again.
fn status(agent: AgentKind, asking: bool, message: Option<String>) -> StatusEvent {
    StatusEvent::Hook { agent, signal: if asking { HookSignal::NeedsInput } else { HookSignal::Working }, message }
}

impl App {
    /// An agent's browser tool call: becomes a job (or is answered at once when it can't run).
    pub fn browser_call(&mut self, call: CallMsg, reply: Reply) {
        let fail = |reply: Reply, e: &str| {
            let _ = reply.send(json!({ "error": e }));
        };
        if !cfg!(target_os = "macos") {
            return fail(reply, "The browser tools run on macOS only for now.");
        }
        let agent_pane = call.pane;
        let Some(project) = self.panes.get(&agent_pane).map(|p| p.project) else {
            return fail(reply, "The pane this agent runs in is gone.");
        };
        // `ternarycode browse` / `$BROWSER` / `open https://…` in a pane: the user's page, in
        // the project's browser (in view when the project is); the default browser when web
        // pages open there (Settings), which the caller then opens.
        if call.call == "open" {
            let url = call.args["url"].as_str().unwrap_or("");
            let web = url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://");
            if !self.pages_here() || !web {
                return fail(reply, "default browser");
            }
            self.open_web_in(project, url, self.active == Some(project));
            let _ = reply.send(json!({ "text": "Opened in TernaryCode." }));
            return;
        }
        if !self.settings.browser_agents {
            return fail(reply, "The user turned the browser tools off in TernaryCode (Settings → Browser).");
        }
        if call.call == "browser_evaluate" && !self.settings.browser_agent_js {
            return fail(
                reply,
                "Running your own JavaScript in pages is off. The user can turn it on in TernaryCode (Settings → Browser → Agents may run JavaScript in pages); the other tools work without it.",
            );
        }
        let who = call.agent.as_deref().and_then(AgentKind::from_slug).map_or("The agent", AgentKind::label).to_string();
        let a = &call.args;
        let s = |k: &str| a[k].as_str().unwrap_or("").trim().to_string();
        // What the agent says the element is (shown to the user), else its ref.
        let element = || Some(s("element")).filter(|e| !e.is_empty()).unwrap_or_else(|| s("ref"));
        let (doing, ops): (String, Vec<Op>) = match call.call.as_str() {
            "browser_navigate" => {
                let url = crate::browser::normalize(&s("url"));
                if url.is_empty() {
                    return fail(reply, "No address given.");
                }
                (tf("Opening {}", &[&crate::browser::split_url(&url).0]), vec![Op::Go(url), Op::Read])
            }
            "browser_navigate_back" => (t("Going back").into(), vec![Op::Back, Op::Read]),
            "browser_snapshot" => (t("Reading the page").into(), vec![Op::Read]),
            "browser_click" => {
                let e = element();
                let args = json!({ "ref": s("ref"), "label": "click" });
                let then = if a["doubleClick"].as_bool().unwrap_or(false) { "Double-clicked" } else { "Clicked" };
                (tf("Clicking {}", &[&e]), vec![Op::Aim(args, Then::Click, format!("{then} {e}.")), Op::Settle, Op::Read])
            }
            "browser_hover" => {
                let e = element();
                (tf("Pointing at {}", &[&e]), vec![Op::Aim(json!({ "ref": s("ref"), "label": "hover" }), Then::Hover, format!("The pointer is on {e}.")), Op::Settle, Op::Read])
            }
            "browser_drag" => {
                let (from, to) = (s("startElement"), s("endElement"));
                let args = json!({ "ref": s("startRef"), "label": "drag" });
                (tf("Dragging {} to {}", &[&from, &to]), vec![Op::Aim(args, Then::DragFrom(s("endRef")), format!("Dragged {from} to {to}.")), Op::Settle, Op::Read])
            }
            "browser_mouse_click_xy" => {
                let (x, y) = (a["x"].as_f64().unwrap_or(0.0), a["y"].as_f64().unwrap_or(0.0));
                let e = Some(s("element")).filter(|e| !e.is_empty()).unwrap_or_else(|| format!("{x:.0}, {y:.0}"));
                (tf("Clicking {}", &[&e]), vec![Op::Act("point", json!({ "x": x, "y": y }), String::new()), Op::Native(Native::ClickAt(x, y), format!("Clicked at {x:.0}, {y:.0}.")), Op::Settle, Op::Read])
            }
            "browser_keyboard_type" => {
                let text = a["text"].as_str().unwrap_or("").to_string();
                let submit = a["submit"].as_bool().unwrap_or(false);
                (t("Typing").into(), vec![Op::Native(Native::Type(text, submit), "Typed.".into()), Op::Settle, Op::Read])
            }
            "browser_file_upload" => {
                let e = element();
                let root = self.project(project).map(|p| p.path.clone()).unwrap_or_default();
                let mut files = Vec::new();
                for f in a["paths"].as_array().cloned().unwrap_or_default() {
                    let f = f.as_str().unwrap_or("");
                    match upload_path(f, &root) {
                        Ok(p) => files.push(p),
                        Err(e) => return fail(reply, &e),
                    }
                }
                if files.is_empty() {
                    return fail(reply, "No files given (paths).");
                }
                let names: Vec<String> = files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().to_string()).collect();
                let line = format!("Chose {} for {e}.", names.join(", "));
                (tf("Uploading {} to {}", &[&names.join(", "), &e]), vec![Op::Aim(json!({ "ref": s("ref"), "label": "upload" }), Then::Upload(files), line), Op::Settle, Op::Read])
            }
            "browser_type" => {
                let e = element();
                let args = json!({ "ref": s("ref"), "text": a["text"].as_str().unwrap_or(""), "submit": a["submit"].as_bool().unwrap_or(false) });
                (tf("Typing into {}", &[&e]), vec![Op::Act("type", args, format!("Typed into {e}.")), Op::Settle, Op::Read])
            }
            "browser_select_option" => {
                let e = element();
                let args = json!({ "ref": s("ref"), "values": a["values"].clone() });
                (tf("Choosing in {}", &[&e]), vec![Op::Act("select", args, format!("Chose in {e}.")), Op::Settle, Op::Read])
            }
            "browser_press_key" => {
                let key = s("key");
                (tf("Pressing {}", &[&key]), vec![Op::Native(Native::Key(key.clone()), format!("Pressed {key}.")), Op::Settle, Op::Read])
            }
            "browser_wait_for" => {
                let what = if !s("text").is_empty() { tf("Waiting for “{}”", &[&s("text")]) } else { t("Waiting").to_string() };
                let args = json!({ "text": s("text"), "textGone": s("textGone"), "time": a["time"].as_f64().unwrap_or(0.0) });
                (what, vec![Op::Act("wait", args, "Waited.".into()), Op::Read])
            }
            "browser_scroll" => {
                let dir = Some(s("direction")).filter(|d| !d.is_empty());
                let what = match (&dir, s("ref").is_empty()) {
                    (None, false) => tf("Scrolling to {}", &[&element()]),
                    _ => t("Scrolling the page").to_string(),
                };
                let line = match &dir {
                    Some(d) => format!("Scrolled {d}."),
                    None if !s("ref").is_empty() => format!("Scrolled {} into view.", element()),
                    None => "Scrolled down.".to_string(),
                };
                let args = json!({ "ref": s("ref"), "direction": dir, "pages": a["pages"].as_f64().unwrap_or(1.0) });
                (what, vec![Op::Act("scroll", args, line), Op::Settle, Op::Read])
            }
            "browser_take_screenshot" => (t("Taking a screenshot").into(), vec![Op::Shot]),
            "browser_console_messages" => (t("Reading the console").into(), vec![Op::Act("console", json!({}), String::new())]),
            "browser_evaluate" => (t("Running a script in the page").into(), vec![Op::Eval(s("function"))]),
            "browser_handle_dialog" => (t("Answering the page’s dialog").into(), vec![Op::Dialog(a["accept"].as_bool().unwrap_or(true), a["promptText"].as_str().map(str::to_string)), Op::Settle, Op::Read]),
            other => return fail(reply, &format!("Unknown tool {other}.")),
        };
        let Some(id) = self.agent_browser(agent_pane, project, call.call == "browser_navigate") else {
            return fail(reply, "No page is open in this project's browser yet: use browser_navigate first.");
        };
        let Some(b) = self.browsers.get_mut(&id) else { return };
        if b.driver.as_ref().is_some_and(|d| d.paused) {
            return fail(reply, &format!("The user has the browser now (they took it over from {who} in TernaryCode). Wait until they hand it back, or ask them."));
        }
        b.driver = Some(Driver { pane: agent_pane, who, doing, last: Instant::now(), paused: false });
        // The user handed the browser back: the agent hears where they left it.
        let text = if std::mem::take(&mut b.handed_back) {
            format!("The user had taken the browser over and handed it back. It is at {} ({}).\n", b.url, b.title)
        } else {
            String::new()
        };
        self.browser_jobs.push(Job {
            browser: id,
            agent_pane,
            reply: Some(reply),
            ops: ops.into(),
            waiting: None,
            text,
            image: None,
            reread: false,
            seqs: Vec::new(),
            on_popup: None,
            acted: None,
            since: Instant::now(),
            uploading: false,
            redo: None,
        });
        self.mark_dirty();
    }

    /// The page an agent's tools act on: its own in its project's Browser panel, else
    /// (`create`) a new one there. It comes to the front when the agent's terminal is the one
    /// picked (or the panel had none), and the panel opens (Settings), unless the Code drawer
    /// is open: then the Browser button says so. The keyboard and the tab in view stay as they
    /// are.
    fn agent_browser(&mut self, agent_pane: PaneId, project: tc_core::ProjectId, create: bool) -> Option<PaneId> {
        if let Some(id) = self.browsers.values().find(|b| b.owner == Some(agent_pane) && b.project == project).map(|b| b.id) {
            return Some(id);
        }
        if !create {
            return None;
        }
        self.project(project)?;
        let id = self.add_page(project, "", Some(agent_pane));
        let picked = self.project(project).and_then(|p| p.current_pane()) == Some(agent_pane);
        let open = self.settings.browser_agent_beside;
        if let Some(panel) = self.panel_mut(project) {
            if picked || panel.shown().is_none() {
                panel.front = Some(id);
            }
            panel.open |= open;
        }
        Some(id)
    }

    /// The sites an agent of `project` may open without asking: every project's and its own.
    pub fn agent_sites(&self, project: tc_core::ProjectId) -> Vec<String> {
        let mut sites = self.settings.browser_agent_sites.clone();
        if let Some(p) = self.project(project) {
            sites.extend(p.sites.iter().cloned());
        }
        sites
    }

    /// The browsers jobs are waiting on (their pages are kept up, out of view if need be).
    pub fn browsers_in_use(&self) -> HashSet<PaneId> {
        let mut ids: HashSet<PaneId> = self.browser_jobs.iter().map(|j| j.browser).collect();
        ids.extend(self.browsers.values().filter(|b| b.driven() || b.ask.is_some()).map(|b| b.id));
        ids
    }

    /// Answers job `i` and takes it away.
    fn end_job(&mut self, i: usize, error: Option<String>) {
        let mut job = self.browser_jobs.remove(i);
        {
            let mut done = self.browser_done.lock();
            for s in &job.seqs {
                done.remove(s);
            }
        }
        // Ended with its question open (the pane was closed, the user took the browser over):
        // the question goes, and the agent is no longer waiting for the user.
        if matches!(job.waiting, Some(Waiting::User { .. })) {
            if let Some(b) = self.browsers.get_mut(&job.browser) {
                b.ask = None;
            }
            self.asked(job.agent_pane);
        }
        // What the page downloaded meanwhile (the agent may need the file).
        if let Some(b) = self.browsers.get(&job.browser) {
            for d in b.downloads.iter().filter(|d| d.at >= job.since) {
                let name = d.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                job.text.push_str(&match d.done {
                    Some(true) => format!("\nDownloaded {name} to {}.", d.path.display()),
                    Some(false) => format!("\nThe download of {name} failed."),
                    None => format!("\nDownloading {name} to {} (not finished yet).", d.path.display()),
                });
            }
        }
        let answer = match error {
            Some(e) => json!({ "error": e }),
            None => {
                let mut v = json!({ "text": job.text.trim() });
                if let Some(png) = job.image.take() {
                    v["image"] = json!(png);
                }
                v
            }
        };
        if let Some(r) = job.reply.take() {
            let _ = r.send(answer);
        }
        if let Some(b) = self.browsers.get_mut(&job.browser)
            && let Some(d) = b.driver.as_mut()
        {
            d.last = Instant::now();
        }
    }

    /// Asks the user whether the agent may open `url` (in the pane, and as the agent's
    /// question in the queue).
    fn ask_site(&mut self, browser: PaneId, agent_pane: PaneId, url: &str, click: bool) {
        let host = crate::browser::host_of(url);
        if let Some(b) = self.browsers.get_mut(&browser) {
            b.ask = Some(Ask { url: url.to_string(), host: host.clone(), click, answer: None });
        }
        if let Some(p) = self.panes.get_mut(&agent_pane)
            && let Some(agent) = p.tracker.agent.or(p.last_agent)
        {
            p.tracker.apply(status(agent, true, Some(tf("Open {}?", &[&host]))), Instant::now());
        }
    }

    /// The user answered a site question: the agent's status goes back to working.
    fn asked(&mut self, agent_pane: PaneId) {
        if let Some(p) = self.panes.get_mut(&agent_pane)
            && let Some(agent) = p.tracker.agent.or(p.last_agent)
        {
            p.tracker.apply(status(agent, false, None), Instant::now());
        }
    }

    /// A load came into a page an agent uses from a site it may not open (a redirect, a
    /// script, a click the page turned into a navigation): the page goes back, and the user is
    /// asked. (A click on a link or a form to such a site is asked about before it is made.)
    #[cfg(target_os = "macos")]
    pub fn stop_unasked_load(&mut self, id: PaneId, url: &str, before: &str) {
        let Some(project) = self.browsers.get(&id).map(|b| b.project) else { return };
        let sites = self.agent_sites(project);
        let Some(b) = self.browsers.get_mut(&id) else { return };
        let Some(agent_pane) = b.driver.as_ref().filter(|d| !d.paused).map(|d| d.pane) else { return };
        if b.ask.is_some() || crate::browser::agent_may_open(url, &sites, b.pass.as_deref()) {
            if b.pass.as_deref() == Some(crate::browser::host_of(url).as_str()) {
                b.pass = None;
            }
            return;
        }
        if let Some(v) = &b.view {
            crate::browser::turn_back(v);
        }
        b.url = before.to_string();
        b.loading = false;
        self.ask_site(id, agent_pane, url, true);
        // The job acting on the page waits for the answer, then goes there.
        if let Some(job) = self.browser_jobs.iter_mut().find(|j| j.browser == id) {
            job.waiting = Some(Waiting::User { then: Op::Go(url.to_string()) });
        }
    }

    /// Carries the jobs on (each frame while there are any): the first job of each browser
    /// takes its next steps; the others wait their turn.
    #[cfg(target_os = "macos")]
    pub fn run_browser_jobs(&mut self, ctx: &eframe::egui::Context) {
        let now = Instant::now();
        // Agents that stopped using their browser: the pane says so no longer.
        let jobs: HashSet<PaneId> = self.browser_jobs.iter().map(|j| j.browser).collect();
        for b in self.browsers.values_mut() {
            let Some(d) = &b.driver else { continue };
            // Taken over, it stays the user's until they hand it back (the agent's next calls are
            // told so), or the agent's pane is gone.
            let gone = !self.panes.contains_key(&d.pane);
            let turn_over = self.panes.get(&d.pane).is_none_or(|p| {
                !matches!(p.tracker.status, tc_core::status::Status::Running | tc_core::status::Status::NeedsInput)
            });
            let idle = !jobs.contains(&b.id) && b.ask.is_none() && (turn_over || now.duration_since(d.last) > DRIVER_LINGER);
            if gone || (idle && !d.paused) {
                b.driver = None;
            }
        }
        if self.browser_jobs.is_empty() {
            return;
        }
        let mut busy = HashSet::new();
        let mut i = 0;
        while i < self.browser_jobs.len() {
            let id = self.browser_jobs[i].browser;
            if !busy.insert(id) {
                i += 1;
                continue;
            }
            match self.step_job(i, now) {
                Some(error) => self.end_job(i, error),
                None => i += 1,
            }
        }
        if !self.browser_jobs.is_empty() {
            // Typing goes a key a frame: sooner then.
            let typing = self.browser_jobs.iter().any(|j| matches!(j.waiting, Some(Waiting::Pause(_))));
            ctx.request_repaint_after(Duration::from_millis(if typing { 8 } else { 40 }));
        }
    }

    /// Takes job `i` as far as it goes this frame. `Some`: it is over (with an error, or
    /// `None` for its answer).
    #[cfg(target_os = "macos")]
    fn step_job(&mut self, i: usize, now: Instant) -> Option<Option<String>> {
        loop {
            let id = self.browser_jobs[i].browser;
            let Some(b) = self.browsers.get(&id) else {
                return Some(Some("The page was closed in TernaryCode.".into()));
            };
            if let Some(d) = b.driver.as_ref().filter(|d| d.paused) {
                return Some(Some(format!("The user took the browser over from {} in TernaryCode. Wait until they hand it back, or ask them.", d.who)));
            }
            // A dialog of the page: its scripts wait for the answer, so no step that runs one can
            // go on; the agent is told (unless this step answers it).
            if let Some(d) = &b.dialog {
                let job = &self.browser_jobs[i];
                let answering = matches!(job.ops.front(), Some(Op::Dialog(..))) && job.waiting.is_none();
                if !answering && !matches!(job.waiting, Some(Waiting::User { .. })) {
                    let kind = match d.kind {
                        crate::browser::DialogKind::Alert => "alert",
                        crate::browser::DialogKind::Confirm => "confirm",
                        crate::browser::DialogKind::Prompt => "prompt",
                    };
                    let note = format!(
                        "A dialog is open in the page ({kind} from {}): “{}”. The page waits for it: answer it with browser_handle_dialog (accept true for OK, false for Cancel; promptText for a prompt), or ask the user.\n",
                        d.host, d.message
                    );
                    self.browser_jobs[i].text.push_str(&note);
                    return Some(None);
                }
            }
            // Where the steps act: the newest pop-up the page opened while one is open, else the
            // page (it comes up out of view if need be, `sync_browsers`).
            let popup = crate::browser_popup::popup_of(id);
            let view = popup.clone().or_else(|| b.view.as_ref().map(crate::browser::web_view_of));
            // A page not up yet (a new browser gets its web view once it has an address) counts
            // as loading.
            let loading = view.as_ref().is_none_or(|w| crate::browser::loading_in(w));
            let sites = self.agent_sites(b.project);
            let pass = b.pass.clone();
            let answer = b.ask.as_ref().and_then(|a| a.answer.map(|x| (x, a.host.clone(), a.url.clone())));
            let here = b.url.clone();
            let page_url = view.as_ref().map(|w| crate::browser::url_in(w)).unwrap_or_default();
            let job = &mut self.browser_jobs[i];
            // A pop-up opened (the agent's click, or the user's) or closed since the last step:
            // the agent hears of it.
            let seen = popup.as_ref().map(|w| std::ptr::from_ref::<objc2_web_kit::WKWebView>(w) as usize);
            match job.on_popup.replace(seen) {
                Some(before) if before != seen && seen.is_some() => {
                    let host = crate::browser::host_of(&page_url);
                    let on = if host.is_empty() { String::new() } else { format!(" ({host})") };
                    job.text.push_str(&format!(
                        "A pop-up window opened{on}. It shows over TernaryCode; your next steps act in it until it closes (browser_navigate closes it). If it asks for a password or a one-time code, ask the user to type it there themselves.\n"
                    ));
                }
                Some(Some(_)) if seen.is_none() => job.text.push_str("The pop-up window closed; the steps act in the page again.\n"),
                _ => {}
            }
            match job.waiting.take() {
                Some(Waiting::Page { seq, op, since, at }) => {
                    let got = self.browser_done.lock().remove(&seq);
                    // The page went to another address (or began loading): WebKit won't answer.
                    let left = page_url != at.0 || (loading && !at.1);
                    let got = match got {
                        Some(got) => got,
                        None if left => Err("the page went to another address".into()),
                        None if now.duration_since(since) > STEP_LIMIT => {
                            return Some(Some("The page did not answer in time.".into()));
                        }
                        None => {
                            job.waiting = Some(Waiting::Page { seq, op, since, at });
                            return None;
                        }
                    };
                    match (op, got) {
                        // The page went away while the step ran (it navigated): its next one.
                        (Op::Act(name, _, line), Err(_)) => {
                            if name == "wait" {
                                job.text.push_str("The page changed while waiting.\n");
                            } else {
                                job.text.push_str(&line);
                                job.text.push('\n');
                            }
                            if !matches!(job.ops.front(), Some(Op::Settle)) {
                                job.ops.push_front(Op::Settle);
                            }
                        }
                        (Op::Aim(_, _, line), Err(_)) => {
                            job.text.push_str(&line);
                            job.text.push('\n');
                            if !matches!(job.ops.front(), Some(Op::Settle)) {
                                job.ops.push_front(Op::Settle);
                            }
                        }
                        (Op::Read, Err(e)) => {
                            if job.reread {
                                return Some(Some(format!("Could not read the page: {e}")));
                            }
                            job.reread = true;
                            job.ops.push_front(Op::Read);
                            job.ops.push_front(Op::Settle);
                        }
                        (Op::Eval(_), Err(e)) => return Some(Some(format!("The script failed: {e}"))),
                        (op, Ok(text)) => {
                            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                            if let Some(e) = v["error"].as_str() {
                                return Some(Some(e.to_string()));
                            }
                            match op {
                                Op::Act(name, args, line) => {
                                    // A pop-up the step opened to a site the agent may not open:
                                    // asked first, then the step is made again.
                                    let blocked = job.acted.take().and_then(crate::browser_popup::take_blocked);
                                    if let Some(url) = v["leaves"].as_str().map(str::to_string).or(blocked) {
                                        // A click to a site the agent may not open: asked first.
                                        let mut args = args;
                                        args["force"] = json!(true);
                                        let pane = job.agent_pane;
                                        job.waiting = Some(Waiting::User { then: Op::Act(name, args, line) });
                                        self.ask_site(id, pane, &url, true);
                                        return None;
                                    }
                                    if name == "console" {
                                        job.text.push_str(v["text"].as_str().unwrap_or(""));
                                    } else {
                                        job.text.push_str(&line);
                                        job.text.push_str(v["note"].as_str().unwrap_or(""));
                                        job.text.push('\n');
                                    }
                                }
                                Op::Read => {
                                    job.text.push_str(&format!(
                                        "\nPage: {}{}\nURL: {}\n\n{}\n",
                                        v["title"].as_str().unwrap_or(""),
                                        if popup.is_some() { " (a pop-up window)" } else { "" },
                                        v["url"].as_str().unwrap_or(""),
                                        v["text"].as_str().unwrap_or("")
                                    ));
                                }
                                Op::Eval(_) => {
                                    job.text.push_str(&format!("Result: {}", v["result"].as_str().unwrap_or("null")));
                                }
                                Op::Aim(mut args, then, line) => {
                                    if let Some(url) = v["leaves"].as_str() {
                                        // A link to a site the agent may not open: asked first.
                                        args["force"] = json!(true);
                                        let pane = job.agent_pane;
                                        job.waiting = Some(Waiting::User { then: Op::Aim(args, then, line) });
                                        self.ask_site(id, pane, url, true);
                                        return None;
                                    }
                                    let (x, y) = (v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0));
                                    let Some(wk) = view.as_deref() else { return Some(Some("The page went away.".into())) };
                                    let note = v["note"].as_str().unwrap_or("").to_string();
                                    match then {
                                        Then::Click | Then::Upload(_) => {
                                            if let Then::Upload(files) = &then {
                                                crate::browser_dialog::upload_next(wk, files.clone());
                                                job.uploading = true;
                                            }
                                            // A pop-up its click opens is the agent's.
                                            crate::browser_popup::agent_acting(wk, sites.clone(), pass.clone());
                                            job.acted = Some(std::ptr::from_ref(wk) as usize);
                                            let mut again = args.clone();
                                            again["force"] = json!(true);
                                            job.redo = Some(Op::Aim(again, then.clone(), line.clone()));
                                            let double = line.starts_with("Double-clicked");
                                            crate::browser_input::click(wk, x, y, if double { 2 } else { 1 });
                                        }
                                        Then::Hover => {
                                            crate::browser_input::hover(wk, x, y);
                                        }
                                        Then::DragFrom(to) => {
                                            // The page's own drag and drop for a draggable element; real
                                            // mouse events for the rest (sliders, sortable lists).
                                            let next = if v["draggable"].as_bool().unwrap_or(false) {
                                                Op::Act("drag5", json!({ "ref": args["ref"], "to": to }), line)
                                            } else {
                                                Op::Aim(json!({ "ref": to, "label": "drop" }), Then::DragTo(x, y), line)
                                            };
                                            job.ops.push_front(next);
                                            continue;
                                        }
                                        Then::DragTo(fx, fy) => {
                                            crate::browser_input::drag(wk, (fx, fy), (x, y));
                                        }
                                    }
                                    job.text.push_str(&line);
                                    job.text.push_str(&note);
                                    job.text.push('\n');
                                }
                                _ => {}
                            }
                        }
                        (_, Err(e)) => return Some(Some(e)),
                    }
                }
                Some(Waiting::Load { since, started, grace, quiet }) => {
                    let started = started || loading;
                    let quiet = if loading { 0 } else { quiet + 1 };
                    let waited = now.duration_since(since);
                    if (started && quiet >= 2) || (!started && waited > grace) {
                        // Loaded (or nothing began loading).
                    } else if waited > LOAD_LIMIT {
                        job.text.push_str("(The page was still loading.)\n");
                    } else {
                        job.waiting = Some(Waiting::Load { since, started, grace, quiet });
                        return None;
                    }
                }
                Some(Waiting::Picture { seq, since }) => {
                    let got = self.browser_done.lock().remove(&seq);
                    match got {
                        Some(Ok(png)) => job.image = Some(png),
                        Some(Err(e)) => return Some(Some(format!("Could not take a screenshot: {e}"))),
                        None if now.duration_since(since) > STEP_LIMIT => return Some(Some("The screenshot did not come.".into())),
                        None => {
                            job.waiting = Some(Waiting::Picture { seq, since });
                            return None;
                        }
                    }
                }
                Some(Waiting::Pause(until)) => {
                    if now < until {
                        job.waiting = Some(Waiting::Pause(until));
                        return None;
                    }
                }
                Some(Waiting::User { then }) => {
                    let Some((answer, host, url)) = answer else {
                        job.waiting = Some(Waiting::User { then });
                        return None;
                    };
                    let pane = job.agent_pane;
                    if answer != Answer::Block {
                        job.ops.push_front(match then {
                            // Opened now that it may (the pass lets its load through).
                            Op::Go(_) => Op::Go(url.clone()),
                            other => other,
                        });
                    }
                    if let Some(b) = self.browsers.get_mut(&id) {
                        b.ask = None;
                        // Blocked: no pass (the agent asking again is asked about again).
                        if answer != Answer::Block {
                            b.pass = Some(host.clone());
                        }
                    }
                    self.asked(pane);
                    match answer {
                        Answer::Block => {
                            return Some(Some(format!("Blocked by the user: {host}. The page stayed at {here}.")));
                        }
                        Answer::Always => {
                            let project = self.browsers.get(&id).map(|b| b.project);
                            if let Some(p) = project.and_then(|p| self.project_mut(p))
                                && !p.sites.contains(&host)
                            {
                                p.sites.push(host);
                            }
                            self.mark_dirty();
                        }
                        Answer::Once => {}
                    }
                    continue;
                }
                None => {
                    // A pop-up the last real click (or key) opened to a site the agent may not
                    // open (WebKit asks for it after the click): the user is asked, then the click
                    // is made again.
                    if !matches!(job.ops.front(), Some(Op::Settle))
                        && let Some(k) = job.acted
                        && let Some(url) = crate::browser_popup::take_blocked(k)
                        && let Some(redo) = job.redo.take()
                    {
                        job.acted = None;
                        let pane = job.agent_pane;
                        job.waiting = Some(Waiting::User { then: redo });
                        self.ask_site(id, pane, &url, true);
                        return None;
                    }
                    // Files given for a file chooser that never came (the click was not on a file
                    // field): taken back, and the agent told.
                    if job.uploading && !matches!(job.ops.front(), Some(Op::Settle)) {
                        job.uploading = false;
                        if let Some(wk) = view.as_deref()
                            && crate::browser_dialog::upload_unused(wk)
                        {
                            return Some(Some("The click did not open a file chooser: give the ref of the file field (or the button that opens it).".into()));
                        }
                    }
                    let Some(op) = job.ops.pop_front() else {
                        return Some(None);
                    };
                    let seq = self.browser_seq;
                    match op {
                        Op::Go(url) => {
                            let pass = self.browsers.get(&id).and_then(|b| b.pass.clone());
                            if !crate::browser::agent_may_open(&url, &sites, pass.as_deref()) {
                                let pane = job.agent_pane;
                                job.waiting = Some(Waiting::User { then: Op::Go(url.clone()) });
                                self.ask_site(id, pane, &url, false);
                                return None;
                            }
                            job.text.push_str(&format!("Opened {url}.\n"));
                            job.waiting = Some(Waiting::Load { since: now, started: false, grace: Duration::from_secs(3), quiet: 0 });
                            // A new address is the page's: pop-ups it opened close.
                            crate::browser_popup::close_of(id);
                            if let Some(b) = self.browsers.get_mut(&id) {
                                b.navigate(&url);
                            }
                        }
                        Op::Settle => {
                            job.waiting = Some(Waiting::Load { since: now, started: loading, grace: Duration::from_millis(600), quiet: 0 });
                        }
                        Op::Pause(d) => {
                            job.waiting = Some(Waiting::Pause(now + d));
                        }
                        Op::Dialog(accept, text) => {
                            let line = match self.browsers.get_mut(&id).and_then(|b| b.dialog.as_mut()) {
                                Some(d) => {
                                    if let Some(t) = &text {
                                        d.text = t.clone();
                                    }
                                    format!("{} the dialog “{}”.\n", if accept { "Accepted" } else { "Dismissed" }, d.message)
                                }
                                None => return Some(Some("No dialog is open in the page.".into())),
                            };
                            self.answer_dialog(id, accept);
                            self.browser_jobs[i].text.push_str(&line);
                            continue;
                        }
                        op => {
                            // The other steps need the page up: until then, they wait.
                            let Some(view) = view.as_deref() else {
                                job.ops.push_front(op);
                                return None;
                            };
                            let done = self.browser_done.clone();
                            let page = move |r: Result<String, String>| {
                                done.lock().insert(seq, r);
                            };
                            match op {
                                Op::Back => {
                                    // SAFETY: a plain WKWebView method on the main thread.
                                    unsafe { view.goBack() };
                                    job.text.push_str("Went back.\n");
                                    job.waiting = Some(Waiting::Load { since: now, started: false, grace: Duration::from_millis(1500), quiet: 0 });
                                }
                                Op::Act(name, mut args, line) => {
                                    if name == "click" {
                                        args["allowed"] = json!(sites);
                                    }
                                    // A click or key may open a pop-up (Continue with Google): as
                                    // the agent's, to the sites it may open.
                                    if matches!(name, "click" | "press" | "type" | "select") {
                                        crate::browser_popup::agent_acting(view, sites.clone(), pass.clone());
                                        job.acted = Some(std::ptr::from_ref(view) as usize);
                                    }
                                    crate::browser::run_js_in(view, &helper_call(name, &args), move |r| {
                                        page(r);
                                    });
                                    job.waiting = Some(Waiting::Page { seq, op: Op::Act(name, args, line), since: now, at: (page_url.clone(), loading) });
                                }
                                Op::Aim(args, then, line) => {
                                    let mut args = args;
                                    args["allowed"] = json!(sites);
                                    crate::browser::run_js_in(view, &helper_call("aim", &args), move |r| {
                                        page(r);
                                    });
                                    job.waiting = Some(Waiting::Page { seq, op: Op::Aim(args, then, line), since: now, at: (page_url.clone(), loading) });
                                }
                                Op::Native(native, line) => {
                                    crate::browser_popup::agent_acting(view, sites.clone(), pass.clone());
                                    job.acted = Some(std::ptr::from_ref(view) as usize);
                                    if !matches!(native, Native::Char(_) | Native::Type(..)) {
                                        job.redo = Some(Op::Native(native.clone(), line.clone()));
                                    }
                                    let done = match native {
                                        Native::ClickAt(x, y) => crate::browser_input::click(view, x, y, 1).then_some(()).ok_or_else(|| "The page has no window.".to_string()),
                                        Native::Key(k) => crate::browser_input::key(view, &k),
                                        Native::Char(c) => crate::browser_input::type_text(view, &c.to_string()).then_some(()).ok_or_else(|| "The page has no window.".to_string()),
                                        Native::Type(text, submit) => {
                                            // A character at a time, a moment apart (then Enter).
                                            let mut keys: Vec<Op> = Vec::new();
                                            for c in text.chars() {
                                                keys.push(Op::Native(Native::Char(c), String::new()));
                                                keys.push(Op::Pause(KEY_GAP));
                                            }
                                            if submit {
                                                keys.push(Op::Native(Native::Key("Enter".into()), String::new()));
                                            }
                                            for k in keys.into_iter().rev() {
                                                job.ops.push_front(k);
                                            }
                                            Ok(())
                                        }
                                    };
                                    if let Err(e) = done {
                                        return Some(Some(e));
                                    }
                                    if !line.is_empty() {
                                        job.text.push_str(&line);
                                        job.text.push('\n');
                                    }
                                    continue;
                                }
                                Op::Eval(src) => {
                                    crate::browser::run_js_in(view, &eval_call(&src), move |r| {
                                        page(r);
                                    });
                                    job.waiting = Some(Waiting::Page { seq, op: Op::Eval(src), since: now, at: (page_url.clone(), loading) });
                                }
                                Op::Read => {
                                    crate::browser::run_js_in(view, &helper_call("snapshot", &json!({})), move |r| {
                                        page(r);
                                    });
                                    job.waiting = Some(Waiting::Page { seq, op: Op::Read, since: now, at: (page_url.clone(), loading) });
                                }
                                Op::Shot => {
                                    // The page's size, for clicks at a point of the picture.
                                    // SAFETY: a plain WKWebView getter on the main thread.
                                    let z = unsafe { view.pageZoom() }.max(0.01);
                                    let b = view.bounds();
                                    job.text.push_str(&format!(
                                        "The page shows {:.0}×{:.0} CSS pixels (browser_mouse_click_xy takes a point in those; scale from the picture's size).\n",
                                        b.size.width / z,
                                        b.size.height / z
                                    ));
                                    crate::browser::snapshot_view(view, None, move |png| {
                                        page(png.map(|d| tc_agent::shell::base64(&d)).ok_or_else(|| "the page gave no picture".to_string()));
                                    });
                                    job.waiting = Some(Waiting::Picture { seq, since: now });
                                }
                                Op::Go(_) | Op::Settle | Op::Dialog(..) | Op::Pause(_) => {}
                            }
                            self.browser_seq += 1;
                            job.seqs.push(seq);
                        }
                    }
                }
            }
        }
    }

    /// The user's answer to a site question in a browser pane.
    pub fn answer_site(&mut self, id: PaneId, answer: Answer) {
        let waiting = self.browser_jobs.iter().any(|j| j.browser == id);
        let Some(b) = self.browsers.get_mut(&id) else { return };
        let Some(ask) = b.ask.as_mut() else { return };
        if waiting {
            ask.answer = Some(answer);
            return;
        }
        // No call waits any more (it timed out): the answer acts by itself.
        let (url, host) = (ask.url.clone(), ask.host.clone());
        let pane = b.driver.as_ref().map(|d| d.pane);
        let project = b.project;
        b.ask = None;
        if answer != Answer::Block {
            b.pass = Some(host.clone());
            b.navigate(&url);
        }
        if answer == Answer::Always
            && let Some(p) = self.project_mut(project)
            && !p.sites.contains(&host)
        {
            p.sites.push(host);
        }
        if let Some(pane) = pane {
            self.asked(pane);
        }
        self.mark_dirty();
    }

    /// The user takes the browser over from its agent: the pane is theirs, and its page has
    /// the keyboard. Handing it back sends the keyboard to the agent's pane.
    pub fn take_over_browser(&mut self, id: PaneId, take: bool) {
        let Some(b) = self.browsers.get_mut(&id) else { return };
        let Some(d) = b.driver.as_mut() else { return };
        d.paused = take;
        d.last = Instant::now();
        let agent = d.pane;
        b.handed_back = !take;
        if take {
            #[cfg(target_os = "macos")]
            crate::browser_popup::agent_stopped();
            self.actions.push(crate::actions::Action::FocusPane(id));
            self.browser_focus_request = Some(id);
        } else {
            self.actions.push(crate::actions::Action::FocusPane(agent));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_for_the_page() {
        let c = helper_call("click", &json!({ "ref": "e3" }));
        assert!(c.ends_with("return JSON.stringify(await window.__tc.run(\"click\", {\"ref\":\"e3\"}));"));
        assert!(eval_call("() => document.title").contains("await (() => document.title)()"));
        assert!(eval_call("document.title").contains("await (async () => (document.title))()"));
        assert!(eval_call("async function () { return 1 }").contains("await (async function () { return 1 })()"));
    }
}
