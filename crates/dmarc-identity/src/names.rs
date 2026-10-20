//! Display names for senders.
//!
//! Two layers:
//! 1. User overrides, kept by the caller (Marcan stores them in its database
//!    and edits them as text, see [`parse_overrides`] / [`format_overrides`]).
//!    One targets an IP, an IP range (CIDR) or a domain; the most specific
//!    match wins (an IP, then the narrowest range, then the sender's host name
//!    or its nearest parent, down to the operator domain).
//!    An empty name hides the name a less specific layer would give.
//! 2. The vendored parsedmarc map (`data/base_reverse_dns_map.csv`), ~38k
//!    entries keyed by reverse-DNS base domain. Only `name` is read; its
//!    `type` column is ignored on purpose.

use dmarc_domain::normalize_name;
use ipnet::IpNet;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::OnceLock;

/// parsedmarc's `base_reverse_dns,name,type` map, verbatim (see `data/README.md`).
const BUNDLED_MAP: &str = include_str!("../data/base_reverse_dns_map.csv");

fn bundled() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(BUNDLED_MAP.as_bytes());
        let mut map = HashMap::new();
        for record in reader.records() {
            let record = record.expect("vendored sender name map is valid CSV");
            let (Some(domain), Some(name)) = (record.get(0), record.get(1)) else {
                continue;
            };
            let (domain, name) = (domain.trim().to_ascii_lowercase(), name.trim());
            if !domain.is_empty() && !name.is_empty() {
                map.insert(domain, name.to_owned());
            }
        }
        map
    })
}

/// One user override: `name` for `target` (an IP, a range or an operator
/// domain, see [`parse_target`]), or `None` to hide the name it would get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameOverride {
    pub target: String,
    pub name: Option<String>,
}

/// The canonical form of an override target, or why it is not one: an IP
/// (`192.0.2.7`), a range (`192.0.2.0/24`, normalized to its network) or an
/// operator domain (`example.com`, lowercase).
pub fn parse_target(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_end_matches('.');
    if let Ok(ip) = raw.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    if let Ok(net) = raw.parse::<IpNet>() {
        return Ok(net.trunc().to_string());
    }
    let domain = raw.to_ascii_lowercase();
    if looks_like_domain(&domain) {
        Ok(domain)
    } else {
        Err(format!("not an IP, IP range or domain: {raw:?}"))
    }
}

/// `target` as a network when it is an IP or a range.
fn as_network(target: &str) -> Option<IpNet> {
    target
        .parse::<IpNet>()
        .or_else(|_| target.parse::<IpAddr>().map(IpNet::from))
        .ok()
}

/// A line of override text that could not be read (the API sends it as its
/// `LineError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideLineError {
    pub line: u64,
    pub message: String,
}

/// Parse override text: `target, name` per line, `#` comments, an optional
/// header row; further columns are ignored (so a parsedmarc-format file pastes
/// as is). Every bad line is reported, not just the first.
pub fn parse_overrides(text: &str) -> Result<Vec<NameOverride>, Vec<OverrideLineError>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for (index, record) in reader.records().enumerate() {
        let record = match record {
            Ok(record) => record,
            Err(e) => {
                let line = e.position().map_or(0, csv::Position::line);
                errors.push(OverrideLineError {
                    line,
                    message: e.to_string(),
                });
                continue;
            }
        };
        let line = record.position().map_or(0, csv::Position::line);
        let raw = record.get(0).unwrap_or_default();
        if index == 0
            && matches!(
                raw.to_ascii_lowercase().as_str(),
                "target" | "domain" | "base_reverse_dns"
            )
        {
            continue;
        }
        if raw.is_empty() && record.len() <= 1 {
            continue;
        }
        let target = match parse_target(raw) {
            Ok(target) if seen.insert(target.clone()) => target,
            Ok(target) => {
                errors.push(OverrideLineError {
                    line,
                    message: format!("{target} is listed twice"),
                });
                continue;
            }
            Err(message) => {
                errors.push(OverrideLineError { line, message });
                continue;
            }
        };
        let name = record.get(1).unwrap_or_default();
        out.push(NameOverride {
            target,
            name: (!name.is_empty()).then(|| name.to_owned()),
        });
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// `overrides` as text [`parse_overrides`] reads back: domains first, then
/// IPs and ranges, each sorted.
pub fn format_overrides(overrides: &[NameOverride]) -> String {
    let mut sorted: Vec<&NameOverride> = overrides.iter().collect();
    sorted.sort_by_key(|o| match as_network(&o.target) {
        None => (0, None, o.target.clone()),
        Some(net) => (1, Some((net.network(), net.prefix_len())), String::new()),
    });
    let mut writer = csv::WriterBuilder::new()
        .flexible(true)
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    for o in sorted {
        writer
            .write_record([o.target.as_str(), o.name.as_deref().unwrap_or_default()])
            .expect("writing to memory");
    }
    String::from_utf8(writer.into_inner().expect("writing to memory")).expect("UTF-8 in, UTF-8 out")
}

/// Name lookup: user overrides first, then the bundled map.
#[derive(Debug, Clone, Default)]
pub struct SenderNames {
    /// By operator domain; `None` hides the bundled name.
    domains: HashMap<String, Option<String>>,
    /// IPs and ranges, narrowest first, so the first containing one is the
    /// most specific; `None` hides any name.
    networks: Vec<(IpNet, Option<String>)>,
}

impl SenderNames {
    /// The bundled map only.
    pub fn bundled() -> Self {
        Self::default()
    }

    /// The bundled map with `overrides` on top. Targets that do not parse
    /// (see [`parse_target`]) are skipped.
    pub fn with_overrides(overrides: impl IntoIterator<Item = NameOverride>) -> Self {
        let mut names = Self::default();
        for o in overrides {
            let Ok(target) = parse_target(&o.target) else {
                continue;
            };
            match as_network(&target) {
                Some(net) => names.networks.push((net, o.name)),
                None => {
                    names.domains.insert(target, o.name);
                }
            }
        }
        names
            .networks
            .sort_by_key(|(net, _)| std::cmp::Reverse(net.prefix_len()));
        names
    }

    /// Display name for a sender at `ip`, named by `host` (its PTR host name or
    /// SPF domain) under `operator_domain`: an override for the IP or a range
    /// containing it, else one for `host` or its nearest parent below the
    /// operator domain (`mail.example.com`), else the operator domain's name.
    pub fn name_for(
        &self,
        ip: Option<IpAddr>,
        host: Option<&str>,
        operator_domain: Option<&str>,
    ) -> Option<&str> {
        let ip = ip.map(|ip| match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            v4 @ IpAddr::V4(_) => v4,
        });
        if let Some(ip) = ip {
            if let Some((_, name)) = self.networks.iter().find(|(net, _)| net.contains(&ip)) {
                return name.as_deref();
            }
        }
        let operator = operator_domain?;
        if let Some(name) = host.and_then(|host| self.host_override(host, operator)) {
            return name;
        }
        self.name(operator)
    }

    /// The override for `host` or its nearest parent strictly below `operator`;
    /// `Some(None)` when that override hides the name.
    fn host_override(&self, host: &str, operator: &str) -> Option<Option<&str>> {
        let host = normalize_name(host);
        let suffix = format!(".{}", normalize_name(operator));
        let mut name = host.as_str();
        while name.ends_with(&suffix) {
            if let Some(found) = self.domains.get(name) {
                return Some(found.as_deref());
            }
            name = name.split_once('.')?.1;
        }
        None
    }

    /// Display name for `operator_domain`, if one is known.
    pub fn name(&self, operator_domain: &str) -> Option<&str> {
        let key = normalize_name(operator_domain);
        match self.domains.get(&key) {
            Some(name) => name.as_deref(),
            None => bundled().get(&key).map(String::as_str),
        }
    }
}

/// Looser than `dmarc_domain::normalize_domain` on purpose: override targets
/// may name DNS labels with `_`, which a hostname may not carry.
fn looks_like_domain(s: &str) -> bool {
    s.contains('.')
        && !s.starts_with('.')
        && s.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> SenderNames {
        SenderNames::with_overrides(parse_overrides(text).unwrap())
    }

    #[test]
    fn bundled_map_names_common_senders() {
        let names = SenderNames::bundled();
        assert!(bundled().len() > 10_000);
        assert_eq!(names.name("amazonses.com"), Some("Amazon SES"));
        assert_eq!(names.name("HOSTPOINT.CH."), Some("Hostpoint"));
        assert_eq!(names.name("example.com"), None);
    }

    #[test]
    fn quoted_names_keep_their_commas() {
        let (domain, name) = bundled()
            .iter()
            .find(|(_, name)| name.contains(','))
            .expect("vendored map has a quoted name");
        assert_eq!(SenderNames::bundled().name(domain), Some(name.as_str()));
    }

    #[test]
    fn overrides_win_and_can_hide() {
        let names = names(
            "domain,name\n# ours\ncrm.example.com, Our CRM\nhostpoint.ch,\n\namazonses.com,SES,Web Host\n",
        );
        assert_eq!(names.name("crm.example.com"), Some("Our CRM"));
        assert_eq!(names.name("hostpoint.ch"), None);
        assert_eq!(names.name("amazonses.com"), Some("SES"));
        assert_eq!(
            names.name("google.com"),
            SenderNames::bundled().name("google.com")
        );
    }

    #[test]
    fn every_bad_line_is_reported() {
        let errors =
            parse_overrides("ok.example,Fine\nnot a domain,Bad\nok.example,Again\n").unwrap_err();
        assert_eq!(
            errors,
            [
                OverrideLineError {
                    line: 2,
                    message: "not an IP, IP range or domain: \"not a domain\"".into()
                },
                OverrideLineError {
                    line: 3,
                    message: "ok.example is listed twice".into()
                }
            ]
        );
    }

    #[test]
    fn ips_and_ranges_win_over_the_domain_most_specific_first() {
        let names = names(
            "target,name\nexample.net,Domain\n192.0.2.0/24,Range\n192.0.2.0/28,Narrow\n192.0.2.7,Host\n198.51.100.0/24,\n",
        );
        let ip = |s: &str| Some(s.parse::<IpAddr>().unwrap());
        assert_eq!(
            names.name_for(ip("192.0.2.7"), None, Some("example.net")),
            Some("Host")
        );
        assert_eq!(
            names.name_for(ip("192.0.2.8"), None, Some("example.net")),
            Some("Narrow")
        );
        assert_eq!(names.name_for(ip("192.0.2.99"), None, None), Some("Range"));
        assert_eq!(
            names.name_for(ip("::ffff:192.0.2.99"), None, None),
            Some("Range")
        );
        assert_eq!(
            names.name_for(ip("203.0.113.1"), None, Some("example.net")),
            Some("Domain")
        );
        // An empty name on a range hides the operator's name too.
        assert_eq!(
            names.name_for(ip("198.51.100.1"), None, Some("amazonses.com")),
            None
        );
    }

    #[test]
    fn host_overrides_match_the_host_or_its_nearest_parent() {
        let names =
            names("target,name\nexample.net,Domain\nmail.example.net,Mail\nbulk.example.net,\n");
        let at = |host: &str| names.name_for(None, Some(host), Some("example.net"));
        assert_eq!(at("mail.example.net"), Some("Mail"));
        assert_eq!(at("out-1.MAIL.example.net."), Some("Mail"));
        assert_eq!(at("web.example.net"), Some("Domain"));
        assert_eq!(at("example.net"), Some("Domain"));
        // An empty name hides the operator's name for that host only.
        assert_eq!(at("x.bulk.example.net"), None);
        // A host outside the operator domain is not looked up.
        assert_eq!(
            names.name_for(None, Some("mail.example.net"), Some("other.org")),
            None
        );
    }

    #[test]
    fn targets_are_normalized() {
        assert_eq!(parse_target("192.0.2.77/24").unwrap(), "192.0.2.0/24");
        assert_eq!(
            parse_target("Mail.Example.COM.").unwrap(),
            "mail.example.com"
        );
        assert_eq!(parse_target("2001:DB8::1").unwrap(), "2001:db8::1");
        assert!(parse_target("192.0.2.0/33").is_err());
    }

    #[test]
    fn text_round_trips() {
        let text =
            "10.0.0.0/8,Internal\nb.example,\"Name, with comma\"\n10.0.0.1,Relay\na.example,\n";
        let parsed = parse_overrides(text).unwrap();
        let formatted = format_overrides(&parsed);
        assert_eq!(
            formatted,
            "a.example,\nb.example,\"Name, with comma\"\n10.0.0.0/8,Internal\n10.0.0.1,Relay\n"
        );
        assert_eq!(parse_overrides(&formatted).unwrap().len(), 4);
    }
}
