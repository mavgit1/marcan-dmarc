//! The credential alert: an account's Microsoft login certificate expires
//! within [`CREDENTIAL_EXPIRING_DAYS`](crate::CREDENTIAL_EXPIRING_DAYS), or
//! has expired. It follows the certificate's condition
//! ([`credential_events`]): one event (info) when it starts expiring, one
//! (warn) when it has expired; a new certificate, with another date, starts
//! over. (Its kind keeps the name `account_credential_expiring`, which
//! stored events and alert settings carry.)
//! Like the update alert it is about the installation, not a domain, and it
//! is kept between evaluations in its own state ([`CredentialAlertState`]).
//!
//! There is no baseline: a credential already expiring when it is first
//! seen is alerted (no Microsoft login predates the alert), one already
//! expired once, as expired.

use super::events::{AlertEvent, AlertKind};
use crate::credential::{credential_condition, CredentialCondition};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An account whose credential expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpiringCredential<'a> {
    pub account_id: i64,
    pub account: &'a str,
    /// Unix seconds.
    pub expires_at: i64,
}

/// How far an expiry date was alerted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStage {
    /// It expires within the warning window.
    Expiring,
    /// It has expired.
    Expired,
}

/// The expiry date an account was alerted for, and how far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertedCredential {
    /// Unix seconds.
    pub expires_at: i64,
    pub stage: CredentialStage,
}

/// What the credential alert keeps between evaluations: for each account,
/// the expiry date it was alerted for and how far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAlertState {
    pub alerted: BTreeMap<i64, AlertedCredential>,
}

/// The credential alert's next state and its events from the `previous`
/// state and the accounts' `credentials` at `now`: an event when a
/// credential enters a stage it was not alerted for at that date, info when
/// it expires within the warning window, warn once it has expired (only the
/// latter for one first seen expired). Accounts gone, or whose credential
/// now has another date, are forgotten, and a date outside the window
/// alerts again when its time comes.
pub fn credential_events(
    previous: Option<&CredentialAlertState>,
    credentials: &[ExpiringCredential<'_>],
    now: i64,
) -> (CredentialAlertState, Vec<AlertEvent>) {
    let previous = previous.cloned().unwrap_or_default();
    let mut state = CredentialAlertState::default();
    let mut events = Vec::new();
    for c in credentials {
        let stage = match credential_condition(None, Some(c.expires_at), now) {
            CredentialCondition::Expiring => CredentialStage::Expiring,
            CredentialCondition::Expired => CredentialStage::Expired,
            CredentialCondition::Ok | CredentialCondition::Locked => continue,
        };
        let alerted = previous
            .alerted
            .get(&c.account_id)
            .filter(|a| a.expires_at == c.expires_at)
            .map(|a| a.stage);
        if alerted.is_none_or(|was| was < stage) {
            let status = match stage {
                CredentialStage::Expiring => Status::Info,
                CredentialStage::Expired => Status::Warn,
            };
            events.push(AlertEvent::new(
                AlertKind::AccountCredentialExpiring {
                    account_id: c.account_id,
                    account: c.account.to_owned(),
                    expires_at: c.expires_at,
                    expired: stage == CredentialStage::Expired,
                },
                status,
            ));
        }
        state.alerted.insert(
            c.account_id,
            AlertedCredential {
                expires_at: c.expires_at,
                stage: alerted.map_or(stage, |was| was.max(stage)),
            },
        );
    }
    (state, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_793_145_600;

    fn credential(id: i64, expires_at: i64) -> ExpiringCredential<'static> {
        ExpiringCredential {
            account_id: id,
            account: "Reports",
            expires_at,
        }
    }

    fn event(id: i64, expires_at: i64, expired: bool) -> AlertEvent {
        AlertEvent::new(
            AlertKind::AccountCredentialExpiring {
                account_id: id,
                account: "Reports".into(),
                expires_at,
                expired,
            },
            if expired { Status::Warn } else { Status::Info },
        )
    }

    /// Thirty days before (info), once; then once more when it expires
    /// (warn); a renewed credential with a new date starts over in its own
    /// window.
    #[test]
    fn a_credential_is_alerted_when_expiring_and_when_expired() {
        let at = NOW + 30 * DAY;
        let (state, events) = credential_events(None, &[credential(1, at)], NOW - DAY);
        assert!(events.is_empty(), "31 days before: nothing yet");
        assert!(state.alerted.is_empty());
        let (state, events) = credential_events(Some(&state), &[credential(1, at)], NOW);
        assert_eq!(events, [event(1, at, false)]);
        let (state, events) = credential_events(Some(&state), &[credential(1, at)], NOW + DAY);
        assert!(events.is_empty(), "expiring was alerted for this date");
        let (state, events) = credential_events(Some(&state), &[credential(1, at)], at + DAY);
        assert_eq!(events, [event(1, at, true)], "expired: once more");
        let (state, events) = credential_events(Some(&state), &[credential(1, at)], at + 2 * DAY);
        assert!(events.is_empty());
        assert_eq!(
            state.alerted.get(&1),
            Some(&AlertedCredential {
                expires_at: at,
                stage: CredentialStage::Expired
            })
        );

        let renewed = at + 365 * DAY;
        let (state, events) = credential_events(Some(&state), &[credential(1, renewed)], at + DAY);
        assert!(events.is_empty());
        assert!(state.alerted.is_empty(), "the old date is forgotten");
        let (state, events) =
            credential_events(Some(&state), &[credential(1, renewed)], renewed - 10 * DAY);
        assert_eq!(events, [event(1, renewed, false)]);
        // A date entered again after it was alerted re-arms too.
        let (state, _) = credential_events(Some(&state), &[credential(1, at)], renewed - 10 * DAY);
        let (_, events) =
            credential_events(Some(&state), &[credential(1, renewed)], renewed - 9 * DAY);
        assert_eq!(events, [event(1, renewed, false)]);
    }

    /// A credential first seen expired is alerted once, as expired; each
    /// account on its own; an account gone is forgotten.
    #[test]
    fn expired_and_several_accounts() {
        let (state, events) = credential_events(
            None,
            &[credential(1, NOW - DAY), credential(2, NOW + 5 * DAY)],
            NOW,
        );
        assert_eq!(
            events,
            [event(1, NOW - DAY, true), event(2, NOW + 5 * DAY, false)]
        );
        let (state, events) = credential_events(Some(&state), &[credential(2, NOW + 5 * DAY)], NOW);
        assert!(events.is_empty());
        assert_eq!(state.alerted.keys().copied().collect::<Vec<_>>(), [2]);
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"alerted":{{"2":{{"expires_at":{},"stage":"expiring"}}}}}}"#,
                NOW + 5 * DAY
            )
        );
        assert_eq!(
            serde_json::from_str::<CredentialAlertState>(&json).unwrap(),
            state
        );
    }
}
