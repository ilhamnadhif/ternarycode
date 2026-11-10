//! The editor's language servers (LSP), for its IDE features: which server runs for which
//! file, keeping it in step with the open buffers, the requests behind the commands and what is
//! done with the answers.
//!
//! A server runs per (server, root) and starts when a file it serves is opened in the editor
//! (none runs for a user who only uses terminals and agents); it stops 10 minutes after its last
//! file was closed. Its messages reach the UI thread as `AppMsg::Lsp`. Servers are installed by
//! the user: a missing one shows its install command in the editor's status line.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::{Value, json};
use tc_lsp::assist::Block;
use tc_lsp::client::{Client, Incoming, RpcError};
use tc_lsp::pos::{Encoding, LineIndex};
use tc_lsp::servers::{self, Spec};
use tc_lsp::types as lt;

use crate::app::{App, AppMsg, PaletteMode};
use crate::i18n::{t, tf};

/// A server with no file open stops after this.
pub const IDLE_STOP: Duration = Duration::from_secs(600);
/// Edits are sent once typing pauses this long (or before a request).
const CHANGE_DELAY: Duration = Duration::from_millis(150);
/// Format on save waits this long for the server, then saves the file as it is.
const FORMAT_WAIT: Duration = Duration::from_secs(2);
/// A file made this recently that a server has not heard of is reported as created.
const FRESH: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServerKey {
    pub name: &'static str,
    pub root: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServerState {
    Starting,
    Ready,
    /// It could not start, or it stopped (why).
    Failed(String),
    /// Not installed (not on the user's PATH).
    Missing,
}

pub struct Server {
    pub spec: &'static Spec,
    pub state: ServerState,
    client: Option<Client>,
    pub caps: lt::ServerCapabilities,
    pub enc: Encoding,
    docs: HashMap<PathBuf, Doc>,
    /// What it is busy with (`$/progress`), e.g. "Indexing".
    pub progress: Option<String>,
    tokens: HashMap<String, String>,
    idle_since: Option<Instant>,
    init: Option<i64>,
    pub pid: Option<u32>,
    /// Which start of the server this is (messages of an earlier one are ignored).
    generation: u64,
    /// Files it published diagnostics for (they go when it stops).
    published: HashSet<PathBuf>,
    /// Files it was told exist (`workspace/didChangeWatchedFiles`).
    seen: HashSet<PathBuf>,
}

struct Doc {
    lang: &'static str,
    version: i32,
    /// The buffer revision the server has (none: not opened with it yet).
    sent: Option<u64>,
    /// When a newer revision was first seen.
    changed: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goto {
    Definition,
    TypeDefinition,
    Implementation,
}

/// What a request was for.
#[derive(Clone, Debug)]
enum Pending {
    Goto(Goto),
    References,
    Hover { path: PathBuf, at: usize },
    /// `seq`: which completion request (only the latest one's answer is shown); `rev`: the
    /// file's revision it was asked at.
    Completion { path: PathBuf, start: usize, seq: u64, rev: u64 },
    Signature { path: PathBuf },
    PrepareRename { path: PathBuf, at: usize },
    Rename { revs: Revs },
    CodeActions { path: PathBuf, revs: Revs },
    Execute,
    Format { path: PathBuf, rev: u64, then_save: bool, asked: Instant },
    /// The file's symbols, for the file asked about (not whichever is active when they come).
    Symbols { path: PathBuf },
    WorkspaceSymbols,
}

/// The editor's IDE commands (the active file, at its caret unless said otherwise).
#[derive(Clone, Debug, PartialEq)]
pub enum IdeCmd {
    Goto(Goto),
    /// ⌘-click: go to the definition of what is at this char.
    DefinitionAt(usize),
    References,
    Rename,
    /// Rename to this name (the rename field was confirmed).
    RenameTo(String),
    CodeActions,
    /// Run code action `n` of the open menu.
    RunAction(usize),
    Format,
    Symbols,
    WorkspaceSymbols,
    Problems,
    Back,
    Forward,
    /// Completion: `true` when asked for (⌃Space), else while typing.
    Complete(bool),
    Signature,
    HoverAt(usize),
    UndoRefactor,
    RestartServer,
}

/// A place in a file (line and char column, 0-based) for the location lists.
#[derive(Clone, Debug)]
pub struct LocItem {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub label: String,
    pub detail: String,
    pub error: bool,
}

pub struct HoverPopup {
    pub path: PathBuf,
    pub at: usize,
    pub blocks: Vec<Block>,
}

pub struct CompletionPopup {
    pub path: PathBuf,
    /// Where the word being completed starts (char index).
    pub start: usize,
    pub items: Vec<lt::CompletionItem>,
    /// Items matching what was typed, best first.
    pub shown: Vec<usize>,
    pub selected: usize,
    pub enc: Encoding,
    /// The server said the list is incomplete: when typing narrows it to nothing, it is asked
    /// again rather than closed (it may have what was typed).
    pub incomplete: bool,
}

pub struct SignaturePopup {
    pub path: PathBuf,
    pub label: String,
    /// The active parameter in `label` (char range).
    pub active: Option<(usize, usize)>,
}

pub struct RenameField {
    pub path: PathBuf,
    pub at: usize,
    pub text: String,
    pub focus: bool,
}

pub struct ActionsMenu {
    pub path: PathBuf,
    pub key: ServerKey,
    pub items: Vec<lt::CodeActionOrCommand>,
    pub selected: usize,
    /// The open files' revisions the actions were made for.
    revs: Revs,
}

/// One refactor's files before and after, to undo it.
pub struct RefactorUndo {
    pub files: Vec<(PathBuf, String, String)>,
    pub at: Instant,
}

/// The popups, lists and history of the IDE features.
#[derive(Default)]
pub struct Ide {
    pub hover: Option<HoverPopup>,
    /// The pointer rests on a char of a file since: hover is asked once it rested long enough.
    pub hover_wait: Option<(PathBuf, usize, Instant)>,
    pub hover_asked: Option<(PathBuf, usize)>,
    pub completion: Option<CompletionPopup>,
    pub signature: Option<SignaturePopup>,
    pub rename: Option<RenameField>,
    pub actions: Option<ActionsMenu>,
    pub back: Vec<(PathBuf, usize)>,
    pub forward: Vec<(PathBuf, usize)>,
    pub undo: Option<RefactorUndo>,
    /// A completion item was clicked: accept it with the next keys.
    pub accept_pending: bool,
    /// Where the hover popup was drawn (it stays while the pointer is on it).
    pub hover_rect: Option<eframe::egui::Rect>,
}

#[derive(Default)]
pub struct Lsp {
    pub servers: HashMap<ServerKey, Server>,
    pending: HashMap<(ServerKey, i64), Pending>,
    /// Diagnostics per file and the encoding of their positions.
    pub diagnostics: HashMap<PathBuf, (Encoding, Vec<lt::Diagnostic>)>,
    /// The user's PATH, once read from their login shell.
    pub path_env: Option<OsString>,
    pub ide: Ide,
    next_generation: u64,
    /// Completion requests sent so far (an answer to an older one is dropped).
    completion_seq: u64,
    /// The server's last list was incomplete: its file, where its word starts and the revision
    /// last asked for. Typing on in that word asks again rather than only narrowing it.
    incomplete: Option<(PathBuf, usize, u64)>,
}

impl Lsp {
    /// The server a file is open with.
    fn key_of(&self, path: &Path) -> Option<ServerKey> {
        self.servers
            .iter()
            .find(|(_, s)| s.docs.contains_key(path))
            .map(|(k, _)| k.clone())
    }

    /// The server serving a file and its state, for the status line.
    pub fn server_for(&self, path: &Path) -> Option<&Server> {
        self.servers.values().find(|s| s.docs.contains_key(path))
    }

    /// Errors and warnings in a file.
    pub fn counts(&self, path: &Path) -> (usize, usize) {
        self.diagnostics.get(path).map_or((0, 0), |(_, d)| {
            let errors = d.iter().filter(|d| d.severity == Some(lt::DiagnosticSeverity::ERROR)).count();
            let warnings = d.iter().filter(|d| d.severity == Some(lt::DiagnosticSeverity::WARNING)).count();
            (errors, warnings)
        })
    }
}

/// Revisions of the files open in editors when a request that answers with edits was sent:
/// the edits are for that text.
type Revs = HashMap<PathBuf, u64>;

/// Text and revision of a file open in an editor.
fn open_text<'a>(editors: &'a HashMap<u64, crate::editor::Editor>, path: &Path) -> Option<(&'a str, u64)> {
    editors
        .values()
        .flat_map(|e| e.files.iter())
        .find(|f| f.path == path)
        .map(|f| (f.text.as_str(), f.rev))
}

/// The program to start for a server: a test server in place of every server
/// (`TERNARYCODE_LSP_OVERRIDE`, debug builds), else the spec's.
fn program_for(spec: &'static Spec) -> (String, Vec<&'static str>) {
    if cfg!(debug_assertions)
        && let Some(p) = std::env::var_os("TERNARYCODE_LSP_OVERRIDE")
    {
        return (p.to_string_lossy().into_owned(), Vec::new());
    }
    (spec.program.to_string(), spec.args.to_vec())
}

fn init_params(root: &Path) -> Value {
    let uri = tc_lsp::pos::uri(root);
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    json!({
        "processId": std::process::id(),
        "rootUri": uri.as_str(),
        "rootPath": root.to_string_lossy(),
        "workspaceFolders": [{"uri": uri.as_str(), "name": name}],
        "clientInfo": {"name": "TernaryCode", "version": env!("CARGO_PKG_VERSION")},
        "capabilities": {
            "general": {"positionEncodings": ["utf-8", "utf-16"]},
            "textDocument": {
                "synchronization": {"didSave": true},
                "completion": {
                    "completionItem": {"snippetSupport": true, "documentationFormat": ["markdown", "plaintext"]},
                    "contextSupport": true
                },
                "hover": {"contentFormat": ["markdown", "plaintext"]},
                "signatureHelp": {"signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": {"labelOffsetSupport": true},
                    "activeParameterSupport": true
                }},
                "definition": {"linkSupport": true},
                "typeDefinition": {"linkSupport": true},
                "implementation": {"linkSupport": true},
                "references": {},
                "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
                "codeAction": {
                    "codeActionLiteralSupport": {"codeActionKind": {"valueSet": [
                        "", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite",
                        "source", "source.organizeImports", "source.fixAll"
                    ]}},
                    "isPreferredSupport": true
                },
                "formatting": {},
                "rename": {"prepareSupport": true},
                "publishDiagnostics": {"versionSupport": true}
            },
            "workspace": {
                "applyEdit": true,
                "workspaceEdit": {"documentChanges": true},
                "configuration": true,
                "workspaceFolders": true,
                "symbol": {},
                "didChangeWatchedFiles": {"dynamicRegistration": false}
            },
            "window": {"workDoneProgress": true}
        }
    })
}

impl App {
    // ----- servers ---------------------------------------------------------------------------

    /// Reads the user's PATH in the background (servers are looked up on it).
    pub fn lsp_find_path(&self) {
        let tx = self.app_tx.clone();
        self.rt.spawn_blocking(move || {
            let _ = tx.send(AppMsg::LspPath(servers::user_path()));
        });
    }

    pub fn lsp_path_found(&mut self, ctx: &egui::Context, path: OsString) {
        self.lsp.path_env = Some(path);
        // Servers missing on the system PATH may be on the user's.
        let missing: Vec<ServerKey> = self
            .lsp
            .servers
            .iter()
            .filter(|(_, s)| s.state == ServerState::Missing)
            .map(|(k, _)| k.clone())
            .collect();
        for key in missing {
            if let Some(s) = self.lsp.servers.remove(&key) {
                for path in s.docs.keys() {
                    self.lsp_open(ctx, path);
                }
            }
        }
    }

    /// A file was opened in the editor: its server serves it (started if needed).
    pub fn lsp_open(&mut self, ctx: &egui::Context, path: &Path) {
        if !self.settings.lsp {
            return;
        }
        let Some((spec, lang)) = servers::for_path(path) else { return };
        let big = self
            .editors
            .values()
            .flat_map(|e| e.files.iter())
            .find(|f| f.path == path)
            .is_some_and(|f| f.read_only || f.text.len() > crate::editor::HIGHLIGHT_AT_MOST);
        if big {
            return;
        }
        let project = self
            .projects
            .iter()
            .find(|p| path.starts_with(&p.path))
            .map(|p| p.path.clone())
            .unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf());
        let key = ServerKey {
            name: spec.name,
            root: servers::find_root(spec, path, &project),
        };
        if !self.lsp.servers.contains_key(&key) {
            self.lsp_start(ctx, spec, key.clone());
        }
        if let Some(s) = self.lsp.servers.get_mut(&key) {
            s.idle_since = None;
            s.docs.entry(path.to_path_buf()).or_insert(Doc {
                lang,
                version: 0,
                sent: None,
                changed: None,
            });
        }
        self.lsp_sync(true);
    }

    fn lsp_start(&mut self, ctx: &egui::Context, spec: &'static Spec, key: ServerKey) {
        let path_env = self
            .lsp
            .path_env
            .clone()
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        let (program, args) = program_for(spec);
        let mut server = Server {
            spec,
            state: ServerState::Starting,
            client: None,
            caps: lt::ServerCapabilities::default(),
            enc: Encoding::Utf16,
            docs: HashMap::new(),
            progress: None,
            tokens: HashMap::new(),
            idle_since: None,
            init: None,
            pid: None,
            generation: self.lsp.next_generation,
            published: HashSet::new(),
            seen: HashSet::new(),
        };
        self.lsp.next_generation += 1;
        let generation = server.generation;
        match servers::resolve(&program, &path_env) {
            None => server.state = ServerState::Missing,
            Some(bin) => {
                let tx = self.app_tx.clone();
                let repaint = ctx.clone();
                let k = key.clone();
                let started = Client::spawn(self.rt.handle(), &bin, &args, &key.root, Some(&path_env), move |m| {
                    // Answers are awaited (completion, hover): at once. Notifications come in
                    // floods (progress, diagnostics while indexing): a few per frame.
                    let delay = match &m {
                        Incoming::Response { .. } | Incoming::Request { .. } | Incoming::Exited { .. } => Duration::ZERO,
                        Incoming::Notification { method, .. } if method == "$/progress" => Duration::from_millis(150),
                        Incoming::Notification { .. } => Duration::from_millis(30),
                    };
                    let _ = tx.send(AppMsg::Lsp(k.clone(), generation, m));
                    if delay.is_zero() {
                        repaint.request_repaint();
                    } else {
                        repaint.request_repaint_after(delay);
                    }
                });
                match started {
                    Ok(client) => {
                        server.init = Some(client.request("initialize", init_params(&key.root)));
                        server.pid = client.pid();
                        server.client = Some(client);
                    }
                    Err(e) => server.state = ServerState::Failed(e.to_string()),
                }
            }
        }
        self.lsp.servers.insert(key, server);
    }

    /// Stops every server (the setting was switched off, or the app quits).
    pub fn lsp_stop_all(&mut self) {
        let keys: Vec<ServerKey> = self.lsp.servers.keys().cloned().collect();
        for key in &keys {
            self.lsp_gone(key);
        }
        for (_, s) in self.lsp.servers.drain() {
            if let Some(c) = &s.client {
                c.shutdown();
            }
        }
        self.lsp.pending.clear();
        self.lsp.diagnostics.clear();
        self.lsp.ide = Ide::default();
        self.lsp.incomplete = None;
    }

    /// A server stops (idle, restarted, exited): what it was asked gets no answer, so a file
    /// waiting to be formatted on save is saved as it is; its diagnostics would never update.
    fn lsp_gone(&mut self, key: &ServerKey) {
        let mut unsaved = Vec::new();
        self.lsp.pending.retain(|(k, _), p| {
            if k != key {
                return true;
            }
            if let Pending::Format { path, then_save: true, .. } = p {
                unsaved.push(path.clone());
            }
            false
        });
        if let Some(s) = self.lsp.servers.get_mut(key) {
            for path in s.published.drain() {
                self.lsp.diagnostics.remove(&path);
            }
        }
        for path in unsaved {
            self.save_unformatted(&path, Some(key.name));
        }
    }

    /// Each frame: buffers that changed go to their servers once typing paused; files closed
    /// in the editor are closed with them; servers without files stop after a while.
    pub fn lsp_tick(&mut self, ctx: &egui::Context) {
        if self.lsp.servers.is_empty() {
            return;
        }
        self.lsp_sync(false);
        let now = Instant::now();
        let mut stop = Vec::new();
        let mut wake: Option<Duration> = None;
        for (key, s) in self.lsp.servers.iter_mut() {
            if s.docs.is_empty() {
                let since = *s.idle_since.get_or_insert(now);
                let left = IDLE_STOP.saturating_sub(now.duration_since(since));
                if left.is_zero() {
                    stop.push(key.clone());
                } else {
                    wake = Some(wake.map_or(left, |w| w.min(left)));
                }
            }
        }
        for key in stop {
            self.lsp_gone(&key);
            if let Some(s) = self.lsp.servers.remove(&key)
                && let Some(c) = &s.client
            {
                c.shutdown();
            }
        }
        // Format on save the server did not answer in time: saved as it is (a late answer is
        // dropped with the request).
        let mut late = Vec::new();
        for (id, p) in &self.lsp.pending {
            if let Pending::Format { then_save: true, asked, .. } = p {
                let left = FORMAT_WAIT.saturating_sub(now.duration_since(*asked));
                if left.is_zero() {
                    late.push(id.clone());
                } else {
                    wake = Some(wake.map_or(left, |w| w.min(left)));
                }
            }
        }
        for id in late {
            if let Some(Pending::Format { path, .. }) = self.lsp.pending.remove(&id) {
                self.save_unformatted(&path, Some(id.0.name));
            }
        }
        self.lsp_complete_again(ctx);
        if let Some(w) = wake {
            ctx.request_repaint_after(w);
        }
    }

    /// An incomplete completion list is asked for again once its word changed: narrowing what
    /// the server sent may miss what it has for the word typed now.
    fn lsp_complete_again(&mut self, ctx: &egui::Context) {
        let Some((path, start, asked)) = self.lsp.incomplete.clone() else { return };
        // Only while its list is open: once that closed (an item taken, Esc, nothing left),
        // typing starts a new one.
        let open = self.lsp.ide.completion.as_ref().is_some_and(|c| c.path == path && c.start == start);
        let here = self.ide_target().filter(|_| open);
        let rev = open_text(&self.editors, &path).map(|(_, r)| r);
        let Some(((active, caret), rev)) = here.zip(rev).filter(|((p, _), _)| *p == path) else {
            self.lsp.incomplete = None;
            return;
        };
        if rev == asked {
            return;
        }
        // The caret left the word (or it was deleted): done.
        if caret <= start || word_start(&self.editors, &active, caret) != start {
            self.lsp.incomplete = None;
            return;
        }
        // Asked once per revision, even when no server can take it now.
        self.lsp.incomplete = Some((path, start, rev));
        self.ide_cmd(ctx, IdeCmd::Complete(false));
    }

    /// Sends buffer changes: all that waited `CHANGE_DELAY` (or every one, `now`); opens files
    /// newly served and closes the ones the editor closed.
    fn lsp_sync(&mut self, now_all: bool) {
        let now = Instant::now();
        let editors = &self.editors;
        let mut retry: Option<Duration> = None;
        for s in self.lsp.servers.values_mut() {
            // One not ready (starting, missing, stopped) is sent nothing, but forgets files
            // closed meanwhile: with none left it stops when idle like any other.
            let client = s.client.as_ref().filter(|_| s.state == ServerState::Ready);
            let mut closed = Vec::new();
            for (path, doc) in s.docs.iter_mut() {
                let Some((text, rev)) = open_text(editors, path) else {
                    closed.push(path.clone());
                    continue;
                };
                let Some(client) = client else { continue };
                let uri = tc_lsp::pos::uri(path);
                match doc.sent {
                    None => {
                        doc.version = 1;
                        doc.sent = Some(rev);
                        client.notify(
                            "textDocument/didOpen",
                            json!({"textDocument": {"uri": uri.as_str(), "languageId": doc.lang, "version": 1, "text": text}}),
                        );
                    }
                    Some(sent) if sent != rev => {
                        let since = *doc.changed.get_or_insert(now);
                        if now_all || now.duration_since(since) >= CHANGE_DELAY {
                            doc.version += 1;
                            doc.sent = Some(rev);
                            doc.changed = None;
                            client.notify(
                                "textDocument/didChange",
                                json!({"textDocument": {"uri": uri.as_str(), "version": doc.version}, "contentChanges": [{"text": text}]}),
                            );
                        } else {
                            let left = CHANGE_DELAY - now.duration_since(since);
                            retry = Some(retry.map_or(left, |r| r.min(left)));
                        }
                    }
                    _ => {}
                }
            }
            for path in closed {
                s.docs.remove(&path);
                if let Some(client) = client {
                    client.notify("textDocument/didClose", json!({"textDocument": {"uri": tc_lsp::pos::uri(&path).as_str()}}));
                }
            }
        }
        if let Some(r) = retry {
            self.lsp_repaint_after = Some(r);
        }
    }

    /// A file was saved.
    pub fn lsp_saved(&mut self, path: &Path) {
        self.lsp_sync(true);
        if let Some(key) = self.lsp.key_of(path)
            && let Some(c) = self.lsp.servers.get(&key).and_then(|s| s.client.as_ref())
        {
            c.notify("textDocument/didSave", json!({"textDocument": {"uri": tc_lsp::pos::uri(path).as_str()}}));
        }
    }

    /// Files changed on disk under a project (an agent, git): servers that watch files hear.
    pub fn lsp_files_changed(&mut self, paths: &[PathBuf]) {
        let now = std::time::SystemTime::now();
        for (key, s) in self.lsp.servers.iter_mut() {
            let Some(c) = s.client.as_ref().filter(|_| s.state == ServerState::Ready) else { continue };
            let mut changes: Vec<Value> = Vec::new();
            for p in paths
                .iter()
                .filter(|p| p.starts_with(&key.root) && servers::for_path(p).is_some_and(|(sp, _)| sp.name == s.spec.name))
            {
                // 1 created, 2 changed, 3 deleted. The events don't say which: a file is new
                // when the server never heard of it and it was made just now (or when the
                // system keeps no creation time).
                let kind = match std::fs::metadata(p) {
                    Err(_) => {
                        s.seen.remove(p);
                        3
                    }
                    Ok(m) => {
                        let fresh = m.created().map_or(true, |b| now.duration_since(b).is_ok_and(|age| age < FRESH));
                        let known = !s.seen.insert(p.clone()) || s.docs.contains_key(p);
                        if fresh && !known { 1 } else { 2 }
                    }
                };
                changes.push(json!({"uri": tc_lsp::pos::uri(p).as_str(), "type": kind}));
            }
            if !changes.is_empty() {
                c.notify("workspace/didChangeWatchedFiles", json!({"changes": changes}));
            }
        }
    }

    // ----- messages --------------------------------------------------------------------------

    pub fn lsp_message(&mut self, key: ServerKey, generation: u64, msg: Incoming) {
        if self.lsp.servers.get(&key).is_none_or(|s| s.generation != generation) {
            return;
        }
        match msg {
            Incoming::Response { id, result } => {
                let init = self.lsp.servers.get(&key).and_then(|s| s.init);
                if init == Some(id) {
                    self.lsp_initialized(&key, result);
                } else if let Some(p) = self.lsp.pending.remove(&(key.clone(), id)) {
                    match result {
                        Ok(v) => self.lsp_answer(&key, p, v),
                        // Cancelled or not supported: nothing to show for typing help.
                        Err(_) if matches!(p, Pending::Hover { .. } | Pending::Completion { .. } | Pending::Signature { .. }) => {}
                        Err(e) => {
                            self.toast(tf("{}: {}", &[key.name, &e.message]));
                            // Format on save that failed still saves (unformatted).
                            if let Pending::Format { path, then_save: true, .. } = p {
                                self.save_unformatted(&path, None);
                            }
                        }
                    }
                }
            }
            Incoming::Notification { method, params } => self.lsp_notification(&key, &method, params),
            Incoming::Request { id, method, params } => {
                let reply = self.lsp_server_request(&key, &method, params);
                if let Some(c) = self.lsp.servers.get(&key).and_then(|s| s.client.as_ref()) {
                    c.respond(id, reply);
                }
            }
            Incoming::Exited { code, stderr } => {
                if let Some(s) = self.lsp.servers.get_mut(&key) {
                    s.client = None;
                    s.progress = None;
                    if !matches!(s.state, ServerState::Failed(_)) {
                        let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
                        s.state = ServerState::Failed(if last.is_empty() {
                            tf("stopped (code {})", &[&code.map_or("?".into(), |c| c.to_string())])
                        } else {
                            last
                        });
                    }
                }
                self.lsp_gone(&key);
            }
        }
    }

    fn lsp_initialized(&mut self, key: &ServerKey, result: Result<Value, RpcError>) {
        let Some(s) = self.lsp.servers.get_mut(key) else { return };
        match result {
            Ok(v) => {
                let init: lt::InitializeResult = serde_json::from_value(v).unwrap_or_default();
                s.enc = match &init.capabilities.position_encoding {
                    Some(k) if *k == lt::PositionEncodingKind::UTF8 => Encoding::Utf8,
                    _ => Encoding::Utf16,
                };
                s.caps = init.capabilities;
                s.state = ServerState::Ready;
                if let Some(c) = &s.client {
                    c.notify("initialized", json!({}));
                }
                self.lsp_sync(true);
            }
            Err(e) => {
                s.state = ServerState::Failed(e.message);
                self.lsp_gone(key);
            }
        }
    }

    fn lsp_notification(&mut self, key: &ServerKey, method: &str, params: Value) {
        match method {
            "textDocument/publishDiagnostics" => {
                let Ok(p) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params) else { return };
                let Some(path) = tc_lsp::pos::path(&p.uri) else { return };
                let Some(s) = self.lsp.servers.get_mut(key) else { return };
                let enc = s.enc;
                if p.diagnostics.is_empty() {
                    s.published.remove(&path);
                    self.lsp.diagnostics.remove(&path);
                } else {
                    s.published.insert(path.clone());
                    let mut d = p.diagnostics;
                    d.sort_by_key(|d| (d.range.start.line, d.range.start.character));
                    self.lsp.diagnostics.insert(path, (enc, d));
                }
            }
            "$/progress" => {
                let Ok(p) = serde_json::from_value::<lt::ProgressParams>(params) else { return };
                let token = match &p.token {
                    lt::NumberOrString::Number(n) => n.to_string(),
                    lt::NumberOrString::String(s) => s.clone(),
                };
                let Some(s) = self.lsp.servers.get_mut(key) else { return };
                let lt::ProgressParamsValue::WorkDone(w) = p.value;
                match w {
                    lt::WorkDoneProgress::Begin(b) => {
                        s.tokens.insert(token, b.title.clone());
                        s.progress = Some(b.title);
                    }
                    lt::WorkDoneProgress::Report(r) => {
                        if let Some(title) = s.tokens.get(&token) {
                            s.progress = Some(match (r.message, r.percentage) {
                                (_, Some(pc)) => format!("{title} {pc}%"),
                                (Some(m), _) => format!("{title} · {m}"),
                                _ => title.clone(),
                            });
                        }
                    }
                    lt::WorkDoneProgress::End(_) => {
                        s.tokens.remove(&token);
                        s.progress = s.tokens.values().next().cloned();
                    }
                }
            }
            "window/showMessage" => {
                if let Ok(m) = serde_json::from_value::<lt::ShowMessageParams>(params)
                    && m.typ == lt::MessageType::ERROR
                {
                    self.toast(format!("{}: {}", key.name, m.message));
                }
            }
            _ => {}
        }
    }

    fn lsp_server_request(&mut self, key: &ServerKey, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "workspace/configuration" => {
                let n = params["items"].as_array().map_or(0, Vec::len);
                Ok(Value::Array(vec![Value::Null; n]))
            }
            "client/registerCapability" | "client/unregisterCapability" | "window/workDoneProgress/create" => Ok(Value::Null),
            "window/showMessageRequest" => Ok(Value::Null),
            "workspace/workspaceFolders" => {
                let uri = tc_lsp::pos::uri(&key.root);
                Ok(json!([{"uri": uri.as_str(), "name": key.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()}]))
            }
            "workspace/applyEdit" => {
                let Ok(p) = serde_json::from_value::<lt::ApplyWorkspaceEditParams>(params) else {
                    return Ok(json!({"applied": false}));
                };
                let enc = self.lsp.servers.get(key).map_or(Encoding::Utf16, |s| s.enc);
                match self.apply_workspace_edit(&p.edit, enc, None) {
                    Ok(_) => Ok(json!({"applied": true})),
                    Err(e) => Ok(json!({"applied": false, "failureReason": e})),
                }
            }
            _ => Err(RpcError {
                code: -32601,
                message: format!("{method} is not supported"),
            }),
        }
    }

    // ----- requests --------------------------------------------------------------------------

    /// The active editor file and its caret (char index).
    fn ide_target(&self) -> Option<(PathBuf, usize)> {
        let e = self.editors.get(&self.active?)?;
        let f = e.file()?;
        Some((f.path.clone(), f.caret_index.unwrap_or(0)))
    }

    /// `textDocument` + `position` of a char in an open file, in the server's encoding.
    fn text_pos(&self, path: &Path, at: usize) -> Option<(ServerKey, Value)> {
        let key = self.lsp.key_of(path)?;
        let enc = self.lsp.servers.get(&key)?.enc;
        let (text, _) = open_text(&self.editors, path)?;
        let pos = LineIndex::new(text).position(text, at, enc);
        Some((key, json!({"textDocument": {"uri": tc_lsp::pos::uri(path).as_str()}, "position": pos})))
    }

    /// Sends a request to a ready server (the latest buffer first). False when there is none.
    fn lsp_request(&mut self, key: &ServerKey, method: &str, params: Value, pending: Pending) -> bool {
        self.lsp_sync(true);
        let Some(s) = self.lsp.servers.get(key).filter(|s| s.state == ServerState::Ready) else {
            return false;
        };
        let Some(c) = &s.client else { return false };
        let id = c.request(method, params);
        self.lsp.pending.insert((key.clone(), id), pending);
        true
    }

    /// Why an IDE command can't run for a file: no server for its language, or not ready.
    fn no_server(&mut self, path: &Path) {
        let msg = match self.lsp.server_for(path).map(|s| (&s.state, s.spec)) {
            None if !self.settings.lsp => t("Language features are off (Settings → Editor).").to_string(),
            None => t("No language server for this file.").to_string(),
            Some((ServerState::Missing, spec)) => tf("{} is not installed: {}", &[spec.name, spec.install]),
            Some((ServerState::Starting, spec)) => tf("{} is starting…", &[spec.name]),
            Some((ServerState::Failed(why), spec)) => format!("{}: {why}", spec.name),
            Some((ServerState::Ready, _)) => return,
        };
        self.toast(msg);
    }

    pub fn ide_cmd(&mut self, ctx: &egui::Context, cmd: IdeCmd) {
        match cmd {
            IdeCmd::Back => return self.ide_history(true),
            IdeCmd::Forward => return self.ide_history(false),
            IdeCmd::UndoRefactor => return self.undo_refactor(),
            IdeCmd::Problems => return self.show_problems(),
            IdeCmd::RunAction(n) => return self.run_code_action(n),
            IdeCmd::RestartServer => {
                if let Some((path, _)) = self.ide_target()
                    && let Some(key) = self.lsp.key_of(&path)
                {
                    self.lsp_gone(&key);
                    if let Some(s) = self.lsp.servers.remove(&key) {
                        if let Some(c) = &s.client {
                            c.kill();
                        }
                        // Every file it served goes to the new one, not only the active one.
                        for p in s.docs.keys() {
                            self.lsp_open(ctx, p);
                        }
                    }
                }
                return;
            }
            _ => {}
        }
        let Some((path, caret)) = self.ide_target() else { return };
        // Asked for while typing or hovering: quiet when no server can answer.
        let quiet = matches!(cmd, IdeCmd::HoverAt(_) | IdeCmd::Complete(false) | IdeCmd::Signature);
        let at = match cmd {
            IdeCmd::DefinitionAt(a) | IdeCmd::HoverAt(a) => a,
            _ => caret,
        };
        let Some((key, pos)) = self.text_pos(&path, at) else {
            if !quiet {
                self.no_server(&path);
            }
            return;
        };
        let uri = tc_lsp::pos::uri(&path);
        let doc = json!({"textDocument": {"uri": uri.as_str()}});
        let sent = match cmd {
            IdeCmd::Goto(_) | IdeCmd::DefinitionAt(_) => {
                let kind = if let IdeCmd::Goto(k) = cmd { k } else { Goto::Definition };
                let method = match kind {
                    Goto::Definition => "textDocument/definition",
                    Goto::TypeDefinition => "textDocument/typeDefinition",
                    Goto::Implementation => "textDocument/implementation",
                };
                self.lsp_request(&key, method, pos, Pending::Goto(kind))
            }
            IdeCmd::References => {
                let mut p = pos;
                p["context"] = json!({"includeDeclaration": true});
                self.lsp_request(&key, "textDocument/references", p, Pending::References)
            }
            IdeCmd::HoverAt(at) => self.lsp_request(&key, "textDocument/hover", pos, Pending::Hover { path: path.clone(), at }),
            IdeCmd::Complete(_) => {
                let start = word_start(&self.editors, &path, caret);
                let rev = open_text(&self.editors, &path).map_or(0, |(_, r)| r);
                // Invoked (1): the list is filtered here as typing goes on. Typing on in the word
                // of an incomplete list asks for it again (3).
                let again = match self.lsp.incomplete.as_mut() {
                    Some((p, s, asked)) if *p == path && *s == start => {
                        *asked = rev;
                        true
                    }
                    _ => false,
                };
                let mut p = pos;
                p["context"] = json!({"triggerKind": if again { 3 } else { 1 }});
                self.lsp.completion_seq += 1;
                let seq = self.lsp.completion_seq;
                self.lsp_request(&key, "textDocument/completion", p, Pending::Completion { path: path.clone(), start, seq, rev })
            }
            IdeCmd::Signature => self.lsp_request(&key, "textDocument/signatureHelp", pos, Pending::Signature { path: path.clone() }),
            IdeCmd::Rename => {
                let prepare = self.lsp.servers.get(&key).is_some_and(|s| {
                    matches!(&s.caps.rename_provider, Some(lt::OneOf::Right(o)) if o.prepare_provider == Some(true))
                });
                if prepare {
                    self.lsp_request(&key, "textDocument/prepareRename", pos, Pending::PrepareRename { path: path.clone(), at })
                } else {
                    let word = word_at(&self.editors, &path, at).unwrap_or_default();
                    self.lsp.ide.rename = Some(RenameField { path: path.clone(), at, text: word, focus: true });
                    true
                }
            }
            IdeCmd::RenameTo(name) => {
                let at = self.lsp.ide.rename.take().map_or(at, |r| r.at);
                let Some((key, mut p)) = self.text_pos(&path, at) else { return };
                p["newName"] = json!(name);
                let revs = self.open_revs();
                self.lsp_request(&key, "textDocument/rename", p, Pending::Rename { revs })
            }
            IdeCmd::CodeActions => {
                let diags: Vec<lt::Diagnostic> = self
                    .lsp
                    .diagnostics
                    .get(&path)
                    .map(|(_, d)| d.iter().filter(|d| d.range.start.line <= pos["position"]["line"].as_u64().unwrap_or(0) as u32 && d.range.end.line >= pos["position"]["line"].as_u64().unwrap_or(0) as u32).cloned().collect())
                    .unwrap_or_default();
                let p = json!({"textDocument": {"uri": uri.as_str()}, "range": {"start": pos["position"], "end": pos["position"]}, "context": {"diagnostics": diags}});
                let revs = self.open_revs();
                self.lsp_request(&key, "textDocument/codeAction", p, Pending::CodeActions { path: path.clone(), revs })
            }
            IdeCmd::Format => {
                let (text, rev) = open_text(&self.editors, &path).unwrap_or(("", 0));
                let mut p = doc;
                p["options"] = format_options(text);
                let pending = Pending::Format { path: path.clone(), rev, then_save: false, asked: Instant::now() };
                self.lsp_request(&key, "textDocument/formatting", p, pending)
            }
            IdeCmd::Symbols => self.lsp_request(&key, "textDocument/documentSymbol", doc, Pending::Symbols { path: path.clone() }),
            IdeCmd::WorkspaceSymbols => {
                let q = word_at(&self.editors, &path, caret).unwrap_or_default();
                self.lsp_request(&key, "workspace/symbol", json!({"query": q}), Pending::WorkspaceSymbols)
            }
            _ => false,
        };
        if !sent && !quiet {
            self.no_server(&path);
        }
    }

    /// Saving with "format on save": formats first when a server can, then saves.
    pub fn format_then_save(&mut self) -> bool {
        let Some((path, _)) = self.ide_target() else { return false };
        let Some(key) = self.lsp.key_of(&path) else { return false };
        let can = self
            .lsp
            .servers
            .get(&key)
            .is_some_and(|s| s.state == ServerState::Ready && s.caps.document_formatting_provider.is_some());
        if !can {
            return false;
        }
        let (text, rev) = open_text(&self.editors, &path).unwrap_or(("", 0));
        let p = json!({"textDocument": {"uri": tc_lsp::pos::uri(&path).as_str()}, "options": format_options(text)});
        self.lsp_request(&key, "textDocument/formatting", p, Pending::Format { path, rev, then_save: true, asked: Instant::now() })
    }

    /// Saves a file as it is: formatting it first failed, or `server` gave no answer (it
    /// stopped, or took longer than `FORMAT_WAIT`).
    fn save_unformatted(&mut self, path: &Path, server: Option<&str>) {
        let result = self
            .editors
            .values_mut()
            .flat_map(|e| e.files.iter_mut())
            .find(|f| f.path == path)
            .map(|f| (crate::editor::save(f, false), f.name()));
        match result {
            Some((Ok(()), name)) => {
                self.lsp_saved(path);
                self.toast(match server {
                    Some(server) => tf("Saved {} without formatting: {} did not answer.", &[&name, server]),
                    None => tf("Saved {}.", &[&name]),
                });
            }
            Some((Err(crate::editor::SaveError::ChangedOnDisk), _)) => {
                self.toast(t("Not saved: the file changed on disk. Choose what to keep in the editor.").into())
            }
            Some((Err(crate::editor::SaveError::Io(e)), _)) => self.toast(e),
            None => {}
        }
    }

    // ----- answers ---------------------------------------------------------------------------

    fn lsp_answer(&mut self, key: &ServerKey, pending: Pending, v: Value) {
        let enc = self.lsp.servers.get(key).map_or(Encoding::Utf16, |s| s.enc);
        match pending {
            Pending::Goto(kind) => {
                let locs = goto_locations(v);
                match locs.len() {
                    0 => self.toast(match kind {
                        Goto::Definition => t("No definition found.").into(),
                        Goto::TypeDefinition => t("No type definition found.").into(),
                        Goto::Implementation => t("No implementation found.").into(),
                    }),
                    1 => {
                        let (path, range) = &locs[0];
                        self.goto_range(path, range.start, enc, true);
                    }
                    _ => {
                        let items = self.loc_items(&locs, enc);
                        self.show_locations(t("Definitions"), items);
                    }
                }
            }
            Pending::References => {
                let locs: Vec<(PathBuf, lt::Range)> = serde_json::from_value::<Vec<lt::Location>>(v)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|l| Some((tc_lsp::pos::path(&l.uri)?, l.range)))
                    .collect();
                if locs.is_empty() {
                    self.toast(t("No references found.").into());
                } else {
                    let items = self.loc_items(&locs, enc);
                    self.show_locations(&tf("{} references", &[&items.len().to_string()]), items);
                }
            }
            Pending::Hover { path, at } => {
                let Ok(Some(h)) = serde_json::from_value::<Option<lt::Hover>>(v) else { return };
                let blocks = tc_lsp::assist::hover_blocks(&h.contents);
                if !blocks.is_empty() && self.lsp.ide.hover_asked.as_ref() == Some(&(path.clone(), at)) {
                    self.lsp.ide.hover = Some(HoverPopup { path, at, blocks });
                }
            }
            Pending::Completion { path, start, seq, rev } => {
                // Asked again since (typing went on): only the latest answer fits the text.
                if seq != self.lsp.completion_seq {
                    return;
                }
                let (items, incomplete) = match serde_json::from_value::<Option<lt::CompletionResponse>>(v) {
                    Ok(Some(lt::CompletionResponse::Array(a))) => (a, false),
                    Ok(Some(lt::CompletionResponse::List(l))) => (l.items, l.is_incomplete),
                    _ => (Vec::new(), false),
                };
                self.lsp.incomplete = incomplete.then(|| (path.clone(), start, rev));
                let mut popup = CompletionPopup { path, start, items, shown: Vec::new(), selected: 0, enc, incomplete };
                self.refilter(&mut popup);
                self.lsp.ide.completion = (!popup.shown.is_empty()).then_some(popup);
            }
            Pending::Signature { path } => {
                let Ok(Some(help)) = serde_json::from_value::<Option<lt::SignatureHelp>>(v) else {
                    self.lsp.ide.signature = None;
                    return;
                };
                let idx = help.active_signature.unwrap_or(0) as usize;
                let Some(sig) = help.signatures.get(idx).or(help.signatures.first()) else {
                    self.lsp.ide.signature = None;
                    return;
                };
                let param = sig.active_parameter.or(help.active_parameter).unwrap_or(0) as usize;
                let active = sig.parameters.as_ref().and_then(|ps| ps.get(param)).and_then(|p| match &p.label {
                    lt::ParameterLabel::Simple(s) => {
                        let b = sig.label.find(s.as_str())?;
                        let a = sig.label[..b].chars().count();
                        Some((a, a + s.chars().count()))
                    }
                    lt::ParameterLabel::LabelOffsets([a, b]) => {
                        // UTF-16 offsets in the label.
                        let to_char = |u: u32| {
                            let mut units = 0;
                            sig.label.chars().take_while(|c| {
                                let take = units < u as usize;
                                units += c.len_utf16();
                                take
                            }).count()
                        };
                        Some((to_char(*a), to_char(*b)))
                    }
                });
                self.lsp.ide.signature = Some(SignaturePopup { path, label: sig.label.clone(), active });
            }
            Pending::PrepareRename { path, at } => {
                let word = match serde_json::from_value::<Option<lt::PrepareRenameResponse>>(v) {
                    Ok(Some(lt::PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. })) => Some(placeholder),
                    Ok(Some(lt::PrepareRenameResponse::Range(r))) => {
                        open_text(&self.editors, &path).map(|(text, _)| {
                            let ix = LineIndex::new(text);
                            let (a, b) = ix.chars(text, &r, enc);
                            text.chars().skip(a).take(b - a).collect()
                        })
                    }
                    Ok(Some(lt::PrepareRenameResponse::DefaultBehavior { .. })) => word_at(&self.editors, &path, at),
                    _ => None,
                };
                match word {
                    Some(text) => self.lsp.ide.rename = Some(RenameField { path, at, text, focus: true }),
                    None => self.toast(t("This can't be renamed.").into()),
                }
            }
            Pending::Rename { revs } => {
                let Ok(Some(edit)) = serde_json::from_value::<Option<lt::WorkspaceEdit>>(v) else {
                    self.toast(t("Nothing to rename.").into());
                    return;
                };
                match self.apply_workspace_edit(&edit, enc, Some(&revs)) {
                    Ok(n) => self.toast(tf("Renamed in {} files. Undo: ⌘K → Undo Refactor.", &[&n.to_string()])),
                    Err(e) => self.toast(e),
                }
            }
            Pending::CodeActions { path, revs } => {
                let items = serde_json::from_value::<Option<Vec<lt::CodeActionOrCommand>>>(v).ok().flatten().unwrap_or_default();
                if items.is_empty() {
                    self.toast(t("No code actions here.").into());
                } else {
                    self.lsp.ide.actions = Some(ActionsMenu { path, key: key.clone(), items, selected: 0, revs });
                }
            }
            Pending::Execute => {}
            Pending::Format { path, rev, then_save, .. } => {
                let edits = serde_json::from_value::<Option<Vec<lt::TextEdit>>>(v).ok().flatten().unwrap_or_default();
                let mut saved_name = None;
                let mut failed = None;
                if let Some(f) = self.editors.values_mut().flat_map(|e| e.files.iter_mut()).find(|f| f.path == path) {
                    if f.rev == rev && !edits.is_empty() {
                        match tc_lsp::edit::apply(&f.text, &edits, enc) {
                            Ok(text) if text != f.text => {
                                f.text = text;
                                f.rev += 1;
                                f.lines = crate::editor::count_lines(&f.text);
                            }
                            Ok(_) => {}
                            Err(e) => failed = Some(e),
                        }
                    }
                    if then_save && failed.is_none() {
                        saved_name = Some((crate::editor::save(f, false), f.name()));
                    }
                }
                if let Some(e) = failed {
                    self.toast(e);
                    if then_save {
                        self.save_unformatted(&path, None);
                    }
                    return;
                }
                match saved_name {
                    Some((Ok(()), name)) => {
                        self.toast(tf("Saved {}.", &[&name]));
                        self.lsp_saved(&path);
                    }
                    Some((Err(crate::editor::SaveError::ChangedOnDisk), _)) => {
                        self.toast(t("Not saved: the file changed on disk. Choose what to keep in the editor.").into())
                    }
                    Some((Err(crate::editor::SaveError::Io(e)), _)) => self.toast(e),
                    None => {}
                }
            }
            Pending::Symbols { path } => {
                let mut items = Vec::new();
                match serde_json::from_value::<Option<lt::DocumentSymbolResponse>>(v) {
                    Ok(Some(lt::DocumentSymbolResponse::Nested(syms))) => {
                        fn walk(syms: &[lt::DocumentSymbol], depth: usize, out: &mut Vec<(String, String, lt::Range)>) {
                            for s in syms {
                                out.push((format!("{}{}", "  ".repeat(depth), s.name), symbol_kind(s.kind).to_string(), s.selection_range));
                                if let Some(c) = &s.children {
                                    walk(c, depth + 1, out);
                                }
                            }
                        }
                        let mut flat = Vec::new();
                        walk(&syms, 0, &mut flat);
                        let locs: Vec<(PathBuf, lt::Range)> = flat.iter().map(|(_, _, r)| (path.clone(), *r)).collect();
                        let mut li = self.loc_items(&locs, enc);
                        for (item, (name, kind, _)) in li.iter_mut().zip(flat) {
                            item.label = name;
                            item.detail = format!("{kind} · {}", item.line + 1);
                        }
                        items = li;
                    }
                    Ok(Some(lt::DocumentSymbolResponse::Flat(syms))) => {
                        items = self.symbol_items(&syms.into_iter().map(|s| (s.name, s.kind, s.location)).collect::<Vec<_>>(), enc);
                    }
                    _ => {}
                }
                if items.is_empty() {
                    self.toast(t("No symbols in this file.").into());
                } else {
                    self.show_locations(t("Symbols in File"), items);
                }
            }
            Pending::WorkspaceSymbols => {
                let syms: Vec<(String, lt::SymbolKind, lt::Location)> = match serde_json::from_value::<Option<lt::WorkspaceSymbolResponse>>(v) {
                    Ok(Some(lt::WorkspaceSymbolResponse::Flat(s))) => s.into_iter().map(|s| (s.name, s.kind, s.location)).collect(),
                    Ok(Some(lt::WorkspaceSymbolResponse::Nested(s))) => s
                        .into_iter()
                        .filter_map(|s| match s.location {
                            lt::OneOf::Left(l) => Some((s.name, s.kind, l)),
                            lt::OneOf::Right(_) => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let items = self.symbol_items(&syms, enc);
                if items.is_empty() {
                    self.toast(t("No symbols found.").into());
                } else {
                    self.show_locations(t("Symbols in Project"), items);
                }
            }
        }
    }

    fn symbol_items(&self, syms: &[(String, lt::SymbolKind, lt::Location)], enc: Encoding) -> Vec<LocItem> {
        let locs: Vec<(PathBuf, lt::Range)> = syms
            .iter()
            .filter_map(|(_, _, l)| Some((tc_lsp::pos::path(&l.uri)?, l.range)))
            .collect();
        let mut items = self.loc_items(&locs, enc);
        for (item, (name, kind, _)) in items.iter_mut().zip(syms) {
            let file = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            item.detail = format!("{} · {file}:{}", symbol_kind(*kind), item.line + 1);
            item.label = name.clone();
        }
        items
    }

    /// Locations as list items: the line's text as the label, the file and line as detail.
    fn loc_items(&self, locs: &[(PathBuf, lt::Range)], enc: Encoding) -> Vec<LocItem> {
        let mut texts: HashMap<PathBuf, String> = HashMap::new();
        let root = self.active_project().map(|p| p.path.clone()).unwrap_or_default();
        locs.iter()
            .map(|(path, range)| {
                let text = texts.entry(path.clone()).or_insert_with(|| {
                    open_text(&self.editors, path)
                        .map(|(t, _)| t.to_string())
                        .unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default())
                });
                let ix = LineIndex::new(text);
                let line = range.start.line as usize;
                let at = ix.char_index(text, range.start, enc);
                let col = at - ix.line_start(line);
                let shown = path.strip_prefix(&root).unwrap_or(path).display().to_string();
                LocItem {
                    path: path.clone(),
                    line,
                    col,
                    label: ix.line_text(text, line).trim().to_string(),
                    detail: format!("{shown}:{}", line + 1),
                    error: false,
                }
            })
            .collect()
    }

    /// A list of places in the command palette.
    fn show_locations(&mut self, title: &str, items: Vec<LocItem>) {
        self.open_palette(PaletteMode::Locations);
        if let Some(p) = self.palette.as_mut() {
            p.title = title.to_string();
            p.locations = items;
        }
    }

    fn show_problems(&mut self) {
        let mut items = Vec::new();
        let root = self.active_project().map(|p| p.path.clone()).unwrap_or_default();
        let mut paths: Vec<&PathBuf> = self.lsp.diagnostics.keys().filter(|p| p.starts_with(&root)).collect();
        paths.sort();
        for path in paths {
            let (enc, diags) = &self.lsp.diagnostics[path];
            let locs: Vec<(PathBuf, lt::Range)> = diags.iter().map(|d| (path.clone(), d.range)).collect();
            for (mut item, d) in self.loc_items(&locs, *enc).into_iter().zip(diags) {
                item.error = d.severity == Some(lt::DiagnosticSeverity::ERROR);
                item.label = d.message.lines().next().unwrap_or("").to_string();
                let sev = match d.severity {
                    Some(lt::DiagnosticSeverity::ERROR) => t("Error"),
                    Some(lt::DiagnosticSeverity::WARNING) => t("Warning"),
                    _ => t("Info"),
                };
                item.detail = format!("{sev} · {}", item.detail);
                items.push(item);
            }
        }
        if items.is_empty() {
            self.toast(t("No problems found.").into());
        } else {
            self.show_locations(t("Problems"), items);
        }
    }

    // ----- moving around ---------------------------------------------------------------------

    /// Opens `path` at an LSP position, remembering where the caret was (for Back).
    fn goto_range(&mut self, path: &Path, pos: lt::Position, enc: Encoding, remember: bool) {
        let text = open_text(&self.editors, path)
            .map(|(t, _)| t.to_string())
            .unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default());
        let ix = LineIndex::new(&text);
        let at = ix.char_index(&text, pos, enc);
        let line = pos.line as usize;
        let col = at - ix.line_start(line);
        if remember {
            self.remember_place();
        }
        // Through the action: the file is opened with its server too.
        self.actions.push(crate::actions::Action::GotoLocation(path.to_path_buf(), line, col));
    }

    /// Puts the caret's place on the Back list.
    pub fn remember_place(&mut self) {
        if let Some(place) = self.ide_target() {
            if self.lsp.ide.back.last() != Some(&place) {
                self.lsp.ide.back.push(place);
                if self.lsp.ide.back.len() > 50 {
                    self.lsp.ide.back.remove(0);
                }
            }
            self.lsp.ide.forward.clear();
        }
    }

    fn ide_history(&mut self, back: bool) {
        let from = if back { &mut self.lsp.ide.back } else { &mut self.lsp.ide.forward };
        let Some((path, at)) = from.pop() else { return };
        if let Some(here) = self.ide_target() {
            if back {
                self.lsp.ide.forward.push(here);
            } else {
                self.lsp.ide.back.push(here);
            }
        }
        let text = open_text(&self.editors, &path)
            .map(|(t, _)| t.to_string())
            .unwrap_or_else(|| std::fs::read_to_string(&path).unwrap_or_default());
        let ix = LineIndex::new(&text);
        let line = ix.line_of(at);
        self.actions.push(crate::actions::Action::GotoLocation(path, line, at - ix.line_start(line)));
    }

    // ----- typing help -----------------------------------------------------------------------

    /// Narrows a completion list to what was typed since it opened.
    pub fn refilter(&self, popup: &mut CompletionPopup) {
        if let (Some((text, _)), Some((_, caret))) = (open_text(&self.editors, &popup.path), self.ide_target()) {
            refilter_in(text, caret, popup);
        }
    }

    // ----- edits -----------------------------------------------------------------------------

    /// The revision of every file open in an editor, for a request that answers with edits.
    fn open_revs(&self) -> Revs {
        self.editors.values().flat_map(|e| e.files.iter()).map(|f| (f.path.clone(), f.rev)).collect()
    }

    /// Applies a workspace edit: open files in their buffers (saved too when they had no
    /// unsaved changes), others on disk. Returns how many files changed. One level of undo.
    ///
    /// Edits are positions in the text the server had. Nothing is applied when an open file
    /// was edited since the request (`revs`) or since the server last got its text (typing
    /// within the pause before changes are sent), or when the edit names another version.
    pub fn apply_workspace_edit(&mut self, edit: &lt::WorkspaceEdit, enc: Encoding, revs: Option<&Revs>) -> Result<usize, String> {
        let (files, _skipped) = tc_lsp::edit::files(edit);
        // Check everything first: all or nothing.
        let mut plan: Vec<(PathBuf, String, String, bool)> = Vec::new(); // path, before, after, open
        for fe in &files {
            let open = open_text(&self.editors, &fe.path).map(|(t, r)| (t.to_string(), r));
            let doc = self.lsp.key_of(&fe.path).and_then(|k| self.lsp.servers.get(&k)).and_then(|s| s.docs.get(&fe.path));
            let stale = match &open {
                Some((_, rev)) => {
                    revs.and_then(|r| r.get(&fe.path)).is_some_and(|r| r != rev) || doc.is_some_and(|d| d.sent != Some(*rev))
                }
                None => false,
            } || fe.version.is_some_and(|v| doc.is_some_and(|d| d.version != v));
            let name = || fe.path.file_name().map_or_else(|| fe.path.display().to_string(), |n| n.to_string_lossy().into_owned());
            if stale {
                return Err(tf("{} changed while the language server worked; nothing was applied. Try again.", &[&name()]));
            }
            let open = open.map(|(t, _)| t);
            let before = match &open {
                Some(t) => t.clone(),
                None => std::fs::read_to_string(&fe.path).map_err(|e| format!("{}: {e}", fe.path.display()))?,
            };
            let after = tc_lsp::edit::apply(&before, &fe.edits, enc)?;
            // An open file that can't be saved (too large, cut short, not UTF-8 or not
            // writable) would keep the edit in memory only.
            let read_only = || self.editors.values().flat_map(|e| e.files.iter()).any(|f| f.path == fe.path && (f.read_only || f.truncated));
            if after != before && open.is_some() && read_only() {
                return Err(tf("{} is open read-only; nothing was applied.", &[&name()]));
            }
            if after != before {
                plan.push((fe.path.clone(), before, after, open.is_some()));
            }
        }
        let mut undo = Vec::new();
        for (path, before, after, open) in plan {
            if open {
                if let Some(f) = self.editors.values_mut().flat_map(|e| e.files.iter_mut()).find(|f| f.path == path) {
                    let was_clean = !f.dirty();
                    f.text = after.clone();
                    f.rev += 1;
                    f.lines = crate::editor::count_lines(&f.text);
                    if was_clean {
                        let _ = crate::editor::save(f, false);
                    }
                }
            } else {
                write_atomic(&path, &after)?;
            }
            undo.push((path, before, after));
        }
        let n = undo.len();
        if n > 0 {
            self.lsp.ide.undo = Some(RefactorUndo { files: undo, at: Instant::now() });
        }
        Ok(n)
    }

    fn undo_refactor(&mut self) {
        let Some(u) = self.lsp.ide.undo.take() else {
            self.toast(t("Nothing to undo.").into());
            return;
        };
        let mut restored = 0;
        let mut skipped = 0;
        for (path, before, after) in u.files {
            if let Some(f) = self.editors.values_mut().flat_map(|e| e.files.iter_mut()).find(|f| f.path == path) {
                if f.text == after && !f.read_only {
                    let was_clean = !f.dirty();
                    f.text = before;
                    f.rev += 1;
                    f.lines = crate::editor::count_lines(&f.text);
                    if was_clean {
                        let _ = crate::editor::save(f, false);
                    }
                    restored += 1;
                } else {
                    skipped += 1;
                }
            } else if std::fs::read_to_string(&path).is_ok_and(|t| t == after) && write_atomic(&path, &before).is_ok() {
                restored += 1;
            } else {
                skipped += 1;
            }
        }
        self.toast(if skipped == 0 {
            tf("Undone in {} files.", &[&restored.to_string()])
        } else {
            tf("Undone in {} files; {} changed since and were left.", &[&restored.to_string(), &skipped.to_string()])
        });
    }

    fn run_code_action(&mut self, n: usize) {
        let Some(menu) = self.lsp.ide.actions.take() else { return };
        let Some(item) = menu.items.get(n).cloned() else { return };
        let enc = self.lsp.servers.get(&menu.key).map_or(Encoding::Utf16, |s| s.enc);
        let command = match item {
            lt::CodeActionOrCommand::Command(c) => Some(c),
            lt::CodeActionOrCommand::CodeAction(a) => {
                if let Some(edit) = &a.edit
                    && let Err(e) = self.apply_workspace_edit(edit, enc, Some(&menu.revs))
                {
                    self.toast(e);
                    return;
                }
                a.command
            }
        };
        if let Some(c) = command {
            self.lsp_request(
                &menu.key,
                "workspace/executeCommand",
                json!({"command": c.command, "arguments": c.arguments.unwrap_or_default()}),
                Pending::Execute,
            );
        }
    }
}

/// Narrows a completion list to what was typed between its start and `caret` in `text`.
pub fn refilter_in(text: &str, caret: usize, popup: &mut CompletionPopup) {
    let typed: String = if caret >= popup.start {
        text.chars().skip(popup.start).take(caret - popup.start).collect()
    } else {
        String::new()
    };
    let mut scored: Vec<(i32, usize)> = popup
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let key = it.filter_text.as_deref().unwrap_or(&it.label);
            tc_lsp::assist::score(key, &typed).map(|s| (s + if it.preselect == Some(true) { 5 } else { 0 }, i))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            let sa = popup.items[a.1].sort_text.as_deref().unwrap_or(&popup.items[a.1].label);
            let sb = popup.items[b.1].sort_text.as_deref().unwrap_or(&popup.items[b.1].label);
            sa.cmp(sb)
        })
    });
    scored.truncate(200);
    popup.shown = scored.into_iter().map(|(_, i)| i).collect();
    popup.selected = popup.selected.min(popup.shown.len().saturating_sub(1));
}

/// `text` with a completion item accepted at `caret`, and the new caret (char index). The
/// item's own edits elsewhere (an import it needs) are applied too.
pub fn accept_completion(text: &str, popup: &CompletionPopup, item: &lt::CompletionItem, caret: usize) -> Option<(String, usize)> {
    let ix = LineIndex::new(text);
    let (mut from, mut to) = (popup.start.min(caret), caret);
    let mut insert = item.insert_text.clone().unwrap_or_else(|| item.label.clone());
    if let Some(edit) = &item.text_edit {
        // Of the two ranges, the one that inserts (as VS Code does by default): the rest of
        // the word after the caret stays.
        let (range, new) = match edit {
            lt::CompletionTextEdit::Edit(e) => (e.range, e.new_text.clone()),
            lt::CompletionTextEdit::InsertAndReplace(e) => (e.insert, e.new_text.clone()),
        };
        let (a, _) = ix.chars(text, &range, popup.enc);
        from = a.min(caret);
        to = caret;
        insert = new;
    }
    let (body, cursor) = if item.insert_text_format == Some(lt::InsertTextFormat::SNIPPET) {
        tc_lsp::assist::snippet_text(&insert)
    } else {
        (insert, None)
    };
    let caret_in = cursor.unwrap_or_else(|| body.chars().count());
    let main = lt::TextEdit::new(ix.range(text, from, to, popup.enc), body);
    let extra = item.additional_text_edits.clone().unwrap_or_default();
    // Edits before the completion move it.
    let shift: isize = extra
        .iter()
        .map(|e| (ix.chars(text, &e.range, popup.enc), e.new_text.chars().count()))
        .filter(|((a, _), _)| *a < from)
        .map(|((a, b), n)| n as isize - (b - a) as isize)
        .sum();
    let mut all = extra;
    all.push(main);
    let new = tc_lsp::edit::apply(text, &all, popup.enc).ok()?;
    Some((new, (from as isize + shift).max(0) as usize + caret_in))
}

/// The location(s) of a definition answer.
fn goto_locations(v: Value) -> Vec<(PathBuf, lt::Range)> {
    match serde_json::from_value::<Option<lt::GotoDefinitionResponse>>(v) {
        Ok(Some(lt::GotoDefinitionResponse::Scalar(l))) => tc_lsp::pos::path(&l.uri).map(|p| vec![(p, l.range)]).unwrap_or_default(),
        Ok(Some(lt::GotoDefinitionResponse::Array(ls))) => ls
            .into_iter()
            .filter_map(|l| Some((tc_lsp::pos::path(&l.uri)?, l.range)))
            .collect(),
        Ok(Some(lt::GotoDefinitionResponse::Link(ls))) => ls
            .into_iter()
            .filter_map(|l| Some((tc_lsp::pos::path(&l.target_uri)?, l.target_selection_range)))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Where the identifier ending at `at` starts.
fn word_start(editors: &HashMap<u64, crate::editor::Editor>, path: &Path, at: usize) -> usize {
    let Some((text, _)) = open_text(editors, path) else { return at };
    let before: Vec<char> = text.chars().take(at).collect();
    let n = before.iter().rev().take_while(|c| is_ident(**c)).count();
    at - n
}

/// The identifier around `at`.
fn word_at(editors: &HashMap<u64, crate::editor::Editor>, path: &Path, at: usize) -> Option<String> {
    let (text, _) = open_text(editors, path)?;
    word_range(text, at).map(|(a, b)| text.chars().skip(a).take(b - a).collect())
}

/// The char range of the identifier around `at` (none outside one).
pub fn word_range(text: &str, at: usize) -> Option<(usize, usize)> {
    // Only the line around `at` is looked at (a line break ends an identifier), and the text
    // is not copied.
    let (at, byte) = match text.char_indices().nth(at) {
        Some((byte, _)) => (at, byte),
        None => (text.chars().count(), text.len()),
    };
    let a = at - text[..byte].chars().rev().take_while(|c| is_ident(*c)).count();
    let b = at + text[byte..].chars().take_while(|c| is_ident(*c)).count();
    (a < b).then_some((a, b))
}

fn symbol_kind(k: lt::SymbolKind) -> &'static str {
    match k {
        lt::SymbolKind::FUNCTION | lt::SymbolKind::METHOD => "fn",
        lt::SymbolKind::STRUCT | lt::SymbolKind::CLASS => "type",
        lt::SymbolKind::ENUM => "enum",
        lt::SymbolKind::INTERFACE => "interface",
        lt::SymbolKind::FIELD | lt::SymbolKind::PROPERTY => "field",
        lt::SymbolKind::VARIABLE => "var",
        lt::SymbolKind::CONSTANT => "const",
        lt::SymbolKind::MODULE | lt::SymbolKind::NAMESPACE | lt::SymbolKind::PACKAGE => "module",
        lt::SymbolKind::CONSTRUCTOR => "constructor",
        lt::SymbolKind::ENUM_MEMBER => "variant",
        lt::SymbolKind::TYPE_PARAMETER => "type param",
        _ => "symbol",
    }
}

/// Writes a file not open in the editor (as a save does: whole or not at all, keeping its
/// permissions and symbolic links).
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    crate::editor::write_atomic(path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Formatting options that keep the file's indentation: tabs when most indented lines start
/// with one, else spaces, as many as a line most often goes in by (4 when nothing tells).
fn format_options(text: &str) -> Value {
    let (mut tabs, mut spaces) = (0, 0);
    let mut steps = [0usize; 9];
    let mut prev = 0;
    for line in text.lines() {
        let body = line.trim_start_matches([' ', '\t']);
        if body.is_empty() {
            continue;
        }
        let indent = line.len() - body.len();
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        if indent > 0 {
            spaces += 1;
        }
        // The ` * ` lines of a block comment are not a level in.
        if body.starts_with('*') {
            continue;
        }
        if indent > prev && indent - prev < steps.len() {
            steps[indent - prev] += 1;
        }
        prev = indent;
    }
    let spaces = spaces >= tabs;
    let size = (1..steps.len())
        .filter(|n| spaces && steps[*n] > 0)
        .max_by_key(|n| (steps[*n], std::cmp::Reverse(*n)))
        .unwrap_or(4);
    json!({"tabSize": size, "insertSpaces": spaces, "trimTrailingWhitespace": true, "insertFinalNewline": true})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_found_around_a_char() {
        let text = "let foo_bar = baz(1);\nnext";
        assert_eq!(word_range(text, 5), Some((4, 11)));
        assert_eq!(word_range(text, 11), Some((4, 11)));
        assert_eq!(word_range(text, 12), None);
        assert_eq!(word_range(text, 22), Some((22, 26)));
        assert_eq!(word_range(text, 99), Some((22, 26)));
        assert_eq!(word_range("ab\ncd", 2), Some((0, 2)));
        assert_eq!(word_range("ab\ncd", 3), Some((3, 5)));
        assert_eq!(word_range("é = ñandú", 6), Some((4, 9)));
    }

    #[test]
    fn formatting_keeps_the_files_indentation() {
        let opts = |text: &str| {
            let o = format_options(text);
            (o["tabSize"].as_u64().unwrap(), o["insertSpaces"].as_bool().unwrap())
        };
        let ts = "/**\n * Doc.\n */\nexport function f() {\n  if (x) {\n    return {\n      a: 1,\n    };\n  }\n}\n";
        assert_eq!(opts(ts), (2, true));
        let rs = "fn main() {\n    let x = 1;\n    if x {\n        y();\n    }\n}\n";
        assert_eq!(opts(rs), (4, true));
        let go = "func f() {\n\tif x {\n\t\ty()\n\t}\n}\n";
        assert_eq!(opts(go), (4, false));
        assert_eq!(opts(""), (4, true));
        assert_eq!(opts("a\nb\n"), (4, true));
    }

    #[test]
    fn completion_inserts_with_the_insert_range() {
        let enc = Encoding::Utf16;
        let at = |c| lt::Position::new(0, c);
        let item = lt::CompletionItem {
            label: "foobar".into(),
            text_edit: Some(lt::CompletionTextEdit::InsertAndReplace(lt::InsertReplaceEdit {
                new_text: "foobar".into(),
                insert: lt::Range::new(at(4), at(6)),
                replace: lt::Range::new(at(4), at(9)),
            })),
            ..Default::default()
        };
        let popup = CompletionPopup { path: PathBuf::new(), start: 4, items: Vec::new(), shown: Vec::new(), selected: 0, enc, incomplete: false };
        // `let fo|bar;`: the word after the caret stays, the caret goes after what was inserted.
        let (text, caret) = accept_completion("let fobar;", &popup, &item, 6).unwrap();
        assert_eq!((text.as_str(), caret), ("let foobarbar;", 10));
    }
}
