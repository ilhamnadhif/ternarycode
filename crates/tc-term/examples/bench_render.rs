//! Gate A renderer benchmark (R1: the egui galley renderer from TernaryShell).
//!
//! Runs the terminal widget headless for many frames while every pane streams coloured output,
//! and reports the CPU time per frame: layout of the UI pass plus tessellation, which is all the
//! CPU work of a frame except the GPU upload.
//!
//! `cargo run -p tc-term --release --example bench_render`

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Pos2, Rect, ViewportId, ViewportInfo, vec2};
use parking_lot::Mutex;
use tc_term::core::{Feeder, GridSize, new_terminal, term_config};
use tc_term::fonts;
use tc_term::prefs::FontChoice;
use tc_term::scheme::SCHEMES;
use tc_term::widget::{self, TermStyle, TermView};

struct Pane {
    feeder: Arc<Mutex<Feeder>>,
    shared: tc_term::core::SharedTerm,
    view: TermView,
}

fn line(seed: u64, n: u64) -> String {
    // Coloured, mixed output similar to an agent's or a build's: SGR colours, bold, box drawing
    // now and then, and plain text.
    let colors = [31, 32, 33, 34, 35, 36, 90, 37];
    let c = colors[((seed + n) % colors.len() as u64) as usize];
    match n % 7 {
        0 => format!("\x1b[1;{c}m✓\x1b[0m Updated src/module_{n}.rs \x1b[90m(+{} -{})\x1b[0m  compiling crate number {seed} with features [default, std]\r\n", n % 40, n % 9),
        1 => format!("\x1b[{c}m│\x1b[0m {:<60} \x1b[{c}m│\x1b[0m\r\n", format!("test tests::case_{n} ... ok")),
        2 => format!("\x1b[38;2;{};{};200m{}\x1b[0m\r\n", (n * 7) % 255, (seed * 3) % 255, "fn render(&mut self, ctx: &egui::Context) -> Result<(), Error> { self.frame += 1; Ok(()) }"),
        3 => "    Finished `release` profile [optimized] target(s) in 34.81s — all good, 0 warnings\r\n".to_string(),
        4 => format!("\x1b[48;5;236m\x1b[{c}m > Running tool: Edit(src/app.rs) line {n}             \x1b[0m\r\n"),
        5 => "────────────────────────────────────────────────────────────────────────────\r\n".to_string(),
        _ => format!("plain output line {n} from pane {seed}: the quick brown fox jumps over the lazy dog 0123456789\r\n"),
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn run(name: &str, panes_n: usize, size: GridSize, streaming: usize, lines_per_frame: u64) {
    run_ex(name, panes_n, size, streaming, lines_per_frame, false);
}

/// `flood`: a background thread pushes 100 MB into pane 0 in 1 KB reads (what a macOS PTY
/// delivers for `cat` of a big file) while the frames are measured.
fn run_ex(
    name: &str,
    panes_n: usize,
    size: GridSize,
    streaming: usize,
    lines_per_frame: u64,
    flood: bool,
) {
    let ctx = egui::Context::default();
    let choice = fonts::effective_choice(&FontChoice::default());
    fonts::apply(&ctx, &choice, false);
    let ppp = 2.0;
    let font_size = 13.0;

    let mut panes: Vec<Pane> = (0..panes_n)
        .map(|_| {
            let (shared, rx) = new_terminal(
                term_config(10_000, Default::default(), false),
                size,
                (&SCHEMES[0]).into(),
            );
            Pane {
                feeder: Arc::new(Mutex::new(Feeder::new(shared.clone(), rx))),
                shared,
                view: TermView::default(),
            }
        })
        .collect();

    // Warm-up frame to load fonts and measure the cell.
    let raw_for = |screen: Rect| {
        let mut raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        raw.viewports.insert(
            ViewportId::ROOT,
            ViewportInfo {
                native_pixels_per_point: Some(ppp),
                ..Default::default()
            },
        );
        raw
    };
    let _ = ctx.run_ui(raw_for(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), |_| {});
    let cell = widget::cell_size(&ctx, font_size);
    let pane_w = size.cols as f32 * cell.x + 2.0 * widget::PAD;
    let pane_h = size.rows as f32 * cell.y + 2.0 * widget::PAD;
    let cols = (panes_n as f32).sqrt().ceil() as usize;
    let rows = panes_n.div_ceil(cols);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(pane_w * cols as f32, pane_h * rows as f32));
    let scheme = &SCHEMES[0];
    let style = TermStyle {
        font_size,
        cell,
        scheme,
        real_bold: false,
        real_italic: false,
        option_as_meta: false,
        copy_on_select: false,
        cursor_blink: false,
    };

    // Fill the screens first.
    for (i, p) in panes.iter().enumerate() {
        let mut f = p.feeder.lock();
        for n in 0..size.rows as u64 {
            f.feed(line(i as u64, n).as_bytes());
        }
    }

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flooder = flood.then(|| {
        let feeder = panes[0].feeder.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut block = Vec::new();
            let mut n = 0;
            while block.len() < 1024 * 1024 {
                block.extend_from_slice(line(0, n).as_bytes());
                n += 1;
            }
            let mut fed = 0usize;
            let t = Instant::now();
            'outer: while fed < 100 * 1024 * 1024 {
                for chunk in block.chunks(1024) {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break 'outer;
                    }
                    feeder.lock().feed(chunk);
                    fed += chunk.len();
                }
            }
            (fed, t.elapsed())
        })
    });
    let frames = 400;
    let mut times = Vec::with_capacity(frames);
    let mut counter = 1000u64;
    for frame in 0..frames + 30 {
        for (i, p) in panes.iter().enumerate().take(streaming) {
            let mut f = p.feeder.lock();
            let mut chunk = String::new();
            for _ in 0..lines_per_frame {
                counter += 1;
                chunk.push_str(&line(i as u64, counter));
            }
            f.feed(chunk.as_bytes());
        }
        let t0 = Instant::now();
        let out = ctx.run_ui(raw_for(screen), |ui| {
            for (i, p) in panes.iter_mut().enumerate() {
                let (c, r) = (i % cols, i / cols);
                let rect = Rect::from_min_size(
                    Pos2::new(c as f32 * pane_w, r as f32 * pane_h),
                    vec2(pane_w, pane_h),
                );
                let _ = widget::show(ui, rect, &mut p.view, &p.shared, &style, false, true, true);
            }
        });
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let dt = t0.elapsed();
        std::hint::black_box(prims);
        if frame >= 30 {
            times.push(dt);
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(h) = flooder {
        let (fed, took) = h.join().unwrap();
        println!(
            "  flood into pane 0 during the frames: {:.0} MB in {:.2} s",
            fed as f64 / (1024.0 * 1024.0),
            took.as_secs_f64()
        );
    }
    times.sort();
    let mean: Duration = times.iter().sum::<Duration>() / times.len() as u32;
    println!(
        "{name:<44} panes={panes_n} {}x{} streaming={streaming} lines/frame={lines_per_frame}  \
         mean={:>6.2} ms  p50={:>6.2} ms  p95={:>6.2} ms  max={:>6.2} ms",
        size.cols,
        size.rows,
        mean.as_secs_f64() * 1e3,
        percentile(&times, 0.5).as_secs_f64() * 1e3,
        percentile(&times, 0.95).as_secs_f64() * 1e3,
        times.last().unwrap().as_secs_f64() * 1e3,
    );
}

fn main() {
    let big = GridSize { cols: 120, rows: 40 };
    println!("R1 renderer (egui galleys), CPU time per frame incl. tessellation, ppp=2\n");
    run("Gate A: 9 panes, all streaming full screens", 9, big, 9, 40);
    run("9 panes, all streaming 5 lines/frame", 9, big, 9, 5);
    run("9 panes, 3 streaming", 9, big, 3, 5);
    run("9 panes, idle (static content)", 9, big, 0, 0);
    run("Focus: 1 pane 200x60 streaming full screens", 1, GridSize { cols: 200, rows: 60 }, 1, 60);
    run_ex("Gate A: 9 panes, pane 0 flooded (cat 100 MB)", 9, big, 0, 0, true);
}
