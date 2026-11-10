//! Parsers for git's diff output.

use std::collections::HashMap;

use crate::{ChangeKind, DiffLine, FileChange, FileDiff, Hunk, LineKind};

/// Parses `git diff --raw --numstat -z`: every raw record first, then every numstat record.
///
/// Raw: `:<old mode> <new mode> <old id> <new id> <status>\0<path>\0`, with a second path field
/// for renames and copies. Numstat: `<added>\t<deleted>\t<path>\0`, or for a rename
/// `<added>\t<deleted>\t\0<old path>\0<new path>\0`; binary files show `-` for both counts.
pub(crate) fn changes(out: &[u8]) -> Result<Vec<FileChange>, String> {
    let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    let mut i = 0;
    let next_path = |i: &mut usize| -> Result<String, String> {
        let field = fields
            .get(*i)
            .filter(|field| !field.is_empty())
            .ok_or("git diff output ended where a path was expected")?;
        *i += 1;
        Ok(lossy(field))
    };

    let mut changes = Vec::new();
    // (additions, deletions, binary) by new path.
    let mut stats: HashMap<String, (Option<u32>, Option<u32>, bool)> = HashMap::new();
    while let Some(&field) = fields.get(i) {
        i += 1;
        if field.is_empty() {
            continue;
        }
        if field[0] == b':' {
            let status = field.rsplit(|&b| b == b' ').next().unwrap_or_default();
            let letter = *status
                .first()
                .ok_or_else(|| format!("unexpected git diff record {:?}", lossy(field)))?;
            let first = next_path(&mut i)?;
            let (old_path, path) = if matches!(letter, b'R' | b'C') {
                (Some(first), next_path(&mut i)?)
            } else {
                (None, first)
            };
            let kind = match letter {
                b'A' => ChangeKind::Added,
                b'D' => ChangeKind::Deleted,
                b'R' => ChangeKind::Renamed,
                // A copy is a new file whose source is unchanged; `-M` never reports one, but
                // a `diff.renames=copies` setting must not turn it into an error.
                b'C' => ChangeKind::Added,
                // M, and T (type change such as a symlink replaced by a file).
                _ => ChangeKind::Modified,
            };
            let old_path = old_path.filter(|_| kind == ChangeKind::Renamed);
            changes.push(FileChange {
                path,
                old_path,
                kind,
                additions: None,
                deletions: None,
                binary: false,
            });
        } else {
            let mut parts = field.splitn(3, |&b| b == b'\t');
            let (Some(added), Some(deleted), Some(rest)) =
                (parts.next(), parts.next(), parts.next())
            else {
                return Err(format!("unexpected git diff record {:?}", lossy(field)));
            };
            let path = if rest.is_empty() {
                next_path(&mut i)?;
                next_path(&mut i)?
            } else {
                lossy(rest)
            };
            let binary = added == b"-" || deleted == b"-";
            stats.insert(path, (count(added), count(deleted), binary));
        }
    }

    for change in &mut changes {
        if let Some(&(additions, deletions, binary)) = stats.get(&change.path) {
            change.additions = additions;
            change.deletions = deletions;
            change.binary = binary;
        }
    }
    Ok(changes)
}

fn count(field: &[u8]) -> Option<u32> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

/// Parses a unified diff limited to `path` (and `old_path`) and returns the part about `path`.
///
/// The output can hold more than one file: a type change (symlink to file) is printed as a
/// deletion plus an addition of the same path, a rename that git did not pair is a deletion of
/// `old_path` plus an addition of `path`, and a pathspec also matches files below a directory of
/// that name. Sections whose new path is `path` are merged; the others are dropped.
pub(crate) fn file_diff(
    out: &[u8],
    path: &str,
    old_path: Option<&str>,
) -> Result<FileDiff, String> {
    let mut sections = sections(out)?;
    let is_path = |section: &Section| section.new.as_deref() == Some(path);
    let paired = |section: &Section| is_path(section) && section.old.as_deref() == old_path;
    let keep: Vec<Section> = if old_path.is_some() && sections.iter().any(paired) {
        sections.into_iter().filter(paired).collect()
    } else if sections.iter().any(is_path) {
        sections.into_iter().filter(is_path).collect()
    } else if sections.len() == 1 {
        // Header paths that could not be read; the pathspec still limited the diff to `path`.
        sections.truncate(1);
        sections
    } else {
        Vec::new()
    };
    let mut diff = FileDiff::default();
    for section in keep {
        diff.binary |= section.diff.binary;
        diff.hunks.extend(section.diff.hunks);
    }
    Ok(diff)
}

/// The diff of one file in git's output (one `diff --git` header and what follows it).
struct Section {
    old: Option<String>,
    new: Option<String>,
    diff: FileDiff,
}

fn sections(out: &[u8]) -> Result<Vec<Section>, String> {
    let mut lines: Vec<&[u8]> = out.split(|&b| b == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let mut sections: Vec<Section> = Vec::new();
    let mut i = 0;
    while let Some(&line) = lines.get(i) {
        i += 1;
        if let Some(rest) = line.strip_prefix(b"diff --git ") {
            let (old, new) = header_paths(rest).unzip();
            sections.push(Section {
                old,
                new,
                diff: FileDiff::default(),
            });
            continue;
        }
        let Some(section) = sections.last_mut() else {
            continue;
        };
        if line.starts_with(b"@@ ") {
            let hunk = hunk(line, &lines, &mut i)?;
            section.diff.hunks.push(hunk);
        } else if let Some(name) = line.strip_prefix(b"rename from ") {
            section.old = Some(lossy(&unquote(name)));
        } else if let Some(name) = line.strip_prefix(b"rename to ") {
            section.new = Some(lossy(&unquote(name)));
        } else if (line.starts_with(b"Binary files ") && line.ends_with(b" differ"))
            || line == b"GIT binary patch"
        {
            section.diff.binary = true;
        }
        // Other header lines (index, modes, similarity, ---/+++) carry nothing needed here.
    }
    Ok(sections)
}

/// Parses the hunk starting with `header`; its body is `lines[*i..]`, and `*i` ends past it. The
/// body length comes from the line counts in the header, so a body line can never be mistaken
/// for a header.
fn hunk(header: &[u8], lines: &[&[u8]], i: &mut usize) -> Result<Hunk, String> {
    let header = text(header);
    let (old_start, old_lines, new_start, new_lines) =
        hunk_ranges(&header).ok_or_else(|| format!("unexpected hunk header {header:?}"))?;
    let (mut old_no, mut new_no) = (old_start, new_start);
    let (mut old_left, mut new_left) = (old_lines, new_lines);
    let mut body = Vec::new();
    while old_left > 0 || new_left > 0 {
        let Some(&line) = lines.get(*i) else {
            return Err(format!("git diff output ended inside the hunk {header:?}"));
        };
        *i += 1;
        let kind = match line.first() {
            // An empty line is an empty context line under `diff.suppressBlankEmpty`.
            Some(b' ') | None => LineKind::Context,
            Some(b'+') => LineKind::Added,
            Some(b'-') => LineKind::Removed,
            // "\ No newline at end of file", about the line before.
            Some(b'\\') => continue,
            Some(_) => {
                return Err(format!(
                    "unexpected line {:?} in the hunk {header:?}",
                    lossy(line)
                ));
            }
        };
        let uses_old = kind != LineKind::Added;
        let uses_new = kind != LineKind::Removed;
        if (uses_old && old_left == 0) || (uses_new && new_left == 0) {
            return Err(format!(
                "the hunk {header:?} is longer than its header says"
            ));
        }
        body.push(DiffLine {
            kind,
            old_no: uses_old.then_some(old_no),
            new_no: uses_new.then_some(new_no),
            text: text(line.get(1..).unwrap_or_default()),
        });
        if uses_old {
            old_no += 1;
            old_left -= 1;
        }
        if uses_new {
            new_no += 1;
            new_left -= 1;
        }
    }
    while lines.get(*i).is_some_and(|line| line.starts_with(b"\\")) {
        *i += 1;
    }
    Ok(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        header,
        lines: body,
    })
}

/// Reads `@@ -<start>[,<count>] +<start>[,<count>] @@...`; a missing count means 1.
fn hunk_ranges(header: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = header.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let range = |range: &str| -> Option<(u32, u32)> {
        match range.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((range.parse().ok()?, 1)),
        }
    };
    let (old_start, old_lines) = range(old)?;
    let (new_start, new_lines) = range(new)?;
    Some((old_start, old_lines, new_start, new_lines))
}

/// Reads the two paths of `diff --git a/<old> b/<new>`. A path is C-quoted when it holds a
/// quote, a backslash or a control character. Two unquoted paths are only split when they are
/// equal, since a space inside a path makes the split ambiguous; renames name their paths again
/// in `rename from`/`rename to` lines.
fn header_paths(rest: &[u8]) -> Option<(String, String)> {
    let (a, b) = if rest.first() == Some(&b'"') {
        let (a, used) = unquote_prefix(rest)?;
        (a, unquote(rest[used..].strip_prefix(b" ")?))
    } else if rest.last() == Some(&b'"') {
        // An unquoted path never holds a quote, and inside a quoted one every quote is escaped,
        // so the first ` "` separates them.
        let space = rest.windows(2).position(|pair| pair == b" \"")?;
        (rest[..space].to_vec(), unquote(&rest[space + 1..]))
    } else {
        let len = rest.len();
        if len < 5 || !(len - 5).is_multiple_of(2) {
            return None;
        }
        let half = (len - 5) / 2;
        let (a, b) = (&rest[..2 + half], &rest[3 + half..]);
        if rest[2 + half] != b' ' || a.get(2..) != b.get(2..) {
            return None;
        }
        (a.to_vec(), b.to_vec())
    };
    Some((lossy(a.strip_prefix(b"a/")?), lossy(b.strip_prefix(b"b/")?)))
}

/// Decodes `name` if it is one C-quoted string, else returns it as is.
fn unquote(name: &[u8]) -> Vec<u8> {
    match unquote_prefix(name) {
        Some((decoded, used)) if used == name.len() => decoded,
        _ => name.to_vec(),
    }
}

/// Decodes the C-quoted string at the start of `s`, as git writes names that need quoting.
/// Returns the bytes and the length of the quoted string in `s`.
fn unquote_prefix(s: &[u8]) -> Option<(Vec<u8>, usize)> {
    if s.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    while let Some(&b) = s.get(i) {
        i += 1;
        match b {
            b'"' => return Some((out, i)),
            b'\\' => {
                let escaped = *s.get(i)?;
                i += 1;
                out.push(match escaped {
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 0x0b,
                    b'f' => 0x0c,
                    b'r' => b'\r',
                    b'"' | b'\\' => escaped,
                    b'0'..=b'3' => {
                        let digits = s.get(i..i + 2)?;
                        if !digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                            return None;
                        }
                        i += 2;
                        ((escaped - b'0') << 6) | ((digits[0] - b'0') << 3) | (digits[1] - b'0')
                    }
                    _ => return None,
                });
            }
            _ => out.push(b),
        }
    }
    None
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A line of diff text without a trailing `\r` (from a CRLF file).
fn text(bytes: &[u8]) -> String {
    lossy(bytes.strip_suffix(b"\r").unwrap_or(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_and_numstat_records() {
        let out = b":100644 100644 ce01362 97531f3 M\0a.txt\0\
:100644 000000 b023018 0000000 D\0b.txt\0\
:000000 100644 0000000 88768ef A\0bin.dat\0\
:100644 100644 96cc558 1c5a36f R097\0c.txt\0d.txt\0\
:120000 100644 8d14cbf 0e04407 T\0link\0\
1\t0\ta.txt\0\
0\t1\tb.txt\0\
-\t-\tbin.dat\0\
1\t0\t\0c.txt\0d.txt\0\
1\t1\tlink\0";
        let changes = changes(out).unwrap();
        let summary: Vec<_> = changes
            .iter()
            .map(|c| {
                (
                    c.path.as_str(),
                    c.old_path.as_deref(),
                    c.kind,
                    c.additions,
                    c.deletions,
                    c.binary,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("a.txt", None, ChangeKind::Modified, Some(1), Some(0), false),
                ("b.txt", None, ChangeKind::Deleted, Some(0), Some(1), false),
                ("bin.dat", None, ChangeKind::Added, None, None, true),
                (
                    "d.txt",
                    Some("c.txt"),
                    ChangeKind::Renamed,
                    Some(1),
                    Some(0),
                    false
                ),
                ("link", None, ChangeKind::Modified, Some(1), Some(1), false),
            ]
        );
        assert!(super::changes(b"").unwrap().is_empty());
        assert!(super::changes(b":100644 100644 a b M\0").is_err());
    }

    #[test]
    fn reads_header_paths() {
        let paths = |s: &str| header_paths(s.as_bytes());
        let pair = |a: &str, b: &str| Some((a.to_string(), b.to_string()));
        assert_eq!(paths("a/x.txt b/x.txt"), pair("x.txt", "x.txt"));
        assert_eq!(
            paths("a/dir sp/f b/x b/dir sp/f b/x"),
            pair("dir sp/f b/x", "dir sp/f b/x")
        );
        assert_eq!(
            paths(r#""a/we\"ird\ttab.txt" "b/we\"ird\ttab.txt""#),
            pair("we\"ird\ttab.txt", "we\"ird\ttab.txt")
        );
        assert_eq!(paths(r#"a/plain "b/q\"uote""#), pair("plain", "q\"uote"));
        assert_eq!(paths(r#""a/\303\251" b/e"#), pair("é", "e"));
        // A rename with unquoted names: left to the `rename from/to` lines.
        assert_eq!(paths("a/old name b/new name"), None);
    }

    #[test]
    fn parses_hunks_with_markers_and_blank_context() {
        let out = [
            "diff --git a/f b/f",
            "index 1..2 100644",
            "--- a/f",
            "+++ b/f",
            "@@ -1,3 +1,3 @@ fn main() {\r",
            " one\r",
            "",
            "-three",
            "\\ No newline at end of file",
            "+three",
            "",
        ]
        .join("\n");
        let diff = file_diff(out.as_bytes(), "f", None).unwrap();
        assert!(!diff.binary);
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.header, "@@ -1,3 +1,3 @@ fn main() {");
        let lines: Vec<_> = hunk
            .lines
            .iter()
            .map(|l| (l.kind, l.old_no, l.new_no, l.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                (LineKind::Context, Some(1), Some(1), "one"),
                (LineKind::Context, Some(2), Some(2), ""),
                (LineKind::Removed, Some(3), None, "three"),
                (LineKind::Added, None, Some(3), "three"),
            ]
        );
    }

    #[test]
    fn rejects_truncated_hunks() {
        let out = b"diff --git a/f b/f\n@@ -1,2 +1,2 @@\n a\n";
        assert!(file_diff(out, "f", None).is_err());
    }

    #[test]
    fn hunk_ranges_default_to_one_line() {
        assert_eq!(hunk_ranges("@@ -1 +1,2 @@"), Some((1, 1, 1, 2)));
        assert_eq!(hunk_ranges("@@ -0,0 +1 @@ x"), Some((0, 0, 1, 1)));
        assert_eq!(hunk_ranges("@@ -a +1 @@"), None);
    }

    #[test]
    fn unquotes_c_strings() {
        assert_eq!(unquote(br#""a\tb\\c\"d\001""#), b"a\tb\\c\"d\x01");
        assert_eq!(unquote(b"plain"), b"plain");
        assert_eq!(unquote(br#""unterminated"#), br#""unterminated"#);
    }
}
