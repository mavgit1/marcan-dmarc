//! The one rule that names a sending IP from its evidence.

use crate::names::SenderNames;
use crate::operator::operator_domain;
use serde::Serialize;
use std::net::IpAddr;
use ts_rs::TS;

/// What is known about one sending IP before naming it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    /// First reverse-DNS name, if any.
    pub ptr_hostname: Option<String>,
    /// `None` without a PTR; whether the PTR's A/AAAA includes the IP.
    pub ptr_confirmed: Option<bool>,
    /// The domain whose SPF passed for this IP most often in reports, without
    /// being aligned with the From domain (typically a provider's bounce
    /// domain). Aligned passes name the customer, not the operator.
    pub spf_domain: Option<String>,
}

/// Which evidence named the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum IdentitySource {
    /// Reverse DNS. Check `ptr_confirmed` for whether it was forward-confirmed.
    Ptr,
    /// A non-aligned SPF pass in reports: the domain authorizes this IP.
    Spf,
}

/// A sending IP's identity: a neutral caption, never an input to a verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SenderIdentity {
    /// Display name: a user's override for this IP, a range containing it or
    /// its operator, else the name map's name for the operator.
    pub name: Option<String>,
    /// Who operates the IP, per `source`.
    pub operator_domain: Option<String>,
    pub source: Option<IdentitySource>,
    pub ptr_hostname: Option<String>,
    /// `null` without a PTR; `false` when the name does not resolve back to
    /// the IP (still shown, but labelled).
    pub ptr_confirmed: Option<bool>,
    pub spf_domain: Option<String>,
}

/// Name a sending IP from its `evidence`, strongest verified evidence first:
///
/// 1. forward-confirmed reverse DNS;
/// 2. a non-aligned SPF pass (a receiver verified the domain authorizes the IP);
/// 3. unconfirmed reverse DNS (anyone controlling the IP's PTR can claim a name).
///
/// DKIM is deliberately not used: a signature says who signed the message,
/// not who sent it from this IP, and a forwarder relays signed mail.
///
/// `ip` only picks a user's name for the IP or its range; it is not evidence.
pub fn resolve(ip: Option<IpAddr>, evidence: Evidence, names: &SenderNames) -> SenderIdentity {
    let ptr_operator = evidence.ptr_hostname.as_deref().and_then(operator_domain);
    let spf_operator = evidence.spf_domain.as_deref().and_then(operator_domain);

    let (operator, source) = match (ptr_operator, evidence.ptr_confirmed, spf_operator) {
        (Some(ptr), Some(true), _) => (Some(ptr), Some(IdentitySource::Ptr)),
        (_, _, Some(spf)) => (Some(spf), Some(IdentitySource::Spf)),
        (Some(ptr), _, None) => (Some(ptr), Some(IdentitySource::Ptr)),
        (None, _, None) => (None, None),
    };
    // The full name behind the operator domain, for overrides of one host.
    let host = match source {
        Some(IdentitySource::Ptr) => evidence.ptr_hostname.as_deref(),
        Some(IdentitySource::Spf) => evidence.spf_domain.as_deref(),
        None => None,
    };

    SenderIdentity {
        name: names
            .name_for(ip, host, operator.as_deref())
            .map(str::to_owned),
        operator_domain: operator,
        source,
        ptr_hostname: evidence.ptr_hostname,
        ptr_confirmed: evidence.ptr_confirmed,
        spf_domain: evidence.spf_domain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptr(host: &str, confirmed: bool) -> Evidence {
        Evidence {
            ptr_hostname: Some(host.into()),
            ptr_confirmed: Some(confirmed),
            spf_domain: None,
        }
    }

    #[test]
    fn confirmed_ptr_names_the_operator() {
        let id = resolve(
            None,
            ptr("a1-2.smtp-out.amazonses.com", true),
            &SenderNames::bundled(),
        );
        assert_eq!(id.operator_domain.as_deref(), Some("amazonses.com"));
        assert_eq!(id.name.as_deref(), Some("Amazon SES"));
        assert_eq!(id.source, Some(IdentitySource::Ptr));
    }

    #[test]
    fn spf_pass_beats_an_unconfirmed_ptr() {
        let evidence = Evidence {
            spf_domain: Some("bounces.mcsv.net".into()),
            ..ptr("mail.google.com", false)
        };
        let id = resolve(None, evidence, &SenderNames::bundled());
        assert_eq!(id.operator_domain.as_deref(), Some("mcsv.net"));
        assert_eq!(id.source, Some(IdentitySource::Spf));
        assert_eq!(id.ptr_hostname.as_deref(), Some("mail.google.com"));
    }

    #[test]
    fn unconfirmed_ptr_is_the_last_resort() {
        let id = resolve(
            None,
            ptr("mail.unlisted-example.net", false),
            &SenderNames::bundled(),
        );
        assert_eq!(id.operator_domain.as_deref(), Some("unlisted-example.net"));
        assert_eq!(id.name, None);
        assert_eq!(id.ptr_confirmed, Some(false));
    }

    #[test]
    fn no_evidence_no_name() {
        let id = resolve(None, Evidence::default(), &SenderNames::bundled());
        assert_eq!(id.operator_domain, None);
        assert_eq!(id, SenderIdentity::default());
    }

    #[test]
    fn overrides_rename_operators() {
        let names = SenderNames::with_overrides(
            crate::parse_overrides("amazonses.com,Our mailer").unwrap(),
        );
        let id = resolve(None, ptr("x.amazonses.com", true), &names);
        assert_eq!(id.name.as_deref(), Some("Our mailer"));
    }

    #[test]
    fn overrides_name_servers_without_evidence() {
        let names =
            SenderNames::with_overrides(crate::parse_overrides("192.0.2.0/24,Office").unwrap());
        let id = resolve("192.0.2.9".parse().ok(), Evidence::default(), &names);
        assert_eq!(id.name.as_deref(), Some("Office"));
        assert_eq!(id.operator_domain, None);
    }
}
