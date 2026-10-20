//! Persisted DNS results ("snapshots") and what keeps them current: the
//! [`DnsService`] (the checker for the configured resolvers, this process's
//! record of check attempts with its retry backoff, the refreshes in flight)
//! and the warm pass over every monitored domain. The API serves these
//! results; it never waits on a background check.
//!
//! Only complete results are ever stored; a partial one never replaces the
//! last complete result (see `api::dns` for the states a result is shown in).

use crate::error::{AppError, AppResult};
use dmarc_assess::critical_count;
use dmarc_dns::dkim::{DkimProbe, ObservedSelector};
use dmarc_dns::{
    CheckBudget, DnsChecker, DnsError, DnsReport, Resolver, TableResolver, CHECKER_VERSION,
};
use dmarc_store::{
    dns_snapshot_is_fresh, get_custom_dkim_selectors, get_dns_snapshot, get_dns_snapshots,
    list_observed_dkim_selectors, unix_now, upsert_dns_snapshot, AppSettings, DnsSnapshot,
    SnapshotDmarc,
};
use futures::stream::{self, StreamExt};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, RwLock};
use tokio_util::sync::CancellationToken;

/// Serve a snapshot without waiting on DNS when it is newer than this.
pub(crate) const FRESH_SECS: i64 = 60 * 60;

/// Resolvers used while no nameservers are stored (Cloudflare, then Google).
pub(crate) const DEFAULT_NAMESERVERS: [IpAddr; 2] = [
    IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
    IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
];

/// Domains the warm pass checks at the same time. Three stays gentle on the
/// resolvers (a check is a few dozen queries, and each lane pauses
/// [`WARM_PAUSE`] between domains) while a pass over 500 stale domains takes
/// a third of the sequential time, and one slow domain (up to the two-minute
/// [`CheckBudget::BACKGROUND`]) no longer holds up all the others.
pub(crate) const WARM_CONCURRENCY: usize = 3;

/// Pause after each warm check before that lane starts the next domain.
const WARM_PAUSE: Duration = Duration::from_millis(500);

/// First pause before a background retry of a domain whose check failed;
/// doubles per consecutive failure up to [`RETRY_MAX_SECS`]. Shorter than the
/// warm interval so the first retry lands on the next warm cycle.
pub(crate) const RETRY_BASE_SECS: i64 = 10 * 60;
pub(crate) const RETRY_MAX_SECS: i64 = 4 * 60 * 60;

/// For a demo database (one the `demo-data` example made), the resolver that
/// answers its domains' TXT lookups from the records stored with it, so their
/// DNS says what their made-up reports say; PTR and A/AAAA lookups (the
/// senders' names) still go to the default nameservers. `None` for every
/// real database.
pub async fn demo_resolver(pool: &SqlitePool) -> anyhow::Result<Option<Arc<dyn Resolver>>> {
    let Some(records) = dmarc_store::get_demo_dns_records(pool).await? else {
        return Ok(None);
    };
    let table = records
        .iter()
        .fold(TableResolver::new(), |table, (name, txt)| {
            let txt: Vec<&str> = txt.iter().map(String::as_str).collect();
            table.with_txt(name, &txt)
        });
    let table = table.with_address_lookups(dmarc_dns::default_resolver()?);
    Ok(Some(Arc::new(table)))
}

/// This process's record of check attempts for one domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Attempts {
    pub(crate) last_attempt_at: i64,
    /// Consecutive failed or incomplete attempts; 0 after a complete one.
    pub(crate) failures: u32,
}

impl Attempts {
    /// Earliest time a background check may try this domain again.
    pub(crate) fn next_retry_at(&self) -> i64 {
        if self.failures == 0 {
            return self.last_attempt_at;
        }
        let shift = (self.failures - 1).min(16);
        let wait = RETRY_BASE_SECS
            .saturating_mul(1 << shift)
            .min(RETRY_MAX_SECS);
        self.last_attempt_at.saturating_add(wait)
    }

    pub(crate) fn retry_due(&self, now: i64) -> bool {
        now >= self.next_retry_at()
    }
}

/// The live DNS checking of this process: one checker built for the stored
/// resolvers (rebuilt when they change), the attempts per domain with their
/// backoff (in memory on purpose: after a restart every domain is simply
/// tried again), and the domains with a refresh in flight so repeated page
/// loads of a stale domain start one live check, not one per request. Also
/// the warm loop's state: whether a pass runs, and its wake-up call for
/// domains that appeared since the last pass.
#[derive(Default)]
pub struct DnsService {
    /// The checker and the configured resolvers it was built for.
    checker: RwLock<Option<(Vec<IpAddr>, Arc<DnsChecker>)>>,
    /// Answers every lookup in place of the configured resolvers (tests).
    resolver: Option<Arc<dyn Resolver>>,
    attempts: Mutex<HashMap<String, Attempts>>,
    refreshing: Mutex<HashSet<String>>,
    /// A warm pass is running.
    warming: AtomicBool,
    /// [`Self::wake`] was called and the warm pass it asks for has not started.
    woken: AtomicBool,
    wake: Arc<Notify>,
}

impl DnsService {
    /// A service whose checker looks everything up through `resolver`
    /// instead of the stored resolvers: a stub that answers at once, so tests
    /// neither wait on DNS timeouts nor depend on the machine's network.
    pub fn with_resolver(resolver: Arc<dyn Resolver>) -> Self {
        Self {
            resolver: Some(resolver),
            ..Self::default()
        }
    }

    /// The checker for the resolvers stored in `pool` (the defaults when none
    /// are stored, or when the settings cannot be read: DNS checks must not
    /// fail on a database hiccup).
    pub(crate) async fn checker(&self, pool: &SqlitePool) -> AppResult<Arc<DnsChecker>> {
        let setting = AppSettings::load(pool)
            .await
            .map(|s| s.dns.addresses())
            .unwrap_or_default();
        {
            let r = self.checker.read().await;
            if let Some((cached, checker)) = r.as_ref() {
                if cached == &setting {
                    return Ok(checker.clone());
                }
            }
        }
        let mut w = self.checker.write().await;
        if let Some((cached, checker)) = w.as_ref() {
            if cached == &setting {
                return Ok(checker.clone());
            }
        }
        let ips = if setting.is_empty() {
            DEFAULT_NAMESERVERS.to_vec()
        } else {
            setting.clone()
        };
        let created = match &self.resolver {
            Some(resolver) => DnsChecker::with_resolver(Arc::clone(resolver)),
            None => DnsChecker::with_nameservers(&ips),
        };
        let created = created.map_err(|e| match e {
            DnsError::Init(msg) => {
                tracing::error!(error = %msg, "dns checker init failed");
                AppError::internal("dns resolver is unavailable")
            }
        })?;
        let checker = Arc::new(created);
        *w = Some((setting, checker.clone()));
        Ok(checker)
    }

    /// The resolvers changed: build a new checker, and forget every domain's
    /// failed attempts, whose backoff (up to 4 h) was earned against the old
    /// resolvers.
    pub(crate) async fn reconfigure(&self, pool: &SqlitePool) {
        *self.checker.write().await = None;
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let _ = self.checker(pool).await;
    }

    /// Ask the warm loop for a pass soon (new domains appeared: added, or
    /// found in a report) instead of at its next interval. Calls coalesce.
    pub(crate) fn wake(&self) {
        self.woken.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }

    /// What [`Self::wake`] notifies: the warm loop waits on it (a call since
    /// the last wait counts at once).
    pub(crate) fn wake_signal(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }

    /// A warm pass is running, or one was asked for by [`Self::wake`].
    pub(crate) fn warm_pending(&self) -> bool {
        self.warming.load(Ordering::Relaxed) || self.woken.load(Ordering::Relaxed)
    }

    pub(crate) fn attempts_for(&self, domain: &str) -> Option<Attempts> {
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(domain)
            .copied()
    }

    /// Record an attempt at `now`; returns the updated state.
    pub(crate) fn record_attempt(&self, domain: &str, complete: bool, now: i64) -> Attempts {
        let mut map = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = map.entry(domain.to_owned()).or_insert(Attempts {
            last_attempt_at: now,
            failures: 0,
        });
        entry.last_attempt_at = now;
        entry.failures = if complete {
            0
        } else {
            entry.failures.saturating_add(1)
        };
        *entry
    }

    /// Whether this process's latest attempt at `domain` failed.
    pub(crate) fn failing(&self, domain: &str) -> bool {
        self.attempts_for(domain).is_some_and(|a| a.failures > 0)
    }

    /// Whether a background check may run for `domain` now (not backing off).
    fn retry_due(&self, domain: &str, now: i64) -> bool {
        self.attempts_for(domain).is_none_or(|a| a.retry_due(now))
    }

    /// Claim the one refresh slot of `domain`; `None` when one is running.
    fn claim(self: &Arc<Self>, domain: &str) -> Option<RefreshSlot> {
        let mut set = self
            .refreshing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set.insert(domain.to_owned()).then(|| RefreshSlot {
            service: Arc::clone(self),
            domain: domain.to_owned(),
        })
    }

    /// One patient background check ([`CheckBudget::BACKGROUND`]): record the
    /// attempt and store the result only when complete.
    async fn background_check(&self, pool: &SqlitePool, engine: &DnsChecker, domain: &str) -> bool {
        let probe = dkim_probe(pool, domain).await;
        let report = engine
            .check_all_within(domain, &probe, CheckBudget::BACKGROUND)
            .await;
        let complete = report.is_complete();
        let attempts = self.record_attempt(domain, complete, unix_now());
        if !complete {
            tracing::warn!(
                domain = %domain,
                failures = attempts.failures,
                next_retry_in_secs = attempts.next_retry_at() - attempts.last_attempt_at,
                "dns background check incomplete; keeping previous result"
            );
            return false;
        }
        match persist_report(pool, &report).await {
            Ok(_) => {
                tracing::info!(domain = %domain, grade = %report.grade, "dns snapshot refreshed");
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, domain = %domain, "failed to persist dns snapshot");
                false
            }
        }
    }

    /// Refresh `domain` in the background, unless it is backing off after
    /// failed attempts or a refresh is already in flight.
    pub(crate) fn spawn_refresh(self: &Arc<Self>, pool: SqlitePool, domain: String) {
        if !self.retry_due(&domain, unix_now()) {
            tracing::debug!(domain = %domain, "dns refresh backing off after failed attempts");
            return;
        }
        let Some(slot) = self.claim(&domain) else {
            tracing::debug!(domain = %domain, "dns refresh already in flight");
            return;
        };
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let _slot = slot;
            let engine = match service.checker(&pool).await {
                Ok(engine) => engine,
                Err(e) => {
                    tracing::warn!(error = %e, domain = %domain, "dns refresh checker init failed");
                    return;
                }
            };
            service.background_check(&pool, &engine, &domain).await;
        });
    }

    /// One warm pass: re-check every monitored domain whose snapshot is not
    /// fresh, up to [`WARM_CONCURRENCY`] domains at a time, each with the
    /// patient [`CheckBudget::BACKGROUND`] and a short pause after it so the
    /// resolvers are not stampeded. A domain backing off after failed attempts
    /// (see [`Attempts::next_retry_at`]) is skipped, and so is one whose
    /// refresh is already in flight (the same slot a UI-triggered
    /// [`Self::spawn_refresh`] claims, so the two never check one domain
    /// twice). Once `token` is cancelled no further domain starts; checks
    /// already running finish (or are aborted with the task at shutdown).
    pub(crate) async fn warm_once(self: &Arc<Self>, pool: &SqlitePool, token: &CancellationToken) {
        self.warming.store(true, Ordering::Relaxed);
        self.woken.store(false, Ordering::Relaxed);
        let _warming = WarmingFlag(&self.warming);
        let engine = match self.checker(pool).await {
            Ok(engine) => engine,
            Err(e) => {
                tracing::warn!(error = %e, "dns warm: checker init failed");
                return;
            }
        };
        let domains = match dmarc_store::list_monitored_domains(pool).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "dns warm: failed to list domains");
                return;
            }
        };
        if domains.is_empty() {
            return;
        }
        let snapshots = match get_dns_snapshots(pool, &domains).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "dns warm: snapshot lookup failed");
                return;
            }
        };

        // Grades of an older checker first: they are on screen marked "updating".
        let mut queue: Vec<(&String, Option<&DnsSnapshot>)> = domains
            .iter()
            .map(|d| (d, snapshots.get(d).filter(|s| complete_report(s).is_some())))
            .collect();
        queue.sort_by_key(|(_, s)| !s.is_some_and(|s| dns_outdated(s.checker_version)));

        let checked = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        stream::iter(queue)
            .for_each_concurrent(WARM_CONCURRENCY, |(domain, snapshot)| {
                let (engine, checked, failed) = (&engine, &checked, &failed);
                async move {
                    if token.is_cancelled() {
                        return;
                    }
                    let Some(_slot) = self.claim_for_warm(domain, snapshot, unix_now()) else {
                        return;
                    };
                    let counter = if self.background_check(pool, engine, domain).await {
                        checked
                    } else {
                        failed
                    };
                    counter.fetch_add(1, Ordering::Relaxed);
                    tokio::select! {
                        () = token.cancelled() => {}
                        () = tokio::time::sleep(WARM_PAUSE) => {}
                    }
                }
            })
            .await;

        let (checked, failed) = (checked.into_inner(), failed.into_inner());
        if checked > 0 || failed > 0 {
            tracing::info!(
                checked,
                failed,
                total = domains.len(),
                "dns warm: cycle done"
            );
        }
    }

    /// The refresh slot of `domain` for a warm check at `now`, or `None` when
    /// the domain needs none: its `snapshot` is fresh (and the latest attempt
    /// did not fail), it is backing off, or a refresh is already in flight.
    fn claim_for_warm(
        self: &Arc<Self>,
        domain: &str,
        snapshot: Option<&DnsSnapshot>,
        now: i64,
    ) -> Option<RefreshSlot> {
        if let Some(s) = snapshot {
            if !self.failing(domain) && dns_snapshot_is_fresh(s, now, FRESH_SECS, CHECKER_VERSION) {
                return None;
            }
        }
        if !self.retry_due(domain, now) {
            return None;
        }
        self.claim(domain)
    }
}

/// Clears [`DnsService`]'s `warming` when the pass ends, however it ends.
struct WarmingFlag<'a>(&'a AtomicBool);

impl Drop for WarmingFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Removes its domain from the service's in-flight set when the refresh ends,
/// however it ends.
struct RefreshSlot {
    service: Arc<DnsService>,
    domain: String,
}

impl Drop for RefreshSlot {
    fn drop(&mut self) {
        self.service
            .refreshing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.domain);
    }
}

/// The DKIM probe for a monitored domain: selectors aggregate reports have seen
/// signing for it, plus the user's configured selector list.
///
/// Nothing is guessed. A failure to read either source degrades to probing
/// less (and, with nothing left, a zero-deduction "can't be assessed yet"
/// notice) instead of failing the grade, so a DB hiccup cannot take the
/// dashboard down.
pub(crate) async fn dkim_probe(pool: &SqlitePool, domain: &str) -> DkimProbe {
    let observed = match list_observed_dkim_selectors(pool, domain).await {
        Ok(rows) => rows
            .into_iter()
            .map(|o| ObservedSelector {
                selector: o.selector,
                pass_messages: o.pass_messages,
                fail_messages: o.fail_messages,
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, domain = %domain, "could not read observed dkim selectors");
            Vec::new()
        }
    };
    DkimProbe::new(observed, &configured_selectors(pool).await)
}

pub(crate) async fn configured_selectors(pool: &SqlitePool) -> Vec<String> {
    get_custom_dkim_selectors(pool).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not read configured dkim selectors");
        Vec::new()
    })
}

/// Persist a complete report of a monitored domain. Returns whether a row was
/// written. Any other domain (looked up in the DNS inspector) leaves nothing
/// behind: its result is answered and forgotten.
///
/// An incomplete one (DMARC/SPF timeout or SERVFAIL, or an SPF include walk
/// that did not finish) must not overwrite a good snapshot or invent a first
/// grade: that would turn our resolver failure into the domain's letter.
pub(crate) async fn persist_report(
    pool: &SqlitePool,
    report: &DnsReport,
) -> Result<bool, dmarc_store::StoreError> {
    if !report.is_complete() {
        tracing::info!(
            domain = %report.domain,
            "skipping dns snapshot persist: result is incomplete"
        );
        return Ok(false);
    }
    // False when the domain is not (or no longer) monitored.
    upsert_dns_snapshot(pool, &dns_snapshot(report)?).await
}

/// The row a complete `report` is stored as: the report itself, its verdicts
/// and the DMARC record it found as the columns the queries read (the tags
/// of the parsed record, none without one).
pub fn dns_snapshot(report: &DnsReport) -> Result<DnsSnapshot, serde_json::Error> {
    let dmarc = &report.dmarc;
    let parsed = dmarc.parsed.as_ref().filter(|_| !dmarc.records.is_empty());
    let tag = |f: fn(&dmarc_dns::dmarc::ParsedDmarc) -> &Option<String>| {
        parsed.and_then(|p| f(p).clone())
    };
    Ok(DnsSnapshot {
        domain: report.domain.clone(),
        checked_at: report.checked_at.timestamp(),
        grade: report.grade.clone(),
        score: i64::from(report.score),
        critical_count: critical_count(&report.findings),
        dmarc: SnapshotDmarc {
            dmarc_record_count: dmarc.records.len() as i64,
            dmarc_record: match dmarc.records.as_slice() {
                [record] => Some(record.clone()),
                _ => None,
            },
            policy_p: tag(|p| &p.p),
            policy_sp: tag(|p| &p.sp),
            policy_pct: parsed.and_then(|p| p.pct).map(i64::from),
            policy_adkim: tag(|p| &p.adkim),
            policy_aspf: tag(|p| &p.aspf),
        },
        report_json: serde_json::to_string(report)?,
        checker_version: i64::from(CHECKER_VERSION),
    })
}

/// Whether `checker_version` (of a stored result) is older than this checker:
/// the result is still shown, marked updating, and re-checked first.
pub(crate) fn dns_outdated(checker_version: i64) -> bool {
    checker_version != i64::from(CHECKER_VERSION)
}

/// The complete report a stored snapshot holds, if it is one: valid JSON and
/// complete (see [`DnsReport::is_complete`]). Results of an older checker
/// count: until the re-check replaces them, the last result is better than none.
pub(crate) fn complete_report(snap: &DnsSnapshot) -> Option<DnsReport> {
    let Ok(report) = serde_json::from_str::<DnsReport>(&snap.report_json) else {
        tracing::warn!(domain = %snap.domain, "stored dns snapshot was not valid JSON; ignoring it");
        return None;
    };
    if !report.is_complete() {
        tracing::info!(domain = %snap.domain, "stored dns snapshot is incomplete; ignoring it");
        return None;
    }
    Some(report)
}

/// The stored snapshot for `domain` when it holds a complete result.
pub(crate) async fn load_complete_snapshot(
    pool: &SqlitePool,
    domain: &str,
) -> AppResult<Option<(DnsSnapshot, DnsReport)>> {
    let Some(snap) = get_dns_snapshot(pool, domain).await? else {
        return Ok(None);
    };
    Ok(complete_report(&snap).map(|report| (snap, report)))
}

/// The complete reports stored for `domains`, keyed by name (one query).
pub(crate) async fn load_complete_snapshots(
    pool: &SqlitePool,
    domains: &[String],
) -> AppResult<HashMap<String, DnsReport>> {
    Ok(get_dns_snapshots(pool, domains)
        .await?
        .into_iter()
        .filter_map(|(domain, snap)| complete_report(&snap).map(|r| (domain, r)))
        .collect())
}

#[cfg(test)]
mod probe_tests {
    use super::dkim_probe;
    use dmarc_store::{init, set_custom_dkim_selectors};

    /// The grade probes what reports observed plus the configured list, and
    /// nothing else: no guessed names.
    #[tokio::test]
    async fn probe_is_reports_plus_configured_list() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&dir.path().join("test.db")).await.unwrap();
        sqlx::query(
            "INSERT INTO domains (id, organization_id, domain) VALUES (7, 1, 'example.com')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"INSERT INTO reports (id, report_id, org_name, email, domain_id, date_begin,
               date_end, total_messages, compliant_messages)
               VALUES (1, 'r1', 'google.com', 'x@y', 7, 1000, 2000, 30, 30)"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO records (id, report_id, source_ip, count) VALUES (1, 1, '192.0.2.1', 30)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO record_dkim_auth (record_id, domain, selector, result) VALUES (1, 'example.com', 'Selector1', 'pass')",
        )
        .execute(&pool)
        .await
        .unwrap();
        set_custom_dkim_selectors(&pool, &["k1".into()])
            .await
            .unwrap();

        let probe = dkim_probe(&pool, "example.com").await;
        assert_eq!(probe.selectors(), vec!["k1", "selector1"]);
        assert_eq!(probe.observation("selector1").unwrap().pass_messages, 30);
        assert!(probe.is_configured("k1"));

        let unseen = dkim_probe(&pool, "example.org").await;
        assert_eq!(unseen.selectors(), vec!["k1"]);
        assert!(unseen.observed.is_empty());
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;

    #[test]
    fn one_refresh_per_domain_until_it_finishes() {
        let service = Arc::new(DnsService::default());
        let first = service.claim("slot.example").expect("first claim");
        assert!(service.claim("slot.example").is_none());
        assert!(service.claim("other-slot.example").is_some());
        drop(first);
        assert!(service.claim("slot.example").is_some());
    }

    fn snapshot(domain: &str, checked_at: i64) -> DnsSnapshot {
        DnsSnapshot {
            domain: domain.into(),
            checked_at,
            grade: "A".into(),
            score: 100,
            report_json: "{}".into(),
            checker_version: i64::from(CHECKER_VERSION),
            ..Default::default()
        }
    }

    /// The warm pass skips a fresh domain, one backing off and one a UI
    /// refresh is already checking; it takes the slot of a stale one.
    #[test]
    fn warm_claims_only_stale_idle_domains() {
        let service = Arc::new(DnsService::default());
        let now = 100_000;
        let fresh = snapshot("fresh.example", now);
        let stale = snapshot("stale.example", now - FRESH_SECS);

        assert!(service
            .claim_for_warm("fresh.example", Some(&fresh), now)
            .is_none());
        let slot = service.claim_for_warm("stale.example", Some(&stale), now);
        assert!(slot.is_some());
        // Held by the warm pass: a UI refresh of it (or another lane) waits.
        assert!(service.claim("stale.example").is_none());
        drop(slot);

        let ui_refresh = service.claim("new.example").expect("ui refresh");
        assert!(service.claim_for_warm("new.example", None, now).is_none());
        drop(ui_refresh);
        assert!(service.claim_for_warm("new.example", None, now).is_some());

        // A failed attempt re-checks even a fresh snapshot, but only once due.
        service.record_attempt("fresh.example", false, now);
        assert!(service
            .claim_for_warm("fresh.example", Some(&fresh), now + 1)
            .is_none());
        assert!(service
            .claim_for_warm("fresh.example", Some(&fresh), now + RETRY_BASE_SECS)
            .is_some());
    }

    /// After cancellation the warm pass starts no domain.
    #[tokio::test]
    async fn warm_starts_nothing_once_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let pool = dmarc_store::init(&dir.path().join("test.db"))
            .await
            .unwrap();
        for (id, domain) in [(1, "a.example"), (2, "b.example"), (3, "c.example")] {
            sqlx::query("INSERT INTO domains (id, organization_id, domain) VALUES (?, 1, ?)")
                .bind(id)
                .bind(domain)
                .execute(&pool)
                .await
                .unwrap();
        }
        let service = Arc::new(DnsService::default());
        let token = CancellationToken::new();
        token.cancel();
        service.warm_once(&pool, &token).await;
        for domain in ["a.example", "b.example", "c.example"] {
            assert!(
                service.attempts_for(domain).is_none(),
                "{domain} was checked"
            );
        }
    }

    #[test]
    fn attempts_back_off_per_service() {
        let a = DnsService::default();
        let b = DnsService::default();
        a.record_attempt("x.example", false, 1_000);
        assert!(a.failing("x.example"));
        assert!(!b.failing("x.example"));
        assert!(!a.retry_due("x.example", 1_000 + RETRY_BASE_SECS - 1));
        assert!(a.retry_due("x.example", 1_000 + RETRY_BASE_SECS));
        a.record_attempt("x.example", true, 2_000);
        assert!(!a.failing("x.example"));
    }
}
