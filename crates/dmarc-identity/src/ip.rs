//! Coarse address-range classification (RFC 1918, documentation, CGNAT, …).
//!
//! Only public addresses can have a public reverse-DNS name, so callers use
//! this to skip lookups that cannot answer.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Address range an IP falls in. IPv4-mapped IPv6 is classified as the inner
/// IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpKind {
    Public,
    Private,
    Loopback,
    LinkLocal,
    Shared,
    Documentation,
    Multicast,
    Reserved,
}

/// Classify `ip` into an [`IpKind`].
pub fn classify_ip(ip: IpAddr) -> IpKind {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                IpKind::Loopback
            } else if v4.is_private() {
                IpKind::Private
            } else if v4.is_link_local() {
                IpKind::LinkLocal
            } else if v4.is_multicast() {
                IpKind::Multicast
            } else if v4.is_broadcast() || v4.is_unspecified() {
                IpKind::Reserved
            } else if is_v4_cgnat(v4) {
                IpKind::Shared
            } else if is_v4_documentation(v4) {
                IpKind::Documentation
            } else if is_v4_reserved(v4) {
                IpKind::Reserved
            } else {
                IpKind::Public
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                IpKind::Loopback
            } else if v6.is_multicast() {
                IpKind::Multicast
            } else if v6.is_unspecified() {
                IpKind::Reserved
            } else if is_v6_unique_local(v6) {
                IpKind::Private
            } else if is_v6_link_local(v6) {
                IpKind::LinkLocal
            } else if is_v6_documentation(v6) {
                IpKind::Documentation
            } else {
                IpKind::Public
            }
        }
    }
}

/// Prefer the inner IPv4 when the address is IPv4-mapped (`::ffff:x.x.x.x`).
fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

fn is_v4_cgnat(v4: Ipv4Addr) -> bool {
    v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 64
}

fn is_v4_documentation(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    matches!(
        (o[0], o[1], o[2]),
        (192, 0, 2) | (198, 51, 100) | (203, 0, 113)
    )
}

fn is_v4_reserved(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] >= 240 || o[0] == 0 || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
}

fn is_v6_unique_local(v6: Ipv6Addr) -> bool {
    v6.segments()[0] & 0xfe00 == 0xfc00
}

fn is_v6_link_local(v6: Ipv6Addr) -> bool {
    v6.segments()[0] & 0xffc0 == 0xfe80
}

fn is_v6_documentation(v6: Ipv6Addr) -> bool {
    let s = v6.segments();
    s[0] == 0x2001 && s[1] == 0x0db8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(s: &str) -> IpKind {
        classify_ip(s.parse().unwrap())
    }

    #[test]
    fn classifies_ranges() {
        assert_eq!(kind("40.107.243.75"), IpKind::Public);
        assert_eq!(kind("192.168.1.20"), IpKind::Private);
        assert_eq!(kind("127.0.0.1"), IpKind::Loopback);
        assert_eq!(kind("100.64.0.1"), IpKind::Shared);
        assert_eq!(kind("203.0.113.50"), IpKind::Documentation);
        assert_eq!(kind("240.0.0.1"), IpKind::Reserved);
        assert_eq!(kind("::1"), IpKind::Loopback);
        assert_eq!(kind("fd00::1"), IpKind::Private);
        assert_eq!(kind("fe80::1"), IpKind::LinkLocal);
        assert_eq!(kind("2001:db8::1"), IpKind::Documentation);
        assert_eq!(kind("2a00:d70:0:e::321"), IpKind::Public);
    }

    #[test]
    fn ipv4_mapped_uses_inner_address() {
        assert_eq!(kind("::ffff:192.168.1.1"), IpKind::Private);
        assert_eq!(kind("::ffff:40.107.243.75"), IpKind::Public);
    }
}
