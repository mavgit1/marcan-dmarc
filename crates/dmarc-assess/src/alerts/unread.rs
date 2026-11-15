//! The "Account not read" alert: a receiving account has not been read
//! successfully for [`ACCOUNT_NOT_READ_DAYS`] (the rule the client reports
//! hold on, [`AccountReading::not_read`]). Raised once when an account
//! becomes unread ([`unread_events`]), and armed again once it is read: a
//! later outage raises again. Like the credential alert it is about the
//! installation, not a domain, and kept between evaluations in its own
//! state ([`UnreadAlertState`]).
//!
//! There is no baseline: an account already unread when it is first seen
//! is alerted.

use super::events::{AlertEvent, AlertKind};
use crate::receiving::{AccountReading, ACCOUNT_NOT_READ_DAYS};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// What the alert keeps between evaluations: the accounts alerted as not
/// read and not read since.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadAlertState {
    pub alerted: BTreeSet<i64>,
}

/// The alert's next state and its events from the `previous` state and the
/// receiving `accounts` at `now`: one event (a warning) for each account
/// not read that was not alerted yet. An account read again, or gone, is
/// forgotten, so its next outage raises again.
pub fn unread_events(
    previous: Option<&UnreadAlertState>,
    accounts: &[AccountReading],
    now: i64,
) -> (UnreadAlertState, Vec<AlertEvent>) {
    let previous = previous.cloned().unwrap_or_default();
    let mut state = UnreadAlertState::default();
    let mut events = Vec::new();
    for a in accounts.iter().filter(|a| a.not_read(now)) {
        state.alerted.insert(a.account_id);
        if !previous.alerted.contains(&a.account_id) {
            events.push(AlertEvent::new(
                AlertKind::AccountNotRead {
                    account_id: a.account_id,
                    account: a.account.clone(),
                    last_read_at: a.last_read_at,
                    unread_since: a.unread_since(),
                    days: ACCOUNT_NOT_READ_DAYS,
                    locked: a.locked,
                    error: a.last_error.clone(),
                },
                Status::Warn,
            ));
        }
    }
    (state, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::LockReason;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_793_145_600;

    fn reading(id: i64, last_read_at: Option<i64>) -> AccountReading {
        AccountReading {
            account_id: id,
            account: "Reports".into(),
            last_read_at,
            receiving_since: NOW - 30 * DAY,
            locked: None,
            last_error: Some("login failed".into()),
        }
    }

    fn event(id: i64, last_read_at: Option<i64>) -> AlertEvent {
        AlertEvent::new(
            AlertKind::AccountNotRead {
                account_id: id,
                account: "Reports".into(),
                last_read_at,
                unread_since: last_read_at.unwrap_or(NOW - 30 * DAY),
                days: 3,
                locked: None,
                error: Some("login failed".into()),
            },
            Status::Warn,
        )
    }

    /// Raised once when the account goes three days unread, not again
    /// while it stays unread; read again it is armed, and the next outage
    /// raises again.
    #[test]
    fn an_unread_account_is_alerted_once_per_outage() {
        let last = NOW - 3 * DAY;
        let (state, events) = unread_events(None, &[reading(1, Some(last))], NOW);
        assert!(events.is_empty(), "exactly three days: still read");
        assert!(state.alerted.is_empty());
        let (state, events) = unread_events(Some(&state), &[reading(1, Some(last))], NOW + 1);
        assert_eq!(events, [event(1, Some(last))]);
        let (state, events) = unread_events(Some(&state), &[reading(1, Some(last))], NOW + DAY);
        assert!(events.is_empty());
        assert_eq!(state.alerted.iter().copied().collect::<Vec<_>>(), [1]);

        // Read again: armed.
        let read = NOW + DAY;
        let (state, events) = unread_events(Some(&state), &[reading(1, Some(read))], read);
        assert!(events.is_empty());
        assert!(state.alerted.is_empty());
        let later = read + 3 * DAY + 1;
        let (_, events) = unread_events(Some(&state), &[reading(1, Some(read))], later);
        assert_eq!(events, [event(1, Some(read))]);
    }

    /// Never read, locked, several accounts, one gone; the state round-trips.
    #[test]
    fn never_read_locked_and_several_accounts() {
        let locked = AccountReading {
            locked: Some(LockReason::KeyChanged),
            last_error: None,
            ..reading(2, None)
        };
        let (state, events) = unread_events(None, &[reading(1, None), locked.clone()], NOW);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], event(1, None));
        match &events[1].kind {
            AlertKind::AccountNotRead {
                locked,
                last_read_at,
                ..
            } => {
                assert_eq!(*locked, Some(LockReason::KeyChanged));
                assert_eq!(*last_read_at, None);
            }
            other => panic!("{other:?}"),
        }
        let (state, events) = unread_events(Some(&state), &[locked], NOW);
        assert!(events.is_empty());
        assert_eq!(state.alerted.iter().copied().collect::<Vec<_>>(), [2]);
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<UnreadAlertState>(&json).unwrap(),
            state
        );
    }
}
