//! Reverse-DNS evidence per sender IP (`identity_snapshots`) and the warm loop
//! that keeps it filled for the highest-volume senders, so naming a sender
//! never waits on live DNS.

use crate::snapshots::dns::DnsService;
use dmarc_dns::DnsChecker;
use dmarc_identity::{classify_ip, IpKind};
use dmarc_store::{
    get_identity_snapshots, identity_snapshot_is_fresh, list_sender_ips_by_volume, unix_now,
    upsert_identity_snapshot, IdentitySnapshot, StoreError,
};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Lookups in flight during a warm pass (PTR + optional forward confirm per IP).
const DNS_CONCURRENCY: usize = 12;

/// Positive results (a PTR was found) stay fresh this long.
///
/// Reverse DNS for mail servers changes far less often than DMARC/SPF policy,
/// so this is intentionally longer than the DNS snapshot one-hour window.
const FRESH_SECS: i64 = 24 * 60 * 60;

/// Negatives (no PTR) stay fresh this long.
///
/// Unresolved names burn the full DNS timeout; re-checking them on the same
/// cadence as positives would waste resolver time for almost no gain.
const NEGATIVE_FRESH_SECS: i64 = 7 * 24 * 60 * 60;

/// Cap how many distinct source IPs one warm pass considers, highest volume first.
const WARM_CANDIDATE_LIMIT: i64 = 2_000;

/// Reverse-DNS evidence for one IP, as persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PtrEvidence {
    hostname: Option<String>,
    /// `None` when there is no PTR to confirm; otherwise whether FCrDNS passed.
    confirmed: Option<bool>,
}

fn snapshot_is_fresh(snap: &IdentitySnapshot, now: i64) -> bool {
    identity_snapshot_is_fresh(snap, now, FRESH_SECS, NEGATIVE_FRESH_SECS)
}

async fn persist_evidence(
    pool: &SqlitePool,
    ip: IpAddr,
    evidence: &PtrEvidence,
) -> Result<(), StoreError> {
    upsert_identity_snapshot(
        pool,
        &ip.to_string(),
        unix_now(),
        evidence.hostname.as_deref(),
        evidence.confirmed,
    )
    .await
}

/// Reverse DNS + optional forward confirmation for `ip`; `None` when the PTR
/// lookup did not complete (timeout, SERVFAIL, no resolver). That is not "no
/// PTR": it must not be stored as a negative, or the name stays unknown for
/// the whole negative window.
///
/// Only public addresses are queried: a private, loopback or documentation
/// address has no public PTR, so asking is a guaranteed round trip to nowhere.
///
/// Uses the configured resolvers, like every other DNS check. A forward
/// lookup that did not complete is no evidence either way, so it is not stored.
async fn resolve_ptr_evidence(checker: &DnsChecker, ip: IpAddr) -> Option<PtrEvidence> {
    if classify_ip(ip) != IpKind::Public {
        return Some(PtrEvidence::default());
    }

    let lookup = checker.lookup_ptr(ip).await;
    if !lookup.lookup.is_measured() {
        tracing::info!(%ip, status = ?lookup.lookup, "ptr lookup incomplete; retrying next pass");
        return None;
    }
    let Some(hostname) = lookup.hostnames.into_iter().next() else {
        return Some(PtrEvidence::default());
    };

    let forward = checker.lookup_forward(&hostname).await;
    if !forward.lookup.is_measured() {
        tracing::info!(%ip, hostname = %hostname, status = ?forward.lookup, "forward lookup incomplete; retrying next pass");
        return None;
    }

    Some(PtrEvidence {
        confirmed: Some(forward.confirms(ip)),
        hostname: Some(hostname),
    })
}

/// Resolve every distinct IP in `ips`, at most [`DNS_CONCURRENCY`] at a time.
/// IPs whose lookup did not complete are left out.
///
/// Each task does PTR then forward-confirm so concurrency stays bounded and we
/// do not run a second wave of lookups after the first. The resolver's own
/// timeout caps a single bad nameserver.
async fn resolve_ptr_evidence_many(
    checker: &Arc<DnsChecker>,
    ips: &[IpAddr],
) -> HashMap<IpAddr, PtrEvidence> {
    let mut seen = HashSet::with_capacity(ips.len());
    let unique: Vec<IpAddr> = ips.iter().copied().filter(|ip| seen.insert(*ip)).collect();

    let mut resolved = HashMap::with_capacity(unique.len());
    let mut pending = unique.into_iter();
    let mut inflight = JoinSet::new();
    for ip in pending.by_ref().take(DNS_CONCURRENCY) {
        let checker = Arc::clone(checker);
        inflight.spawn(async move { (ip, resolve_ptr_evidence(&checker, ip).await) });
    }
    while let Some(joined) = inflight.join_next().await {
        match joined {
            Ok((ip, Some(evidence))) => {
                resolved.insert(ip, evidence);
            }
            Ok((_, None)) => {}
            Err(err) => tracing::warn!(error = %err, "ptr evidence task failed"),
        }
        if let Some(ip) = pending.next() {
            let checker = Arc::clone(checker);
            inflight.spawn(async move { (ip, resolve_ptr_evidence(&checker, ip).await) });
        }
    }
    resolved
}

/// One warm pass: resolve stale/missing snapshots for top senders by volume.
/// Ingest adds new source IPs continuously; without this pass those IPs would
/// stay unnamed until the next restart. Stops before resolving when `token`
/// is cancelled.
pub(crate) async fn warm_once(pool: &SqlitePool, dns: &DnsService, token: &CancellationToken) {
    let candidates = match list_sender_ips_by_volume(pool, WARM_CANDIDATE_LIMIT).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "identity warm: failed to list senders");
            return;
        }
    };
    if candidates.is_empty() {
        return;
    }

    let now = unix_now();
    let existing = match get_identity_snapshots(pool, &candidates).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "identity warm: snapshot batch lookup failed");
            return;
        }
    };

    let mut need: Vec<IpAddr> = Vec::new();
    let mut first_time = 0usize;
    for raw in &candidates {
        let Ok(ip) = raw.parse::<IpAddr>() else {
            continue;
        };
        // Non-public addresses cannot have a public PTR; persist an empty
        // snapshot once so we do not keep selecting them as "missing".
        if classify_ip(ip) != IpKind::Public {
            if !existing.contains_key(raw) {
                if let Err(e) = persist_evidence(pool, ip, &PtrEvidence::default()).await {
                    tracing::warn!(error = %e, %ip, "identity warm: persist non-public failed");
                }
            }
            continue;
        }
        match existing.get(raw) {
            Some(snap) if snapshot_is_fresh(snap, now) => {}
            Some(_) => need.push(ip),
            None => {
                first_time += 1;
                need.push(ip);
            }
        }
    }

    if need.is_empty() || token.is_cancelled() {
        return;
    }

    let checker = match dns.checker(pool).await {
        Ok(checker) => checker,
        Err(e) => {
            tracing::warn!(error = ?e, "identity warm: no DNS resolver");
            return;
        }
    };
    let resolved = resolve_ptr_evidence_many(&checker, &need).await;
    let mut checked = 0usize;
    for (ip, evidence) in &resolved {
        if let Err(e) = persist_evidence(pool, *ip, evidence).await {
            tracing::warn!(error = %e, %ip, "identity warm: persist failed");
            continue;
        }
        checked += 1;
    }

    if checked > 0 {
        tracing::info!(
            checked,
            first_time,
            candidates = candidates.len(),
            "identity warm: refreshed snapshots"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker() -> DnsChecker {
        DnsChecker::new().unwrap()
    }

    fn snapshot(ptr: Option<&str>) -> IdentitySnapshot {
        IdentitySnapshot {
            ip: "203.0.113.1".into(),
            checked_at: 0,
            ptr_hostname: ptr.map(str::to_owned),
            ptr_confirmed: ptr.map(|_| true),
        }
    }

    #[test]
    fn freshness_windows_differ_for_positive_and_negative() {
        let positive = snapshot(Some("mail.example.com"));
        let negative = snapshot(None);
        assert!(snapshot_is_fresh(&positive, FRESH_SECS - 1));
        assert!(!snapshot_is_fresh(&positive, FRESH_SECS));
        assert!(snapshot_is_fresh(&negative, NEGATIVE_FRESH_SECS - 1));
        assert!(!snapshot_is_fresh(&negative, NEGATIVE_FRESH_SECS));
        // Past the positive window but inside the negative one.
        assert!(!snapshot_is_fresh(&positive, FRESH_SECS + 1));
        assert!(snapshot_is_fresh(&negative, FRESH_SECS + 1));
    }

    /// No network: every address here is outside public space.
    #[tokio::test]
    async fn non_public_addresses_are_never_looked_up() {
        for raw in ["127.0.0.1", "10.0.0.5", "192.0.2.1", "::1", "fd00::1"] {
            let evidence = resolve_ptr_evidence(&checker(), raw.parse().unwrap()).await;
            assert_eq!(
                evidence,
                Some(PtrEvidence::default()),
                "{raw} must not be resolved"
            );
        }
    }

    #[tokio::test]
    async fn batch_resolution_dedupes_repeated_ips() {
        let ips: Vec<IpAddr> = ["192.0.2.1", "192.0.2.2", "192.0.2.1"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let resolved = resolve_ptr_evidence_many(&Arc::new(checker()), &ips).await;
        assert_eq!(resolved.len(), 2, "duplicate IP resolved once");
    }
}
