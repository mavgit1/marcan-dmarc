//! MTA-STS, TLS-RPT, and BIMI record parsing and evaluation.
//!
//! Every check in this file is optional to publish, so its findings are capped
//! at `Info` on the `dmarc_assess::dns` severity scale: BIMI is inbox logo
//! branding and cannot affect authentication at all, while TLS-RPT and MTA-STS
//! are reporting and transport-security niceties whose broken states leave the
//! domain exactly where it would be with no record published.
//!
//! The one exception is an `enforce` MTA-STS policy that pins no MX names.
//! Senders honouring that policy can match no host and will not deliver, so it
//! is a genuine delivery failure and stays a `Warning`.
//!
//! A name that only holds other protocols' records (a wildcard SPF answering
//! at `_mta-sts.<domain>`, say) counts as nothing published for the check, not
//! as a malformed record.

use crate::lookup::{parse_tag_list, LookupStatus, TxtLookup};
use dmarc_assess::{
    mta_sts_fetch_status, mta_sts_max_age_short, mta_sts_mode_status, CheckKind, Finding, Severity,
    Status, MTA_STS_GOOD_MAX_AGE, MTA_STS_MIN_MAX_AGE,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// True when the name answered with TXT records that all belong to some other
/// `v=` protocol (none matched the check's own prefix, or the caller would not
/// ask): nothing is published for this check, the name is just a wildcard.
fn only_other_protocols(lookup: &TxtLookup) -> bool {
    let records = lookup.records();
    !records.is_empty()
        && records.iter().all(|r| {
            r.trim()
                .as_bytes()
                .get(..2)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"v="))
        })
}

// --- MTA-STS ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMtaStsTxt {
    pub version: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParsedMtaStsPolicy {
    pub version: Option<String>,
    pub mode: Option<String>,
    pub max_age: Option<u64>,
    pub mx: Vec<String>,
}

/// How reading `https://mta-sts.<domain>/.well-known/mta-sts.txt` went.
///
/// Everything but `NotAttempted`, `Ok` and `Timeout` is the policy host
/// answering wrongly or not at all: receivers cannot apply the policy either,
/// so that is the domain's state, not a failed check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyFetch {
    /// Not fetched: without a `v=STSv1` TXT record no policy is in effect, so
    /// a file that happens to be served is not looked at.
    NotAttempted,
    /// HTTP 2xx with a body under the size cap. `content_type` is the
    /// response's media type, when the server sent one.
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        content_type: Option<String>,
    },
    /// `mta-sts.<domain>` does not resolve.
    NameNotFound,
    /// `mta-sts.<domain>` resolves to `address`, which is not on the public
    /// internet (loopback, private, link-local, ...): not fetched, so a
    /// report cannot point Marcan at an internal service.
    PrivateAddress { address: String },
    /// No TCP connection to `mta-sts.<domain>` on port 443.
    ConnectFailed,
    /// The TLS handshake failed, usually a certificate not valid for
    /// `mta-sts.<domain>`.
    TlsFailed { detail: String },
    /// The server redirected (to `location`, when it said where). RFC 8461
    /// §3.3 forbids following it.
    Redirect {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        location: Option<String>,
    },
    /// Any non-2xx, non-redirect status.
    HttpStatus { code: u16 },
    /// The body exceeded the cap or was not UTF-8.
    TooLarge,
    /// The request did not finish in time. Nothing was measured.
    Timeout,
    /// Any other transport error.
    Failed { detail: String },
}

impl PolicyFetch {
    /// The policy host answered wrongly or not at all (its problem, not ours).
    pub fn is_unreachable(&self) -> bool {
        !matches!(self, Self::NotAttempted | Self::Ok { .. } | Self::Timeout)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MtaStsCheckResult {
    pub domain: String,
    pub txt_name: String,
    pub txt_lookup: LookupStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub txt_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub policy_id: Option<String>,
    pub policy_url: String,
    /// How the policy file fetch went; `not_attempted` without a TXT record.
    pub fetch: PolicyFetch,
    /// Verdict on reaching the policy: `warn` when the host answered wrongly
    /// or not at all, else `neutral`.
    pub fetch_status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub policy: Option<ParsedMtaStsPolicy>,
    /// Verdict on the policy's `mode=`; `None` without a policy.
    pub mode_status: Option<Status>,
    pub findings: Vec<Finding>,
}

pub fn parse_mta_sts_txt(raw: &str) -> ParsedMtaStsTxt {
    let mut parsed = ParsedMtaStsTxt {
        version: None,
        id: None,
    };
    for (k, v, ok) in parse_tag_list(raw) {
        if !ok {
            continue;
        }
        match k.as_str() {
            "v" => parsed.version = Some(v),
            "id" => parsed.id = Some(v),
            _ => {}
        }
    }
    parsed
}

pub fn parse_mta_sts_policy(body: &str) -> ParsedMtaStsPolicy {
    let mut parsed = ParsedMtaStsPolicy {
        version: None,
        mode: None,
        max_age: None,
        mx: Vec::new(),
    };
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().to_string();
        match k.as_str() {
            "version" => parsed.version = Some(v),
            "mode" => parsed.mode = Some(v.to_ascii_lowercase()),
            "max_age" => parsed.max_age = v.parse().ok(),
            "mx" => parsed.mx.push(v),
            _ => {}
        }
    }
    parsed
}

/// Evaluate MTA-STS from the `_mta-sts` TXT lookup and the policy fetch.
///
/// The policy only counts when a `v=STSv1` record exists: receivers discover
/// MTA-STS through the record, so a file served without one is inactive and
/// is ignored here (callers do not fetch it in the first place).
pub fn evaluate_mta_sts(
    domain: &str,
    txt: &TxtLookup,
    policy_body: Option<&str>,
    fetch: &PolicyFetch,
) -> MtaStsCheckResult {
    let txt_name = format!("_mta-sts.{domain}");
    let policy_url = format!("https://mta-sts.{domain}/.well-known/mta-sts.txt");
    let mut findings = Vec::new();
    let records = txt.records_starting_with("v=stsv1");
    let txt_record = records.first().cloned();
    let parsed_txt = txt_record.as_deref().map(parse_mta_sts_txt);
    let mut result = MtaStsCheckResult {
        domain: domain.to_string(),
        txt_name,
        txt_lookup: txt.status(),
        txt_record: txt_record.clone(),
        policy_id: parsed_txt.as_ref().and_then(|p| p.id.clone()),
        policy_url: policy_url.clone(),
        fetch_status: mta_sts_fetch_status(fetch.is_unreachable()),
        fetch: fetch.clone(),
        policy: None,
        mode_status: None,
        findings: Vec::new(),
    };

    match txt {
        TxtLookup::Timeout => findings.push(Finding::notice(
            CheckKind::MtaSts,
            "mta_sts_txt_timeout",
            format!("DNS lookup for _mta-sts.{domain} timed out."),
            "Retry later. MTA-STS is optional but timeouts hide whether a policy exists.",
        )),
        TxtLookup::ServFail { detail } => findings.push(Finding::notice(
            CheckKind::MtaSts,
            "mta_sts_txt_servfail",
            format!("DNS lookup for _mta-sts.{domain} failed ({detail})."),
            "Fix nameserver errors for _mta-sts if you intend to publish MTA-STS.",
        )),
        TxtLookup::Records(_) if records.is_empty() && !only_other_protocols(txt) => {
            findings.push(Finding::info(
                CheckKind::MtaSts,
                "mta_sts_bad_txt",
                format!("_mta-sts.{domain} has TXT records but none start with v=STSv1."),
                "Set the TXT value to `v=STSv1; id=<changing-id>`.",
            ));
        }
        // No name, no TXT at all, or TXT for other protocols only (the
        // wildcard case): nothing MTA-STS is published.
        TxtLookup::NxDomain | TxtLookup::NoRecords | TxtLookup::Records(_)
            if records.is_empty() =>
        {
            findings.push(Finding::info(
                CheckKind::MtaSts,
                "mta_sts_missing",
                format!("No MTA-STS TXT record at _mta-sts.{domain}. Receivers will not require STARTTLS to a pinned MX set."),
                format!("Publish `_mta-sts.{domain}` TXT `v=STSv1; id=<unique>` and a policy at {policy_url}."),
            ));
        }
        // `records` is non-empty here, so this is a `Records` answer.
        _ => {
            if records.len() > 1 {
                findings.push(Finding::info(
                    CheckKind::MtaSts,
                    "mta_sts_multiple_records",
                    format!(
                        "_mta-sts.{domain} has {} v=STSv1 TXT records. RFC 8461 §3.1 says senders then treat MTA-STS as not published.",
                        records.len()
                    ),
                    "Keep exactly one v=STSv1 record at _mta-sts.",
                ));
            }
            if let Some(p) = &parsed_txt {
                if !p
                    .version
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case("STSv1"))
                {
                    findings.push(Finding::info(
                        CheckKind::MtaSts,
                        "mta_sts_bad_version",
                        "MTA-STS TXT version is not STSv1.",
                        "Use `v=STSv1`.",
                    ));
                }
                if p.id.as_deref().is_none_or(str::is_empty) {
                    findings.push(Finding::info(
                        CheckKind::MtaSts,
                        "mta_sts_missing_id",
                        "MTA-STS TXT record has no id= tag, which RFC 8461 §3.1 requires. Senders cannot tell when the policy changed.",
                        "Add `id=<value>` and change it whenever the policy file changes, e.g. `id=20260101T120000`.",
                    ));
                }
            }
        }
    }

    // The policy only matters with a record that points receivers at it.
    if txt_record.is_some() {
        if let Some(body) = policy_body {
            evaluate_policy(domain, body, fetch, &mut findings, &mut result);
        } else {
            fetch_findings(domain, &policy_url, fetch, &mut findings);
        }
    }

    result.findings = findings;
    result
}

fn evaluate_policy(
    domain: &str,
    body: &str,
    fetch: &PolicyFetch,
    findings: &mut Vec<Finding>,
    result: &mut MtaStsCheckResult,
) {
    let policy = parse_mta_sts_policy(body);
    if let PolicyFetch::Ok {
        content_type: Some(ct),
    } = fetch
    {
        if !ct.trim().to_ascii_lowercase().starts_with("text/plain") {
            findings.push(Finding::notice(
                CheckKind::MtaSts,
                "mta_sts_policy_not_text",
                format!(
                    "The MTA-STS policy file is served as `{}`, not text/plain. RFC 8461 §3.3 says senders should check the media type.",
                    ct.trim()
                ),
                "Serve /.well-known/mta-sts.txt with `Content-Type: text/plain`.",
            ));
        }
    }
    if !policy
        .version
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("STSv1"))
    {
        findings.push(Finding::info(
            CheckKind::MtaSts,
            "mta_sts_bad_policy_version",
            match &policy.version {
                Some(v) => format!("MTA-STS policy file has `version: {v}`, not STSv1."),
                None => "MTA-STS policy file has no version field.".to_owned(),
            },
            "Start the policy file with `version: STSv1`.",
        ));
    }
    match policy.mode.as_deref() {
        Some("enforce") => findings.push(Finding::ok(
            CheckKind::MtaSts,
            "mta_sts_enforce",
            format!("MTA-STS policy for {domain} is mode=enforce."),
            "Keep max_age high and bump id= whenever the policy file changes.",
        )),
        Some("testing") => findings.push(Finding::info(
            CheckKind::MtaSts,
            "mta_sts_testing",
            "MTA-STS policy is in testing mode; receivers report failures but still deliver.",
            "Move mode to enforce once TLS-RPT shows clean MX connectivity.",
        )),
        Some("none") => findings.push(Finding::info(
            CheckKind::MtaSts,
            "mta_sts_none",
            "MTA-STS policy mode=none disables the policy.",
            "Set mode: enforce (or testing during rollout).",
        )),
        Some(other) => findings.push(Finding::info(
            CheckKind::MtaSts,
            "mta_sts_bad_mode",
            format!("MTA-STS mode={other} is not none, testing, or enforce."),
            "Use mode: enforce.",
        )),
        None => findings.push(Finding::info(
            CheckKind::MtaSts,
            "mta_sts_missing_mode",
            "MTA-STS policy file has no mode field.",
            "Add `mode: enforce`.",
        )),
    }
    if policy.mx.is_empty() {
        // The only state in this file that breaks mail: an enforcing policy
        // with nothing to match means every MX fails the pin.
        let enforcing = policy.mode.as_deref() == Some("enforce");
        let severity = if enforcing {
            Severity::Warning
        } else {
            Severity::Info
        };
        findings.push(Finding::new(
            CheckKind::MtaSts,
            severity,
            "mta_sts_no_mx",
            if enforcing {
                "MTA-STS policy is mode=enforce but lists no mx: names, so senders honouring it can match no host and will not deliver."
            } else {
                "MTA-STS policy lists no mx: names, so it cannot pin mail hosts."
            },
            "Add `mx: mail.example.com` lines matching your MX hostnames (wildcards allowed as *.example.com).",
        ));
    }
    if mta_sts_max_age_short(policy.max_age) {
        findings.push(Finding::info(
            CheckKind::MtaSts,
            "mta_sts_short_max_age",
            format!("MTA-STS max_age is under {MTA_STS_MIN_MAX_AGE} seconds, so senders will forget the policy quickly."),
            format!("Use max_age of at least {MTA_STS_GOOD_MAX_AGE} (one week) once the policy is stable."),
        ));
    }
    result.mode_status = policy.mode.as_deref().map(mta_sts_mode_status);
    result.policy = Some(policy);
}

/// The finding for a `v=STSv1` record whose policy file could not be read.
fn fetch_findings(
    domain: &str,
    policy_url: &str,
    fetch: &PolicyFetch,
    findings: &mut Vec<Finding>,
) {
    let serve = "Serve https://mta-sts.<domain>/.well-known/mta-sts.txt with a certificate valid for mta-sts.<domain>, without a redirect.";
    let why = match fetch {
        PolicyFetch::NotAttempted | PolicyFetch::Ok { .. } => return,
        PolicyFetch::Redirect { location } => {
            findings.push(Finding::info(
                CheckKind::MtaSts,
                "mta_sts_policy_redirect",
                format!(
                    "The MTA-STS policy URL {policy_url} redirects{}, which RFC 8461 forbids. Receivers cannot apply the policy.",
                    location.as_deref().map(|l| format!(" to {l}")).unwrap_or_default()
                ),
                "Serve the policy file at https://mta-sts.<domain>/.well-known/mta-sts.txt itself, without a redirect.",
            ));
            return;
        }
        PolicyFetch::Timeout => {
            findings.push(Finding::notice(
                CheckKind::MtaSts,
                "mta_sts_policy_fetch_failed",
                format!(
                    "MTA-STS TXT exists but fetching the policy file at {policy_url} timed out, so it could not be checked."
                ),
                "Retry the check. If the host keeps timing out, receivers cannot apply the policy either.",
            ));
            return;
        }
        PolicyFetch::PrivateAddress { address } => {
            findings.push(Finding::info(
                CheckKind::MtaSts,
                "mta_sts_policy_private_address",
                format!(
                    "MTA-STS host resolves to a private address: mta-sts.{domain} points at {address}, so Marcan did not fetch {policy_url}. Receivers on the internet cannot reach it either, so they cannot apply the policy."
                ),
                "Point mta-sts.<domain> at a public address that serves the policy file over HTTPS.",
            ));
            return;
        }
        PolicyFetch::NameNotFound => format!("mta-sts.{domain} does not resolve"),
        PolicyFetch::ConnectFailed => format!("no connection to mta-sts.{domain} on port 443"),
        PolicyFetch::TlsFailed { detail } => format!(
            "the TLS handshake failed ({}); the certificate must be valid for mta-sts.{domain}",
            detail.trim()
        ),
        PolicyFetch::HttpStatus { code } => format!("the server answered HTTP {code}"),
        PolicyFetch::TooLarge => "the response is over 64 KB or not text".to_owned(),
        PolicyFetch::Failed { detail } => detail.trim().to_owned(),
    };
    findings.push(Finding::info(
        CheckKind::MtaSts,
        "mta_sts_policy_fetch_failed",
        format!(
            "MTA-STS TXT exists but the policy file at {policy_url} could not be fetched: {why}. Receivers cannot apply the policy."
        ),
        serve,
    ));
}

// --- TLS-RPT ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParsedTlsRpt {
    pub version: Option<String>,
    pub rua: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TlsRptCheckResult {
    pub domain: String,
    pub name: String,
    pub lookup: LookupStatus,
    pub records: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parsed: Option<ParsedTlsRpt>,
    pub findings: Vec<Finding>,
}

pub fn parse_tls_rpt_record(raw: &str) -> ParsedTlsRpt {
    let mut parsed = ParsedTlsRpt {
        version: None,
        rua: Vec::new(),
    };
    for (k, v, ok) in parse_tag_list(raw) {
        if !ok {
            continue;
        }
        match k.as_str() {
            "v" => parsed.version = Some(v),
            "rua" => parsed.rua.extend(
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            ),
            _ => {}
        }
    }
    parsed
}

pub fn evaluate_tls_rpt(domain: &str, lookup: &TxtLookup) -> TlsRptCheckResult {
    let name = format!("_smtp._tls.{domain}");
    let records = lookup.records_starting_with("v=tlsrptv1");
    let mut findings = Vec::new();
    let parsed = records.first().map(|r| parse_tls_rpt_record(r));
    match lookup {
        TxtLookup::Timeout | TxtLookup::ServFail { .. } => findings.push(Finding::notice(
            CheckKind::TlsRpt,
            "tls_rpt_lookup_failed",
            format!("Could not look up {name}."),
            "Retry the check; TLS-RPT is optional.",
        )),
        TxtLookup::Records(_) if records.is_empty() && !only_other_protocols(lookup) => findings
            .push(Finding::info(
                CheckKind::TlsRpt,
                "tls_rpt_bad_record",
                format!("{name} has TXT data that is not v=TLSRPTv1."),
                "Use `v=TLSRPTv1; rua=mailto:...`.",
            )),
        _ if records.is_empty() => findings.push(Finding::info(
            CheckKind::TlsRpt,
            "tls_rpt_missing",
            format!("No TLS-RPT record at {name}. You will not receive SMTP TLS failure reports."),
            format!("Publish TXT at {name}: `v=TLSRPTv1; rua=mailto:tlsrpt@{domain}`."),
        )),
        _ => {
            if records.len() > 1 {
                findings.push(Finding::info(
                    CheckKind::TlsRpt,
                    "tls_rpt_multiple_records",
                    format!(
                        "{name} has {} v=TLSRPTv1 TXT records. RFC 8460 §3 says senders then treat TLS-RPT as not published.",
                        records.len()
                    ),
                    "Keep exactly one v=TLSRPTv1 record at _smtp._tls.",
                ));
            }
            if let Some(p) = &parsed {
                if !p
                    .version
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case("TLSRPTv1"))
                {
                    findings.push(Finding::info(
                        CheckKind::TlsRpt,
                        "tls_rpt_bad_version",
                        "TLS-RPT version is not TLSRPTv1.",
                        "Set v=TLSRPTv1.",
                    ));
                }
                if p.rua.is_empty() {
                    findings.push(Finding::info(
                        CheckKind::TlsRpt,
                        "tls_rpt_no_rua",
                        "TLS-RPT record has no rua= reporting URI.",
                        format!("Add `rua=mailto:tlsrpt@{domain}`."),
                    ));
                } else {
                    findings.push(Finding::ok(
                        CheckKind::TlsRpt,
                        "tls_rpt_ok",
                        format!(
                            "TLS-RPT is published with {} reporting URI(s).",
                            p.rua.len()
                        ),
                        "Keep rua= reachable so TLS failure reports arrive.",
                    ));
                }
            }
        }
    }
    TlsRptCheckResult {
        domain: domain.to_string(),
        name,
        lookup: lookup.status(),
        records,
        parsed,
        findings,
    }
}

// --- BIMI ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParsedBimi {
    pub version: Option<String>,
    /// `l=`; `None` when absent or empty.
    pub logo_url: Option<String>,
    /// `a=`; `None` when absent or empty (an empty `a=` means no certificate).
    pub authority_url: Option<String>,
    /// `l=` is present and empty: the domain declares that it does not
    /// participate in BIMI, so receivers show no logo.
    pub declined: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BimiCheckResult {
    pub domain: String,
    pub name: String,
    pub lookup: LookupStatus,
    pub records: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parsed: Option<ParsedBimi>,
    pub findings: Vec<Finding>,
}

pub fn parse_bimi_record(raw: &str) -> ParsedBimi {
    let mut parsed = ParsedBimi {
        version: None,
        logo_url: None,
        authority_url: None,
        declined: false,
    };
    for (k, v, ok) in parse_tag_list(raw) {
        if !ok {
            continue;
        }
        match k.as_str() {
            "v" => parsed.version = Some(v),
            "l" if v.is_empty() => parsed.declined = true,
            "l" => parsed.logo_url = Some(v),
            "a" if v.is_empty() => {}
            "a" => parsed.authority_url = Some(v),
            _ => {}
        }
    }
    parsed
}

pub fn evaluate_bimi(domain: &str, lookup: &TxtLookup) -> BimiCheckResult {
    let name = format!("default._bimi.{domain}");
    let records = lookup.records_starting_with("v=bimi1");
    let mut findings = Vec::new();
    let parsed = records.first().map(|r| parse_bimi_record(r));
    match lookup {
        // BIMI is inbox logo branding only. Absence, malformation, and our own
        // incomplete lookups never affect authentication or delivery, so they
        // are Notice (visible, zero deduction) rather than Info or Ok.
        TxtLookup::Timeout | TxtLookup::ServFail { .. } => findings.push(Finding::notice(
            CheckKind::Bimi,
            "bimi_lookup_failed",
            format!("Could not look up {name}."),
            "Retry the check. BIMI is optional.",
        )),
        TxtLookup::Records(_) if records.is_empty() && !only_other_protocols(lookup) => {
            findings.push(Finding::notice(
                CheckKind::Bimi,
                "bimi_bad_record",
                format!("{name} exists but is not a v=BIMI1 record."),
                "Use `v=BIMI1; l=https://...; a=https://...`.",
            ));
        }
        _ if records.is_empty() => findings.push(Finding::notice(
            CheckKind::Bimi,
            "bimi_missing",
            format!("No BIMI record at {name}. Inbox logo branding is not advertised."),
            format!(
                "BIMI requires DMARC p=quarantine/reject at pct=100 plus a TXT at {name}: `v=BIMI1; l=https://...svg`."
            ),
        )),
        _ => {
            if let Some(p) = &parsed {
                if p.declined {
                    findings.push(Finding::notice(
                        CheckKind::Bimi,
                        "bimi_declined",
                        format!("{name} has an empty l= tag: the domain declares that it does not use BIMI, so receivers show no logo."),
                        "Nothing to do unless you want a logo; then set l= to an HTTPS SVG logo URL.",
                    ));
                } else {
                    match p.logo_url.as_deref() {
                        Some(url) if url.starts_with("https://") => {}
                        Some(_) => findings.push(Finding::notice(
                            CheckKind::Bimi,
                            "bimi_logo_not_https",
                            "BIMI l= logo URL is not https://. Receivers will ignore it.",
                            "Host an SVG Tiny PS logo on HTTPS and set l= to that URL.",
                        )),
                        None => findings.push(Finding::notice(
                            CheckKind::Bimi,
                            "bimi_no_logo",
                            "BIMI record has no l= logo URL.",
                            "Set l= to an HTTPS SVG logo URL.",
                        )),
                    }
                    if p.authority_url.is_none() {
                        findings.push(Finding::notice(
                            CheckKind::Bimi,
                            "bimi_no_vmc",
                            "BIMI record has no a= VMC URL. Gmail and others require a Verified Mark Certificate to show the logo.",
                            "Add `a=https://...` pointing at your PEM-encoded VMC, or skip BIMI until you have one.",
                        ));
                    }
                }
                if findings.is_empty() {
                    findings.push(Finding::ok(
                        CheckKind::Bimi,
                        "bimi_ok",
                        "BIMI record is present with an HTTPS logo URL.",
                        "Confirm the SVG meets BIMI profile rules and that DMARC is at enforcement.",
                    ));
                }
            }
        }
    }
    BimiCheckResult {
        domain: domain.to_string(),
        name,
        lookup: lookup.status(),
        records,
        parsed,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup::TxtAnswer;

    fn rec(s: &str) -> TxtLookup {
        TxtLookup::Records(TxtAnswer::from_records(vec![s.to_string()]))
    }

    fn recs(s: &[&str]) -> TxtLookup {
        TxtLookup::Records(TxtAnswer::from_records(
            s.iter().map(ToString::to_string).collect(),
        ))
    }

    fn ok_text() -> PolicyFetch {
        PolicyFetch::Ok {
            content_type: Some("text/plain; charset=utf-8".into()),
        }
    }

    fn codes(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.code.as_str()).collect()
    }

    const ENFORCE: &str = "version: STSv1\nmode: enforce\nmax_age: 86400\nmx: mail.example.com\n";

    #[test]
    fn mta_sts_enforce_policy() {
        let r = evaluate_mta_sts(
            "example.com",
            &rec("v=STSv1; id=20240101"),
            Some(ENFORCE),
            &ok_text(),
        );
        assert_eq!(r.policy.as_ref().unwrap().mode.as_deref(), Some("enforce"));
        assert_eq!(codes(&r.findings), vec!["mta_sts_enforce"]);
        assert_eq!(r.mode_status, Some(Status::Pass));
        assert_eq!(r.fetch_status, Status::Neutral);
    }

    /// dmarc-dns reports the measured `max_age`; dmarc-assess places it below
    /// or above the floor (ENFORCE sits exactly on it and has no finding).
    #[test]
    fn mta_sts_short_max_age_is_info() {
        let r = evaluate_mta_sts(
            "example.com",
            &rec("v=STSv1; id=20240101"),
            Some("version: STSv1\nmode: enforce\nmax_age: 3600\nmx: mail.example.com\n"),
            &ok_text(),
        );
        let short = r
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_short_max_age")
            .expect("short max_age finding");
        assert_eq!(short.severity, Severity::Info);
        assert!(short.message.contains("86400"));
    }

    /// Receivers find the policy through the TXT record: a file served without
    /// one (a wildcard host, say) is inactive and must not be graded.
    #[test]
    fn mta_sts_policy_without_txt_is_ignored() {
        let r = evaluate_mta_sts(
            "example.com",
            &TxtLookup::NoRecords,
            Some("version: STSv1\nmode: testing\nmax_age: 60\n"),
            &ok_text(),
        );
        assert!(r.policy.is_none());
        assert_eq!(r.mode_status, None);
        assert_eq!(codes(&r.findings), vec!["mta_sts_missing"]);
    }

    #[test]
    fn mta_sts_fetch_failures_say_why_and_warn() {
        let txt = rec("v=STSv1; id=20240101");
        let redirect = PolicyFetch::Redirect {
            location: Some("https://www.example.com/".to_owned()),
        };
        let r = evaluate_mta_sts("example.com", &txt, None, &redirect);
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_policy_redirect")
            .unwrap();
        assert!(f
            .message
            .contains("redirects to https://www.example.com/, which RFC 8461 forbids"));
        assert_eq!(r.fetch_status, Status::Warn);

        for (fetch, expect) in [
            (PolicyFetch::HttpStatus { code: 404 }, "HTTP 404"),
            (
                PolicyFetch::NameNotFound,
                "mta-sts.example.com does not resolve",
            ),
            (
                PolicyFetch::ConnectFailed,
                "no connection to mta-sts.example.com",
            ),
            (
                PolicyFetch::TlsFailed {
                    detail: "invalid peer certificate: NotValidForName".into(),
                },
                "TLS handshake failed (invalid peer certificate: NotValidForName)",
            ),
            (PolicyFetch::TooLarge, "over 64 KB"),
            (
                PolicyFetch::Failed {
                    detail: "connection reset".into(),
                },
                "connection reset",
            ),
        ] {
            let r = evaluate_mta_sts("example.com", &txt, None, &fetch);
            let f = r
                .findings
                .iter()
                .find(|f| f.code == "mta_sts_policy_fetch_failed")
                .unwrap();
            assert!(f.message.contains(expect), "{}", f.message);
            assert_eq!(f.severity, Severity::Info);
            assert_eq!(r.fetch_status, Status::Warn, "{expect}");
        }

        // A private address is refused before connecting: the host's fault,
        // like any unreachable policy host.
        let private = PolicyFetch::PrivateAddress {
            address: "10.0.0.5".into(),
        };
        let r = evaluate_mta_sts("example.com", &txt, None, &private);
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_policy_private_address")
            .unwrap();
        assert!(f.message.starts_with(
            "MTA-STS host resolves to a private address: mta-sts.example.com points at 10.0.0.5"
        ));
        assert_eq!(f.severity, Severity::Info);
        assert_eq!(r.fetch_status, Status::Warn);

        // Our own timeout measured nothing: a notice, no deduction, no warning.
        let r = evaluate_mta_sts("example.com", &txt, None, &PolicyFetch::Timeout);
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_policy_fetch_failed")
            .unwrap();
        assert!(f.message.contains("timed out"));
        assert_eq!(f.severity, Severity::Notice);
        assert_eq!(r.fetch_status, Status::Neutral);
    }

    #[test]
    fn mta_sts_txt_syntax_findings() {
        let r = evaluate_mta_sts("example.com", &rec("v=STSv1"), Some(ENFORCE), &ok_text());
        assert!(codes(&r.findings).contains(&"mta_sts_missing_id"));

        let r = evaluate_mta_sts(
            "example.com",
            &recs(&["v=STSv1; id=1", "v=STSv1; id=2"]),
            Some(ENFORCE),
            &ok_text(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_multiple_records")
            .unwrap();
        assert!(f.message.contains("2 v=STSv1"));
    }

    #[test]
    fn mta_sts_policy_file_findings() {
        let txt = rec("v=STSv1; id=1");
        let r = evaluate_mta_sts(
            "example.com",
            &txt,
            Some("mode: enforce\nmax_age: 604800\nmx: mail.example.com\n"),
            &ok_text(),
        );
        assert_eq!(
            codes(&r.findings),
            vec!["mta_sts_bad_policy_version", "mta_sts_enforce"]
        );

        let r = evaluate_mta_sts(
            "example.com",
            &txt,
            Some("version: STSv2\nmode: enforce\nmax_age: 604800\nmx: a.example.com\n"),
            &ok_text(),
        );
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "mta_sts_bad_policy_version" && f.message.contains("STSv2")));

        let html = PolicyFetch::Ok {
            content_type: Some("text/html; charset=utf-8".into()),
        };
        let r = evaluate_mta_sts("example.com", &txt, Some(ENFORCE), &html);
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_policy_not_text")
            .unwrap();
        assert_eq!(f.severity, Severity::Notice);
        assert!(f.message.contains("text/html"));

        // No Content-Type at all is not flagged.
        let r = evaluate_mta_sts(
            "example.com",
            &txt,
            Some(ENFORCE),
            &PolicyFetch::Ok { content_type: None },
        );
        assert_eq!(codes(&r.findings), vec!["mta_sts_enforce"]);
    }

    /// A wildcard SPF answering at `_mta-sts` / `_smtp._tls` / `default._bimi`
    /// means nothing is published for the check, not that it is malformed.
    #[test]
    fn other_protocols_at_the_name_count_as_missing() {
        let spf = rec("v=spf1 redirect=_spf.example.com");
        let r = evaluate_mta_sts("example.com", &spf, None, &PolicyFetch::NotAttempted);
        assert_eq!(codes(&r.findings), vec!["mta_sts_missing"]);
        assert_eq!(
            codes(&evaluate_tls_rpt("example.com", &spf).findings),
            vec!["tls_rpt_missing"]
        );
        assert_eq!(
            codes(&evaluate_bimi("example.com", &spf).findings),
            vec!["bimi_missing"]
        );

        let token = rec("mmdxtly5wtl64qpt6hn4jgpfsr1hl0np");
        let r = evaluate_mta_sts("example.com", &token, None, &PolicyFetch::NotAttempted);
        assert_eq!(codes(&r.findings), vec!["mta_sts_bad_txt"]);
        assert_eq!(
            codes(&evaluate_tls_rpt("example.com", &token).findings),
            vec!["tls_rpt_bad_record"]
        );
        assert_eq!(
            codes(&evaluate_bimi("example.com", &token).findings),
            vec!["bimi_bad_record"]
        );
    }

    #[test]
    fn tls_rpt_and_bimi_parsers() {
        let t = evaluate_tls_rpt(
            "example.com",
            &rec("v=TLSRPTv1; rua=mailto:tls@example.com"),
        );
        assert!(t.findings.iter().any(|f| f.code == "tls_rpt_ok"));
        let b = evaluate_bimi(
            "example.com",
            &rec("v=BIMI1; l=https://example.com/logo.svg; a=https://example.com/vmc.pem"),
        );
        assert!(b.findings.iter().any(|f| f.code == "bimi_ok"));
        let missing = evaluate_bimi("example.com", &TxtLookup::NxDomain);
        assert!(missing.findings.iter().any(|f| f.code == "bimi_missing"));
    }

    #[test]
    fn tls_rpt_multiple_records_flagged() {
        let t = evaluate_tls_rpt(
            "example.com",
            &recs(&[
                "v=TLSRPTv1; rua=mailto:a@example.com",
                "v=TLSRPTv1; rua=mailto:b@example.com",
            ]),
        );
        assert!(t
            .findings
            .iter()
            .any(|f| f.code == "tls_rpt_multiple_records"));
    }

    /// `v=BIMI1; l=; a=;` (Proton, iCloud) declares "no BIMI": a declination,
    /// not a malformed logo URL; an empty `a=` is no certificate.
    #[test]
    fn bimi_declination_and_empty_tags() {
        let b = evaluate_bimi("example.com", &rec("v=BIMI1; l=; a=;"));
        let p = b.parsed.as_ref().unwrap();
        assert!(p.declined);
        assert_eq!(p.logo_url, None);
        assert_eq!(p.authority_url, None);
        assert_eq!(codes(&b.findings), vec!["bimi_declined"]);

        let b = evaluate_bimi(
            "example.com",
            &rec("v=BIMI1; l=https://example.com/l.svg; a=;"),
        );
        assert_eq!(b.parsed.as_ref().unwrap().authority_url, None);
        assert_eq!(codes(&b.findings), vec!["bimi_no_vmc"]);
    }

    /// BIMI is logo branding: every BIMI finding is Notice or Ok (zero score
    /// impact).
    #[test]
    fn bimi_findings_are_notice_or_ok() {
        let broken = [
            rec("this is not a bimi record"),
            rec("v=BIMI1;"),
            rec("v=BIMI1; l=http://example.com/logo.svg"),
            rec("v=BIMI1; l=;"),
        ];
        for lookup in &broken {
            let r = evaluate_bimi("example.com", lookup);
            assert!(!r.findings.is_empty());
            for f in &r.findings {
                assert!(
                    f.severity >= Severity::Notice,
                    "{} was {:?}",
                    f.code,
                    f.severity
                );
            }
        }
    }

    #[test]
    fn bimi_absence_and_lookup_failure_are_notice() {
        let missing = evaluate_bimi("example.com", &TxtLookup::NxDomain);
        let f = missing
            .findings
            .iter()
            .find(|f| f.code == "bimi_missing")
            .expect("bimi_missing");
        assert_eq!(f.severity, Severity::Notice);

        let failed = evaluate_bimi("example.com", &TxtLookup::Timeout);
        let f = failed
            .findings
            .iter()
            .find(|f| f.code == "bimi_lookup_failed")
            .expect("bimi_lookup_failed");
        assert_eq!(f.severity, Severity::Notice);
    }

    #[test]
    fn optional_lookup_failures_are_notice() {
        let mta = evaluate_mta_sts(
            "example.com",
            &TxtLookup::Timeout,
            None,
            &PolicyFetch::NotAttempted,
        );
        assert!(mta
            .findings
            .iter()
            .any(|f| { f.code == "mta_sts_txt_timeout" && f.severity == Severity::Notice }));

        let tls = evaluate_tls_rpt("example.com", &TxtLookup::Timeout);
        assert!(tls
            .findings
            .iter()
            .any(|f| { f.code == "tls_rpt_lookup_failed" && f.severity == Severity::Notice }));
    }

    #[test]
    fn tls_rpt_findings_never_exceed_info() {
        for lookup in [rec("v=TLSRPTv1"), rec("junk"), rec("v=TLSRPTv2; rua=x")] {
            for f in &evaluate_tls_rpt("example.com", &lookup).findings {
                assert!(f.severity >= Severity::Info, "{} escalated", f.code);
            }
        }
    }

    /// A policy nobody enforces is a nudge; an enforcing policy that pins no MX
    /// stops delivery, so only that one earns a warning.
    #[test]
    fn mta_sts_no_mx_escalates_only_under_enforce() {
        let txt = rec("v=STSv1; id=1");
        let testing = evaluate_mta_sts(
            "example.com",
            &txt,
            Some("version: STSv1\nmode: testing\nmax_age: 604800\n"),
            &ok_text(),
        );
        let f = testing
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_no_mx")
            .expect("no_mx finding");
        assert_eq!(f.severity, Severity::Info);

        let enforcing = evaluate_mta_sts(
            "example.com",
            &txt,
            Some("version: STSv1\nmode: enforce\nmax_age: 604800\n"),
            &ok_text(),
        );
        let f = enforcing
            .findings
            .iter()
            .find(|f| f.code == "mta_sts_no_mx")
            .expect("no_mx finding");
        assert_eq!(f.severity, Severity::Warning);
    }
}
