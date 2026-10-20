//! A record's SPF and DKIM checks and override reasons, read as JSON arrays
//! per record ([`RECORD_CHECKS_JSON`]) by the report view, the Failures tab and
//! the auth-issues export.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One SPF check of a record, as stored at ingest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
pub struct SpfCheckRow {
    pub domain: Option<String>,
    pub scope: Option<String>,
    pub result: Option<String>,
    /// Aligned with `header_from` (PSL, relaxed/strict per `aspf`), computed at ingest.
    #[serde(deserialize_with = "de_flag")]
    pub aligned: bool,
}

/// One DKIM signature check of a record, as stored at ingest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
pub struct DkimCheckRow {
    pub domain: Option<String>,
    pub selector: Option<String>,
    pub result: Option<String>,
    /// Aligned with `header_from` (PSL, relaxed/strict per `adkim`), computed at ingest.
    #[serde(deserialize_with = "de_flag")]
    pub aligned: bool,
}

/// One policy override reason the receiver gave for a record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct OverrideReason {
    #[serde(rename = "type")]
    pub reason_type: Option<String>,
    pub comment: Option<String>,
}

/// SQLite JSON carries the stored `aligned` as `0` / `1` (or `null`).
fn de_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Option::<i64>::deserialize(d)?.is_some_and(|v| v != 0))
}

pub(crate) fn parse_json_list<T: serde::de::DeserializeOwned>(raw: Option<&str>) -> Vec<T> {
    raw.and_then(|r| serde_json::from_str(r).ok())
        .unwrap_or_default()
}

/// Correlated subqueries over `records rec`: every SPF / DKIM check and override of
/// the record as a JSON array in stored (report) order, as columns `spf_auth`,
/// `dkim_auth` and `overrides`. Also part of the Failures tab's grouping key,
/// so a row's checks hold for every record it aggregates.
pub(crate) const RECORD_CHECKS_JSON: &str = r#"
                (SELECT json_group_array(json_object(
                            'domain', a.domain, 'scope', a.scope,
                            'result', a.result, 'aligned', a.aligned))
                 FROM (SELECT * FROM record_spf_auth WHERE record_id = rec.id ORDER BY id) a
                ) AS spf_auth,
                (SELECT json_group_array(json_object(
                            'domain', a.domain, 'selector', a.selector,
                            'result', a.result, 'aligned', a.aligned))
                 FROM (SELECT * FROM record_dkim_auth WHERE record_id = rec.id ORDER BY id) a
                ) AS dkim_auth,
                (SELECT json_group_array(json_object('type', o.reason_type, 'comment', o.comment))
                 FROM (SELECT * FROM record_overrides WHERE record_id = rec.id ORDER BY id) o
                ) AS overrides"#;

/// SPF and DKIM checks and override reasons of a record, in stored order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordChecks {
    pub spf: Vec<SpfCheckRow>,
    pub dkim: Vec<DkimCheckRow>,
    pub overrides: Vec<OverrideReason>,
}

impl RecordChecks {
    /// From the three [`RECORD_CHECKS_JSON`] columns.
    pub(crate) fn from_json(
        spf_auth: Option<&str>,
        dkim_auth: Option<&str>,
        overrides: Option<&str>,
    ) -> Self {
        Self {
            spf: parse_json_list(spf_auth),
            dkim: parse_json_list(dkim_auth),
            overrides: parse_json_list(overrides),
        }
    }
}
