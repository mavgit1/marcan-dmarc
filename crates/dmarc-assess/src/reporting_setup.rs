//! Whether a domain's DMARC aggregate reports reach Marcan's mailbox: the
//! reporting setup's verdict per domain, and the mailbox domain's
//! authorization of other domains' reports (RFC 7489 §7.1).
//!
//! The server does the live lookups (the domain's DMARC record, the
//! authorization records) and passes what they found; this decides.

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
    /// The reports go to an address whose domain does not accept this
    /// domain's reports: the mailbox domain (direct mode), or every
    /// forwarding address named (forwarded mode).
    NotAuthorized,
    /// A record is published but its `rua=` names neither the report address
    /// nor (forwarded mode) an address that could forward to it.
    AddressMissing,
    /// The DMARC record could not be read: the lookup timed out or failed.
    CheckFailed,
    /// The record is right, but whether the report address's domain accepts
    /// the reports could not be looked up.
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
    /// a record without the address is worth fixing (warn), a failed lookup
    /// or a wait states nothing (neutral).
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
    /// It names the report address.
    pub names_report_address: bool,
    /// Its other `mailto:` addresses, each with its domain's authorization of
    /// this domain's reports: `None` when none is needed (the same
    /// organizational domain).
    pub other_addresses: Vec<Option<AuthState>>,
}

/// What one domain's [`ReportingStatus`] is decided from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportingSetupFacts {
    /// Whether its reports arrive (see [`crate::reporting`]).
    pub reporting: ReportingState,
    /// Direct mode: every domain names the report address. Forwarded: each
    /// may name an address of its own that forwards to it.
    pub direct: bool,
    pub dmarc: DmarcLookup,
    /// Whether the report address's domain accepts this domain's reports
    /// (its own record or the wildcard): `None` when none is needed (the same
    /// organizational domain) or not looked up (forwarded mode, a record that
    /// does not name the report address).
    pub report_address_auth: Option<AuthState>,
}

/// One domain's reporting setup. Reports that arrive settle it. Otherwise the
/// record decides: none, or one that cannot be read; then, in direct mode, the
/// address in `rua=` (the first thing to change, also while the mailbox
/// domain refuses the reports: `mailbox_auth_missing` names that beside it)
/// and a mailbox domain that refuses the reports; in
/// forwarded mode the report address when named, else the forwarding
/// addresses, of which at least one must be accepted. A set-up domain waits
/// for its first report, or for reports again.
pub fn reporting_status(facts: &ReportingSetupFacts) -> ReportingStatus {
    if facts.reporting == ReportingState::Reporting {
        return ReportingStatus::Reporting;
    }
    let rua = match &facts.dmarc {
        DmarcLookup::Failed => return ReportingStatus::CheckFailed,
        DmarcLookup::NotPublished => return ReportingStatus::NoRecord,
        DmarcLookup::Published(rua) => rua,
    };
    let waiting = if facts.reporting == ReportingState::NoReports {
        ReportingStatus::WaitingForFirstReport
    } else {
        ReportingStatus::WaitingForReports
    };
    let by_report_address = || match facts.report_address_auth {
        Some(AuthState::NotAuthorized) => ReportingStatus::NotAuthorized,
        Some(AuthState::CheckFailed) => ReportingStatus::AuthorizationCheckFailed,
        Some(AuthState::Authorized) | None => waiting,
    };
    if facts.direct {
        if !rua.names_report_address {
            return ReportingStatus::AddressMissing;
        }
        return by_report_address();
    }
    if rua.names_report_address {
        return by_report_address();
    }
    if rua.other_addresses.is_empty() {
        return ReportingStatus::AddressMissing;
    }
    let refused = |auth: &Option<AuthState>| *auth == Some(AuthState::NotAuthorized);
    if rua.other_addresses.iter().all(refused) {
        ReportingStatus::NotAuthorized
    } else {
        waiting
    }
}

/// Whether the mailbox domain accepts other domains' reports (direct mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MailboxAuthorization {
    /// Every monitored domain shares the mailbox's organizational domain:
    /// nothing to publish.
    NotNeeded,
    /// `*._report._dmarc.<mailbox domain>` is published: every domain may
    /// send its reports there, also ones added later.
    Wildcard,
    /// No wildcard, but every domain outside the mailbox's organizational
    /// domain is accepted by a record of its own (or its reports arrive).
    PerDomain,
    /// No wildcard, and a domain outside it is not accepted yet (or none is
    /// listed): the per-domain rows say which.
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

/// The mailbox domain's authorization: `wildcard` is the wildcard lookup,
/// `None` when no listed domain needs one (and at least one is listed);
/// `external` the authorization of each listed domain outside the mailbox's
/// organizational domain (a domain whose reports arrive counts as
/// authorized).
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

    fn facts(direct: bool, dmarc: DmarcLookup) -> ReportingSetupFacts {
        ReportingSetupFacts {
            reporting: ReportingState::NoReports,
            direct,
            dmarc,
            report_address_auth: None,
        }
    }

    fn named() -> DmarcLookup {
        DmarcLookup::Published(RuaFacts {
            names_report_address: true,
            other_addresses: vec![],
        })
    }

    fn others(auth: Vec<Option<AuthState>>) -> DmarcLookup {
        DmarcLookup::Published(RuaFacts {
            names_report_address: false,
            other_addresses: auth,
        })
    }

    #[test]
    fn arriving_reports_settle_it_whatever_dns_says() {
        let mut f = facts(true, DmarcLookup::Failed);
        f.reporting = ReportingState::Reporting;
        assert_eq!(reporting_status(&f), ReportingStatus::Reporting);
        f.dmarc = DmarcLookup::NotPublished;
        f.report_address_auth = Some(AuthState::NotAuthorized);
        assert_eq!(reporting_status(&f), ReportingStatus::Reporting);
    }

    #[test]
    fn the_record_comes_first() {
        for direct in [true, false] {
            assert_eq!(
                reporting_status(&facts(direct, DmarcLookup::Failed)),
                ReportingStatus::CheckFailed
            );
            assert_eq!(
                reporting_status(&facts(direct, DmarcLookup::NotPublished)),
                ReportingStatus::NoRecord
            );
        }
    }

    #[test]
    fn direct_mode_needs_the_address_and_the_mailbox_domains_consent() {
        assert_eq!(
            reporting_status(&facts(true, others(vec![]))),
            ReportingStatus::AddressMissing
        );
        assert_eq!(
            reporting_status(&facts(true, named())),
            ReportingStatus::WaitingForFirstReport
        );
        let mut f = facts(true, named());
        f.report_address_auth = Some(AuthState::Authorized);
        assert_eq!(reporting_status(&f), ReportingStatus::WaitingForFirstReport);
        f.report_address_auth = Some(AuthState::NotAuthorized);
        assert_eq!(reporting_status(&f), ReportingStatus::NotAuthorized);
        f.report_address_auth = Some(AuthState::CheckFailed);
        assert_eq!(
            reporting_status(&f),
            ReportingStatus::AuthorizationCheckFailed
        );
    }

    #[test]
    fn the_missing_address_is_named_before_a_refusing_mailbox_domain() {
        // The record is the first thing to change, so its badge says so; the
        // authorization follows once the record names the address.
        let mut f = facts(true, others(vec![None]));
        f.report_address_auth = Some(AuthState::NotAuthorized);
        assert_eq!(reporting_status(&f), ReportingStatus::AddressMissing);
        f.report_address_auth = Some(AuthState::CheckFailed);
        assert_eq!(reporting_status(&f), ReportingStatus::AddressMissing);
        f.dmarc = named();
        f.report_address_auth = Some(AuthState::NotAuthorized);
        assert_eq!(reporting_status(&f), ReportingStatus::NotAuthorized);
    }

    #[test]
    fn direct_mode_ignores_forwarders() {
        assert_eq!(
            reporting_status(&facts(true, others(vec![None, None]))),
            ReportingStatus::AddressMissing
        );
    }

    #[test]
    fn forwarded_mode_waits_while_one_forwarder_is_accepted() {
        assert_eq!(
            reporting_status(&facts(false, others(vec![]))),
            ReportingStatus::AddressMissing
        );
        assert_eq!(
            reporting_status(&facts(false, others(vec![None]))),
            ReportingStatus::WaitingForFirstReport
        );
        assert_eq!(
            reporting_status(&facts(
                false,
                others(vec![
                    Some(AuthState::NotAuthorized),
                    Some(AuthState::Authorized)
                ])
            )),
            ReportingStatus::WaitingForFirstReport
        );
        assert_eq!(
            reporting_status(&facts(
                false,
                others(vec![
                    Some(AuthState::NotAuthorized),
                    Some(AuthState::NotAuthorized)
                ])
            )),
            ReportingStatus::NotAuthorized
        );
        // A failed lookup is not a refusal.
        assert_eq!(
            reporting_status(&facts(false, others(vec![Some(AuthState::CheckFailed)]))),
            ReportingStatus::WaitingForFirstReport
        );
    }

    #[test]
    fn forwarded_mode_naming_the_report_address_is_judged_like_direct() {
        let mut f = facts(false, named());
        assert_eq!(reporting_status(&f), ReportingStatus::WaitingForFirstReport);
        f.report_address_auth = Some(AuthState::NotAuthorized);
        assert_eq!(reporting_status(&f), ReportingStatus::NotAuthorized);
        f.report_address_auth = Some(AuthState::CheckFailed);
        assert_eq!(
            reporting_status(&f),
            ReportingStatus::AuthorizationCheckFailed
        );
    }

    #[test]
    fn stopped_reports_wait_for_reports_again() {
        let mut f = facts(true, named());
        f.reporting = ReportingState::Stopped;
        assert_eq!(reporting_status(&f), ReportingStatus::WaitingForReports);
    }

    #[test]
    fn order_set_up_and_colours() {
        use ReportingStatus::*;
        let all = [
            NoRecord,
            NotAuthorized,
            AddressMissing,
            CheckFailed,
            AuthorizationCheckFailed,
            WaitingForFirstReport,
            WaitingForReports,
            Reporting,
        ];
        assert!(all.windows(2).all(|w| w[0].rank() < w[1].rank()));
        let set_up: Vec<_> = all.iter().filter(|s| s.is_set_up()).collect();
        assert_eq!(
            set_up,
            [&WaitingForFirstReport, &WaitingForReports, &Reporting]
        );
        assert_eq!(NoRecord.status(), Status::Fail);
        assert_eq!(NotAuthorized.status(), Status::Fail);
        assert_eq!(AddressMissing.status(), Status::Warn);
        assert_eq!(CheckFailed.status(), Status::Neutral);
        assert_eq!(WaitingForFirstReport.status(), Status::Neutral);
        assert_eq!(Reporting.status(), Status::Pass);
    }

    #[test]
    fn mailbox_authorization_states() {
        use AuthState::*;
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
