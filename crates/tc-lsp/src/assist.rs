//! Typing help on the client side: which completion items match what was typed and in which
//! order, snippets as plain text (with the cursor where the first stop was), and hover
//! documentation as plain text blocks.

use lsp_types::{Documentation, HoverContents, MarkedString, MarkupContent};

/// How well `label` matches `typed` (higher is better): a prefix beats a word start beats a
/// subsequence; case-insensitive. None when it does not match at all. Empty `typed` matches.
pub fn score(label: &str, typed: &str) -> Option<i32> {
    if typed.is_empty() {
        return Some(0);
    }
    // One lower-case char per char of the label (some lower to two, "İ" → "i̇": the first
    // stands in), so positions in `l` are positions in `chars`.
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let chars: Vec<char> = label.chars().collect();
    let l: Vec<char> = chars.iter().map(|c| lower(*c)).collect();
    let t: Vec<char> = typed.chars().map(lower).collect();
    if l.starts_with(&t) {
        // Exact case prefix ranks a little higher.
        let exact = label.starts_with(typed) as i32;
        return Some(1000 - l.len() as i32 + exact * 50);
    }
    // Subsequence, rewarding matches at word starts (after _ or a lower→upper change).
    let mut ti = 0;
    let mut s = 0;
    for (i, c) in l.iter().enumerate() {
        if ti < t.len() && *c == t[ti] {
            let start = i == 0
                || matches!(chars[i - 1], '_' | '-' | '.' | ':')
                || (chars[i].is_uppercase() && chars[i - 1].is_lowercase());
            s += if start { 30 } else { 5 };
            ti += 1;
        }
    }
    (ti == t.len()).then_some(s - l.len() as i32)
}

/// A snippet (`foo(${1:a}, $2)$0`) as the text to insert and where the cursor goes (char offset
/// in that text): placeholders keep their default text (nested ones too), a choice its first
/// option, variables (`$TM_FILENAME`) their default or nothing; the first tab stop takes the
/// cursor ($0, the final position, only when there is no other).
pub fn snippet_text(snippet: &str) -> (String, Option<usize>) {
    let chars: Vec<char> = snippet.chars().collect();
    let mut p = Snippet { chars: &chars, at: 0, out: String::new(), count: 0, first: None };
    p.parse(false);
    (p.out, p.first.map(|(_, at)| at))
}

struct Snippet<'a> {
    chars: &'a [char],
    at: usize,
    out: String,
    count: usize,
    first: Option<(u32, usize)>,
}

impl Snippet<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn emit(&mut self, c: char) {
        self.out.push(c);
        self.count += 1;
    }

    fn stop(&mut self, n: u32) {
        let rank = if n == 0 { u32::MAX } else { n };
        if self.first.is_none_or(|(r, _)| rank < r) {
            self.first = Some((rank, self.count));
        }
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.at;
        let mut n = 0u32;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(d);
            self.at += 1;
        }
        (self.at > start).then_some(n)
    }

    fn name(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            self.at += 1;
        }
        self.at > start
    }

    /// Text up to the end (or, inside `${…}`, up to its closing brace, which it takes).
    fn parse(&mut self, inside: bool) {
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '\\' => {
                    if let Some(n) = self.peek() {
                        self.at += 1;
                        self.emit(n);
                    }
                }
                '}' if inside => return,
                '$' => self.dollar(),
                _ => self.emit(c),
            }
        }
    }

    fn dollar(&mut self) {
        match self.peek() {
            Some(d) if d.is_ascii_digit() => {
                let n = self.number().unwrap_or(0);
                self.stop(n);
            }
            Some('{') => {
                self.at += 1;
                if let Some(n) = self.number() {
                    self.stop(n);
                    match self.peek() {
                        Some(':') => {
                            self.at += 1;
                            self.parse(true);
                        }
                        Some('|') => {
                            self.at += 1;
                            self.choice();
                        }
                        Some('}') => self.at += 1,
                        _ => self.skip_to_brace(),
                    }
                } else if self.name() {
                    // A variable: its default, if any (its value is not known here).
                    match self.peek() {
                        Some(':') => {
                            self.at += 1;
                            self.parse(true);
                        }
                        Some('}') => self.at += 1,
                        _ => self.skip_to_brace(),
                    }
                } else {
                    self.emit('$');
                    self.emit('{');
                }
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                self.name();
            }
            _ => self.emit('$'),
        }
    }

    /// `a,b,c|}`: the first option.
    fn choice(&mut self) {
        let mut first = true;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '\\' => {
                    if let Some(n) = self.peek() {
                        self.at += 1;
                        if first {
                            self.emit(n);
                        }
                    }
                }
                ',' => first = false,
                '|' => {
                    if self.peek() == Some('}') {
                        self.at += 1;
                    }
                    return;
                }
                _ if first => self.emit(c),
                _ => {}
            }
        }
    }

    fn skip_to_brace(&mut self) {
        while let Some(c) = self.peek() {
            self.at += 1;
            if c == '}' {
                return;
            }
        }
    }
}

/// A block of hover or documentation text: code (shown monospace) or prose.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub code: bool,
    pub text: String,
}

/// Markdown as blocks: fenced code apart, prose with its markup simplified.
pub fn markdown(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut prose = String::new();
    let mut code: Option<String> = None;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(c) => out.push(Block { code: true, text: c.trim_end().to_string() }),
                None => {
                    flush(&mut prose, &mut out);
                    code = Some(String::new());
                }
            }
            continue;
        }
        match code.as_mut() {
            Some(c) => {
                c.push_str(line);
                c.push('\n');
            }
            None => {
                let l = line.trim_end();
                if l == "---" || l == "***" {
                    flush(&mut prose, &mut out);
                    continue;
                }
                prose.push_str(&simplify(l));
                prose.push('\n');
            }
        }
    }
    if let Some(c) = code {
        out.push(Block { code: true, text: c.trim_end().to_string() });
    }
    flush(&mut prose, &mut out);
    out
}

fn flush(prose: &mut String, out: &mut Vec<Block>) {
    let t = prose.trim();
    if !t.is_empty() {
        out.push(Block { code: false, text: t.to_string() });
    }
    prose.clear();
}

/// Bold, italic, inline code and links as their text; heading marks dropped.
fn simplify(line: &str) -> String {
    let line = line.trim_start_matches('#').trim_start();
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '`' => {}
            '_' if chars.peek() == Some(&'_') => {
                chars.next();
            }
            '[' => {
                let mut label = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    label.push(c);
                }
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&label);
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// A hover's contents as blocks.
pub fn hover_blocks(contents: &HoverContents) -> Vec<Block> {
    let marked = |m: &MarkedString| match m {
        MarkedString::String(s) => markdown(s),
        MarkedString::LanguageString(ls) => vec![Block { code: true, text: ls.value.clone() }],
    };
    match contents {
        HoverContents::Scalar(m) => marked(m),
        HoverContents::Array(ms) => ms.iter().flat_map(marked).collect(),
        HoverContents::Markup(MarkupContent { value, .. }) => markdown(value),
    }
}

/// Documentation as blocks.
pub fn doc_blocks(doc: &Documentation) -> Vec<Block> {
    match doc {
        Documentation::String(s) => markdown(s),
        Documentation::MarkupContent(m) => markdown(&m.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rank_prefixes_then_word_starts() {
        assert!(score("println", "pri").unwrap() > score("eprintln", "pri").unwrap());
        assert!(score("to_string", "ts").is_some());
        assert!(score("toString", "tS").is_some());
        assert!(score("format", "xyz").is_none());
        // A word start beats letters in the middle.
        assert!(score("get_value", "gv").unwrap() > score("give", "gv").unwrap());
        assert_eq!(score("x", ""), Some(0));
        // A label whose lower case is longer must not trip the scoring.
        assert!(score("İx", "x").is_some());
        assert!(score("aİbc", "bc").is_some());
    }

    #[test]
    fn snippets_become_text_with_a_cursor() {
        assert_eq!(snippet_text("foo(${1:a}, ${2:b})$0"), ("foo(a, b)".into(), Some(4)));
        assert_eq!(snippet_text("println!($0)"), ("println!()".into(), Some(9)));
        assert_eq!(snippet_text("if ${1|true,false|} {}"), ("if true {}".into(), Some(3)));
        assert_eq!(snippet_text("cost \\$5"), ("cost $5".into(), None));
        assert_eq!(snippet_text("plain"), ("plain".into(), None));
        // Nested placeholders keep their text; variables their default or nothing.
        assert_eq!(snippet_text("${1:foo(${2:bar})}"), ("foo(bar)".into(), Some(0)));
        assert_eq!(snippet_text("// $TM_FILENAME ${TM_SELECTED_TEXT:x}$0"), ("//  x".into(), Some(5)));
        assert_eq!(snippet_text("a\\}b${1}"), ("a}b".into(), Some(3)));
    }

    #[test]
    fn markdown_keeps_code_apart() {
        let md = "```rust\nfn main()\n```\n---\nPrints **hello**, see [docs](http://x) and `x`.";
        let b = markdown(md);
        assert_eq!(b[0], Block { code: true, text: "fn main()".into() });
        assert_eq!(b[1], Block { code: false, text: "Prints hello, see docs and x.".into() });
    }
}
