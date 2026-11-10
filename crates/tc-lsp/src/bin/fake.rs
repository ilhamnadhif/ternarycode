//! A language server that answers from plain text, for TernaryCode's end-to-end checks
//! (`TERNARYCODE_LSP_OVERRIDE=<path of this binary>`). It knows `.rs`-style files: a function is
//! a line with `fn <name>`; words are ASCII identifiers; columns count chars.
//!
//! - definition: the `fn <word>` line; references and rename: every whole-word occurrence;
//! - completion: three items (one a snippet); hover and signature help: fixed texts;
//! - diagnostics: an error on each `BUG`, a warning on each `TODO`;
//! - a code action inserting `// TODO` above the line; formatting trims trailing spaces;
//! - document and workspace symbols: the `fn` lines;
//! - `TC_FAKE_LSP_DELAY_MS`: rename answers after this long.

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn main() {
    let mut input = std::io::stdin().lock();
    let mut docs: BTreeMap<String, String> = BTreeMap::new();
    let mut root = PathBuf::from(".");
    loop {
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; len];
        if input.read_exact(&mut body).is_err() {
            return;
        }
        let Ok(msg) = serde_json::from_slice::<Value>(&body) else { continue };
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id = msg.get("id").cloned();
        let p = &msg["params"];
        let reply = match method.as_str() {
            "initialize" => {
                if let Some(r) = p["rootUri"].as_str().and_then(file_path) {
                    root = r;
                }
                Some(json!({
                    "capabilities": {
                        "positionEncoding": "utf-16",
                        "textDocumentSync": 1,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "renameProvider": {"prepareProvider": true},
                        "completionProvider": {"triggerCharacters": ["."]},
                        "hoverProvider": true,
                        "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
                        "codeActionProvider": true,
                        "documentFormattingProvider": true,
                        "documentSymbolProvider": true,
                        "workspaceSymbolProvider": true
                    },
                    "serverInfo": {"name": "tc-fake-lsp"}
                }))
            }
            "shutdown" => Some(Value::Null),
            "exit" => std::process::exit(0),
            "textDocument/didOpen" => {
                let uri = p["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                docs.insert(uri.clone(), p["textDocument"]["text"].as_str().unwrap_or("").to_string());
                publish(&uri, &docs[&uri]);
                None
            }
            "textDocument/didChange" => {
                let uri = p["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                if let Some(t) = p["contentChanges"][0]["text"].as_str() {
                    docs.insert(uri.clone(), t.to_string());
                    publish(&uri, t);
                }
                None
            }
            "textDocument/didClose" => {
                docs.remove(p["textDocument"]["uri"].as_str().unwrap_or(""));
                None
            }
            "textDocument/definition" => Some(word_here(&docs, p).map_or(Value::Null, |w| {
                let defs: Vec<Value> = occurrences(&root, &docs, &w)
                    .into_iter()
                    .filter(|(_, line, _)| line.contains(&format!("fn {w}")))
                    .map(|(uri, _, r)| json!({"uri": uri, "range": r}))
                    .collect();
                json!(defs)
            })),
            "textDocument/references" => Some(word_here(&docs, p).map_or(Value::Null, |w| {
                json!(occurrences(&root, &docs, &w)
                    .into_iter()
                    .map(|(uri, _, r)| json!({"uri": uri, "range": r}))
                    .collect::<Vec<_>>())
            })),
            "textDocument/prepareRename" => Some(word_range(&docs, p).map_or(Value::Null, |(_, r)| r)),
            "textDocument/rename" => Some(word_here(&docs, p).map_or(Value::Null, |w| {
                // A slow server (`TC_FAKE_LSP_DELAY_MS`): the answer is for the text as it was.
                if let Some(ms) = std::env::var("TC_FAKE_LSP_DELAY_MS").ok().and_then(|v| v.parse().ok()) {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
                let name = p["newName"].as_str().unwrap_or("");
                let mut changes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                for (uri, _, r) in occurrences(&root, &docs, &w) {
                    changes.entry(uri).or_default().push(json!({"range": r, "newText": name}));
                }
                json!({"changes": changes})
            })),
            "textDocument/completion" => Some(json!({"isIncomplete": false, "items": [
                {"label": "alpha_item", "kind": 6, "detail": "i32"},
                {"label": "beta_item", "kind": 6, "detail": "String"},
                {"label": "demo_fn", "kind": 3, "detail": "fn demo_fn(a: i32)", "insertTextFormat": 2, "insertText": "demo_fn(${1:a})$0"}
            ]})),
            "textDocument/hover" => Some(word_range(&docs, p).map_or(Value::Null, |(w, r)| {
                json!({"contents": {"kind": "markdown", "value": format!("```rust\nfn {w}()\n```\nFake **docs** for `{w}`.")}, "range": r})
            })),
            "textDocument/signatureHelp" => {
                let line = line_at(&docs, p).unwrap_or_default();
                let col = p["position"]["character"].as_u64().unwrap_or(0) as usize;
                let before: String = line.chars().take(col).collect();
                let args = before.rsplit_once('(').map_or("", |(_, a)| a);
                Some(json!({"signatures": [{"label": "fn demo(a: i32, b: i32)", "parameters": [{"label": "a: i32"}, {"label": "b: i32"}]}],
                    "activeSignature": 0, "activeParameter": args.matches(',').count()}))
            }
            "textDocument/codeAction" => {
                let uri = p["textDocument"]["uri"].as_str().unwrap_or("");
                let line = p["range"]["start"]["line"].as_u64().unwrap_or(0);
                Some(json!([{"title": "Insert TODO above", "kind": "refactor",
                    "edit": {"changes": {uri: [{"range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 0}}, "newText": "// TODO\n"}]}}}]))
            }
            "textDocument/formatting" => {
                let uri = p["textDocument"]["uri"].as_str().unwrap_or("");
                let text = docs.get(uri).cloned().unwrap_or_default();
                let edits: Vec<Value> = text
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.ends_with(' ') || l.ends_with('\t'))
                    .map(|(i, l)| {
                        let keep = l.trim_end().chars().count();
                        json!({"range": {"start": {"line": i, "character": keep}, "end": {"line": i, "character": l.chars().count()}}, "newText": ""})
                    })
                    .collect();
                Some(json!(edits))
            }
            "textDocument/documentSymbol" => {
                let uri = p["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                let text = docs.get(&uri).cloned().unwrap_or_default();
                Some(json!(symbols(&uri, &text, "")))
            }
            "workspace/symbol" => {
                let q = p["query"].as_str().unwrap_or("");
                let mut out = Vec::new();
                for (uri, text) in all_files(&root, &docs) {
                    out.extend(symbols(&uri, &text, q));
                }
                Some(json!(out))
            }
            _ => id.as_ref().map(|_| Value::Null),
        };
        if let (Some(id), Some(result)) = (id, reply) {
            send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }
    }
}

fn send(msg: &Value) {
    let body = serde_json::to_vec(msg).unwrap();
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n", body.len());
    let _ = out.write_all(&body);
    let _ = out.flush();
}

fn publish(uri: &str, text: &str) {
    let mut diags = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (word, severity, message) in [("BUG", 1, "fake error: BUG here"), ("TODO", 2, "fake warning: TODO left")] {
            if let Some(b) = line.find(word) {
                let c = line[..b].chars().count();
                diags.push(json!({"range": {"start": {"line": i, "character": c}, "end": {"line": i, "character": c + word.len()}},
                    "severity": severity, "message": message, "source": "fake"}));
            }
        }
    }
    send(&json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": diags}}));
}

// As the client writes and reads them, so that a file's URI is the same both ways.
fn file_path(uri: &str) -> Option<PathBuf> {
    tc_lsp::pos::path(&uri.parse().ok()?)
}

fn file_uri(p: &Path) -> String {
    tc_lsp::pos::uri(p).as_str().to_string()
}

fn line_at(docs: &BTreeMap<String, String>, p: &Value) -> Option<String> {
    let text = docs.get(p["textDocument"]["uri"].as_str()?)?;
    let line = p["position"]["line"].as_u64()? as usize;
    text.lines().nth(line).map(str::to_string)
}

/// The identifier at the request's position and its range.
fn word_range(docs: &BTreeMap<String, String>, p: &Value) -> Option<(String, Value)> {
    let line = line_at(docs, p)?;
    let l = p["position"]["line"].as_u64()?;
    let col = p["position"]["character"].as_u64()? as usize;
    let chars: Vec<char> = line.chars().collect();
    let is_id = |c: &char| c.is_ascii_alphanumeric() || *c == '_';
    let mut a = col.min(chars.len());
    while a > 0 && is_id(&chars[a - 1]) {
        a -= 1;
    }
    let mut b = col.min(chars.len());
    while b < chars.len() && is_id(&chars[b]) {
        b += 1;
    }
    (a < b).then(|| {
        let w: String = chars[a..b].iter().collect();
        (w, json!({"start": {"line": l, "character": a}, "end": {"line": l, "character": b}}))
    })
}

fn word_here(docs: &BTreeMap<String, String>, p: &Value) -> Option<String> {
    word_range(docs, p).map(|(w, _)| w)
}

/// Every file under the root (open documents as the editor has them).
fn all_files(root: &Path, docs: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|x| x == "rs")
                && let Ok(t) = std::fs::read_to_string(&path)
            {
                out.insert(file_uri(&path), t);
            }
        }
    }
    for (u, t) in docs {
        out.insert(u.clone(), t.clone());
    }
    out.into_iter().collect()
}

/// Whole-word occurrences of `word`: (uri, the line, range).
fn occurrences(root: &Path, docs: &BTreeMap<String, String>, word: &str) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for (uri, text) in all_files(root, docs) {
        for (i, line) in text.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let w: Vec<char> = word.chars().collect();
            let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
            let mut c = 0;
            while c + w.len() <= chars.len() {
                if chars[c..c + w.len()] == w[..]
                    && (c == 0 || !is_id(chars[c - 1]))
                    && (c + w.len() == chars.len() || !is_id(chars[c + w.len()]))
                {
                    out.push((uri.clone(), line.to_string(), json!({"start": {"line": i, "character": c}, "end": {"line": i, "character": c + w.len()}})));
                    c += w.len();
                } else {
                    c += 1;
                }
            }
        }
    }
    out
}

fn symbols(uri: &str, text: &str, query: &str) -> Vec<Value> {
    text.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let at = l.find("fn ")?;
            let name: String = l[at + 3..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            (!name.is_empty() && name.contains(query)).then(|| {
                let c = l[..at + 3].chars().count();
                json!({"name": name, "kind": 12, "location": {"uri": uri, "range": {"start": {"line": i, "character": c}, "end": {"line": i, "character": c + name.len()}}}})
            })
        })
        .collect()
}
