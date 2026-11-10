//! PDF pages, rendered on demand on a thread of their own (one per open file): the UI asks
//! for the pages it shows at the scale it shows them, and takes the pictures as they come.
//! The thread waits without using the CPU when nothing is asked, and ends when the
//! [`Renderer`] is dropped.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use crate::Rgba;

/// Longest side of a rendered page, in pixels (bigger zooms are drawn scaled up).
pub const MAX_SIDE: f32 = 4096.0;

#[derive(Debug)]
pub enum Event {
    /// The file was read: each page's size in points (as shown, rotation applied).
    Opened { pages: Vec<(f32, f32)> },
    /// A page as asked: `scale` is pixels per point (as asked; the picture may be smaller when
    /// it would be too big).
    Page { index: usize, scale: f32, image: Rgba },
    Failed(String),
}

/// A page and how many pixels per point it is wanted at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Request {
    index: usize,
    scale: f32,
}

pub struct Renderer {
    tx: mpsc::Sender<Request>,
    pub events: mpsc::Receiver<Event>,
}

impl Renderer {
    /// Starts reading `path`; `notify` is called after each event is sent (to wake the UI).
    pub fn open(path: PathBuf, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, requests) = mpsc::channel::<Request>();
        let (events_tx, events) = mpsc::channel::<Event>();
        let send = move |e: Event| {
            let ok = events_tx.send(e).is_ok();
            notify();
            ok
        };
        let _ = std::thread::Builder::new()
            .name("ternarycode-pdf".into())
            .spawn(move || work(path, requests, send));
        Self { tx, events }
    }

    /// Asks for a page (again, when the scale changed). Pages asked last are drawn first.
    pub fn request(&self, index: usize, scale: f32) {
        let _ = self.tx.send(Request { index, scale });
    }
}

fn error_text(e: LoadPdfError) -> String {
    match e {
        LoadPdfError::Decryption(_) => "the PDF is password-protected".into(),
        LoadPdfError::Invalid => "the file is not a readable PDF".into(),
    }
}

fn work(path: PathBuf, requests: mpsc::Receiver<Request>, send: impl Fn(Event) -> bool) {
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            send(Event::Failed(e.to_string()));
            return;
        }
    };
    if bytes.len() as u64 > crate::MAX_BYTES {
        send(Event::Failed("the file is too big to show".into()));
        return;
    }
    // A broken file can make the parser panic: report it instead of losing the thread quietly.
    let pdf = match std::panic::catch_unwind(move || Pdf::new(bytes)) {
        Ok(Ok(pdf)) => pdf,
        Ok(Err(e)) => {
            send(Event::Failed(error_text(e)));
            return;
        }
        Err(_) => {
            send(Event::Failed("the file is not a readable PDF".into()));
            return;
        }
    };
    let pages: Vec<(f32, f32)> = pdf.pages().iter().map(|p| p.render_dimensions()).collect();
    if pages.is_empty() {
        send(Event::Failed("the PDF has no pages".into()));
        return;
    }
    if !send(Event::Opened { pages: pages.clone() }) {
        return;
    }
    let cache = RenderCache::new();
    let interpreter = InterpreterSettings::default();
    let settings = RenderSettings::default();
    let mut queue: Vec<Request> = Vec::new();
    loop {
        // Wait for work, then take everything asked meanwhile.
        if queue.is_empty() {
            match requests.recv() {
                Ok(r) => queue.push(r),
                Err(_) => return,
            }
        }
        while let Ok(r) = requests.try_recv() {
            queue.retain(|q| q.index != r.index);
            queue.push(r);
        }
        let Some(req) = queue.pop() else { continue };
        let Some(page) = pdf.pages().get(req.index) else { continue };
        let (w, h) = pages[req.index];
        let scale = req.scale.clamp(0.05, MAX_SIDE / w.max(h).max(1.0));
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hayro::render(
                page,
                &cache,
                &interpreter,
                &settings,
                &PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: WHITE,
                },
            )
        }));
        let image = match rendered {
            // On white, every pixel is opaque: premultiplied is the same as straight.
            Ok(pixmap) => Rgba {
                width: pixmap.width() as u32,
                height: pixmap.height() as u32,
                pixels: pixmap.data_as_u8_slice().to_vec(),
            },
            Err(_) => Rgba::default(),
        };
        if !send(Event::Page { index: req.index, scale: req.scale, image }) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A one-page PDF (US Letter) with a black square.
    fn tiny_pdf() -> Vec<u8> {
        let content = b"0 0 0 rg 100 100 200 200 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), String::from_utf8_lossy(content)),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }

    #[test]
    fn pages_render_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, tiny_pdf()).unwrap();
        let r = Renderer::open(path, Arc::new(|| {}));
        let opened = r.events.recv_timeout(Duration::from_secs(10)).unwrap();
        let Event::Opened { pages } = opened else { panic!("{opened:?}") };
        assert_eq!(pages, vec![(612.0, 792.0)]);
        r.request(0, 0.5);
        let Event::Page { index, image, .. } = r.events.recv_timeout(Duration::from_secs(20)).unwrap() else {
            panic!("no page")
        };
        assert_eq!(index, 0);
        assert_eq!((image.width, image.height), (306, 396));
        // White margin, black square (PDF y grows upwards: the square is near the bottom).
        let px = |x: u32, y: u32| image.pixels[((y * image.width + x) * 4) as usize];
        assert_eq!(px(5, 5), 255);
        assert_eq!(px(100, 396 - 100), 0);
    }

    #[test]
    fn a_broken_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.pdf");
        std::fs::write(&path, b"%PDF-1.4 nothing here").unwrap();
        let r = Renderer::open(path, Arc::new(|| {}));
        assert!(matches!(r.events.recv_timeout(Duration::from_secs(10)).unwrap(), Event::Failed(_)));
    }
}
