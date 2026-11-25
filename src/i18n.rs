//! English UI strings and formatting helpers.

#[inline]
pub fn t(en: &'static str) -> &'static str {
    en
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

/// "1 project" or "3 projects".
pub fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { one.to_string() } else { fill(many, &[&n.to_string()]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_count() {
        assert_eq!(t("Run Agent"), "Run Agent");
        assert_eq!(tf("{} of {}", &["1", "2"]), "1 of 2");
        assert_eq!(count(1, "1 project", "{} projects"), "1 project");
        assert_eq!(count(3, "1 project", "{} projects"), "3 projects");
    }
}
