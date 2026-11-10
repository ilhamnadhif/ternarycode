//! Spreadsheets (xlsx, xlsm, xlsb, xls, ods) and CSV as tables of display text.

use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};

/// Cells kept per sheet; the rest is cut (the viewer says so).
pub const MAX_CELLS: usize = 2_000_000;
/// Rows looked at to size the columns.
const WIDTH_SAMPLE: usize = 1000;

#[derive(Debug, Default)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
}

/// A cell as it is shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Cell {
    #[default]
    Empty,
    Text(Box<str>),
    /// A number (or date), formatted; shown right-aligned.
    Number(Box<str>),
    Bool(bool),
    Error(Box<str>),
}

impl Cell {
    pub fn text(&self) -> &str {
        match self {
            Cell::Empty => "",
            Cell::Text(s) | Cell::Number(s) | Cell::Error(s) => s,
            Cell::Bool(true) => "TRUE",
            Cell::Bool(false) => "FALSE",
        }
    }
}

#[derive(Debug, Default)]
pub struct Sheet {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    /// Row and column (0-based) of the first cell kept, so labels read as in the file (a sheet
    /// that starts at C5 shows C and 5).
    pub origin: (u32, u32),
    cells: Vec<Cell>,
    /// Each column's width in characters (its longest text among the first rows).
    pub widths: Vec<u16>,
    /// Rows were left out (too many cells).
    pub truncated: bool,
    /// The sheet could not be read.
    pub error: Option<String>,
}

impl Sheet {
    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        static EMPTY: Cell = Cell::Empty;
        if row >= self.rows || col >= self.cols {
            return &EMPTY;
        }
        self.cells.get(row * self.cols + col).unwrap_or(&EMPTY)
    }

    fn from_rows(name: String, origin: (u32, u32), rows: impl Iterator<Item = Vec<Cell>>) -> Self {
        let mut cells = Vec::new();
        let mut grid: Vec<Vec<Cell>> = Vec::new();
        let mut cols = 0;
        let mut kept = 0usize;
        let mut truncated = false;
        for row in rows {
            if kept + row.len().max(cols) > MAX_CELLS {
                truncated = true;
                break;
            }
            kept += row.len().max(cols);
            cols = cols.max(row.len());
            grid.push(row);
        }
        // Trailing empty rows and columns are noise (a stray format far down the sheet).
        while grid.last().is_some_and(|r| r.iter().all(|c| *c == Cell::Empty)) {
            grid.pop();
        }
        while cols > 0 && grid.iter().all(|r| r.get(cols - 1).is_none_or(|c| *c == Cell::Empty)) {
            cols -= 1;
        }
        let mut widths = vec![3u16; cols];
        for row in grid.iter().take(WIDTH_SAMPLE) {
            for (c, cell) in row.iter().take(cols).enumerate() {
                let n = cell.text().chars().count().min(60) as u16;
                widths[c] = widths[c].max(n);
            }
        }
        let rows = grid.len();
        cells.reserve(rows * cols);
        for mut row in grid {
            row.resize(cols, Cell::Empty);
            row.truncate(cols);
            cells.extend(row);
        }
        Sheet { name, rows, cols, origin, cells, widths, truncated, error: None }
    }
}

/// "A", "B", … "Z", "AA", … for a 0-based column.
pub fn column_name(mut c: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A number the way a spreadsheet shows it by default: whole numbers without decimals, others
/// with up to 10 significant digits, very big or small ones in scientific notation.
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let mag = v.abs();
    if !(1e-5..1e15).contains(&mag) {
        return format!("{v:.4e}");
    }
    let int_digits = (mag.log10().floor() as i32 + 1).max(1);
    let decimals = (10 - int_digits).clamp(0, 10) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

fn cell_of(d: &Data) -> Cell {
    match d {
        Data::Empty => Cell::Empty,
        Data::String(s) => {
            if s.is_empty() {
                Cell::Empty
            } else {
                Cell::Text(s.as_str().into())
            }
        }
        Data::Int(i) => Cell::Number(i.to_string().into()),
        Data::Float(f) => Cell::Number(format_number(*f).into()),
        Data::Bool(b) => Cell::Bool(*b),
        Data::DateTime(dt) => {
            let v = dt.as_f64();
            if dt.is_duration() {
                let secs = (v * 86400.0).round() as i64;
                Cell::Number(format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60).into())
            } else {
                let (y, mo, d, h, mi, s, _) = dt.to_ymd_hms_milli();
                let text = if v.fract() == 0.0 {
                    format!("{y:04}-{mo:02}-{d:02}")
                } else if v < 1.0 {
                    format!("{h:02}:{mi:02}:{s:02}")
                } else {
                    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
                };
                Cell::Number(text.into())
            }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => Cell::Number(s.as_str().into()),
        // As Excel shows them ("#DIV/0!", "#N/A").
        Data::Error(e) => Cell::Error(e.to_string().into()),
    }
}

/// Reads every sheet of a workbook (a sheet that fails says why, the others still show).
pub fn read_workbook(path: &Path) -> Result<Workbook, String> {
    let mut book = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let mut sheets = Vec::new();
    for name in book.sheet_names() {
        match book.worksheet_range(&name) {
            Ok(range) => {
                let origin = range.start().unwrap_or((0, 0));
                let rows = range.rows().map(|r| r.iter().map(cell_of).collect::<Vec<_>>());
                sheets.push(Sheet::from_rows(name, origin, rows));
            }
            Err(e) => sheets.push(Sheet { name, error: Some(e.to_string()), ..Default::default() }),
        }
    }
    if sheets.is_empty() {
        return Err("the workbook has no sheets".into());
    }
    Ok(Workbook { sheets })
}

/// The separator a CSV file most likely uses (from its first lines).
fn sniff_delimiter(head: &[u8], tsv: bool) -> u8 {
    if tsv {
        return b'\t';
    }
    let text = String::from_utf8_lossy(head);
    let lines: Vec<&str> = text.lines().take(5).collect();
    // `max_by_key` takes the last of equals: reversed, a tie goes to `,`, then `;`, tab, `|`.
    b",;\t|"
        .iter()
        .copied()
        .rev()
        .max_by_key(|d| lines.iter().map(|l| l.matches(*d as char).count()).min().unwrap_or(0) * 4 + lines.first().map_or(0, |l| l.matches(*d as char).count()))
        .unwrap_or(b',')
}

/// A CSV field: numbers are shown right-aligned; codes with leading zeros stay text.
fn csv_cell(field: &[u8]) -> Cell {
    let s = String::from_utf8_lossy(field);
    let t = s.trim();
    if t.is_empty() {
        return Cell::Empty;
    }
    let leading_zero = t.len() > 1 && t.starts_with('0') && !t.starts_with("0.");
    if !leading_zero && t.len() < 18 && t.parse::<f64>().is_ok_and(f64::is_finite) {
        return Cell::Number(t.into());
    }
    Cell::Text(s.into_owned().into_boxed_str())
}

pub fn read_csv(path: &Path) -> Result<Workbook, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let tsv = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv"));
    let delimiter = sniff_delimiter(&bytes[..bytes.len().min(16 * 1024)], tsv);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(bytes);
    // A record that cannot be read ends the sheet there, marked as cut (not shortened silently).
    let mut broken = false;
    let rows = reader
        .byte_records()
        .map_while(|r| r.inspect_err(|_| broken = true).ok())
        .map(|r| r.iter().map(csv_cell).collect::<Vec<_>>());
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut sheet = Sheet::from_rows(name, (0, 0), rows);
    sheet.truncated |= broken;
    Ok(Workbook { sheets: vec![sheet] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_named_like_a_spreadsheet() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(27), "AB");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }

    #[test]
    fn numbers_read_like_a_spreadsheet() {
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(-3.0), "-3");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1234.5678), "1234.5678");
        assert_eq!(format_number(1.0 / 3.0), "0.333333333");
        assert_eq!(format_number(1e20), "1.0000e20");
    }

    #[test]
    fn csv_files_become_one_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.csv");
        std::fs::write(&p, "\u{feff}name;qty;code\n\"Smith; J\";3;007\nLee;4.5;\n\n").unwrap();
        let book = read_csv(&p).unwrap();
        let s = &book.sheets[0];
        assert_eq!((s.rows, s.cols), (3, 3));
        assert_eq!(s.cell(1, 0), &Cell::Text("Smith; J".into()));
        assert_eq!(s.cell(1, 1), &Cell::Number("3".into()));
        assert_eq!(s.cell(1, 2), &Cell::Text("007".into()));
        assert_eq!(s.cell(2, 2), &Cell::Empty);
        assert_eq!(s.cell(99, 99), &Cell::Empty);
        assert_eq!(s.widths[0], 8);
    }

    #[test]
    fn the_delimiter_is_the_one_used_and_a_comma_on_ties() {
        assert_eq!(sniff_delimiter(b"one column\nno separator\n", false), b',');
        assert_eq!(sniff_delimiter(b"a;b|c\nd;e|f\n", false), b';');
        assert_eq!(sniff_delimiter(b"a|b|c\nd|e|f\n", false), b'|');
        assert_eq!(sniff_delimiter(b"a\tb\nc\td\n", false), b'\t');
        assert_eq!(sniff_delimiter(b"a,b\n", true), b'\t');
    }

    #[test]
    fn cell_errors_read_as_in_excel() {
        use calamine::CellErrorType as E;
        let text = |e| cell_of(&Data::Error(e)).text().to_string();
        assert_eq!(text(E::Div0), "#DIV/0!");
        assert_eq!(text(E::NA), "#N/A");
        assert_eq!(text(E::Name), "#NAME?");
        assert_eq!(text(E::Null), "#NULL!");
        assert_eq!(text(E::Num), "#NUM!");
        assert_eq!(text(E::Ref), "#REF!");
        assert_eq!(text(E::Value), "#VALUE!");
        assert_eq!(text(E::GettingData), "#DATA!");
    }

    #[test]
    fn a_workbook_reads_every_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.xlsx");
        crate::office::tests::write_zip(
            &p,
            &[
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
                ),
                (
                    "xl/workbook.xml",
                    r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="B2" t="inlineStr"><is><t>hello</t></is></c><c r="C2"><v>2.5</v></c></row></sheetData></worksheet>"#,
                ),
            ],
        );
        let book = read_workbook(&p).unwrap();
        assert_eq!(book.sheets.len(), 1);
        let s = &book.sheets[0];
        assert_eq!(s.name, "Data");
        assert_eq!(s.origin, (1, 1));
        assert_eq!(s.cell(0, 0), &Cell::Text("hello".into()));
        assert_eq!(s.cell(0, 1), &Cell::Number("2.5".into()));
    }
}
