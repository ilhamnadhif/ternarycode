//! Word (.docx) and PowerPoint (.pptx) files as formatted text: paragraphs with their style
//! (headings, lists, quotes), bold/italic/underlined runs, links, tables and pictures. Not the
//! page layout: the viewer flows the text like a reader view.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// Pictures kept per document (the rest are left out).
const MAX_IMAGES: usize = 500;

/// A picture as stored in the file (PNG, JPEG, …) and its size in pixels: it is decoded only
/// when shown (with [`crate::img::decode`]), so a deck of fifty photos stays small in memory.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParaStyle {
    #[default]
    Normal,
    Title,
    Subtitle,
    Heading(u8),
    Quote,
    Code,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// A list item: its depth and the mark before it ("•", "1.", "a)").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListMark {
    pub level: u8,
    pub mark: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Text {
        text: String,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
        code: bool,
        link: bool,
        /// Font size in points, when the run sets one.
        size: Option<f32>,
        color: Option<[u8; 3]>,
    },
    /// A picture (an index into the document's images) and its size in points when known.
    Image(usize, Option<(f32, f32)>),
    Break,
    Tab,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Para {
    pub style: ParaStyle,
    pub align: Align,
    pub list: Option<ListMark>,
    pub spans: Vec<Span>,
}

impl Para {
    /// The paragraph's text (pictures and breaks left out).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for span in &self.spans {
            match span {
                Span::Text { text, .. } => s.push_str(text),
                Span::Tab => s.push('\t'),
                Span::Break => s.push('\n'),
                Span::Image(..) => {}
            }
        }
        s
    }

    fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| matches!(s, Span::Text { text, .. } if text.is_empty()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Para),
    /// Rows of cells, each cell its paragraphs.
    Table(Vec<Vec<Vec<Para>>>),
    PageBreak,
}

#[derive(Debug, Default)]
pub struct RichDoc {
    pub blocks: Vec<Block>,
    pub images: Vec<Picture>,
    /// Pictures left out (too many, or a format that cannot be read such as EMF).
    pub images_skipped: usize,
}

#[derive(Debug, Default)]
pub struct Slide {
    pub title: Option<String>,
    pub blocks: Vec<Block>,
}

#[derive(Debug)]
pub struct Slides {
    pub slides: Vec<Slide>,
    pub images: Vec<Picture>,
    pub images_skipped: usize,
    /// Width over height of the slides (16:9 when the file does not say).
    pub aspect: f32,
}

impl Default for Slides {
    fn default() -> Self {
        Self { slides: Vec::new(), images: Vec::new(), images_skipped: 0, aspect: 16.0 / 9.0 }
    }
}

// ----- zip and XML helpers -------------------------------------------------------------------

struct Package {
    zip: zip::ZipArchive<std::fs::File>,
}

impl Package {
    fn open(path: &Path) -> Result<Self, String> {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let zip = zip::ZipArchive::new(f).map_err(|_| "the file is not a readable Office document".to_string())?;
        Ok(Self { zip })
    }

    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        let mut f = self.zip.by_name(name.trim_start_matches('/')).ok()?;
        // A part this big is not text a person wrote (or a zip bomb).
        if f.size() > 256 * 1024 * 1024 {
            return None;
        }
        let mut out = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut out).ok()?;
        Some(out)
    }

    /// The relationships of a part: id → target path inside the package.
    fn rels(&mut self, part: &str) -> HashMap<String, String> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let rels_path = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let mut out = HashMap::new();
        let Some(xml) = self.read(&rels_path) else { return out };
        let mut r = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Empty(e) | Event::Start(e)) if local(e.name().as_ref()) == b"Relationship" => {
                    let (Some(id), Some(target)) = (attr(&e, b"Id"), attr(&e, b"Target")) else { continue };
                    if attr(&e, b"TargetMode").as_deref() == Some("External") {
                        continue;
                    }
                    out.insert(id, join(dir, &target));
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        out
    }
}

/// `target` relative to the folder `dir` of the package ("../media/a.png" from "ppt/slides").
fn join(dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The name without its namespace prefix (`w:p` → `p`).
fn local(name: &[u8]) -> &[u8] {
    name.iter().position(|b| *b == b':').map_or(name, |i| &name[i + 1..])
}

/// An attribute by local name.
#[allow(deprecated)] // `normalized_value` needs the document's XML version; 1.0 escapes suffice
fn attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == name).map(|a| {
        a.unescape_value()
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned())
    })
}

/// An on/off property (`<w:b/>`, `<w:b w:val="0"/>`).
fn on(e: &BytesStart) -> bool {
    !matches!(attr(e, b"val").as_deref(), Some("0" | "false" | "off" | "none"))
}

fn hex_color(v: &str) -> Option<[u8; 3]> {
    if v.len() != 6 || v.eq_ignore_ascii_case("auto") {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// Pictures of a document, read once each and shared by index.
#[derive(Default)]
struct Images {
    list: Vec<Picture>,
    by_part: HashMap<String, Option<usize>>,
    skipped: usize,
}

impl Images {
    fn get(&mut self, pkg: &mut Package, part: &str) -> Option<usize> {
        if let Some(i) = self.by_part.get(part) {
            return *i;
        }
        let ix = if self.list.len() >= MAX_IMAGES {
            None
        } else {
            pkg.read(part).and_then(|b| {
                let (width, height) = crate::img::dimensions(&b)?;
                self.list.push(Picture { bytes: std::sync::Arc::new(b), width, height });
                Some(self.list.len() - 1)
            })
        };
        if ix.is_none() {
            self.skipped += 1;
        }
        self.by_part.insert(part.to_string(), ix);
        ix
    }
}

/// `mc:AlternateContent` holds the same content more than once (a Word text box as DrawingML
/// in `mc:Choice` and again as VML in `mc:Fallback`): only the first branch is read.
#[derive(Default)]
struct Alternates {
    /// Per open `mc:AlternateContent`: whether one of its branches was read.
    read: Vec<bool>,
}

impl Alternates {
    /// At a start tag (not `<x/>`): whether the element is a branch after the one read, to skip.
    fn skip(&mut self, e: &BytesStart) -> bool {
        match local(e.name().as_ref()) {
            b"AlternateContent" => self.read.push(false),
            b"Choice" | b"Fallback" => {
                if let Some(read) = self.read.last_mut() {
                    return std::mem::replace(read, true);
                }
            }
            _ => {}
        }
        false
    }

    /// At the end of an `mc:AlternateContent`.
    fn end(&mut self) {
        self.read.pop();
    }
}

/// EMUs (English Metric Units) to points.
fn emu_pt(v: &str) -> Option<f32> {
    v.parse::<f64>().ok().map(|e| (e / 12700.0) as f32)
}

// ----- runs ----------------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct RunProps {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    code: bool,
    size: Option<f32>,
    color: Option<[u8; 3]>,
}

impl RunProps {
    fn span(&self, text: String, link: bool) -> Span {
        Span::Text {
            text,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            strike: self.strike,
            code: self.code,
            link,
            size: self.size,
            color: self.color,
        }
    }
}

fn push_text(spans: &mut Vec<Span>, props: &RunProps, link: bool, text: &str) {
    if text.is_empty() {
        return;
    }
    // Runs that look the same are joined (Word splits text into many runs).
    if let Some(Span::Text { text: last, bold, italic, underline, strike, code, link: l, size, color }) = spans.last_mut()
        && *bold == props.bold
        && *italic == props.italic
        && *underline == props.underline
        && *strike == props.strike
        && *code == props.code
        && *l == link
        && *size == props.size
        && *color == props.color
    {
        last.push_str(text);
        return;
    }
    spans.push(props.span(text.to_string(), link));
}

fn monospace_font(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["courier", "consolas", "mono", "menlo", "monaco", "code"].iter().any(|m| n.contains(m))
}

// ----- Word ----------------------------------------------------------------------------------

/// What a Word style means here, from its name.
fn style_of_name(name: &str) -> Option<ParaStyle> {
    let n = name.to_ascii_lowercase();
    if n == "title" {
        return Some(ParaStyle::Title);
    }
    if n == "subtitle" {
        return Some(ParaStyle::Subtitle);
    }
    if let Some(level) = n.strip_prefix("heading ").and_then(|l| l.trim().parse::<u8>().ok()) {
        return Some(ParaStyle::Heading(level.clamp(1, 6)));
    }
    if n.contains("quote") {
        return Some(ParaStyle::Quote);
    }
    if n.contains("code") || n.contains("preformatted") || n.contains("source") {
        return Some(ParaStyle::Code);
    }
    None
}

/// A Word style as read: its name, the style it is based on, its outline level.
#[derive(Default)]
struct WordStyle {
    name: String,
    based: Option<String>,
    outline: Option<u8>,
}

/// styles.xml: style id → what it means (following `basedOn`).
fn word_styles(xml: &[u8]) -> HashMap<String, ParaStyle> {
    let mut names: HashMap<String, WordStyle> = HashMap::new();
    let mut r = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut current: Option<String> = None;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"style" => {
                    current = attr(&e, b"styleId");
                    if let Some(id) = &current {
                        names.insert(id.clone(), WordStyle::default());
                    }
                }
                b"name" => {
                    if let (Some(id), Some(v)) = (&current, attr(&e, b"val"))
                        && let Some(entry) = names.get_mut(id)
                    {
                        entry.name = v;
                    }
                }
                b"basedOn" => {
                    if let (Some(id), Some(v)) = (&current, attr(&e, b"val"))
                        && let Some(entry) = names.get_mut(id)
                    {
                        entry.based = Some(v);
                    }
                }
                b"outlineLvl" => {
                    // Kept apart from the name: `w:name` usually comes first.
                    if let (Some(id), Some(v)) = (&current, attr(&e, b"val"))
                        && let Ok(l) = v.parse::<u8>()
                        && l < 9
                        && let Some(entry) = names.get_mut(id)
                    {
                        entry.outline = Some(l);
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"style" => current = None,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let mut out = HashMap::new();
    for id in names.keys() {
        let mut at = id.clone();
        for _ in 0..8 {
            let Some(style) = names.get(&at) else { break };
            // A paragraph style with an outline level is a heading, whatever its name.
            let heading = style.outline.map(|l| ParaStyle::Heading((l + 1).min(6)));
            if let Some(s) = heading.or_else(|| style_of_name(&style.name)).or_else(|| style_of_name(&at)) {
                out.insert(id.clone(), s);
                break;
            }
            match &style.based {
                Some(b) => at = b.clone(),
                None => break,
            }
        }
    }
    out
}

/// A level of a Word list: its number format ("bullet", "decimal", "lowerLetter", …), the text
/// with `%1` placeholders and the number it starts at.
#[derive(Clone, Debug, PartialEq)]
struct Level {
    fmt: String,
    text: String,
    start: usize,
}

impl Default for Level {
    fn default() -> Self {
        // Word writes `w:start` for numbered levels; without one, count from 1.
        Self { fmt: String::new(), text: String::new(), start: 1 }
    }
}

/// numbering.xml: (numId, level) → the level, started where the list says (`w:startOverride`).
fn word_numbering(xml: &[u8]) -> HashMap<(String, u8), Level> {
    let mut abstracts: HashMap<String, HashMap<u8, Level>> = HashMap::new();
    let mut nums: HashMap<String, String> = HashMap::new();
    let mut starts: HashMap<(String, u8), usize> = HashMap::new();
    let mut r = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let (mut abs, mut lvl, mut num): (Option<String>, Option<u8>, Option<String>) = (None, None, None);
    // Inside a `w:num`: the level its `w:lvlOverride` is about.
    let mut over: Option<u8> = None;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"abstractNum" => abs = attr(&e, b"abstractNumId"),
                b"lvl" => lvl = attr(&e, b"ilvl").and_then(|v| v.parse().ok()),
                b"numFmt" => {
                    if let (Some(a), Some(l), Some(v)) = (&abs, lvl, attr(&e, b"val")) {
                        abstracts.entry(a.clone()).or_default().entry(l).or_default().fmt = v;
                    }
                }
                b"lvlText" => {
                    if let (Some(a), Some(l), Some(v)) = (&abs, lvl, attr(&e, b"val")) {
                        abstracts.entry(a.clone()).or_default().entry(l).or_default().text = v;
                    }
                }
                b"start" if abs.is_some() => {
                    if let (Some(a), Some(l), Some(v)) = (&abs, lvl, attr(&e, b"val").and_then(|v| v.parse().ok())) {
                        abstracts.entry(a.clone()).or_default().entry(l).or_default().start = v;
                    }
                }
                b"num" => num = attr(&e, b"numId"),
                b"abstractNumId" => {
                    if let (Some(n), Some(v)) = (&num, attr(&e, b"val")) {
                        nums.insert(n.clone(), v);
                    }
                }
                b"lvlOverride" => over = attr(&e, b"ilvl").and_then(|v| v.parse().ok()),
                // A restarted list ("Restart at 1", "Set numbering value") is a new `w:num`
                // with its own start; a `w:start` in an overriding `w:lvl` says the same.
                b"startOverride" | b"start" => {
                    if let (Some(n), Some(l), Some(v)) = (&num, over, attr(&e, b"val").and_then(|v| v.parse().ok())) {
                        starts.entry((n.clone(), l)).or_insert(v);
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"abstractNum" => abs = None,
                b"lvl" => lvl = None,
                b"num" => num = None,
                b"lvlOverride" => over = None,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let mut out = HashMap::new();
    for (n, a) in nums {
        if let Some(levels) = abstracts.get(&a) {
            for (l, f) in levels {
                let mut f = f.clone();
                if let Some(start) = starts.get(&(n.clone(), *l)) {
                    f.start = *start;
                }
                out.insert((n.clone(), *l), f);
            }
        }
    }
    out
}

/// A list counter as text in a Word number format.
fn number_text(n: usize, fmt: &str) -> String {
    let letter = |n: usize, upper: bool| {
        let base = if upper { b'A' } else { b'a' };
        let mut s = String::new();
        let reps = (n - 1) / 26 + 1;
        for _ in 0..reps {
            s.push((base + ((n - 1) % 26) as u8) as char);
        }
        s
    };
    let roman = |mut n: usize, upper: bool| {
        let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
        let mut s = String::new();
        for (v, r) in table {
            while n >= v {
                s.push_str(r);
                n -= v;
            }
        }
        if upper { s.to_uppercase() } else { s }
    };
    match fmt {
        "lowerLetter" => letter(n.max(1), false),
        "upperLetter" => letter(n.max(1), true),
        "lowerRoman" => roman(n.max(1), false),
        "upperRoman" => roman(n.max(1), true),
        _ => n.to_string(),
    }
}

/// Counters of numbered lists, per list and level.
#[derive(Default)]
struct Counters {
    counts: HashMap<(String, u8), usize>,
}

impl Counters {
    fn mark(&mut self, numbering: &HashMap<(String, u8), Level>, num: &str, level: u8) -> Option<String> {
        if num == "0" {
            return None;
        }
        let Level { fmt, text, start } =
            numbering.get(&(num.to_string(), level)).cloned().unwrap_or_else(|| Level { fmt: "bullet".into(), ..Default::default() });
        if fmt == "bullet" || fmt == "none" {
            return Some(match level % 3 {
                0 => "•",
                1 => "◦",
                _ => "▪",
            }
            .to_string());
        }
        // Each list and level counts on from its start.
        let n = *self.counts.entry((num.to_string(), level)).and_modify(|c| *c += 1).or_insert(start);
        // Deeper levels start again under a new item.
        self.counts.retain(|(k, l), _| k != num || *l <= level);
        let mut out = if text.is_empty() { format!("%{}.", level + 1) } else { text };
        for l in 0..=level {
            let f = numbering.get(&(num.to_string(), l));
            let v = if l == level { n } else { self.counts.get(&(num.to_string(), l)).copied().unwrap_or(f.map_or(1, |f| f.start)) };
            let f = f.map_or("decimal", |f| f.fmt.as_str());
            out = out.replace(&format!("%{}", l + 1), &number_text(v, f));
        }
        Some(out)
    }
}

/// Where a Word paragraph's content is being collected (paragraphs nest inside text boxes).
#[derive(Default)]
struct WordPara {
    para: Para,
    num: Option<(String, u8)>,
    /// Tables open when the paragraph started (one opened since is in its text box).
    tables: usize,
    /// What its text boxes hold, shown after it.
    boxes: Vec<Block>,
}

/// Where table content is being collected.
#[derive(Default)]
struct WordTable {
    rows: Vec<Vec<Vec<Para>>>,
}

pub fn read_docx(path: &Path) -> Result<RichDoc, String> {
    let mut pkg = Package::open(path)?;
    let main = find_main_part(&mut pkg, "word/document.xml");
    let xml = pkg.read(&main).ok_or("the document has no text part")?;
    let styles = pkg.read("word/styles.xml").map(|x| word_styles(&x)).unwrap_or_default();
    let numbering = pkg.read("word/numbering.xml").map(|x| word_numbering(&x)).unwrap_or_default();
    let rels = pkg.rels(&main);
    let mut images = Images::default();
    let mut counters = Counters::default();

    let mut blocks: Vec<Block> = Vec::new();
    let mut paras: Vec<WordPara> = Vec::new();
    let mut tables: Vec<WordTable> = Vec::new();
    let mut props = RunProps::default();
    let mut in_rpr = false;
    let mut in_ppr = false;
    let mut link_depth = 0usize;
    let mut text_mode: Option<&'static str> = None; // inside w:t / w:instrText (ignored)
    let mut deleted = 0usize;
    let mut drawing_size: Option<(f32, f32)> = None;
    let mut alternates = Alternates::default();

    let mut r = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let mut skipped = Vec::new();
    while let Ok(ev) = r.read_event_into(&mut buf) {
        if let Event::Start(e) = &ev
            && alternates.skip(e)
        {
            let _ = r.read_to_end_into(e.name(), &mut skipped);
            skipped.clear();
            buf.clear();
            continue;
        }
        // `<x/>` has no end event: state that an end would undo is only set by a start.
        let is_empty = matches!(ev, Event::Empty(_));
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                let name = local(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"p" => paras.push(WordPara { tables: tables.len(), ..Default::default() }),
                    b"pPr" if !is_empty => in_ppr = true,
                    b"rPr" if !is_empty => {
                        in_rpr = true;
                        if !in_ppr {
                            props = RunProps::default();
                        }
                    }
                    b"r" => props = RunProps::default(),
                    b"pStyle" if in_ppr => {
                        if let (Some(p), Some(v)) = (paras.last_mut(), attr(&e, b"val"))
                            && let Some(s) = styles.get(&v).copied().or_else(|| style_of_name(&v))
                        {
                            p.para.style = s;
                        }
                    }
                    b"jc" if in_ppr && !in_rpr => {
                        if let Some(p) = paras.last_mut() {
                            p.para.align = match attr(&e, b"val").as_deref() {
                                Some("center") => Align::Center,
                                Some("right" | "end") => Align::Right,
                                Some("both" | "distribute") => Align::Justify,
                                _ => Align::Left,
                            };
                        }
                    }
                    b"ilvl" if in_ppr => {
                        if let Some(p) = paras.last_mut() {
                            let l = attr(&e, b"val").and_then(|v| v.parse().ok()).unwrap_or(0);
                            p.num.get_or_insert((String::new(), 0)).1 = l;
                        }
                    }
                    b"numId" if in_ppr => {
                        if let (Some(p), Some(v)) = (paras.last_mut(), attr(&e, b"val")) {
                            p.num.get_or_insert((String::new(), 0)).0 = v;
                        }
                    }
                    b"b" if in_rpr && !in_ppr => props.bold = on(&e),
                    b"i" if in_rpr && !in_ppr => props.italic = on(&e),
                    b"u" if in_rpr && !in_ppr => props.underline = attr(&e, b"val").as_deref() != Some("none"),
                    b"strike" | b"dstrike" if in_rpr && !in_ppr => props.strike = on(&e),
                    b"sz" if in_rpr && !in_ppr => props.size = attr(&e, b"val").and_then(|v| v.parse::<f32>().ok()).map(|h| h / 2.0),
                    b"color" if in_rpr && !in_ppr => props.color = attr(&e, b"val").and_then(|v| hex_color(&v)),
                    b"rFonts" if in_rpr && !in_ppr => {
                        props.code = attr(&e, b"ascii").is_some_and(|f| monospace_font(&f));
                    }
                    b"hyperlink" if !is_empty => link_depth += 1,
                    b"del" if !is_empty => deleted += 1,
                    b"t" if deleted == 0 && !is_empty => text_mode = Some("t"),
                    b"instrText" | b"delText" if !is_empty => text_mode = Some("skip"),
                    b"tab" if !in_ppr => {
                        if let Some(p) = paras.last_mut() {
                            p.para.spans.push(Span::Tab);
                        }
                    }
                    b"br" | b"cr" => {
                        let page = attr(&e, b"type").as_deref() == Some("page");
                        if page {
                            flush_para_break(&mut paras, &mut blocks, &mut tables);
                        } else if let Some(p) = paras.last_mut() {
                            p.para.spans.push(Span::Break);
                        }
                    }
                    b"extent" => {
                        drawing_size = match (attr(&e, b"cx").and_then(|v| emu_pt(&v)), attr(&e, b"cy").and_then(|v| emu_pt(&v))) {
                            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
                            _ => drawing_size,
                        };
                    }
                    b"blip" => {
                        if let Some(id) = attr(&e, b"embed")
                            && let Some(part) = rels.get(&id)
                            && let Some(ix) = images.get(&mut pkg, part)
                            && let Some(p) = paras.last_mut()
                        {
                            p.para.spans.push(Span::Image(ix, drawing_size.take()));
                        }
                    }
                    b"tbl" => tables.push(WordTable::default()),
                    b"tr" => {
                        if let Some(t) = tables.last_mut() {
                            t.rows.push(Vec::new());
                        }
                    }
                    b"tc" => {
                        if let Some(row) = tables.last_mut().and_then(|t| t.rows.last_mut()) {
                            row.push(Vec::new());
                        }
                    }
                    _ => {}
                }
                // `<w:p/>`: an empty paragraph, ended at once.
                if is_empty && name.as_slice() == b"p" {
                    finish_para(&mut paras, &mut blocks, &mut tables, &numbering, &mut counters);
                }
            }
            Event::Text(t) => {
                if text_mode == Some("t")
                    && let Some(p) = paras.last_mut()
                {
                    let s = t.decode().map(|s| s.into_owned()).unwrap_or_default();
                    let s = quick_xml::escape::unescape(&s).map(|c| c.into_owned()).unwrap_or(s);
                    push_text(&mut p.para.spans, &props, link_depth > 0, &s);
                }
            }
            Event::GeneralRef(r) => {
                if text_mode == Some("t")
                    && let Some(p) = paras.last_mut()
                {
                    let name = String::from_utf8_lossy(&r).into_owned();
                    let s = resolve_entity(&name);
                    push_text(&mut p.para.spans, &props, link_depth > 0, &s);
                }
            }
            Event::End(e) => match local(e.name().as_ref()) {
                b"p" => finish_para(&mut paras, &mut blocks, &mut tables, &numbering, &mut counters),
                b"pPr" => in_ppr = false,
                b"rPr" => in_rpr = false,
                b"hyperlink" => link_depth = link_depth.saturating_sub(1),
                b"del" => deleted = deleted.saturating_sub(1),
                b"t" | b"instrText" | b"delText" => text_mode = None,
                b"drawing" | b"pict" => drawing_size = None,
                b"AlternateContent" => alternates.end(),
                b"tbl" => {
                    if let Some(t) = tables.pop() {
                        let block = Block::Table(t.rows);
                        match paras.last_mut() {
                            // A table in a text box: shown after the paragraph holding it.
                            Some(holder) if tables.len() <= holder.tables => holder.boxes.push(block),
                            // A table inside a table cell: its text goes into that cell.
                            _ => match tables.last_mut().and_then(|t| t.rows.last_mut()).and_then(|r| r.last_mut()) {
                                Some(cell) => into_cell(cell, block),
                                None => blocks.push(block),
                            },
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // Leading and trailing empty paragraphs are only spacing.
    while matches!(blocks.last(), Some(Block::Para(p)) if p.is_empty()) || matches!(blocks.last(), Some(Block::PageBreak)) {
        blocks.pop();
    }
    Ok(RichDoc { blocks, images: images.list, images_skipped: images.skipped })
}

fn resolve_entity(name: &str) -> String {
    match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        n => {
            let code = n
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()));
            code.and_then(char::from_u32).map(String::from).unwrap_or_default()
        }
    }
}

/// A page break: what came before it is a paragraph of its own.
fn flush_para_break(paras: &mut [WordPara], blocks: &mut Vec<Block>, tables: &mut [WordTable]) {
    if tables.is_empty() && paras.len() == 1 {
        if let Some(p) = paras.last_mut() {
            if !p.para.is_empty() {
                let next = Para { style: p.para.style, align: p.para.align, ..Default::default() };
                let done = std::mem::replace(&mut p.para, next);
                blocks.push(Block::Para(done));
            }
            // Its text boxes so far stay with it, before the break.
            blocks.append(&mut p.boxes);
        }
        blocks.push(Block::PageBreak);
    }
}

/// A block as paragraphs of a table cell (a table in it is flattened into it).
fn into_cell(cell: &mut Vec<Para>, block: Block) {
    match block {
        Block::Para(p) => cell.push(p),
        Block::Table(rows) => cell.extend(rows.into_iter().flatten().flatten()),
        Block::PageBreak => {}
    }
}

fn finish_para(
    paras: &mut Vec<WordPara>,
    blocks: &mut Vec<Block>,
    tables: &mut [WordTable],
    numbering: &HashMap<(String, u8), Level>,
    counters: &mut Counters,
) {
    let Some(mut wp) = paras.pop() else { return };
    if let Some((num, level)) = &wp.num
        && !num.is_empty()
        && let Some(mark) = counters.mark(numbering, num, *level)
    {
        wp.para.list = Some(ListMark { level: *level, mark });
    }
    let (para, boxes) = (wp.para, wp.boxes);
    // A text box in another paragraph (not in a table of its own): shown after the one holding
    // it, with what its own text boxes hold.
    if let Some(holder) = paras.last_mut()
        && tables.len() <= holder.tables
    {
        holder.boxes.push(Block::Para(para));
        holder.boxes.extend(boxes);
        return;
    }
    // Inside a table cell: it belongs there.
    if let Some(cell) = tables.last_mut().and_then(|t| t.rows.last_mut()).and_then(|r| r.last_mut()) {
        cell.push(para);
        for b in boxes {
            into_cell(cell, b);
        }
        return;
    }
    // Several empty paragraphs in a row are spacing: keep one (and none at the start).
    let spacing = para.is_empty() && (blocks.is_empty() || matches!(blocks.last(), Some(Block::Para(p)) if p.is_empty()));
    if !spacing {
        blocks.push(Block::Para(para));
    }
    blocks.extend(boxes);
}

/// The main part a package's root relationships point at (the usual name otherwise).
fn find_main_part(pkg: &mut Package, usual: &str) -> String {
    // Root relationships are in `_rels/.rels`: find the officeDocument target.
    if let Some(xml) = pkg.read("_rels/.rels") {
        let mut r = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Empty(e) | Event::Start(e)) if local(e.name().as_ref()) == b"Relationship" => {
                    if attr(&e, b"Type").is_some_and(|t| t.ends_with("/officeDocument"))
                        && let Some(t) = attr(&e, b"Target")
                    {
                        return t.trim_start_matches('/').to_string();
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
    }
    usual.to_string()
}

// ----- PowerPoint ----------------------------------------------------------------------------

/// A shape's text and pictures, and where it sits on the slide (to order shapes top to bottom).
#[derive(Default)]
struct Shape {
    at: (f32, f32),
    title: bool,
    paras: Vec<Para>,
    table: Option<Vec<Vec<Vec<Para>>>>,
}

fn read_slide(pkg: &mut Package, part: &str, images: &mut Images) -> Slide {
    let Some(xml) = pkg.read(part) else { return Slide::default() };
    let rels = pkg.rels(part);
    let mut shapes: Vec<Shape> = Vec::new();
    let mut shape: Option<Shape> = None;
    let mut para: Option<Para> = None;
    let mut props = RunProps::default();
    let mut in_rpr = false;
    // Elements open inside the run properties, each whether it is an `a:solidFill`: the run's
    // colour is the fill right under them, not a highlight's or an outline's.
    let mut rpr_open: Vec<bool> = Vec::new();
    let mut in_t = false;
    let mut in_xfrm = false;
    let mut table: Option<Vec<Vec<Vec<Para>>>> = None;
    let mut r = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    let mut skipped = Vec::new();
    while let Ok(ev) = r.read_event_into(&mut buf) {
        // The slide's background picture is no content.
        if let Event::Start(e) = &ev
            && local(e.name().as_ref()) == b"bg"
        {
            let _ = r.read_to_end_into(e.name(), &mut skipped);
            skipped.clear();
            buf.clear();
            continue;
        }
        let is_empty = matches!(ev, Event::Empty(_));
        let opened = match &ev {
            Event::Start(e) if in_rpr => Some(local(e.name().as_ref()) == b"solidFill"),
            Event::End(_) if in_rpr => {
                rpr_open.pop();
                None
            }
            _ => None,
        };
        match ev {
            Event::Start(e) | Event::Empty(e) => match local(e.name().as_ref()) {
                b"sp" | b"pic" | b"graphicFrame" | b"cxnSp" => shape = Some(Shape::default()),
                b"ph" => {
                    if let Some(s) = shape.as_mut() {
                        s.title = matches!(attr(&e, b"type").as_deref(), Some("title" | "ctrTitle"));
                    }
                }
                b"xfrm" => in_xfrm = !is_empty,
                b"off" if in_xfrm => {
                    if let Some(s) = shape.as_mut() {
                        let x = attr(&e, b"x").and_then(|v| emu_pt(&v)).unwrap_or(0.0);
                        let y = attr(&e, b"y").and_then(|v| emu_pt(&v)).unwrap_or(0.0);
                        s.at = (y, x);
                    }
                }
                b"tbl" => table = Some(Vec::new()),
                b"tr" => {
                    if let Some(t) = table.as_mut() {
                        t.push(Vec::new());
                    }
                }
                b"tc" => {
                    if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                        row.push(Vec::new());
                    }
                }
                b"p" => {
                    let p = Para::default();
                    if is_empty {
                        push_slide_para(&mut shape, &mut table, p);
                    } else {
                        para = Some(p);
                    }
                }
                b"pPr" => {
                    if let Some(p) = para.as_mut() {
                        let level = attr(&e, b"lvl").and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
                        p.align = match attr(&e, b"algn").as_deref() {
                            Some("ctr") => Align::Center,
                            Some("r") => Align::Right,
                            Some("just") => Align::Justify,
                            _ => Align::Left,
                        };
                        if level > 0 {
                            p.list = Some(ListMark { level, mark: "•".into() });
                        }
                    }
                }
                b"buChar" => {
                    if let (Some(p), Some(c)) = (para.as_mut(), attr(&e, b"char")) {
                        let level = p.list.as_ref().map_or(0, |l| l.level);
                        p.list = Some(ListMark { level, mark: if c.trim().is_empty() { "•".into() } else { c } });
                    }
                }
                b"buAutoNum" => {
                    if let Some(p) = para.as_mut() {
                        let level = p.list.as_ref().map_or(0, |l| l.level);
                        p.list = Some(ListMark { level, mark: "1.".into() });
                    }
                }
                b"buNone" => {
                    if let Some(p) = para.as_mut() {
                        p.list = None;
                    }
                }
                b"r" | b"fld" => props = RunProps::default(),
                b"rPr" | b"defRPr" => {
                    in_rpr = !is_empty;
                    rpr_open.clear();
                    props.bold = attr(&e, b"b").is_some_and(|v| v == "1" || v == "true");
                    props.italic = attr(&e, b"i").is_some_and(|v| v == "1" || v == "true");
                    props.underline = attr(&e, b"u").is_some_and(|v| v != "none");
                    props.strike = attr(&e, b"strike").is_some_and(|v| v != "noStrike");
                    props.size = attr(&e, b"sz").and_then(|v| v.parse::<f32>().ok()).map(|s| s / 100.0);
                }
                b"srgbClr" if in_rpr && rpr_open == [true] => props.color = attr(&e, b"val").and_then(|v| hex_color(&v)),
                b"latin" if in_rpr => props.code = attr(&e, b"typeface").is_some_and(|f| monospace_font(&f)),
                b"t" => in_t = !is_empty,
                b"br" => {
                    if let Some(p) = para.as_mut() {
                        p.spans.push(Span::Break);
                    }
                }
                b"blip" => {
                    if let Some(id) = attr(&e, b"embed")
                        && let Some(target) = rels.get(&id)
                        && let Some(ix) = images.get(pkg, target)
                    {
                        let p = Para { spans: vec![Span::Image(ix, None)], ..Default::default() };
                        match shape.as_mut() {
                            Some(s) => s.paras.push(p),
                            None => shapes.push(Shape { paras: vec![p], ..Default::default() }),
                        }
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_t => {
                if let Some(p) = para.as_mut() {
                    let s = t.decode().map(|s| s.into_owned()).unwrap_or_default();
                    push_text(&mut p.spans, &props, false, &s);
                }
            }
            Event::GeneralRef(g) if in_t => {
                if let Some(p) = para.as_mut() {
                    let s = resolve_entity(&String::from_utf8_lossy(&g));
                    push_text(&mut p.spans, &props, false, &s);
                }
            }
            Event::End(e) => match local(e.name().as_ref()) {
                b"t" => in_t = false,
                b"rPr" | b"defRPr" => in_rpr = false,
                b"xfrm" => in_xfrm = false,
                b"p" => {
                    if let Some(p) = para.take() {
                        push_slide_para(&mut shape, &mut table, p);
                    }
                }
                b"tbl" => {
                    if let (Some(t), Some(s)) = (table.take(), shape.as_mut()) {
                        s.table = Some(t);
                    }
                }
                b"sp" | b"pic" | b"graphicFrame" | b"cxnSp" => {
                    if let Some(s) = shape.take() {
                        shapes.push(s);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        if let Some(fill) = opened {
            rpr_open.push(fill);
        }
        buf.clear();
    }
    // Top to bottom, then left to right (rounded, so shapes on one line keep their order).
    shapes.sort_by(|a, b| {
        let key = |s: &Shape| ((s.at.0 / 12.0).round() as i64, s.at.1 as i64);
        key(a).cmp(&key(b))
    });
    let mut slide = Slide::default();
    for s in shapes {
        if s.title && slide.title.is_none() {
            let text: Vec<String> = s.paras.iter().map(Para::text).filter(|t| !t.trim().is_empty()).collect();
            if !text.is_empty() {
                slide.title = Some(text.join(" "));
                continue;
            }
        }
        if let Some(t) = s.table {
            slide.blocks.push(Block::Table(t));
        }
        for p in s.paras {
            if !p.is_empty() {
                slide.blocks.push(Block::Para(p));
            }
        }
    }
    slide
}

fn push_slide_para(shape: &mut Option<Shape>, table: &mut Option<Vec<Vec<Vec<Para>>>>, p: Para) {
    if let Some(cell) = table.as_mut().and_then(|t| t.last_mut()).and_then(|r| r.last_mut()) {
        cell.push(p);
    } else if let Some(s) = shape.as_mut() {
        s.paras.push(p);
    }
}

pub fn read_pptx(path: &Path) -> Result<Slides, String> {
    let mut pkg = Package::open(path)?;
    let main = find_main_part(&mut pkg, "ppt/presentation.xml");
    let xml = pkg.read(&main).ok_or("the presentation has no slides part")?;
    let rels = pkg.rels(&main);
    let mut order: Vec<String> = Vec::new();
    let mut aspect = 16.0 / 9.0;
    let mut r = Reader::from_reader(xml.as_slice());
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) => match local(e.name().as_ref()) {
                b"sldId" => {
                    if let Some(part) = attr_ns_r(&e).and_then(|id| rels.get(&id).cloned()) {
                        order.push(part);
                    }
                }
                b"sldSz" => {
                    if let (Some(cx), Some(cy)) = (attr(&e, b"cx").and_then(|v| v.parse::<f32>().ok()), attr(&e, b"cy").and_then(|v| v.parse::<f32>().ok()))
                        && cy > 0.0
                    {
                        aspect = cx / cy;
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let mut images = Images::default();
    let slides = order.iter().map(|part| read_slide(&mut pkg, part, &mut images)).collect();
    Ok(Slides { slides, images: images.list, images_skipped: images.skipped, aspect })
}

/// A slide id's relationship (`r:id`, not the plain `id` attribute).
fn attr_ns_r(e: &BytesStart) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref().ends_with(b":id")).map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn write_zip(path: &Path, files: &[(&str, &str)]) {
        let files: Vec<(&str, &[u8])> = files.iter().map(|(name, body)| (*name, body.as_bytes())).collect();
        write_zip_bytes(path, &files);
    }

    fn write_zip_bytes(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(body).unwrap();
        }
        z.finish().unwrap();
    }

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    #[test]
    fn word_documents_keep_their_structure() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.docx");
        let doc = format!(
            r#"<?xml version="1.0"?><w:document {W}><w:body>
<w:p><w:pPr><w:pStyle w:val="Judul1"/></w:pPr><w:r><w:t>Intro</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Plain </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t xml:space="preserve"> &amp; more</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>first</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>second</w:t></w:r></w:p>
<w:p/><w:p/>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:del><w:r><w:delText>gone</w:delText></w:r></w:del><w:t>kept</w:t></w:r></w:p>
</w:body></w:document>"#
        );
        write_zip(
            &p,
            &[
                ("word/document.xml", &doc),
                (
                    "word/styles.xml",
                    &format!(r#"<w:styles {W}><w:style w:styleId="Judul1"><w:name w:val="heading 1"/></w:style></w:styles>"#),
                ),
                (
                    "word/numbering.xml",
                    &format!(
                        r#"<w:numbering {W}><w:abstractNum w:abstractNumId="5"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="5"/></w:num></w:numbering>"#
                    ),
                ),
            ],
        );
        let d = read_docx(&p).unwrap();
        let paras: Vec<&Para> = d.blocks.iter().filter_map(|b| if let Block::Para(p) = b { Some(p) } else { None }).collect();
        assert_eq!(paras[0].style, ParaStyle::Heading(1));
        assert_eq!(paras[0].text(), "Intro");
        assert_eq!(paras[1].text(), "Plain bold & more");
        assert!(matches!(&paras[1].spans[1], Span::Text { bold: true, text, .. } if text == "bold"));
        assert_eq!(paras[2].list.as_ref().unwrap().mark, "1.");
        assert_eq!(paras[3].list.as_ref().unwrap().mark, "2.");
        // Two empty paragraphs keep one; the table; the deleted text is gone.
        assert!(paras[4].is_empty());
        assert!(d.blocks.iter().any(|b| matches!(b, Block::Table(rows) if rows[0][1][0].text() == "B1")));
        assert_eq!(paras.last().unwrap().text(), "kept");
    }

    #[test]
    fn slides_come_in_order_with_their_titles() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.pptx");
        let ns = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let slide = |title: &str, body: &str| {
            format!(
                r#"<p:sld {ns}><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="2000000"/></a:xfrm></p:spPr><p:txBody><a:p><a:pPr lvl="1"/><a:r><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="100"/></a:xfrm></p:spPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#
            )
        };
        write_zip(
            &p,
            &[
                (
                    "ppt/presentation.xml",
                    &format!(r#"<p:presentation {ns}><p:sldIdLst><p:sldId id="257" r:id="rId3"/><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#),
                ),
                (
                    "ppt/_rels/presentation.xml.rels",
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Target="slides/slide1.xml"/><Relationship Id="rId3" Target="slides/slide2.xml"/></Relationships>"#,
                ),
                ("ppt/slides/slide1.xml", &slide("One", "a")),
                ("ppt/slides/slide2.xml", &slide("Two", "b")),
            ],
        );
        let s = read_pptx(&p).unwrap();
        assert_eq!(s.slides.len(), 2);
        assert_eq!(s.slides[0].title.as_deref(), Some("Two"));
        assert_eq!(s.slides[1].title.as_deref(), Some("One"));
        let Block::Para(body) = &s.slides[0].blocks[0] else { panic!() };
        assert_eq!(body.text(), "b");
        assert_eq!(body.list.as_ref().unwrap().level, 1);
        assert!((s.aspect - 4.0 / 3.0).abs() < 0.01);
    }

    /// The paragraphs' texts and tables of a document, in order.
    fn outline(d: &RichDoc) -> Vec<String> {
        d.blocks
            .iter()
            .map(|b| match b {
                Block::Para(p) => p.text(),
                Block::Table(rows) => format!("[{}]", rows.iter().flatten().flatten().map(Para::text).collect::<Vec<_>>().join("|")),
                Block::PageBreak => "---".into(),
            })
            .collect()
    }

    #[test]
    fn a_text_box_is_read_once_after_its_paragraph() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("box.docx");
        let boxed = r#"<w:txbxContent><w:p><w:r><w:t>In the box</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>boxed cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:txbxContent>"#;
        let doc = format!(
            r#"<w:document {W} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml"><w:body>
<w:p><w:r><w:t>Before</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Holder </w:t></w:r><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wps:wsp><wps:txbx>{boxed}</wps:txbx></wps:wsp></w:drawing></mc:Choice><mc:Fallback><w:pict><v:rect><v:textbox>{boxed}</v:textbox></v:rect></w:pict></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>text</w:t></w:r></w:p>
<w:p><w:r><mc:AlternateContent><mc:Fallback><w:t>Only a fallback</w:t></mc:Fallback></mc:AlternateContent></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t xml:space="preserve">Cell </w:t></w:r><w:r><w:pict><v:textbox><w:txbxContent><w:p><w:r><w:t>cell box</w:t></w:r></w:p></w:txbxContent></v:textbox></w:pict></w:r><w:r><w:t>holder</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>After</w:t></w:r></w:p>
</w:body></w:document>"#
        );
        write_zip(&p, &[("word/document.xml", &doc)]);
        let d = read_docx(&p).unwrap();
        assert_eq!(
            outline(&d),
            ["Before", "Holder text", "In the box", "[boxed cell]", "Only a fallback", "[Cell holder|cell box]", "After"]
        );
    }

    #[test]
    fn word_lists_start_where_they_say_and_outline_levels_make_headings() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("list.docx");
        let item = |num: u8, level: u8, text: &str| {
            format!(r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num}"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
        };
        let doc = format!(
            r#"<w:document {W}><w:body><w:p><w:pPr><w:pStyle w:val="Bagian"/></w:pPr><w:r><w:t>Part</w:t></w:r></w:p>{}{}{}{}{}</w:body></w:document>"#,
            item(1, 0, "a"),
            item(1, 1, "b"),
            item(1, 0, "c"),
            item(2, 0, "d"),
            item(2, 0, "e"),
        );
        let numbering = format!(
            r#"<w:numbering {W}><w:abstractNum w:abstractNumId="7"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl><w:lvl w:ilvl="1"><w:start w:val="2"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2)"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="7"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="7"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="10"/></w:lvlOverride></w:num></w:numbering>"#
        );
        // The name first, as Word writes it, then the outline level.
        let styles = format!(
            r#"<w:styles {W}><w:style w:type="paragraph" w:styleId="Bagian"><w:name w:val="Judul Bagian"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style></w:styles>"#
        );
        write_zip(&p, &[("word/document.xml", &doc), ("word/numbering.xml", &numbering), ("word/styles.xml", &styles)]);
        let d = read_docx(&p).unwrap();
        let paras: Vec<&Para> = d.blocks.iter().filter_map(|b| if let Block::Para(p) = b { Some(p) } else { None }).collect();
        assert_eq!(paras[0].style, ParaStyle::Heading(2));
        let marks: Vec<&str> = paras[1..].iter().map(|p| p.list.as_ref().unwrap().mark.as_str()).collect();
        assert_eq!(marks, ["3.", "3.b)", "4.", "10.", "11."]);
    }

    #[test]
    fn slide_backgrounds_and_highlights_are_not_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bg.pptx");
        let ns = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let slide = format!(
            r#"<p:sld {ns}><p:cSld><p:bg><p:bgPr><a:blipFill><a:blip r:embed="rId9"/></a:blipFill></p:bgPr></p:bg><p:spTree>
<p:sp><p:txBody><a:p><a:r><a:rPr><a:ln><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln><a:solidFill><a:srgbClr val="00FF00"><a:lumMod val="75000"/></a:srgbClr></a:solidFill><a:highlight><a:srgbClr val="FFFF00"/></a:highlight></a:rPr><a:t>green</a:t></a:r><a:r><a:rPr><a:highlight><a:srgbClr val="FFFF00"/></a:highlight></a:rPr><a:t> plain</a:t></a:r></a:p></p:txBody></p:sp>
<p:pic><p:blipFill><a:blip r:embed="rId9"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="9000000"/></a:xfrm></p:spPr></p:pic>
</p:spTree></p:cSld></p:sld>"#
        );
        let png = crate::img::tests::tiny_png();
        write_zip_bytes(
            &p,
            &[
                (
                    "ppt/presentation.xml",
                    format!(r#"<p:presentation {ns}><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#).as_bytes(),
                ),
                (
                    "ppt/_rels/presentation.xml.rels",
                    br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Target="slides/slide1.xml"/></Relationships>"#,
                ),
                ("ppt/slides/slide1.xml", slide.as_bytes()),
                (
                    "ppt/slides/_rels/slide1.xml.rels",
                    br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Target="../media/bg.png"/></Relationships>"#,
                ),
                ("ppt/media/bg.png", &png),
            ],
        );
        let s = read_pptx(&p).unwrap();
        // No slide size: 16:9, as when nothing is read.
        assert!((s.aspect - 16.0 / 9.0).abs() < 0.01);
        assert!((Slides::default().aspect - 16.0 / 9.0).abs() < 0.01);
        let blocks = &s.slides[0].blocks;
        assert_eq!(blocks.len(), 2, "{blocks:?}");
        let Block::Para(text) = &blocks[0] else { panic!() };
        assert!(matches!(&text.spans[0], Span::Text { text, color: Some([0, 255, 0]), .. } if text == "green"), "{:?}", text.spans);
        assert!(matches!(&text.spans[1], Span::Text { text, color: None, .. } if text == " plain"), "{:?}", text.spans);
        let Block::Para(pic) = &blocks[1] else { panic!() };
        assert!(matches!(pic.spans[..], [Span::Image(0, _)]));
    }

    #[test]
    fn package_paths_resolve() {
        assert_eq!(join("ppt/slides", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(join("word", "media/x.jpeg"), "word/media/x.jpeg");
        assert_eq!(join("word", "/word/media/x.png"), "word/media/x.png");
        assert_eq!(number_text(3, "lowerLetter"), "c");
        assert_eq!(number_text(4, "upperRoman"), "IV");
    }
}
