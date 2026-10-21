//! What changed between two DNS results of one domain, as facts.
//!
//! [`record_facts`] reduces a [`DnsReport`] to the parsed values of its
//! records ([`DnsRecords`], in `dmarc_assess`'s vocabulary), so a record
//! written with other whitespace, other case or its terms in another order is
//! the same record. [`diff_records`] compares two of those and says what
//! changed per check; [`diff_reports`] does both. Nothing here judges:
//! `dmarc_assess::alert_events` decides which change is worth an event and
//! its colour.
//!
//! A lookup that did not complete (timeout, SERVFAIL) is never a change: it
//! says nothing about the record, so the check is skipped for that pair.

use crate::checker::DnsReport;
use crate::dkim::parse_dkim_record;
use crate::spf::SpfTerm;
use dmarc_assess::{
    BimiRecordFacts, CheckKind, DkimKeyFacts, DkimSelectorFacts, DmarcRecordFacts, DnsRecords,
    DnsState, MtaStsPolicyFacts, MtaStsRecordFacts, Observed, RecordChange, RecordDelta, Severity,
    SpfRecordFacts, TlsRptRecordFacts,
};
use std::collections::{BTreeMap, HashMap};

/// What alerts keep of a complete stored result made by checker
/// `checker_version` (see [`crate::CHECKER_VERSION`]): the worst
/// finding per check and the records' facts.
pub fn dns_state(report: &DnsReport, checker_version: u32) -> DnsState {
    let mut worst: BTreeMap<CheckKind, Severity> = BTreeMap::new();
    for f in &report.findings {
        worst
            .entry(f.check)
            .and_modify(|s| *s = (*s).min(f.severity))
            .or_insert(f.severity);
    }
    DnsState {
        checker_version,
        worst,
        records: record_facts(report),
    }
}

/// The parsed values of every record in `report`.
pub fn record_facts(report: &DnsReport) -> DnsRecords {
    DnsRecords {
        dmarc: dmarc_facts(report),
        spf: spf_facts(report),
        dkim: dkim_facts(report),
        mta_sts: mta_sts_facts(report),
        tls_rpt: tls_rpt_facts(report),
        bimi: bimi_facts(report),
    }
}

/// What changed from `previous` to `current`, check by check (DMARC, SPF,
/// DKIM by selector, MTA-STS, TLS-RPT, BIMI).
pub fn diff_reports(previous: &DnsReport, current: &DnsReport) -> Vec<RecordChange> {
    diff_records(&record_facts(previous), &record_facts(current))
}

/// [`diff_reports`] over facts already taken (as alerts store them).
pub fn diff_records(previous: &DnsRecords, current: &DnsRecords) -> Vec<RecordChange> {
    let mut changes = Vec::new();
    let mut push = |check: CheckKind, change: Option<RecordDelta>| {
        if let Some(change) = change {
            changes.push(RecordChange { check, change });
        }
    };
    push(
        CheckKind::Dmarc,
        observed_change(&previous.dmarc, &current.dmarc, |a, b| {
            RecordDelta::DmarcChanged {
                before: a.clone(),
                after: b.clone(),
            }
        }),
    );
    push(
        CheckKind::Spf,
        observed_change(&previous.spf, &current.spf, |a, b| {
            let (added, removed) = term_changes(&a.terms, &b.terms);
            RecordDelta::SpfChanged {
                added,
                removed,
                lookups_before: a.lookups,
                lookups_after: b.lookups,
            }
        }),
    );
    for change in dkim_changes(&previous.dkim, &current.dkim) {
        push(CheckKind::Dkim, Some(change));
    }
    push(
        CheckKind::MtaSts,
        mta_sts_change(&previous.mta_sts, &current.mta_sts),
    );
    push(
        CheckKind::TlsRpt,
        observed_change(&previous.tls_rpt, &current.tls_rpt, |a, b| {
            RecordDelta::TlsRptChanged {
                rua_before: a.rua.clone(),
                rua_after: b.rua.clone(),
            }
        }),
    );
    push(
        CheckKind::Bimi,
        observed_change(&previous.bimi, &current.bimi, |a, b| {
            RecordDelta::BimiChanged {
                before: a.clone(),
                after: b.clone(),
            }
        }),
    );
    changes
}

/// Published, removed, or changed (worded by `changed`); nothing when either
/// side was not measured or nothing differs.
fn observed_change<T: PartialEq>(
    previous: &Observed<T>,
    current: &Observed<T>,
    changed: impl FnOnce(&T, &T) -> RecordDelta,
) -> Option<RecordDelta> {
    match (previous, current) {
        (Observed::Absent, Observed::Published(_)) => Some(RecordDelta::Published),
        (Observed::Published(_), Observed::Absent) => Some(RecordDelta::Removed),
        (Observed::Published(a), Observed::Published(b)) if a != b => Some(changed(a, b)),
        _ => None,
    }
}

/// Terms only in `after` (added) and only in `before` (removed), each sorted.
fn term_changes(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let count = |terms: &[String]| {
        let mut map: BTreeMap<String, i32> = BTreeMap::new();
        for t in terms {
            *map.entry(t.clone()).or_default() += 1;
        }
        map
    };
    let (b, a) = (count(before), count(after));
    let more = |x: &BTreeMap<String, i32>, y: &BTreeMap<String, i32>| -> Vec<String> {
        x.iter()
            .flat_map(|(term, &n)| {
                let extra = n - y.get(term).copied().unwrap_or(0);
                std::iter::repeat_n(term.clone(), extra.max(0) as usize)
            })
            .collect()
    };
    (more(&a, &b), more(&b, &a))
}

/// Selectors probed both times only: a selector that is newly probed (seen
/// in reports for the first time, or newly listed) is a baseline, and one no
/// longer probed has not changed.
fn dkim_changes(previous: &[DkimSelectorFacts], current: &[DkimSelectorFacts]) -> Vec<RecordDelta> {
    let before: HashMap<&str, &Observed<DkimKeyFacts>> = previous
        .iter()
        .map(|s| (s.selector.as_str(), &s.key))
        .collect();
    let mut changes = Vec::new();
    for s in current {
        let Some(old) = before.get(s.selector.as_str()) else {
            continue;
        };
        let selector = s.selector.clone();
        let change = match (*old, &s.key) {
            (Observed::Absent, Observed::Published(_)) => {
                Some(RecordDelta::DkimKeyAppeared { selector })
            }
            (Observed::Published(_), Observed::Absent) => {
                Some(RecordDelta::DkimKeyRemoved { selector })
            }
            (Observed::Published(a), Observed::Published(b)) if a != b => {
                if b.revoked && !a.revoked {
                    Some(RecordDelta::DkimKeyRevoked { selector })
                } else {
                    Some(RecordDelta::DkimKeyChanged { selector })
                }
            }
            _ => None,
        };
        changes.extend(change);
    }
    changes
}

/// The TXT `id=` always counts; the policy file only when it was read both
/// times (a fetch that failed is not a changed policy).
fn mta_sts_change(
    previous: &Observed<MtaStsRecordFacts>,
    current: &Observed<MtaStsRecordFacts>,
) -> Option<RecordDelta> {
    observed_change(previous, current, |a, b| RecordDelta::MtaStsChanged {
        before: a.clone(),
        after: b.clone(),
    })
    .filter(|delta| match delta {
        RecordDelta::MtaStsChanged { before, after } => {
            before.id != after.id
                || matches!((&before.policy, &after.policy), (Some(x), Some(y)) if x != y)
        }
        _ => true,
    })
}

/// Trimmed and lowercase; `None` when blank.
fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

fn sorted(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = values
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn dmarc_facts(report: &DnsReport) -> Observed<DmarcRecordFacts> {
    let dmarc = &report.dmarc;
    if !dmarc.lookup.is_measured() {
        return Observed::Unmeasured;
    }
    let Some(p) = &dmarc.parsed else {
        return Observed::Absent;
    };
    Observed::Published(DmarcRecordFacts {
        p: clean(p.p.as_deref()),
        sp: clean(p.sp.as_deref()),
        pct: p.pct.unwrap_or(100),
        rua: sorted(p.rua.iter().map(|u| u.raw.clone())),
        adkim: clean(p.adkim.as_deref()).unwrap_or_else(|| "r".into()),
        aspf: clean(p.aspf.as_deref()).unwrap_or_else(|| "r".into()),
        testing: p.testing(),
    })
}

/// A term as it is meant: qualifier (`+` left out, it is the default),
/// mechanism and value, lowercase.
fn term_text(term: &SpfTerm) -> String {
    let qualifier = term
        .qualifier
        .map(super::spf::Qualifier::as_str)
        .filter(|q| *q != "+")
        .unwrap_or("");
    let kind = match term.kind.as_str() {
        "unknown" => return term.raw.trim().to_ascii_lowercase(),
        k => k,
    };
    let sep = if term.qualifier.is_none() { "=" } else { ":" };
    match term.value.as_deref().map(str::trim) {
        None | Some("") => format!("{qualifier}{kind}"),
        Some(v) if v.starts_with('/') => format!("{qualifier}{kind}{}", v.to_ascii_lowercase()),
        Some(v) => format!("{qualifier}{kind}{sep}{}", v.to_ascii_lowercase()),
    }
}

fn spf_facts(report: &DnsReport) -> Observed<SpfRecordFacts> {
    let spf = &report.spf;
    if !spf.lookup.is_measured() || !spf.is_complete() {
        return Observed::Unmeasured;
    }
    let Some(parsed) = &spf.parsed else {
        return Observed::Absent;
    };
    let mut terms: Vec<String> = parsed.terms.iter().map(term_text).collect();
    terms.sort();
    Observed::Published(SpfRecordFacts {
        terms,
        lookups: spf.lookup_count,
    })
}

/// A short, stable fingerprint of a public key (FNV-1a, 64 bits): enough to
/// tell two keys apart, never used for anything that needs a secure hash.
fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn dkim_facts(report: &DnsReport) -> Vec<DkimSelectorFacts> {
    report
        .dkim
        .records
        .iter()
        .map(|row| {
            let key = if !row.lookup.is_measured() {
                Observed::Unmeasured
            } else if !row.present {
                Observed::Absent
            } else {
                let parsed = row.raw.as_deref().map(parse_dkim_record);
                let key = parsed
                    .and_then(|p| p.public_key)
                    .filter(|k| !k.is_empty())
                    .map(|k| fingerprint(&k))
                    .unwrap_or_default();
                Observed::Published(DkimKeyFacts {
                    key_id: if row.revoked { String::new() } else { key },
                    revoked: row.revoked,
                })
            };
            DkimSelectorFacts {
                selector: row.selector.trim().to_ascii_lowercase(),
                key,
            }
        })
        .collect()
}

fn mta_sts_facts(report: &DnsReport) -> Observed<MtaStsRecordFacts> {
    let m = &report.mta_sts;
    if !m.txt_lookup.is_measured() {
        return Observed::Unmeasured;
    }
    if m.txt_record.is_none() {
        return Observed::Absent;
    }
    Observed::Published(MtaStsRecordFacts {
        id: m.policy_id.as_deref().map(|s| s.trim().to_owned()),
        policy: m.policy.as_ref().map(|p| MtaStsPolicyFacts {
            mode: clean(p.mode.as_deref()),
            mx: sorted(p.mx.iter().cloned()),
        }),
    })
}

fn tls_rpt_facts(report: &DnsReport) -> Observed<TlsRptRecordFacts> {
    let t = &report.tls_rpt;
    if !t.lookup.is_measured() {
        return Observed::Unmeasured;
    }
    match &t.parsed {
        None => Observed::Absent,
        Some(p) => Observed::Published(TlsRptRecordFacts {
            rua: sorted(p.rua.iter().cloned()),
        }),
    }
}

fn bimi_facts(report: &DnsReport) -> Observed<BimiRecordFacts> {
    let b = &report.bimi;
    if !b.lookup.is_measured() {
        return Observed::Unmeasured;
    }
    match &b.parsed {
        None => Observed::Absent,
        Some(p) => Observed::Published(BimiRecordFacts {
            logo_url: p.logo_url.as_deref().map(|s| s.trim().to_owned()),
            authority_url: p.authority_url.as_deref().map(|s| s.trim().to_owned()),
            declined: p.declined,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::assemble_report;
    use crate::dkim::{evaluate_dkim, evaluate_dkim_selector, DkimProbe};
    use crate::dmarc::evaluate_dmarc;
    use crate::lookup::{TxtAnswer, TxtLookup};
    use crate::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
    use crate::spf::evaluate_spf;

    const DOMAIN: &str = "example.com";
    const KEY_A: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC1";
    const KEY_B: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC2";

    fn txt(records: &[&str]) -> TxtLookup {
        if records.is_empty() {
            TxtLookup::NoRecords
        } else {
            TxtLookup::Records(TxtAnswer::from_records(
                records.iter().map(|r| (*r).to_owned()).collect(),
            ))
        }
    }

    /// The hand-built pieces of one result; `None` means "not published".
    #[derive(Clone)]
    struct Spec {
        dmarc: Option<&'static str>,
        spf: Option<&'static str>,
        /// SPF include targets and their records.
        includes: Vec<(&'static str, &'static str)>,
        dkim: Vec<(&'static str, Option<String>)>,
        dkim_timeout: bool,
        mta_sts: Option<&'static str>,
        mta_sts_policy: Option<&'static str>,
        tls_rpt: Option<&'static str>,
        bimi: Option<&'static str>,
        bimi_timeout: bool,
    }

    fn spec() -> Spec {
        Spec {
            dmarc: Some("v=DMARC1; p=reject; rua=mailto:d@example.com"),
            spf: Some("v=spf1 ip4:192.0.2.0/24 include:_spf.example.net -all"),
            includes: vec![("_spf.example.net", "v=spf1 ip4:198.51.100.1 -all")],
            dkim: vec![("s1", Some(format!("v=DKIM1; k=rsa; p={KEY_A}")))],
            dkim_timeout: false,
            mta_sts: Some("v=STSv1; id=20240101"),
            mta_sts_policy: Some(
                "version: STSv1\nmode: enforce\nmx: mx.example.com\nmax_age: 86400\n",
            ),
            tls_rpt: Some("v=TLSRPTv1; rua=mailto:tls@example.com"),
            bimi: None,
            bimi_timeout: false,
        }
    }

    fn build(s: &Spec) -> DnsReport {
        let opt = |r: Option<&str>| r.map_or_else(|| txt(&[]), |r| txt(&[r]));
        let nested: Vec<(String, TxtLookup)> = s
            .includes
            .iter()
            .map(|(name, r)| ((*name).to_owned(), txt(&[r])))
            .collect();
        let probe = DkimProbe::new(
            vec![],
            &s.dkim
                .iter()
                .map(|(n, _)| (*n).to_owned())
                .collect::<Vec<_>>(),
        );
        let selectors: Vec<_> = s
            .dkim
            .iter()
            .map(|(name, record)| {
                let lookup = if s.dkim_timeout {
                    TxtLookup::Timeout
                } else {
                    match record {
                        Some(r) => txt(&[r]),
                        None => TxtLookup::NxDomain,
                    }
                };
                evaluate_dkim_selector(DOMAIN, name, &lookup)
            })
            .collect();
        let (policy_body, fetch) = match (s.mta_sts, s.mta_sts_policy) {
            (Some(_), Some(body)) => (Some(body), PolicyFetch::Ok { content_type: None }),
            (Some(_), None) => (None, PolicyFetch::ConnectFailed),
            _ => (None, PolicyFetch::NotAttempted),
        };
        let bimi = if s.bimi_timeout {
            TxtLookup::Timeout
        } else {
            opt(s.bimi)
        };
        assemble_report(
            DOMAIN,
            evaluate_dmarc(DOMAIN, &opt(s.dmarc), &[]),
            evaluate_spf(DOMAIN, &opt(s.spf), &nested),
            evaluate_dkim(DOMAIN, &selectors, &probe),
            evaluate_mta_sts(DOMAIN, &opt(s.mta_sts), policy_body, &fetch),
            evaluate_tls_rpt(DOMAIN, &opt(s.tls_rpt)),
            evaluate_bimi(DOMAIN, &bimi),
        )
    }

    fn diff(a: &Spec, b: &Spec) -> Vec<RecordChange> {
        diff_reports(&build(a), &build(b))
    }

    fn one(check: CheckKind, change: RecordDelta) -> Vec<RecordChange> {
        vec![RecordChange { check, change }]
    }

    #[test]
    fn the_same_records_written_differently_are_no_change() {
        let a = spec();
        assert!(diff(&a, &a).is_empty());
        let b = Spec {
            dmarc: Some("v=DMARC1;p=REJECT ;  rua=mailto:d@example.com; pct=100; adkim=r"),
            spf: Some("v=spf1  +include:_spf.example.net IP4:192.0.2.0/24 -all"),
            mta_sts: Some("v=STSv1;  id=20240101"),
            tls_rpt: Some("v=TLSRPTv1;rua=mailto:tls@example.com"),
            ..spec()
        };
        assert_eq!(diff(&a, &b), vec![]);
    }

    #[test]
    fn spf_terms_and_lookups() {
        let a = spec();
        let b = Spec {
            spf: Some(
                "v=spf1 ip4:192.0.2.0/24 include:_spf.example.net include:mail.example.org ~all",
            ),
            includes: vec![
                ("_spf.example.net", "v=spf1 ip4:198.51.100.1 -all"),
                ("mail.example.org", "v=spf1 ip4:203.0.113.1 -all"),
            ],
            ..spec()
        };
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 1);
        match &changes[0].change {
            RecordDelta::SpfChanged {
                added,
                removed,
                lookups_before,
                lookups_after,
            } => {
                assert_eq!(added, &["include:mail.example.org", "~all"]);
                assert_eq!(removed, &["-all"]);
                assert_eq!(*lookups_before, 1);
                assert_eq!(*lookups_after, 2);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(changes[0].check, CheckKind::Spf);
    }

    #[test]
    fn a_changed_include_changes_the_lookup_count_only() {
        let a = spec();
        let b = Spec {
            includes: vec![
                (
                    "_spf.example.net",
                    "v=spf1 include:a.example.net include:b.example.net -all",
                ),
                ("a.example.net", "v=spf1 ip4:198.51.100.2 -all"),
                ("b.example.net", "v=spf1 ip4:198.51.100.3 -all"),
            ],
            ..spec()
        };
        let changes = diff(&a, &b);
        assert_eq!(
            changes,
            one(
                CheckKind::Spf,
                RecordDelta::SpfChanged {
                    added: vec![],
                    removed: vec![],
                    lookups_before: 1,
                    lookups_after: 3
                }
            )
        );
    }

    #[test]
    fn records_appear_and_disappear() {
        let a = spec();
        let none = Spec {
            dmarc: None,
            spf: None,
            mta_sts: None,
            mta_sts_policy: None,
            tls_rpt: None,
            bimi: Some("v=BIMI1; l=https://example.com/logo.svg"),
            ..spec()
        };
        let checks = |changes: Vec<RecordChange>| -> Vec<(CheckKind, RecordDelta)> {
            changes.into_iter().map(|c| (c.check, c.change)).collect()
        };
        assert_eq!(
            checks(diff(&a, &none)),
            [
                (CheckKind::Dmarc, RecordDelta::Removed),
                (CheckKind::Spf, RecordDelta::Removed),
                (CheckKind::MtaSts, RecordDelta::Removed),
                (CheckKind::TlsRpt, RecordDelta::Removed),
                (CheckKind::Bimi, RecordDelta::Published),
            ]
        );
        assert_eq!(
            checks(diff(&none, &a)),
            [
                (CheckKind::Dmarc, RecordDelta::Published),
                (CheckKind::Spf, RecordDelta::Published),
                (CheckKind::MtaSts, RecordDelta::Published),
                (CheckKind::TlsRpt, RecordDelta::Published),
                (CheckKind::Bimi, RecordDelta::Removed),
            ]
        );
    }

    #[test]
    fn dmarc_tags_before_and_after() {
        let b = Spec {
            dmarc: Some("v=DMARC1; p=quarantine; pct=50; rua=mailto:x@example.org; aspf=s"),
            ..spec()
        };
        let changes = diff(&spec(), &b);
        let RecordDelta::DmarcChanged { before, after } = &changes[0].change else {
            panic!("unexpected {changes:?}");
        };
        assert_eq!(before.p.as_deref(), Some("reject"));
        assert_eq!((before.pct, before.aspf.as_str()), (100, "r"));
        assert_eq!(after.p.as_deref(), Some("quarantine"));
        assert_eq!((after.pct, after.aspf.as_str()), (50, "s"));
        assert_eq!(after.rua, ["mailto:x@example.org"]);
    }

    /// `t=y` is part of the facts; `t=n` written out is the default.
    #[test]
    fn testing_mode_before_and_after() {
        let testing = Spec {
            dmarc: Some("v=DMARC1; p=reject; t=y; rua=mailto:d@example.com"),
            ..spec()
        };
        let changes = diff(&spec(), &testing);
        let RecordDelta::DmarcChanged { before, after } = &changes[0].change else {
            panic!("unexpected {changes:?}");
        };
        assert_eq!((before.testing, after.testing), (false, true));
        assert_eq!((&before.p, before.pct), (&after.p, after.pct));
        let explicit_n = Spec {
            dmarc: Some("v=DMARC1; p=reject; t=n; rua=mailto:d@example.com"),
            ..spec()
        };
        assert!(diff(&spec(), &explicit_n).is_empty());
    }

    #[test]
    fn dkim_keys_change_revoke_appear_and_go() {
        let with = |records: Vec<(&'static str, Option<String>)>| Spec {
            dkim: records,
            ..spec()
        };
        let key = |k: &str| Some(format!("v=DKIM1; p={k}"));
        let a = with(vec![("s1", key(KEY_A)), ("s2", None), ("s3", key(KEY_A))]);
        let b = with(vec![
            ("s1", key(KEY_B)),
            ("s2", key(KEY_A)),
            ("s3", Some("v=DKIM1; p=".into())),
            ("new", key(KEY_A)),
        ]);
        let changes: Vec<RecordDelta> = diff(&a, &b).into_iter().map(|c| c.change).collect();
        assert_eq!(
            changes,
            [
                RecordDelta::DkimKeyChanged {
                    selector: "s1".into()
                },
                RecordDelta::DkimKeyAppeared {
                    selector: "s2".into()
                },
                RecordDelta::DkimKeyRevoked {
                    selector: "s3".into()
                },
            ]
        );
        let gone = with(vec![("s1", None)]);
        assert_eq!(
            diff(&a, &gone),
            one(
                CheckKind::Dkim,
                RecordDelta::DkimKeyRemoved {
                    selector: "s1".into()
                }
            )
        );
        // Reformatted keys (whitespace inside p=) are the same key.
        let spaced = with(vec![(
            "s1",
            Some(format!("v=DKIM1; p={} {}", &KEY_A[..10], &KEY_A[10..])),
        )]);
        assert!(diff(&with(vec![("s1", key(KEY_A))]), &spaced).is_empty());
    }

    #[test]
    fn failed_lookups_are_never_changes() {
        let a = spec();
        let timed_out = Spec {
            dkim_timeout: true,
            bimi_timeout: true,
            ..spec()
        };
        assert!(diff(&a, &timed_out).is_empty());
        assert!(diff(&timed_out, &a).is_empty());
        // A policy file that could not be read is not a changed policy.
        let unread = Spec {
            mta_sts_policy: None,
            ..spec()
        };
        assert!(diff(&a, &unread).is_empty());
    }

    #[test]
    fn mta_sts_tls_rpt_and_bimi_changes() {
        let b = Spec {
            mta_sts: Some("v=STSv1; id=20250101"),
            mta_sts_policy: Some(
                "version: STSv1\nmode: testing\nmx: mx.example.com\nmax_age: 86400\n",
            ),
            tls_rpt: Some("v=TLSRPTv1; rua=mailto:other@example.com"),
            ..spec()
        };
        let changes = diff(&spec(), &b);
        assert_eq!(changes.len(), 2);
        let RecordDelta::MtaStsChanged { before, after } = &changes[0].change else {
            panic!("unexpected {changes:?}");
        };
        assert_eq!(before.id.as_deref(), Some("20240101"));
        assert_eq!(
            after.policy.as_ref().unwrap().mode.as_deref(),
            Some("testing")
        );
        assert_eq!(
            changes[1].change,
            RecordDelta::TlsRptChanged {
                rua_before: vec!["mailto:tls@example.com".into()],
                rua_after: vec!["mailto:other@example.com".into()],
            }
        );
        let logo = |l: &'static str| Spec {
            bimi: Some(l),
            ..spec()
        };
        let bimi = diff(
            &logo("v=BIMI1; l=https://example.com/a.svg"),
            &logo("v=BIMI1; l=https://example.com/b.svg"),
        );
        assert_eq!(bimi.len(), 1);
        assert!(matches!(bimi[0].change, RecordDelta::BimiChanged { .. }));
    }

    #[test]
    fn dns_state_keeps_the_worst_finding_per_check() {
        let report = build(&Spec {
            dmarc: Some("v=DMARC1; p=none"),
            ..spec()
        });
        let state = dns_state(&report, 7);
        assert_eq!(state.checker_version, 7);
        let dmarc = state.worst[&CheckKind::Dmarc];
        assert!(dmarc <= Severity::Warning, "{dmarc:?}");
        assert_eq!(state.records, record_facts(&report));
    }
}
