//! Gate A: are hooks passed with `claude --settings` merged with hooks from other settings
//! files, or do they replace them?
//!
//! A temporary project gets a SessionStart hook in `.claude/settings.json` (standing in for the
//! user's own hooks); `--settings` adds another. Both append to one log. Claude Code is started,
//! the folder is trusted (needed for project settings), the session starts, and `/exit` leaves
//! it. No prompt is sent, so no model request is made.
//!
//! `cargo run -p tc-term --release --example claude_settings_check`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use parking_lot::Mutex;
use tc_term::core::{Feeder, GridSize, new_terminal, term_config};
use tc_term::pty::{self, LocalPty, SessionEvent, SpawnOptions};
use tc_term::scheme::SCHEMES;

fn screen(feeder: &Arc<Mutex<Feeder>>) -> String {
    let f = feeder.lock();
    let term = f.shared.term.lock();
    (0..term.screen_lines() as i32)
        .map(|l| {
            let row = &term.grid()[Line(l)];
            (0..term.columns())
                .map(|c| row[Column(c)].c)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn wait_for(feeder: &Arc<Mutex<Feeder>>, any: &[&str], limit: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < limit {
        let s = screen(feeder);
        if any.iter().any(|a| s.contains(a)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn hook(cmd: &str) -> serde_like::Value {
    serde_like::hooks_json(cmd)
}

mod serde_like {
    pub struct Value(pub String);
    pub fn hooks_json(cmd: &str) -> Value {
        let cmd = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        Value(format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"{cmd}"}}]}}]}}}}"#
        ))
    }
}

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let log = root.join("hooks.log");
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    let project = hook(&format!("echo project >> '{}'", log.display()));
    std::fs::write(root.join(".claude/settings.json"), &project.0).unwrap();
    let flag = hook(&format!("echo flag >> '{}'", log.display()));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let size = GridSize { cols: 120, rows: 40 };
    let (shared, rx) = new_terminal(
        term_config(5000, Default::default(), false),
        size,
        (&SCHEMES[0]).into(),
    );
    let feeder = Arc::new(Mutex::new(Feeder::new(shared, rx)));
    let pty = LocalPty::spawn(&SpawnOptions {
        program: Some(("claude".into(), vec!["--settings".into(), flag.0.clone()])),
        cwd: root.clone(),
        env: HashMap::new(),
        size,
        cell_px: (16, 32),
    })
    .unwrap();
    let exited = Arc::new(Mutex::new(false));
    let ex = exited.clone();
    let session = pty::start(
        rt.handle(),
        pty.clone(),
        feeder.clone(),
        Arc::new(move |e| {
            if let SessionEvent::Exited(_) = e {
                *ex.lock() = true;
            }
        }),
    )
    .unwrap();

    if wait_for(&feeder, &["trust this folder"], Duration::from_secs(10)) {
        println!("trust dialog shown: choosing 'Yes, I trust this folder'");
        // The dialog takes input only once it has finished starting up.
        std::thread::sleep(Duration::from_millis(1500));
        for attempt in 0..5 {
            session.write(b"\x1b[B".to_vec()); // Down
            if wait_for(&feeder, &["❯ Yes, I trust"], Duration::from_millis(800)) {
                println!("selection moved (attempt {attempt})");
                break;
            }
        }
        session.write(b"\r".to_vec());
    }
    // Further first-run questions about this project's settings, if any, are printed.
    std::thread::sleep(Duration::from_secs(6));
    println!("----- screen after start -----\n{}\n------------------------------", screen(&feeder));
    session.write(b"/exit".to_vec());
    std::thread::sleep(Duration::from_millis(500));
    session.write(b"\r".to_vec());
    let t = Instant::now();
    while !*exited.lock() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    if !*exited.lock() {
        pty.hang_up(Duration::from_millis(500));
        std::thread::sleep(Duration::from_millis(800));
    }
    let lines = std::fs::read_to_string(&log).unwrap_or_default();
    let ran: Vec<&str> = lines.lines().collect();
    println!("hooks that ran: {ran:?}");
    let verdict = match (ran.contains(&"project"), ran.contains(&"flag")) {
        (true, true) => "MERGED: --settings hooks run alongside hooks from other settings files",
        (false, true) => "REPLACED: --settings hooks replace hooks from other settings files",
        (true, false) => "IGNORED: --settings hooks did not run",
        (false, false) => "INCONCLUSIVE: no SessionStart hook ran",
    };
    println!("verdict: {verdict}");
}
