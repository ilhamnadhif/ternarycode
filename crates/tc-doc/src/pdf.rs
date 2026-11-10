//! PDF pages, rendered on demand on a thread of their own (one per open file): the UI asks
//! for the pages it shows at the scale it shows them, and takes the pictures as they come.
//! For a search it also reads each page's text with where every character is ([`PageText`]),
//! between renders. The thread waits without using the CPU when nothing is asked, and ends
//! when the [`Renderer`] is dropped.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Point, Rect as KRect};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
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
    /// A page's text (asked with [`Renderer::request_text`]).
    Text { index: usize, text: PageText },
    Failed(String),
}

/// A page and how many pixels per point it is wanted at; or every page's text.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Request {
    Render { index: usize, scale: f32 },
    Text,
}

pub struct Renderer {
    tx: mpsc::Sender<Request>,
    pub events: mpsc::Receiver<Event>,
}

/// One character of a page's text and the box it takes, in points from the page's top left
/// (as the page is shown, rotation applied).
#[derive(Clone, Debug, PartialEq)]
pub struct TextChar {
    pub text: String,
    pub rect: [f32; 4],
}

/// A page's text in reading order, lower-cased for searching, with the box of each character
/// (none for the spaces and line ends put between words and lines).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    pub chars: Vec<char>,
    pub rects: Vec<Option<[f32; 4]>>,
}

impl PageText {
    /// From the characters as the page draws them: a space where a gap is wider than a quarter
    /// of the text's height, a line end where the text moves to another line.
    pub fn from_chars(drawn: &[TextChar]) -> Self {
        let mut out = PageText::default();
        let mut last: Option<[f32; 4]> = None;
        for c in drawn {
            let r = c.rect;
            if let Some(l) = last {
                let height = (r[3] - r[1]).max(l[3] - l[1]).max(1.0);
                let same_line = (r[1] + r[3]) / 2.0 > l[1] && (r[1] + r[3]) / 2.0 < l[3];
                let gap = r[0] - l[2];
                if !same_line {
                    out.push_sep('\n');
                } else if gap > height * 0.25 && !c.text.starts_with(' ') && out.chars.last() != Some(&' ') {
                    out.push_sep(' ');
                }
            }
            for ch in c.text.chars().flat_map(char::to_lowercase) {
                out.chars.push(ch);
                out.rects.push(Some(r));
            }
            last = Some(r);
        }
        out
    }

    fn push_sep(&mut self, c: char) {
        self.chars.push(c);
        self.rects.push(None);
    }

    /// Where `query` occurs (case-insensitive; any run of spaces and line ends matches any
    /// other): each match as the boxes of its lines.
    pub fn find(&self, query: &str) -> Vec<Vec<[f32; 4]>> {
        let q: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
        if q.is_empty() {
            return Vec::new();
        }
        let blank = |c: char| c == ' ' || c == '\n';
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.chars.len() {
            // Match from i, letting a space in the query take any run of blanks.
            let (mut a, mut b) = (i, 0);
            while b < q.len() && a < self.chars.len() {
                if blank(q[b]) {
                    if !blank(self.chars[a]) {
                        break;
                    }
                    while a < self.chars.len() && blank(self.chars[a]) {
                        a += 1;
                    }
                    while b < q.len() && blank(q[b]) {
                        b += 1;
                    }
                } else if self.chars[a] == q[b] {
                    a += 1;
                    b += 1;
                } else {
                    break;
                }
            }
            if b == q.len() {
                out.push(merge_lines(self.rects[i..a].iter().flatten()));
                i = a.max(i + 1);
            } else {
                i += 1;
            }
        }
        out
    }
}

/// The boxes of a match's characters joined into one box per line.
fn merge_lines<'a>(rects: impl Iterator<Item = &'a [f32; 4]>) -> Vec<[f32; 4]> {
    let mut out: Vec<[f32; 4]> = Vec::new();
    for r in rects {
        match out.last_mut() {
            Some(l) if (r[1] + r[3]) / 2.0 > l[1] && (r[1] + r[3]) / 2.0 < l[3] => {
                l[0] = l[0].min(r[0]);
                l[1] = l[1].min(r[1]);
                l[2] = l[2].max(r[2]);
                l[3] = l[3].max(r[3]);
            }
            _ => out.push(*r),
        }
    }
    out
}

/// A device that only notes the glyphs drawn and where.
#[derive(Default)]
struct TextDevice {
    chars: Vec<TextChar>,
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let glyphs = run.glyphs();
        for (i, g) in glyphs.iter().enumerate() {
            let Some(text) = g.as_unicode().map(|u| match u {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            }) else {
                continue;
            };
            if text.trim().is_empty() && text != " " {
                continue;
            }
            // Glyph space is 1000 units to the em: the box runs from the baseline's origin to
            // the next glyph's (or the advance), a fifth of the em below to four fifths above.
            let t: Affine = props.transform * g.transform();
            let advance = match &**g {
                Glyph::Outline(o) => o.advance_width(),
                Glyph::Type3(t3) => t3.advance_width(),
            };
            let next = glyphs.get(i + 1).map(|n| t.inverse() * (props.transform * n.transform() * Point::ZERO));
            let width = match (next, advance) {
                (Some(p), _) if p.x > 1.0 && p.y.abs() < 1.0 => p.x,
                (_, Some(a)) if a > 0.0 => a as f64,
                _ => 500.0,
            };
            let bbox = KRect::new(0.0, -200.0, width, 800.0);
            let r = t.transform_rect_bbox(bbox);
            self.chars.push(TextChar { text, rect: [r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32] });
        }
    }
}

/// The text of a page with where each character is (points from the top left, as rendered).
fn page_text<'a>(page: &hayro::hayro_syntax::page::Page<'a>, cache: &InterpreterCache<'a>, settings: &InterpreterSettings) -> PageText {
    let (w, h) = page.render_dimensions();
    let mut ctx = Context::new(
        page.initial_transform(true).to_kurbo(),
        KRect::new(0.0, 0.0, w as f64, h as f64),
        cache,
        page.xref(),
        settings.clone(),
    );
    let mut device = TextDevice::default();
    interpret_page(page, &mut ctx, &mut device);
    PageText::from_chars(&device.chars)
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
        let _ = self.tx.send(Request::Render { index, scale });
    }

    /// Asks for every page's text ([`Event::Text`], page by page, between renders).
    pub fn request_text(&self) {
        let _ = self.tx.send(Request::Text);
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
    let text_cache = InterpreterCache::new();
    let mut queue: Vec<(usize, f32)> = Vec::new();
    // Pages whose text is still to be read (asked once; read between renders).
    let mut texts: Option<std::ops::Range<usize>> = None;
    let add = |r: Request, queue: &mut Vec<(usize, f32)>, texts: &mut Option<std::ops::Range<usize>>| match r {
        Request::Render { index, scale } => {
            queue.retain(|q| q.0 != index);
            queue.push((index, scale));
        }
        Request::Text => {
            if texts.is_none() {
                *texts = Some(0..pages.len());
            }
        }
    };
    loop {
        // Wait for work, then take everything asked meanwhile.
        if queue.is_empty() && texts.as_ref().is_none_or(|t| t.is_empty()) {
            match requests.recv() {
                Ok(r) => add(r, &mut queue, &mut texts),
                Err(_) => return,
            }
        }
        while let Ok(r) = requests.try_recv() {
            add(r, &mut queue, &mut texts);
        }
        // Pictures first (they are on screen); a page's text when none is waiting.
        let Some((index, asked)) = queue.pop() else {
            if let Some(i) = texts.as_mut().and_then(Iterator::next)
                && let Some(page) = pdf.pages().get(i)
            {
                let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| page_text(page, &text_cache, &interpreter)))
                    .unwrap_or_default();
                if !send(Event::Text { index: i, text }) {
                    return;
                }
            }
            continue;
        };
        let Some(page) = pdf.pages().get(index) else { continue };
        let (w, h) = pages[index];
        let scale = asked.clamp(0.05, MAX_SIDE / w.max(h).max(1.0));
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
        if !send(Event::Page { index, scale: asked, image }) {
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

    /// A one-page PDF with two lines of Helvetica text.
    fn text_pdf() -> Vec<u8> {
        let content = b"BT /F1 20 Tf 72 700 Td (Hello World) Tj 0 -30 Td (second LINE here) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), String::from_utf8_lossy(content)),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
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
    fn page_text_is_found_where_it_is_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pdf");
        std::fs::write(&path, text_pdf()).unwrap();
        let r = Renderer::open(path, Arc::new(|| {}));
        assert!(matches!(r.events.recv_timeout(Duration::from_secs(10)).unwrap(), Event::Opened { .. }));
        r.request_text();
        let Event::Text { index, text } = r.events.recv_timeout(Duration::from_secs(20)).unwrap() else {
            panic!("no text")
        };
        assert_eq!(index, 0);
        let s: String = text.chars.iter().collect();
        assert_eq!(s, "hello world\nsecond line here", "{s:?}");
        // "world": on the first line (baseline 92 pt from the top), right of "hello".
        let found = text.find("WORLD");
        assert_eq!(found.len(), 1);
        let [x0, y0, x1, y1] = found[0][0];
        assert!(x0 > 120.0 && x1 < 200.0 && y0 < 92.0 && y1 > 88.0 && y0 > 60.0, "{:?}", found[0]);
        // Across the line end, a space in the query matches it.
        let across = text.find("world second");
        assert_eq!(across.len(), 1);
        assert_eq!(across[0].len(), 2, "one box per line");
        assert!(text.find("nothing").is_empty());
    }

    #[test]
    fn spaces_and_lines_are_put_between_words() {
        let c = |t: &str, x: f32, y: f32| TextChar { text: t.into(), rect: [x, y, x + 6.0, y + 12.0] };
        // "ab" then a gap (a word space drawn as a move) then "c", then a new line "d".
        let text = PageText::from_chars(&[c("A", 0.0, 0.0), c("b", 6.0, 0.0), c("c", 20.0, 0.0), c("d", 0.0, 20.0)]);
        assert_eq!(text.chars.iter().collect::<String>(), "ab c\nd");
        assert_eq!(text.find("b c").len(), 1);
        assert_eq!(text.find("AB").len(), 1);
        assert_eq!(text.find("c d")[0].len(), 2);
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
