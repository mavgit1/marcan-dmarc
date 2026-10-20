//! Async DNS TXT and PTR lookups via hickory-resolver, with per-query timeouts.

use crate::lookup::{LookupStatus, TxtAnswer, TxtLookup};
use dmarc_domain::normalize_name;
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError as WireError, NetError};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::proto::rr::RData;
use hickory_resolver::TokioResolver;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
use ts_rs::TS;

/// One attempt at one TXT name. Short on purpose: a resolver that has not
/// answered in a few seconds is better asked again than waited on.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);

/// Outer guard around one resolver call (hickory's own timeout is
/// [`ATTEMPT_TIMEOUT`]; this only catches a call that never returns).
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Pauses before the 2nd and 3rd attempt of a TXT lookup that timed out or
/// SERVFAILed: three attempts in all, each pause longer than the last.
pub(crate) const TXT_RETRY_PAUSES: [Duration; 2] =
    [Duration::from_millis(500), Duration::from_millis(1500)];

/// Entries kept in the resolver's answer cache, per record type.
///
/// Large enough for repeated PTR and forward lookups across many sender IPs plus
/// the TXT records of the monitored domains. Entries are small; TTLs still
/// govern correctness.
const RESOLVER_CACHE_ENTRIES: u64 = 4096;

#[derive(Debug, Error)]
pub enum DnsError {
    #[error("failed to initialize DNS resolver: {0}")]
    Init(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IpError {
    #[error("ip must not be empty")]
    Empty,
    #[error("ip is not a valid IPv4 or IPv6 address")]
    Invalid,
}

/// Reverse-DNS result. Lookup failures are data, never a hard error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PtrLookup {
    pub ip: String,
    pub lookup: LookupStatus,
    pub hostnames: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lookup_detail: Option<String>,
}

/// Forward A/AAAA result for a hostname. Failures are data, never a hard error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardLookup {
    pub hostname: String,
    pub lookup: LookupStatus,
    pub addresses: Vec<IpAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup_detail: Option<String>,
}

/// Parse a user-supplied IPv4 or IPv6 address (optional `[...]` around IPv6).
pub fn parse_ip(input: &str) -> Result<IpAddr, IpError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(IpError::Empty);
    }
    let s = trimmed
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(trimmed);
    s.parse().map_err(|_| IpError::Invalid)
}

/// Tokio DNS client used by live checks.
#[derive(Clone)]
pub struct DnsClient {
    resolver: TokioResolver,
    /// The configured nameservers, for log lines ("1.1.1.1, 8.8.8.8").
    resolvers: Arc<str>,
}

impl DnsClient {
    /// Client using the public default resolvers (Cloudflare, then Google).
    pub fn new() -> Result<Self, DnsError> {
        Self::with_nameservers(&[IpAddr::from([1, 1, 1, 1]), IpAddr::from([8, 8, 8, 8])])
    }

    pub fn with_nameservers(ips: &[IpAddr]) -> Result<Self, DnsError> {
        if ips.is_empty() {
            return Err(DnsError::Init("nameservers list must not be empty".into()));
        }
        let config = ResolverConfig::from_parts(None, vec![], nameservers(ips));
        let mut builder =
            TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
        {
            let opts = builder.options_mut();
            opts.timeout = ATTEMPT_TIMEOUT;
            // Retries are ours (`retry_txt`), with pauses and a log line each.
            opts.attempts = 1;
            opts.num_concurrent_reqs = 32;
            opts.cache_size = RESOLVER_CACHE_ENTRIES;
            // Advertise a large UDP payload so most answers (2048-bit DKIM keys
            // included) fit in one datagram; anything still truncated retries
            // over the TCP connection configured next to each UDP one.
            opts.edns0 = true;
        }
        let resolvers = ips
            .iter()
            .map(IpAddr::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        Ok(Self {
            resolver: builder.build().map_err(|e| DnsError::Init(e.to_string()))?,
            resolvers: resolvers.into(),
        })
    }

    /// Drop every cached answer, negative ones included.
    pub fn clear_cache(&self) {
        self.resolver.clear_cache();
    }

    /// The configured nameservers, comma-separated (for logs).
    pub fn resolvers(&self) -> &str {
        &self.resolvers
    }

    /// Look up TXT records at `name`, retrying a timeout or SERVFAIL up to
    /// <code>[TXT_RETRY_PAUSES].len()</code> more times. Failures become [`TxtLookup`]
    /// variants; every failed attempt is logged at warn.
    pub async fn txt(&self, name: &str) -> TxtLookup {
        retry_txt(name, &self.resolvers, &TXT_RETRY_PAUSES, || {
            self.txt_once(name)
        })
        .await
    }

    /// One TXT attempt, no retry.
    async fn txt_once(&self, name: &str) -> TxtLookup {
        let fqdn = if name.ends_with('.') {
            name.to_string()
        } else {
            format!("{name}.")
        };
        match timeout(QUERY_TIMEOUT, self.resolver.txt_lookup(fqdn)).await {
            Err(_) => TxtLookup::Timeout,
            Ok(Err(err)) => classify_resolve(&err),
            Ok(Ok(lookup)) => {
                let mut records = Vec::new();
                let mut string_lengths = Vec::new();
                for txt in lookup.answers().iter().filter_map(|r| match &r.data {
                    RData::TXT(txt) => Some(txt),
                    _ => None,
                }) {
                    let mut concat = String::new();
                    let mut lens = Vec::new();
                    for chunk in &txt.txt_data {
                        lens.push(chunk.len());
                        concat.push_str(&String::from_utf8_lossy(chunk));
                    }
                    records.push(concat);
                    string_lengths.push(lens);
                }
                if records.is_empty() {
                    TxtLookup::NoRecords
                } else {
                    TxtLookup::Records(TxtAnswer {
                        records,
                        string_lengths,
                    })
                }
            }
        }
    }

    /// PTR lookup for `ip`. NXDOMAIN/timeout/SERVFAIL become [`PtrLookup::lookup`].
    pub async fn lookup_ptr(&self, ip: IpAddr) -> PtrLookup {
        match timeout(QUERY_TIMEOUT, self.resolver.reverse_lookup(ip)).await {
            Err(_) => PtrLookup {
                ip: ip.to_string(),
                lookup: LookupStatus::Timeout,
                hostnames: Vec::new(),
                lookup_detail: None,
            },
            Ok(Err(err)) => {
                let classified = classify_resolve(&err);
                PtrLookup {
                    ip: ip.to_string(),
                    lookup: classified.status(),
                    hostnames: Vec::new(),
                    lookup_detail: classified.detail().map(str::to_string),
                }
            }
            Ok(Ok(rev)) => {
                let mut hostnames: Vec<String> = rev
                    .answers()
                    .iter()
                    .filter_map(|r| match &r.data {
                        RData::PTR(name) => Some(normalize_name(&name.to_utf8())),
                        _ => None,
                    })
                    .filter(|h| !h.is_empty())
                    .collect();
                hostnames.sort();
                hostnames.dedup();
                let lookup = if hostnames.is_empty() {
                    LookupStatus::NoRecords
                } else {
                    LookupStatus::Ok
                };
                PtrLookup {
                    ip: ip.to_string(),
                    lookup,
                    hostnames,
                    lookup_detail: None,
                }
            }
        }
    }

    /// Resolve A and AAAA for `hostname`. Empty / NXDOMAIN / timeout become
    /// [`ForwardLookup::lookup`] status variants; addresses is empty then.
    pub async fn lookup_forward(&self, hostname: &str) -> ForwardLookup {
        let host = normalize_name(hostname);
        if host.is_empty() || host.parse::<IpAddr>().is_ok() {
            return ForwardLookup {
                hostname: host,
                lookup: LookupStatus::NoRecords,
                addresses: Vec::new(),
                lookup_detail: None,
            };
        }
        let fqdn = format!("{host}.");
        match timeout(QUERY_TIMEOUT, self.resolver.lookup_ip(fqdn)).await {
            Err(_) => ForwardLookup {
                hostname: host,
                lookup: LookupStatus::Timeout,
                addresses: Vec::new(),
                lookup_detail: None,
            },
            Ok(Err(err)) => {
                let classified = classify_resolve(&err);
                ForwardLookup {
                    hostname: host,
                    lookup: classified.status(),
                    addresses: Vec::new(),
                    lookup_detail: classified.detail().map(str::to_string),
                }
            }
            Ok(Ok(lookup)) => {
                let mut addresses: Vec<IpAddr> = lookup.iter().collect();
                addresses.sort();
                addresses.dedup();
                let status = if addresses.is_empty() {
                    LookupStatus::NoRecords
                } else {
                    LookupStatus::Ok
                };
                ForwardLookup {
                    hostname: host,
                    lookup: status,
                    addresses,
                    lookup_detail: None,
                }
            }
        }
    }
}

/// Why [`DnsClient`] could not give the HTTP client an address for a host.
/// Typed so the MTA-STS fetch can tell its own lookup failures apart from
/// connection and TLS errors without reading error messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveFailure {
    /// NXDOMAIN, or no A/AAAA records.
    NotFound(String),
    /// The lookup did not finish: nothing was measured.
    Timeout(String),
    /// SERVFAIL (or similar) from the nameserver.
    ServFail { host: String, detail: String },
    /// The name points at an address that is not on the public internet
    /// (see [`crate::resolver::is_private_address`]): refused, never dialled.
    PrivateAddress { host: String, ip: IpAddr },
}

impl std::fmt::Display for ResolveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(host) => write!(f, "{host} does not resolve"),
            Self::Timeout(host) => write!(f, "DNS lookup for {host} timed out"),
            Self::ServFail { host, detail } => write!(f, "DNS lookup for {host} failed ({detail})"),
            Self::PrivateAddress { host, ip } => {
                write!(f, "{host} resolves to the private address {ip}")
            }
        }
    }
}

impl std::error::Error for ResolveFailure {}

impl crate::resolver::Resolver for DnsClient {
    fn txt<'a>(&'a self, name: &'a str) -> futures::future::BoxFuture<'a, TxtLookup> {
        Box::pin(Self::txt(self, name))
    }

    fn lookup_ptr(&self, ip: IpAddr) -> futures::future::BoxFuture<'_, PtrLookup> {
        Box::pin(Self::lookup_ptr(self, ip))
    }

    fn lookup_forward<'a>(
        &'a self,
        hostname: &'a str,
    ) -> futures::future::BoxFuture<'a, ForwardLookup> {
        Box::pin(Self::lookup_forward(self, hostname))
    }

    fn clear_cache(&self) {
        Self::clear_cache(self);
    }

    fn describe(&self) -> &str {
        self.resolvers()
    }
}

impl ForwardLookup {
    /// Whether this forward result confirms that `hostname` points at `ip`.
    pub fn confirms(&self, ip: IpAddr) -> bool {
        self.lookup == LookupStatus::Ok && self.addresses.contains(&ip)
    }
}

/// Run `attempt` until it yields a measured answer (records, NXDOMAIN, no
/// records), at most `pauses.len() + 1` times, sleeping `pauses[i]` before
/// retry `i + 1`. Each timeout / SERVFAIL is logged at warn with the queried
/// name and the resolvers, so a log reader can tell their network, the
/// resolver and a provider's nameservers apart.
pub(crate) async fn retry_txt<F, Fut>(
    name: &str,
    resolvers: &str,
    pauses: &[Duration],
    mut attempt: F,
) -> TxtLookup
where
    F: FnMut() -> Fut,
    Fut: Future<Output = TxtLookup>,
{
    let attempts = pauses.len() + 1;
    let mut n = 0;
    loop {
        let answer = attempt().await;
        n += 1;
        if answer.is_measured() {
            if n > 1 {
                tracing::info!(
                    name,
                    resolvers,
                    attempt = n,
                    "dns lookup succeeded after retry"
                );
            }
            return answer;
        }
        let outcome = match &answer {
            TxtLookup::Timeout => "timeout".to_string(),
            other => format!("servfail: {}", other.detail().unwrap_or("")),
        };
        if n >= attempts {
            tracing::warn!(
                name,
                resolvers,
                attempt = n,
                attempts,
                outcome = %outcome,
                "dns lookup failed, giving up"
            );
            return answer;
        }
        tracing::warn!(
            name,
            resolvers,
            attempt = n,
            attempts,
            outcome = %outcome,
            "dns lookup failed, retrying"
        );
        tokio::time::sleep(pauses[n - 1]).await;
    }
}

/// UDP and TCP on port 53 for every nameserver.
///
/// hickory retries a truncated (TC=1) UDP answer over TCP only when the pool
/// has stream connections. A UDP-only pool turns every answer larger than the
/// UDP payload (e.g. a CNAME'd 2048-bit DKIM key) into "received truncated
/// response", which surfaced as SERVFAIL.
fn nameservers(ips: &[IpAddr]) -> Vec<NameServerConfig> {
    ips.iter()
        .copied()
        .map(NameServerConfig::udp_and_tcp)
        .collect()
}

/// A resolver error as a lookup outcome. hickory reports a negative answer
/// as `NoRecordsFound` and distinguishes NXDOMAIN (the name does not exist)
/// from NODATA (it exists without this record type) by the response code; an
/// upstream SERVFAIL or REFUSED is its own `ResponseCode` error; a request
/// that hit the resolver's own deadline is a `Timeout`. Anything else is a
/// SERVFAIL with the error text as detail.
fn classify_resolve(err: &NetError) -> TxtLookup {
    match err {
        NetError::Dns(WireError::NoRecordsFound(no)) => match no.response_code {
            ResponseCode::NXDomain => TxtLookup::NxDomain,
            // A failed lookup, not a measured absence, or a resolver hiccup
            // would grade as "DMARC missing".
            code @ (ResponseCode::ServFail | ResponseCode::Refused) => TxtLookup::ServFail {
                detail: format!("upstream answered {code}"),
            },
            _ => TxtLookup::NoRecords,
        },
        NetError::Dns(WireError::ResponseCode(code)) => TxtLookup::ServFail {
            detail: format!("upstream answered {code}"),
        },
        NetError::Timeout => TxtLookup::Timeout,
        NetError::Io(io) if io.kind() == ErrorKind::TimedOut => TxtLookup::Timeout,
        // Everything else, and whatever a later hickory adds (the enums are
        // `#[non_exhaustive]`).
        _ => TxtLookup::ServFail {
            detail: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::config::ProtocolConfig;
    use hickory_resolver::net::NoRecords;
    use hickory_resolver::proto::op::Query;

    fn no_records(code: ResponseCode) -> NetError {
        NetError::Dns(WireError::NoRecordsFound(NoRecords::new(
            Query::new(),
            code,
        )))
    }

    #[test]
    fn resolver_errors_classify_by_response_code() {
        assert_eq!(
            classify_resolve(&no_records(ResponseCode::NXDomain)),
            TxtLookup::NxDomain
        );
        assert_eq!(
            classify_resolve(&no_records(ResponseCode::NoError)),
            TxtLookup::NoRecords
        );
        for code in [ResponseCode::ServFail, ResponseCode::Refused] {
            assert!(
                matches!(
                    classify_resolve(&no_records(code)),
                    TxtLookup::ServFail { .. }
                ),
                "{code} must be a failed lookup"
            );
        }
        assert!(matches!(
            classify_resolve(&NetError::Dns(WireError::ResponseCode(
                ResponseCode::ServFail
            ))),
            TxtLookup::ServFail { .. }
        ));
        assert_eq!(classify_resolve(&NetError::Timeout), TxtLookup::Timeout);
        assert_eq!(
            classify_resolve(&NetError::Io(Arc::new(std::io::Error::from(
                ErrorKind::TimedOut
            )))),
            TxtLookup::Timeout
        );
    }

    #[test]
    fn parse_ip_accepts_v4_v6_and_brackets() {
        assert_eq!(
            parse_ip("8.8.8.8").unwrap(),
            "8.8.8.8".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            parse_ip("  1.1.1.1  ").unwrap(),
            "1.1.1.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            parse_ip("2001:4860:4860::8888").unwrap(),
            "2001:4860:4860::8888".parse::<IpAddr>().unwrap()
        );
        assert_eq!(parse_ip("[::1]").unwrap(), "::1".parse::<IpAddr>().unwrap());
    }

    /// Every nameserver must be reachable over both UDP and TCP, or truncated
    /// UDP answers never get their TCP retry.
    #[test]
    fn every_nameserver_has_udp_and_tcp() {
        let ips: Vec<IpAddr> = vec!["1.1.1.1".parse().unwrap(), "2001:db8::53".parse().unwrap()];
        let servers = nameservers(&ips);
        assert_eq!(servers.len(), ips.len());
        for (ns, ip) in servers.iter().zip(&ips) {
            assert_eq!(ns.ip, *ip);
            assert!(ns.trust_negative_responses);
            for proto in [ProtocolConfig::Udp, ProtocolConfig::Tcp] {
                assert!(
                    ns.connections
                        .iter()
                        .any(|c| c.port == 53 && c.protocol == proto),
                    "{ip} is missing {proto:?}"
                );
            }
        }
    }

    /// Live: a CNAME'd 2048-bit DKIM key whose UDP answer is truncated must be
    /// read over TCP. Needs network, so run with `--ignored`.
    #[tokio::test]
    #[ignore = "queries public DNS"]
    async fn live_truncated_answer_retries_over_tcp() {
        let client = DnsClient::new().expect("resolver");
        let lookup = client.txt("selector1._domainkey.outlook.com").await;
        let TxtLookup::Records(answer) = &lookup else {
            panic!("expected records, got {lookup:?}");
        };
        let row = crate::dkim::evaluate_dkim_selector("outlook.com", "selector1", &lookup);
        assert!(answer.records.iter().any(|r| r.contains("v=DKIM1")));
        assert_eq!(row.key_bits, Some(2048), "{row:?}");
    }

    #[tokio::test]
    async fn retry_txt_retries_failures_with_growing_pauses_then_succeeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let started = tokio::time::Instant::now();
        let pauses = [Duration::from_millis(20), Duration::from_millis(60)];
        let answer = retry_txt("_dmarc.example.com", "192.0.2.53", &pauses, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                match n {
                    0 => TxtLookup::Timeout,
                    1 => TxtLookup::ServFail {
                        detail: "refused".into(),
                    },
                    _ => TxtLookup::NoRecords,
                }
            }
        })
        .await;
        assert_eq!(answer, TxtLookup::NoRecords);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(started.elapsed() >= Duration::from_millis(80));
    }

    #[tokio::test]
    async fn retry_txt_gives_up_after_the_last_attempt() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let pauses = [Duration::from_millis(1), Duration::from_millis(2)];
        let answer = retry_txt("slow.example.com", "192.0.2.53", &pauses, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { TxtLookup::Timeout }
        })
        .await;
        assert_eq!(answer, TxtLookup::Timeout);
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        // A measured answer (even "no such name") is never retried.
        let calls = AtomicUsize::new(0);
        let answer = retry_txt("gone.example.com", "192.0.2.53", &pauses, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { TxtLookup::NxDomain }
        })
        .await;
        assert_eq!(answer, TxtLookup::NxDomain);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parse_ip_rejects_empty_and_hostnames() {
        assert_eq!(parse_ip(""), Err(IpError::Empty));
        assert_eq!(parse_ip("   "), Err(IpError::Empty));
        assert_eq!(parse_ip("dns.google"), Err(IpError::Invalid));
        assert_eq!(parse_ip("8.8.8"), Err(IpError::Invalid));
        assert_eq!(parse_ip("not-an-ip"), Err(IpError::Invalid));
    }

    /// Pure confirmation logic: an IP is confirmed only when the forward
    /// lookup succeeded and the address set contains it. No network.
    #[test]
    fn forward_confirmation_requires_ip_in_successful_lookup() {
        let ip: IpAddr = "203.0.113.50".parse().unwrap();
        let other: IpAddr = "198.51.100.1".parse().unwrap();

        let ok = ForwardLookup {
            hostname: "mail.example.com".into(),
            lookup: LookupStatus::Ok,
            addresses: vec![ip, "2001:db8::1".parse().unwrap()],
            lookup_detail: None,
        };
        assert!(ok.confirms(ip));
        assert!(!ok.confirms(other));

        let empty_ok = ForwardLookup {
            hostname: "mail.example.com".into(),
            lookup: LookupStatus::Ok,
            addresses: Vec::new(),
            lookup_detail: None,
        };
        assert!(!empty_ok.confirms(ip));

        let nx = ForwardLookup {
            hostname: "missing.example.com".into(),
            lookup: LookupStatus::Nxdomain,
            addresses: vec![ip], // should not happen, but status gates confirmation
            lookup_detail: None,
        };
        assert!(!nx.confirms(ip));

        let timeout = ForwardLookup {
            hostname: "slow.example.com".into(),
            lookup: LookupStatus::Timeout,
            addresses: Vec::new(),
            lookup_detail: None,
        };
        assert!(!timeout.confirms(ip));
    }

    #[test]
    fn lookup_forward_rejects_empty_and_ip_literal_without_network() {
        // These short-circuit before any resolver call.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client =
            DnsClient::with_nameservers(&["192.0.2.53".parse().unwrap()]).expect("resolver config");
        let empty = rt.block_on(client.lookup_forward(""));
        assert_eq!(empty.lookup, LookupStatus::NoRecords);
        assert!(empty.addresses.is_empty());
        assert!(!empty.confirms("8.8.8.8".parse().unwrap()));

        let literal = rt.block_on(client.lookup_forward("8.8.8.8"));
        assert_eq!(literal.lookup, LookupStatus::NoRecords);
        assert!(literal.addresses.is_empty());
    }
}
