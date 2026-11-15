//! Is each receiving account read: the one rule the monthly client reports'
//! hold ([`client_report_hold`](crate::client_report_hold)) and the "Account
//! not read" alert ([`unread_events`](crate::unread_events)) share, so the
//! two never disagree.
//!
//! An account counts as unread from its last successful read, or from when
//! it started reading the mailbox it reads now (it was added, its receiving
//! switched on, or a save made it read another mailbox), whichever is later:
//! a new or replaced mailbox gets the same days as one that stopped being
//! read.

use crate::credential::LockReason;

/// A receiving account not read successfully for this many days is not
/// read: the client reports wait for it, and it is alerted.
pub const ACCOUNT_NOT_READ_DAYS: i64 = 3;

/// One account that receives reports, as far as its reading is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountReading {
    pub account_id: i64,
    /// Its name.
    pub account: String,
    /// When a pass last read it without error (unix seconds); `None`: not
    /// since it reads this mailbox.
    pub last_read_at: Option<i64>,
    /// When it started reading the mailbox it reads now (unix seconds).
    pub receiving_since: i64,
    /// Its credentials cannot be read with the server's key: its passes
    /// find nothing to log in with.
    pub locked: Option<LockReason>,
    /// The error of its newest pass, when that pass failed.
    pub last_error: Option<String>,
}

impl AccountReading {
    /// From when it counts as unread: its last successful read, or when it
    /// started reading this mailbox, whichever is later.
    pub fn unread_since(&self) -> i64 {
        self.last_read_at
            .map_or(self.receiving_since, |at| at.max(self.receiving_since))
    }

    /// Whether it has gone unread for more than [`ACCOUNT_NOT_READ_DAYS`]
    /// at `now`.
    pub fn not_read(&self, now: i64) -> bool {
        now - self.unread_since() > ACCOUNT_NOT_READ_DAYS * 86_400
    }
}

/// Of `accounts`, the one unread at `now` that has gone unread longest
/// (the first of equals); `None` while every one was read lately.
pub fn stalest_unread(accounts: &[AccountReading], now: i64) -> Option<&AccountReading> {
    accounts
        .iter()
        .filter(|a| a.not_read(now))
        .min_by_key(|a| a.unread_since())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_793_145_600;

    fn reading(id: i64, last_read_at: Option<i64>, since: i64) -> AccountReading {
        AccountReading {
            account_id: id,
            account: format!("Account {id}"),
            last_read_at,
            receiving_since: since,
            locked: None,
            last_error: None,
        }
    }

    /// Unread from the later of its last read and when it started reading
    /// this mailbox; not read after three days, to the second.
    #[test]
    fn an_account_is_unread_from_its_last_read_or_its_start() {
        let read = reading(1, Some(NOW - 3 * DAY), NOW - 30 * DAY);
        assert_eq!(read.unread_since(), NOW - 3 * DAY);
        assert!(!read.not_read(NOW));
        assert!(read.not_read(NOW + 1));
        // Never read: from when it was added or replaced.
        let new = reading(2, None, NOW - DAY);
        assert_eq!(new.unread_since(), NOW - DAY);
        assert!(!new.not_read(NOW));
        assert!(new.not_read(NOW + 2 * DAY + 1));
        // Receiving switched on again after a long pause: from then.
        let resumed = reading(3, Some(NOW - 30 * DAY), NOW - DAY);
        assert_eq!(resumed.unread_since(), NOW - DAY);
        assert!(!resumed.not_read(NOW));
    }

    #[test]
    fn the_stalest_unread_account_is_named() {
        let accounts = [
            reading(1, Some(NOW - DAY), 0),
            reading(2, Some(NOW - 5 * DAY), 0),
            reading(3, None, NOW - 9 * DAY),
            reading(4, None, NOW - 9 * DAY),
        ];
        assert_eq!(
            stalest_unread(&accounts, NOW).map(|a| a.account_id),
            Some(3)
        );
        assert_eq!(
            stalest_unread(&accounts[..2], NOW).map(|a| a.account_id),
            Some(2)
        );
        assert_eq!(stalest_unread(&accounts[..1], NOW), None);
        assert_eq!(stalest_unread(&[], NOW), None);
    }
}
