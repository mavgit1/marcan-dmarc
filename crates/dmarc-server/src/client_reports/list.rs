//! The recipient list as text: one `organization,email` per line. Pure: the
//! text and the existing organizations in, recipients and line errors out.
//!
//! Organizations are matched by name, trimmed and ignoring case. A name that
//! matches none is a line error with the closest existing name as a
//! suggestion: the list never creates an organization and never skips a line.
//! An optional `organization,email` header, blank lines and `#` comments are
//! skipped. Several lines may name one organization; the same address twice
//! for one organization is an error.

use crate::mail::parse_address;
use crate::view::LineError;
use dmarc_store::{ClientReportRecipient, ScopeOrganization};
use std::collections::HashMap;

/// The list as parsed: every valid line's recipient, and every line error.
#[derive(Debug, Default)]
pub struct ParsedRecipients {
    pub recipients: Vec<ClientReportRecipient>,
    pub errors: Vec<LineError>,
}

/// Edits from `a` to `b` (insert, delete, substitute one character).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// The existing name `name` most likely meant: the nearest by edits (at most
/// a third of the longer name, and at least two edits allowed), or one that
/// contains the other. `None` when nothing is that close.
fn closest<'a>(name: &str, organizations: &'a [ScopeOrganization]) -> Option<&'a str> {
    let typed = name.to_lowercase();
    organizations
        .iter()
        .filter_map(|org| {
            let known = org.name.to_lowercase();
            let d = distance(&typed, &known);
            let longer = typed.chars().count().max(known.chars().count());
            let near = d <= (longer / 3).max(2);
            let contained = typed.len() >= 3 && (known.contains(&typed) || typed.contains(&known));
            (near || contained).then_some((d, org.name.as_str()))
        })
        .min_by_key(|&(d, name)| (d, name.to_lowercase()))
        .map(|(_, name)| name)
}

/// Parse the list against the organizations that exist.
pub fn parse_recipients(text: &str, organizations: &[ScopeOrganization]) -> ParsedRecipients {
    let mut parsed = ParsedRecipients::default();
    let by_name: HashMap<String, &ScopeOrganization> = organizations
        .iter()
        .map(|org| (org.name.trim().to_lowercase(), org))
        .collect();
    // (organization id, lowercased address) → the line it is on.
    let mut seen: HashMap<(i64, String), usize> = HashMap::new();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());

    let mut first = true;
    for record in reader.records() {
        let record = match record {
            Ok(record) => record,
            Err(e) => {
                let line = e.position().map_or(0, |p| p.line() as usize);
                parsed.errors.push(LineError {
                    line,
                    message: e.to_string(),
                });
                continue;
            }
        };
        if record.iter().all(str::is_empty) {
            continue;
        }
        let line = record.position().map_or(0, |p| p.line() as usize);
        let mut error = |message: String| parsed.errors.push(LineError { line, message });
        let field = |i: usize| record.get(i).unwrap_or_default();
        let (org, email) = (field(0), field(1));
        let is_header = first
            && record.len() == 2
            && org.eq_ignore_ascii_case("organization")
            && email.eq_ignore_ascii_case("email");
        first = false;
        if is_header {
            continue;
        }
        if record.len() != 2 {
            error(format!(
                "expected organization,email, got {} column{}",
                record.len(),
                if record.len() == 1 { "" } else { "s" }
            ));
            continue;
        }
        if org.is_empty() {
            error(format!("{email} has no organization"));
            continue;
        }
        let Some(organization) = by_name.get(&org.to_lowercase()) else {
            error(match closest(org, organizations) {
                Some(suggestion) => {
                    format!("no organization is named {org}: did you mean {suggestion}?")
                }
                None => format!(
                    "no organization is named {org}: add it in the Domains list or on the Organizations page first"
                ),
            });
            continue;
        };
        if email.is_empty() {
            error(format!("{} has no email address", organization.name));
            continue;
        }
        let Some(address) = parse_address(email) else {
            error(format!("not a valid email address: {email}"));
            continue;
        };
        let email = address.to_string();
        match seen.get(&(organization.id, email.to_ascii_lowercase())) {
            Some(other) => error(format!(
                "{email} is already listed for {} (line {other})",
                organization.name
            )),
            None => {
                seen.insert((organization.id, email.to_ascii_lowercase()), line);
                parsed.recipients.push(ClientReportRecipient {
                    organization_id: organization.id,
                    organization: organization.name.clone(),
                    email,
                });
            }
        }
    }
    parsed
}

/// `recipients` as the text [`parse_recipients`] reads (organization names
/// with a comma quoted).
pub fn format_recipients(recipients: &[ClientReportRecipient]) -> String {
    crate::csv_text::lf_text(
        recipients
            .iter()
            .map(|r| [r.organization.as_str(), r.email.as_str()]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orgs() -> Vec<ScopeOrganization> {
        [
            (1, "Default"),
            (2, "Acme"),
            (3, "Globex, Inc"),
            (4, "Initech"),
        ]
        .into_iter()
        .map(|(id, name)| ScopeOrganization {
            id,
            name: name.into(),
        })
        .collect()
    }

    fn errors(text: &str) -> Vec<(usize, String)> {
        parse_recipients(text, &orgs())
            .errors
            .into_iter()
            .map(|e| (e.line, e.message))
            .collect()
    }

    #[test]
    fn reads_comments_the_header_and_several_lines_per_organization() {
        let parsed = parse_recipients(
            "Organization,Email\n# who gets what\n\n acme , it@acme.example \nACME,boss@acme.example\n\"Globex, Inc\",ops@globex.example\nInitech,it@acme.example\n",
            &orgs(),
        );
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let got: Vec<(i64, &str, &str)> = parsed
            .recipients
            .iter()
            .map(|r| (r.organization_id, r.organization.as_str(), r.email.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (2, "Acme", "it@acme.example"),
                (2, "Acme", "boss@acme.example"),
                (3, "Globex, Inc", "ops@globex.example"),
                // One address may get several organizations' reports.
                (4, "Initech", "it@acme.example"),
            ]
        );
        assert_eq!(format_recipients(&parsed.recipients), "Acme,it@acme.example\nAcme,boss@acme.example\n\"Globex, Inc\",ops@globex.example\nInitech,it@acme.example\n");
        let again = parse_recipients(&format_recipients(&parsed.recipients), &orgs());
        assert_eq!(again.recipients, parsed.recipients);
    }

    #[test]
    fn an_unknown_organization_is_an_error_with_a_suggestion() {
        assert_eq!(
            errors("Acmee,it@acme.example\nglobex,ops@globex.example\nUmbrella,x@umbrella.example\n"),
            [
                (1, "no organization is named Acmee: did you mean Acme?".to_owned()),
                (2, "no organization is named globex: did you mean Globex, Inc?".to_owned()),
                (
                    3,
                    "no organization is named Umbrella: add it in the Domains list or on the Organizations page first".to_owned()
                ),
            ]
        );
        // A header only counts on the first line.
        assert_eq!(
            errors("Acme,it@acme.example\norganization,email\n"),
            [(2, "no organization is named organization: add it in the Domains list or on the Organizations page first".to_owned())]
        );
    }

    #[test]
    fn invalid_addresses_columns_and_repeats_are_errors() {
        assert_eq!(
            errors("Acme,it@acme\nAcme\nAcme,a@acme.example,extra\n,a@acme.example\nAcme,\nAcme,a@acme.example\nacme,A@ACME.example\n"),
            [
                (1, "not a valid email address: it@acme".to_owned()),
                (2, "expected organization,email, got 1 column".to_owned()),
                (3, "expected organization,email, got 3 columns".to_owned()),
                (4, "a@acme.example has no organization".to_owned()),
                (5, "Acme has no email address".to_owned()),
                (7, "A@ACME.example is already listed for Acme (line 6)".to_owned()),
            ]
        );
    }

    #[test]
    fn distance_counts_edits() {
        assert_eq!(distance("acme", "acme"), 0);
        assert_eq!(distance("acmee", "acme"), 1);
        assert_eq!(distance("initeck", "initech"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(closest("Intech", &orgs()), Some("Initech"));
        assert_eq!(closest("xyz", &orgs()), None);
    }
}
