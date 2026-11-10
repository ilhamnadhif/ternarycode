//! Gate A TUI check: runs full-screen programs and AI agents on a local PTY (no window), waits
//! for their first screen, prints it, checks for an expected text, and quits them.
//!
//! `cargo run -p tc-term --release --example tui_check [names…]`
//!
//! Agents are only started, never given a prompt: no request goes to a model.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use parking_lot::Mutex;
use tc_term::core::{Feeder, GridSize, new_terminal, term_config};
use tc_term::pty::{self, LocalPty, SessionEvent, SpawnOptions};
use tc_term::scheme::SCHEMES;

struct Case {
    name: &'static str,
    argv: &'static [&'static str],
    /// Any of these on screen counts as rendered.
    expect: &'static [&'static str],
    quit: &'static [&'static [u8]],
    wait: Duration,
}

fn screen(feeder: &Arc<Mutex<Feeder>>) -> Vec<String> {
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
        .collect()
}

fn main() {
    let only: Vec<String> = std::env::args().skip(1).collect();
    let cases = [
        Case { name: "vim", argv: &["vim", "-u", "NONE"], expect: &["VIM", "Vi IMproved"], quit: &[b"\x1b:q!\r"], wait: Duration::from_millis(1500) },
        Case { name: "less", argv: &["less", "/etc/hosts"], expect: &["localhost"], quit: &[b"q"], wait: Duration::from_millis(1000) },
        Case { name: "top (htop not installed)", argv: &["top"], expect: &["Processes", "PID"], quit: &[b"q"], wait: Duration::from_millis(2500) },
        Case { name: "claude", argv: &["claude"], expect: &["Claude", "trust", "Welcome"], quit: &[b"\x1b", b"\x03", b"\x03"], wait: Duration::from_secs(6) },
        Case { name: "codex", argv: &["codex"], expect: &["Codex", "OpenAI", "codex", "trust"], quit: &[b"\x03", b"\x03"], wait: Duration::from_secs(6) },
        Case { name: "opencode", argv: &["opencode"], expect: &["opencode", "OpenCode", "ctrl"], quit: &[b"\x03", b"\x03"], wait: Duration::from_secs(8) },
        Case { name: "gemini", argv: &["gemini"], expect: &["Gemini", "gemini", "Tips", "trust"], quit: &[b"\x03", b"\x03"], wait: Duration::from_secs(8) },
    ];
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let size = GridSize { cols: 120, rows: 40 };
    let mut summary = Vec::new();
    for case in &cases {
        if !only.is_empty() && !only.iter().any(|o| case.name.starts_with(o.as_str())) {
            continue;
        }
        let (shared, rx) = new_terminal(
            term_config(5000, Default::default(), false),
            size,
            (&SCHEMES[0]).into(),
        );
        let feeder = Arc::new(Mutex::new(Feeder::new(shared, rx)));
        let pty = match LocalPty::spawn(&SpawnOptions {
            program: Some((
                case.argv[0].to_string(),
                case.argv[1..].iter().map(|s| s.to_string()).collect(),
            )),
            cwd: dir.path().to_path_buf(),
            env: HashMap::new(),
            size,
            cell_px: (16, 32),
        }) {
            Ok(p) => p,
            Err(e) => {
                summary.push(format!("{:<28} SPAWN FAILED: {e}", case.name));
                continue;
            }
        };
        let exited = Arc::new(Mutex::new(None));
        let ex = exited.clone();
        let session = pty::start(
            rt.handle(),
            pty.clone(),
            feeder.clone(),
            Arc::new(move |e| {
                if let SessionEvent::Exited(info) = e {
                    *ex.lock() = Some(info);
                }
            }),
        )
        .unwrap();
        let t0 = Instant::now();
        // Wait for the expected text (or the time limit).
        let mut found = None;
        while t0.elapsed() < case.wait {
            std::thread::sleep(Duration::from_millis(100));
            let s = screen(&feeder).join("\n");
            if let Some(e) = case.expect.iter().find(|e| s.contains(**e)) {
                found = Some(*e);
                // Let the first screen settle.
                std::thread::sleep(Duration::from_millis(600));
                break;
            }
        }
        let lines = screen(&feeder);
        let alt = feeder
            .lock()
            .shared
            .term
            .lock()
            .mode()
            .contains(TermMode::ALT_SCREEN);
        let fg = pty.foreground_pgid();
        println!(
            "===== {} ({} ms, alt screen: {alt}, fg pgid: {fg:?}, shell pid: {}) =====",
            case.name,
            t0.elapsed().as_millis(),
            pty.pid()
        );
        for l in lines.iter().filter(|l| !l.trim().is_empty()).take(18) {
            println!("| {l}");
        }
        for keys in case.quit {
            session.write(keys.to_vec());
            std::thread::sleep(Duration::from_millis(400));
        }
        let t1 = Instant::now();
        while exited.lock().is_none() && t1.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let quit_ok = exited.lock().is_some();
        if !quit_ok {
            pty.hang_up(Duration::from_millis(500));
            std::thread::sleep(Duration::from_millis(700));
        }
        summary.push(format!(
            "{:<28} rendered: {:<5} ({:<12}) quit by keys: {}",
            case.name,
            found.is_some(),
            found.unwrap_or("-"),
            quit_ok
        ));
    }
    println!("\n===== summary =====");
    for s in summary {
        println!("{s}");
    }
}
