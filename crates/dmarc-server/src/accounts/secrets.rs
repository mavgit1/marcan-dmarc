//! Secrets at rest: the accounts' passwords and keys sealed with AES-256-GCM
//! (ring's `aead`) under `MARCAN_SECRET_KEY`.
//!
//! A sealed value carries a random 12-byte nonce and the key's fingerprint
//! ([`SecretKey::id`]); its additional data names the account and the kind of
//! secret, so a value copied onto another account or kind does not open.
//! Without a key, secrets are stored plain, as before 0.4. A value that the
//! server's key cannot open (sealed with another key, sealed while the
//! server has none, or damaged) locks its account: [`open`] answers the
//! [`LockReason`].
//!
//! [`SecretKeys::from_env`] reads the key from `MARCAN_SECRET_KEY` or the
//! file `MARCAN_SECRET_KEY_FILE` names, and the key it replaces from
//! `MARCAN_SECRET_KEY_PREVIOUS` (or `_PREVIOUS_FILE`): the values sealed
//! with that one are sealed again with the new one at start.

use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use dmarc_assess::LockReason;
use dmarc_store::{SecretKind, SecretRow};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use zeroize::Zeroizing;

/// The variable the key is read from.
pub const KEY_VAR: &str = "MARCAN_SECRET_KEY";

/// The variable the key it replaces is read from, while the stored values
/// are sealed again with the new one.
pub const PREVIOUS_KEY_VAR: &str = "MARCAN_SECRET_KEY_PREVIOUS";

/// Bytes of key material: AES-256.
pub const KEY_LEN: usize = 32;

/// Bytes of the fingerprint stored with each sealed value.
const KEY_ID_LEN: usize = 16;

/// Why `MARCAN_SECRET_KEY` (or the previous key) cannot be used. Never
/// contains the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyError(String);

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeyError {}

/// The key secrets are sealed with. Never printed by [`Debug`](fmt::Debug).
pub struct SecretKey {
    key: LessSafeKey,
    id: Vec<u8>,
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKey")
            .field("id", &hex(&self.id))
            .finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()
        .map(Zeroizing::new)
}

impl SecretKey {
    /// The key in variable `name` as given: 32 bytes written as 64 hex
    /// digits or in base64 (`openssl rand -base64 32`; with or without its
    /// padding, URL-safe base64 also read). Blank means no key (a Compose
    /// file passes the variable on empty when it is not set). Anything else
    /// is an error that names the expected form, never the value. The
    /// decoded bytes are wiped once the key is made.
    pub fn parse(name: &str, raw: &str) -> Result<Option<Self>, KeyError> {
        let text = raw.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let unpadded = text.trim_end_matches('=');
        let bytes = from_hex(text)
            .filter(|b| b.len() == KEY_LEN)
            .or_else(|| STANDARD_NO_PAD.decode(unpadded).ok().map(Zeroizing::new))
            .or_else(|| URL_SAFE_NO_PAD.decode(unpadded).ok().map(Zeroizing::new))
            .ok_or_else(|| {
                KeyError(format!(
                    "{name} is neither base64 nor hex. Generate one with `openssl rand -base64 32`, or leave it out for no encryption."
                ))
            })?;
        if bytes.len() != KEY_LEN {
            return Err(KeyError(format!(
                "{name} decodes to {} bytes, not {KEY_LEN}. Generate one with `openssl rand -base64 32`, or leave it out for no encryption.",
                bytes.len()
            )));
        }
        let mut array = Zeroizing::new([0u8; KEY_LEN]);
        array.copy_from_slice(&bytes);
        Ok(Some(Self::from_bytes(&array)))
    }

    /// The key named `name` (such as `MARCAN_SECRET_KEY`) from the
    /// environment `env` looks up: the variable itself, or the file that
    /// `{name}_FILE` names (a Docker secret: nothing in the environment or
    /// `docker inspect`). Both set is an error, and so is a file that cannot
    /// be read or is empty; blank variables count as unset.
    pub fn from_env(
        name: &str,
        env: &impl Fn(&str) -> Option<OsString>,
    ) -> Result<Option<Self>, KeyError> {
        let file_var = format!("{name}_FILE");
        let text = |var: &str| -> Result<Option<Zeroizing<String>>, KeyError> {
            match env(var) {
                None => Ok(None),
                Some(value) => value
                    .into_string()
                    .map(|v| Some(Zeroizing::new(v)).filter(|v| !v.trim().is_empty()))
                    .map_err(|_| KeyError(format!("{var} is not text."))),
            }
        };
        let value = text(name)?;
        let file = text(&file_var)?;
        match (value, file) {
            (Some(_), Some(_)) => Err(KeyError(format!(
                "{name} and {file_var} are both set. Set one of them."
            ))),
            (Some(value), None) => Self::parse(name, &value),
            (None, Some(path)) => {
                let path = Path::new(path.trim());
                let bytes = Zeroizing::new(std::fs::read(path).map_err(|e| {
                    KeyError(format!("{file_var}: cannot read {}: {e}", path.display()))
                })?);
                let contents = std::str::from_utf8(&bytes).map_err(|_| {
                    KeyError(format!("{file_var}: {} is not text.", path.display()))
                })?;
                if contents.trim().is_empty() {
                    return Err(KeyError(format!(
                        "{file_var}: {} is empty.",
                        path.display()
                    )));
                }
                let source = format!("The key in {file_var} ({})", path.display());
                Self::parse(&source, contents)
            }
            (None, None) => Ok(None),
        }
    }

    /// A key from its 32 bytes.
    pub fn from_bytes(bytes: &[u8; KEY_LEN]) -> Self {
        let key = UnboundKey::new(&AES_256_GCM, bytes).expect("an AES-256 key is 32 bytes");
        let mut input = Zeroizing::new(b"marcan secret key id v1\0".to_vec());
        input.extend_from_slice(bytes);
        Self {
            key: LessSafeKey::new(key),
            id: digest(&SHA256, &input).as_ref()[..KEY_ID_LEN].to_vec(),
        }
    }

    /// The key's fingerprint, stored beside each value it seals: which key a
    /// value needs, without anything that helps to guess the key.
    pub fn id(&self) -> &[u8] {
        &self.id
    }

    /// The fingerprint as hex, for the log.
    pub fn id_hex(&self) -> String {
        hex(&self.id)
    }

    /// `plain` sealed for account `account_id`'s secret of `kind`.
    pub fn seal(&self, account_id: i64, kind: SecretKind, plain: &str) -> SecretRow {
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .expect("the system's random number generator");
        let mut value = plain.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad(account_id, kind)),
                &mut value,
            )
            .expect("a secret is far below AES-GCM's length limit");
        SecretRow {
            kind,
            value,
            nonce: Some(nonce.to_vec()),
            key_id: Some(self.id.clone()),
        }
    }

    /// `row`, sealed with this key, sealed again with `next` (the key
    /// replacing this one); `None` when this key does not open it.
    pub fn reseal(&self, next: &Self, account_id: i64, row: &SecretRow) -> Option<SecretRow> {
        if row.key_id.as_deref() != Some(self.id()) {
            return None;
        }
        let plain = Zeroizing::new(self.open(account_id, row)?);
        Some(next.seal(account_id, row.kind, &plain))
    }

    fn open(&self, account_id: i64, row: &SecretRow) -> Option<String> {
        let nonce: [u8; NONCE_LEN] = row.nonce.as_deref()?.try_into().ok()?;
        let mut value = row.value.clone();
        let plain = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad(account_id, row.kind)),
                &mut value,
            )
            .ok()?;
        String::from_utf8(plain.to_vec()).ok()
    }
}

/// The additional data a value is sealed with: the account and the kind.
fn aad(account_id: i64, kind: SecretKind) -> Vec<u8> {
    format!("marcan account secret v1|{account_id}|{}", kind.as_str()).into_bytes()
}

/// The key secrets are sealed with, and the one it replaces.
#[derive(Debug, Default)]
pub struct SecretKeys {
    /// `MARCAN_SECRET_KEY`: what secrets are sealed with; `None`: plain.
    pub current: Option<SecretKey>,
    /// `MARCAN_SECRET_KEY_PREVIOUS`: the values it sealed are sealed again
    /// with the current key at start. Only with a current key.
    pub previous: Option<SecretKey>,
}

impl SecretKeys {
    /// Both keys from the environment `env` looks up (each from its
    /// variable or its `_FILE`). A previous key without a current one is an
    /// error: there is nothing to seal its values again with.
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Result<Self, KeyError> {
        let current = SecretKey::from_env(KEY_VAR, &env)?;
        let previous = SecretKey::from_env(PREVIOUS_KEY_VAR, &env)?;
        if previous.is_some() && current.is_none() {
            return Err(KeyError(format!(
                "{PREVIOUS_KEY_VAR} is set but {KEY_VAR} is not. Set the new key in {KEY_VAR} to encrypt the credentials again with it, or remove {PREVIOUS_KEY_VAR}."
            )));
        }
        Ok(Self { current, previous })
    }
}

/// What a save does to one secret, before it is sealed for the account
/// ([`Accounts::seal`](super::Accounts::seal)).
#[derive(Clone, PartialEq, Eq)]
pub enum SecretChange {
    Keep,
    Set(String),
    Clear,
}

impl fmt::Debug for SecretChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keep => f.write_str("Keep"),
            Self::Set(_) => f.write_str("Set(***)"),
            Self::Clear => f.write_str("Clear"),
        }
    }
}

/// `value` for account `account_id`'s secret of `kind`: sealed with `key`,
/// plain without one.
pub fn store(key: Option<&SecretKey>, account_id: i64, kind: SecretKind, value: &str) -> SecretRow {
    match key {
        Some(key) => key.seal(account_id, kind, value),
        None => SecretRow::plain(kind, value),
    }
}

/// The text of account `account_id`'s stored secret `row`, or why `key`
/// cannot read it. A plain value reads with or without a key.
pub fn open(
    key: Option<&SecretKey>,
    account_id: i64,
    row: &SecretRow,
) -> Result<String, LockReason> {
    if !row.is_encrypted() {
        return String::from_utf8(row.value.clone()).map_err(|_| LockReason::Damaged);
    }
    let key = key.ok_or(LockReason::KeyMissing)?;
    if row.key_id.as_deref() != Some(key.id()) {
        return Err(LockReason::KeyChanged);
    }
    // The key is the one it was sealed with, and it does not open: the
    // value, its nonce, or its account or kind was changed.
    key.open(account_id, row).ok_or(LockReason::Damaged)
}

/// Whether `key` reads `row`, without opening it: plain, or sealed with
/// `key` (a sealed value whose fingerprint matches is taken as readable; a
/// tampered one shows when it is opened).
pub fn lock_of(key: Option<&SecretKey>, row: &SecretRow) -> Option<LockReason> {
    if !row.is_encrypted() {
        return None;
    }
    match key {
        None => Some(LockReason::KeyMissing),
        Some(key) if row.key_id.as_deref() != Some(key.id()) => Some(LockReason::KeyChanged),
        Some(_) => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn key(byte: u8) -> SecretKey {
        SecretKey::from_bytes(&[byte; KEY_LEN])
    }

    use base64::engine::general_purpose::STANDARD;
    use std::collections::HashMap;

    fn parse(raw: &str) -> Result<Option<SecretKey>, KeyError> {
        SecretKey::parse(KEY_VAR, raw)
    }

    #[test]
    fn the_key_reads_as_base64_or_hex_and_blank_is_none() {
        let bytes = [7u8; KEY_LEN];
        let b64 = STANDARD.encode(bytes);
        assert!(b64.ends_with('='));
        for text in [
            format!(" {b64}\n"),
            b64.trim_end_matches('=').to_owned(),
            hex(&bytes).to_uppercase(),
            URL_SAFE_NO_PAD.encode(bytes),
            format!("{}=", URL_SAFE_NO_PAD.encode(bytes)),
        ] {
            let parsed = parse(&text).unwrap().unwrap();
            assert_eq!(parsed.id(), key(7).id(), "{text}");
        }
        assert!(parse("").unwrap().is_none());
        assert!(parse("  ").unwrap().is_none());
        assert_ne!(key(7).id(), key(8).id());
        assert!(!format!("{:?}", key(7)).contains(&b64));
    }

    #[test]
    fn a_key_that_is_not_32_bytes_is_refused_without_echoing_it() {
        for raw in [
            "not a key!",
            &STANDARD.encode([1u8; 16]),
            &hex(&[1u8; 31]),
            &STANDARD.encode([1u8; 33]),
        ] {
            let err = parse(raw).unwrap_err().to_string();
            assert!(err.starts_with("MARCAN_SECRET_KEY"), "{err}");
            assert!(err.contains("openssl rand -base64 32"), "{err}");
            assert!(!err.contains(raw), "{err}");
        }
    }

    /// An environment of `vars`.
    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn the_keys_come_from_their_variables_or_files() {
        let one = STANDARD.encode([1u8; KEY_LEN]);
        let two = hex(&[2u8; KEY_LEN]);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("key");
        std::fs::write(&file, format!("{two}\n")).unwrap();
        let file = file.to_str().unwrap();

        let keys = SecretKeys::from_env(env(&[])).unwrap();
        assert!(keys.current.is_none() && keys.previous.is_none());
        // Compose passes an unset variable on empty.
        let keys =
            SecretKeys::from_env(env(&[(KEY_VAR, ""), ("MARCAN_SECRET_KEY_FILE", " ")])).unwrap();
        assert!(keys.current.is_none());

        let keys = SecretKeys::from_env(env(&[(KEY_VAR, &one)])).unwrap();
        assert_eq!(keys.current.unwrap().id(), key(1).id());
        let keys = SecretKeys::from_env(env(&[
            ("MARCAN_SECRET_KEY_FILE", file),
            (PREVIOUS_KEY_VAR, &one),
        ]))
        .unwrap();
        assert_eq!(keys.current.unwrap().id(), key(2).id());
        assert_eq!(keys.previous.unwrap().id(), key(1).id());
        let keys = SecretKeys::from_env(env(&[
            (KEY_VAR, &one),
            ("MARCAN_SECRET_KEY_PREVIOUS_FILE", file),
        ]))
        .unwrap();
        assert_eq!(keys.previous.unwrap().id(), key(2).id());
    }

    #[test]
    fn a_key_that_cannot_be_used_stops_without_echoing_it() {
        let one = STANDARD.encode([1u8; KEY_LEN]);
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad");
        std::fs::write(&bad, "not-the-key-at-all").unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, "\n").unwrap();
        let missing = dir.path().join("missing");
        let (bad, empty, missing) = (
            bad.to_str().unwrap(),
            empty.to_str().unwrap(),
            missing.to_str().unwrap(),
        );
        let refused = |vars: &[(&str, &str)], says: &str| {
            let err = SecretKeys::from_env(env(vars)).unwrap_err().to_string();
            assert!(err.contains(says), "{err}");
            assert!(!err.contains(&one), "{err}");
            assert!(!err.contains("not-the-key-at-all"), "{err}");
        };
        refused(
            &[(KEY_VAR, &one), ("MARCAN_SECRET_KEY_FILE", bad)],
            "MARCAN_SECRET_KEY and MARCAN_SECRET_KEY_FILE are both set",
        );
        refused(
            &[("MARCAN_SECRET_KEY_FILE", bad)],
            "is neither base64 nor hex",
        );
        refused(&[("MARCAN_SECRET_KEY_FILE", empty)], "is empty");
        refused(&[("MARCAN_SECRET_KEY_FILE", missing)], "cannot read");
        refused(
            &[(PREVIOUS_KEY_VAR, &one)],
            "MARCAN_SECRET_KEY_PREVIOUS is set but MARCAN_SECRET_KEY is not",
        );
        refused(
            &[(KEY_VAR, &one), (PREVIOUS_KEY_VAR, "short")],
            "MARCAN_SECRET_KEY_PREVIOUS is neither",
        );
    }

    #[test]
    fn a_value_of_the_previous_key_is_sealed_again_with_the_new_one() {
        let (old, new) = (key(1), key(2));
        let row = old.seal(3, SecretKind::Password, "secret");
        let moved = old.reseal(&new, 3, &row).unwrap();
        assert_eq!(moved.key_id.as_deref(), Some(new.id()));
        assert_eq!(open(Some(&new), 3, &moved).unwrap(), "secret");
        // Only what the previous key sealed, and opens.
        assert!(new.reseal(&old, 3, &row).is_none());
        assert!(old.reseal(&new, 4, &row).is_none());
        assert!(old
            .reseal(&new, 3, &SecretRow::plain(SecretKind::Password, "p"))
            .is_none());
    }

    #[test]
    fn a_sealed_value_opens_only_with_its_key_account_and_kind() {
        let k = key(1);
        let row = k.seal(3, SecretKind::Password, " spaced secret ");
        assert!(row.is_encrypted());
        assert_eq!(row.nonce.as_ref().map(Vec::len), Some(NONCE_LEN));
        assert!(!row.value.windows(6).any(|w| w == b"secret"));
        assert_eq!(open(Some(&k), 3, &row).unwrap(), " spaced secret ");
        // A fresh nonce each time.
        assert_ne!(
            k.seal(3, SecretKind::Password, "x").nonce,
            k.seal(3, SecretKind::Password, "x").nonce
        );

        assert_eq!(open(None, 3, &row), Err(LockReason::KeyMissing));
        assert_eq!(open(Some(&key(2)), 3, &row), Err(LockReason::KeyChanged));
        // With its own key, a value moved or altered is damaged, not
        // sealed with another key.
        assert_eq!(open(Some(&k), 4, &row), Err(LockReason::Damaged));
        let moved = SecretRow {
            kind: SecretKind::PrivateKey,
            ..row.clone()
        };
        assert_eq!(open(Some(&k), 3, &moved), Err(LockReason::Damaged));
        let mut tampered = row.clone();
        tampered.value[0] ^= 1;
        assert_eq!(open(Some(&k), 3, &tampered), Err(LockReason::Damaged));

        assert_eq!(lock_of(Some(&k), &row), None);
        assert_eq!(lock_of(None, &row), Some(LockReason::KeyMissing));
        assert_eq!(lock_of(Some(&key(2)), &row), Some(LockReason::KeyChanged));
    }

    #[test]
    fn a_plain_value_reads_with_or_without_a_key() {
        let row = store(None, 1, SecretKind::Password, "plain");
        assert!(!row.is_encrypted());
        assert_eq!(open(None, 1, &row).unwrap(), "plain");
        assert_eq!(open(Some(&key(1)), 1, &row).unwrap(), "plain");
        assert_eq!(lock_of(None, &row), None);
        assert!(store(Some(&key(1)), 1, SecretKind::Password, "p").is_encrypted());
    }
}
