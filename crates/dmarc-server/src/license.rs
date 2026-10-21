//! Operator keys and the free-software notice.
//!
//! Every feature is in the code for everyone. The one thing an operator
//! license changes is attribution: the free-software notice ([`notice`], the
//! sentence that ends every email, and a line in the UI) and how the emails
//! open (Marcan first, the operator's letterhead second; see
//! `mail::email_top`). The notice is shown unless a valid operator key is
//! stored ([`Standing::notice_shown`], the one switch), and off whenever one
//! is. There is no setting. Branding is free for everyone and never looks at
//! the key.
//!
//! A key is `<email>.<base64url signature>`: Ed25519 over the email
//! (trimmed, lowercased) by the steward's signing key, which lives outside
//! the repository (`private/license/README.md` says where). Keys never
//! expire and are never checked online; the license is a subscription, and
//! its terms, not the key, say when the notice duty returns.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dmarc_store::LicenseSettings;
use ring::signature::{UnparsedPublicKey, ED25519};
use std::fmt;

/// Where the source code of the version this server runs is: the one place
/// the UI (through `GET /api/meta`) and the emails read it from. A modified
/// version run for others must point this at its own source (LICENSE,
/// AGPLv3 section 13).
pub const SOURCE_URL: &str = concat!(
    "https://github.com/mavgit1/marcan-dmarc/tree/v",
    env!("CARGO_PKG_VERSION")
);

/// Where an operator license is bought: the one place the UI (through
/// `GET /api/meta`) reads it from.
pub const LICENSE_URL: &str = "https://marcan.kaitiaki.tech/license";

/// Where an operator manages or cancels the subscription behind a key (the
/// Stripe customer portal): the one place the UI (through `GET /api/meta`)
/// reads it from. The License page links it only while a valid key is
/// stored; the plans and their prices are the docs' Pricing page's alone.
pub const BILLING_URL: &str = "https://billing.stripe.com/p/login/3cIdRa2g90ar77I1vdeEo01";

/// The free-software notice: the one sentence that ends every email while it
/// is shown, signed by the operator when branding names one. It tells a
/// client what Section 2(a) of the Beneficiary Copyleft License 1.0 asks
/// for: the service runs on free software, theirs to run too, under that
/// license, and where the source of this version is ([`SOURCE_URL`]).
pub fn notice(operator_name: Option<&str>) -> String {
    let by = match operator_name {
        Some(name) => format!("{name} with Marcan"),
        None => "Marcan".to_owned(),
    };
    format!(
        "Sent by {by}, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: {SOURCE_URL}"
    )
}

/// The steward's public key, printed by `private/license/sign-key.sh`.
/// Unused in a build with the `test-keys` feature, which trusts the test
/// keypair instead.
#[cfg_attr(all(feature = "test-keys", not(test)), allow(dead_code))]
const PUBLIC_KEY: [u8; 32] = [
    0x1f, 0xa1, 0x85, 0x35, 0xc1, 0x7b, 0xa5, 0x8c, 0xf7, 0x14, 0xb5, 0xfe, 0xbd, 0xe6, 0x97, 0xab,
    0xeb, 0x5f, 0x26, 0x54, 0x6d, 0x33, 0x16, 0xc8, 0x2d, 0xcb, 0x8b, 0x30, 0x24, 0x03, 0xb1, 0x5e,
];

/// Who a valid key was issued to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicensedTo {
    /// Trimmed and lowercased, as signed.
    pub email: String,
}

/// Why a key is not valid. Both read the same to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Not `<email>.<signature>`, or the signature is not 64 bytes of base64url.
    Format,
    /// Well formed, but not signed by the steward for this email.
    Signature,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("This key is not valid.")
    }
}

impl std::error::Error for KeyError {}

/// The email as it is signed: trimmed and lowercased.
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The public key keys are checked against: the steward's, except in the
/// tests, which sign their keys with [`test_keys`] so that no key signed by
/// the steward is ever published in the code.
fn trusted_public_key() -> [u8; 32] {
    #[cfg(not(any(test, feature = "test-keys")))]
    return PUBLIC_KEY;
    #[cfg(any(test, feature = "test-keys"))]
    return test_keys::public_key();
}

/// Check `key` offline against the steward's [`PUBLIC_KEY`].
pub fn verify_key(key: &str) -> Result<LicensedTo, KeyError> {
    verify_with(key, &trusted_public_key())
}

/// Check `key` against `public_key`.
fn verify_with(key: &str, public_key: &[u8; 32]) -> Result<LicensedTo, KeyError> {
    let (email, signature) = key.trim().rsplit_once('.').ok_or(KeyError::Format)?;
    let email = normalize_email(email);
    let well_formed = email
        .split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if !well_formed || email.chars().any(char::is_whitespace) {
        return Err(KeyError::Format);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature.trim().trim_end_matches('='))
        .map_err(|_| KeyError::Format)?;
    if signature.len() != 64 {
        return Err(KeyError::Format);
    }
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(email.as_bytes(), &signature)
        .map_err(|_| KeyError::Signature)?;
    Ok(LicensedTo { email })
}

/// How the UI names a stored key without showing it: whose it is and the
/// last six characters of its signature, "you@acme.example · …GPL7CQ"; only
/// the end ("…GPL7CQ") when the key does not verify, since its email then
/// names no one.
pub fn key_hint(key: &str) -> String {
    let key = key.trim();
    let signature = key
        .rsplit_once('.')
        .map_or(key, |(_, signature)| signature)
        .trim_end_matches('=');
    let start = signature
        .char_indices()
        .rev()
        .nth(5)
        .map_or(0, |(index, _)| index);
    let tail = &signature[start..];
    match verify_key(key) {
        Ok(licensed) => format!("{} · …{tail}", licensed.email),
        Err(_) => format!("…{tail}"),
    }
}

/// What the stored license settings mean now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// The email of the stored key, when it is valid.
    pub licensed_to: Option<String>,
    /// The notice goes in the UI and the emails: unless a valid key is stored.
    pub notice_shown: bool,
}

impl Standing {
    pub fn new(license: &LicenseSettings) -> Self {
        let licensed_to = license
            .operator_key
            .as_deref()
            .and_then(|key| verify_key(key).ok())
            .map(|l| l.email);
        Self {
            notice_shown: licensed_to.is_none(),
            licensed_to,
        }
    }
}

/// Operator keys for the tests, signed by a test keypair whose seed is
/// public test data. Built only for the unit tests and with the `test-keys`
/// feature, which the integration tests enable (`Cargo.toml`); a release
/// build trusts the steward's key alone, so these keys verify nowhere else.
#[cfg(any(test, feature = "test-keys"))]
pub mod test_keys {
    use super::URL_SAFE_NO_PAD;
    use base64::Engine;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    /// The test keypair's seed. Public, and good for nothing but the tests.
    const SEED: &[u8; 32] = b"marcan-test-operator-keys-only!!";

    /// A key for `test@example.com`: [`sign`] of that email.
    pub const TEST_KEY: &str = "test@example.com.89QIxd3GyhiPFUyytT6JAEy7Ksao-0nNyP7FRFmx6Kg7vKMvZAJIBFetrTlcgBcJFn51xTMJw-1Nrmy6IPPtCQ";

    fn keypair() -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(SEED).expect("a 32-byte seed")
    }

    /// The test keypair's public key.
    pub fn public_key() -> [u8; 32] {
        keypair()
            .public_key()
            .as_ref()
            .try_into()
            .expect("an Ed25519 public key is 32 bytes")
    }

    /// An operator key for `email`, signed by the test keypair.
    pub fn sign(email: &str) -> String {
        let email = super::normalize_email(email);
        let signature = keypair().sign(email.as_bytes());
        format!("{email}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::test_keys::TEST_KEY;
    use super::*;

    #[test]
    fn the_test_key_is_the_test_keypairs_and_not_the_stewards() {
        assert_eq!(test_keys::sign("test@example.com"), TEST_KEY);
        assert_ne!(test_keys::public_key(), PUBLIC_KEY);
        assert_eq!(verify_with(TEST_KEY, &PUBLIC_KEY), Err(KeyError::Signature));
        assert_eq!(
            verify_with(TEST_KEY, &test_keys::public_key()),
            Ok(licensed("test@example.com"))
        );
    }

    fn licensed(email: &str) -> LicensedTo {
        LicensedTo {
            email: email.to_owned(),
        }
    }

    #[test]
    fn a_valid_key_verifies_and_names_its_email() {
        assert_eq!(verify_key(TEST_KEY), Ok(licensed("test@example.com")));
        // Padding the signature is tolerated.
        assert_eq!(
            verify_key(&format!("{TEST_KEY}==")),
            Ok(licensed("test@example.com"))
        );
    }

    #[test]
    fn the_email_is_trimmed_and_lowercased_before_verifying() {
        let (_, signature) = TEST_KEY.rsplit_once('.').unwrap();
        assert_eq!(
            verify_key(&format!("  Test@EXAMPLE.com.{signature} \n")),
            Ok(licensed("test@example.com"))
        );
    }

    #[test]
    fn a_tampered_key_fails() {
        let (_, signature) = TEST_KEY.rsplit_once('.').unwrap();
        // Another email with the same signature.
        assert_eq!(
            verify_key(&format!("other@example.com.{signature}")),
            Err(KeyError::Signature)
        );
        // One character of the signature changed.
        let mut tampered: Vec<char> = signature.chars().collect();
        tampered[10] = if tampered[10] == 'A' { 'B' } else { 'A' };
        let tampered: String = tampered.into_iter().collect();
        assert_eq!(
            verify_key(&format!("test@example.com.{tampered}")),
            Err(KeyError::Signature)
        );
    }

    #[test]
    fn a_wrong_format_fails() {
        for key in [
            "",
            "test@example.com",
            "no-at-sign.example.oXFJ",
            "test@example.com.not base64!",
            "test@example.com.AAAA",
            "@example.com.89QIxd3GyhiPFUyytT6JAEy7Ksao-0nNyP7FRFmx6Kg7vKMvZAJIBFetrTlcgBcJFn51xTMJw-1Nrmy6IPPtCQ",
            "te st@example.com.89QIxd3GyhiPFUyytT6JAEy7Ksao-0nNyP7FRFmx6Kg7vKMvZAJIBFetrTlcgBcJFn51xTMJw-1Nrmy6IPPtCQ",
        ] {
            assert_eq!(verify_key(key), Err(KeyError::Format), "{key:?}");
        }
        assert_eq!(KeyError::Format.to_string(), "This key is not valid.");
        assert_eq!(KeyError::Signature.to_string(), "This key is not valid.");
    }

    #[test]
    fn the_notice_is_off_exactly_with_a_valid_key() {
        let standing = |key: Option<&str>| {
            Standing::new(&LicenseSettings {
                operator_key: key.map(str::to_owned),
            })
        };
        assert_eq!(
            standing(None),
            Standing {
                licensed_to: None,
                notice_shown: true
            }
        );
        assert!(standing(Some("test@example.com.AAAA")).notice_shown);
        assert_eq!(
            standing(Some(TEST_KEY)),
            Standing {
                licensed_to: Some("test@example.com".into()),
                notice_shown: false
            }
        );
    }

    #[test]
    fn the_key_hint_names_the_email_and_the_end_of_the_signature() {
        assert_eq!(key_hint(TEST_KEY), "test@example.com · …IPPtCQ");
        // Padding and surrounding space are not part of it.
        assert_eq!(
            key_hint(&format!("  {TEST_KEY}==\n")),
            "test@example.com · …IPPtCQ"
        );
        // A key that does not verify names no one.
        assert_eq!(key_hint("someone@example.com.AAAAbbbbCCCC"), "…bbCCCC");
        assert_eq!(key_hint("abc"), "…abc");
        // Never the whole signature.
        let (_, signature) = TEST_KEY.rsplit_once('.').unwrap();
        assert!(!key_hint(TEST_KEY).contains(&signature[..signature.len() - 6]));
    }

    #[test]
    fn the_notice_is_one_sentence_with_the_license_and_the_source_url() {
        assert_eq!(
            notice(Some("Acme IT")),
            concat!("Sent by Acme IT with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            notice(None),
            concat!("Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"))
        );
    }
}
