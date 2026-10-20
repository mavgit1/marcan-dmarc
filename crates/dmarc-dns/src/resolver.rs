//! What the checks ask of DNS, as a trait: TXT, PTR and A/AAAA lookups with
//! failures as data. [`DnsClient`](crate::dns::DnsClient) answers from the
//! configured nameservers; a [`TableResolver`] answers at once from a fixed
//! table, for tests and for the demo database.

use crate::dns::{ForwardLookup, PtrLookup, ResolveFailure};
use crate::lookup::{LookupStatus, TxtAnswer, TxtLookup};
use dmarc_domain::normalize_name;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

/// DNS lookups for the checks. Every failure is a value (timeout, SERVFAIL,
/// NXDOMAIN), never an error.
pub trait Resolver: Send + Sync {
    /// TXT records at `name`.
    fn txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, TxtLookup>;
    /// PTR names of `ip`.
    fn lookup_ptr(&self, ip: IpAddr) -> BoxFuture<'_, PtrLookup>;
    /// A and AAAA addresses of `hostname`.
    fn lookup_forward<'a>(&'a self, hostname: &'a str) -> BoxFuture<'a, ForwardLookup>;
    /// Drop every cached answer, negative ones included.
    fn clear_cache(&self);
    /// The nameservers, for log lines ("1.1.1.1, 8.8.8.8").
    fn describe(&self) -> &str;
}

/// The MTA-STS policy fetch resolves its host through the checks' own
/// [`Resolver`], so a policy host is judged by the resolvers the user
/// configured.
///
/// Domains come from incoming reports, so anyone can make Marcan fetch
/// `https://mta-sts.<a name they control>/`. A host with any address that
/// [`is_private_address`] is refused here, before a connection is made. The
/// HTTP client connects to exactly the addresses returned (no lookup of its
/// own, no proxy; the URL's name is used only for SNI and the certificate
/// check), so the name cannot be rebound between this check and the connect.
pub(crate) struct HttpResolve(pub(crate) Arc<dyn Resolver>);

impl reqwest::dns::Resolve for HttpResolve {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = Arc::clone(&self.0);
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let forward = resolver.lookup_forward(&host).await;
            let failure = match forward.lookup {
                LookupStatus::Ok if !forward.addresses.is_empty() => {
                    // One internal address is enough to refuse: the client
                    // could try any of them.
                    if let Some(&ip) = forward.addresses.iter().find(|ip| is_private_address(**ip))
                    {
                        let failure = ResolveFailure::PrivateAddress { host, ip };
                        return Err(Box::new(failure) as Box<dyn std::error::Error + Send + Sync>);
                    }
                    // The HTTP client replaces the port with the URL's.
                    let addrs: reqwest::dns::Addrs = Box::new(
                        forward
                            .addresses
                            .into_iter()
                            .map(|ip| std::net::SocketAddr::new(ip, 0)),
                    );
                    return Ok(addrs);
                }
                LookupStatus::Ok | LookupStatus::Nxdomain | LookupStatus::NoRecords => {
                    ResolveFailure::NotFound(host)
                }
                LookupStatus::Timeout => ResolveFailure::Timeout(host),
                LookupStatus::Servfail => ResolveFailure::ServFail {
                    host,
                    detail: forward.lookup_detail.unwrap_or_default(),
                },
            };
            Err(Box::new(failure) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

/// Whether `ip` is an address Marcan must not fetch from: loopback, private
/// (RFC 1918), shared (RFC 6598, carrier-grade NAT), link-local (with the
/// cloud metadata service at 169.254.169.254), unique-local (`fc00::/7`) and
/// the retired site-local (`fec0::/10`), multicast, broadcast, unspecified and "this network" (0.0.0.0/8). An
/// IPv6 address that carries an IPv4 address (mapped, compatible, or behind
/// the NAT64 prefix `64:ff9b::/96`) is judged as that IPv4 address.
pub(crate) fn is_private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                // ::ffff:a.b.c.d and the deprecated ::a.b.c.d; `::` and `::1`
                // are caught below.
                if !v6.is_unspecified() && !v6.is_loopback() {
                    return is_private_v4(v4);
                }
            }
            if let [0x64, 0xff9b, 0, 0, 0, 0, hi, lo] = v6.segments() {
                return is_private_v4(Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo)));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_unique_local(v6)
                || is_unicast_link_local(v6)
                || is_site_local(v6)
        }
    }
}

/// The cloud instance metadata service (AWS, GCP, Azure, OpenStack...).
const METADATA_ADDRESS: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);

fn is_private_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    v4 == METADATA_ADDRESS
        || v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || a == 0
        // 100.64.0.0/10, shared address space (RFC 6598).
        || (a == 100 && (b & 0xc0) == 64)
}

/// `fc00::/7` (RFC 4193).
fn is_unique_local(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

/// `fe80::/10`.
fn is_unicast_link_local(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

/// `fec0::/10`, deprecated by RFC 3879 but still internal where used.
fn is_site_local(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfec0
}

/// A [`Resolver`] that answers TXT lookups at once from a table: the
/// records given with [`Self::with_txt`] (a `*.` entry answers the names below it
/// that have none of their own, as a DNS wildcard does), and the same answer
/// for every other name (NXDOMAIN from [`Self::new`], a timeout from
/// [`Self::failing`]). PTR and A/AAAA lookups get that answer too, unless
/// [`Self::with_address_lookups`] passes them to a real resolver; a name
/// given addresses with [`Self::with_addresses`] answers with those.
///
/// Tests use it so no lookup waits on, or depends on, the network; the demo
/// database's domains use it so their DNS matches their made-up reports.
#[derive(Clone)]
pub struct TableResolver {
    txt: HashMap<String, TxtLookup>,
    otherwise: TxtLookup,
    addresses: Option<Arc<dyn Resolver>>,
    forward: HashMap<String, Vec<IpAddr>>,
}

impl Default for TableResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl TableResolver {
    /// Nothing is published anywhere: every name is NXDOMAIN.
    pub fn new() -> Self {
        Self {
            txt: HashMap::new(),
            otherwise: TxtLookup::NxDomain,
            addresses: None,
            forward: HashMap::new(),
        }
    }

    /// A resolver nobody answers on: every lookup times out, at once.
    pub fn failing() -> Self {
        Self {
            otherwise: TxtLookup::Timeout,
            ..Self::new()
        }
    }

    /// Answer `name` with these TXT records.
    pub fn with_txt(mut self, name: &str, records: &[&str]) -> Self {
        let records = records.iter().map(|r| (*r).to_owned()).collect();
        self.txt.insert(
            normalize_name(name),
            TxtLookup::Records(TxtAnswer::from_records(records)),
        );
        self
    }

    /// Answer `name` with `lookup` (a timeout for one name, say).
    pub fn with_answer(mut self, name: &str, lookup: TxtLookup) -> Self {
        self.txt.insert(normalize_name(name), lookup);
        self
    }

    /// Answer A/AAAA lookups of `name` with `addresses`.
    pub fn with_addresses(mut self, name: &str, addresses: &[IpAddr]) -> Self {
        self.forward
            .insert(normalize_name(name), addresses.to_vec());
        self
    }

    /// Look PTR and A/AAAA names up through `resolver` instead.
    pub fn with_address_lookups(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.addresses = Some(resolver);
        self
    }

    /// The table's answer for `name`: its own entry, else the nearest
    /// wildcard above it, else the answer for every other name.
    fn lookup(&self, name: &str) -> TxtLookup {
        let name = normalize_name(name);
        if let Some(answer) = self.txt.get(&name) {
            return answer.clone();
        }
        let mut rest = name.as_str();
        while let Some((_, parent)) = rest.split_once('.') {
            if let Some(answer) = self.txt.get(&format!("*.{parent}")) {
                return answer.clone();
            }
            rest = parent;
        }
        self.otherwise.clone()
    }
}

impl Resolver for TableResolver {
    fn txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, TxtLookup> {
        let answer = self.lookup(name);
        Box::pin(async move { answer })
    }

    fn lookup_ptr(&self, ip: IpAddr) -> BoxFuture<'_, PtrLookup> {
        if let Some(resolver) = &self.addresses {
            return resolver.lookup_ptr(ip);
        }
        let lookup = self.otherwise.status();
        Box::pin(async move {
            PtrLookup {
                ip: ip.to_string(),
                lookup,
                hostnames: Vec::new(),
                lookup_detail: None,
            }
        })
    }

    fn lookup_forward<'a>(&'a self, hostname: &'a str) -> BoxFuture<'a, ForwardLookup> {
        if let Some(addresses) = self.forward.get(&normalize_name(hostname)) {
            let answer = ForwardLookup {
                hostname: normalize_name(hostname),
                lookup: LookupStatus::Ok,
                addresses: addresses.clone(),
                lookup_detail: None,
            };
            return Box::pin(async move { answer });
        }
        if let Some(resolver) = &self.addresses {
            return resolver.lookup_forward(hostname);
        }
        let lookup = self.otherwise.status();
        Box::pin(async move {
            ForwardLookup {
                hostname: normalize_name(hostname),
                lookup,
                addresses: Vec::new(),
                lookup_detail: None,
            }
        })
    }

    fn clear_cache(&self) {
        if let Some(resolver) = &self.addresses {
            resolver.clear_cache();
        }
    }

    fn describe(&self) -> &'static str {
        "a fixed table"
    }
}

/// A resolver over the public default nameservers (Cloudflare, then
/// Google), for lookups a [`TableResolver`] passes on.
pub fn default_resolver() -> Result<Arc<dyn Resolver>, crate::DnsError> {
    Ok(Arc::new(crate::dns::DnsClient::new()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_addresses_are_private_and_public_ones_are_not() {
        for ip in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "169.254.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd00:ec2::254",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::10.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(
                is_private_address(ip.parse().unwrap()),
                "{ip} must be refused"
            );
        }
        for ip in [
            "1.1.1.1",
            "192.0.2.1",
            "172.32.0.1",
            "100.128.0.1",
            "169.255.0.1",
            "2001:4860:4860::8888",
            "2001:db8::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(!is_private_address(ip.parse().unwrap()), "{ip} is public");
        }
    }

    #[tokio::test]
    async fn the_table_answers_address_lookups_it_was_given() {
        let ip: IpAddr = "192.0.2.7".parse().unwrap();
        let table = TableResolver::new().with_addresses("MTA-STS.example.com.", &[ip]);
        let found = table.lookup_forward("mta-sts.example.com").await;
        assert_eq!(found.lookup, LookupStatus::Ok);
        assert_eq!(found.addresses, [ip]);
        assert_eq!(
            table.lookup_forward("other.example.com").await.lookup,
            LookupStatus::Nxdomain
        );
    }

    #[tokio::test]
    async fn the_table_answers_its_names_wildcards_and_the_rest() {
        let table = TableResolver::new()
            .with_txt("_dmarc.Example.com.", &["v=DMARC1; p=reject"])
            .with_txt("*._report._dmarc.example.com", &["v=DMARC1"])
            .with_txt("own._report._dmarc.example.com", &["v=DMARC1; own"]);
        assert_eq!(
            table.txt("_dmarc.example.com").await.records(),
            ["v=DMARC1; p=reject"]
        );
        assert_eq!(
            table
                .txt("example.net._report._dmarc.example.com")
                .await
                .records(),
            ["v=DMARC1"]
        );
        assert_eq!(
            table.txt("own._report._dmarc.example.com").await.records(),
            ["v=DMARC1; own"]
        );
        assert_eq!(table.txt("example.org").await, TxtLookup::NxDomain);
        assert_eq!(
            table.lookup_forward("mta-sts.example.com").await.lookup,
            LookupStatus::Nxdomain
        );
        let failing = TableResolver::failing();
        assert_eq!(failing.txt("example.org").await, TxtLookup::Timeout);
        assert_eq!(
            failing
                .lookup_ptr("192.0.2.1".parse().unwrap())
                .await
                .lookup,
            LookupStatus::Timeout
        );
    }
}
