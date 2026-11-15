//! New failing senders by network: an address is no new sender when its
//! network already sent as the domain, and several new addresses of one
//! network are one sender ([`new_senders_by_network`], [`sender_network`]).
//!
//! Large senders send from many neighbouring addresses, so an exact
//! address is too fine a key: the network is a /24 for IPv4 and a /48 for
//! IPv6, the smallest blocks routed on the internet. An IPv4-mapped IPv6
//! address is its IPv4 address.

use super::state::NewSender;
use std::collections::BTreeSet;
use std::net::IpAddr;

/// The network a source address belongs to for the new-sender rule; an
/// address that does not parse is its own network (the exact string).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SenderNetwork {
    /// The first three octets (a /24).
    V4([u8; 3]),
    /// The first three 16-bit groups (a /48).
    V6([u16; 3]),
    /// Not an address: compared as written.
    Unparsed(String),
}

/// `ip`'s network: its /24 (IPv4) or /48 (IPv6).
pub(super) fn sender_network(ip: &str) -> SenderNetwork {
    let v4 = |o: [u8; 4]| SenderNetwork::V4([o[0], o[1], o[2]]);
    match ip.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) => v4(a.octets()),
        Ok(IpAddr::V6(a)) => match a.to_ipv4_mapped() {
            Some(m) => v4(m.octets()),
            None => {
                let s = a.segments();
                SenderNetwork::V6([s[0], s[1], s[2]])
            }
        },
        Err(_) => SenderNetwork::Unparsed(ip.to_owned()),
    }
}

/// The senders of `candidates` (addresses new to a domain in the window, as
/// the store finds them) whose network sent nothing as the domain before
/// the window: `sent_before` are the addresses that did. Pass the result
/// to [`alert_state`](super::alert_state), which keeps one address per
/// network.
pub fn new_senders_by_network<'a>(
    candidates: Vec<NewSender>,
    sent_before: impl IntoIterator<Item = &'a str>,
) -> Vec<NewSender> {
    let old: BTreeSet<SenderNetwork> = sent_before.into_iter().map(sender_network).collect();
    candidates
        .into_iter()
        .filter(|s| !old.contains(&sender_network(&s.source_ip)))
        .collect()
}

/// One sender per network: the address with the most failures (the lowest
/// address on a tie) stands for the others of its network.
pub(super) fn one_per_network(senders: Vec<NewSender>) -> Vec<NewSender> {
    let mut by_network: Vec<(SenderNetwork, NewSender)> = senders
        .into_iter()
        .map(|s| (sender_network(&s.source_ip), s))
        .collect();
    by_network.sort_by(|(na, a), (nb, b)| {
        na.cmp(nb)
            .then(b.failed.cmp(&a.failed))
            .then(a.source_ip.cmp(&b.source_ip))
    });
    by_network.dedup_by(|(later, _), (first, _)| later == first);
    by_network.into_iter().map(|(_, s)| s).collect()
}
