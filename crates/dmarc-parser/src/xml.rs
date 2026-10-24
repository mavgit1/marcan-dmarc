//! Serde/quick-xml mapping from DMARC `feedback` documents.

use crate::types::{
    DkimAuthResult, PolicyOverride, PolicyPublished, Record, Report, SpfAuthResult,
};
use crate::ParseError;
use dmarc_domain::normalize_domain;
use serde::Deserialize;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[derive(Debug, Deserialize)]
struct Feedback {
    report_metadata: ReportMetadata,
    policy_published: XmlPolicyPublished,
    #[serde(default, rename = "record")]
    record: Vec<XmlRecord>,
}

#[derive(Debug, Deserialize)]
struct ReportMetadata {
    org_name: String,
    email: String,
    report_id: String,
    date_range: DateRange,
}

#[derive(Debug, Deserialize)]
struct DateRange {
    begin: i64,
    end: i64,
}

#[derive(Debug, Deserialize)]
struct XmlPolicyPublished {
    domain: String,
    adkim: Option<String>,
    aspf: Option<String>,
    p: Option<String>,
    sp: Option<String>,
    /// Read as text: some reporters send `<pct/>`, which is no number.
    pct: Option<String>,
    fo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XmlRecord {
    row: Row,
    #[serde(default)]
    identifiers: Option<Identifiers>,
    #[serde(default)]
    auth_results: Option<AuthResults>,
}

#[derive(Debug, Deserialize)]
struct Row {
    source_ip: String,
    count: i64,
    #[serde(default)]
    policy_evaluated: Option<PolicyEvaluated>,
}

#[derive(Debug, Deserialize)]
struct PolicyEvaluated {
    disposition: Option<String>,
    dkim: Option<String>,
    spf: Option<String>,
    #[serde(default, rename = "reason")]
    reason: Vec<XmlReason>,
}

#[derive(Debug, Deserialize)]
struct XmlReason {
    #[serde(rename = "type")]
    reason_type: Option<String>,
    comment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Identifiers {
    header_from: Option<String>,
    envelope_from: Option<String>,
    envelope_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthResults {
    #[serde(default, rename = "dkim")]
    dkim: Vec<XmlDkim>,
    #[serde(default, rename = "spf")]
    spf: Vec<XmlSpf>,
}

#[derive(Debug, Deserialize)]
struct XmlDkim {
    /// Some reporters omit it; such a result simply never aligns.
    #[serde(default)]
    domain: String,
    selector: Option<String>,
    result: String,
}

#[derive(Debug, Deserialize)]
struct XmlSpf {
    domain: String,
    result: String,
    scope: Option<String>,
}

/// Largest `count` one record may carry. A record is one source IP seen by
/// one receiver over (usually) one day; a billion is far beyond any real
/// sender, so a larger value is invalid input, not traffic.
pub(crate) const MAX_RECORD_COUNT: i64 = 1_000_000_000;

/// How far `date_range.begin` may lie in the future (clock skew, time zones).
const MAX_FUTURE_BEGIN_SECS: i64 = 24 * 60 * 60;

/// Parse a decompressed DMARC feedback XML document into a [`Report`].
pub fn parse_feedback_xml(raw_xml: String) -> Result<Report, ParseError> {
    let feedback: Feedback =
        quick_xml::de::from_str(&raw_xml).map_err(|e| ParseError::Xml(e.to_string()))?;

    // The only check of the report period: everything downstream trusts it.
    let DateRange { begin, end } = feedback.report_metadata.date_range;
    if end < begin {
        return Err(ParseError::Invalid(format!(
            "date_range ends ({end}) before it begins ({begin})"
        )));
    }
    if begin > unix_now().saturating_add(MAX_FUTURE_BEGIN_SECS) {
        return Err(ParseError::Invalid(format!(
            "date_range begins in the future ({begin})"
        )));
    }

    if feedback.report_metadata.report_id.is_empty() {
        return Err(ParseError::MissingField("report_id"));
    }
    if feedback.policy_published.domain.trim().is_empty() {
        return Err(ParseError::MissingField("policy_published.domain"));
    }
    // The report decides which monitored domain it lands in, and the domain is
    // then checked over DNS and HTTPS: anything but a hostname is refused.
    let domain = normalize_domain(&feedback.policy_published.domain).map_err(|e| {
        ParseError::Invalid(format!(
            "policy_published.domain {:?}: {e}",
            feedback.policy_published.domain
        ))
    })?;

    let records: Vec<Record> = feedback
        .record
        .into_iter()
        .map(|r| {
            if !(0..=MAX_RECORD_COUNT).contains(&r.row.count) {
                return Err(ParseError::Invalid(format!(
                    "message count {} is outside 0..={MAX_RECORD_COUNT}",
                    r.row.count
                )));
            }
            // One spelling per address, so evidence stored under it is found again.
            let source_ip = r
                .row
                .source_ip
                .trim()
                .parse::<IpAddr>()
                .map_err(|_| {
                    ParseError::Invalid(format!(
                        "source_ip {:?} is not an IP address",
                        r.row.source_ip
                    ))
                })?
                .to_string();
            let pe = r.row.policy_evaluated.as_ref();
            let ids = r.identifiers.as_ref();
            let auth = r.auth_results.as_ref();

            Ok(Record {
                source_ip,
                count: r.row.count,
                disposition: pe.and_then(|p| p.disposition.clone()),
                dkim_result: pe.and_then(|p| p.dkim.clone()),
                spf_result: pe.and_then(|p| p.spf.clone()),
                header_from: ids.and_then(|i| i.header_from.clone()),
                envelope_from: ids.and_then(|i| i.envelope_from.clone()),
                envelope_to: ids.and_then(|i| i.envelope_to.clone()),
                dkim_auth: auth
                    .map(|a| {
                        a.dkim
                            .iter()
                            .map(|d| DkimAuthResult {
                                domain: d.domain.clone(),
                                selector: d.selector.clone(),
                                result: d.result.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                spf_auth: auth
                    .map(|a| {
                        a.spf
                            .iter()
                            .map(|s| SpfAuthResult {
                                domain: s.domain.clone(),
                                result: s.result.clone(),
                                scope: s.scope.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                policy_overrides: pe
                    .map(|p| {
                        p.reason
                            .iter()
                            .map(|r| PolicyOverride {
                                reason_type: r.reason_type.clone(),
                                comment: r.comment.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect::<Result<_, ParseError>>()?;

    Ok(Report {
        report_id: feedback.report_metadata.report_id,
        org_name: feedback.report_metadata.org_name,
        email: feedback.report_metadata.email,
        date_begin: begin,
        date_end: end,
        policy: PolicyPublished {
            domain,
            adkim: feedback.policy_published.adkim,
            aspf: feedback.policy_published.aspf,
            p: feedback.policy_published.p,
            sp: feedback.policy_published.sp,
            pct: feedback
                .policy_published
                .pct
                .as_deref()
                .and_then(|p| p.trim().parse().ok()),
            fo: feedback.policy_published.fo,
        },
        records,
        raw_xml,
    })
}
