//! Reads documents and prints what was found (counts and timings only, never their text):
//! `cargo run -p tc-doc --example smoke -- FILE...`.

use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    for arg in std::env::args().skip(1) {
        let path = std::path::PathBuf::from(&arg);
        let name = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let t = Instant::now();
        match tc_doc::Kind::of(&path) {
            Some(tc_doc::Kind::Pdf) => {
                let r = tc_doc::pdf::Renderer::open(path.clone(), Arc::new(|| {}));
                match r.events.recv_timeout(Duration::from_secs(30)) {
                    Ok(tc_doc::pdf::Event::Opened { pages }) => {
                        let opened = t.elapsed();
                        r.request(0, 2.0);
                        let page = r.events.recv_timeout(Duration::from_secs(60));
                        let ok = matches!(&page, Ok(tc_doc::pdf::Event::Page { image, .. }) if image.width > 0);
                        println!("pdf: {} pages, opened {:?}, page 1 at 2x {:?} ok={ok}", pages.len(), opened, t.elapsed() - opened);
                    }
                    other => println!("pdf: {other:?}"),
                }
            }
            Some(_) => match tc_doc::load(&path) {
                Ok(tc_doc::Doc::Text(d)) => println!("{name}: {} blocks, {} images ({} skipped), {:?}", d.blocks.len(), d.images.len(), d.images_skipped, t.elapsed()),
                Ok(tc_doc::Doc::Slides(s)) => println!("{name}: {} slides ({} titled), {} images ({} skipped), aspect {:.2}, {:?}", s.slides.len(), s.slides.iter().filter(|x| x.title.is_some()).count(), s.images.len(), s.images_skipped, s.aspect, t.elapsed()),
                Ok(tc_doc::Doc::Sheet(b)) => println!("{name}: {} sheets, sizes {:?}, {:?}", b.sheets.len(), b.sheets.iter().map(|s| (s.rows, s.cols, s.error.is_some())).collect::<Vec<_>>(), t.elapsed()),
                Ok(tc_doc::Doc::Image(i)) => println!("{name}: {}x{}", i.width, i.height),
                Err(e) => println!("{name}: error {e}"),
            },
            None => println!("{name}: not a document"),
        }
    }
}
