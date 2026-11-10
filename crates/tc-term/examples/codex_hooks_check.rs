//! Can Codex hooks be added for one launch with `-c hooks.<Event>=[…]`, and what does a hook
//! receive? Starts Codex with a SessionStart hook that saves its stdin, waits, prints the screen
//! and the captured payload, and quits. No prompt is sent.
//!
//! `cargo run -p tc-term --release --example codex_hooks_check`

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

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let out = root.join("payload.json");
    let cmd = format!("cat > '{}'", out.display());
    let hooks = format!(
        r#"hooks.SessionStart=[{{hooks=[{{type="command",command="{}"}}]}}]"#,
        cmd.replace('\\', "\\\\").replace('"', "\\\"")
    );
    println!("override: {hooks}");
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
        program: Some(("codex".into(), vec!["-c".into(), hooks])),
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
    let t = Instant::now();
    let mut skipped = false;
    while t.elapsed() < Duration::from_secs(15) && !out.exists() {
        std::thread::sleep(Duration::from_millis(200));
        let s = screen(&feeder);
        if !skipped && s.contains("Update available") && s.contains("esc skip") {
            std::thread::sleep(Duration::from_millis(800));
            println!("update dialog: pressing Esc (skip)");
            session.write(b"\x1b".to_vec());
            skipped = true;
        }
    }
    std::thread::sleep(Duration::from_millis(800));
    println!("----- screen -----\n{}\n------------------", screen(&feeder));
    for keys in [&b"\x03"[..], b"\x03"] {
        session.write(keys.to_vec());
        std::thread::sleep(Duration::from_millis(500));
    }
    let t = Instant::now();
    while !*exited.lock() && t.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(100));
    }
    if !*exited.lock() {
        pty.hang_up(Duration::from_millis(500));
        std::thread::sleep(Duration::from_millis(700));
    }
    match std::fs::read_to_string(&out) {
        Ok(p) => println!("SessionStart payload: {p}"),
        Err(_) => println!("SessionStart hook did not run"),
    }
}
