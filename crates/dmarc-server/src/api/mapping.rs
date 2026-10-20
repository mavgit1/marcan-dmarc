//! Bulk organization ↔ domain mapping as text, a list like the others
//! ([`ListText`]):
//!
//! `GET /api/settings/domain-mapping` — the current mapping.
//! `POST /api/settings/domain-mapping/preview` — a draft → what saving it would
//! change, no writes.
//! `PUT /api/settings/domain-mapping` — a draft → the same summary, applied in
//! one transaction; refused with `errors` when the text has errors. When it
//! monitors an ignored domain again, the mailbox is re-imported for its history.
//!
//! Format: CSV, one `organization, domain[, status]` per line; the status is
//! empty, `parked` (sends no mail) or `ignored` (removed: its reports are
//! deleted and new ones dropped). An optional header row, blank lines and `#`
//! comments are skipped. Listed domains are moved to or added to their
//! organization (created when no organization has that name, matched
//! case-insensitively) and given the listed status. Domains that are not
//! listed are left alone.

use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::view::{LineError, ListText};
use crate::AppState;
use axum::extract::State;
use dmarc_domain::{normalize_domain, DomainError};
use dmarc_store::{DomainStatus, MappingChanges, MappingEntry, MappingPlan};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct MappingSummary {
    pub changes: MappingChanges,
    pub unchanged_count: usize,
    pub errors: Vec<LineError>,
}

/// Parsed mapping: one entry per distinct domain, plus every line error.
#[derive(Debug, Default)]
pub struct ParsedMapping {
    pub entries: Vec<MappingEntry>,
    pub errors: Vec<LineError>,
}

/// Parse the mapping CSV. Organization names are matched case-insensitively,
/// so `Acme` and `ACME` are the same organization (first spelling wins).
pub fn parse_mapping(text: &str) -> ParsedMapping {
    let mut parsed = ParsedMapping::default();
    // Lowercased org name → first spelling.
    let mut org_names: HashMap<String, String> = HashMap::new();
    // Domain → (lowercased org, display org, line).
    let mut seen: HashMap<String, (String, String, usize)> = HashMap::new();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());

    for (index, record) in reader.records().enumerate() {
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
        let line = record.position().map_or(0, |p| p.line() as usize);
        let mut error = |message: String| parsed.errors.push(LineError { line, message });
        let field = |i: usize| record.get(i).unwrap_or_default();
        let (org, value, flag) = (field(0), field(1), field(2));
        if record.iter().all(str::is_empty) {
            continue;
        }
        if index == 0 && org.eq_ignore_ascii_case("organization") {
            continue;
        }
        if record.len() < 2 || record.len() > 3 {
            error(format!(
                "expected organization, domain[, parked|ignored], got {} column{}",
                record.len(),
                if record.len() == 1 { "" } else { "s" }
            ));
            continue;
        }
        let domain = match normalize_domain(value) {
            Ok(domain) => domain,
            Err(DomainError::Empty) => {
                error("domain is empty".to_owned());
                continue;
            }
            Err(DomainError::TooLong) => {
                error(format!("domain is too long: {value}"));
                continue;
            }
            Err(DomainError::Invalid) => {
                error(format!("not a valid domain: {value}"));
                continue;
            }
        };
        if org.is_empty() {
            error(format!("{domain} has no organization"));
            continue;
        }
        let status = match flag {
            "" => DomainStatus::Monitored,
            flag if flag.eq_ignore_ascii_case("parked") => DomainStatus::Parked,
            flag if flag.eq_ignore_ascii_case("ignored") => DomainStatus::Ignored,
            flag => {
                error(format!(
                    "the third column is `parked`, `ignored` or empty, not: {flag}"
                ));
                continue;
            }
        };
        let org = org_names
            .entry(org.to_lowercase())
            .or_insert_with(|| org.to_owned())
            .clone();
        let key = org.to_lowercase();
        match seen.get(&domain) {
            Some((other_key, _, _)) if *other_key == key => {}
            Some((_, other_org, other_line)) => error(format!(
                "{domain} is already listed under {other_org} (line {other_line})"
            )),
            None => {
                seen.insert(domain.clone(), (key, org.clone(), line));
                parsed.entries.push(MappingEntry {
                    domain,
                    org,
                    status,
                });
            }
        }
    }
    parsed
}

/// `rows` (`domain, organization, status`) as the CSV [`parse_mapping`] reads:
/// organization first, so a sorted list reads as groups.
pub fn format_mapping(rows: &[(String, String, DomainStatus)]) -> String {
    crate::csv_text::lf_text(rows.iter().map(|(domain, org, status)| {
        let status = match status {
            DomainStatus::Monitored => None,
            DomainStatus::Parked => Some("parked"),
            DomainStatus::Ignored => Some("ignored"),
        };
        [org.as_str(), domain.as_str()].into_iter().chain(status)
    }))
}

fn summary(plan: MappingPlan, errors: Vec<LineError>) -> MappingSummary {
    MappingSummary {
        changes: plan.changes,
        unchanged_count: plan.unchanged_count,
        errors,
    }
}

pub async fn get_mapping(State(state): State<Arc<AppState>>) -> AppResult<Json<ListText>> {
    let text = format_mapping(&dmarc_store::domain_mapping_rows(&state.pool).await?);
    Ok(Json(ListText { text }))
}

/// Summary of what applying `text` would change. Lines with errors are left
/// out of the changes and reported in `errors`.
pub async fn preview_mapping(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<MappingSummary>> {
    let parsed = parse_mapping(&req.text);
    let plan = dmarc_store::preview_domain_mapping(&state.pool, &parsed.entries).await?;
    Ok(Json(summary(plan, parsed.errors)))
}

/// Apply `text` atomically. Any line error rejects the whole mapping (with
/// the same `errors` list as the preview) and nothing is written. A domain
/// monitored again gets its history from a mailbox re-import.
pub async fn apply_mapping(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<MappingSummary>> {
    let parsed = parse_mapping(&req.text);
    if !parsed.errors.is_empty() {
        return Err(rejected_lines(&parsed.errors, "mapping"));
    }
    let plan = dmarc_store::apply_domain_mapping(&state.pool, &parsed.entries).await?;
    if !plan.changes.unignore.is_empty() {
        state.imap.request_reimport();
    }
    Ok(Json(summary(plan, Vec::new())))
}

/// The `422` of a text list with line errors: how many, and every one as
/// `errors`. Nothing was written.
pub(super) fn rejected_lines(errors: &[LineError], list: &str) -> AppError {
    let count = errors.len();
    AppError::unprocessable(format!(
        "the {list} has {count} error{}; nothing was changed",
        if count == 1 { "" } else { "s" }
    ))
    .with_line_errors(errors.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rows_headers_comments_and_flags() {
        let parsed = parse_mapping(
            "organization,domain,parked\n# comment\nAcme, Example.COM.\nAcme, b.example, PARKED\n\nacme,c.example,Ignored\n\"Other, Inc\",d.example\nOther,e.example,later\n",
        );
        assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
        assert_eq!(parsed.errors[0].line, 8);
        assert!(parsed.errors[0].message.contains("later"));
        let got: Vec<_> = parsed
            .entries
            .iter()
            .map(|e| (e.domain.as_str(), e.org.as_str(), e.status))
            .collect();
        assert_eq!(
            got,
            [
                ("example.com", "Acme", DomainStatus::Monitored),
                ("b.example", "Acme", DomainStatus::Parked),
                ("c.example", "Acme", DomainStatus::Ignored),
                ("d.example", "Other, Inc", DomainStatus::Monitored),
            ]
        );
    }

    #[test]
    fn format_round_trips_quoted_names() {
        let rows = vec![
            (
                "a.example".to_owned(),
                "Acme, Inc".to_owned(),
                DomainStatus::Monitored,
            ),
            (
                "b.example".to_owned(),
                "Beta".to_owned(),
                DomainStatus::Parked,
            ),
            (
                "c.example".to_owned(),
                "Beta".to_owned(),
                DomainStatus::Ignored,
            ),
        ];
        let text = format_mapping(&rows);
        assert_eq!(
            text,
            "\"Acme, Inc\",a.example\nBeta,b.example,parked\nBeta,c.example,ignored\n"
        );
        assert_eq!(parse_mapping(&text).entries.len(), 3);
    }
}
