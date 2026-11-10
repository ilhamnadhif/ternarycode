//! A document shown in the editor instead of text (PDF, Word, PowerPoint, spreadsheets, CSV,
//! images): what was read, and the pictures made from it. Files are read and pages rendered
//! off the UI thread (`tc-doc`); results come back through a channel the view polls when it is
//! drawn. Pictures are kept only while they are on screen. Drawn by `ui::docview_ui`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use eframe::egui;
use tc_doc::{Doc, Kind, Rgba};

pub enum Content {
    Pdf,
    Text(tc_doc::office::RichDoc),
    Slides(tc_doc::office::Slides),
    Sheet(tc_doc::sheet::Workbook),
    Image,
}

pub enum State {
    Loading,
    Ready(Content),
    Failed(String),
}

enum Msg {
    Loaded(Result<Doc, String>),
    Picture(usize, Result<Rgba, String>),
}

/// A picture of the document as the view has it.
pub enum Pic {
    Pending,
    Ready(egui::TextureHandle),
    Failed,
}

pub struct Pdf {
    renderer: tc_doc::pdf::Renderer,
    /// Each page's size in points.
    pub pages: Vec<(f32, f32)>,
    /// Rendered pages on screen, and the scale each was rendered at.
    pub textures: HashMap<usize, (f32, egui::TextureHandle)>,
    asked: HashMap<usize, f32>,
    /// The page at the middle of the view (0-based).
    pub page: usize,
    /// Scroll to this page on the next frame.
    pub jump: Option<usize>,
}

pub struct DocView {
    pub kind: Kind,
    pub path: PathBuf,
    pub state: State,
    /// 1 = fit the view's width (pages, slides, pictures) or the normal text size.
    pub zoom: f32,
    pub sheet: usize,
    /// The selected cell of the sheet (row, column).
    pub cell: Option<(usize, usize)>,
    pub pdf: Option<Pdf>,
    /// Pictures by index (an image file is picture 0).
    pub pictures: HashMap<usize, Pic>,
    /// The picture of an image file, until it is made a texture.
    image: Option<Rgba>,
    pub image_size: Option<(u32, u32)>,
    /// Laid-out text of a Word or PowerPoint file, for the width it was laid out at.
    pub layout: Option<crate::ui::docview_ui::Layout>,
    /// The file changed on disk: read it again when shown.
    pub reload: bool,
    /// The sheet's scroll offset, and whether to bring the selected cell into view.
    pub sheet_scroll: egui::Vec2,
    pub scroll_to_cell: bool,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
}

impl DocView {
    /// Starts reading `path` (shown as loading until it is read).
    pub fn open(path: PathBuf, ctx: &egui::Context) -> Self {
        let kind = Kind::of(&path).unwrap_or(Kind::Unsupported);
        let (tx, rx) = mpsc::channel();
        let mut view = Self {
            kind,
            path,
            state: State::Loading,
            zoom: 1.0,
            sheet: 0,
            cell: None,
            pdf: None,
            pictures: HashMap::new(),
            image: None,
            image_size: None,
            layout: None,
            reload: false,
            sheet_scroll: egui::Vec2::ZERO,
            scroll_to_cell: false,
            tx,
            rx,
        };
        view.start(ctx);
        view
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.layout = None;
        self.pictures.clear();
        self.image = None;
        match self.kind {
            Kind::Unsupported => {
                self.state = State::Failed(crate::i18n::t("This format cannot be shown here.").to_string());
            }
            Kind::Pdf => {
                let c = ctx.clone();
                let renderer = tc_doc::pdf::Renderer::open(self.path.clone(), Arc::new(move || c.request_repaint()));
                let (page, zoom) = (self.pdf.as_ref().map_or(0, |p| p.page), self.zoom);
                self.pdf = Some(Pdf {
                    renderer,
                    pages: Vec::new(),
                    textures: HashMap::new(),
                    asked: HashMap::new(),
                    page,
                    jump: (page > 0).then_some(page),
                });
                self.zoom = zoom;
                self.state = State::Loading;
            }
            _ => {
                let (tx, path, c) = (self.tx.clone(), self.path.clone(), ctx.clone());
                self.state = State::Loading;
                let _ = std::thread::Builder::new().name("ternarycode-doc".into()).spawn(move || {
                    let doc = std::panic::catch_unwind(|| tc_doc::load(&path))
                        .unwrap_or_else(|_| Err(crate::i18n::t("The file could not be read.").to_string()));
                    let _ = tx.send(Msg::Loaded(doc));
                    c.request_repaint();
                });
            }
        }
    }

    /// Takes what arrived from the reading threads. Call before drawing.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if self.reload {
            self.reload = false;
            self.start(ctx);
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Loaded(Ok(doc)) => {
                    self.state = State::Ready(match doc {
                        Doc::Text(d) => Content::Text(d),
                        Doc::Slides(s) => Content::Slides(s),
                        Doc::Sheet(b) => {
                            self.sheet = self.sheet.min(b.sheets.len().saturating_sub(1));
                            Content::Sheet(b)
                        }
                        Doc::Image(img) => {
                            self.image_size = Some((img.width, img.height));
                            self.image = Some(img);
                            Content::Image
                        }
                    });
                }
                Msg::Loaded(Err(e)) => self.state = State::Failed(e),
                Msg::Picture(ix, Ok(img)) => {
                    let tex = texture(ctx, &format!("doc-pic-{ix}"), &img);
                    self.pictures.insert(ix, Pic::Ready(tex));
                }
                Msg::Picture(ix, Err(_)) => {
                    self.pictures.insert(ix, Pic::Failed);
                }
            }
        }
        if let Some(pdf) = self.pdf.as_mut() {
            while let Ok(e) = pdf.renderer.events.try_recv() {
                match e {
                    tc_doc::pdf::Event::Opened { pages } => {
                        pdf.page = pdf.page.min(pages.len().saturating_sub(1));
                        pdf.pages = pages;
                        self.state = State::Ready(Content::Pdf);
                    }
                    tc_doc::pdf::Event::Page { index, scale, image } => {
                        // Still wanted at that scale (the view may have moved on).
                        if pdf.asked.get(&index) == Some(&scale) && image.width > 0 {
                            let tex = texture(ctx, &format!("pdf-page-{index}"), &image);
                            pdf.textures.insert(index, (scale, tex));
                        }
                    }
                    tc_doc::pdf::Event::Failed(e) => self.state = State::Failed(e),
                }
            }
        }
        if let Some(img) = self.image.take() {
            let tex = texture(ctx, "doc-image", &img);
            self.pictures.insert(0, Pic::Ready(tex));
        }
    }

    /// Asks for a page at a scale (pixels per point); nothing when it is already there or asked.
    pub fn want_page(&mut self, index: usize, scale: f32) {
        let Some(pdf) = self.pdf.as_mut() else { return };
        if pdf.asked.get(&index) == Some(&scale) {
            return;
        }
        pdf.asked.insert(index, scale);
        pdf.renderer.request(index, scale);
    }

    /// Lets go of pages far from the view (they are rendered again when they come back).
    pub fn keep_pages(&mut self, keep: std::ops::Range<usize>) {
        if let Some(pdf) = self.pdf.as_mut() {
            pdf.textures.retain(|i, _| keep.contains(i));
            pdf.asked.retain(|i, _| keep.contains(i));
        }
    }

    /// Starts decoding a picture of a Word or PowerPoint file when it comes into view.
    pub fn want_picture(&mut self, ix: usize, ctx: &egui::Context) {
        if self.pictures.contains_key(&ix) {
            return;
        }
        let bytes = match &self.state {
            State::Ready(Content::Text(d)) => d.images.get(ix).map(|p| p.bytes.clone()),
            State::Ready(Content::Slides(s)) => s.images.get(ix).map(|p| p.bytes.clone()),
            _ => None,
        };
        let Some(bytes) = bytes else { return };
        self.pictures.insert(ix, Pic::Pending);
        let (tx, c) = (self.tx.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("ternarycode-picture".into()).spawn(move || {
            let _ = tx.send(Msg::Picture(ix, tc_doc::img::decode(&bytes, 2048)));
            c.request_repaint();
        });
    }

    /// Lets go of pictures not on screen.
    pub fn keep_pictures(&mut self, shown: &std::collections::HashSet<usize>) {
        if matches!(self.kind, Kind::Image) {
            return;
        }
        self.pictures.retain(|i, p| shown.contains(i) || matches!(p, Pic::Pending));
    }

    /// Frees what can be made again (the editor was not shown for a while).
    pub fn release(&mut self) {
        if !matches!(self.kind, Kind::Image) {
            self.pictures.clear();
        }
        self.layout = None;
        if let Some(pdf) = self.pdf.as_mut() {
            pdf.textures.clear();
            pdf.asked.clear();
        }
    }

    /// Facts for the scripted UI checks: `kind=pdf state=ready pages=4 rendered=1 sheets=2
    /// rows=13 cols=5 slides=6 blocks=40 pictures=1`.
    pub fn facts(&self) -> String {
        let mut out = vec![format!("kind={}", format!("{:?}", self.kind).to_lowercase())];
        out.push(
            match self.state {
                State::Loading => "state=loading",
                State::Ready(_) => "state=ready",
                State::Failed(_) => "state=failed",
            }
            .to_string(),
        );
        if let Some(pdf) = &self.pdf {
            out.push(format!("pages={}", pdf.pages.len()));
            out.push(format!("rendered={}", pdf.textures.len()));
        }
        match &self.state {
            State::Ready(Content::Sheet(b)) => {
                out.push(format!("sheets={}", b.sheets.len()));
                if let Some(s) = b.sheets.get(self.sheet) {
                    out.push(format!("rows={}", s.rows));
                    out.push(format!("cols={}", s.cols));
                }
            }
            State::Ready(Content::Slides(s)) => out.push(format!("slides={}", s.slides.len())),
            State::Ready(Content::Text(d)) => out.push(format!("blocks={}", d.blocks.len())),
            _ => {}
        }
        out.push(format!("pictures={}", self.pictures.values().filter(|p| matches!(p, Pic::Ready(_))).count()));
        out.join(" ")
    }

    /// What the status line says about it ("PDF · 12 pages").
    pub fn summary(&self) -> String {
        use crate::i18n::tf;
        let kind = crate::i18n::t(self.kind.label());
        let more = match &self.state {
            State::Ready(Content::Pdf) => self.pdf.as_ref().map(|p| tf("{} pages", &[&p.pages.len().to_string()])),
            State::Ready(Content::Slides(s)) => Some(tf("{} slides", &[&s.slides.len().to_string()])),
            State::Ready(Content::Sheet(b)) => b.sheets.get(self.sheet).map(|s| {
                let mut t = tf("{} rows × {} columns", &[&s.rows.to_string(), &s.cols.to_string()]);
                if b.sheets.len() > 1 {
                    t = format!("{} · {t}", tf("{} sheets", &[&b.sheets.len().to_string()]));
                }
                t
            }),
            State::Ready(Content::Image) => self.image_size.map(|(w, h)| format!("{w} × {h} px")),
            _ => None,
        };
        match more {
            Some(m) => format!("{kind} · {m}"),
            None => kind.to_string(),
        }
    }
}

fn texture(ctx: &egui::Context, name: &str, img: &Rgba) -> egui::TextureHandle {
    let image = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}
