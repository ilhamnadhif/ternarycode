//! English / Indonesian UI strings.
//!
//! UI code writes English through `t("…")`; in Indonesian the string is looked up in `ID` and
//! falls back to English when missing. A unit test checks that every literal passed to `t(` or
//! `tf(` in the source has a translation.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub use tc_core::model::Language;

static INDONESIAN: AtomicBool = AtomicBool::new(false);

static ON_CHANGE: std::sync::Mutex<Option<fn()>> = std::sync::Mutex::new(None);

/// Called after the language changes (the native menu bar is built again).
pub fn on_language_change(f: fn()) {
    if let Ok(mut g) = ON_CHANGE.lock() {
        *g = Some(f);
    }
}

pub fn set_language(lang: Language) {
    let id = match lang {
        Language::English => false,
        Language::Indonesian => true,
        Language::System => sys_locale::get_locales()
            .next()
            .is_some_and(|l| l.starts_with("id") || l.starts_with("in")),
    };
    let was = INDONESIAN.swap(id, Ordering::Relaxed);
    if was != id
        && let Ok(g) = ON_CHANGE.lock()
        && let Some(f) = *g
    {
        f();
    }
}

pub fn is_indonesian() -> bool {
    INDONESIAN.load(Ordering::Relaxed)
}

pub fn t(en: &'static str) -> &'static str {
    if !is_indonesian() {
        return en;
    }
    table().get(en).copied().unwrap_or(en)
}

/// `t` and then each `{}` replaced by the next argument.
pub fn tf(en: &'static str, args: &[&str]) -> String {
    fill(t(en), args)
}

/// `text` with each `{}` replaced by the next argument.
fn fill(text: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut args = args.iter();
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        out.push_str(args.next().copied().unwrap_or(""));
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| ID.iter().copied().collect())
}

include!("i18n_table.rs");

/// "1 project" or "3 projects", from forms already translated (`many` with `{}`): callers
/// write `t("1 project")`, where the tests' scan of the sources finds the strings.
pub fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { one.to_string() } else { fill(many, &[&n.to_string()]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ui_string_is_translated() {
        let mut missing = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || path
                        .file_name()
                        .is_some_and(|n| n == "i18n_table.rs" || n == "i18n.rs")
                {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for lit in extract_literals(&text) {
                    if seen.insert(lit.clone()) && !table().contains_key(lit.as_str()) {
                        missing.push(format!("{}: {lit:?}", path.display()));
                    }
                }
            }
        }
        missing.sort();
        assert!(missing.is_empty(), "untranslated strings:\n{}", missing.join("\n"));
    }

    #[test]
    fn placeholders_match() {
        for (en, id) in ID {
            assert_eq!(en.matches("{}").count(), id.matches("{}").count(), "{en}");
        }
    }

    fn extract_literals(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for pat in ["t(", "tf("] {
            let mut rest = text;
            while let Some(pos) = rest.find(pat) {
                let prev = rest[..pos].chars().last();
                rest = &rest[pos + pat.len()..];
                if prev.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                    continue;
                }
                let trimmed = rest.trim_start();
                let Some(after) = trimmed.strip_prefix('"') else {
                    continue;
                };
                rest = after;
                let mut lit = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => lit.push('\n'),
                            Some(other) => lit.push(other),
                            None => break,
                        },
                        '"' => break,
                        c => lit.push(c),
                    }
                }
                out.push(lit);
            }
        }
        out
    }
}
