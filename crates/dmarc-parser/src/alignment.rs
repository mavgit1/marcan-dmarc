//! DMARC identifier alignment (RFC 7489 §3.1): whether one check's domain
//! aligns with the From domain. Whether the record passed DMARC is
//! `dmarc_assess::RecordFacts`'s, decided at ingest.
//!
//! Organizational domains come from `dmarc-domain` (the compiled-in Public
//! Suffix List): a bare public suffix has none and aligns with nothing.

use crate::types::{DkimAuthResult, SpfAuthResult};
use dmarc_domain::{normalize_name, organizational_domain};

/// `adkim` / `aspf` alignment mode. RFC 7489 defaults to relaxed when the tag
/// is absent or unrecognized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentMode {
    Relaxed,
    Strict,
}

impl AlignmentMode {
    /// Parse a DMARC `adkim`/`aspf` tag (`r` or `s`, case-insensitive).
    ///
    /// Anything other than `s` — including `None`, empty, and unknown values —
    /// is treated as relaxed, matching the RFC default.
    pub fn from_tag(tag: Option<&str>) -> Self {
        match tag.map(str::trim) {
            Some(t) if t.eq_ignore_ascii_case("s") => Self::Strict,
            _ => Self::Relaxed,
        }
    }
}

/// Trim, strip trailing dots, and lowercase a domain. Returns `None` when
/// nothing remains.
fn normalize_fqdn(raw: &str) -> Option<String> {
    let normalized = normalize_name(raw);
    (!normalized.is_empty()).then_some(normalized)
}

/// Whether `auth_domain` is aligned with `header_from` under `mode`.
///
/// Strict: normalized FQDNs must be identical.
/// Relaxed: both sides must have an organizational domain and those must match.
pub fn domains_aligned(auth_domain: &str, header_from: &str, mode: AlignmentMode) -> bool {
    let Some(auth) = normalize_fqdn(auth_domain) else {
        return false;
    };
    let Some(from) = normalize_fqdn(header_from) else {
        return false;
    };

    match mode {
        AlignmentMode::Strict => auth == from,
        AlignmentMode::Relaxed => {
            match (organizational_domain(&auth), organizational_domain(&from)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        }
    }
}

fn header_aligned(auth_domain: &str, header_from: Option<&str>, mode: AlignmentMode) -> bool {
    header_from.is_some_and(|from| domains_aligned(auth_domain, from, mode))
}

/// Whether this DKIM `d=` domain aligns with `header_from` under `adkim`.
pub fn dkim_auth_aligned(
    dkim: &DkimAuthResult,
    header_from: Option<&str>,
    mode: AlignmentMode,
) -> bool {
    header_aligned(&dkim.domain, header_from, mode)
}

/// Whether this SPF result is DMARC's SPF identity. DMARC checks MAIL FROM
/// (RFC 7489 §4.1); a HELO result only stands in when the record has no other
/// (a bounce with an empty MAIL FROM). Results without a scope count.
pub fn spf_counts_for_dmarc(spf: &SpfAuthResult, all: &[SpfAuthResult]) -> bool {
    !is_helo(spf) || all.iter().all(is_helo)
}

fn is_helo(spf: &SpfAuthResult) -> bool {
    spf.scope
        .as_deref()
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("helo"))
}

/// Whether this SPF auth-result domain aligns with `header_from` under `aspf`.
pub fn spf_auth_aligned(
    spf: &SpfAuthResult,
    header_from: Option<&str>,
    mode: AlignmentMode,
) -> bool {
    header_aligned(&spf.domain, header_from, mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_tag_defaults_to_relaxed() {
        assert_eq!(AlignmentMode::from_tag(None), AlignmentMode::Relaxed);
        assert_eq!(AlignmentMode::from_tag(Some("")), AlignmentMode::Relaxed);
        assert_eq!(AlignmentMode::from_tag(Some("r")), AlignmentMode::Relaxed);
        assert_eq!(AlignmentMode::from_tag(Some("R")), AlignmentMode::Relaxed);
        assert_eq!(
            AlignmentMode::from_tag(Some("nope")),
            AlignmentMode::Relaxed
        );
        assert_eq!(AlignmentMode::from_tag(Some("s")), AlignmentMode::Strict);
        assert_eq!(AlignmentMode::from_tag(Some("S")), AlignmentMode::Strict);
        assert_eq!(AlignmentMode::from_tag(Some(" s ")), AlignmentMode::Strict);
    }

    #[test]
    fn relaxed_matches_subdomain() {
        assert!(domains_aligned(
            "mail.example.com",
            "example.com",
            AlignmentMode::Relaxed
        ));
        assert!(domains_aligned(
            "example.com",
            "mail.example.com",
            AlignmentMode::Relaxed
        ));
    }

    #[test]
    fn strict_rejects_subdomain() {
        assert!(!domains_aligned(
            "mail.example.com",
            "example.com",
            AlignmentMode::Strict
        ));
    }

    #[test]
    fn exact_match_under_both_modes() {
        assert!(domains_aligned(
            "example.com",
            "example.com",
            AlignmentMode::Relaxed
        ));
        assert!(domains_aligned(
            "example.com",
            "example.com",
            AlignmentMode::Strict
        ));
    }

    #[test]
    fn multi_label_public_suffix() {
        assert_eq!(
            organizational_domain("example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            organizational_domain("mail.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert!(domains_aligned(
            "mail.example.co.uk",
            "example.co.uk",
            AlignmentMode::Relaxed
        ));
        assert!(!domains_aligned(
            "mail.example.co.uk",
            "example.co.uk",
            AlignmentMode::Strict
        ));
        // Different orgs under the same suffix must not align.
        assert!(!domains_aligned(
            "other.co.uk",
            "example.co.uk",
            AlignmentMode::Relaxed
        ));
    }

    #[test]
    fn unrelated_domains_do_not_align() {
        assert!(!domains_aligned(
            "evil.example",
            "example.com",
            AlignmentMode::Relaxed
        ));
        assert!(!domains_aligned(
            "evil.example",
            "example.com",
            AlignmentMode::Strict
        ));
        assert!(!domains_aligned(
            "example.net",
            "example.com",
            AlignmentMode::Relaxed
        ));
    }

    #[test]
    fn comparison_is_case_insensitive() {
        assert!(domains_aligned(
            "Mail.Example.COM",
            "EXAMPLE.com",
            AlignmentMode::Relaxed
        ));
        assert!(domains_aligned(
            "Example.COM",
            "example.com",
            AlignmentMode::Strict
        ));
    }

    #[test]
    fn trailing_dot_is_ignored() {
        assert!(domains_aligned(
            "mail.example.com.",
            "example.com.",
            AlignmentMode::Relaxed
        ));
        assert!(domains_aligned(
            "example.com.",
            "example.com",
            AlignmentMode::Strict
        ));
    }

    #[test]
    fn none_and_empty_inputs_do_not_align() {
        assert!(!domains_aligned("", "example.com", AlignmentMode::Relaxed));
        assert!(!domains_aligned("example.com", "", AlignmentMode::Relaxed));
        assert!(!domains_aligned(
            "...",
            "example.com",
            AlignmentMode::Strict
        ));
        assert!(!domains_aligned("  ", "  ", AlignmentMode::Relaxed));
        assert_eq!(organizational_domain(""), None);
        assert_eq!(organizational_domain("   "), None);
        assert_eq!(organizational_domain("."), None);
        assert!(!header_aligned("example.com", None, AlignmentMode::Relaxed));
    }

    #[test]
    fn ip_literals_have_no_organizational_domain() {
        assert_eq!(organizational_domain("192.0.2.1"), None);
        assert_eq!(organizational_domain("8.8.8.8."), None);
        assert_eq!(organizational_domain("2001:db8::1"), None);
        // Two addresses in the same /16 must not "align" through a fake `8.8`.
        assert!(!domains_aligned(
            "1.8.8.8",
            "2.8.8.8",
            AlignmentMode::Relaxed
        ));
        assert!(domains_aligned(
            "192.0.2.1",
            "192.0.2.1",
            AlignmentMode::Strict
        ));
    }

    #[test]
    fn unlisted_tld_and_formatting_noise() {
        assert_eq!(
            organizational_domain("host.example.unknowntld").as_deref(),
            Some("example.unknowntld")
        );
        assert_eq!(organizational_domain("unknowntld"), None);
        assert_eq!(
            organizational_domain(" MAIL.Example.COM. ").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn public_suffix_has_no_organizational_domain() {
        assert_eq!(organizational_domain("co.uk"), None);
        assert_eq!(organizational_domain("com"), None);
        // The footgun: two names that are (or lack) an org domain must not
        // collapse to a universal match.
        assert!(!domains_aligned("co.uk", "co.uk", AlignmentMode::Relaxed));
        assert!(!domains_aligned("co.uk", "com", AlignmentMode::Relaxed));
        assert!(!domains_aligned(
            "co.uk",
            "example.co.uk",
            AlignmentMode::Relaxed
        ));
        assert!(!domains_aligned(
            "com",
            "example.com",
            AlignmentMode::Relaxed
        ));
        // Strict still requires an exact FQDN match; a suffix vs a child fails.
        assert!(!domains_aligned(
            "co.uk",
            "example.co.uk",
            AlignmentMode::Strict
        ));
    }
}
