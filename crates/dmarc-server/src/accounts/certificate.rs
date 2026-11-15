//! The certificate a Microsoft login proves itself with: an RSA-2048 key ([`generate`]; the `rsa` crate generates
//! it, since ring cannot), a self-signed X.509 certificate for it (rcgen,
//! signed by ring) valid [`VALID_YEARS`], and the signed client assertion
//! each token request carries ([`client_assertion`], RS256 by ring).
//!
//! The certificate (DER) is uploaded to the app registration and stored in
//! the account's columns with its thumbprint and expiry; the private key is
//! the account's `private_key` secret, sealed like the others when a key is
//! set. A key generated for a form not saved yet waits in [`Drafts`], in
//! memory only, so an abandoned form leaves no key in the database.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{DateTime, Datelike, Utc};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_RSA_SHA256};
use ring::digest::{digest, SHA1_FOR_LEGACY_USE_ONLY, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::RsaPrivateKey;
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The key size: what Entra accepts and ring signs (2048 to 8192 bits).
pub const KEY_BITS: usize = 2048;
/// How long a generated certificate is valid.
pub const VALID_YEARS: i32 = 2;
/// How long a client assertion is valid: one token request.
pub const ASSERTION_LIFETIME_SECS: i64 = 600;
/// How long an unsaved form's certificate is kept: until the form is saved,
/// which comes before uploading it (a day, so an open form keeps it).
pub const DRAFT_TTL: Duration = Duration::from_hours(24);
/// At most this many unsaved certificates are kept; the oldest go first.
const MAX_DRAFTS: usize = 16;

/// A generated key and its certificate. The key is never printed by
/// [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct GeneratedCertificate {
    /// The private key, PKCS#8 PEM: the account's `private_key` secret.
    pub private_key_pem: String,
    pub cert_der: Vec<u8>,
    /// SHA-1 of the DER in upper-case hex: the thumbprint the Entra portal
    /// lists for an uploaded certificate, so the two can be matched by eye.
    pub thumbprint: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// The common name.
    pub subject: String,
}

impl fmt::Debug for GeneratedCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedCertificate")
            .field("private_key_pem", &"***")
            .field("thumbprint", &self.thumbprint)
            .field("expires_at", &self.expires_at)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

/// `der`'s SHA-1 in upper-case hex, as the Entra portal shows thumbprints.
pub fn thumbprint(der: &[u8]) -> String {
    digest(&SHA1_FOR_LEGACY_USE_ONLY, der)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

/// `der`'s SHA-256, base64url without padding: the assertion's `x5t#S256`.
pub fn x5t_s256(der: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(digest(&SHA256, der))
}

/// The day `years` after `day` (29 February becomes the 28th).
fn years_later(day: DateTime<Utc>, years: i32) -> (i32, u8, u8) {
    let year = day.year() + years;
    let (month, mut d) = (day.month(), day.day());
    if month == 2 && d == 29 {
        d = 28;
    }
    (
        year,
        u8::try_from(month).unwrap_or(1),
        u8::try_from(d).unwrap_or(1),
    )
}

/// A new RSA-2048 key and a self-signed certificate for it with `subject`
/// as its common name (the account's address or name; "Marcan" when
/// empty), valid from today (midnight UTC) for [`VALID_YEARS`]. Slow (key
/// generation): call it off the async threads.
pub fn generate(subject: &str, now: i64) -> Result<GeneratedCertificate, String> {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, KEY_BITS)
        .map_err(|e| format!("RSA key generation failed: {e}"))?;
    let pkcs8 = key
        .to_pkcs8_der()
        .map_err(|e| format!("the key could not be encoded: {e}"))?;
    let pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| format!("the key could not be encoded: {e}"))?;
    let key_pair = KeyPair::try_from(pkcs8.as_bytes())
        .map_err(|e| format!("the key could not be read for signing: {e}"))?;
    if key_pair.algorithm() != &PKCS_RSA_SHA256 {
        return Err("the key is not read as an RSA key".to_owned());
    }

    let subject = match subject.trim() {
        "" => "Marcan",
        s => s,
    };
    let today = DateTime::<Utc>::from_timestamp(now, 0).unwrap_or_default();
    let mut params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| format!("the certificate could not be prepared: {e}"))?;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, subject);
    params.distinguished_name = name;
    params.not_before = rcgen::date_time_ymd(
        today.year(),
        u8::try_from(today.month()).unwrap_or(1),
        u8::try_from(today.day()).unwrap_or(1),
    );
    let (y, m, d) = years_later(today, VALID_YEARS);
    params.not_after = rcgen::date_time_ymd(y, m, d);
    let expires_at = params.not_after.unix_timestamp();
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("the certificate could not be signed: {e}"))?;
    let cert_der = cert.der().to_vec();
    Ok(GeneratedCertificate {
        private_key_pem: pem.to_string(),
        thumbprint: thumbprint(&cert_der),
        cert_der,
        expires_at,
        subject: subject.to_owned(),
    })
}

/// The DER inside a PEM block (any label); `None` when it is no PEM.
pub fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    STANDARD.decode(body).ok().filter(|der| !der.is_empty())
}

/// The signed client assertion (RFC 7523) of one token request: RS256 with
/// the private key `private_key_pem`, the header naming the certificate by
/// its SHA-256 thumbprint (`x5t#S256`), `iss` and `sub` the client ID,
/// `aud` the token endpoint, a random `jti`, valid from `now` for
/// [`ASSERTION_LIFETIME_SECS`].
pub fn client_assertion(
    private_key_pem: &str,
    cert_der: &[u8],
    client_id: &str,
    audience: &str,
    now: i64,
) -> Result<String, String> {
    let pkcs8 = pem_der(private_key_pem).ok_or("the stored key is no PEM")?;
    let key = RsaKeyPair::from_pkcs8(&pkcs8).map_err(|e| format!("the stored key: {e}"))?;
    let rng = SystemRandom::new();
    let mut jti = [0u8; 16];
    rng.fill(&mut jti).map_err(|_| "no randomness")?;
    let header = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT",
        "x5t#S256": x5t_s256(cert_der),
    });
    let claims = serde_json::json!({
        "aud": audience,
        "iss": client_id,
        "sub": client_id,
        "jti": jti.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "nbf": now,
        "iat": now,
        "exp": now + ASSERTION_LIFETIME_SECS,
    });
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut signature = vec![0u8; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &rng,
        signing_input.as_bytes(),
        &mut signature,
    )
    .map_err(|_| "signing failed")?;
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

/// The certificates generated for forms not saved yet, by a random id, each
/// for [`DRAFT_TTL`]: saving the form takes its certificate
/// ([`Drafts::take`]), so only a saved one reaches the database.
#[derive(Default)]
pub struct Drafts {
    inner: Mutex<HashMap<String, (Instant, GeneratedCertificate)>>,
}

impl Drafts {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, GeneratedCertificate)>> {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, (at, _)| at.elapsed() < DRAFT_TTL);
        map
    }

    /// Keep `cert` for an unsaved form; its id.
    pub fn insert(&self, cert: GeneratedCertificate) -> String {
        let mut bytes = [0u8; 16];
        // SystemRandom only fails without an OS source of randomness, which
        // the key generation before this needed too.
        let _ = SystemRandom::new().fill(&mut bytes);
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut map = self.lock();
        while map.len() >= MAX_DRAFTS {
            let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            map.remove(&oldest);
        }
        map.insert(id.clone(), (Instant::now(), cert));
        id
    }

    /// The certificate of draft `id`, kept (to download it before saving).
    pub fn get(&self, id: &str) -> Option<GeneratedCertificate> {
        self.lock().get(id).map(|(_, cert)| cert.clone())
    }

    /// The certificate of draft `id`, no longer kept (the form was saved).
    pub fn take(&self, id: &str) -> Option<GeneratedCertificate> {
        self.lock().remove(id).map(|(_, cert)| cert)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ring::signature::{UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};
    use rsa::pkcs1::EncodeRsaPublicKey;
    use rsa::pkcs8::DecodePrivateKey;

    /// One key for the tests of this crate: generating takes a while.
    pub(crate) fn test_certificate() -> &'static GeneratedCertificate {
        static CERT: std::sync::OnceLock<GeneratedCertificate> = std::sync::OnceLock::new();
        CERT.get_or_init(|| {
            generate("dmarc@example.com", 1_793_145_600 + 3_600).expect("generated")
        })
    }

    /// A second key, not uploaded to the tests' app registration.
    pub(crate) fn other_certificate() -> &'static GeneratedCertificate {
        static CERT: std::sync::OnceLock<GeneratedCertificate> = std::sync::OnceLock::new();
        CERT.get_or_init(|| generate("other@example.com", 1_793_145_600).expect("generated"))
    }

    /// The PKCS#1 public key of `pem`, as ring verifies with it.
    pub(crate) fn public_key(pem: &str) -> Vec<u8> {
        let key = RsaPrivateKey::from_pkcs8_pem(pem).unwrap();
        key.to_public_key().to_pkcs1_der().unwrap().into_vec()
    }

    /// The header and claims of `jwt`, and whether its signature verifies
    /// with `public_key` (PKCS#1 DER).
    pub(crate) fn decode(
        jwt: &str,
        public_key: &[u8],
    ) -> (serde_json::Value, serde_json::Value, bool) {
        let mut parts = jwt.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let json = |part: &str| {
            serde_json::from_slice::<serde_json::Value>(&URL_SAFE_NO_PAD.decode(part).unwrap())
                .unwrap()
        };
        let signature = URL_SAFE_NO_PAD.decode(s).unwrap();
        let verified = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public_key)
            .verify(format!("{h}.{c}").as_bytes(), &signature)
            .is_ok();
        (json(h), json(c), verified)
    }

    #[test]
    fn a_generated_certificate_holds_its_key_and_signs_assertions() {
        let cert = test_certificate();
        assert!(cert
            .private_key_pem
            .starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(!format!("{cert:?}").contains("PRIVATE"));
        assert_eq!(cert.thumbprint.len(), 40);
        assert_eq!(cert.thumbprint, thumbprint(&cert.cert_der));
        // Two years from 2026-10-28, at midnight UTC.
        assert_eq!(cert.expires_at, 1_856_304_000);
        assert_eq!(cert.subject, "dmarc@example.com");
        // The certificate carries the key's public half and the subject.
        let public = public_key(&cert.private_key_pem);
        assert!(cert
            .cert_der
            .windows(public.len())
            .any(|w| w == public.as_slice()));
        assert!(cert.cert_der.windows(17).any(|w| w == b"dmarc@example.com"));

        let aud = "https://login.microsoftonline.com/t/oauth2/v2.0/token";
        let jwt =
            client_assertion(&cert.private_key_pem, &cert.cert_der, "client", aud, 100).unwrap();
        let (header, claims, verified) = decode(&jwt, &public);
        assert!(verified, "the signature verifies with the public key");
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["x5t#S256"], x5t_s256(&cert.cert_der));
        assert_eq!(claims["iss"], "client");
        assert_eq!(claims["sub"], "client");
        assert_eq!(claims["aud"], aud);
        assert_eq!(claims["exp"], 100 + ASSERTION_LIFETIME_SECS);
        assert_eq!(claims["jti"].as_str().unwrap().len(), 32);
        let again =
            client_assertion(&cert.private_key_pem, &cert.cert_der, "client", aud, 100).unwrap();
        let (_, claims2, _) = decode(&again, &public);
        assert_ne!(claims["jti"], claims2["jti"]);

        assert!(client_assertion("not a key", &cert.cert_der, "c", aud, 1).is_err());
    }

    #[test]
    fn a_leap_day_ends_on_the_28th() {
        let leap = DateTime::<Utc>::from_timestamp(1_709_164_800, 0).unwrap(); // 2024-02-29
        assert_eq!(years_later(leap, 2), (2026, 2, 28));
    }

    #[test]
    fn drafts_are_taken_once() {
        let drafts = Drafts::default();
        let id = drafts.insert(test_certificate().clone());
        assert_eq!(id.len(), 32);
        assert!(drafts.get(&id).is_some());
        assert!(drafts.take(&id).is_some());
        assert!(drafts.take(&id).is_none());
        assert!(drafts.get("nothing").is_none());
        for _ in 0..(MAX_DRAFTS + 3) {
            drafts.insert(test_certificate().clone());
        }
        assert_eq!(drafts.lock().len(), MAX_DRAFTS);
    }
}
