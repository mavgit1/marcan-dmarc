//! DMARC record parsing and evaluation (RFC 7489, and the tags RFC 9989,
//! `DMARCbis`, adds or drops: `np=` and `t=` are parsed, `rf=` and `ri=` are
//! obsolete, `pct=` is still honoured by RFC 7489 receivers).

use crate::lookup::{parse_tag_list, LookupStatus, TxtLookup};
use dmarc_assess::{
    authorization_summary, authorization_verdict, dmarc_record_status, enforcement,
    no_issue_above_info, pct_status, testing_status, AuthVerdict, CheckKind, Enforcement, Finding,
    Status,
};
use dmarc_domain::report_authorization_needed;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const VALID_POLICIES: &[&str] = &["none", "quarantine", "reject"];
const VALID_ALIGNMENT: &[&str] = &["r", "s"];
const VALID_RF: &[&str] = &["afrf", "iodef"];
const VALID_TESTING: &[&str] = &["y", "n"];
const VALID_PSD: &[&str] = &["y", "n", "u"];

/// A `mailto:` / `https:` aggregate or forensic reporting URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReportUri {
    pub raw: String,
    pub scheme: String,
    pub address: Option<String>,
    pub host: Option<String>,
    pub size_limit: Option<String>,
    pub valid: bool,
}

/// Parsed DMARC tags. Missing tags are `None` / empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParsedDmarc {
    pub v: Option<String>,
    pub p: Option<String>,
    pub sp: Option<String>,
    /// `np=` (RFC 9989): the policy for subdomains that do not exist.
    pub np: Option<String>,
    pub pct: Option<u32>,
    /// Raw pct value when the tag was present but not an integer 0–100.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pct_raw: Option<String>,
    pub rua: Vec<ReportUri>,
    pub ruf: Vec<ReportUri>,
    pub adkim: Option<String>,
    pub aspf: Option<String>,
    pub fo: Option<String>,
    pub ri: Option<u32>,
    pub rf: Option<String>,
    /// `t=` (RFC 9989), lowercase: `y` asks receivers not to apply the policy
    /// (testing), `n` (the default) to apply it.
    pub t: Option<String>,
    /// Tags RFC 9989 dropped that the record still publishes (`rf`, `ri`),
    /// in record order; receivers ignore them. Defaulted: results stored by
    /// an older checker lack it and are shown until their re-check.
    #[serde(default)]
    pub obsolete_tags: Vec<String>,
    /// `psd=` (RFC 9989), lowercase: whether the domain is a public suffix
    /// domain (`y`, `n`, `u` the default). Read only to judge its value,
    /// neither stored nor shown.
    #[serde(skip)]
    #[ts(skip)]
    pub psd: Option<String>,
    pub unknown_tags: Vec<String>,
    /// True when the first tag was not `v`.
    pub v_not_first: bool,
    /// Tag parts that lacked `=`.
    pub malformed_parts: Vec<String>,
}

impl ParsedDmarc {
    /// `t=y`: the policy is published for testing, and RFC 9989 receivers do
    /// not apply it. Any other value (or none) is `n`, the default.
    pub fn testing(&self) -> bool {
        self.t.as_deref() == Some("y")
    }
}

/// External reporting authorization result (RFC 7489 §7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExternalAuthorization {
    pub uri: String,
    pub host: String,
    pub authorized: bool,
    pub auth_record_name: String,
    pub auth_record: Option<String>,
    pub lookup: LookupStatus,
    pub verdict: AuthVerdict,
}

/// Full DMARC check result for one domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DmarcCheckResult {
    pub domain: String,
    pub name: String,
    pub lookup: LookupStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lookup_detail: Option<String>,
    pub records: Vec<String>,
    pub record_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parsed: Option<ParsedDmarc>,
    /// Verdict on the published `p=`; `unknown` without a record.
    pub enforcement: Enforcement,
    /// Verdict on `sp=` when published (subdomains otherwise follow `p=`).
    pub subdomain_enforcement: Option<Enforcement>,
    /// Verdict on `np=` when published (subdomains that do not exist
    /// otherwise follow `sp=`, then `p=`).
    pub nonexistent_enforcement: Option<Enforcement>,
    /// `pct=` was published below 100: the same fact as `pct_status`, kept
    /// because both are on the wire.
    pub pct_below_100: bool,
    /// Verdict on `pct=`: `warn` below 100 (the policy skips some failing
    /// mail), `neutral` otherwise.
    pub pct_status: Status,
    /// Verdict on `t=`: `warn` for `t=y` (RFC 9989 receivers do not apply
    /// the policy), `neutral` otherwise. Defaulted, as `obsolete_tags`.
    #[serde(default)]
    pub testing_status: Status,
    /// Verdict on the record's presence: `pass` when published, `fail` when
    /// absent (DMARC is required), `neutral` when the lookup failed.
    pub record_status: Status,
    pub external_authorization: Vec<ExternalAuthorization>,
    /// Every external destination as one verdict; `None` without external ones.
    pub rua_authorization: Option<AuthVerdict>,
    pub findings: Vec<Finding>,
}

/// Parse a single DMARC TXT value into tags. Does not produce findings.
pub fn parse_dmarc_record(raw: &str) -> ParsedDmarc {
    let tags = parse_tag_list(raw);
    let mut parsed = ParsedDmarc {
        v: None,
        p: None,
        sp: None,
        np: None,
        pct: None,
        pct_raw: None,
        rua: Vec::new(),
        ruf: Vec::new(),
        adkim: None,
        aspf: None,
        fo: None,
        ri: None,
        rf: None,
        t: None,
        obsolete_tags: Vec::new(),
        psd: None,
        unknown_tags: Vec::new(),
        v_not_first: false,
        malformed_parts: Vec::new(),
    };

    if let Some((k, _, _)) = tags.first() {
        if k != "v" {
            parsed.v_not_first = true;
        }
    }

    for (key, value, has_eq) in tags {
        if !has_eq {
            parsed.malformed_parts.push(key);
            continue;
        }
        match key.as_str() {
            "v" => parsed.v = Some(value),
            "p" => parsed.p = Some(value.to_ascii_lowercase()),
            "sp" => parsed.sp = Some(value.to_ascii_lowercase()),
            "np" => parsed.np = Some(value.to_ascii_lowercase()),
            "t" => parsed.t = Some(value.to_ascii_lowercase()),
            "psd" => parsed.psd = Some(value.to_ascii_lowercase()),
            "pct" => match value.parse::<u32>() {
                Ok(n) if n <= 100 => parsed.pct = Some(n),
                _ => parsed.pct_raw = Some(value),
            },
            "rua" => parsed.rua = parse_uri_list(&value),
            "ruf" => parsed.ruf = parse_uri_list(&value),
            "adkim" => parsed.adkim = Some(value.to_ascii_lowercase()),
            "aspf" => parsed.aspf = Some(value.to_ascii_lowercase()),
            "fo" => parsed.fo = Some(value.to_ascii_lowercase()),
            "ri" => {
                parsed.obsolete_tags.push(key.clone());
                parsed.ri = value.parse().ok();
            }
            "rf" => {
                parsed.obsolete_tags.push(key.clone());
                parsed.rf = Some(value.to_ascii_lowercase());
            }
            other => parsed.unknown_tags.push(other.to_string()),
        }
    }
    parsed
}

fn parse_uri_list(raw: &str) -> Vec<ReportUri> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_report_uri)
        .collect()
}

fn parse_report_uri(raw: &str) -> ReportUri {
    let (uri_part, size_limit) = match raw.rsplit_once('!') {
        Some((uri, limit)) if !limit.is_empty() && !limit.contains('@') => {
            (uri, Some(limit.to_string()))
        }
        _ => (raw, None),
    };

    let lower = uri_part.to_ascii_lowercase();
    if let Some(rest) = uri_part.get(7..).filter(|_| lower.starts_with("mailto:")) {
        let addr = rest.to_string();
        let host = addr.split('@').nth(1).map(str::to_ascii_lowercase);
        let valid = addr.contains('@')
            && host
                .as_ref()
                .is_some_and(|h| !h.is_empty() && h.contains('.'));
        return ReportUri {
            raw: raw.to_string(),
            scheme: "mailto".into(),
            address: Some(addr),
            host,
            size_limit,
            valid,
        };
    }
    if lower.starts_with("https://") {
        let host = lower
            .trim_start_matches("https://")
            .split('/')
            .next()
            .and_then(|h| h.split('@').next())
            .filter(|h| !h.is_empty())
            .map(str::to_string);
        let valid = host.as_ref().is_some_and(|h| h.contains('.'));
        return ReportUri {
            raw: raw.to_string(),
            scheme: "https".into(),
            address: None,
            host,
            size_limit,
            valid,
        };
    }

    ReportUri {
        raw: raw.to_string(),
        scheme: String::new(),
        address: None,
        host: None,
        size_limit,
        valid: false,
    }
}

/// DNS name of the RFC 7489 §7.1 authorization record.
pub fn report_auth_name(checked_domain: &str, rua_host: &str) -> String {
    format!("{checked_domain}._report._dmarc.{rua_host}")
}

/// The `v=DMARC1` record in an RFC 7489 §7.1 authorization lookup, if any.
pub fn authorization_record(lookup: &TxtLookup) -> Option<String> {
    lookup
        .records()
        .iter()
        .find(|r| {
            parse_dmarc_record(r)
                .v
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case("DMARC1"))
        })
        .cloned()
}

/// Whether a report destination domain accepts reports for a monitored domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAuthorization {
    /// The name that was queried, `<domain>._report._dmarc.<rua domain>`.
    pub name: String,
    /// `None` when the lookup timed out or failed, so nothing was measured.
    pub authorized: Option<bool>,
}

/// Evaluate the TXT lookup at an authorization record name.
pub fn evaluate_report_authorization(name: String, lookup: &TxtLookup) -> ReportAuthorization {
    let authorized = lookup
        .is_measured()
        .then(|| authorization_record(lookup).is_some());
    ReportAuthorization { name, authorized }
}

fn dmarc_records_from(lookup: &TxtLookup) -> Vec<String> {
    let mut records = lookup.records_starting_with("v=dmarc1");
    if records.is_empty() {
        records = lookup
            .records()
            .iter()
            .filter(|r| {
                parse_dmarc_record(r)
                    .v
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case("DMARC1"))
            })
            .cloned()
            .collect();
    }
    records
}

/// Minimal shape check for a report mailbox address (`rua=mailto:`):
/// `local@domain` with a valid hostname, no whitespace, and nothing that would
/// break a `mailto:` URI inside a DMARC record (`;`, `,`, `!`).
pub fn is_valid_report_address(addr: &str) -> bool {
    let Some((local, domain)) = addr.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !addr
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, ';' | ',' | '!' | '<' | '>'))
        && dmarc_domain::normalize_domain(domain).is_ok()
}

/// Evaluate a DMARC TXT lookup. `auth_lookups` is keyed by authorization record name.
pub fn evaluate_dmarc(
    domain: &str,
    lookup: &TxtLookup,
    auth_lookups: &[(String, TxtLookup)],
) -> DmarcCheckResult {
    let name = format!("_dmarc.{domain}");
    let records = dmarc_records_from(lookup);
    let mut findings = Vec::new();

    let mut result = DmarcCheckResult {
        domain: domain.to_string(),
        name,
        lookup: lookup.status(),
        lookup_detail: lookup.detail().map(str::to_string),
        records: records.clone(),
        record_count: records.len(),
        parsed: None,
        enforcement: Enforcement::default(),
        subdomain_enforcement: None,
        nonexistent_enforcement: None,
        pct_below_100: false,
        pct_status: Status::Neutral,
        testing_status: Status::Neutral,
        record_status: dmarc_record_status(!records.is_empty(), lookup.is_measured()),
        external_authorization: Vec::new(),
        rua_authorization: None,
        findings: Vec::new(),
    };

    match lookup {
        TxtLookup::Timeout => {
            findings.push(Finding::notice(
                CheckKind::Dmarc,
                "dmarc_timeout",
                format!(
                    "DNS lookup for _dmarc.{domain} timed out, so DMARC could not be measured."
                ),
                "Retry the check. This finding is about an incomplete lookup, not a confirmed DMARC defect.",
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::ServFail { detail } => {
            findings.push(Finding::notice(
                CheckKind::Dmarc,
                "dmarc_servfail",
                format!(
                    "DNS lookup for _dmarc.{domain} returned SERVFAIL ({detail}), so DMARC could not be measured."
                ),
                "Retry the check. This finding is about an incomplete lookup, not a confirmed DMARC defect.",
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::NxDomain | TxtLookup::NoRecords => {
            findings.push(Finding::critical(
                CheckKind::Dmarc,
                "dmarc_missing",
                format!(
                    "No DMARC TXT record was found at _dmarc.{domain}. Receivers have no policy to apply, so spoofed mail is not quarantined or rejected."
                ),
                format!(
                    "Publish a TXT record at _dmarc.{domain} such as `v=DMARC1; p=none; rua=mailto:dmarc@{domain}` and monitor reports before moving p= to quarantine or reject."
                ),
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::Records(_) => {}
    }

    if records.is_empty() {
        findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_missing",
            format!(
                "_dmarc.{domain} has TXT records, but none start with v=DMARC1, so receivers will ignore them."
            ),
            "Replace the TXT value with a valid DMARC record beginning with `v=DMARC1;`.",
        ));
        result.findings = findings;
        return result;
    }

    if records.len() > 1 {
        findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_multiple_records",
            format!(
                "Found {} DMARC TXT records at _dmarc.{domain}. RFC 7489 allows only one; receivers may ignore all of them.",
                records.len()
            ),
            "Delete extra _dmarc TXT records and keep a single v=DMARC1 record.",
        ));
    }

    let parsed = parse_dmarc_record(&records[0]);
    validate_parsed(domain, &parsed, &mut findings);

    result.enforcement = enforcement(parsed.p.as_deref());
    result.subdomain_enforcement = parsed.sp.as_deref().map(|sp| enforcement(Some(sp)));
    result.nonexistent_enforcement = parsed.np.as_deref().map(|np| enforcement(Some(np)));
    result.testing_status = testing_status(parsed.testing());
    let pct = parsed.pct.unwrap_or(100);
    result.pct_status = pct_status(parsed.pct);
    result.pct_below_100 = result.pct_status == Status::Warn;

    let mut external = Vec::new();
    for uri in parsed.rua.iter().chain(parsed.ruf.iter()) {
        let Some(host) = uri.host.as_deref() else {
            continue;
        };
        if !report_authorization_needed(domain, host) {
            continue;
        }
        let auth_name = report_auth_name(domain, host);
        let auth_lookup = auth_lookups
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&auth_name))
            .map(|(_, l)| l);
        let (auth_record, lookup_status) = match auth_lookup {
            Some(lookup) => (authorization_record(lookup), lookup.status()),
            None => (None, LookupStatus::NoRecords),
        };
        let authorized = auth_record.is_some();

        if !authorized {
            if lookup_status.is_measured() {
                findings.push(Finding::critical(
                    CheckKind::Dmarc,
                    "dmarc_rua_unauthorized",
                    format!(
                        "Reporting URI `{}` is outside the organizational domain of {domain}, but the authorization record at {auth_name} is missing or invalid. Receivers will silently drop reports to that address (RFC 7489 §7.1).",
                        uri.raw
                    ),
                    format!(
                        "Ask the report receiver to publish a TXT record at {auth_name} with value `v=DMARC1`. Until then, add a mailto: URI under {domain} as well so reports are not lost."
                    ),
                ));
            } else {
                findings.push(Finding::notice(
                    CheckKind::Dmarc,
                    "dmarc_rua_auth_unmeasured",
                    format!(
                        "DNS lookup for authorization record at {auth_name} timed out or failed to resolve. Authorization could not be verified."
                    ),
                    "Retry the check. This finding is about an incomplete lookup, not a confirmed DMARC defect.",
                ));
            }
        }

        external.push(ExternalAuthorization {
            uri: uri.raw.clone(),
            host: host.to_string(),
            authorized,
            auth_record_name: auth_name,
            auth_record,
            lookup: lookup_status,
            verdict: authorization_verdict(authorized, lookup_status.is_measured()),
        });
    }

    if no_issue_above_info(&findings) {
        let policy = parsed.p.as_deref().unwrap_or("none");
        findings.push(Finding::ok(
            CheckKind::Dmarc,
            "dmarc_ok",
            format!("DMARC record is valid with p={policy} and pct={pct}."),
            "Keep monitoring aggregate reports; no DMARC syntax changes are required.",
        ));
    }

    result.parsed = Some(parsed);
    result.rua_authorization = authorization_summary(external.iter().map(|e| e.verdict));
    result.external_authorization = external;
    result.findings = findings;
    result
}

fn validate_parsed(domain: &str, parsed: &ParsedDmarc, findings: &mut Vec<Finding>) {
    if parsed.v_not_first {
        findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_v_not_first",
            "The DMARC record does not start with the v tag. RFC 7489 requires `v=DMARC1` to be first; receivers will ignore the record.",
            "Reorder tags so the record begins with `v=DMARC1;`.",
        ));
    }
    match parsed.v.as_deref() {
        Some(v) if v.eq_ignore_ascii_case("DMARC1") => {}
        Some(v) => findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_bad_version",
            format!("DMARC version is `{v}`, not DMARC1. Receivers will ignore this record."),
            "Set the first tag to `v=DMARC1`.",
        )),
        None => findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_missing_version",
            "DMARC record is missing the required v tag.",
            "Start the record with `v=DMARC1;`.",
        )),
    }

    match parsed.p.as_deref() {
        Some(p) if VALID_POLICIES.contains(&p) => {
            if p == "none" {
                findings.push(Finding::critical(
                    CheckKind::Dmarc,
                    "dmarc_p_none",
                    "DMARC policy is p=none, which is monitoring only: receivers report on spoofed mail but still deliver it. That protects the domain no more than having no DMARC record.",
                    "Use the aggregate reports to confirm every legitimate sender passes SPF or DKIM alignment, then raise p= to quarantine and, once stable, to reject.",
                ));
            }
        }
        Some(p) => findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_bad_policy",
            format!("DMARC p={p} is not one of none, quarantine, or reject. Receivers may ignore the record."),
            "Set `p=` to none, quarantine, or reject.",
        )),
        None => findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_missing_policy",
            "DMARC record is missing the required p tag, so receivers have no policy to apply.",
            "Add `p=none` (or quarantine/reject) to the record.",
        )),
    }

    if let Some(sp) = parsed.sp.as_deref() {
        if !VALID_POLICIES.contains(&sp) {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_sp",
                format!("DMARC sp={sp} is not one of none, quarantine, or reject."),
                "Set `sp=` to none, quarantine, or reject, or omit it to inherit p=.",
            ));
        } else if sp == "none" && matches!(parsed.p.as_deref(), Some("quarantine" | "reject")) {
            // With p=none the domain already carries the critical finding; sp=none
            // only adds something when it opts subdomains out of enforcement.
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_sp_none",
                format!(
                    "DMARC sp=none exempts subdomains of {domain} from enforcement: mail spoofing any subdomain is monitored but still delivered."
                ),
                "Remove `sp=` so subdomains inherit p=, or set `sp=quarantine` or `sp=reject`. If some subdomains really need sp=none, `np=reject` (RFC 9989) still rejects mail from subdomains that do not exist.",
            ));
        }
    }

    if let Some(np) = parsed.np.as_deref() {
        if !VALID_POLICIES.contains(&np) {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_np",
                format!("DMARC np={np} is not one of none, quarantine, or reject."),
                "Set `np=` to none, quarantine, or reject, or omit it to inherit sp= (or p=).",
            ));
        }
    }
    let enforcing = matches!(parsed.p.as_deref(), Some("quarantine" | "reject"));
    let testing = parsed.testing();
    if let Some(t) = parsed.t.as_deref() {
        if !VALID_TESTING.contains(&t) {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_t",
                format!("DMARC t={t} is not y or n."),
                "Set `t=y` while testing a policy; otherwise remove the tag (n is the default).",
            ));
        }
    }
    if let Some(psd) = parsed.psd.as_deref() {
        if !VALID_PSD.contains(&psd) {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_psd",
                format!("DMARC psd={psd} is not y, n or u."),
                "Remove `psd=` (u is the default); only the operator of a public suffix publishes `psd=y`.",
            ));
        }
    }

    if let Some(raw) = parsed.pct_raw.as_deref() {
        findings.push(Finding::warning(
            CheckKind::Dmarc,
            "dmarc_pct_range",
            format!("DMARC pct={raw} is not an integer 0–100. Receivers may ignore pct or the whole record."),
            "Set `pct=` to an integer from 0 to 100. Use 100 unless you are rolling out enforcement.",
        ));
    } else if let Some(pct) = parsed.pct {
        if pct == 0 && enforcing {
            // pct=0 applies the policy to no failing mail: monitoring only, the
            // same outcome as p=none, so it carries the same severity.
            findings.push(Finding::critical(
                CheckKind::Dmarc,
                "dmarc_pct_zero",
                format!(
                    "DMARC p={} but pct=0, so the policy applies to no failing messages. This is monitoring only, like p=none.",
                    parsed.p.as_deref().unwrap_or("?")
                ),
                "Raise `pct=` (ideally to 100, or remove the tag) once legitimate mail passes alignment.",
            ));
        } else if pct < 100 && enforcing {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_pct_partial",
                format!(
                    "DMARC p={} but pct={pct}, so only {pct}% of failing messages receive that disposition.",
                    parsed.p.as_deref().unwrap_or("?")
                ),
                "Raise `pct=100` once you are confident legitimate mail passes alignment.",
            ));
        }
        if pct < 100 && enforcing && !testing {
            // RFC 9989 dropped pct: its receivers apply the policy to all
            // failing mail. With t=y they apply none of it, which the
            // testing finding (or pct=0's) already says.
            findings.push(Finding::info(
                CheckKind::Dmarc,
                "dmarc_pct_ignored",
                format!(
                    "DMARC pct={pct} is ignored by receivers that follow RFC 9989 (DMARCbis): they apply p={} to all failing mail.",
                    parsed.p.as_deref().unwrap_or("?")
                ),
                "Raise `pct=100` once legitimate mail passes alignment. To test a policy without enforcing it, add `t=y`, which DMARCbis receivers honour.",
            ));
        }
    }
    if testing && enforcing && parsed.pct != Some(0) {
        // t=y is DMARCbis's pct=0: the policy is published but not applied,
        // so it weighs like pct=0 (which already carries this when both are
        // set).
        findings.push(Finding::critical(
            CheckKind::Dmarc,
            "dmarc_testing",
            format!(
                "DMARC p={} but t=y (testing), so receivers that follow RFC 9989 (DMARCbis) do not apply the policy. This is monitoring only, like p=none.",
                parsed.p.as_deref().unwrap_or("?")
            ),
            "Remove `t=y` (or set `t=n`) once legitimate mail passes alignment.",
        ));
    }

    for uri in &parsed.rua {
        if !uri.valid {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_rua",
                format!(
                    "rua URI `{}` is not a well-formed mailto: (or https:) reporting address.",
                    uri.raw
                ),
                format!("Use `rua=mailto:dmarc@{domain}` (comma-separate additional URIs)."),
            ));
        }
    }
    for uri in &parsed.ruf {
        if !uri.valid {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_ruf",
                format!(
                    "ruf URI `{}` is not a well-formed mailto: (or https:) reporting address.",
                    uri.raw
                ),
                "Fix or remove the ruf= tag. Forensic URIs must be mailto: addresses.",
            ));
        }
    }
    if parsed.rua.is_empty() {
        findings.push(Finding::warning(
            CheckKind::Dmarc,
            "dmarc_no_rua",
            "DMARC record has no rua= tag, so you will not receive aggregate reports.",
            format!("Add `rua=mailto:dmarc@{domain}` (or your report-collector address)."),
        ));
    }

    if let Some(adkim) = parsed.adkim.as_deref() {
        if !VALID_ALIGNMENT.contains(&adkim) {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_adkim",
                format!("adkim={adkim} is not r (relaxed) or s (strict)."),
                "Set `adkim=r` or `adkim=s`.",
            ));
        }
    }
    if let Some(aspf) = parsed.aspf.as_deref() {
        if !VALID_ALIGNMENT.contains(&aspf) {
            findings.push(Finding::warning(
                CheckKind::Dmarc,
                "dmarc_bad_aspf",
                format!("aspf={aspf} is not r (relaxed) or s (strict)."),
                "Set `aspf=r` or `aspf=s`.",
            ));
        }
    }
    if let Some(rf) = parsed.rf.as_deref() {
        for part in rf.split(':') {
            if !VALID_RF.contains(&part) {
                findings.push(Finding::warning(
                    CheckKind::Dmarc,
                    "dmarc_bad_rf",
                    format!("rf contains `{part}`, which is not afrf or iodef."),
                    "Set `rf=afrf` or omit the tag.",
                ));
            }
        }
    }
    if let Some(fo) = parsed.fo.as_deref() {
        for part in fo.split(':') {
            if !matches!(part, "0" | "1" | "d" | "s") {
                findings.push(Finding::warning(
                    CheckKind::Dmarc,
                    "dmarc_bad_fo",
                    format!("fo contains `{part}`, which is not 0, 1, d, or s."),
                    "Use `fo=0` (default), `fo=1`, `fo=d`, and/or `fo=s`, colon-separated.",
                ));
            }
        }
    }
    for tag in &parsed.obsolete_tags {
        findings.push(Finding::info(
            CheckKind::Dmarc,
            "dmarc_obsolete_tag",
            format!(
                "DMARC record contains `{tag}=`, which RFC 9989 (DMARCbis) removed; receivers ignore it."
            ),
            format!("Remove `{tag}=` from the record; it has no effect."),
        ));
    }
    for tag in &parsed.unknown_tags {
        // Receivers ignore unknown tags; the record still authenticates.
        findings.push(Finding::notice(
            CheckKind::Dmarc,
            "dmarc_unknown_tag",
            format!("DMARC record contains unknown tag `{tag}=`, which receivers will ignore."),
            "Remove the unknown tag unless a receiver documents it as an extension.",
        ));
    }
    for part in &parsed.malformed_parts {
        findings.push(Finding::warning(
            CheckKind::Dmarc,
            "dmarc_malformed_tag",
            format!("DMARC record contains `{part}` without an `=` value."),
            "Each tag must be `name=value` and tags must be semicolon-separated.",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup::TxtAnswer;
    use dmarc_assess::EnforcementState;
    use dmarc_assess::Severity;

    fn rec(s: &str) -> TxtLookup {
        TxtLookup::Records(TxtAnswer::from_records(vec![s.to_string()]))
    }

    #[test]
    fn parses_full_record() {
        let p = parse_dmarc_record(
            "v=DMARC1; p=reject; sp=quarantine; pct=100; rua=mailto:dmarc@example.com; ruf=mailto:forensic@example.com; adkim=s; aspf=r; fo=1:d; ri=86400; rf=afrf",
        );
        assert_eq!(p.v.as_deref(), Some("DMARC1"));
        assert_eq!(p.p.as_deref(), Some("reject"));
        assert_eq!(p.sp.as_deref(), Some("quarantine"));
        assert_eq!(p.pct, Some(100));
        assert_eq!(p.rua.len(), 1);
        assert!(p.rua[0].valid);
        assert_eq!(p.rua[0].host.as_deref(), Some("example.com"));
        assert_eq!(p.adkim.as_deref(), Some("s"));
        assert_eq!(p.aspf.as_deref(), Some("r"));
        assert_eq!(p.fo.as_deref(), Some("1:d"));
        assert_eq!(p.ri, Some(86400));
        assert_eq!(p.rf.as_deref(), Some("afrf"));
        assert!(!p.v_not_first);
        assert!(p.unknown_tags.is_empty());
        assert_eq!(p.obsolete_tags, vec!["ri", "rf"]);
    }

    fn codes(r: &DmarcCheckResult) -> Vec<(&str, Severity)> {
        r.findings
            .iter()
            .map(|f| (f.code.as_str(), f.severity))
            .collect()
    }

    /// RFC 9989's `t=` and `np=` are tags, not unknown ones.
    #[test]
    fn parses_dmarcbis_tags() {
        let p = parse_dmarc_record("v=DMARC1; p=reject; np=Reject; t=Y");
        assert_eq!(p.np.as_deref(), Some("reject"));
        assert_eq!(p.t.as_deref(), Some("y"));
        assert!(p.testing());
        assert!(p.unknown_tags.is_empty());
        assert!(!parse_dmarc_record("v=DMARC1; p=reject; t=n").testing());
        assert!(!parse_dmarc_record("v=DMARC1; p=reject; t=yes").testing());
        assert!(!parse_dmarc_record("v=DMARC1; p=reject").testing());

        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; np=reject; t=n; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert_eq!(codes(&r), [("dmarc_ok", Severity::Ok)]);
        assert_eq!(
            r.nonexistent_enforcement.map(|e| e.state),
            Some(EnforcementState::Reject)
        );
        assert_eq!(r.testing_status, Status::Neutral);
    }

    #[test]
    fn malformed_t_and_np_are_warnings() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; np=drop; t=yes; rua=mailto:dmarc@example.com"),
            &[],
        );
        let c = codes(&r);
        assert!(c.contains(&("dmarc_bad_t", Severity::Warning)));
        assert!(c.contains(&("dmarc_bad_np", Severity::Warning)));
        assert!(!c.iter().any(|(code, _)| *code == "dmarc_unknown_tag"));
        // A malformed t is not testing.
        assert!(!c.iter().any(|(code, _)| *code == "dmarc_testing"));
    }

    /// RFC 9989's `psd=` is a tag: y, n and u raise nothing, another value warns.
    #[test]
    fn psd_is_a_tag_and_only_a_bad_value_warns() {
        let check = |psd: &str| {
            let raw = format!("v=DMARC1; p=reject; psd={psd}; rua=mailto:dmarc@example.com");
            let r = evaluate_dmarc("example.com", &rec(&raw), &[]);
            assert!(r.parsed.as_ref().unwrap().unknown_tags.is_empty(), "{psd}");
            let c = codes(&r);
            c.into_iter()
                .map(|(code, s)| (code.to_owned(), s))
                .collect::<Vec<_>>()
        };
        for ok in ["y", "n", "u", "Y"] {
            assert_eq!(check(ok), [("dmarc_ok".to_owned(), Severity::Ok)], "{ok}");
        }
        let bad = check("yes");
        assert_eq!(bad, [("dmarc_bad_psd".to_owned(), Severity::Warning)]);
    }

    /// t=y weighs like pct=0: monitoring only under an enforcing policy,
    /// nothing under p=none, and said once when both are published.
    #[test]
    fn testing_is_critical_like_pct_zero() {
        let testing = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=quarantine; t=y; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert_eq!(codes(&testing), [("dmarc_testing", Severity::Critical)]);
        assert_eq!(testing.testing_status, Status::Warn);
        let zero = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=quarantine; pct=0; rua=mailto:dmarc@example.com"),
            &[],
        );
        // The same weight as pct=0 (which also carries dmarc_pct_ignored:
        // DMARCbis receivers apply that policy in full).
        assert!(codes(&zero).contains(&("dmarc_pct_zero", Severity::Critical)));
        assert_eq!(
            dmarc_assess::score_from_findings(&testing.findings),
            dmarc_assess::score_from_findings(
                &zero
                    .findings
                    .iter()
                    .filter(|f| f.code != "dmarc_pct_ignored")
                    .cloned()
                    .collect::<Vec<_>>()
            )
        );

        let both = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; pct=0; t=y; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert_eq!(codes(&both), [("dmarc_pct_zero", Severity::Critical)]);

        let monitoring = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=none; t=y; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert_eq!(codes(&monitoring), [("dmarc_p_none", Severity::Critical)]);
    }

    /// pct below 100 is still honoured by RFC 7489 receivers; RFC 9989
    /// receivers apply the policy in full, which an Info finding says.
    #[test]
    fn partial_pct_notes_that_dmarcbis_ignores_it() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; pct=50; rua=mailto:dmarc@example.com"),
            &[],
        );
        let c = codes(&r);
        assert!(c.contains(&("dmarc_pct_partial", Severity::Warning)));
        assert!(c.contains(&("dmarc_pct_ignored", Severity::Info)));
        let zero = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; pct=0; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert!(codes(&zero).contains(&("dmarc_pct_ignored", Severity::Info)));
        // Not at 100, not under p=none, not with t=y (testing applies none).
        for record in [
            "v=DMARC1; p=reject; pct=100; rua=mailto:dmarc@example.com",
            "v=DMARC1; p=none; pct=50; rua=mailto:dmarc@example.com",
            "v=DMARC1; p=reject; pct=50; t=y; rua=mailto:dmarc@example.com",
        ] {
            let r = evaluate_dmarc("example.com", &rec(record), &[]);
            assert!(
                !r.findings.iter().any(|f| f.code == "dmarc_pct_ignored"),
                "{record}"
            );
        }
    }

    /// A result stored by checker 12, before the RFC 9989 fields, still
    /// reads back: it is shown, marked updating, until its re-check.
    #[test]
    fn a_result_without_the_dmarcbis_fields_still_reads() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; rua=mailto:dmarc@example.com"),
            &[],
        );
        let mut json = serde_json::to_value(&r).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("nonexistent_enforcement");
        obj.remove("testing_status");
        let parsed = obj["parsed"].as_object_mut().unwrap();
        for key in ["np", "t", "obsolete_tags"] {
            parsed.remove(key);
        }
        let old: DmarcCheckResult = serde_json::from_value(json).expect("old shape reads");
        assert_eq!(old, r);
    }

    #[test]
    fn rf_and_ri_are_obsolete_not_unknown() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; rua=mailto:dmarc@example.com; ri=86400; rf=afrf"),
            &[],
        );
        let obsolete: Vec<_> = r
            .findings
            .iter()
            .filter(|f| f.code == "dmarc_obsolete_tag")
            .collect();
        assert_eq!(obsolete.len(), 2);
        assert!(obsolete.iter().all(|f| f.severity == Severity::Info));
        assert!(obsolete[0].message.contains("`ri=`"));
        assert!(obsolete[1].message.contains("`rf=`"));
        // Info only: the record is still valid.
        assert!(r.findings.iter().any(|f| f.code == "dmarc_ok"));
        assert!(!r.findings.iter().any(|f| f.code == "dmarc_unknown_tag"));
        // An unparseable ri is still the obsolete tag.
        let p = parse_dmarc_record("v=DMARC1; p=none; ri=daily");
        assert_eq!((p.ri, p.obsolete_tags), (None, vec!["ri".to_owned()]));
    }

    #[test]
    fn flags_v_not_first_and_unknown_tags() {
        let p = parse_dmarc_record("p=none; v=DMARC1; foo=bar; pct=abc; notag");
        assert!(p.v_not_first);
        assert_eq!(p.unknown_tags, vec!["foo"]);
        assert_eq!(p.malformed_parts, vec!["notag"]);
        assert_eq!(p.pct, None);
        assert_eq!(p.pct_raw.as_deref(), Some("abc"));
    }

    #[test]
    fn mailto_size_suffix_and_https() {
        let p = parse_dmarc_record(
            "v=DMARC1; p=none; rua=mailto:dmarc@example.com!10m,https://reports.example.net/dmarc",
        );
        assert_eq!(p.rua.len(), 2);
        assert_eq!(p.rua[0].size_limit.as_deref(), Some("10m"));
        assert!(p.rua[0].valid);
        assert_eq!(p.rua[1].scheme, "https");
        assert!(p.rua[1].valid);
    }

    #[test]
    fn missing_record_is_critical() {
        let r = evaluate_dmarc("example.com", &TxtLookup::NxDomain, &[]);
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_missing" && f.severity == Severity::Critical));
        assert_eq!(r.enforcement.state, EnforcementState::Unknown);
    }

    #[test]
    fn multiple_records_are_an_rfc_violation() {
        let lookup = TxtLookup::Records(TxtAnswer::from_records(vec![
            "v=DMARC1; p=none".into(),
            "v=DMARC1; p=reject".into(),
        ]));
        let r = evaluate_dmarc("example.com", &lookup, &[]);
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_multiple_records"));
        assert_eq!(r.record_count, 2);
    }

    #[test]
    fn malformed_tags_and_bad_policy() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=drop; pct=150; rua=not-a-uri; xyz=1"),
            &[],
        );
        let codes: Vec<_> = r.findings.iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"dmarc_bad_policy"));
        assert!(codes.contains(&"dmarc_pct_range"));
        assert!(codes.contains(&"dmarc_bad_rua"));
        assert!(codes.contains(&"dmarc_unknown_tag"));
    }

    /// Monitoring-only protects no more than no record, so it weighs the same.
    #[test]
    fn p_none_is_critical_like_a_missing_record() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=none; rua=mailto:dmarc@example.com"),
            &[],
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "dmarc_p_none")
            .expect("p=none finding");
        assert_eq!(f.severity, Severity::Critical);
        assert!(f.message.contains("monitoring only"));
        assert!(f.suggestion.contains("quarantine") && f.suggestion.contains("reject"));
        assert!(!r.findings.iter().any(|f| f.code == "dmarc_ok"));
        let missing = evaluate_dmarc("example.com", &TxtLookup::NxDomain, &[]);
        assert_eq!(
            dmarc_assess::score_from_findings(&r.findings),
            dmarc_assess::score_from_findings(&missing.findings)
        );
    }

    #[test]
    fn sp_none_warns_only_when_the_parent_enforces() {
        let enforced = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; sp=none; rua=mailto:dmarc@example.com"),
            &[],
        );
        let f = enforced
            .findings
            .iter()
            .find(|f| f.code == "dmarc_sp_none")
            .expect("sp=none finding");
        assert_eq!(f.severity, Severity::Warning);

        let monitoring = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=none; sp=none; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert!(!monitoring
            .findings
            .iter()
            .any(|f| f.code == "dmarc_sp_none"));

        let strict_sub = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=quarantine; sp=reject; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert!(!strict_sub
            .findings
            .iter()
            .any(|f| f.code == "dmarc_sp_none"));
    }

    /// pct=0 enforces nothing, so it is treated like p=none.
    #[test]
    fn pct_zero_is_critical_partial_pct_is_a_warning() {
        let zero = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; pct=0; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert!(zero
            .findings
            .iter()
            .any(|f| f.code == "dmarc_pct_zero" && f.severity == Severity::Critical));
        assert!(!zero.findings.iter().any(|f| f.code == "dmarc_pct_partial"));

        let partial = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=quarantine; pct=50; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert!(partial
            .findings
            .iter()
            .any(|f| f.code == "dmarc_pct_partial" && f.severity == Severity::Warning));
    }

    #[test]
    fn enforcement_and_partial_pct() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; pct=25; rua=mailto:dmarc@example.com"),
            &[],
        );
        assert!(matches!(
            r.enforcement.state,
            EnforcementState::Quarantine | EnforcementState::Reject
        ));
        assert!(r.pct_below_100);
        assert!(r.findings.iter().any(|f| f.code == "dmarc_pct_partial"));
    }

    #[test]
    fn external_rua_requires_auth_record() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=none; rua=mailto:reports@thirdparty.test"),
            &[],
        );
        assert_eq!(r.external_authorization.len(), 1);
        assert!(!r.external_authorization[0].authorized);
        assert_eq!(
            r.external_authorization[0].auth_record_name,
            "example.com._report._dmarc.thirdparty.test"
        );
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_rua_unauthorized"));
    }

    #[test]
    fn external_rua_authorized_when_txt_present() {
        let auth_name = report_auth_name("example.com", "thirdparty.test");
        let auth = TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1".into()]));
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; rua=mailto:reports@thirdparty.test"),
            &[(auth_name, auth)],
        );
        assert!(r.external_authorization[0].authorized);
        assert!(!r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_rua_unauthorized"));
        assert!(matches!(
            r.enforcement.state,
            EnforcementState::Quarantine | EnforcementState::Reject
        ));
    }

    #[test]
    fn external_rua_timeout_is_unmeasured_notice_not_unauthorized() {
        let auth_name = report_auth_name("example.com", "thirdparty.test");
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=reject; rua=mailto:reports@thirdparty.test"),
            &[(auth_name, TxtLookup::Timeout)],
        );
        assert_eq!(r.external_authorization.len(), 1);
        assert!(!r.external_authorization[0].authorized);
        assert!(!r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_rua_unauthorized"));
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "dmarc_rua_auth_unmeasured")
            .expect("unmeasured finding");
        assert_eq!(f.severity, Severity::Notice);
    }

    #[test]
    fn report_authorization_is_measured_or_unknown() {
        let name = "example.com._report._dmarc.reports.test".to_string();
        let eval =
            |lookup: &TxtLookup| evaluate_report_authorization(name.clone(), lookup).authorized;
        assert_eq!(eval(&rec("v=DMARC1")), Some(true));
        assert_eq!(eval(&rec("some other text")), Some(false));
        assert_eq!(eval(&TxtLookup::NxDomain), Some(false));
        assert_eq!(eval(&TxtLookup::NoRecords), Some(false));
        assert_eq!(eval(&TxtLookup::Timeout), None);
        let r = evaluate_report_authorization(name.clone(), &TxtLookup::Timeout);
        assert_eq!(r.name, name);
    }

    #[test]
    fn same_domain_rua_skips_auth() {
        let r = evaluate_dmarc(
            "example.com",
            &rec("v=DMARC1; p=quarantine; rua=mailto:dmarc@mail.example.com"),
            &[],
        );
        assert!(r.external_authorization.is_empty());
        assert!(matches!(
            r.enforcement.state,
            EnforcementState::Quarantine | EnforcementState::Reject
        ));
    }

    #[test]
    fn same_organizational_domain_rua_needs_no_authorization() {
        let r = evaluate_dmarc(
            "shop.acme.example",
            &rec("v=DMARC1; p=reject; rua=mailto:dmarc@acme.example"),
            &[],
        );
        assert!(r.external_authorization.is_empty());
        assert!(!r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_rua_unauthorized"));
    }

    #[test]
    fn timeout_is_notice_and_unmeasured() {
        let r = evaluate_dmarc("example.com", &TxtLookup::Timeout, &[]);
        assert!(!r.lookup.is_measured());
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "dmarc_timeout")
            .expect("timeout finding");
        assert_eq!(f.severity, Severity::Notice);
    }

    #[test]
    fn servfail_is_notice_and_unmeasured() {
        let r = evaluate_dmarc(
            "example.com",
            &TxtLookup::ServFail {
                detail: "refused".into(),
            },
            &[],
        );
        assert!(!r.lookup.is_measured());
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "dmarc_servfail")
            .expect("servfail finding");
        assert_eq!(f.severity, Severity::Notice);
    }

    #[test]
    fn nxdomain_stays_critical_and_measured() {
        let r = evaluate_dmarc("example.com", &TxtLookup::NxDomain, &[]);
        assert!(r.lookup.is_measured());
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "dmarc_missing" && f.severity == Severity::Critical));
    }

    #[test]
    fn report_address_shape() {
        assert!(is_valid_report_address("dmarc@example.com"));
        assert!(is_valid_report_address("dmarc+reports@sub.example.org"));
        assert!(!is_valid_report_address("example.com"));
        assert!(!is_valid_report_address("@example.com"));
        assert!(!is_valid_report_address("a b@example.com"));
        assert!(!is_valid_report_address("a;b@example.com"));
        assert!(!is_valid_report_address("a@not a host"));
    }
}
