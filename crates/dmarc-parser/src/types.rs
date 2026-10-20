//! Typed DMARC aggregate report structures ready for storage.

use serde::{Deserialize, Serialize};

/// Fully parsed DMARC aggregate report and its raw XML. The totals and every
/// verdict are derived at ingest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub report_id: String,
    pub org_name: String,
    pub email: String,
    pub date_begin: i64,
    pub date_end: i64,
    pub policy: PolicyPublished,
    pub records: Vec<Record>,
    /// Original XML text after decompression (stored compressed in `report_xml`).
    pub raw_xml: String,
}

/// Published DMARC policy as declared in the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPublished {
    pub domain: String,
    pub adkim: Option<String>,
    pub aspf: Option<String>,
    pub p: Option<String>,
    pub sp: Option<String>,
    pub pct: Option<i32>,
    /// Failure reporting options (`fo` in the XML).
    pub fo: Option<String>,
}

/// One `<record>` row from the aggregate report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub source_ip: String,
    pub count: i64,
    pub disposition: Option<String>,
    /// Policy-evaluated DKIM result (`pass` / `fail`).
    pub dkim_result: Option<String>,
    /// Policy-evaluated SPF result (`pass` / `fail`).
    pub spf_result: Option<String>,
    pub header_from: Option<String>,
    pub envelope_from: Option<String>,
    pub envelope_to: Option<String>,
    pub dkim_auth: Vec<DkimAuthResult>,
    pub spf_auth: Vec<SpfAuthResult>,
    /// Policy override reasons from `<policy_evaluated><reason>`.
    pub policy_overrides: Vec<PolicyOverride>,
}

/// DKIM authentication result from `<auth_results>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DkimAuthResult {
    pub domain: String,
    pub selector: Option<String>,
    pub result: String,
}

/// SPF authentication result from `<auth_results>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpfAuthResult {
    pub domain: String,
    pub result: String,
    /// SPF scope (`mfrom` / `helo`), when the reporter included it.
    pub scope: Option<String>,
}

/// A `<reason>` entry under `<policy_evaluated>` (e.g. forwarded, sampled).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyOverride {
    pub reason_type: Option<String>,
    pub comment: Option<String>,
}
