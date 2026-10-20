//! The server's sentences, written once for the emails and the UI: an alert
//! event and the alert catalogue ([`alerts`]), a domain's open issues
//! ([`issues`]), its policy readiness ([`readiness`]) and the client reports
//! that did not go out ([`client_reports`]). The emails (`mail`) word with
//! them and the API sends them beside the verdicts they word
//! (`AlertItem::text`, `WordedIssue::label`, `Readiness::lead`, the
//! catalogues in `GET /api/meta`), so the UI words none of this itself: it colours and lays out what it is sent. Numbers read like
//! the UI's `$lib/format`: `12,345`, `99.8%`, `1 day`, `3 days`.

pub mod alerts;
pub mod client_reports;
pub mod issues;
pub mod readiness;

/// `12,345`.
pub(crate) fn format_count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `99.8%`: one decimal, like the UI.
pub(crate) fn format_pct(pct: f64) -> String {
    format!("{pct:.1}%")
}

/// `1 day`, `1,204 messages`: the count with its noun, as the UI's `plural`.
pub(crate) fn count_noun(n: i64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{} {noun}s", format_count(n))
    }
}

/// [`count_noun`] of a length: `1 change`, `3 changes`.
pub(crate) fn plural(n: usize, noun: &str) -> String {
    count_noun(i64::try_from(n).unwrap_or(i64::MAX), noun)
}

/// `one` for 1, else `many`: `was` / `were`.
pub(crate) fn one_or_many(n: i64, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// `text` with its first letter in upper case.
pub(crate) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `a`, `a and b`, `a, b and c`.
pub(crate) fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_like_the_ui() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(format_count(-12_345), "-12,345");
        assert_eq!(format_pct(99.84), "99.8%");
        assert_eq!(format_pct(100.0), "100.0%");
        assert_eq!(count_noun(1, "day"), "1 day");
        assert_eq!(count_noun(1_204, "message"), "1,204 messages");
        assert_eq!(plural(0, "change"), "0 changes");
        assert_eq!(capitalized("failing mail"), "Failing mail");
        assert_eq!(join_and(&["a", "b", "c"]), "a, b and c");
        assert_eq!(join_and(&["a"]), "a");
    }
}
