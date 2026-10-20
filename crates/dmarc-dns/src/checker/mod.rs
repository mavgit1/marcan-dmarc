//! Live DNS checks with timeouts and a short in-memory TTL cache.

use crate::dkim::{
    evaluate_dkim, evaluate_dkim_selector, DkimCheckResult, DkimProbe, DkimSelectorResult,
};
use crate::dmarc::{
    evaluate_dmarc, evaluate_report_authorization, parse_dmarc_record, report_auth_name,
    DmarcCheckResult, ReportAuthorization,
};
use crate::dns::DnsClient;
use crate::lookup::{LookupStatus, TxtLookup};
use crate::policy::{
    evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, BimiCheckResult, MtaStsCheckResult,
    PolicyFetch, TlsRptCheckResult,
};
use crate::resolver::{HttpResolve, Resolver};
use crate::spf::{evaluate_spf, parse_spf_record, Mechanism, SpfCheckResult, SPF_MAX_WALK};
use crate::{DnsError, ForwardLookup, PtrLookup};
use chrono::{DateTime, Utc};
use dmarc_assess::{
    assign_points, grade_from_score, score_from_findings, sort_findings, CheckKind, Finding,
};
use dmarc_domain::{normalize_domain, normalize_name, report_authorization_needed};
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::{timeout, timeout_at, Instant};
use ts_rs::TS;

mod cache;
mod policy_fetch;

use cache::{CacheKey, Cached};
use policy_fetch::http_client;

const CACHE_TTL: Duration = Duration::from_secs(300);
const DKIM_CONCURRENCY: usize = 8;
/// SPF include/redirect targets looked up at once. Small on purpose: a big
/// tree (Microsoft, Google, an ESP or two) must not burst dozens of queries at
/// one resolver, which is what makes some of them time out.
const SPF_INCLUDE_CONCURRENCY: usize = 4;

/// Time limits for one full check. A check that runs out of time is a failed
/// attempt (nothing partial is kept), so the budget decides how patient we are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckBudget {
    /// The whole `check_all`.
    pub overall: Duration,
    /// Each individual check (DMARC, SPF with its include walk, DKIM, ...).
    pub per_check: Duration,
}

impl CheckBudget {
    /// Someone is waiting on the answer (page load, Re-check DNS button).
    pub const INTERACTIVE: Self = Self {
        overall: Duration::from_secs(45),
        per_check: Duration::from_secs(35),
    };
    /// Background refresh: nobody is waiting, so give slow nameservers and
    /// lookup retries room to finish.
    pub const BACKGROUND: Self = Self {
        overall: Duration::from_secs(120),
        per_check: Duration::from_secs(110),
    };
}

const CHECK_TIMEOUT: Duration = CheckBudget::INTERACTIVE.per_check;

/// Combined authentication posture for one domain.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DnsReport {
    pub domain: String,
    pub checked_at: DateTime<Utc>,
    pub score: u8,
    pub grade: String,
    /// Whether DMARC and SPF (with its whole include walk) were measured
    /// completely. When false, `score` / `grade` must not be published: they
    /// would turn "unknown" into a letter. See [`DnsReport::is_complete`].
    pub core_measured: bool,
    pub findings: Vec<Finding>,
    pub dmarc: DmarcCheckResult,
    pub spf: SpfCheckResult,
    pub dkim: DkimCheckResult,
    pub mta_sts: MtaStsCheckResult,
    pub tls_rpt: TlsRptCheckResult,
    pub bimi: BimiCheckResult,
}

impl DnsReport {
    /// A complete result: DMARC answered and SPF answered with every include
    /// resolved. Only a complete report may be stored, cached or shown as the
    /// domain's state; anything else is a failed attempt.
    pub fn is_complete(&self) -> bool {
        self.core_measured && self.dmarc.lookup.is_measured() && self.spf.is_complete()
    }

    /// The report for a domain with no complete result: every lookup reads
    /// as timed out, so every card says "Check failed" and no grade is shown.
    pub fn check_failed(domain: &str, probe: &DkimProbe) -> Self {
        timed_out_report(domain, probe)
    }

    /// Compute [`Finding::points`] on the combined and per-check findings,
    /// for a report that never went through a check.
    pub fn assign_points(&mut self) {
        assign_points(&mut self.findings);
        assign_points(&mut self.dmarc.findings);
        assign_points(&mut self.spf.findings);
        assign_points(&mut self.dkim.findings);
        assign_points(&mut self.mta_sts.findings);
        assign_points(&mut self.tls_rpt.findings);
        assign_points(&mut self.bimi.findings);
    }
}

/// Live checker: outbound DNS/HTTPS with a 5-minute cache.
#[derive(Clone)]
pub struct DnsChecker {
    dns: Arc<dyn Resolver>,
    http: reqwest::Client,
    cache: Cache<String, Cached>,
    /// Port of `https://mta-sts.<domain>/`; only tests move it off 443.
    policy_port: u16,
}

impl DnsChecker {
    pub fn new() -> Result<Self, DnsError> {
        Self::with_resolver(Arc::new(DnsClient::new()?))
    }

    pub fn with_nameservers(ips: &[IpAddr]) -> Result<Self, DnsError> {
        Self::with_resolver(Arc::new(DnsClient::with_nameservers(ips)?))
    }

    /// A checker that looks everything up through `dns`, the MTA-STS policy
    /// host included (a [`crate::TableResolver`] in tests and the demo).
    pub fn with_resolver(dns: Arc<dyn Resolver>) -> Result<Self, DnsError> {
        let http = http_client(Arc::new(HttpResolve(Arc::clone(&dns))), None)?;
        Ok(Self::from_parts(dns, http, 443))
    }

    fn from_parts(dns: Arc<dyn Resolver>, http: reqwest::Client, policy_port: u16) -> Self {
        Self {
            dns,
            http,
            cache: Cache::builder()
                .time_to_live(CACHE_TTL)
                .max_capacity(2048)
                .support_invalidation_closures()
                .build(),
            policy_port,
        }
    }

    /// Forget everything cached about `_dmarc.<domain>` so the next DMARC check
    /// asks the nameservers again: this checker's results for `domain` and the
    /// resolver's answer cache, negative answers included. The resolver cannot
    /// evict a single name, so its whole cache is cleared.
    pub async fn invalidate_dmarc(&self, domain: &str) {
        self.invalidate_domain(domain).await;
        self.dns.clear_cache();
    }

    /// Clear the resolver's answer cache (positive and negative entries), e.g.
    /// before re-checking an authorization record the user just published.
    pub fn clear_resolver_cache(&self) {
        self.dns.clear_cache();
    }

    /// Full posture check.
    ///
    /// `probe` names the DKIM selectors to check: the ones aggregate reports
    /// have seen signing for this domain plus any the user configured. Nothing
    /// is guessed; an empty probe yields a "can't be assessed yet" notice.
    ///
    /// Only a complete report ([`DnsReport::is_complete`]) is cached; an
    /// incomplete one is returned for the caller to treat as a failed attempt.
    pub async fn check_all(&self, domain: &str, probe: &DkimProbe) -> DnsReport {
        self.check_all_within(domain, probe, CheckBudget::INTERACTIVE)
            .await
    }

    /// [`Self::check_all`] with an explicit time budget (background refresh
    /// uses [`CheckBudget::BACKGROUND`]).
    pub async fn check_all_within(
        &self,
        domain: &str,
        probe: &DkimProbe,
        budget: CheckBudget,
    ) -> DnsReport {
        let limit = budget.per_check;
        let work = async {
            let dmarc = self.dmarc_within(domain, limit).await;
            let spf = self.spf_within(domain, limit).await;
            let (dkim, mta_sts, tls_rpt, bimi) = tokio::join!(
                self.dkim_within(domain, probe, limit),
                self.mta_sts_within(domain, limit),
                self.tls_rpt_within(domain, limit),
                self.bimi_within(domain, limit),
            );
            assemble_report(domain, dmarc, spf, dkim, mta_sts, tls_rpt, bimi)
        };
        self.cached(CacheKey::All(domain, &probe.cache_key()), async {
            match timeout(budget.overall, work).await {
                Ok(report) => report,
                Err(_) => {
                    tracing::warn!(
                        domain,
                        resolvers = self.dns.describe(),
                        budget_secs = budget.overall.as_secs(),
                        "dns check ran out of time"
                    );
                    timed_out_report(domain, probe)
                }
            }
        })
        .await
    }

    pub async fn check_dmarc(&self, domain: &str) -> DmarcCheckResult {
        self.dmarc_within(domain, CHECK_TIMEOUT).await
    }

    async fn dmarc_within(&self, domain: &str, limit: Duration) -> DmarcCheckResult {
        self.cached(CacheKey::check(CheckKind::Dmarc, domain), async {
            match timeout(limit, self.probe_dmarc(domain)).await {
                Ok(r) => r,
                Err(_) => evaluate_dmarc(domain, &TxtLookup::Timeout, &[]),
            }
        })
        .await
    }

    /// The `_dmarc` record plus the authorization record of every external
    /// report destination it names.
    async fn probe_dmarc(&self, domain: &str) -> DmarcCheckResult {
        let lookup = self.dns.txt(&format!("_dmarc.{domain}")).await;
        let mut auth = Vec::new();
        if let Some(raw) = lookup.records_starting_with("v=dmarc1").first() {
            let parsed = parse_dmarc_record(raw);
            for uri in parsed.rua.iter().chain(parsed.ruf.iter()) {
                let Some(host) = uri.host.as_deref() else {
                    continue;
                };
                if !report_authorization_needed(domain, host) {
                    continue;
                }
                let name = report_auth_name(domain, host);
                let ans = self.dns.txt(&name).await;
                auth.push((name, ans));
            }
        }
        evaluate_dmarc(domain, &lookup, &auth)
    }

    /// RFC 7489 §7.1: does `rua_domain` accept aggregate reports for `domain`?
    /// Wildcard records (`*._report._dmarc.<rua_domain>`) match through normal DNS.
    pub async fn check_report_authorization(
        &self,
        domain: &str,
        rua_domain: &str,
    ) -> ReportAuthorization {
        let name = report_auth_name(domain, rua_domain);
        let lookup = self.dns.txt(&name).await;
        evaluate_report_authorization(name, &lookup)
    }

    pub async fn check_spf(&self, domain: &str) -> SpfCheckResult {
        self.spf_within(domain, CHECK_TIMEOUT).await
    }

    /// SPF with its include tree. Includes still unanswered when `limit` runs
    /// out stay unresolved, which makes the result incomplete
    /// (`spf_walk_incomplete`): a failed attempt, never cached.
    async fn spf_within(&self, domain: &str, limit: Duration) -> SpfCheckResult {
        self.cached(CacheKey::check(CheckKind::Spf, domain), async {
            let deadline = Instant::now() + limit;
            let lookup = timeout_at(deadline, self.dns.txt(domain))
                .await
                .unwrap_or(TxtLookup::Timeout);
            let nested = if lookup.is_measured() {
                let dns = self.dns.clone();
                collect_spf_includes(&lookup, deadline, move |name| {
                    let dns = dns.clone();
                    async move { dns.txt(&name).await }
                })
                .await
            } else {
                Vec::new()
            };
            evaluate_spf(domain, &lookup, &nested)
        })
        .await
    }

    pub async fn check_dkim(&self, domain: &str, probe: &DkimProbe) -> DkimCheckResult {
        self.dkim_within(domain, probe, CHECK_TIMEOUT).await
    }

    async fn dkim_within(
        &self,
        domain: &str,
        probe: &DkimProbe,
        limit: Duration,
    ) -> DkimCheckResult {
        let probe_key = probe.cache_key();
        let key = CacheKey::Check(CheckKind::Dkim, domain, &probe_key);
        self.cached(key, async {
            let dns = self.dns.clone();
            let mut rows = probe_dkim_selectors(domain, probe.selectors(), limit, move |name| {
                let dns = dns.clone();
                async move { dns.txt(&name).await }
            })
            .await;
            rows.sort_by(|a, b| a.selector.cmp(&b.selector));
            evaluate_dkim(domain, &rows, probe)
        })
        .await
    }

    pub async fn check_mta_sts(&self, domain: &str) -> MtaStsCheckResult {
        self.mta_sts_within(domain, CHECK_TIMEOUT).await
    }

    async fn mta_sts_within(&self, domain: &str, limit: Duration) -> MtaStsCheckResult {
        self.cached(CacheKey::check(CheckKind::MtaSts, domain), async {
            match timeout(limit, self.probe_mta_sts(domain)).await {
                Ok(r) => r,
                Err(_) => evaluate_mta_sts(
                    domain,
                    &TxtLookup::Timeout,
                    None,
                    &PolicyFetch::NotAttempted,
                ),
            }
        })
        .await
    }

    /// The `_mta-sts` TXT record and, when it points receivers at one, the
    /// policy file behind it.
    async fn probe_mta_sts(&self, domain: &str) -> MtaStsCheckResult {
        let txt = self.dns.txt(&format!("_mta-sts.{domain}")).await;
        // Receivers only read the policy when a v=STSv1 record points at
        // it, so neither do we: a file on a wildcard host is not a policy.
        // Only a hostname goes into the URL (no port, path or userinfo).
        let published = !txt.records_starting_with("v=stsv1").is_empty();
        let (body, fetch) = if published && normalize_domain(domain).as_deref() == Ok(domain) {
            self.fetch_policy(domain).await
        } else {
            (None, PolicyFetch::NotAttempted)
        };
        evaluate_mta_sts(domain, &txt, body.as_deref(), &fetch)
    }

    pub async fn check_tls_rpt(&self, domain: &str) -> TlsRptCheckResult {
        self.tls_rpt_within(domain, CHECK_TIMEOUT).await
    }

    async fn tls_rpt_within(&self, domain: &str, limit: Duration) -> TlsRptCheckResult {
        self.cached(CacheKey::check(CheckKind::TlsRpt, domain), async {
            let lookup = timeout(limit, self.dns.txt(&format!("_smtp._tls.{domain}")))
                .await
                .unwrap_or(TxtLookup::Timeout);
            evaluate_tls_rpt(domain, &lookup)
        })
        .await
    }

    pub async fn check_bimi(&self, domain: &str) -> BimiCheckResult {
        self.bimi_within(domain, CHECK_TIMEOUT).await
    }

    async fn bimi_within(&self, domain: &str, limit: Duration) -> BimiCheckResult {
        self.cached(CacheKey::check(CheckKind::Bimi, domain), async {
            let lookup = timeout(limit, self.dns.txt(&format!("default._bimi.{domain}")))
                .await
                .unwrap_or(TxtLookup::Timeout);
            evaluate_bimi(domain, &lookup)
        })
        .await
    }

    /// Forward lookup (A + AAAA) of `hostname`, to confirm a PTR name.
    pub async fn lookup_forward(&self, hostname: &str) -> ForwardLookup {
        match timeout(CHECK_TIMEOUT, self.dns.lookup_forward(hostname)).await {
            Ok(r) => r,
            Err(_) => ForwardLookup {
                hostname: hostname.to_owned(),
                lookup: LookupStatus::Timeout,
                addresses: Vec::new(),
                lookup_detail: None,
            },
        }
    }

    /// Reverse DNS for an IP, cached.
    pub async fn lookup_ptr(&self, ip: IpAddr) -> PtrLookup {
        self.cached(CacheKey::Ptr(ip), async {
            match timeout(CHECK_TIMEOUT, self.dns.lookup_ptr(ip)).await {
                Ok(r) => r,
                Err(_) => PtrLookup {
                    ip: ip.to_string(),
                    lookup: LookupStatus::Timeout,
                    hostnames: Vec::new(),
                    lookup_detail: None,
                },
            }
        })
        .await
    }
}

/// Look up every selector's `_domainkey` TXT record, at most
/// `DKIM_CONCURRENCY` at a time, and return one row per selector.
///
/// `budget` is a shared deadline covering both the wait for a concurrency
/// permit and the lookup itself. A selector that misses it becomes a
/// `TxtLookup::Timeout` row (and so a `dkim_timeout` finding) instead of
/// discarding the selectors that did finish.
async fn probe_dkim_selectors<F, Fut>(
    domain: &str,
    selectors: Vec<String>,
    budget: Duration,
    lookup: F,
) -> Vec<DkimSelectorResult>
where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = TxtLookup> + Send + 'static,
{
    let deadline = Instant::now() + budget;
    let sem = Arc::new(Semaphore::new(DKIM_CONCURRENCY));
    let mut handles = Vec::new();
    for sel in selectors {
        let lookup = lookup.clone();
        let domain = domain.to_string();
        let permit = sem.clone();
        let task_sel = sel.clone();
        let handle = tokio::spawn(async move {
            let name = format!("{task_sel}._domainkey.{domain}");
            let work = async move {
                let _g = permit.acquire_owned().await.expect("semaphore");
                lookup(name).await
            };
            let answer = timeout_at(deadline, work)
                .await
                .unwrap_or(TxtLookup::Timeout);
            evaluate_dkim_selector(&domain, &task_sel, &answer)
        });
        handles.push((sel, handle));
    }
    let mut rows = Vec::with_capacity(handles.len());
    for (sel, h) in handles {
        let row = h.await.unwrap_or_else(|e| {
            evaluate_dkim_selector(
                domain,
                &sel,
                &TxtLookup::ServFail {
                    detail: format!("lookup task failed: {e}"),
                },
            )
        });
        rows.push(row);
    }
    rows
}

/// The include and redirect targets an SPF record names, in record order
/// (`redirect=` last, and only when no `all` makes it dead).
fn spf_targets(lookup: &TxtLookup) -> Vec<String> {
    lookup
        .records_starting_with("v=spf1")
        .first()
        .map(|raw| {
            let parsed = parse_spf_record(raw);
            let mut t: Vec<String> = parsed
                .terms
                .iter()
                .filter(|term| matches!(term.kind, Mechanism::Include | Mechanism::Redirect))
                .filter_map(|term| term.value.clone())
                .collect();
            if !parsed.has_all {
                if let Some(r) = parsed.redirect {
                    if !t.iter().any(|x| x == &r) {
                        t.push(r);
                    }
                }
            }
            t
        })
        .unwrap_or_default()
}

/// Resolve the SPF include/redirect tree under `root`, level by level, at most
/// [`SPF_INCLUDE_CONCURRENCY`] lookups at a time.
///
/// Stops at `deadline`: names not answered by then are left out, and
/// `evaluate_spf` reports them as unresolved (an incomplete walk). Answers
/// already in hand are kept for the inspector, but the result as a whole is
/// a failed attempt.
async fn collect_spf_includes<F, Fut>(
    root: &TxtLookup,
    deadline: Instant,
    lookup: F,
) -> Vec<(String, TxtLookup)>
where
    F: Fn(String) -> Fut + Clone,
    Fut: Future<Output = TxtLookup>,
{
    let sem = Semaphore::new(SPF_INCLUDE_CONCURRENCY);
    let mut nested = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = spf_targets(root);
    while !queue.is_empty() && (nested.len() as u32) < SPF_MAX_WALK {
        let mut batch = Vec::new();
        for name in queue.drain(..) {
            let key = normalize_name(&name);
            if seen.insert(key.clone()) {
                batch.push(key);
                if (nested.len() + batch.len()) as u32 >= SPF_MAX_WALK {
                    break;
                }
            }
        }
        if batch.is_empty() {
            break;
        }
        let futures = batch.into_iter().map(|key| {
            let lookup = lookup.clone();
            let sem = &sem;
            async move {
                let work = async {
                    let _permit = sem.acquire().await.expect("semaphore");
                    lookup(key.clone()).await
                };
                (key.clone(), timeout_at(deadline, work).await.ok())
            }
        });
        let results = futures::future::join_all(futures).await;
        let mut out_of_time = false;
        for (key, answer) in results {
            let Some(answer) = answer else {
                out_of_time = true;
                continue;
            };
            queue.extend(
                spf_targets(&answer)
                    .into_iter()
                    .filter(|n| !seen.contains(&normalize_name(n))),
            );
            nested.push((key, answer));
        }
        if out_of_time {
            break;
        }
    }
    nested
}

/// Build a [`DnsReport`] from per-check results (also used by persistence tests).
///
/// Points are assigned on the combined findings here; each check's own
/// findings got theirs when the check ran ([`DnsChecker::cached`]).
pub fn assemble_report(
    domain: &str,
    dmarc: DmarcCheckResult,
    spf: SpfCheckResult,
    dkim: DkimCheckResult,
    mta_sts: MtaStsCheckResult,
    tls_rpt: TlsRptCheckResult,
    bimi: BimiCheckResult,
) -> DnsReport {
    let mut findings = Vec::new();
    findings.extend(dmarc.findings.iter().cloned());
    findings.extend(spf.findings.iter().cloned());
    findings.extend(dkim.findings.iter().cloned());
    findings.extend(mta_sts.findings.iter().cloned());
    findings.extend(tls_rpt.findings.iter().cloned());
    findings.extend(bimi.findings.iter().cloned());
    sort_findings(&mut findings);
    assign_points(&mut findings);
    let score = score_from_findings(&findings);
    let core_measured = dmarc.lookup.is_measured() && spf.is_complete();
    DnsReport {
        domain: domain.to_string(),
        checked_at: Utc::now(),
        score,
        grade: grade_from_score(score).to_string(),
        core_measured,
        findings,
        dmarc,
        spf,
        dkim,
        mta_sts,
        tls_rpt,
        bimi,
    }
}

fn timed_out_report(domain: &str, probe: &DkimProbe) -> DnsReport {
    let empty = TxtLookup::Timeout;
    let mut report = assemble_report(
        domain,
        evaluate_dmarc(domain, &empty, &[]),
        evaluate_spf(domain, &empty, &[]),
        evaluate_dkim(domain, &[], probe),
        evaluate_mta_sts(domain, &empty, None, &PolicyFetch::NotAttempted),
        evaluate_tls_rpt(domain, &empty),
        evaluate_bimi(domain, &empty),
    );
    // These results never went through a check, so their points are unset.
    report.assign_points();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn slow_dkim_selector_times_out_without_dropping_the_others() {
        let rows = probe_dkim_selectors(
            "example.com",
            vec!["fast".into(), "slow".into()],
            Duration::from_millis(200),
            |name: String| async move {
                if name.starts_with("slow.") {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                TxtLookup::Records(crate::lookup::TxtAnswer::from_records(vec![
                    "v=DKIM1; k=ed25519; p=11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=".into(),
                ]))
            },
        )
        .await;
        assert_eq!(rows.len(), 2);
        let fast = rows.iter().find(|r| r.selector == "fast").unwrap();
        assert!(fast.present);
        let slow = rows.iter().find(|r| r.selector == "slow").unwrap();
        assert_eq!(slow.lookup, LookupStatus::Timeout);
        assert!(slow.findings.iter().any(|f| f.code == "dkim_timeout"));

        let probe = DkimProbe::requested(&["fast".into(), "slow".into()]);
        let agg = evaluate_dkim("example.com", &rows, &probe);
        assert_eq!(agg.discovered_count, 1);
        assert!(agg.findings.iter().any(|f| f.code == "dkim_timeout"));
        assert!(!agg
            .findings
            .iter()
            .any(|f| f.code == "dkim_selector_not_published"));
    }

    fn spf(raw: &str) -> TxtLookup {
        TxtLookup::Records(crate::lookup::TxtAnswer::from_records(vec![raw.into()]))
    }

    /// A wide include tree is resolved at most `SPF_INCLUDE_CONCURRENCY`
    /// lookups at a time, and every level is walked.
    #[tokio::test]
    async fn include_walk_runs_at_most_four_lookups_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let terms: String = (0..12).map(|i| format!("include:n{i}.test ")).collect();
        let root = spf(&format!("v=spf1 {terms}-all"));
        let (f, p) = (in_flight.clone(), peak.clone());
        let nested = collect_spf_includes(
            &root,
            Instant::now() + Duration::from_secs(5),
            move |name: String| {
                let (f, p) = (f.clone(), p.clone());
                async move {
                    let now = f.fetch_add(1, Ordering::SeqCst) + 1;
                    p.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    f.fetch_sub(1, Ordering::SeqCst);
                    if name == "n0.test" {
                        spf("v=spf1 include:deep.test -all")
                    } else {
                        spf("v=spf1 -all")
                    }
                }
            },
        )
        .await;
        assert_eq!(nested.len(), 13, "12 includes plus one nested level");
        assert_eq!(peak.load(Ordering::SeqCst), SPF_INCLUDE_CONCURRENCY);
        let result = evaluate_spf("example.com", &root, &nested);
        assert!(result.is_complete());
    }

    /// Includes still unanswered at the deadline make the SPF result
    /// incomplete, and so the whole report: a failed attempt, not a result.
    #[tokio::test]
    async fn include_walk_out_of_time_is_an_incomplete_report() {
        let root = spf("v=spf1 include:fast.test include:slow.test -all");
        let nested = collect_spf_includes(
            &root,
            Instant::now() + Duration::from_millis(100),
            |name: String| async move {
                if name == "slow.test" {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                spf("v=spf1 -all")
            },
        )
        .await;
        assert_eq!(nested.len(), 1);
        let spf_result = evaluate_spf("example.com", &root, &nested);
        assert!(!spf_result.is_complete());
        assert!(spf_result.lookup.is_measured());

        let dmarc = spf("v=DMARC1; p=reject");
        let report = assemble_report(
            "example.com",
            evaluate_dmarc("example.com", &dmarc, &[]),
            spf_result,
            evaluate_dkim("example.com", &[], &DkimProbe::default()),
            evaluate_mta_sts(
                "example.com",
                &TxtLookup::NoRecords,
                None,
                &PolicyFetch::NotAttempted,
            ),
            evaluate_tls_rpt("example.com", &TxtLookup::NoRecords),
            evaluate_bimi("example.com", &TxtLookup::NoRecords),
        );
        assert!(!report.core_measured);
        assert!(!report.is_complete());
    }

    #[test]
    fn cache_keys_keep_their_prefix_layout() {
        assert_eq!(
            CacheKey::All("example.com", "o=s1;c=").render(),
            "all:example.com:o=s1;c="
        );
        assert_eq!(
            CacheKey::Check(CheckKind::Dkim, "example.com", "o=s1;c=").render(),
            "dkim:example.com:o=s1;c="
        );
        assert_eq!(
            CacheKey::check(CheckKind::MtaSts, "example.com").render(),
            "mta_sts:example.com"
        );
        assert_eq!(
            CacheKey::Ptr("192.0.2.1".parse().unwrap()).render(),
            "ptr:192.0.2.1"
        );
    }

    #[tokio::test]
    async fn invalidate_domain_drops_prefixed_entries() {
        let checker =
            DnsChecker::with_nameservers(&["192.0.2.53".parse().unwrap()]).expect("checker");
        let report = Arc::new(timed_out_report("example.com", &DkimProbe::default()));
        let dkim = Arc::new(report.dkim.clone());
        for key in ["all:example.com:o=s1;c=", "all:example.com:"] {
            checker
                .cache
                .insert(key.to_string(), Cached::Report(report.clone()))
                .await;
        }
        checker
            .cache
            .insert(
                "dkim:example.com:o=s1;c=".into(),
                Cached::Dkim(dkim.clone()),
            )
            .await;
        checker
            .cache
            .insert("all:example.net:".into(), Cached::Report(report.clone()))
            .await;

        checker.invalidate_domain("example.com").await;

        assert!(checker.cache.get("all:example.com:o=s1;c=").await.is_none());
        assert!(checker.cache.get("all:example.com:").await.is_none());
        assert!(checker
            .cache
            .get("dkim:example.com:o=s1;c=")
            .await
            .is_none());
        assert!(checker.cache.get("all:example.net:").await.is_some());
    }

    /// A measured result is served from the cache; a timed-out one is not kept.
    #[tokio::test]
    async fn cached_keeps_measurements_and_assigns_points() {
        let checker =
            DnsChecker::with_nameservers(&["192.0.2.53".parse().unwrap()]).expect("checker");
        let key = CacheKey::check(CheckKind::TlsRpt, "example.com");
        let missing = checker
            .cached(key, async {
                evaluate_tls_rpt("example.com", &TxtLookup::NoRecords)
            })
            .await;
        let tls_rpt_missing = missing
            .findings
            .iter()
            .find(|f| f.code == "tls_rpt_missing")
            .expect("tls_rpt_missing");
        assert_eq!(tls_rpt_missing.points, 2);
        // The second call never runs its work.
        let ran = std::sync::atomic::AtomicBool::new(false);
        let again = checker
            .cached(key, async {
                ran.store(true, std::sync::atomic::Ordering::SeqCst);
                evaluate_tls_rpt("example.com", &TxtLookup::NxDomain)
            })
            .await;
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(again, missing);

        let key = CacheKey::check(CheckKind::Bimi, "example.com");
        let timed_out = checker
            .cached(key, async {
                evaluate_bimi("example.com", &TxtLookup::Timeout)
            })
            .await;
        assert_eq!(timed_out.lookup, LookupStatus::Timeout);
        let retried = checker
            .cached(key, async {
                evaluate_bimi("example.com", &TxtLookup::NxDomain)
            })
            .await;
        assert_eq!(retried.lookup, LookupStatus::Nxdomain);
    }
}
