//! Can an account's stored credentials be used: are they readable with the
//! secret key the server has, and how long until they expire.

use crate::status::Status;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A credential that expires (a Microsoft login's certificate) warns from
/// this many days before it does.
pub const CREDENTIAL_EXPIRING_DAYS: i64 = 30;

/// Why stored credentials cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LockReason {
    /// Encrypted, and the server has no secret key.
    KeyMissing,
    /// Encrypted with a key other than the server's.
    KeyChanged,
    /// Encrypted with the server's key but does not open: the stored value
    /// was damaged or altered.
    Damaged,
}

/// What an account's credentials are good for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CredentialCondition {
    /// Readable, and not about to expire (or never expiring).
    Ok,
    /// Not readable with the server's key: the account neither receives nor
    /// sends until the key is restored or the credentials are entered again.
    Locked,
    /// Expires within [`CREDENTIAL_EXPIRING_DAYS`].
    Expiring,
    /// Expired: the provider refuses it.
    Expired,
}

impl CredentialCondition {
    pub fn status(self) -> Status {
        match self {
            Self::Ok => Status::Pass,
            Self::Expiring => Status::Warn,
            Self::Locked | Self::Expired => Status::Fail,
        }
    }
}

/// The condition of credentials that are `lock`ed or readable, expiring at
/// `expires_at` (unix seconds; `None`: never), at `now`. A lock comes first:
/// a credential that cannot be read is no use whatever its date.
pub fn credential_condition(
    lock: Option<LockReason>,
    expires_at: Option<i64>,
    now: i64,
) -> CredentialCondition {
    if lock.is_some() {
        return CredentialCondition::Locked;
    }
    match expires_at {
        Some(at) if at <= now => CredentialCondition::Expired,
        Some(at) if at - now <= CREDENTIAL_EXPIRING_DAYS * 86_400 => CredentialCondition::Expiring,
        _ => CredentialCondition::Ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_comes_first_then_the_expiry() {
        let day = 86_400;
        let now = 1_000 * day;
        assert_eq!(
            credential_condition(None, None, now),
            CredentialCondition::Ok
        );
        assert_eq!(
            credential_condition(Some(LockReason::KeyMissing), None, now),
            CredentialCondition::Locked
        );
        assert_eq!(
            credential_condition(Some(LockReason::KeyChanged), Some(now - day), now),
            CredentialCondition::Locked
        );
        assert_eq!(
            credential_condition(None, Some(now + 31 * day), now),
            CredentialCondition::Ok
        );
        assert_eq!(
            credential_condition(None, Some(now + 30 * day), now),
            CredentialCondition::Expiring
        );
        assert_eq!(
            credential_condition(None, Some(now), now),
            CredentialCondition::Expired
        );
        assert_eq!(CredentialCondition::Locked.status(), Status::Fail);
        assert_eq!(CredentialCondition::Expiring.status(), Status::Warn);
        assert_eq!(CredentialCondition::Ok.status(), Status::Pass);
    }
}
