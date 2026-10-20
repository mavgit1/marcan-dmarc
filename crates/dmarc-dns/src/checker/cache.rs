//! The checker's five-minute cache: what it keeps ([`Cached`], one variant
//! per result), which results are worth keeping ([`CacheEntry`]), the keys
//! ([`CacheKey`]), and the lookup and invalidation on [`DnsChecker`].

use super::{DnsChecker, DnsReport};
use crate::dkim::DkimCheckResult;
use crate::dmarc::DmarcCheckResult;
use crate::lookup::LookupStatus;
use crate::policy::{BimiCheckResult, MtaStsCheckResult, TlsRptCheckResult};
use crate::spf::SpfCheckResult;
use crate::PtrLookup;
use dmarc_assess::{assign_points, CheckKind, Finding};
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;

/// What the checker remembers for five minutes, keyed by [`CacheKey`].
#[derive(Clone)]
pub(super) enum Cached {
    Report(Arc<DnsReport>),
    Dmarc(Arc<DmarcCheckResult>),
    Spf(Arc<SpfCheckResult>),
    Dkim(Arc<DkimCheckResult>),
    MtaSts(Arc<MtaStsCheckResult>),
    TlsRpt(Arc<TlsRptCheckResult>),
    Bimi(Arc<BimiCheckResult>),
    Ptr(Arc<PtrLookup>),
}

/// A result the checker can cache: how it is stored in [`Cached`], whether it
/// is worth keeping (a measurement, not a timeout), and where its findings
/// live so their points are assigned once, right after the check.
pub(super) trait CacheEntry: Clone {
    fn wrap(entry: Arc<Self>) -> Cached;
    fn unwrap(cached: Cached) -> Option<Arc<Self>>;
    fn cacheable(&self) -> bool;
    fn findings_mut(&mut self) -> Option<&mut Vec<Finding>> {
        None
    }
}

macro_rules! cache_entry {
    ($variant:ident, $ty:ty, |$r:ident| $cacheable:expr $(, $findings:ident)?) => {
        impl CacheEntry for $ty {
            fn wrap(entry: Arc<Self>) -> Cached {
                Cached::$variant(entry)
            }
            fn unwrap(cached: Cached) -> Option<Arc<Self>> {
                match cached {
                    Cached::$variant(v) => Some(v),
                    _ => None,
                }
            }
            fn cacheable(&self) -> bool {
                let $r = self;
                $cacheable
            }
            $(fn findings_mut(&mut self) -> Option<&mut Vec<Finding>> {
                Some(&mut self.$findings)
            })?
        }
    };
}

cache_entry!(Report, DnsReport, |r| r.is_complete());
cache_entry!(
    Dmarc,
    DmarcCheckResult,
    |r| r.lookup.is_measured(),
    findings
);
cache_entry!(Spf, SpfCheckResult, |r| r.is_complete(), findings);
// Nothing requested, or at least one selector answered.
cache_entry!(
    Dkim,
    DkimCheckResult,
    |r| r.records.is_empty() || r.records.iter().any(|s| s.lookup.is_measured()),
    findings
);
cache_entry!(
    MtaSts,
    MtaStsCheckResult,
    |r| r.txt_lookup != LookupStatus::Timeout,
    findings
);
cache_entry!(
    TlsRpt,
    TlsRptCheckResult,
    |r| r.lookup != LookupStatus::Timeout,
    findings
);
cache_entry!(
    Bimi,
    BimiCheckResult,
    |r| r.lookup != LookupStatus::Timeout,
    findings
);
cache_entry!(Ptr, PtrLookup, |r| r.lookup != LookupStatus::Timeout);

/// What a cache entry is about. Rendered as `check:domain[:suffix]`;
/// [`DnsChecker::invalidate_domain`] matches on the `check:domain` prefix.
#[derive(Debug, Clone, Copy)]
pub(super) enum CacheKey<'a> {
    /// The full report for a domain, per DKIM probe.
    All(&'a str, &'a str),
    /// One check for a domain; DKIM carries the probe as suffix.
    Check(CheckKind, &'a str, &'a str),
    Ptr(IpAddr),
}

impl CacheKey<'_> {
    pub(super) fn check(kind: CheckKind, domain: &str) -> CacheKey<'_> {
        CacheKey::Check(kind, domain, "")
    }

    pub(super) fn render(self) -> String {
        match self {
            Self::All(domain, probe) => format!("all:{domain}:{probe}"),
            Self::Check(CheckKind::Dkim, domain, probe) => format!("dkim:{domain}:{probe}"),
            Self::Check(kind, domain, _) => format!("{}:{domain}", kind.as_str()),
            Self::Ptr(ip) => format!("ptr:{ip}"),
        }
    }
}

impl DnsChecker {
    /// Invalidate in-memory cache entries for `domain`.
    pub async fn invalidate_domain(&self, domain: &str) {
        // `all:` and `dkim:` keys carry a selector suffix, so match by prefix.
        let all_prefix = CacheKey::All(domain, "").render();
        let dkim_prefix = CacheKey::check(CheckKind::Dkim, domain).render();
        if let Err(e) = self.cache.invalidate_entries_if(move |k, _| {
            k.starts_with(&all_prefix) || k.starts_with(&dkim_prefix)
        }) {
            tracing::warn!(domain, error = %e, "failed to invalidate cached DNS reports");
        }
        for kind in [
            CheckKind::Dmarc,
            CheckKind::Spf,
            CheckKind::MtaSts,
            CheckKind::TlsRpt,
            CheckKind::Bimi,
        ] {
            self.cache
                .invalidate(&CacheKey::check(kind, domain).render())
                .await;
        }
    }

    /// One cached probe: answer from the cache, else run `work`, assign the
    /// points its findings cost, and keep the result when it is a measurement
    /// ([`CacheEntry::cacheable`]).
    pub(super) async fn cached<R: CacheEntry>(
        &self,
        key: CacheKey<'_>,
        work: impl Future<Output = R>,
    ) -> R {
        let key = key.render();
        if let Some(hit) = self.cache.get(&key).await.and_then(R::unwrap) {
            return (*hit).clone();
        }
        let mut result = work.await;
        if let Some(findings) = result.findings_mut() {
            assign_points(findings);
        }
        if result.cacheable() {
            self.cache
                .insert(key, R::wrap(Arc::new(result.clone())))
                .await;
        }
        result
    }
}
