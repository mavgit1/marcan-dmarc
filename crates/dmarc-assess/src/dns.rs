//! Judgement of a domain's DNS posture: the finding vocabulary, the score and
//! grade, and the verdict on each record. `dmarc-dns` measures (looks up and
//! parses records) and reports findings in this vocabulary, choosing each
//! finding's severity on the scale below; the scale itself, the thresholds,
//! the scoring and the verdicts live here.
//!
//! Severity reflects impact on authentication, deliverability, or security:
//!
//! * `Critical` — authentication is broken or trivially bypassable.
//! * `Warning` — a real weakening a receiver can act on today.
//! * `Info` — a gap worth fixing that breaks nothing on its own. Deducts a little.
//! * `Notice` — no score impact: a cosmetic gap (e.g. BIMI) or a lookup that
//!   could not complete. Not a pass.
//! * `Ok` — correct configuration. Never deducts.
//!
//! A finding whose text praises the configuration must be `Ok`. Incomplete core
//! lookups (DMARC/SPF timeout or SERVFAIL) are `Notice`, and callers should not
//! publish a grade from them.
//!
//! `Severity` derives `Ord` worst-first so findings sort by it; [`Status`]
//! compares through [`Status::worst`] instead.

use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use ts_rs::TS;

/// Which authentication / policy check produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Dmarc,
    Spf,
    Dkim,
    MtaSts,
    TlsRpt,
    Bimi,
}

impl CheckKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dmarc => "dmarc",
            Self::Spf => "spf",
            Self::Dkim => "dkim",
            Self::MtaSts => "mta_sts",
            Self::TlsRpt => "tls_rpt",
            Self::Bimi => "bimi",
        }
    }
}

/// Finding severity. Ordered so `sort` places the worst issues first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Warning,
    Info,
    /// Zero deduction; not a pass. Cosmetic gaps and incomplete lookups.
    Notice,
    Ok,
}

impl Severity {
    /// Every severity, worst first.
    pub const ALL: [Self; 5] = [
        Self::Critical,
        Self::Warning,
        Self::Info,
        Self::Notice,
        Self::Ok,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Notice => "notice",
            Self::Ok => "ok",
        }
    }
}

/// One diagnostic: what is wrong (or right), why it matters, and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Finding {
    pub check: CheckKind,
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `spf_lookup_limit`.
    pub code: String,
    pub message: String,
    pub suggestion: String,
    /// Score points this finding actually costs. A code is charged once, so
    /// only its worst instance carries the deduction; repeats carry 0. Set by
    /// [`assign_points`].
    pub points: u8,
}

impl Finding {
    pub fn new(
        check: CheckKind,
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            check,
            severity,
            code: code.into(),
            message: message.into(),
            suggestion: suggestion.into(),
            points: 0,
        }
    }

    pub fn critical(
        check: CheckKind,
        code: &str,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(check, Severity::Critical, code, message, suggestion)
    }

    pub fn warning(
        check: CheckKind,
        code: &str,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(check, Severity::Warning, code, message, suggestion)
    }

    pub fn info(
        check: CheckKind,
        code: &str,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(check, Severity::Info, code, message, suggestion)
    }

    pub fn notice(
        check: CheckKind,
        code: &str,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(check, Severity::Notice, code, message, suggestion)
    }

    pub fn ok(
        check: CheckKind,
        code: &str,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self::new(check, Severity::Ok, code, message, suggestion)
    }
}

/// Sort findings by severity (critical first), then check kind, then code.
pub fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(a.check.cmp(&b.check))
            .then(a.code.cmp(&b.code))
    });
}

/// True when nothing in `findings` is worse than `Info`: only `Ok`, `Info`
/// and `Notice`. Checks use it to decide whether to add their summary `Ok`.
pub fn no_issue_above_info(findings: &[Finding]) -> bool {
    findings.iter().all(|f| f.severity >= Severity::Info)
}

/// Lowest score of each letter grade, best first; anything lower is `F`.
pub const GRADE_BANDS: [(&str, u8); 4] = [("A", 90), ("B", 80), ("C", 70), ("D", 60)];

/// Letter grade derived from a 0–100 numeric score.
pub fn grade_from_score(score: u8) -> &'static str {
    GRADE_BANDS
        .iter()
        .find(|(_, min)| score >= *min)
        .map_or("F", |(grade, _)| grade)
}

/// Score points one finding of `severity` deducts (each code is charged once).
pub fn severity_points(severity: Severity) -> u8 {
    match severity {
        Severity::Critical => 28,
        Severity::Warning => 8,
        Severity::Info => 2,
        Severity::Notice | Severity::Ok => 0,
    }
}

/// Set each finding's [`Finding::points`] to the deduction it is charged by
/// [`score_from_findings`]: the worst instance of each code (the first one
/// among equally severe instances) carries the points, every other instance 0.
/// Before clamping, the points add up to `100 - score`.
pub fn assign_points(findings: &mut [Finding]) {
    let mut charged: HashMap<String, usize> = HashMap::new();
    for (i, f) in findings.iter().enumerate() {
        match charged.get(&f.code) {
            Some(&j) if findings[j].severity <= f.severity => {}
            _ => {
                charged.insert(f.code.clone(), i);
            }
        }
    }
    let charged: HashSet<usize> = charged.into_values().collect();
    for (i, f) in findings.iter_mut().enumerate() {
        f.points = if charged.contains(&i) {
            severity_points(f.severity)
        } else {
            0
        };
    }
}

/// Deduct from 100, once per distinct finding `code`, then clamp to 0–100.
///
/// Several checks legitimately emit one finding per occurrence: `dkim_key_dated`
/// fires for every weak selector, `dmarc_unknown_tag` for every stray tag,
/// `spf_syntax` for every parse error. Those repeats are worth showing — the
/// operator needs to know which four selectors to rotate — but they are one
/// conceptual problem with one fix, so they cost one deduction.
///
/// When a code appears at more than one severity, the worst one wins.
pub fn score_from_findings(findings: &[Finding]) -> u8 {
    let mut worst: HashMap<&str, Severity> = HashMap::new();
    for f in findings {
        worst
            .entry(f.code.as_str())
            .and_modify(|s| {
                if f.severity < *s {
                    *s = f.severity;
                }
            })
            .or_insert(f.severity);
    }
    let total: i32 = worst.values().map(|s| i32::from(severity_points(*s))).sum();
    (100 - total).clamp(0, 100) as u8
}

/// The check behind the worst finding: most severe first, then the largest
/// deduction, then the first listed. DMARC when there are no findings.
pub fn worst_check(findings: &[Finding]) -> CheckKind {
    let mut worst: Option<&Finding> = None;
    for f in findings {
        let worse = worst.is_none_or(|w| {
            f.severity < w.severity || (f.severity == w.severity && f.points > w.points)
        });
        if worse {
            worst = Some(f);
        }
    }
    worst.map_or(CheckKind::Dmarc, |f| f.check)
}

/// Whether a domain's shown DNS result is current.
///
/// - `Ok`: the shown result is complete and the latest attempt succeeded.
/// - `Updating`: the shown result comes from an older checker version; a
///   re-check is running.
/// - `Stale`: the latest attempt failed or was incomplete, so the last
///   complete result is shown and a retry is scheduled.
/// - `Failed`: no complete result exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DnsStatus {
    Ok,
    Updating,
    Stale,
    Failed,
}

/// The status of a shown result (`has_result`), given whether this process's
/// latest attempt at the domain failed and whether the result is from an
/// older checker.
pub fn dns_status(has_result: bool, latest_attempt_failed: bool, outdated: bool) -> DnsStatus {
    if !has_result {
        DnsStatus::Failed
    } else if latest_attempt_failed {
        DnsStatus::Stale
    } else if outdated {
        DnsStatus::Updating
    } else {
        DnsStatus::Ok
    }
}

/// How many of `findings` are critical: what [`dns_grade_status`] and the
/// stored snapshot's `critical_count` take.
pub fn critical_count(findings: &[Finding]) -> i64 {
    findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count() as i64
}

/// [`crate::grade_status`] of a result's grade, but neutral unless DMARC and
/// SPF were measured completely: a letter without a measurement behind it is
/// not a verdict.
pub fn dns_grade_status(core_measured: bool, grade: &str, critical_count: i64) -> Status {
    if !core_measured {
        return Status::Neutral;
    }
    crate::grade_status(Some(grade), critical_count)
}

// ---- DKIM keys and selectors -------------------------------------------------

/// RSA keys below this many bits are weak (critical).
pub const RSA_MIN_BITS: usize = 1024;
/// RSA keys below this many bits (and at least [`RSA_MIN_BITS`]) are dated (warning).
pub const RSA_GOOD_BITS: usize = 2048;

/// Strength bucket for RSA (and similar) keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum KeyStrength {
    Weak,
    Dated,
    Good,
    Unknown,
}

impl KeyStrength {
    /// An RSA key of `bits` bits, judged by [`RSA_MIN_BITS`] and [`RSA_GOOD_BITS`].
    pub fn of_rsa_bits(bits: usize) -> Self {
        if bits < RSA_MIN_BITS {
            Self::Weak
        } else if bits < RSA_GOOD_BITS {
            Self::Dated
        } else {
            Self::Good
        }
    }
}

/// A key's strength as a verdict: weak fails, dated warns, good passes.
pub fn key_status(strength: Option<KeyStrength>) -> Status {
    match strength {
        Some(KeyStrength::Weak) => Status::Fail,
        Some(KeyStrength::Dated) => Status::Warn,
        Some(KeyStrength::Good) => Status::Pass,
        Some(KeyStrength::Unknown) | None => Status::Neutral,
    }
}

/// What one probed DKIM selector (or the domain's selectors as a whole) says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SelectorState {
    Published,
    #[default]
    NotPublished,
    Revoked,
    WeakKey,
    /// `t=y`: receivers may treat signatures as informational only.
    TestMode,
    /// 1024-bit RSA.
    DatedKey,
    /// The lookup timed out or SERVFAILed: nothing was measured.
    CheckFailed,
    /// No selector was probed at all.
    NoSelectors,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SelectorVerdict {
    pub state: SelectorState,
    pub status: Status,
}

impl SelectorVerdict {
    const fn new(state: SelectorState, status: Status) -> Self {
        Self { state, status }
    }
}

/// What was measured about one DKIM selector.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectorFacts {
    /// The lookup completed (an answer or a confirmed absence).
    pub measured: bool,
    /// A key record is published.
    pub present: bool,
    /// Aggregate reports show mail signed with this selector.
    pub seen_in_reports: bool,
    /// Published with an empty `p=`.
    pub revoked: bool,
    pub key_strength: Option<KeyStrength>,
    /// `t=y`.
    pub test_mode: bool,
}

/// One selector. A missing key only fails when reports show mail signed with it;
/// a published key is judged by revocation, strength and test mode.
pub fn selector_verdict(facts: &SelectorFacts) -> SelectorVerdict {
    use SelectorState::*;
    if !facts.measured {
        return SelectorVerdict::new(CheckFailed, Status::Neutral);
    }
    if !facts.present {
        let status = if facts.seen_in_reports {
            Status::Fail
        } else {
            Status::Neutral
        };
        return SelectorVerdict::new(NotPublished, status);
    }
    if facts.revoked {
        return SelectorVerdict::new(Revoked, Status::Fail);
    }
    match facts.key_strength {
        Some(KeyStrength::Weak) => SelectorVerdict::new(WeakKey, Status::Fail),
        _ if facts.test_mode => SelectorVerdict::new(TestMode, Status::Warn),
        Some(KeyStrength::Dated) => SelectorVerdict::new(DatedKey, Status::Warn),
        _ => SelectorVerdict::new(Published, Status::Pass),
    }
}

/// The domain's selectors as one verdict: the worst one that says something.
/// A selector without a key that nothing signs with is inventory (neutral),
/// never the verdict while another selector is published.
pub fn dkim_summary(verdicts: impl IntoIterator<Item = SelectorVerdict>) -> SelectorVerdict {
    let verdicts: Vec<SelectorVerdict> = verdicts.into_iter().collect();
    if verdicts.is_empty() {
        return SelectorVerdict::new(SelectorState::NoSelectors, Status::Neutral);
    }
    let candidates = verdicts
        .iter()
        .find(|v| v.status == Status::Pass)
        .into_iter()
        .chain(verdicts.iter().filter(|v| v.status != Status::Neutral))
        .chain(
            verdicts
                .iter()
                .find(|v| v.state == SelectorState::CheckFailed),
        )
        .copied()
        .chain([SelectorVerdict::new(
            SelectorState::NotPublished,
            Status::Neutral,
        )]);
    candidates
        .reduce(|best, v| {
            if best.status.worst(v.status) == best.status {
                best
            } else {
                v
            }
        })
        .expect("at least the fallback candidate")
}

// ---- DMARC ---------------------------------------------------------------------

/// RFC 7489 §7.1 authorization of one external report destination.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Authorized,
    #[default]
    NotAuthorized,
    /// The lookup timed out or SERVFAILed: not the same as unauthorized.
    CheckFailed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuthVerdict {
    pub state: AuthState,
    pub status: Status,
}

pub fn authorization_verdict(authorized: bool, lookup_measured: bool) -> AuthVerdict {
    let (state, status) = if authorized {
        (AuthState::Authorized, Status::Pass)
    } else if !lookup_measured {
        (AuthState::CheckFailed, Status::Neutral)
    } else {
        (AuthState::NotAuthorized, Status::Fail)
    };
    AuthVerdict { state, status }
}

/// All external destinations as one verdict, worst first: any unauthorized,
/// else any failed check, else authorized. `None` without external ones.
pub fn authorization_summary(
    verdicts: impl IntoIterator<Item = AuthVerdict>,
) -> Option<AuthVerdict> {
    let verdicts: Vec<AuthVerdict> = verdicts.into_iter().collect();
    let worst = |state| verdicts.iter().copied().find(|v| v.state == state);
    worst(AuthState::NotAuthorized)
        .or_else(|| worst(AuthState::CheckFailed))
        .or_else(|| verdicts.first().copied())
}

/// DMARC is required: an absent record fails; a lookup that did not
/// complete states nothing.
pub fn dmarc_record_status(published: bool, lookup_measured: bool) -> Status {
    if published {
        Status::Pass
    } else if lookup_measured {
        Status::Fail
    } else {
        Status::Neutral
    }
}

/// `pct=` below 100 leaves some failing mail outside the policy: worth fixing.
pub fn pct_status(pct: Option<u32>) -> Status {
    if pct.is_some_and(|n| n < 100) {
        Status::Warn
    } else {
        Status::Neutral
    }
}

// ---- SPF, MTA-STS ----------------------------------------------------------------

/// What the effective SPF `all` (the qualifier character) does with
/// unlisted senders: `-all` passes, `+all` fails, anything else warns.
pub fn spf_all_status(all_qualifier: Option<&str>) -> Status {
    match all_qualifier {
        Some("-") => Status::Pass,
        Some("+") => Status::Fail,
        // `~all`, `?all` and no `all` at all leave unlisted senders unfailed;
        // the same three cases `dmarc-dns` reports as warnings.
        _ => Status::Warn,
    }
}

/// RFC 7208 §4.6.4: SPF evaluation may cause at most this many DNS lookups;
/// past it receivers return permerror.
pub const SPF_LOOKUP_LIMIT: u32 = 10;
/// From this many lookups on, an SPF record is close enough to
/// [`SPF_LOOKUP_LIMIT`] that one more include can break it (warning).
pub const SPF_LOOKUP_WARN_AT: u32 = 8;
/// RFC 7208 §4.6.4: more void lookups (NXDOMAIN or no data) than this may be
/// treated as permerror (warning).
pub const SPF_VOID_LIMIT: u32 = 2;

/// Where an SPF record's measured DNS lookup count sits on the scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpfLookupLoad {
    /// Below [`SPF_LOOKUP_WARN_AT`].
    Comfortable,
    /// From [`SPF_LOOKUP_WARN_AT`] up to [`SPF_LOOKUP_LIMIT`]: valid, with
    /// little room left (warning).
    NearLimit,
    /// Over [`SPF_LOOKUP_LIMIT`]: receivers return permerror (critical).
    OverLimit,
}

impl SpfLookupLoad {
    /// The load of an SPF evaluation that took `lookups` DNS lookups.
    pub fn of_count(lookups: u32) -> Self {
        if lookups > SPF_LOOKUP_LIMIT {
            Self::OverLimit
        } else if lookups >= SPF_LOOKUP_WARN_AT {
            Self::NearLimit
        } else {
            Self::Comfortable
        }
    }
}

/// Whether `void_lookups` exceeds [`SPF_VOID_LIMIT`].
pub fn spf_void_lookups_exceeded(void_lookups: u32) -> bool {
    void_lookups > SPF_VOID_LIMIT
}

/// MTA-STS `max_age` below this many seconds (one day) lets senders forget
/// the policy quickly (info).
pub const MTA_STS_MIN_MAX_AGE: u64 = 86_400;
/// The `max_age` a stable MTA-STS policy is advised to use (one week).
pub const MTA_STS_GOOD_MAX_AGE: u64 = 604_800;

/// Whether an MTA-STS policy's `max_age` (absent counts as zero) is below
/// [`MTA_STS_MIN_MAX_AGE`].
pub fn mta_sts_max_age_short(max_age: Option<u64>) -> bool {
    max_age.unwrap_or(0) < MTA_STS_MIN_MAX_AGE
}

/// MTA-STS `mode=`: `enforce` passes, `testing` warns, `none` is neutral.
pub fn mta_sts_mode_status(mode: &str) -> Status {
    match mode.trim().to_ascii_lowercase().as_str() {
        "enforce" => Status::Pass,
        "testing" => Status::Warn,
        _ => Status::Neutral,
    }
}

/// Reaching the MTA-STS policy file: a host that answered wrongly or not at
/// all (`unreachable`) warns, because receivers cannot apply the policy; not
/// fetched, read, or our own timeout states nothing.
pub fn mta_sts_fetch_status(unreachable: bool) -> Status {
    if unreachable {
        Status::Warn
    } else {
        Status::Neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(severity: Severity, code: &str) -> Finding {
        Finding::new(CheckKind::Dkim, severity, code, "msg", "fix")
    }

    #[test]
    fn points_charge_only_the_worst_instance_of_a_code() {
        let mut findings = vec![
            f(Severity::Warning, "dkim_key_dated"),
            f(Severity::Warning, "dkim_key_dated"),
            f(Severity::Notice, "dkim_observed_selector_missing"),
            f(Severity::Warning, "dkim_observed_selector_missing"),
            f(Severity::Info, "tls_rpt_missing"),
            f(Severity::Ok, "spf_ok"),
        ];
        assign_points(&mut findings);
        let points: Vec<u8> = findings.iter().map(|f| f.points).collect();
        assert_eq!(points, vec![8, 0, 0, 8, 2, 0]);
        let total: i32 = findings.iter().map(|f| i32::from(f.points)).sum();
        assert_eq!(100 - total, i32::from(score_from_findings(&findings)));
    }

    #[test]
    fn ok_and_notice_findings_never_deduct() {
        let findings = vec![
            f(Severity::Ok, "spf_ok"),
            f(Severity::Ok, "dmarc_ok"),
            f(Severity::Ok, "spf_fail_all"),
            f(Severity::Notice, "bimi_missing"),
            f(Severity::Notice, "bimi_lookup_failed"),
        ];
        assert_eq!(score_from_findings(&findings), 100);
        assert_eq!(grade_from_score(score_from_findings(&findings)), "A");
    }

    #[test]
    fn repeated_code_is_charged_once() {
        // Four selectors, one 1024-bit-key problem.
        let repeated: Vec<Finding> = ["k1", "mail", "selector1", "selector2"]
            .iter()
            .map(|_| f(Severity::Warning, "dkim_key_dated"))
            .collect();
        assert_eq!(score_from_findings(&repeated), 92);
        assert_eq!(score_from_findings(&repeated[..1]), 92);
    }

    #[test]
    fn distinct_codes_still_stack() {
        let findings = vec![
            f(Severity::Warning, "dkim_key_dated"),
            f(Severity::Warning, "dkim_key_dated"),
            f(Severity::Warning, "spf_softfail_all"),
            f(Severity::Notice, "bimi_missing"),
            f(Severity::Info, "tls_rpt_missing"),
        ];
        assert_eq!(score_from_findings(&findings), 100 - 8 - 8 - 2);
    }

    #[test]
    fn worst_severity_wins_for_a_shared_code() {
        let findings = vec![
            f(Severity::Notice, "dkim_observed_selector_missing"),
            f(Severity::Warning, "dkim_observed_selector_missing"),
        ];
        assert_eq!(score_from_findings(&findings), 92);
    }

    #[test]
    fn optional_lookup_failures_do_not_move_score() {
        // Same DNS evidence, plus our own incomplete optional lookups: score
        // must not depend on whether those lookups happened to time out.
        let base = vec![
            f(Severity::Warning, "spf_softfail_all"),
            f(Severity::Info, "mta_sts_missing"),
            f(Severity::Info, "tls_rpt_missing"),
        ];
        let with_failures = {
            let mut v = base.clone();
            v.push(f(Severity::Notice, "mta_sts_txt_timeout"));
            v.push(f(Severity::Notice, "tls_rpt_lookup_failed"));
            v.push(f(Severity::Notice, "bimi_lookup_failed"));
            v
        };
        let expected = 100 - 8 - 2 - 2;
        assert_eq!(score_from_findings(&base), expected);
        assert_eq!(score_from_findings(&with_failures), expected);
    }

    #[test]
    fn no_issue_above_info_ignores_ok_info_and_notice() {
        let benign = vec![
            f(Severity::Ok, "a"),
            f(Severity::Info, "b"),
            f(Severity::Notice, "c"),
        ];
        assert!(no_issue_above_info(&benign));
        assert!(no_issue_above_info(&[]));
        for bad in [Severity::Warning, Severity::Critical] {
            let mut v = benign.clone();
            v.push(f(bad, "d"));
            assert!(!no_issue_above_info(&v));
        }
    }

    #[test]
    fn notice_sorts_between_info_and_ok() {
        assert!(Severity::Info < Severity::Notice);
        assert!(Severity::Notice < Severity::Ok);
    }

    #[test]
    fn score_floors_at_zero() {
        let findings: Vec<Finding> = (0..10)
            .map(|i| f(Severity::Critical, &format!("code_{i}")))
            .collect();
        assert_eq!(score_from_findings(&findings), 0);
    }

    #[test]
    fn worst_check_ranks_severity_then_points() {
        let f = |check, severity, points| Finding {
            points,
            ..Finding::new(check, severity, "c", "m", "s")
        };
        assert_eq!(worst_check(&[]), CheckKind::Dmarc);
        assert_eq!(
            worst_check(&[
                f(CheckKind::Spf, Severity::Warning, 10),
                f(CheckKind::Dkim, Severity::Critical, 5),
                f(CheckKind::MtaSts, Severity::Critical, 5),
            ]),
            CheckKind::Dkim
        );
        assert_eq!(
            worst_check(&[
                f(CheckKind::Spf, Severity::Warning, 3),
                f(CheckKind::TlsRpt, Severity::Warning, 8),
            ]),
            CheckKind::TlsRpt
        );
        assert_eq!(
            worst_check(&[f(CheckKind::Bimi, Severity::Ok, 0)]),
            CheckKind::Bimi
        );
    }

    fn published(key: KeyStrength) -> SelectorFacts {
        SelectorFacts {
            measured: true,
            present: true,
            key_strength: Some(key),
            ..SelectorFacts::default()
        }
    }

    fn absent() -> SelectorFacts {
        SelectorFacts {
            measured: true,
            ..SelectorFacts::default()
        }
    }

    #[test]
    fn selector_states() {
        assert_eq!(
            selector_verdict(&published(KeyStrength::Good)).state,
            SelectorState::Published
        );
        let missing_but_signing = SelectorFacts {
            seen_in_reports: true,
            ..absent()
        };
        assert_eq!(selector_verdict(&missing_but_signing).status, Status::Fail);
        assert_eq!(selector_verdict(&absent()).status, Status::Neutral);
        let dated_test = SelectorFacts {
            test_mode: true,
            ..published(KeyStrength::Dated)
        };
        assert_eq!(selector_verdict(&dated_test).state, SelectorState::TestMode);
        let revoked = SelectorFacts {
            revoked: true,
            ..published(KeyStrength::Good)
        };
        assert_eq!(selector_verdict(&revoked).state, SelectorState::Revoked);
        assert_eq!(
            selector_verdict(&SelectorFacts::default()).state,
            SelectorState::CheckFailed
        );
    }

    #[test]
    fn summary_prefers_what_says_something() {
        assert_eq!(dkim_summary([]).state, SelectorState::NoSelectors);
        // A published key beats an unused, absent one.
        let s = dkim_summary([
            selector_verdict(&absent()),
            selector_verdict(&published(KeyStrength::Good)),
        ]);
        assert_eq!(s.state, SelectorState::Published);
        // A missing key that reports show signing beats a published one.
        let signing = SelectorFacts {
            seen_in_reports: true,
            ..absent()
        };
        let s = dkim_summary([
            selector_verdict(&published(KeyStrength::Good)),
            selector_verdict(&signing),
        ]);
        assert_eq!(
            (s.state, s.status),
            (SelectorState::NotPublished, Status::Fail)
        );
        assert_eq!(
            dkim_summary([selector_verdict(&absent())]).status,
            Status::Neutral
        );
    }

    #[test]
    fn key_strength_and_statuses() {
        assert_eq!(KeyStrength::of_rsa_bits(512), KeyStrength::Weak);
        assert_eq!(KeyStrength::of_rsa_bits(1024), KeyStrength::Dated);
        assert_eq!(KeyStrength::of_rsa_bits(2048), KeyStrength::Good);
        assert_eq!(key_status(Some(KeyStrength::Weak)), Status::Fail);
        assert_eq!(key_status(Some(KeyStrength::Dated)), Status::Warn);
        assert_eq!(key_status(Some(KeyStrength::Good)), Status::Pass);
        assert_eq!(key_status(Some(KeyStrength::Unknown)), Status::Neutral);
        assert_eq!(key_status(None), Status::Neutral);
        assert_eq!(dmarc_record_status(true, true), Status::Pass);
        assert_eq!(dmarc_record_status(false, true), Status::Fail);
        assert_eq!(dmarc_record_status(false, false), Status::Neutral);
        assert_eq!(pct_status(Some(50)), Status::Warn);
        assert_eq!(pct_status(Some(100)), Status::Neutral);
        assert_eq!(pct_status(None), Status::Neutral);
    }

    #[test]
    fn spf_all_and_mta_sts() {
        assert_eq!(spf_all_status(Some("-")), Status::Pass);
        assert_eq!(spf_all_status(Some("~")), Status::Warn);
        assert_eq!(spf_all_status(Some("+")), Status::Fail);
        assert_eq!(spf_all_status(Some("?")), Status::Warn);
        assert_eq!(spf_all_status(None), Status::Warn);
        assert_eq!(mta_sts_mode_status("enforce"), Status::Pass);
        assert_eq!(mta_sts_mode_status("testing"), Status::Warn);
        assert_eq!(mta_sts_mode_status("none"), Status::Neutral);
        assert_eq!(mta_sts_fetch_status(true), Status::Warn);
        assert_eq!(mta_sts_fetch_status(false), Status::Neutral);
    }

    #[test]
    fn spf_lookup_load_and_void_limit() {
        assert_eq!(SpfLookupLoad::of_count(0), SpfLookupLoad::Comfortable);
        assert_eq!(SpfLookupLoad::of_count(7), SpfLookupLoad::Comfortable);
        assert_eq!(SpfLookupLoad::of_count(8), SpfLookupLoad::NearLimit);
        assert_eq!(SpfLookupLoad::of_count(10), SpfLookupLoad::NearLimit);
        assert_eq!(SpfLookupLoad::of_count(11), SpfLookupLoad::OverLimit);
        assert!(!spf_void_lookups_exceeded(2));
        assert!(spf_void_lookups_exceeded(3));
    }

    #[test]
    fn mta_sts_max_age_floor() {
        assert!(mta_sts_max_age_short(None));
        assert!(mta_sts_max_age_short(Some(86_399)));
        assert!(!mta_sts_max_age_short(Some(86_400)));
        assert!(!mta_sts_max_age_short(Some(MTA_STS_GOOD_MAX_AGE)));
    }

    #[test]
    fn dns_status_and_grade() {
        assert_eq!(dns_status(false, false, false), DnsStatus::Failed);
        assert_eq!(dns_status(false, true, true), DnsStatus::Failed);
        assert_eq!(dns_status(true, true, true), DnsStatus::Stale);
        assert_eq!(dns_status(true, false, true), DnsStatus::Updating);
        assert_eq!(dns_status(true, false, false), DnsStatus::Ok);
        assert_eq!(dns_grade_status(false, "A", 0), Status::Neutral);
        assert_eq!(dns_grade_status(true, "A", 0), Status::Pass);
        assert_eq!(dns_grade_status(true, "A", 1), Status::Fail);
    }

    #[test]
    fn authorization() {
        assert_eq!(
            authorization_verdict(true, true).state,
            AuthState::Authorized
        );
        assert_eq!(
            authorization_verdict(false, false).state,
            AuthState::CheckFailed
        );
        assert_eq!(authorization_verdict(false, true).status, Status::Fail);
        assert_eq!(authorization_summary([]), None);
        let ok = authorization_verdict(true, true);
        let failed = authorization_verdict(false, false);
        let denied = authorization_verdict(false, true);
        assert_eq!(authorization_summary([ok, failed]), Some(failed));
        assert_eq!(authorization_summary([ok, failed, denied]), Some(denied));
        assert_eq!(authorization_summary([ok]), Some(ok));
    }
}
