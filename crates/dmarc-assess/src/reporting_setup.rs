//! Whether a domain's DMARC aggregate reports reach Marcan: the reporting
//! setup's verdict per domain, and each report address domain's
//! authorization of other domains' reports (RFC 9990 §4).
//!
//! The known addresses are every receiving account's address and the
//! addresses forwarded to it (`forwarded_from`). The server does the live
//! lookups (the domain's DMARC record, the authorization records) and passes
//! what they found; this decides.

use crate::dns::AuthState;
use crate::domain::ReportingState;
use crate::status::Status;
use serde::Serialize;
use ts_rs::TS;

/// One domain's reporting setup. The order is the display order: the
/// domains that need a DNS change first, the ones whose reports arrive last
/// ([`ReportingStatus::rank`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ReportingStatus {
    /// No DMARC record is published.
    NoRecord,
    /// The record names known addresses, and the domain of every one of
    /// them refuses this domain's reports.
    NotAuthorized,
    /// A record is published but its `rua=` names no known address.
    AddressMissing,
    /// The DMARC record could not be read: the lookup timed out or failed.
    CheckFailed,
    /// The record names known addresses, none is accepted, and whether one
    /// is could not be looked up.
    AuthorizationCheckFailed,
    /// Set up; no report has arrived yet.
    WaitingForFirstReport,
    /// Set up; reports arrived before and then stopped.
    WaitingForReports,
    /// Reports arrive.
    Reporting,
}

impl ReportingStatus {
    /// Display order, lowest first: the ones that need a DNS change first.
    pub fn rank(self) -> u8 {
        self as u8
    }

    /// Nothing is left to publish: reports arrive, or the records are right
    /// and the first ones are on their way.
    pub fn is_set_up(self) -> bool {
        matches!(
            self,
            Self::WaitingForFirstReport | Self::WaitingForReports | Self::Reporting
        )
    }

    /// Its colour: missing DMARC or authorization loses every report (fail),
    /// a record without a known address is worth fixing (warn), a failed
    /// lookup or a wait states nothing (neutral).
    pub fn status(self) -> Status {
        match self {
            Self::NoRecord | Self::NotAuthorized => Status::Fail,
            Self::AddressMissing => Status::Warn,
            Self::CheckFailed
            | Self::AuthorizationCheckFailed
            | Self::WaitingForFirstReport
            | Self::WaitingForReports => Status::Neutral,
            Self::Reporting => Status::Pass,
        }
    }
}

/// The domain's live DMARC lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmarcLookup {
    /// Timed out or failed: nothing was measured.
    Failed,
    /// Measured: no record.
    NotPublished,
    /// A record, and what its `rua=` names.
    Published(RuaFacts),
}

/// What a published record's `rua=` names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuaFacts {
    /// Each known address it names (a receiving account's address or the
    /// address forwarded to one), with its domain's authorization of this
    /// domain's reports: `None` when none is needed (the same organizational
    /// domain).
    pub known: Vec<Option<AuthState>>,
    /// How many other `mailto:` addresses it names: Marcan cannot see where
    /// they go, so they settle nothing.
    pub other: usize,
}

/// What one domain's [`ReportingStatus`] is decided from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportingSetupFacts {
    /// Whether its reports arrive (see [`crate::reporting`]).
    pub reporting: ReportingState,
    pub dmarc: DmarcLookup,
}

/// One domain's reporting setup. Reports that arrive settle it. Otherwise the
/// record decides: none, or one that cannot be read; then the known addresses
/// it names. One of them accepted (or on the domain's own organizational
/// domain) is enough, and the domain waits for its first report, or for
/// reports again. With none accepted, a failed authorization lookup is not a
/// refusal. A record that names no known address misses it, whatever else it
/// names.
pub fn reporting_status(facts: &ReportingSetupFacts) -> ReportingStatus {
    if facts.reporting == ReportingState::Reporting {
        return ReportingStatus::Reporting;
    }
    let rua = match &facts.dmarc {
        DmarcLookup::Failed => return ReportingStatus::CheckFailed,
        DmarcLookup::NotPublished => return ReportingStatus::NoRecord,
        DmarcLookup::Published(rua) => rua,
    };
    if rua.known.is_empty() {
        return ReportingStatus::AddressMissing;
    }
    let accepted = |auth: &Option<AuthState>| matches!(auth, None | Some(AuthState::Authorized));
    if rua.known.iter().any(accepted) {
        if facts.reporting == ReportingState::NoReports {
            ReportingStatus::WaitingForFirstReport
        } else {
            ReportingStatus::WaitingForReports
        }
    } else if rua.known.contains(&Some(AuthState::CheckFailed)) {
        ReportingStatus::AuthorizationCheckFailed
    } else {
        ReportingStatus::NotAuthorized
    }
}

/// Whether one report address domain accepts other domains' reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MailboxAuthorization {
    /// No listed domain outside its organizational domain sends reports
    /// there: nothing to publish.
    NotNeeded,
    /// `*._report._dmarc.<address domain>` is published: every domain may
    /// send its reports there, also ones added later.
    Wildcard,
    /// No wildcard, but every domain that needs it is accepted by a record
    /// of its own.
    PerDomain,
    /// No wildcard, and a domain that needs it is not accepted yet (or none
    /// is listed): the per-domain rows say which.
    NoWildcard,
    /// The wildcard lookup timed out or failed.
    CheckFailed,
}

impl MailboxAuthorization {
    /// Published (either way) passes; the rest states nothing on its own.
    pub fn status(self) -> Status {
        match self {
            Self::Wildcard | Self::PerDomain => Status::Pass,
            Self::NotNeeded | Self::NoWildcard | Self::CheckFailed => Status::Neutral,
        }
    }
}

/// One address domain's authorization: `wildcard` is the wildcard lookup,
/// `None` when no listed domain needs one (and at least one is listed);
/// `external` the authorization of each listed domain that needs it.
pub fn mailbox_authorization(
    wildcard: Option<AuthState>,
    external: &[AuthState],
) -> MailboxAuthorization {
    match wildcard {
        None => MailboxAuthorization::NotNeeded,
        Some(AuthState::Authorized) => MailboxAuthorization::Wildcard,
        Some(AuthState::CheckFailed) => MailboxAuthorization::CheckFailed,
        Some(AuthState::NotAuthorized) => {
            if !external.is_empty() && external.iter().all(|a| *a == AuthState::Authorized) {
                MailboxAuthorization::PerDomain
            } else {
                MailboxAuthorization::NoWildcard
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AuthState::{Authorized, NotAuthorized};

    fn facts(dmarc: DmarcLookup) -> ReportingSetupFacts {
        ReportingSetupFacts {
            reporting: ReportingState::NoReports,
            dmarc,
        }
    }

    fn rua(known: Vec<Option<AuthState>>, other: usize) -> DmarcLookup {
        DmarcLookup::Published(RuaFacts { known, other })
    }

    fn status(known: Vec<Option<AuthState>>) -> ReportingStatus {
        reporting_status(&facts(rua(known, 0)))
    }

    const FAILED: Option<AuthState> = Some(AuthState::CheckFailed);

    #[test]
    fn arriving_reports_settle_it_whatever_dns_says() {
        for dmarc in [
            DmarcLookup::Failed,
            DmarcLookup::NotPublished,
            rua(vec![], 2),
            rua(vec![Some(NotAuthorized)], 0),
        ] {
            let mut f = facts(dmarc);
            f.reporting = ReportingState::Reporting;
            assert_eq!(reporting_status(&f), ReportingStatus::Reporting);
        }
    }

    #[test]
    fn the_record_comes_first() {
        assert_eq!(
            reporting_status(&facts(DmarcLookup::Failed)),
            ReportingStatus::CheckFailed
        );
        assert_eq!(
            reporting_status(&facts(DmarcLookup::NotPublished)),
            ReportingStatus::NoRecord
        );
    }

    #[test]
    fn a_record_naming_no_known_address_misses_it() {
        assert_eq!(status(vec![]), ReportingStatus::AddressMissing);
        // Other addresses may forward somewhere; Marcan cannot see where.
        assert_eq!(
            reporting_status(&facts(rua(vec![], 3))),
            ReportingStatus::AddressMissing
        );
    }

    #[test]
    fn one_known_address_accepted_or_needing_nothing_is_enough() {
        assert_eq!(status(vec![None]), ReportingStatus::WaitingForFirstReport);
        assert_eq!(
            status(vec![Some(Authorized)]),
            ReportingStatus::WaitingForFirstReport
        );
        assert_eq!(
            status(vec![Some(NotAuthorized), Some(Authorized)]),
            ReportingStatus::WaitingForFirstReport
        );
        assert_eq!(
            status(vec![FAILED, None]),
            ReportingStatus::WaitingForFirstReport
        );
    }

    #[test]
    fn with_none_accepted_a_failed_check_is_not_a_refusal() {
        assert_eq!(
            status(vec![FAILED]),
            ReportingStatus::AuthorizationCheckFailed
        );
        assert_eq!(
            status(vec![Some(NotAuthorized), FAILED]),
            ReportingStatus::AuthorizationCheckFailed
        );
        assert_eq!(
            status(vec![Some(NotAuthorized)]),
            ReportingStatus::NotAuthorized
        );
        assert_eq!(
            status(vec![Some(NotAuthorized), Some(NotAuthorized)]),
            ReportingStatus::NotAuthorized
        );
    }

    /// Two receiving accounts, one with a forwarded-from address: a record
    /// is judged by the known addresses it names, whatever else it names.
    #[test]
    fn several_accounts_each_count() {
        // Account 1's address refuses, account 2's (the same organizational
        // domain as the reporting one) needs nothing.
        assert_eq!(
            reporting_status(&facts(rua(vec![Some(NotAuthorized), None], 1))),
            ReportingStatus::WaitingForFirstReport
        );
        // Only the forwarded-from address, on a domain that refuses.
        assert_eq!(
            reporting_status(&facts(rua(vec![Some(NotAuthorized)], 2))),
            ReportingStatus::NotAuthorized
        );
        // Both accounts' addresses, both lookups failed.
        assert_eq!(
            reporting_status(&facts(rua(vec![FAILED, FAILED], 0))),
            ReportingStatus::AuthorizationCheckFailed
        );
    }

    #[test]
    fn stopped_reports_wait_for_reports_again() {
        let mut f = facts(rua(vec![None], 0));
        f.reporting = ReportingState::Stopped;
        assert_eq!(reporting_status(&f), ReportingStatus::WaitingForReports);
    }

    #[test]
    fn order_set_up_and_colours() {
        use ReportingStatus as R;
        let all = [
            R::NoRecord,
            R::NotAuthorized,
            R::AddressMissing,
            R::CheckFailed,
            R::AuthorizationCheckFailed,
            R::WaitingForFirstReport,
            R::WaitingForReports,
            R::Reporting,
        ];
        assert!(all.windows(2).all(|w| w[0].rank() < w[1].rank()));
        let set_up: Vec<_> = all.iter().filter(|s| s.is_set_up()).collect();
        assert_eq!(
            set_up,
            [
                &R::WaitingForFirstReport,
                &R::WaitingForReports,
                &R::Reporting
            ]
        );
        assert_eq!(R::NoRecord.status(), Status::Fail);
        assert_eq!(R::NotAuthorized.status(), Status::Fail);
        assert_eq!(R::AddressMissing.status(), Status::Warn);
        assert_eq!(R::CheckFailed.status(), Status::Neutral);
        assert_eq!(R::WaitingForFirstReport.status(), Status::Neutral);
        assert_eq!(R::Reporting.status(), Status::Pass);
    }

    #[test]
    fn mailbox_authorization_states() {
        use AuthState::CheckFailed;
        assert_eq!(
            mailbox_authorization(None, &[]),
            MailboxAuthorization::NotNeeded
        );
        assert_eq!(
            mailbox_authorization(Some(Authorized), &[NotAuthorized]),
            MailboxAuthorization::Wildcard
        );
        assert_eq!(
            mailbox_authorization(Some(CheckFailed), &[Authorized]),
            MailboxAuthorization::CheckFailed
        );
        assert_eq!(
            mailbox_authorization(Some(NotAuthorized), &[Authorized, Authorized]),
            MailboxAuthorization::PerDomain
        );
        assert_eq!(
            mailbox_authorization(Some(NotAuthorized), &[Authorized, CheckFailed]),
            MailboxAuthorization::NoWildcard
        );
        // No domain listed yet: the wildcard is the only way to cover them.
        assert_eq!(
            mailbox_authorization(Some(NotAuthorized), &[]),
            MailboxAuthorization::NoWildcard
        );
        assert_eq!(MailboxAuthorization::PerDomain.status(), Status::Pass);
        assert_eq!(MailboxAuthorization::NoWildcard.status(), Status::Neutral);
    }
}
