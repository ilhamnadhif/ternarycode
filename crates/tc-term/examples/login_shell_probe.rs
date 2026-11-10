//! What is in the foreground of a PTY started with the default (login) shell, and while a job
//! runs? `cargo run -p tc-term --example login_shell_probe`
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tc_term::core::{Feeder, GridSize, new_terminal, term_config};
use tc_term::pty::{self, LocalPty, SpawnOptions};
use tc_term::scheme::SCHEMES;

fn ps(pids: &[i32]) {
    let list: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
    let out = std::process::Command::new("ps")
        .args(["-o", "pid,ppid,pgid,comm", "-p", &list.join(",")])
        .output()
        .unwrap();
    print!("{}", String::from_utf8_lossy(&out.stdout));
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let size = GridSize { cols: 80, rows: 24 };
    let (shared, rx) = new_terminal(term_config(100, Default::default(), false), size, (&SCHEMES[0]).into());
    let feeder = Arc::new(Mutex::new(Feeder::new(shared, rx)));
    let pty = LocalPty::spawn(&SpawnOptions {
        program: None,
        cwd: std::env::temp_dir(),
        env: HashMap::new(),
        size,
        cell_px: (8, 16),
    })
    .unwrap();
    let s = pty::start(rt.handle(), pty.clone(), feeder, Arc::new(|_| {})).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let fg = pty.foreground_pgid();
    println!("at the prompt: pty child pid {}, foreground pgid {fg:?}", pty.pid());
    ps(&[pty.pid() as i32, fg.unwrap_or(0)]);
    s.write(b"sleep 3\r".to_vec());
    std::thread::sleep(Duration::from_millis(600));
    let fg2 = pty.foreground_pgid();
    println!("running `sleep 3`: foreground pgid {fg2:?}");
    ps(&[fg2.unwrap_or(0)]);
    pty.hang_up(Duration::from_millis(300));
    std::thread::sleep(Duration::from_millis(500));
}
