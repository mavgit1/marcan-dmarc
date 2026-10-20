//! DMARC records to publish: a new monitoring record, a published one with
//! the report address added to `rua=`, or moved to another policy. Every tag
//! the change does not name stays exactly as published (its case, spacing
//! and order), so what a user pastes into DNS is their own record with one
//! change; the tags are found as [`parse_dmarc_record`](crate::dmarc::parse_dmarc_record)
//! reads them (`;`-separated, the name before the first `=`, any case).

/// A monitoring record for a domain without DMARC: `p=none` only asks for
/// reports and does not affect delivery.
pub fn new_record(report_address: &str) -> String {
    format!("v=DMARC1; p=none; rua=mailto:{report_address}")
}

/// Whether `part` (one `;`-separated piece of a record) is the tag `name`.
fn is_tag(part: &str, name: &str) -> bool {
    part.split_once('=')
        .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case(name))
}

/// `record` with `name=value` after its last tag (and any trailing `;`).
fn with_appended_tag(record: &str, name: &str, value: &str) -> String {
    let base = record
        .trim()
        .trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    format!("{base}; {name}={value}")
}

/// `part`'s text and the whitespace that ends it.
fn split_trailing(part: &str) -> (&str, &str) {
    let body = part.trim_end();
    (body, &part[body.len()..])
}

/// `record` with `mailto:<report_address>` added to its `rua=` list (or a
/// new `rua=` tag when it has none, or fills an empty one). Every other tag,
/// `p=` included, is left as published.
pub fn with_report_address(record: &str, report_address: &str) -> String {
    let uri = format!("mailto:{report_address}");
    let mut parts: Vec<String> = record.split(';').map(str::to_owned).collect();
    // The parser reads the last `rua=` when a record repeats it.
    let Some(i) = parts.iter().rposition(|p| is_tag(p, "rua")) else {
        return with_appended_tag(record, "rua", &uri);
    };
    let (body, trailing) = split_trailing(&parts[i]);
    let value = body.split_once('=').map_or("", |(_, v)| v);
    let separator = if value.trim().is_empty() { "" } else { "," };
    parts[i] = format!("{body}{separator}{uri}{trailing}");
    parts.join(";")
}

/// `record` with every tag `name` set to `value`, or removed when `value`
/// is `None`; a tag it lacked goes at the end.
fn with_tag(record: &str, name: &str, value: Option<&str>) -> String {
    let parts: Vec<&str> = record.split(';').collect();
    if !parts.iter().any(|p| is_tag(p, name)) {
        return match value {
            Some(v) => with_appended_tag(record, name, v),
            None => record.to_owned(),
        };
    }
    parts
        .into_iter()
        .filter_map(|part| {
            if !is_tag(part, name) {
                return Some(part.to_owned());
            }
            let v = value?;
            let (body, trailing) = split_trailing(part);
            let (key, old) = body.split_once('=').expect("a tag has a `=`");
            // The spacing around `=` as published.
            let gap = &old[..old.len() - old.trim_start().len()];
            Some(format!("{key}={gap}{v}{trailing}"))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// `record` moved to policy `p` (`quarantine`, `reject`) at `pct` percent:
/// `pct=` is left out at 100 (its default); every other tag stays as
/// published.
pub fn with_policy(record: &str, p: &str, pct: i64) -> String {
    let pct = (pct < 100).then(|| pct.to_string());
    with_tag(&with_tag(record, "p", Some(p)), "pct", pct.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmarc::parse_dmarc_record;

    const ADDR: &str = "dmarc@reports.example.net";

    fn rua(record: &str) -> Vec<String> {
        parse_dmarc_record(record)
            .rua
            .into_iter()
            .map(|u| u.raw)
            .collect()
    }

    #[test]
    fn a_new_record_monitors_and_names_the_address() {
        let record = new_record(ADDR);
        assert_eq!(
            record,
            "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net"
        );
        let parsed = parse_dmarc_record(&record);
        assert_eq!(parsed.p.as_deref(), Some("none"));
        assert_eq!(rua(&record), [format!("mailto:{ADDR}")]);
    }

    #[test]
    fn the_report_address_joins_the_published_rua() {
        let out = with_report_address("v=DMARC1; p=reject; rua=mailto:a@example.com", ADDR);
        assert_eq!(
            out,
            format!("v=DMARC1; p=reject; rua=mailto:a@example.com,mailto:{ADDR}")
        );
        assert_eq!(
            rua(&out),
            ["mailto:a@example.com".to_owned(), format!("mailto:{ADDR}")]
        );
        assert_eq!(parse_dmarc_record(&out).p.as_deref(), Some("reject"));
        // Whitespace and the tags after rua stay as published.
        assert_eq!(
            with_report_address("v=DMARC1; rua=mailto:a@example.com ; p=none", ADDR),
            format!("v=DMARC1; rua=mailto:a@example.com,mailto:{ADDR} ; p=none")
        );
        // Matched in any case; ruf is not rua.
        assert_eq!(
            with_report_address(
                "v=DMARC1; ruf=mailto:f@example.com; RUA=mailto:a@example.com",
                ADDR
            ),
            format!("v=DMARC1; ruf=mailto:f@example.com; RUA=mailto:a@example.com,mailto:{ADDR}")
        );
    }

    #[test]
    fn an_empty_or_missing_rua_gets_the_address() {
        assert_eq!(
            with_report_address("v=DMARC1; p=none; rua=", ADDR),
            format!("v=DMARC1; p=none; rua=mailto:{ADDR}")
        );
        let out = with_report_address("v=DMARC1; p=quarantine;  ", ADDR);
        assert_eq!(out, format!("v=DMARC1; p=quarantine; rua=mailto:{ADDR}"));
        assert_eq!(rua(&out), [format!("mailto:{ADDR}")]);
    }

    #[test]
    fn a_policy_step_changes_p_and_pct_only() {
        let out = with_policy(
            "v=DMARC1; p=none; sp=none; rua=mailto:a@example.com",
            "quarantine",
            100,
        );
        assert_eq!(
            out,
            "v=DMARC1; p=quarantine; sp=none; rua=mailto:a@example.com"
        );
        let parsed = parse_dmarc_record(&out);
        assert_eq!(parsed.p.as_deref(), Some("quarantine"));
        assert_eq!(parsed.sp.as_deref(), Some("none"));
        assert_eq!(parsed.pct, None);
        // pct=100 is the default, so it goes; any other value is set.
        assert_eq!(
            with_policy(
                "v=DMARC1; p=quarantine; pct=20; rua=mailto:a@example.com",
                "quarantine",
                100
            ),
            "v=DMARC1; p=quarantine; rua=mailto:a@example.com"
        );
        let out = with_policy("v=DMARC1; P = none ;", "reject", 50);
        assert_eq!(out, "v=DMARC1; P = reject; pct=50");
        let parsed = parse_dmarc_record(&out);
        assert_eq!(
            (parsed.p.as_deref(), parsed.pct),
            (Some("reject"), Some(50))
        );
        assert_eq!(
            with_policy("v=DMARC1; p=none; pct=10", "quarantine", 25),
            "v=DMARC1; p=quarantine; pct=25"
        );
    }

    #[test]
    fn a_repeated_tag_is_set_everywhere_the_parser_could_read_it() {
        let out = with_policy("v=DMARC1; p=none; p=none", "reject", 100);
        assert_eq!(out, "v=DMARC1; p=reject; p=reject");
        assert_eq!(parse_dmarc_record(&out).p.as_deref(), Some("reject"));
    }
}
