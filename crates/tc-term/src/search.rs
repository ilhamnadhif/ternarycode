//! Searching a terminal's screen and scrollback (FR-T6): plain text, case-insensitive unless the
//! query has capitals, from the bottom up as terminals usually do.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexSearch};

use crate::core::TermShared;

/// The query as a regular expression matching it literally.
pub fn escape(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The next match before (`backward`) or after `from`; from the bottom (or top) when `from` is
/// `None`. Wraps around.
pub fn find(shared: &TermShared, query: &str, from: Option<&Match>, backward: bool) -> Option<Match> {
    if query.is_empty() {
        return None;
    }
    let mut regex = RegexSearch::new(&escape(query)).ok()?;
    let term = shared.term.lock();
    let origin = match (from, backward) {
        (Some(m), true) => {
            let p = *m.start();
            if p.column.0 > 0 {
                Point::new(p.line, Column(p.column.0 - 1))
            } else if p.line <= term.topmost_line() {
                // Before the oldest line: go round to the end.
                Point::new(term.bottommost_line(), term.last_column())
            } else {
                Point::new(p.line - 1, term.last_column())
            }
        }
        (Some(m), false) => {
            let p = *m.end();
            p.add(&*term, alacritty_terminal::index::Boundary::None, 1)
        }
        (None, true) => Point::new(term.bottommost_line(), term.last_column()),
        (None, false) => Point::new(term.topmost_line(), Column(0)),
    };
    let (direction, side) = if backward {
        (Direction::Left, Side::Right)
    } else {
        (Direction::Right, Side::Left)
    };
    term.search_next(&mut regex, origin, direction, side, None)
}

/// Up to `limit` matches, newest first, each with its line of text.
pub fn find_all(shared: &TermShared, query: &str, limit: usize) -> Vec<(Match, String)> {
    let mut out: Vec<(Match, String)> = Vec::new();
    let mut from: Option<Match> = None;
    while out.len() < limit {
        let Some(m) = find(shared, query, from.as_ref(), true) else { break };
        if out.iter().any(|(x, _)| x == &m) {
            break; // wrapped around
        }
        let text = line_text(shared, m.start().line.0);
        out.push((m.clone(), text));
        from = Some(m);
    }
    out
}

fn line_text(shared: &TermShared, line: i32) -> String {
    let term = shared.term.lock();
    let row = &term.grid()[alacritty_terminal::index::Line(line)];
    (0..term.columns())
        .map(|c| row[Column(c)].c)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Selects the match and scrolls it into view.
pub fn reveal(shared: &TermShared, m: &Match) {
    let mut term = shared.term.lock();
    let mut sel = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
    sel.update(*m.end(), Side::Right);
    term.selection = Some(sel);
    term.scroll_to_point(*m.start());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Feeder, GridSize, new_terminal, term_config};
    use crate::scheme::SCHEMES;

    #[test]
    fn finds_newest_first_and_wraps() {
        let (shared, rx) = new_terminal(term_config(1000, Default::default(), false), GridSize { cols: 40, rows: 5 }, (&SCHEMES[0]).into());
        let mut f = Feeder::new(shared.clone(), rx);
        for i in 0..30 {
            f.feed(format!("line {i} {}\r\n", if i % 10 == 3 { "error: boom" } else { "ok" }).as_bytes());
        }
        let all = find_all(&shared, "Error", 10);
        // Capital letters make it case-sensitive: nothing.
        assert!(all.is_empty());
        let all = find_all(&shared, "error: boom", 10);
        let lines: Vec<&str> = all.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(lines, ["line 23 error: boom", "line 13 error: boom", "line 3 error: boom"]);
        // Regex characters are literal.
        assert!(find(&shared, "boom.", None, true).is_none());
        reveal(&shared, &all[2].0);
        assert!(shared.term.lock().selection.is_some());
        assert_eq!(escape("a.b*"), "a\\.b\\*");
    }
}
