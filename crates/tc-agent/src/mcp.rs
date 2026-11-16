//! `ternarycode-hook mcp <agent>`: the MCP server (JSON-RPC over stdio, one message per line) an
//! agent in a TernaryCode pane starts, giving it tools for its project's browser pane. Each tool
//! call goes to the app over the pane's hook socket and waits for the answer there (see
//! `server::OnCall`); the page itself is the app's.
//!
//! Outside a TernaryCode pane (no `TERNARYCODE_*` variables), or where browser panes don't run
//! (only macOS for now), it lists no tools.

use std::io::{BufRead, Write};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{ENV_PANE, ENV_SOCK, ENV_TOKEN};

/// Protocol versions this server speaks (the newest first): it answers in the client's when it
/// is one of these, else in the newest.
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What the agent is told about these tools (Claude Code puts it in its system prompt): this is
/// the browser the user means while it works in TernaryCode, before other browser tools it may
/// have (a browser extension's, Playwright) and before `open <url>`.
const INSTRUCTIONS: &str = "You run in a terminal pane of TernaryCode, which has a browser built \
in, beside this terminal: it is the user's browser in this session, and the user sees every page \
and every click. Whenever the user asks to open, show, check or search something on a website or \
in a browser (\"open YouTube\", \"check it in the browser\", a URL, a dev server), use these \
tools, not `open <url>` in a shell and not other browser tools (Chrome, Zen, Safari, Playwright), \
unless the user names that other browser. Start with browser_navigate: this machine's addresses \
open at once; another site waits the first time for the user's OK, asked in TernaryCode. Then \
act on the refs of the snapshot each tool returns. A pop-up a page opens (a sign-in such as \
Continue with Google) shows in a window of its own; the tools act in it until it closes. Never \
type the user's passwords or one-time codes: ask the user to type them there.";

/// The tools, as `tools/list` gives them.
pub fn tools() -> Value {
    let r#ref = json!({"type": "string", "description": "The element's ref from the last snapshot (e.g. e12)."});
    let element = json!({"type": "string", "description": "What the element is, in a few words (shown to the user, e.g. “Add to cart” button)."});
    json!([
        {
            "name": "browser_navigate",
            "description": "Open a URL in TernaryCode's built-in browser, beside the terminal: the browser the user means in this session (any website or page they ask for, unless they name another browser). Local addresses (localhost, 127.0.0.1, a port like :5173) open at once; another site asks the user the first time. Waits for the page to load; returns its title and a snapshot.",
            "inputSchema": {"type": "object", "properties": {"url": {"type": "string", "description": "The address (a bare port or host:port means this machine)."}}, "required": ["url"]},
        },
        {
            "name": "browser_navigate_back",
            "description": "Go back to the previous page; returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "browser_snapshot",
            "description": "A text outline of the page: headings, text, links, buttons and fields, each element you can act on with a ref (e.g. [ref=e12]). Better than a screenshot for acting on the page.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "browser_click",
            "description": "Click an element of the page with the mouse, as the user would (a real click: it opens file choosers and pop-ups and counts as the user's action); returns a snapshot after it (and after the page it leads to has loaded).",
            "inputSchema": {"type": "object", "properties": {"ref": r#ref, "element": element, "doubleClick": {"type": "boolean"}}, "required": ["ref", "element"]},
        },
        {
            "name": "browser_mouse_click_xy",
            "description": "Click at a point of the page, in CSS pixels from its top left (the screenshot tool says the page's size). For what a snapshot has no ref for: inside a frame of another site (a card form, a captcha), a canvas, a map. Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "element": element}, "required": ["x", "y"]},
        },
        {
            "name": "browser_keyboard_type",
            "description": "Type text with the keyboard into what has the focus in the page (after clicking a field, also inside another site's frame), a key at a time; submit presses Enter after it. Never type the user's passwords or one-time codes: ask the user. Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "submit": {"type": "boolean"}}, "required": ["text"]},
        },
        {
            "name": "browser_hover",
            "description": "Move the mouse onto an element (menus and tooltips that show on hover). Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"ref": r#ref, "element": element}, "required": ["ref", "element"]},
        },
        {
            "name": "browser_drag",
            "description": "Drag one element onto another with the mouse (sortable lists, sliders, drag and drop). Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"startRef": r#ref, "startElement": element, "endRef": r#ref, "endElement": element}, "required": ["startRef", "startElement", "endRef", "endElement"]},
        },
        {
            "name": "browser_file_upload",
            "description": "Choose files for a file field: clicks ref (the file field, or the button that opens its chooser) and answers the chooser with paths. Only files in the project's folder (relative paths start there) or the Downloads folder. Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"ref": r#ref, "element": element, "paths": {"type": "array", "items": {"type": "string"}}}, "required": ["ref", "element", "paths"]},
        },
        {
            "name": "browser_type",
            "description": "Put text into a field (replacing what it holds); submit presses Enter after it. Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"ref": r#ref, "element": element, "text": {"type": "string"}, "submit": {"type": "boolean", "description": "Press Enter after typing."}}, "required": ["ref", "element", "text"]},
        },
        {
            "name": "browser_select_option",
            "description": "Choose options of a select element (by value or by label). Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"ref": r#ref, "element": element, "values": {"type": "array", "items": {"type": "string"}}}, "required": ["ref", "element", "values"]},
        },
        {
            "name": "browser_press_key",
            "description": "Press a key in the page, where the focus is, as the keyboard would: Enter, Escape, Tab, Backspace, ArrowDown, a letter… Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]},
        },
        {
            "name": "browser_wait_for",
            "description": "Wait until a text shows on the page, until a text is gone, or for some seconds (at most 60).",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "textGone": {"type": "string"}, "time": {"type": "number", "description": "Seconds."}}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "browser_scroll",
            "description": "Scroll the page (or the scrollable part holding ref) up or down by screenfuls, or bring the element ref into view (no direction). Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"ref": r#ref, "element": element, "direction": {"type": "string", "enum": ["up", "down", "left", "right"]}, "pages": {"type": "number", "description": "How many screenfuls (default 1)."}}},
        },
        {
            "name": "browser_take_screenshot",
            "description": "A picture of the page as it shows in the pane (PNG). Use it to check how something looks; use the snapshot to act.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "browser_console_messages",
            "description": "What the page logged to its console, with its errors, since it loaded.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "browser_handle_dialog",
            "description": "Answer the page's dialog (alert, confirm or prompt), which a step's result reports: the page waits for it. accept true is OK, false is Cancel; promptText fills a prompt. Returns a snapshot.",
            "inputSchema": {"type": "object", "properties": {"accept": {"type": "boolean"}, "promptText": {"type": "string"}}, "required": ["accept"]},
        },
        {
            "name": "browser_evaluate",
            "description": "Run a JavaScript function in the page and return its result as JSON, e.g. \"() => document.title\" (it may be async). Off unless the user allows it in TernaryCode's settings.",
            "inputSchema": {"type": "object", "properties": {"function": {"type": "string"}}, "required": ["function"]},
        },
    ])
}

/// The answer to one JSON-RPC message (`None` for a notification or a response).
fn answer(msg: &Value, env: &Option<Pane>, call: &dyn Fn(&Pane, &str, &Value) -> Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg["method"].as_str()?;
    let result = match method {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "ternarycode", "title": "TernaryCode browser", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            let available = env.is_some();
            json!({"tools": if available { tools() } else { json!([]) }})
        }
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
            let known = tools().as_array().is_some_and(|t| t.iter().any(|x| x["name"] == name));
            match env {
                Some(pane) if known => tool_result(&call(pane, name, &args)),
                _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": format!("Unknown tool: {name}")}})),
            }
        }
        _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {method}")}})),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// The app's answer (`{"text"}`, `{"image"}` in base64 PNG, or `{"error"}`) as a tool result.
fn tool_result(reply: &Value) -> Value {
    if let Some(e) = reply["error"].as_str() {
        return json!({"content": [{"type": "text", "text": e}], "isError": true});
    }
    let mut content = Vec::new();
    if let Some(t) = reply["text"].as_str() {
        content.push(json!({"type": "text", "text": t}));
    }
    if let Some(png) = reply["image"].as_str() {
        content.push(json!({"type": "image", "data": png, "mimeType": "image/png"}));
    }
    json!({"content": content, "isError": false})
}

/// The pane this server's agent runs in.
#[derive(Clone)]
pub(crate) struct Pane {
    pub(crate) sock: String,
    pub(crate) pane: u64,
    pub(crate) token: String,
    pub(crate) agent: Option<String>,
}

/// Sends a tool call to the app and waits for its answer.
#[cfg(unix)]
pub(crate) fn call_app(p: &Pane, name: &str, args: &Value) -> Value {
    use std::io::Read;
    let msg = crate::protocol::CallMsg { pane: p.pane, token: p.token.clone(), agent: p.agent.clone(), call: name.to_string(), args: args.clone() };
    let result = (|| -> std::io::Result<Value> {
        let mut stream = std::os::unix::net::UnixStream::connect(&p.sock)?;
        let mut line = serde_json::to_vec(&msg)?;
        line.push(b'\n');
        stream.write_all(&line)?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer)?;
        Ok(serde_json::from_slice(&answer)?)
    })();
    result.unwrap_or_else(|_| json!({"error": "TernaryCode did not answer: is it still running?"}))
}

/// Sends a tool call to the app over its named pipe and waits for its answer (a line).
#[cfg(windows)]
pub(crate) fn call_app(p: &Pane, name: &str, args: &Value) -> Value {
    use std::io::BufRead;
    const ERROR_PIPE_BUSY: i32 = 231;
    let msg = crate::protocol::CallMsg { pane: p.pane, token: p.token.clone(), agent: p.agent.clone(), call: name.to_string(), args: args.clone() };
    let result = (|| -> std::io::Result<Value> {
        let start = std::time::Instant::now();
        let mut pipe = loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&p.sock) {
                Ok(pipe) => break pipe,
                // Every instance busy for a moment (the app makes the next as one connects).
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && start.elapsed() < std::time::Duration::from_secs(2) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        };
        let mut line = serde_json::to_vec(&msg)?;
        line.push(b'\n');
        pipe.write_all(&line)?;
        let mut answer = String::new();
        std::io::BufReader::new(pipe).read_line(&mut answer)?;
        Ok(serde_json::from_str(&answer)?)
    })();
    result.unwrap_or_else(|_| json!({"error": "TernaryCode did not answer: is it still running?"}))
}

/// The TernaryCode pane this process runs in (from its environment), if any.
pub(crate) fn pane_from_env(agent: Option<String>) -> Option<Pane> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    match (var(ENV_SOCK), var(ENV_PANE).and_then(|p| p.parse().ok()), var(ENV_TOKEN)) {
        (Some(sock), Some(pane), Some(token)) => Some(Pane { sock, pane, token, agent }),
        _ => None,
    }
}

/// Entry point of `ternarycode-hook mcp [agent]`; returns the exit code.
pub fn run(args: &[String]) -> i32 {
    let env = pane_from_env(args.first().cloned());
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let write = |out: &Mutex<std::io::Stdout>, v: &Value| {
        let mut o = out.lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    };
    let mut calls = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            if !line.trim().is_empty() {
                write(&out, &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}));
            }
            continue;
        };
        // A tool call can wait long (a page loading, the user asked): each runs on its own
        // thread, so pings and other calls are answered meanwhile.
        if msg["method"] == "tools/call" {
            let (out, env) = (out.clone(), env.clone());
            calls.push(std::thread::spawn(move || {
                if let Some(a) = answer(&msg, &env, &call_app) {
                    write(&out, &a);
                }
            }));
            calls.retain(|c| !c.is_finished());
        } else if let Some(a) = answer(&msg, &env, &call_app) {
            write(&out, &a);
        }
    }
    // The input ended: the calls still running are answered before it exits.
    for c in calls {
        let _ = c.join();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Option<Pane> {
        Some(Pane { sock: "/tmp/x".into(), pane: 3, token: "t".into(), agent: Some("claude".into()) })
    }

    #[test]
    fn speaks_mcp() {
        let none = |_: &Pane, _: &str, _: &Value| json!({});
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}});
        let a = answer(&init, &pane(), &none).unwrap();
        assert_eq!(a["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(a["result"]["serverInfo"]["name"], "ternarycode");
        // The agent is told this is the browser the user means here, unless they name another.
        let told = a["result"]["instructions"].as_str().unwrap_or("");
        assert!(told.contains("browser built in") && told.contains("unless the user names that other browser"), "{told}");
        assert!(told.contains("pop-up") && told.contains("Never type the user's passwords"), "{told}");
        let navigate = tools().as_array().and_then(|t| t.iter().find(|x| x["name"] == "browser_navigate").cloned()).unwrap_or_default();
        assert!(navigate["description"].as_str().unwrap_or("").contains("the browser the user means in this session"));
        let newer = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2099-01-01"}});
        assert_eq!(answer(&newer, &pane(), &none).unwrap()["result"]["protocolVersion"], VERSIONS[0]);
        // Notifications get no answer; unknown methods an error.
        assert!(answer(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), &pane(), &none).is_none());
        let bad = answer(&json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}), &pane(), &none).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
        // Outside a pane, no tools.
        let list = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"});
        assert_eq!(answer(&list, &None, &none).unwrap()["result"]["tools"], json!([]));
        let n = answer(&list, &pane(), &none).unwrap()["result"]["tools"].as_array().map(Vec::len).unwrap_or(0);
        assert_eq!(n, 18);
    }

    #[test]
    fn calls_go_to_the_app_and_come_back_as_content() {
        let app = |p: &Pane, name: &str, args: &Value| -> Value {
            assert_eq!((p.pane, name, args["url"].as_str()), (3, "browser_navigate", Some("localhost:3000")));
            json!({"text": "Page: Home", "image": "iVBO"})
        };
        let call = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "browser_navigate", "arguments": {"url": "localhost:3000"}}});
        let r = answer(&call, &pane(), &app).unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "Page: Home");
        assert_eq!(r["result"]["content"][1]["mimeType"], "image/png");
        assert_eq!(r["result"]["isError"], false);
        let failing = |_: &Pane, _: &str, _: &Value| json!({"error": "No page"});
        let r = answer(&call, &pane(), &failing).unwrap();
        assert_eq!(r["result"]["isError"], true);
        let unknown = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "rm_rf"}});
        assert_eq!(answer(&unknown, &pane(), &app).unwrap()["error"]["code"], -32602);
    }
}
