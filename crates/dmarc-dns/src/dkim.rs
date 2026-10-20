//! DKIM selector TXT parsing and key-strength evaluation (RFC 6376).

use crate::lookup::{parse_tag_list, LookupStatus, TxtLookup};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dmarc_assess::Status;
use dmarc_assess::{
    dkim_summary, key_status, selector_verdict, KeyStrength, SelectorFacts, SelectorVerdict,
};
use dmarc_assess::{no_issue_above_info, CheckKind, Finding};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A selector seen signing mail for the domain in aggregate reports.
///
/// The counts are the report volume behind the observation: how many messages
/// carried a signature with this selector that verified, and how many did not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedSelector {
    pub selector: String,
    pub pass_messages: i64,
    pub fail_messages: i64,
}

impl ObservedSelector {
    /// An observation without volume (tests, callers without report data).
    pub fn named(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            pass_messages: 0,
            fail_messages: 0,
        }
    }
}

/// Which selectors to probe, and where each one came from.
///
/// Nothing is guessed. Aggregate reports are the source of truth: a selector
/// seen signing real mail is evidence that a key must exist. Selectors the user
/// configured (or typed into the inspector) are probes, not evidence, so a
/// missing key there is only worth a notice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DkimProbe {
    /// Selectors seen signing mail for this domain in aggregate reports.
    pub observed: Vec<ObservedSelector>,
    /// Selectors the user asked us to check: the settings list, or names
    /// passed explicitly to the inspector.
    pub configured: Vec<String>,
}

/// A selector is one DNS label, not a hostname: only trimmed and lowercased
/// (no trailing-dot handling, unlike `dmarc_domain::normalize_name`).
fn normalize_selector(s: &str) -> Option<String> {
    let n = s.trim().to_ascii_lowercase();
    (!n.is_empty()).then_some(n)
}

fn normalize(selectors: &[String]) -> Vec<String> {
    let mut v: Vec<String> = selectors
        .iter()
        .filter_map(|s| normalize_selector(s))
        .collect();
    v.sort();
    v.dedup();
    v
}

impl DkimProbe {
    /// Probe the report-observed selectors plus the user-configured ones.
    pub fn new(observed: Vec<ObservedSelector>, configured: &[String]) -> Self {
        let mut merged: Vec<ObservedSelector> = Vec::with_capacity(observed.len());
        for o in observed {
            let Some(name) = normalize_selector(&o.selector) else {
                continue;
            };
            match merged.iter_mut().find(|m| m.selector == name) {
                Some(m) => {
                    m.pass_messages += o.pass_messages;
                    m.fail_messages += o.fail_messages;
                }
                None => merged.push(ObservedSelector {
                    selector: name,
                    ..o
                }),
            }
        }
        Self {
            observed: merged,
            configured: normalize(configured),
        }
    }

    /// Probe exactly what the caller named (the standalone inspector).
    pub fn requested(selectors: &[String]) -> Self {
        Self::new(Vec::new(), selectors)
    }

    /// Observed selector names, in report order (most used first).
    pub fn observed_names(&self) -> Vec<String> {
        self.observed.iter().map(|o| o.selector.clone()).collect()
    }

    pub fn observation(&self, selector: &str) -> Option<&ObservedSelector> {
        self.observed.iter().find(|o| o.selector == selector)
    }

    pub fn is_configured(&self, selector: &str) -> bool {
        self.configured.iter().any(|c| c == selector)
    }

    /// Nothing to probe: no reports have shown a selector and none is configured.
    pub fn is_empty(&self) -> bool {
        self.observed.is_empty() && self.configured.is_empty()
    }

    /// The selectors to query: observed and configured, sorted and deduplicated.
    pub fn selectors(&self) -> Vec<String> {
        let mut all = self.observed_names();
        all.extend(self.configured.iter().cloned());
        all.sort();
        all.dedup();
        all
    }

    /// Cache key that changes whenever a selector's origin changes.
    pub fn cache_key(&self) -> String {
        let mut observed = self.observed_names();
        observed.sort();
        format!("o={};c={}", observed.join(","), self.configured.join(","))
    }
}

/// Parsed DKIM key record tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedDkim {
    pub v: Option<String>,
    pub key_type: String,
    pub public_key: Option<String>,
    pub flags: Vec<String>,
    pub notes: Option<String>,
    pub unknown_tags: Vec<String>,
    pub revoked: bool,
}

/// Result for one selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DkimSelectorResult {
    pub selector: String,
    pub name: String,
    pub present: bool,
    pub lookup: LookupStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key_bits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key_strength: Option<KeyStrength>,
    /// Verdict on the key's strength; `neutral` without a key.
    pub key_status: Status,
    pub test_mode: bool,
    pub revoked: bool,
    /// Seen signing mail for the domain in aggregate reports.
    pub seen_in_reports: bool,
    /// On the user's selector list (or named explicitly in the request).
    pub configured: bool,
    /// Report messages whose signature with this selector verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub report_pass_messages: Option<i64>,
    /// Report messages whose signature with this selector did not verify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub report_fail_messages: Option<i64>,
    pub findings: Vec<Finding>,
    /// What this selector says (set once its report observations are known).
    pub verdict: SelectorVerdict,
}

impl DkimSelectorResult {
    /// What was measured, for the verdict.
    fn facts(&self) -> SelectorFacts {
        SelectorFacts {
            measured: self.lookup.is_measured(),
            present: self.present,
            seen_in_reports: self.seen_in_reports,
            revoked: self.revoked,
            key_strength: self.key_strength,
            test_mode: self.test_mode,
        }
    }
}

/// Aggregated DKIM check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DkimCheckResult {
    pub domain: String,
    pub selectors_queried: Vec<String>,
    /// Selectors our aggregate reports have seen signing mail for this domain.
    pub observed_selectors: Vec<String>,
    /// Selectors checked because the user configured or requested them.
    pub configured_selectors: Vec<String>,
    /// One row per probed selector, whether or not a key answered.
    pub records: Vec<DkimSelectorResult>,
    pub discovered_count: usize,
    pub findings: Vec<Finding>,
    /// The selectors as one verdict (see [`dkim_summary`]).
    pub summary: SelectorVerdict,
}

/// Parse a DKIM TXT value (`v=DKIM1; k=rsa; p=...`).
pub fn parse_dkim_record(raw: &str) -> ParsedDkim {
    let mut parsed = ParsedDkim {
        v: None,
        key_type: "rsa".into(),
        public_key: None,
        flags: Vec::new(),
        notes: None,
        unknown_tags: Vec::new(),
        revoked: false,
    };
    for (key, value, has_eq) in parse_tag_list(raw) {
        if !has_eq {
            continue;
        }
        match key.as_str() {
            "v" => parsed.v = Some(value),
            "k" => parsed.key_type = value.to_ascii_lowercase(),
            "p" => {
                let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
                if compact.is_empty() {
                    parsed.revoked = true;
                    parsed.public_key = Some(String::new());
                } else {
                    parsed.public_key = Some(compact);
                }
            }
            "t" => {
                parsed.flags = value
                    .split(':')
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "n" => parsed.notes = Some(value),
            other => parsed.unknown_tags.push(other.to_string()),
        }
    }
    parsed
}

/// A selector is one or more DNS labels (`s1`, `k1`, `2024.mail`).
pub fn is_valid_selector(selector: &str) -> bool {
    !selector.is_empty()
        && selector.len() <= 63
        && selector.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Evaluate one selector's TXT lookup.
pub fn evaluate_dkim_selector(
    domain: &str,
    selector: &str,
    lookup: &TxtLookup,
) -> DkimSelectorResult {
    let name = format!("{selector}._domainkey.{domain}");
    let mut findings = Vec::new();
    let mut result = DkimSelectorResult {
        selector: selector.to_string(),
        name: name.clone(),
        present: false,
        lookup: lookup.status(),
        raw: None,
        key_type: None,
        key_bits: None,
        key_strength: None,
        key_status: Status::Neutral,
        test_mode: false,
        revoked: false,
        seen_in_reports: false,
        configured: false,
        report_pass_messages: None,
        report_fail_messages: None,
        findings: Vec::new(),
        verdict: SelectorVerdict::default(),
    };

    match lookup {
        TxtLookup::Timeout => {
            findings.push(Finding::notice(
                CheckKind::Dkim,
                "dkim_timeout",
                format!("DNS lookup for {name} timed out, so this selector could not be measured."),
                "Retry the check. This finding is about an incomplete lookup, not a confirmed DKIM defect.",
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::ServFail { detail } => {
            findings.push(Finding::notice(
                CheckKind::Dkim,
                "dkim_servfail",
                format!(
                    "DNS lookup for {name} returned SERVFAIL ({detail}), so this selector could not be measured."
                ),
                "Retry the check. This finding is about an incomplete lookup, not a confirmed DKIM defect.",
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::NxDomain | TxtLookup::NoRecords => {
            result.findings = findings;
            return result;
        }
        TxtLookup::Records(_) => {}
    }

    let records = lookup.records();
    if records.is_empty() {
        result.findings = findings;
        return result;
    }

    let raw = records.join("");
    result.raw = Some(raw.clone());
    result.present = true;
    let parsed = parse_dkim_record(&raw);
    result.key_type = Some(parsed.key_type.clone());
    result.revoked = parsed.revoked;
    result.test_mode = parsed.flags.iter().any(|f| f == "y");

    if parsed.revoked {
        findings.push(Finding::warning(
            CheckKind::Dkim,
            "dkim_revoked",
            format!("DKIM selector `{selector}` has an empty p= tag, which revokes the key. Signatures using it will fail."),
            format!("Publish a new key at {name} or stop signing with `{selector}`."),
        ));
        result.findings = findings;
        return result;
    }

    match parsed.key_type.as_str() {
        "rsa" => match decoded_key_bits(parsed.public_key.as_deref()) {
            Some(bits) => {
                result.key_bits = Some(bits);
                let strength = KeyStrength::of_rsa_bits(bits);
                result.key_strength = Some(strength);
                match strength {
                    KeyStrength::Weak => findings.push(Finding::critical(
                        CheckKind::Dkim,
                        "dkim_key_weak",
                        format!("DKIM selector `{selector}` uses a {bits}-bit RSA key, below 1024 bits. Receivers may reject the signature."),
                        "Generate a new 2048-bit RSA key (or Ed25519) and rotate the selector.",
                    )),
                    KeyStrength::Dated => findings.push(Finding::warning(
                        CheckKind::Dkim,
                        "dkim_key_dated",
                        format!("DKIM selector `{selector}` uses a {bits}-bit RSA key. 1024-bit RSA is acceptable on some receivers but dated; 2048-bit is the current baseline."),
                        "Rotate to a 2048-bit RSA or Ed25519 key.",
                    )),
                    KeyStrength::Good => findings.push(Finding::ok(
                        CheckKind::Dkim,
                        "dkim_key_good",
                        format!("DKIM selector `{selector}` uses a {bits}-bit RSA key."),
                        "No key-length change needed.",
                    )),
                    KeyStrength::Unknown => {}
                }
            }
            None => {
                result.key_strength = Some(KeyStrength::Unknown);
                findings.push(Finding::warning(
                    CheckKind::Dkim,
                    "dkim_key_unreadable",
                    format!("DKIM selector `{selector}` has a p= value that is not a decodable RSA public key."),
                    "Republish p= as unpadded Base64 of a PKCS#1 or SPKI RSA public key.",
                ));
            }
        },
        "ed25519" => {
            result.key_strength = Some(KeyStrength::Good);
            findings.push(Finding::ok(
                CheckKind::Dkim,
                "dkim_ed25519",
                format!("DKIM selector `{selector}` uses Ed25519."),
                "Ed25519 is a modern DKIM algorithm; keep a dual RSA selector if some receivers still require RSA.",
            ));
        }
        other => findings.push(Finding::warning(
            CheckKind::Dkim,
            "dkim_unknown_k",
            format!("DKIM selector `{selector}` has k={other}, which most receivers will ignore."),
            "Use k=rsa (default) or k=ed25519.",
        )),
    }

    if result.test_mode {
        findings.push(Finding::warning(
            CheckKind::Dkim,
            "dkim_test_mode",
            format!("DKIM selector `{selector}` sets t=y (testing). Some receivers treat signatures as informational only."),
            "Remove t=y from the key record when you are ready for production verification.",
        ));
    }

    result.findings = findings;
    result
}

/// Combine per-selector results into the domain's DKIM verdict.
///
/// * Key found: judged on its own merits (strength, revocation, test mode).
/// * Observed selector, key definitively absent: `Warning`, because mail signed
///   with it cannot verify.
/// * Configured-only selector, key absent: `Notice`. It is a probe, not
///   evidence that the domain signs with it.
/// * Lookup timed out or SERVFAILed: `Notice` from the selector itself; no
///   verdict about absence is drawn from it.
/// * Nothing to probe: a zero-deduction `Notice` that DKIM cannot be assessed
///   yet. Nothing is guessed.
pub fn evaluate_dkim(
    domain: &str,
    selectors: &[DkimSelectorResult],
    probe: &DkimProbe,
) -> DkimCheckResult {
    let mut records: Vec<DkimSelectorResult> = selectors.to_vec();
    for row in &mut records {
        let observation = probe.observation(&row.selector);
        row.seen_in_reports = observation.is_some();
        row.configured = probe.is_configured(&row.selector);
        row.report_pass_messages = observation.map(|o| o.pass_messages);
        row.report_fail_messages = observation.map(|o| o.fail_messages);
        row.verdict = selector_verdict(&row.facts());
        row.key_status = key_status(row.key_strength);
        if row.present || !row.lookup.is_measured() {
            continue;
        }
        let finding = match observation {
            Some(o) => observed_selector_missing(domain, row, o),
            None => configured_selector_missing(domain, row),
        };
        row.findings.push(finding);
    }

    let mut findings: Vec<Finding> = records
        .iter()
        .flat_map(|s| s.findings.iter().cloned())
        .collect();
    let discovered = records.iter().filter(|s| s.present).count();
    if probe.is_empty() {
        findings.push(Finding::notice(
            CheckKind::Dkim,
            "dkim_unassessed",
            format!(
                "DKIM can't be assessed yet: no signing selectors for {domain} have been seen in aggregate reports."
            ),
            "Add a selector to check it now, or wait for aggregate reports to show which selectors your senders sign with.",
        ));
    } else if discovered > 0 && no_issue_above_info(&findings) {
        findings.push(Finding::ok(
            CheckKind::Dkim,
            "dkim_ok",
            format!("Found {discovered} DKIM selector(s) for {domain} with acceptable keys."),
            "Keep rotating keys periodically; no DKIM syntax changes are required.",
        ));
    }
    DkimCheckResult {
        summary: dkim_summary(records.iter().map(|r| r.verdict)),
        domain: domain.to_string(),
        selectors_queried: records.iter().map(|s| s.selector.clone()).collect(),
        observed_selectors: probe.observed_names(),
        configured_selectors: probe.configured.clone(),
        records,
        discovered_count: discovered,
        findings,
    }
}

fn absence(row: &DkimSelectorResult) -> &'static str {
    if row.lookup == LookupStatus::Nxdomain {
        "the name does not exist (NXDOMAIN)"
    } else {
        "no TXT record answered"
    }
}

/// Reports show mail signed with this selector, but its key is gone.
fn observed_selector_missing(
    domain: &str,
    row: &DkimSelectorResult,
    seen: &ObservedSelector,
) -> Finding {
    let total = seen.pass_messages + seen.fail_messages;
    let volume = if total > 0 {
        format!(
            " Recent aggregate reports show {total} message(s) signed with it ({} verified, {} did not).",
            seen.pass_messages, seen.fail_messages
        )
    } else {
        String::new()
    };
    Finding::warning(
        CheckKind::Dkim,
        "dkim_observed_selector_missing",
        format!(
            "Selector `{}` signs mail for {domain}, but no DKIM key is published at {}: {}. Mail signed with it cannot verify.{volume}",
            row.selector,
            row.name,
            absence(row),
        ),
        format!(
            "Publish the public key for `{}` at {} (or the CNAME your provider gives you), or stop signing with it.",
            row.selector, row.name
        ),
    )
}

/// A selector the user asked about is not published. Not evidence of failing mail.
fn configured_selector_missing(domain: &str, row: &DkimSelectorResult) -> Finding {
    Finding::notice(
        CheckKind::Dkim,
        "dkim_selector_not_published",
        format!(
            "Selector `{}` is not published at {}: {}. It was checked on request, not because aggregate reports show {domain} signing with it.",
            row.selector,
            row.name,
            absence(row),
        ),
        "If this domain does not sign with this selector, ignore this or remove it from the DKIM selector list.",
    )
}

fn decoded_key_bits(p: Option<&str>) -> Option<usize> {
    let p = p?;
    let der = STANDARD.decode(p).ok()?;
    rsa_modulus_bits(&der)
}

fn rsa_modulus_bits(der: &[u8]) -> Option<usize> {
    pkcs1_modulus_bits(der).or_else(|| spki_modulus_bits(der))
}

fn der_len(data: &[u8]) -> Option<(usize, usize)> {
    let first = *data.first()?;
    if first < 0x80 {
        return Some((1, first as usize));
    }
    let n = (first - 0x80) as usize;
    if n == 0 || n > 4 || data.len() < 1 + n {
        return None;
    }
    let mut len = 0usize;
    for b in &data[1..=n] {
        len = (len << 8) | usize::from(*b);
    }
    Some((1 + n, len))
}

fn seq_body(data: &[u8]) -> Option<&[u8]> {
    if *data.first()? != 0x30 {
        return None;
    }
    let (hl, len) = der_len(&data[1..])?;
    data.get(1 + hl..1 + hl + len)
}

fn take_int(data: &[u8]) -> Option<(&[u8], &[u8])> {
    if *data.first()? != 0x02 {
        return None;
    }
    let (hl, len) = der_len(&data[1..])?;
    let start = 1 + hl;
    Some((data.get(start..start + len)?, data.get(start + len..)?))
}

fn modulus_bit_len(n: &[u8]) -> usize {
    let n = n.strip_prefix(&[0x00]).unwrap_or(n);
    if n.is_empty() {
        return 0;
    }
    n.len() * 8 - n[0].leading_zeros() as usize
}

fn pkcs1_modulus_bits(der: &[u8]) -> Option<usize> {
    let body = seq_body(der)?;
    let (n, _rest) = take_int(body)?;
    Some(modulus_bit_len(n))
}

fn spki_modulus_bits(der: &[u8]) -> Option<usize> {
    let body = seq_body(der)?;
    if *body.first()? != 0x30 {
        return None;
    }
    let (hl, len) = der_len(&body[1..])?;
    let after_alg = body.get(1 + hl + len)?;
    if *after_alg != 0x03 {
        return None;
    }
    let rest = &body[1 + hl + len..];
    let (bhl, blen) = der_len(rest.get(1..)?)?;
    let bits = rest.get(1 + bhl..1 + bhl + blen)?;
    let pkcs1 = bits.strip_prefix(&[0x00]).unwrap_or(bits);
    pkcs1_modulus_bits(pkcs1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup::TxtAnswer;
    use dmarc_assess::Severity;

    fn encode_len(n: usize) -> Vec<u8> {
        if n < 0x80 {
            vec![n as u8]
        } else if n < 256 {
            vec![0x81, n as u8]
        } else {
            vec![0x82, (n >> 8) as u8, (n & 0xff) as u8]
        }
    }

    fn encode_int(bytes: &[u8]) -> Vec<u8> {
        let mut v = vec![0x02];
        v.extend(encode_len(bytes.len()));
        v.extend(bytes);
        v
    }

    fn pkcs1(modulus: &[u8], exp: &[u8]) -> Vec<u8> {
        let inner = {
            let mut b = encode_int(modulus);
            b.extend(encode_int(exp));
            b
        };
        let mut out = vec![0x30];
        out.extend(encode_len(inner.len()));
        out.extend(inner);
        out
    }

    #[test]
    fn parses_tags_and_revocation() {
        let p = parse_dkim_record("v=DKIM1; k=rsa; t=y:s; p=");
        assert!(p.revoked);
        assert!(p.flags.contains(&"y".to_string()));
        assert_eq!(p.key_type, "rsa");
    }

    #[test]
    fn rsa_1024_is_dated_2048_is_good() {
        let n1024 = vec![0x80; 128];
        let n2048 = vec![0x80; 256];
        let e = vec![0x01, 0x00, 0x01];
        let b1024 = STANDARD.encode(pkcs1(&n1024, &e));
        let b2048 = STANDARD.encode(pkcs1(&n2048, &e));
        let weak = evaluate_dkim_selector(
            "example.com",
            "s1",
            &TxtLookup::Records(TxtAnswer::from_records(vec![format!(
                "v=DKIM1; k=rsa; p={b1024}"
            )])),
        );
        assert_eq!(weak.key_bits, Some(1024));
        assert_eq!(weak.key_strength, Some(KeyStrength::Dated));
        assert!(weak.findings.iter().any(|f| f.code == "dkim_key_dated"));

        let good = evaluate_dkim_selector(
            "example.com",
            "s2",
            &TxtLookup::Records(TxtAnswer::from_records(vec![format!("v=DKIM1; p={b2048}")])),
        );
        assert_eq!(good.key_bits, Some(2048));
        assert_eq!(good.key_strength, Some(KeyStrength::Good));
        assert!(!good.test_mode);
    }

    fn key_2048() -> TxtLookup {
        let n = vec![0x80; 256];
        let e = vec![0x01, 0x00, 0x01];
        let b64 = STANDARD.encode(pkcs1(&n, &e));
        TxtLookup::Records(TxtAnswer::from_records(vec![format!("v=DKIM1; p={b64}")]))
    }

    fn observed(names: &[&str]) -> Vec<ObservedSelector> {
        names.iter().map(|n| ObservedSelector::named(*n)).collect()
    }

    #[test]
    fn test_mode_is_flagged() {
        let n = vec![0x80; 256];
        let e = vec![0x01, 0x00, 0x01];
        let b64 = STANDARD.encode(pkcs1(&n, &e));
        let r = evaluate_dkim_selector(
            "example.com",
            "google",
            &TxtLookup::Records(TxtAnswer::from_records(vec![format!(
                "v=DKIM1; t=y; p={b64}"
            )])),
        );
        assert!(r.test_mode);
        assert!(r.findings.iter().any(|f| f.code == "dkim_test_mode"));
    }

    /// No guessing: the probe is exactly observed plus configured.
    #[test]
    fn probe_is_observed_union_configured_without_guesses() {
        let probe = DkimProbe::new(
            vec![
                ObservedSelector {
                    selector: "crsend".into(),
                    pass_messages: 5,
                    fail_messages: 1,
                },
                ObservedSelector {
                    selector: " CRSEND ".into(),
                    pass_messages: 2,
                    fail_messages: 0,
                },
                ObservedSelector::named("selector1-example-onmicrosoft-com"),
                ObservedSelector::named("  "),
            ],
            &["K1".into(), "k1".into(), "crsend".into(), String::new()],
        );
        assert_eq!(
            probe.selectors(),
            vec!["crsend", "k1", "selector1-example-onmicrosoft-com"]
        );
        let crsend = probe.observation("crsend").expect("merged");
        assert_eq!((crsend.pass_messages, crsend.fail_messages), (7, 1));
        assert!(probe.is_configured("crsend") && probe.is_configured("k1"));
        assert!(DkimProbe::default().selectors().is_empty());
        assert_eq!(DkimProbe::requested(&["k1".into()]).selectors(), vec!["k1"]);
    }

    /// No reports and no configured selectors: say so, cost nothing.
    #[test]
    fn nothing_to_probe_is_an_unassessed_notice() {
        let r = evaluate_dkim("example.com", &[], &DkimProbe::default());
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "dkim_unassessed")
            .expect("unassessed notice");
        assert_eq!(f.severity, Severity::Notice);
        assert!(f.message.contains("can't be assessed yet"));
        assert!(f.suggestion.contains("Add a selector"));
        assert_eq!(dmarc_assess::score_from_findings(&r.findings), 100);
    }

    /// An observed selector with no key means signed mail cannot verify.
    #[test]
    fn observed_selector_without_key_is_a_warning_with_report_counts() {
        let probe = DkimProbe::new(
            vec![ObservedSelector {
                selector: "crsend".into(),
                pass_messages: 40,
                fail_messages: 12,
            }],
            &[],
        );
        for lookup in [TxtLookup::NxDomain, TxtLookup::NoRecords] {
            let row = evaluate_dkim_selector("example.ch", "crsend", &lookup);
            let r = evaluate_dkim("example.ch", &[row], &probe);
            let f = r
                .findings
                .iter()
                .find(|f| f.code == "dkim_observed_selector_missing")
                .expect("warning");
            assert_eq!(f.severity, Severity::Warning);
            assert!(f.message.contains("crsend") && f.message.contains("cannot verify"));
            assert!(f.message.contains("52 message(s)") && f.message.contains("40 verified"));
            assert_eq!(r.records.len(), 1, "absent rows are still reported");
            assert!(r.records[0].seen_in_reports);
            assert_eq!(r.records[0].report_fail_messages, Some(12));
            assert_eq!(dmarc_assess::score_from_findings(&r.findings), 92);
        }
    }

    /// A configured selector that is not published is a probe, not evidence.
    #[test]
    fn configured_selector_without_key_is_only_a_notice() {
        let row = evaluate_dkim_selector("example.ch", "k1", &TxtLookup::NxDomain);
        let r = evaluate_dkim("example.ch", &[row], &DkimProbe::requested(&["k1".into()]));
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "dkim_selector_not_published")
            .expect("notice");
        assert_eq!(f.severity, Severity::Notice);
        assert!(r.records[0].configured && !r.records[0].seen_in_reports);
        assert_eq!(dmarc_assess::score_from_findings(&r.findings), 100);
    }

    /// A resolver failure is not a measurement: notice only, no absence claim,
    /// no deduction, even though the selector was observed.
    #[test]
    fn unmeasured_observed_selector_claims_nothing() {
        let probe = DkimProbe::new(observed(&["crsend"]), &[]);
        for lookup in [
            TxtLookup::Timeout,
            TxtLookup::ServFail {
                detail: "refused".into(),
            },
        ] {
            let unread = evaluate_dkim_selector("example.ch", "crsend", &lookup);
            let r = evaluate_dkim("example.ch", std::slice::from_ref(&unread), &probe);
            assert!(
                !r.findings
                    .iter()
                    .any(|f| f.code == "dkim_observed_selector_missing"
                        || f.code == "dkim_unassessed"),
                "unmeasured lookups must not produce an absence verdict: {lookup:?}"
            );
            assert!(r.findings.iter().all(|f| f.severity == Severity::Notice));
            assert_eq!(dmarc_assess::score_from_findings(&r.findings), 100);
        }
    }

    /// One observed selector unreadable, another measured absent: only the
    /// measured one is judged.
    #[test]
    fn mixed_measured_and_unmeasured_selectors() {
        let unread = evaluate_dkim_selector("example.ch", "crsend", &TxtLookup::Timeout);
        let absent = evaluate_dkim_selector("example.ch", "k1", &TxtLookup::NxDomain);
        let r = evaluate_dkim(
            "example.ch",
            &[unread, absent],
            &DkimProbe::new(observed(&["crsend", "k1"]), &[]),
        );
        let missing: Vec<_> = r
            .findings
            .iter()
            .filter(|f| f.code == "dkim_observed_selector_missing")
            .collect();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].message.contains("`k1`"));
        assert!(r.findings.iter().any(|f| f.code == "dkim_timeout"));
    }

    /// Finding the observed selector's key is the whole point: no complaint.
    #[test]
    fn a_resolving_observed_selector_produces_no_complaint() {
        let found = evaluate_dkim_selector("example.ch", "crsend", &key_2048());
        let agg = evaluate_dkim(
            "example.ch",
            &[found],
            &DkimProbe::new(observed(&["crsend"]), &[]),
        );
        assert_eq!(agg.discovered_count, 1);
        assert!(agg.findings.iter().any(|f| f.code == "dkim_ok"));
        assert_eq!(agg.records[0].key_bits, Some(2048));
        assert_eq!(dmarc_assess::score_from_findings(&agg.findings), 100);
    }

    /// Probes that never ran (overall timeout) say nothing either way.
    #[test]
    fn probing_nothing_says_nothing() {
        let agg = evaluate_dkim(
            "example.com",
            &[],
            &DkimProbe::new(observed(&["crsend"]), &[]),
        );
        assert!(agg.findings.is_empty());
        assert_eq!(dmarc_assess::score_from_findings(&agg.findings), 100);
    }

    #[test]
    fn selector_shape() {
        assert!(is_valid_selector("s1"));
        assert!(is_valid_selector("2024.mail"));
        assert!(is_valid_selector("k-1_a"));
        assert!(!is_valid_selector(""));
        assert!(!is_valid_selector("a..b"));
        assert!(!is_valid_selector("a b"));
        assert!(!is_valid_selector(&"x".repeat(64)));
    }
}
