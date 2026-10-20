//! Domain names as every Marcan crate handles them: one place for hostname
//! normalization and validation, the organizational (registrable) domain, and
//! the RFC 7489 §7.1 rule for when a report destination needs authorization.
//!
//! Two normalizers, on purpose:
//!
//! * [`normalize_domain`] is strict: it validates a name from a report or a
//!   user and rejects anything that is not an LDH hostname.
//! * [`normalize_name`] is lenient: it only canonicalizes (trim, lowercase,
//!   drop the trailing dot) so names from DNS answers, SPF terms and PTR
//!   records compare equal. It never rejects.

#![forbid(unsafe_code)]

use thiserror::Error;

/// Maximum length of a DNS name excluding a trailing dot (RFC 1035).
pub const MAX_DOMAIN_LEN: usize = 253;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("domain must not be empty")]
    Empty,
    #[error("domain is too long (max {MAX_DOMAIN_LEN} characters)")]
    TooLong,
    #[error("domain is not a valid hostname")]
    Invalid,
}

/// Normalize a domain from a report or a user: trim, strip a trailing dot, lowercase.
///
/// Rejects empty values, oversized names, IP addresses, URLs, and anything
/// that is not an LDH hostname with at least two labels.
pub fn normalize_domain(input: &str) -> Result<String, DomainError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Empty);
    }

    let without_dot = trimmed.strip_suffix('.').unwrap_or(trimmed);
    if without_dot.is_empty() {
        return Err(DomainError::Empty);
    }

    let lower = without_dot.to_ascii_lowercase();
    if lower.len() > MAX_DOMAIN_LEN {
        return Err(DomainError::TooLong);
    }

    if !is_hostname(&lower) {
        return Err(DomainError::Invalid);
    }

    Ok(lower)
}

/// Canonical spelling of a name without validating it: surrounding
/// whitespace and trailing dots removed, ASCII-lowercased. Empty input stays
/// empty; an IP literal or junk passes through unchanged.
pub fn normalize_name(name: &str) -> String {
    name.trim()
        .trim_end_matches('.')
        .trim_end()
        .to_ascii_lowercase()
}

fn is_hostname(s: &str) -> bool {
    if s.starts_with('[') || s.contains(':') || s.contains('/') || s.contains(' ') {
        return false;
    }
    if s.contains('@') || s.contains('?') || s.contains('#') {
        return false;
    }
    if s.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }

    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    labels.iter().all(|label| is_dns_label(label))
}

fn is_dns_label(label: &str) -> bool {
    let len = label.len();
    if len == 0 || len > 63 {
        return false;
    }
    let bytes = label.as_bytes();
    if bytes[0] == b'-' || bytes[len - 1] == b'-' {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Registrable / organizational domain, or `None` when the input is empty,
/// an IP literal, not a domain, or *is itself* a public suffix (`co.uk`,
/// `com`, …). Case, surrounding whitespace and trailing dots are ignored; an
/// unlisted TLD falls back to the PSL's implicit `*` rule (`a.b.zz` -> `b.zz`).
///
/// Uses the compiled-in Public Suffix List from the `psl` crate (no runtime
/// network fetch). Naive "last two labels" matching is wrong for multi-label
/// suffixes such as `co.uk` and can make a bare public suffix appear to align
/// with everything: a name that is a public suffix has no organizational
/// domain and must not be treated as aligning with every other suffix-less
/// name.
pub fn organizational_domain(domain: &str) -> Option<String> {
    let normalized = normalize_name(domain);
    if normalized.is_empty() {
        return None;
    }
    // The PSL would read `192.0.2.1` as a name under the TLD `1`.
    if normalized.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let bytes = normalized.as_bytes();

    // `psl::suffix` equals the whole name ⇔ this name *is* a public suffix.
    if let Some(suffix) = psl::suffix(bytes) {
        if suffix.as_bytes() == bytes {
            return None;
        }
    }

    let registrable = psl::domain(bytes)?;
    if registrable.suffix().as_bytes() == registrable.as_bytes() {
        return None;
    }

    let org = std::str::from_utf8(registrable.as_bytes()).ok()?;
    Some(org.to_ascii_lowercase())
}

/// RFC 7489 §7.1: reports for `domain` sent to an address at `rua_host` need
/// an authorization record only when the two organizational domains differ
/// (`shop.acme.example` reporting to `x@acme.example` needs none). When either
/// organizational domain cannot be derived, only the domain itself and its
/// subdomains count as the same.
pub fn report_authorization_needed(domain: &str, rua_host: &str) -> bool {
    match (
        organizational_domain(domain),
        organizational_domain(rua_host),
    ) {
        (Some(a), Some(b)) => a != b,
        _ => !host_within_domain(rua_host, domain),
    }
}

/// True when `host` is the same domain as `checked` or a subdomain of it.
pub fn host_within_domain(host: &str, checked: &str) -> bool {
    let host = normalize_name(host);
    let checked = normalize_name(checked);
    host == checked || host.ends_with(&format!(".{checked}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_trailing_dot() {
        assert_eq!(normalize_domain("Example.COM.").unwrap(), "example.com");
        assert_eq!(normalize_domain("  GitHub.com  ").unwrap(), "github.com");
    }

    #[test]
    fn rejects_empty_and_junk() {
        assert_eq!(normalize_domain(""), Err(DomainError::Empty));
        assert_eq!(normalize_domain("   "), Err(DomainError::Empty));
        assert_eq!(normalize_domain("."), Err(DomainError::Empty));
        for junk in [
            "localhost",
            "not a domain",
            "https://example.com",
            "example.com/path",
            "127.0.0.1",
            "example.com:8080",
            "bad_label.example.com",
            "-bad.example.com",
        ] {
            assert_eq!(normalize_domain(junk), Err(DomainError::Invalid), "{junk}");
        }
    }

    #[test]
    fn rejects_oversized() {
        let label = "a".repeat(63);
        let huge = format!("{label}.{label}.{label}.{label}.com");
        assert!(huge.len() > MAX_DOMAIN_LEN);
        assert_eq!(normalize_domain(&huge), Err(DomainError::TooLong));
    }

    #[test]
    fn lenient_normalization_never_rejects() {
        assert_eq!(normalize_name(" MAIL.Example.COM. "), "mail.example.com");
        assert_eq!(normalize_name("example.com.."), "example.com");
        assert_eq!(normalize_name("_spf.Example.net"), "_spf.example.net");
        assert_eq!(normalize_name("192.0.2.1"), "192.0.2.1");
        assert_eq!(normalize_name("..."), "");
        assert_eq!(normalize_name("  "), "");
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
    }

    #[test]
    fn empty_inputs_and_ip_literals_have_no_organizational_domain() {
        assert_eq!(organizational_domain(""), None);
        assert_eq!(organizational_domain("   "), None);
        assert_eq!(organizational_domain("."), None);
        assert_eq!(organizational_domain("192.0.2.1"), None);
        assert_eq!(organizational_domain("8.8.8.8."), None);
        assert_eq!(organizational_domain("2001:db8::1"), None);
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
    }

    #[test]
    fn authorization_only_needed_across_organizational_domains() {
        assert!(!report_authorization_needed(
            "shop.acme.example",
            "acme.example"
        ));
        assert!(!report_authorization_needed(
            "acme.example",
            "mail.acme.example"
        ));
        assert!(!report_authorization_needed(
            "a.example.co.uk",
            "b.example.co.uk"
        ));
        assert!(report_authorization_needed("example.co.uk", "other.co.uk"));
        assert!(report_authorization_needed(
            "acme.example",
            "reports.example.net"
        ));
    }

    #[test]
    fn host_within_domain_matches_subdomains() {
        assert!(host_within_domain("example.com", "example.com"));
        assert!(host_within_domain("mail.example.com", "example.com"));
        assert!(!host_within_domain("example.org", "example.com"));
        assert!(!host_within_domain("notexample.com", "example.com"));
    }
}
