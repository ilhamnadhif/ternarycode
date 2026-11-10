//! Gate A throughput benchmark: how fast `Feeder::feed` parses typical output, and what share of
//! that the OSC scanner costs (target: under 3%).
//!
//! `cargo run -p tc-term --release --example bench_feed`

use std::time::Instant;

use tc_term::core::{Feeder, GridSize, new_terminal, term_config};
use tc_term::osc::OscScanner;
use tc_term::scheme::SCHEMES;

fn corpus(target: usize) -> Vec<u8> {
    let lines: [&[u8]; 6] = [
        b"\x1b[1;32m\xe2\x9c\x93\x1b[0m Updated src/module.rs \x1b[90m(+12 -3)\x1b[0m compiling crate with features [default, std]\r\n",
        b"test tests::some_case_name ... \x1b[32mok\x1b[0m\r\n",
        b"\x1b[38;2;120;80;200mfn render(&mut self, ctx: &egui::Context) -> Result<(), Error> { Ok(()) }\x1b[0m\r\n",
        b"    Finished `release` profile [optimized] target(s) in 34.81s\r\n",
        b"plain output line: the quick brown fox jumps over the lazy dog 0123456789 abcdefghijklmnop\r\n",
        b"\x1b[2K\x1b[1G\x1b[36m\xe2\xa0\x8b\x1b[0m Thinking\xe2\x80\xa6 \x1b[90m(esc to interrupt)\x1b[0m\r",
    ];
    let mut out = Vec::with_capacity(target + 256);
    let mut i = 0;
    while out.len() < target {
        out.extend_from_slice(lines[i % lines.len()]);
        // An occasional title update and notification, as agents emit.
        if i % 997 == 0 {
            out.extend_from_slice(b"\x1b]0;claude: working\x07\x1b]9;4;1;50\x07");
        }
        i += 1;
    }
    out
}

fn main() {
    let data = corpus(100 * 1024 * 1024);
    let mb = data.len() as f64 / (1024.0 * 1024.0);
    let read = 64 * 1024;

    // Scanner alone.
    let mut best_scan = f64::MAX;
    for _ in 0..5 {
        let mut s = OscScanner::new();
        let mut out = Vec::new();
        let t = Instant::now();
        for chunk in data.chunks(read) {
            s.scan(chunk, &mut out);
        }
        best_scan = best_scan.min(t.elapsed().as_secs_f64());
        std::hint::black_box(&out);
    }

    // Full feed (scanner + parser + grid), 120x40 with 10 000 lines of scrollback.
    let mut best_feed = f64::MAX;
    for _ in 0..3 {
        let (shared, rx) = new_terminal(
            term_config(10_000, Default::default(), false),
            GridSize { cols: 120, rows: 40 },
            (&SCHEMES[0]).into(),
        );
        let mut f = Feeder::new(shared, rx);
        let t = Instant::now();
        let mut signals = 0;
        for chunk in data.chunks(read) {
            signals += f.feed(chunk).signals.len();
        }
        best_feed = best_feed.min(t.elapsed().as_secs_f64());
        std::hint::black_box(signals);
    }

    println!("corpus: {mb:.0} MB, read size {} KB", read / 1024);
    println!(
        "OSC scanner alone : {:>7.3} s  ({:>6.0} MB/s)",
        best_scan,
        mb / best_scan
    );
    println!(
        "Feeder::feed total: {:>7.3} s  ({:>6.0} MB/s)",
        best_feed,
        mb / best_feed
    );
    println!(
        "scanner share of feed: {:.2}% (Gate A target < 3%)",
        best_scan / best_feed * 100.0
    );
}
