//! The recipient list as text: one `organization,email` per line, a
//! projection of the organizations like the Domains list. Pure: the text and
//! the existing organizations in, the organizations it names, their
//! recipients and the line errors out.
//!
//! An organization named in the text gets exactly the recipients named for
//! it (several lines for one organization form its set); `organization,none`
//! names it with none, so saving clears its recipients. An organization the
//! text does not name keeps its recipients, so a line taken out changes
//! nothing. [`format_recipients`] writes every organization, `none` for one
//! without a recipient.
//!
//! Organizations are matched by name, trimmed and ignoring case. A name that
//! matches none is a line error with the closest existing name as a
//! suggestion: the list never creates an organization and never skips a line.
//! An optional `organization,email` header, blank lines and `#` comments are
//! skipped. The same address twice for one organization is an error, and so
//! is `none` beside an address for it.

use crate::mail::parse_address;
use crate::view::LineError;
use dmarc_store::{ClientReportRecipient, ScopeOrganization};
use std::collections::HashMap;

/// The value that names an organization with no recipient: `Acme,none`.
const NONE: &str = "none";

/// The list as parsed: the organizations it names (once each, in the order
/// they first appear), every valid line's recipient, and every line error.
#[derive(Debug, Default)]
pub struct ParsedRecipients {
    pub organizations: Vec<i64>,
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
    // (organization id, lowercased address or `none`) → the line it is on.
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
        // Unlike the Domains list, an unknown name never creates an
        // organization: a typo would make a new, empty one and send that
        // client's report to nobody, with the real organization unchanged.
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
        let none_key = (organization.id, NONE.to_owned());
        if email.eq_ignore_ascii_case(NONE) {
            let listed = seen
                .iter()
                .filter(|((id, address), _)| *id == organization.id && address != NONE)
                .min_by_key(|(_, other)| **other);
            match (seen.get(&none_key), listed) {
                (Some(other), _) => error(format!(
                    "{} is already listed as none (line {other})",
                    organization.name
                )),
                (None, Some((_, other))) => error(format!(
                    "{} has an address on line {other}, so it cannot also be none",
                    organization.name
                )),
                (None, None) => {
                    seen.insert(none_key, line);
                    name(&mut parsed.organizations, organization.id);
                }
            }
            continue;
        }
        let Some(address) = parse_address(email) else {
            error(format!("not a valid email address: {email}"));
            continue;
        };
        let email = address.to_string();
        if let Some(other) = seen.get(&none_key) {
            error(format!(
                "{} is listed as none on line {other}: remove that line to add {email}",
                organization.name
            ));
            continue;
        }
        match seen.get(&(organization.id, email.to_ascii_lowercase())) {
            Some(other) => error(format!(
                "{email} is already listed for {} (line {other})",
                organization.name
            )),
            None => {
                seen.insert((organization.id, email.to_ascii_lowercase()), line);
                name(&mut parsed.organizations, organization.id);
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

/// Add `id` to the named organizations unless it is there.
fn name(organizations: &mut Vec<i64>, id: i64) {
    if !organizations.contains(&id) {
        organizations.push(id);
    }
}

/// Every organization in `organizations`' order with its `recipients`, as
/// the text [`parse_recipients`] reads: one line per recipient, or
/// `organization,none` without one (names with a comma quoted).
pub fn format_recipients(
    organizations: &[ScopeOrganization],
    recipients: &[ClientReportRecipient],
) -> String {
    crate::csv_text::lf_text(organizations.iter().flat_map(|org| {
        let mut lines: Vec<[&str; 2]> = recipients
            .iter()
            .filter(|r| r.organization_id == org.id)
            .map(|r| [org.name.as_str(), r.email.as_str()])
            .collect();
        if lines.is_empty() {
            lines.push([org.name.as_str(), NONE]);
        }
        lines
    }))
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
        assert_eq!(parsed.organizations, [2, 3, 4]);
    }

    #[test]
    fn none_names_an_organization_without_recipients() {
        let parsed = parse_recipients("Acme,None\nInitech,it@initech.example\n", &orgs());
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        // Acme is named (its recipients are cleared on save), with none.
        assert_eq!(parsed.organizations, [2, 4]);
        assert_eq!(parsed.recipients.len(), 1);
        assert_eq!(
            errors("Acme,none\nAcme,it@acme.example\nInitech,a@initech.example\nInitech,none\nAcme,none\n"),
            [
                (
                    2,
                    "Acme is listed as none on line 1: remove that line to add it@acme.example"
                        .to_owned()
                ),
                (4, "Initech has an address on line 3, so it cannot also be none".to_owned()),
                (5, "Acme is already listed as none (line 1)".to_owned()),
            ]
        );
    }

    #[test]
    fn the_text_lists_every_organization_and_reads_back_the_same() {
        let parsed = parse_recipients(
            "Acme,it@acme.example\nAcme,boss@acme.example\n\"Globex, Inc\",ops@globex.example\n",
            &orgs(),
        );
        let text = format_recipients(&orgs(), &parsed.recipients);
        assert_eq!(
            text,
            "Default,none\nAcme,it@acme.example\nAcme,boss@acme.example\n\"Globex, Inc\",ops@globex.example\nInitech,none\n"
        );
        let again = parse_recipients(&text, &orgs());
        assert!(again.errors.is_empty(), "{:?}", again.errors);
        assert_eq!(again.recipients, parsed.recipients);
        assert_eq!(again.organizations, [1, 2, 3, 4]);
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
