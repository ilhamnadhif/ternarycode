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
/// Entries are pre-inserted as `None` when a step starts and removed when the job finishes, so
/// late callbacks from timed-out or cancelled jobs never leak entries.
pub type Done = Arc<Mutex<HashMap<u64, Option<Result<String, String>>>>>;

/// After its last call, an agent is shown using the browser this long (unless its turn ends).
const DRIVER_LINGER: Duration = Duration::from_secs(30);
/// Longest wait for a page to finish loading.
const LOAD_LIMIT: Duration = Duration::from_secs(30);
/// Longest wait for the page to answer a step (`browser_wait_for` waits up to 60 s in it).
const STEP_LIMIT: Duration = Duration::from_secs(70);
/// The moment between typed characters.
const KEY_GAP: Duration = Duration::from_millis(12);
/// The moment a page takes to hear it has the keyboard (WebKit tells it after a key sent with
/// it, and an editor such as x.com's then puts its caret back where it was).
#[cfg(target_os = "macos")]
const FOCUS_GAP: Duration = Duration::from_millis(80);
/// Most pages of its own an agent keeps open (browser_tabs new): the panel's strip stays
/// readable.
const AGENT_PAGES: usize = 3;

/// The site of an address, as pages are matched for an agent: its host without `www.`, and its
/// port (two servers on this machine are two sites).
fn site_of(url: &str) -> String {
    let host = crate::browser::host_of(url);
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let (hostport, _) = crate::browser::split_url(url);
    let hostport = hostport.rsplit_once('@').map_or(hostport.as_str(), |(_, h)| h).to_ascii_lowercase();
    match hostport.rsplit_once(':').filter(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        Some((_, port)) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

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
    /// The page takes the keyboard, as when the user clicks in it.
    Focus,
    /// Text put in at once where the page has its selection, as the system's input does
    /// (dictation, the emoji picker): editors take it as typed.
    Insert(String),
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

/// A file an agent may upload: one that exists, in the project's folder (or automation's active
/// worktree, a relative path is taken from there) or the Downloads folder; nothing else of the
/// user's leaves for a site.
fn upload_path(f: &str, root: &std::path::Path, extra_root: Option<&std::path::Path>) -> Result<std::path::PathBuf, String> {
    let raw = std::path::Path::new(f.trim());
    let p = if raw.is_absolute() {
        raw.to_path_buf()
    } else if let Some(extra) = extra_root
        && extra.join(raw).exists()
    {
        extra.join(raw)
    } else {
        root.join(raw)
    };
    let p = p.canonicalize().map_err(|_| format!("No such file: {f}"))?;
    if !p.is_file() {
        return Err(format!("Not a file: {f}"));
    }
    let mut places = vec![root.to_path_buf()];
    if let Some(extra) = extra_root {
        places.push(extra.to_path_buf());
    }
    if let Some(dl) = dirs::download_dir().or_else(|| Some(crate::paths::home_dir().join("Downloads"))) {
        places.push(dl);
    }
    #[cfg(debug_assertions)]
    if let Some(d) = std::env::var_os("TERNARYCODE_DOWNLOAD_DIR") {
        places.push(d.into());
    }
    if places.iter().filter_map(|d| d.canonicalize().ok()).any(|d| p.starts_with(d)) {
        Ok(crate::paths::canonical(&p))
    } else {
        Err(format!("{f} is outside the project's folder and Downloads: only files there can be uploaded. Ask the user to copy it into the project."))
    }
}

/// The helper's function `name` with `args`, as the body of an async function.
fn helper_call(name: &str, args: &Value) -> String {
    format!("{}\nreturn JSON.stringify(await window.__tc.run({}, {}));", crate::browser::AGENT_JS, json!(name), args)
}

/// The page's editor gets its caret back where it was before the page lost the keyboard
/// (WebKit puts it at the start of the field when the page gets the keyboard back).
pub fn restore_caret(view: &crate::browser::View) {
    crate::browser::run_js(view, &helper_call("caret", &json!({})), |_| {});
}

/// Whether `src` is a top-level JS function or arrow function expression (not an expression
/// that merely contains an inline arrow callback like `.map(a => a.href)`).
fn is_js_function(s: &str) -> bool {
    let rest = if let Some(r) = s.strip_prefix("async")
        && r.chars().next().is_some_and(|c| c.is_ascii_whitespace() || c == '(')
    {
        r.trim_start()
    } else {
        s
    };
    if let Some(r) = rest.strip_prefix("function")
        && r.chars().next().is_some_and(|c| c.is_ascii_whitespace() || matches!(c, '(' | '*'))
    {
        return true;
    }
    if rest.starts_with('(') {
        let mut depth = 0usize;
        let mut quote: Option<char> = None;
        let mut esc = false;
        for (i, c) in rest.char_indices() {
            if let Some(q) = quote {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == q {
                    quote = None;
                }
                continue;
            }
            match c {
                '\'' | '"' | '`' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return rest[i + 1..].trim_start().starts_with("=>");
                    }
                }
                _ => {}
            }
        }
        return false;
    }
    let mut chars = rest.char_indices();
    let Some((_, first)) = chars.next() else { return false };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    let end = chars
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .map_or(rest.len(), |(i, _)| i);
    rest[end..].trim_start().starts_with("=>")
}

/// The agent's function (or expression) run in the page, its result as JSON.
fn eval_call(src: &str) -> String {
    let s = src.trim().trim_end_matches(';').trim();
    let f = if is_js_function(s) { s.to_string() } else { format!("async () => ({s})") };
    format!(
        "try {{ const r = await ({f})(); let s; try {{ s = JSON.stringify(r === undefined ? null : r); }} catch (e) {{ s = JSON.stringify(String(r)); }} return JSON.stringify({{ result: s }}); }} catch (e) {{ return JSON.stringify({{ error: String(e && e.stack || e) }}); }}"
    )
}

/// The agent's status for a site question: asking (with the question), or working again.
fn status(agent: AgentKind, asking: bool, message: Option<String>) -> StatusEvent {
    StatusEvent::Hook { agent, signal: if asking { HookSignal::NeedsInput } else { HookSignal::Working }, message }
}

impl App {
    /// Resolves the project owning an agent pane — either a live terminal pane or an active
    /// background automation's virtual MCP pane.
    fn pane_project_id(&self, agent_pane: PaneId) -> Option<tc_core::ProjectId> {
        self.panes
            .get(&agent_pane)
            .map(|p| p.project)
            .or_else(|| self.automation_mcp_project(agent_pane))
    }

    /// An agent's browser tool call: becomes a job (or is answered at once when it can't run).
    pub fn browser_call(&mut self, call: CallMsg, reply: Reply) {
        let fail = |reply: Reply, e: &str| {
            let _ = reply.send(json!({ "error": e }));
        };
        if !crate::browser::supported() {
            return fail(reply, crate::browser::UNSUPPORTED);
        }
        let agent_pane = call.pane;
        let Some(project) = self.pane_project_id(agent_pane) else {
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
        let who = self
            .automation_mcp_label(agent_pane)
            .unwrap_or_else(|| call.agent.as_deref().and_then(AgentKind::from_slug).map_or("The agent", AgentKind::label).to_string());
        if call.call == "browser_close" {
            let panel = call.args["panel"].as_bool().unwrap_or(false);
            let page = self.agent_page(agent_pane, project);
            let _ = reply.send(self.close_for_agent(agent_pane, project, page, panel, &who));
            return;
        }
        let a = &call.args;
        let s = |k: &str| a[k].as_str().unwrap_or("").trim().to_string();
        // browser_tabs: the pages of the panel; one to act on, a new one, one closed.
        let mut picked_page = None;
        let mut note = String::new();
        if call.call == "browser_tabs" {
            let pages = self.panel_of(project).map(|p| p.pages.clone()).unwrap_or_default();
            let index = a["index"].as_f64().map(|n| n as usize);
            let at = |n: Option<usize>| n.and_then(|n| n.checked_sub(1)).and_then(|i| pages.get(i).copied());
            match a["action"].as_str().unwrap_or("list") {
                "list" => {
                    let _ = reply.send(json!({ "text": self.page_list(agent_pane, project) }));
                    return;
                }
                "close" => {
                    let Some(page) = at(index) else { return fail(reply, &format!("No tab {} here: browser_tabs list shows them.", index.unwrap_or(0))) };
                    let _ = reply.send(self.close_for_agent(agent_pane, project, Some(page), false, &who));
                    return;
                }
                "select" => {
                    let Some(page) = at(index) else { return fail(reply, &format!("No tab {} here: browser_tabs list shows them.", index.unwrap_or(0))) };
                    if let Some(d) = self.browsers.get(&page).and_then(|b| b.driver.as_ref()).filter(|d| d.pane != agent_pane && !d.paused) {
                        return fail(reply, &format!("{} is using that page now: pick another, or open one of your own (browser_tabs new).", d.who));
                    }
                    picked_page = Some(page);
                }
                "new" => {
                    let own: Vec<usize> = pages.iter().enumerate().filter(|(_, id)| self.browsers.get(id).is_some_and(|b| b.owner == Some(agent_pane))).map(|(i, _)| i + 1).collect();
                    if own.len() >= AGENT_PAGES {
                        let tabs: Vec<String> = own.iter().map(usize::to_string).collect();
                        return fail(reply, &format!("You have {} pages open already (tabs {}): close one first (browser_tabs close).", own.len(), tabs.join(", ")));
                    }
                    let Some(id) = self.new_agent_page(agent_pane, project) else { return fail(reply, "The project is gone.") };
                    let url = crate::browser::normalize(&s("url"));
                    if url.is_empty() {
                        self.agent_pages.insert(agent_pane, id);
                        let n = self.panel_of(project).and_then(|p| p.pages.iter().position(|x| *x == id)).map_or(0, |i| i + 1);
                        let _ = reply.send(json!({ "text": format!("Opened a new page of yours (tab {n}); your tools act on it now. browser_navigate opens an address in it.") }));
                        self.mark_dirty();
                        return;
                    }
                    picked_page = Some(id);
                }
                other => return fail(reply, &format!("Unknown action {other}: list, new, select or close.")),
            }
            if let Some(page) = picked_page {
                self.agent_pages.insert(agent_pane, page);
                self.bring_agent_page(agent_pane, project, page);
            }
        }
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
            "browser_tabs" => {
                let url = crate::browser::normalize(&s("url"));
                if a["action"].as_str() == Some("new") && !url.is_empty() {
                    (tf("Opening {}", &[&crate::browser::split_url(&url).0]), vec![Op::Go(url), Op::Read])
                } else {
                    (t("Reading the page").into(), vec![Op::Read])
                }
            }
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
                let extra_root = self.automation_mcp_work_dir(agent_pane);
                let mut files = Vec::new();
                for f in a["paths"].as_array().cloned().unwrap_or_default() {
                    let f = f.as_str().unwrap_or("");
                    match upload_path(f, &root, extra_root.as_deref()) {
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
                // macOS: an editor (contenteditable) gets the text as typed, by the system's input
                // (`native`), the page having the keyboard first.
                let native = cfg!(target_os = "macos");
                let args = json!({ "ref": s("ref"), "text": a["text"].as_str().unwrap_or(""), "submit": a["submit"].as_bool().unwrap_or(false), "native": native });
                let mut ops = vec![Op::Act("type", args, format!("Typed into {e}.")), Op::Settle, Op::Read];
                if native {
                    ops.insert(0, Op::Native(Native::Focus, String::new()));
                }
                (tf("Typing into {}", &[&e]), ops)
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
        let target = if call.call == "browser_navigate" {
            let (page, borrowed) = self.navigate_target(agent_pane, project, &crate::browser::normalize(&s("url")));
            note = borrowed;
            page
        } else {
            picked_page.or_else(|| self.agent_page(agent_pane, project))
        };
        let Some(id) = target else {
            return fail(reply, "No page is open in this project's browser yet: use browser_navigate first.");
        };
        self.agent_pages.insert(agent_pane, id);
        // A new session of the agent in this terminal: the page is where an earlier one left it.
        if let Some(session) = self.panes.get(&agent_pane).and_then(|p| p.agent_session.clone()) {
            let before = self.agent_page_sessions.insert(agent_pane, session.clone());
            if before.is_some_and(|b| b != session)
                && call.call != "browser_navigate"
                && let Some(b) = self.browsers.get(&id)
            {
                note.push_str(&format!("This page was left by an earlier session in this terminal, at {} (“{}”).\n", b.url, b.label()));
            }
        }
        // The first call after a prompt shows the panel (the user may hide it again).
        if !self.browsers.get(&id).is_some_and(|b| b.driver.as_ref().is_some_and(|d| d.paused)) {
            self.show_for_prompt(agent_pane, project, id);
        }
        let Some(b) = self.browsers.get_mut(&id) else { return };
        if b.driver.as_ref().is_some_and(|d| d.paused) {
            return fail(reply, &format!("The user has the browser now (they took it over from {who} in TernaryCode). Wait until they hand it back, or ask them."));
        }
        b.driver = Some(Driver { pane: agent_pane, who, doing, last: Instant::now(), paused: false });
        // The user handed the browser back: the agent hears where they left it.
        let mut text = if std::mem::take(&mut b.handed_back) {
            format!("The user had taken the browser over and handed it back. It is at {} ({}).\n", b.url, b.title)
        } else {
            String::new()
        };
        text.push_str(&note);
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

    /// The page an agent's tools act on: the one it last used (its own, or one of the user's it
    /// was given or took), else its own first in the panel.
    pub fn agent_page(&self, agent_pane: PaneId, project: tc_core::ProjectId) -> Option<PaneId> {
        let pages = self.panel_of(project).map(|p| p.pages.as_slice()).unwrap_or_default();
        self.agent_pages
            .get(&agent_pane)
            .copied()
            .filter(|id| pages.contains(id))
            .or_else(|| pages.iter().copied().find(|id| self.browsers.get(id).is_some_and(|b| b.owner == Some(agent_pane))))
    }

    /// Where browser_navigate opens `url`: the agent's page when it is on that site already;
    /// else a page of the user's on that site, open and idle (no sound, no form being filled
    /// in, no camera, not in the user's hands or another agent's: the user opened YouTube and
    /// asks for a song there), which stays theirs; else the agent's own page (made if need be:
    /// never a page of the user's on another site). With a note for the answer when it takes a
    /// page of the user's.
    fn navigate_target(&mut self, agent_pane: PaneId, project: tc_core::ProjectId, url: &str) -> (Option<PaneId>, String) {
        let site = site_of(url);
        let current = self.agent_page(agent_pane, project);
        if let Some(c) = current
            && !site.is_empty()
            && self.browsers.get(&c).is_some_and(|b| site_of(&b.url) == site)
        {
            return (Some(c), String::new());
        }
        if let Some(page) = self.idle_user_page(agent_pane, project, &site) {
            self.bring_agent_page(agent_pane, project, page);
            let n = self.panel_of(project).and_then(|p| p.pages.iter().position(|x| *x == page)).map_or(0, |i| i + 1);
            let label = self.browsers.get(&page).map(|b| b.label()).unwrap_or_default();
            let note = format!("The user had {site} open already (tab {n}, “{label}”, idle): you work in that page instead of opening another. It stays the user's page.\n");
            return (Some(page), note);
        }
        let own = current.filter(|c| self.browsers.get(c).is_some_and(|b| b.owner == Some(agent_pane)));
        let own = own.or_else(|| {
            let pages = self.panel_of(project).map(|p| p.pages.clone()).unwrap_or_default();
            pages.into_iter().find(|id| self.browsers.get(id).is_some_and(|b| b.owner == Some(agent_pane)))
        });
        (own.or_else(|| self.new_agent_page(agent_pane, project)), String::new())
    }

    /// A page of the user's on `site` an agent may work in: open and idle, the one in front
    /// first.
    fn idle_user_page(&self, agent_pane: PaneId, project: tc_core::ProjectId, site: &str) -> Option<PaneId> {
        if site.is_empty() {
            return None;
        }
        let panel = self.panel_of(project)?;
        let mut pages = panel.pages.clone();
        if let Some(f) = panel.front {
            pages.retain(|p| *p != f);
            pages.insert(0, f);
        }
        let taken: HashSet<PaneId> = self.agent_pages.iter().filter(|(a, _)| **a != agent_pane).map(|(_, p)| *p).collect();
        pages.into_iter().find(|id| {
            let Some(b) = self.browsers.get(id) else { return false };
            b.owner.is_none()
                && site_of(&b.url) == site
                && !b.busy()
                && b.ask.is_none()
                && b.driver.as_ref().is_none_or(|d| d.pane == agent_pane && !d.paused)
                && !taken.contains(id)
                && self.browser_focus != Some(*id)
                && crate::browser_popup::popup_of(*id).is_none()
        })
    }

    /// A new page of the agent's in its project's panel (empty, to be given an address).
    fn new_agent_page(&mut self, agent_pane: PaneId, project: tc_core::ProjectId) -> Option<PaneId> {
        self.project(project)?;
        let id = self.add_page(project, "", Some(agent_pane));
        self.bring_agent_page(agent_pane, project, id);
        Some(id)
    }

    /// A page an agent turns to comes to the front when the agent's terminal is the one picked
    /// (or the panel shows none). The panel opens by `show_for_prompt`.
    fn bring_agent_page(&mut self, agent_pane: PaneId, project: tc_core::ProjectId, page: PaneId) {
        let picked = self.project(project).and_then(|p| p.current_pane()) == Some(agent_pane);
        if let Some(panel) = self.panel_mut(project)
            && (picked || panel.shown().is_none())
        {
            panel.front = Some(page);
        }
    }

    /// The first browser call of an agent after a prompt (the user pressed Enter in its
    /// terminal since the panel last showed for it, or it never did) shows the project's panel with the
    /// agent's page (Settings: else the Browser button lights), unless the Code drawer is open:
    /// then the Browser button says so. The user may hide it again: the agent's later calls
    /// leave it hidden until the next prompt. The keyboard, the view and the tab in view stay as
    /// they are.
    fn show_for_prompt(&mut self, agent_pane: PaneId, project: tc_core::ProjectId, page: PaneId) {
        // A new prompt: Enter in the agent's terminal since the panel last showed for it (keys
        // typed meanwhile are not one: the panel would come back and the agent's page to the
        // front at each of its calls while the user types).
        let typed = self.panes.get(&agent_pane).and_then(|p| p.last_prompt);
        let prompted = match self.agent_panel_shown.get(&agent_pane) {
            None => true,
            Some(shown) => typed.is_some_and(|t| t > *shown),
        };
        if !prompted {
            return;
        }
        self.agent_panel_shown.insert(agent_pane, Instant::now());
        if !self.settings.browser_agent_beside {
            return;
        }
        self.bring_agent_page(agent_pane, project, page);
        if let Some(panel) = self.panel_mut(project) {
            panel.open = true;
        }
        // Only leave the Code Editor in front if the user actually has an open file in it;
        // an empty editor overview yields automatically to the agent's Browser panel.
        if !self.editors.get(&project).is_some_and(|e| e.visible && !e.files.is_empty()) {
            self.hide_drawer(project);
        }
        self.mark_dirty();
    }

    /// The panel's pages as an agent's browser_tabs list gives them.
    fn page_list(&self, agent_pane: PaneId, project: tc_core::ProjectId) -> String {
        let Some(panel) = self.panel_of(project) else { return "The project is gone.".into() };
        if panel.pages.is_empty() {
            return "No page is open in this project's Browser panel. browser_navigate opens one.".into();
        }
        let current = self.agent_page(agent_pane, project);
        let mut out = format!("Pages in this project's Browser panel (the panel is {}):", if panel.open { "shown" } else { "hidden" });
        for (i, id) in panel.pages.iter().enumerate() {
            let Some(b) = self.browsers.get(id) else { continue };
            let mut marks = vec![self.page_owner(b, agent_pane)];
            if current == Some(*id) {
                marks.push("your tools act here".into());
            }
            if panel.open && panel.front == Some(*id) {
                marks.push("in front".into());
            }
            if b.state.audible {
                marks.push(if b.muted { "playing sound, muted by the user".into() } else { "playing sound".into() });
            }
            if b.state.editing {
                marks.push("a form being filled in".into());
            }
            if b.asleep && b.view.is_none() {
                marks.push("asleep (it loads again when used)".into());
            }
            if let Some(d) = &b.driver {
                if d.paused {
                    marks.push("the user took it over".into());
                } else if d.pane != agent_pane {
                    marks.push(format!("{} is using it", d.who));
                }
            }
            let url = if b.url.is_empty() { "no address yet".to_string() } else { b.url.clone() };
            out.push_str(&format!("\n{}. “{}” {url} · {}", i + 1, b.label(), marks.join(" · ")));
        }
        out.push_str("\nbrowser_tabs select N makes your tools act on tab N (a page of the user's: when they point you to it); new opens a page of your own; close N closes tab N (another's only when the user asks).");
        out
    }

    /// Whose a page is, as an agent hears it.
    fn page_owner(&self, b: &crate::browser::Browser, agent_pane: PaneId) -> String {
        match b.owner {
            None => "the user's".into(),
            Some(o) if o == agent_pane => "yours".into(),
            Some(o) => {
                if let Some(label) = self.automation_mcp_label(o) {
                    format!("{label}'s")
                } else {
                    let kind = self.panes.get(&o).and_then(|p| p.tracker.agent.or(p.last_agent));
                    format!("{}'s", kind.map_or("another agent", AgentKind::label))
                }
            }
        }
    }

    /// An agent closes `page` (browser_close: the page it acts on; browser_tabs close: a tab);
    /// a job still on it is told the page was closed. With `panel` the project's Browser panel
    /// hides too (the pages left in it keep running). The answer says what the panel shows now.
    fn close_for_agent(&mut self, agent_pane: PaneId, project: tc_core::ProjectId, page: Option<PaneId>, panel: bool, who: &str) -> Value {
        let page = page.and_then(|id| self.browsers.get(&id));
        if page.and_then(|b| b.driver.as_ref()).is_some_and(|d| d.paused) {
            return json!({ "error": format!("The user has the browser now (they took it over from {who} in TernaryCode). Wait until they hand it back, or ask them.") });
        }
        let mut text = match page.map(|b| (b.id, self.page_owner(b, agent_pane), b.label())) {
            Some((id, owner, label)) => {
                self.close_page(id);
                if owner == "yours" { "Closed your page in TernaryCode's browser.".to_string() } else { format!("Closed {owner} page “{label}” in TernaryCode's browser.") }
            }
            None => "You had no page open in TernaryCode's browser.".to_string(),
        };
        let left = self.panel_of(project).map(|p| p.pages.len()).unwrap_or(0);
        let open = self.panel_of(project).is_some_and(|p| p.open);
        if panel && open {
            self.hide_panel(project);
            text.push_str(" The Browser panel is hidden.");
            if left > 0 {
                text.push_str(&format!(" Its other pages ({left}) keep running out of view; the Browser button shows them again."));
            }
        } else if left == 0 {
            text.push_str(" No page is open, so the Browser panel is closed.");
        } else if !open {
            text.push_str(&format!(" The Browser panel is hidden; its other pages ({left}) keep running out of view."));
        } else {
            let front = self.panel_of(project).and_then(|p| p.front).and_then(|id| self.browsers.get(&id));
            let showing = front.map(|b| if b.title.is_empty() { b.url.clone() } else { b.title.clone() }).filter(|s| !s.is_empty());
            let showing = showing.map(|s| format!(": it shows “{s}” now")).unwrap_or_default();
            text.push_str(&format!(" Other pages stay in the Browser panel ({left}: the user's or other agents'){showing}; browser_close with panel true hides the panel."));
        }
        self.mark_dirty();
        json!({ "text": text })
    }

    /// Other pages of the project playing sound while `page` plays too: the agent hears of
    /// them (the user may want only one).
    fn sound_note(&self, page: PaneId, agent_pane: PaneId) -> String {
        let Some(b) = self.browsers.get(&page).filter(|b| b.state.audible && !b.muted) else { return String::new() };
        let Some(panel) = self.panel_of(b.project) else { return String::new() };
        let mut out = String::new();
        for (i, id) in panel.pages.iter().enumerate() {
            let Some(o) = self.browsers.get(id).filter(|o| o.id != page && o.state.audible && !o.muted) else { continue };
            let n = i + 1;
            let whose = match self.page_owner(o, agent_pane).as_str() {
                "yours" => "your page".to_string(),
                owner => format!("{owner} page"),
            };
            out.push_str(&format!(
                "\nAlso playing sound: tab {n}, “{}” ({whose}). If the user wants only one playing, pause it there (browser_tabs select {n}), or ask them.",
                o.label()
            ));
        }
        out
    }

    /// Sites `agent_pane` in `project` may open without asking: unattended automations with
    /// Auto-approve tools enabled can open any site without stalling on an interactive prompt.
    fn agent_sites_for(&self, project: tc_core::ProjectId, agent_pane: PaneId) -> Vec<String> {
        if self.automation_mcp_auto_approve(agent_pane) {
            return vec!["*".into()];
        }
        self.agent_sites(project)
    }

    /// The sites an agent of `project` may open without asking: any (unless the user wants to
    /// be asked, Settings → Browser), else every project's and its own.
    pub fn agent_sites(&self, project: tc_core::ProjectId) -> Vec<String> {
        if !self.settings.browser_agent_ask {
            return vec!["*".into()];
        }
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
        if job.uploading {
            let popup = crate::browser_popup::popup_of(job.browser);
            let view = popup.or_else(|| self.browsers.get(&job.browser).and_then(|b| b.view.as_ref().map(crate::browser::web_view_of)));
            if let Some(wk) = view.as_deref() {
                let _ = crate::browser_dialog::upload_unused(wk);
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
        // Two pages playing sound: the agent hears of the other.
        let sound = self.sound_note(job.browser, job.agent_pane);
        job.text.push_str(&sound);
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
    pub fn stop_unasked_load(&mut self, id: PaneId, url: &str, before: &str) {
        let Some(project) = self.browsers.get(&id).map(|b| b.project) else { return };
        let Some(agent_pane) = self.browsers.get(&id).and_then(|b| b.driver.as_ref().filter(|d| !d.paused).map(|d| d.pane)) else { return };
        let sites = self.agent_sites_for(project, agent_pane);
        let Some(b) = self.browsers.get_mut(&id) else { return };
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
    pub fn run_browser_jobs(&mut self, ctx: &eframe::egui::Context) {
        let now = Instant::now();
        // Agents that stopped using their browser: the pane says so no longer.
        let jobs: HashSet<PaneId> = self.browser_jobs.iter().map(|j| j.browser).collect();
        for b in self.browsers.values_mut() {
            let Some(d) = &b.driver else { continue };
            // Taken over, it stays the user's until they hand it back (the agent's next calls are
            // told so), or the agent's pane is gone.
            let is_auto = self
                .active_automation_runs
                .values()
                .any(|r| r.mcp_pane == Some(d.pane) || r.live_pane == Some(d.pane));
            let gone = !self.panes.contains_key(&d.pane) && !is_auto;
            let turn_over = !is_auto
                && self.panes.get(&d.pane).is_none_or(|p| {
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
            let sites = self.agent_sites_for(b.project, self.browser_jobs[i].agent_pane);
            let pass = b.pass.clone();
            let answer = b.ask.as_ref().and_then(|a| a.answer.map(|x| (x, a.host.clone(), a.url.clone())));
            let here = b.url.clone();
            let page_url = view.as_ref().map(|w| crate::browser::url_in(w)).unwrap_or_default();
            let job = &mut self.browser_jobs[i];
            // A pop-up opened (the agent's click, or the user's) or closed since the last step:
            // the agent hears of it.
            let seen = popup.as_ref().map(|w| crate::browser::page_key(w));
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
                    let got = {
                        let mut done = self.browser_done.lock();
                        if done.get(&seq).is_some_and(Option::is_some) {
                            done.remove(&seq).flatten()
                        } else {
                            None
                        }
                    };
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
                                    // An editor took the agent's text as typed: its text is
                                    // selected for the system's input to replace (then Enter).
                                    if name == "type" && v["insert"].as_bool() == Some(true) {
                                        let text = args["text"].as_str().unwrap_or("").to_string();
                                        if args["submit"].as_bool() == Some(true) {
                                            job.ops.push_front(Op::Native(Native::Key("Enter".into()), String::new()));
                                        }
                                        let put = if text.is_empty() { Native::Key("Backspace".into()) } else { Native::Insert(text) };
                                        job.ops.push_front(Op::Native(put, String::new()));
                                    }
                                    if name == "console" {
                                        job.text.push_str(v["text"].as_str().unwrap_or(""));
                                    } else if name != "caret" {
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
                                            job.acted = Some(crate::browser::page_key(wk));
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
                    let got = {
                        let mut done = self.browser_done.lock();
                        if done.get(&seq).is_some_and(Option::is_some) {
                            done.remove(&seq).flatten()
                        } else {
                            None
                        }
                    };
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
                                if here.is_empty() && popup.is_none() {
                                    return Some(Some("This tab has no page loaded yet: use browser_navigate first.".into()));
                                }
                                if now.duration_since(job.since) > LOAD_LIMIT {
                                    return Some(Some("The browser page did not finish opening in time.".into()));
                                }
                                job.ops.push_front(op);
                                return None;
                            };
                            let done = self.browser_done.clone();
                            done.lock().insert(seq, None);
                            let page = move |r: Result<String, String>| {
                                if let Some(slot) = done.lock().get_mut(&seq) {
                                    *slot = Some(r);
                                }
                            };
                            match op {
                                Op::Back => {
                                    crate::browser::back_in(view);
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
                                        job.acted = Some(crate::browser::page_key(view));
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
                                    // Keys go to a page that has its window's keyboard, a moment
                                    // after it took it: else its focus comes after the first key.
                                    // (macOS: the page's own window, `browser_host`, whose
                                    // keyboard is not the user's unless they typed in the page.)
                                    // Its editor gets its caret back where it was (WebKit puts it
                                    // at the start when the page gets the keyboard back).
                                    #[cfg(target_os = "macos")]
                                    if !matches!(native, Native::ClickAt(..)) && crate::browser_input::take_keyboard(view) {
                                        if !matches!(native, Native::Focus) {
                                            job.ops.push_front(Op::Native(native, line));
                                        }
                                        job.ops.push_front(Op::Act("caret", json!({}), String::new()));
                                        job.ops.push_front(Op::Pause(FOCUS_GAP));
                                        continue;
                                    }
                                    if matches!(native, Native::Focus) {
                                        continue;
                                    }
                                    crate::browser_popup::agent_acting(view, sites.clone(), pass.clone());
                                    job.acted = Some(crate::browser::page_key(view));
                                    if !matches!(native, Native::Char(_) | Native::Type(..) | Native::Insert(_)) {
                                        job.redo = Some(Op::Native(native.clone(), line.clone()));
                                    }
                                    let done = match native {
                                        Native::ClickAt(x, y) => crate::browser_input::click(view, x, y, 1).then_some(()).ok_or_else(|| "The page has no window.".to_string()),
                                        Native::Key(k) => crate::browser_input::key(view, &k),
                                        Native::Char(c) => crate::browser_input::type_text(view, &c.to_string()).then_some(()).ok_or_else(|| "The page has no window.".to_string()),
                                        #[cfg(target_os = "macos")]
                                        Native::Insert(text) => crate::browser_input::insert_text(view, &text).then_some(()).ok_or_else(|| "The page has no window.".to_string()),
                                        #[cfg(not(target_os = "macos"))]
                                        Native::Insert(text) => crate::browser_input::type_text(view, &text).then_some(()).ok_or_else(|| "The page has no window.".to_string()),
                                        Native::Focus => Ok(()),
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
                                    let (w, h) = crate::browser::css_size(view);
                                    job.text.push_str(&format!("The page shows {w:.0}×{h:.0} CSS pixels (browser_mouse_click_xy takes a point in those; scale from the picture's size).\n"));
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
            crate::browser_popup::agent_stopped();
            self.actions.push(crate::actions::Action::FocusPane(id));
            self.browser_focus_request = Some(id);
        } else if self.panes.contains_key(&agent) {
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
        assert!(eval_call("[...document.querySelectorAll('a')].map(a => a.href)").contains("await (async () => ([...document.querySelectorAll('a')].map(a => a.href)))()"));
        assert!(eval_call("functionName()").contains("await (async () => (functionName()))()"));
        assert!(eval_call("async () => 1").contains("await (async () => 1)()"));
    }

    #[test]
    fn sites_of_addresses() {
        assert_eq!(site_of("https://www.youtube.com/watch?v=1"), "youtube.com");
        assert_eq!(site_of("https://youtube.com/"), "youtube.com");
        assert_eq!(site_of("http://localhost:3000/a"), "localhost:3000");
        assert_ne!(site_of("http://localhost:3000/"), site_of("http://localhost:5173/"));
        assert_eq!(site_of("https://user@Example.com:8443/x"), "example.com:8443");
        assert_eq!(site_of(""), "");
    }
}
