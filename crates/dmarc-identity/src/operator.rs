//! Operator domain of a hostname: who runs the machine behind a name.
//!
//! The registrable domain (eTLD+1 via the Public Suffix List), folded by
//! parsedmarc's PSL overrides (`data/psl_overrides.txt`): hosting patterns
//! such as `198-51-100-7-customer.example-isp.net` would otherwise make every
//! customer its own "operator".
//!
//! This is a label, not DMARC alignment: alignment keeps using the plain
//! organizational domain from `dmarc-domain`.

use dmarc_domain::{normalize_name, organizational_domain};
use std::net::IpAddr;
use std::sync::OnceLock;

/// parsedmarc's suffix overrides, verbatim (see `data/README.md`).
const PSL_OVERRIDES: &str = include_str!("../data/psl_overrides.txt");

fn overrides() -> &'static [&'static str] {
    static PARSED: OnceLock<Vec<&'static str>> = OnceLock::new();
    PARSED.get_or_init(|| {
        PSL_OVERRIDES
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    })
}

/// Lowercased name without a trailing dot; `None` when empty or an IP literal.
fn normalize(name: &str) -> Option<String> {
    let host = normalize_name(name);
    if host.is_empty() || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    Some(host)
}

/// The base an override folds `host` to. An entry starting with `.` or `-`
/// matches as a plain suffix (`x-1-2-3-4-isp.net` under `-isp.net`); a bare
/// entry matches on a label boundary only, so `isp.net` never claims
/// `notisp.net`. The first matching entry wins, as in parsedmarc.
fn folded_base(host: &str) -> Option<&'static str> {
    overrides().iter().find_map(|entry| {
        let base = entry.trim_start_matches(['.', '-']);
        let matches = if base.len() < entry.len() {
            host.ends_with(entry)
        } else {
            host == base || host.ends_with(&format!(".{base}"))
        };
        (matches && !base.is_empty()).then_some(base)
    })
}

/// Operator domain of `hostname`, or `None` when the input is empty, an IP
/// literal, not a domain, or is itself a public suffix (`co.uk`, `com`).
///
/// Examples:
/// - `mail-1.mailchannels.net` -> `mailchannels.net`
/// - `a27-1.smtp-out.eu-west-1.amazonses.com` -> `amazonses.com`
/// - `mail.example.co.uk` -> `example.co.uk`
pub fn operator_domain(hostname: &str) -> Option<String> {
    let host = normalize(hostname)?;
    if let Some(base) = folded_base(&host) {
        return Some(base.to_owned());
    }
    organizational_domain(&host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(host: &str) -> Option<String> {
        operator_domain(host)
    }

    #[test]
    fn registrable_domain_of_common_hosts() {
        assert_eq!(
            op("mail-1.mailchannels.net").as_deref(),
            Some("mailchannels.net")
        );
        assert_eq!(
            op("mail-yw1-f123.google.com.").as_deref(),
            Some("google.com")
        );
        assert_eq!(
            op("a27-1.smtp-out.eu-west-1.amazonses.com").as_deref(),
            Some("amazonses.com")
        );
        assert_eq!(op("mail.example.co.uk").as_deref(), Some("example.co.uk"));
        assert_eq!(op("SMTP1.EXAMPLE.COM").as_deref(), Some("example.com"));
    }

    #[test]
    fn rejects_suffixes_and_garbage() {
        for bad in [
            "co.uk",
            "com",
            "",
            "   ",
            "8.8.8.8",
            "192.0.2.1.",
            "2001:db8::1",
            "not a hostname",
        ] {
            assert_eq!(op(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn overrides_fold_customer_hostnames() {
        // `-applefibernet.com` is a separator entry in the vendored list.
        assert!(overrides().contains(&"-applefibernet.com"));
        assert_eq!(
            op("203-0-113-7-applefibernet.com").as_deref(),
            Some("applefibernet.com")
        );
    }

    #[test]
    fn bare_overrides_respect_label_boundaries() {
        let bare = overrides()
            .iter()
            .find(|e| !e.starts_with(['.', '-']))
            .expect("vendored list has a bare entry");
        assert_eq!(folded_base(&format!("mail.{bare}")), Some(*bare));
        assert_eq!(folded_base(bare), Some(*bare));
        assert_eq!(folded_base(&format!("x{bare}")), None);
    }
}
