//! Edits from a server (formatting, rename, code actions, completion), applied to text.

use std::collections::BTreeMap;
use std::path::PathBuf;

use lsp_types::{DocumentChangeOperation, DocumentChanges, OneOf, TextEdit, WorkspaceEdit};

use crate::pos::{Encoding, LineIndex, byte_of, path};

/// The edits for one file, and the document version the server based them on (when it said).
#[derive(Clone, Debug, PartialEq)]
pub struct FileEdit {
    pub path: PathBuf,
    pub version: Option<i32>,
    pub edits: Vec<TextEdit>,
}

/// A workspace edit as edits per file, in a stable order. File creations, renames and
/// deletions are listed apart: the app does not apply those.
pub fn files(edit: &WorkspaceEdit) -> (Vec<FileEdit>, usize) {
    let mut by_path: BTreeMap<PathBuf, FileEdit> = BTreeMap::new();
    let mut skipped = 0;
    let mut add = |p: PathBuf, version: Option<i32>, edits: Vec<TextEdit>| {
        let e = by_path.entry(p.clone()).or_insert(FileEdit {
            path: p,
            version,
            edits: Vec::new(),
        });
        e.edits.extend(edits);
    };
    match &edit.document_changes {
        Some(DocumentChanges::Edits(docs)) => {
            for d in docs {
                if let Some(p) = path(&d.text_document.uri) {
                    add(p, d.text_document.version, plain(&d.edits));
                }
            }
        }
        Some(DocumentChanges::Operations(ops)) => {
            for op in ops {
                match op {
                    DocumentChangeOperation::Edit(d) => {
                        if let Some(p) = path(&d.text_document.uri) {
                            add(p, d.text_document.version, plain(&d.edits));
                        }
                    }
                    DocumentChangeOperation::Op(_) => skipped += 1,
                }
            }
        }
        None => {
            for (uri, edits) in edit.changes.iter().flatten() {
                if let Some(p) = path(uri) {
                    add(p, None, edits.clone());
                }
            }
        }
    }
    (by_path.into_values().collect(), skipped)
}

fn plain(edits: &[OneOf<TextEdit, lsp_types::AnnotatedTextEdit>]) -> Vec<TextEdit> {
    edits
        .iter()
        .map(|e| match e {
            OneOf::Left(t) => t.clone(),
            OneOf::Right(a) => a.text_edit.clone(),
        })
        .collect()
}

/// `text` with `edits` applied (positions refer to the text before any of them, as LSP says).
/// Overlapping edits are refused.
pub fn apply(text: &str, edits: &[TextEdit], enc: Encoding) -> Result<String, String> {
    let ix = LineIndex::new(text);
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let (a, b) = ix.chars(text, &e.range, enc);
            (a, b.max(a), e.new_text.as_str())
        })
        .collect();
    // Stable: edits at the same place keep their order.
    spans.sort_by_key(|(a, b, _)| (*a, *b));
    for w in spans.windows(2) {
        if w[1].0 < w[0].1 {
            return Err("overlapping edits".into());
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    // Char indices → byte offsets in one walk (the spans are in order and do not overlap).
    let mut wanted: Vec<usize> = spans.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    wanted.dedup();
    let mut at = std::collections::HashMap::with_capacity(wanted.len());
    let mut next = wanted.iter().peekable();
    for (ci, (bi, _)) in text.char_indices().enumerate() {
        while next.peek().is_some_and(|w| **w == ci) {
            at.insert(ci, bi);
            next.next();
        }
        if next.peek().is_none() {
            break;
        }
    }
    let byte = |c: usize| at.get(&c).copied().unwrap_or_else(|| byte_of(text, c));
    let offsets: Vec<(usize, usize)> = spans.iter().map(|(a, b, _)| (byte(*a), byte(*b))).collect();
    for ((_, _, new), (ba, bb)) in spans.iter().zip(offsets) {
        out.push_str(&text[last..ba]);
        out.push_str(new);
        last = bb;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    fn edit(l0: u32, c0: u32, l1: u32, c1: u32, t: &str) -> TextEdit {
        TextEdit::new(Range::new(Position::new(l0, c0), Position::new(l1, c1)), t.into())
    }

    #[test]
    fn edits_apply_against_the_original_text() {
        let text = "let a = 1;\nlet b = a + a;\n";
        // Rename `a` in three places, listed out of order.
        let edits = [
            edit(1, 12, 1, 13, "alpha"),
            edit(0, 4, 0, 5, "alpha"),
            edit(1, 8, 1, 9, "alpha"),
        ];
        assert_eq!(
            apply(text, &edits, Encoding::Utf16).unwrap(),
            "let alpha = 1;\nlet b = alpha + alpha;\n"
        );
        // Insertions at one point keep their order; an emoji counts two units.
        let text = "😀x";
        let edits = [edit(0, 2, 0, 2, "1"), edit(0, 2, 0, 2, "2")];
        assert_eq!(apply(text, &edits, Encoding::Utf16).unwrap(), "😀12x");
        // Overlap.
        assert!(apply("abcdef", &[edit(0, 0, 0, 3, ""), edit(0, 2, 0, 4, "")], Encoding::Utf16).is_err());
    }

    #[test]
    #[allow(clippy::mutable_key_type)] // lsp-types' own map type
    fn workspace_edits_are_grouped_by_file() {
        let a = crate::pos::uri(std::path::Path::new("/p/a.rs"));
        let b = crate::pos::uri(std::path::Path::new("/p/b.rs"));
        let mut changes = std::collections::HashMap::new();
        changes.insert(b, vec![edit(0, 0, 0, 1, "x")]);
        changes.insert(a, vec![edit(0, 0, 0, 1, "y"), edit(1, 0, 1, 1, "z")]);
        let (files, skipped) = files(&WorkspaceEdit::new(changes));
        assert_eq!(skipped, 0);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("/p/a.rs"));
        assert_eq!(files[0].edits.len(), 2);
    }
}
