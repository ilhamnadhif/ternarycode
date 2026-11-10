//! Times reading every page's text of the PDFs given (counts only, no content):
//! `cargo run -p tc-doc --release --example pdf_text -- a.pdf b.pdf`.
use std::sync::Arc;
use std::time::{Duration, Instant};
use tc_doc::pdf::{Event, Renderer};

fn main() {
    for (n, path) in std::env::args().skip(1).enumerate() {
        let r = Renderer::open(path.into(), Arc::new(|| {}));
        let pages = match r.events.recv_timeout(Duration::from_secs(30)) {
            Ok(Event::Opened { pages }) => pages.len(),
            _ => { println!("pdf {n}: not readable"); continue; }
        };
        let t = Instant::now();
        r.request_text();
        let (mut chars, mut got) = (0, 0);
        while got < pages {
            match r.events.recv_timeout(Duration::from_secs(60)) {
                Ok(Event::Text { text, .. }) => { chars += text.chars.len(); got += 1; }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        println!("pdf {n}: {pages} pages, {chars} chars, {:?} ({:?}/page)", t.elapsed(), t.elapsed() / pages.max(1) as u32);
    }
}
