//! `GET /api/reporting-setup`: how the monitored domains' DMARC reports reach
//! the mailbox, each domain's [`ReportingStatus`] decided by `dmarc_assess`
//! from the live lookups the endpoint does.

use dmarc_assess::{
    reporting_status, AuthState, DmarcLookup, MailboxAuthorization, ReportingSetupFacts,
    ReportingState, ReportingStatus, RuaFacts, Status,
};
use dmarc_dns::dmarc::{DmarcCheckResult, ExternalAuthorization};
use dmarc_dns::dmarc_publish::{new_record, with_report_address};
use dmarc_store::ReportingMode;
use serde::Serialize;
use ts_rs::TS;

/// The reporting setup of the monitored domains.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ReportingSetupResponse {
    /// The address `rua=` tags point at; empty while none is set, and then
    /// `domains` is empty too (nothing to check them against).
    pub report_address: String,
    /// The report address's domain; empty without an address.
    pub mailbox_domain: String,
    pub reporting_mode: ReportingMode,
    /// Direct mode with an address, the whole list asked for: whether the
    /// mailbox domain accepts other domains' reports. `null` otherwise.
    pub mailbox_authorization: Option<MailboxAuthorizationVerdict>,
    /// The domains (or the one asked for), the ones that need a DNS change
    /// first ([`ReportingStatus::rank`]), then by name.
    pub domains: Vec<ReportingDomain>,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export)]
pub struct MailboxAuthorizationVerdict {
    pub state: MailboxAuthorization,
    pub status: Status,
}

impl From<MailboxAuthorization> for MailboxAuthorizationVerdict {
    fn from(state: MailboxAuthorization) -> Self {
        Self {
            state,
            status: state.status(),
        }
    }
}

/// One domain's reporting setup.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ReportingDomain {
    pub id: i64,
    pub domain: String,
    /// Newest report's `date_end` (unix seconds).
    pub last_report_at: Option<i64>,
    pub state: ReportingStatus,
    pub status: Status,
    /// Nothing is left to publish ([`ReportingStatus::is_set_up`]).
    pub set_up: bool,
    /// The live DMARC check; `null` for a domain whose reports arrive, unless
    /// it alone was asked for (`domain_id`).
    pub dmarc: Option<DmarcCheckResult>,
    /// The record's `rua=` names the report address (any case).
    pub names_report_address: bool,
    /// The record's other `mailto:` addresses, as published: the forwarding
    /// addresses in forwarded mode.
    pub other_addresses: Vec<String>,
    /// The report address's domain does not accept this domain's reports:
    /// `<domain>._report._dmarc.<mailbox_domain>` (or the wildcard) is
    /// missing.
    pub mailbox_auth_missing: bool,
    /// The other `rua=` destinations that were looked up and refuse this
    /// domain's reports.
    pub refused: Vec<ExternalAuthorization>,
    /// The DMARC record to publish at `_dmarc.<domain>`: a new monitoring
    /// record naming the report address (no record), or the published one
    /// with the address added to `rua=` and every other tag as it is
    /// (address missing); `null` otherwise.
    pub record_to_publish: Option<String>,
}

/// What [`reporting_domain`] is built from.
pub struct ReportingDomainInput {
    pub id: i64,
    pub domain: String,
    pub last_report_at: Option<i64>,
    pub reporting: ReportingState,
    /// `None` when not looked up (only for a domain whose reports arrive).
    pub dmarc: Option<DmarcCheckResult>,
    /// The mailbox domain's authorization of this domain, when it was looked
    /// up (direct mode, another organizational domain).
    pub mailbox_auth: Option<AuthState>,
}

/// [`AuthState`] of an authorization lookup: `None` when it timed out or
/// failed.
pub fn auth_state(authorized: Option<bool>) -> AuthState {
    match authorized {
        Some(true) => AuthState::Authorized,
        Some(false) => AuthState::NotAuthorized,
        None => AuthState::CheckFailed,
    }
}

fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// One domain's row: what its record names, and its verdict.
pub fn reporting_domain(
    input: ReportingDomainInput,
    mode: ReportingMode,
    report_address: &str,
) -> ReportingDomain {
    let published = input
        .dmarc
        .as_ref()
        .filter(|c| c.record_count > 0)
        .and_then(|c| c.parsed.as_ref().map(|p| (c, p)));
    let mut names_report_address = false;
    let mut other_addresses = Vec::new();
    let mut other_auth = Vec::new();
    let mut report_address_entry = None;
    let mut refused = Vec::new();
    if let Some((check, parsed)) = published {
        let auth_of = |raw: &str| check.external_authorization.iter().find(|a| a.uri == raw);
        for uri in &parsed.rua {
            let Some(address) = uri.address.as_deref() else {
                continue;
            };
            let auth = auth_of(&uri.raw);
            if same_address(address, report_address) {
                names_report_address = true;
                report_address_entry = auth.map(|a| a.verdict.state);
                continue;
            }
            other_addresses.push(address.to_owned());
            other_auth.push(auth.map(|a| a.verdict.state));
            if let Some(a) = auth.filter(|a| a.verdict.state == AuthState::NotAuthorized) {
                refused.push(a.clone());
            }
        }
    }
    let dmarc = match (&input.dmarc, published) {
        (_, Some(_)) => DmarcLookup::Published(RuaFacts {
            names_report_address,
            other_addresses: other_auth,
        }),
        (Some(check), None) if check.lookup.is_measured() => DmarcLookup::NotPublished,
        _ => DmarcLookup::Failed,
    };
    let direct = mode == ReportingMode::Direct;
    // The record's own check of the report address, else the separate lookup
    // (the record does not name it yet, or direct mode looked it up).
    let report_address_auth = input.mailbox_auth.or(report_address_entry);
    let state = reporting_status(&ReportingSetupFacts {
        reporting: input.reporting,
        direct,
        dmarc,
        report_address_auth,
    });
    let arriving = state == ReportingStatus::Reporting;
    let record_to_publish = match state {
        ReportingStatus::NoRecord => Some(new_record(report_address)),
        ReportingStatus::AddressMissing => published
            .and_then(|(check, _)| check.records.first())
            .map(|record| with_report_address(record, report_address)),
        _ => None,
    };
    ReportingDomain {
        id: input.id,
        domain: input.domain,
        last_report_at: input.last_report_at,
        state,
        status: state.status(),
        set_up: state.is_set_up(),
        dmarc: input.dmarc,
        names_report_address,
        other_addresses,
        mailbox_auth_missing: !arriving && report_address_auth == Some(AuthState::NotAuthorized),
        refused: if arriving { Vec::new() } else { refused },
        record_to_publish,
    }
}

/// Worst first, then by name.
pub fn sort_domains(domains: &mut [ReportingDomain]) {
    domains.sort_by(|a, b| (a.state.rank(), &a.domain).cmp(&(b.state.rank(), &b.domain)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_dns::dmarc::evaluate_dmarc;
    use dmarc_dns::{TxtAnswer, TxtLookup};

    const ADDRESS: &str = "dmarc@reports.example.net";

    fn record(domain: &str, value: &str, auth: &[(&str, TxtLookup)]) -> DmarcCheckResult {
        let lookup = TxtLookup::Records(TxtAnswer::from_records(vec![value.into()]));
        let auth: Vec<(String, TxtLookup)> = auth
            .iter()
            .map(|(name, l)| ((*name).to_owned(), l.clone()))
            .collect();
        evaluate_dmarc(domain, &lookup, &auth)
    }

    fn input(
        dmarc: Option<DmarcCheckResult>,
        mailbox_auth: Option<AuthState>,
    ) -> ReportingDomainInput {
        ReportingDomainInput {
            id: 1,
            domain: "example.com".into(),
            last_report_at: None,
            reporting: ReportingState::NoReports,
            dmarc,
            mailbox_auth,
        }
    }

    fn accepted() -> TxtLookup {
        TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1".into()]))
    }

    #[test]
    fn a_record_naming_the_address_in_any_case_waits() {
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:DMARC@Reports.Example.net",
            &[("example.com._report._dmarc.reports.example.net", accepted())],
        );
        let row = reporting_domain(
            input(Some(check), Some(AuthState::Authorized)),
            ReportingMode::Direct,
            ADDRESS,
        );
        assert!(row.names_report_address);
        assert_eq!(row.state, ReportingStatus::WaitingForFirstReport);
        assert!(row.set_up);
        assert_eq!(row.record_to_publish, None);
        assert!(!row.mailbox_auth_missing);
        assert!(row.refused.is_empty());
    }

    #[test]
    fn a_refusing_mailbox_domain_is_not_authorized() {
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net",
            &[(
                "example.com._report._dmarc.reports.example.net",
                TxtLookup::NxDomain,
            )],
        );
        let row = reporting_domain(input(Some(check), None), ReportingMode::Forwarded, ADDRESS);
        assert_eq!(row.state, ReportingStatus::NotAuthorized);
        assert_eq!(row.status, Status::Fail);
        assert!(row.mailbox_auth_missing);
        // The mailbox domain's refusal is `mailbox_auth_missing`, not a second entry.
        assert!(row.refused.is_empty());
    }

    #[test]
    fn forwarders_and_their_refusals() {
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:a@vendor.example,mailto:dmarc@example.com",
            &[(
                "example.com._report._dmarc.vendor.example",
                TxtLookup::NxDomain,
            )],
        );
        let row = reporting_domain(input(Some(check), None), ReportingMode::Forwarded, ADDRESS);
        assert_eq!(
            row.other_addresses,
            ["a@vendor.example", "dmarc@example.com"]
        );
        assert_eq!(row.state, ReportingStatus::WaitingForFirstReport);
        assert_eq!(row.refused.len(), 1);
        assert_eq!(row.refused[0].host, "vendor.example");
        // Direct mode wants the report address itself.
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:dmarc@example.com",
            &[],
        );
        let row = reporting_domain(input(Some(check), None), ReportingMode::Direct, ADDRESS);
        assert_eq!(row.state, ReportingStatus::AddressMissing);
        // The record to publish is the published one with the address added.
        assert_eq!(
            row.record_to_publish.as_deref(),
            Some("v=DMARC1; p=none; rua=mailto:dmarc@example.com,mailto:dmarc@reports.example.net")
        );
    }

    #[test]
    fn no_record_and_failed_lookups() {
        let none = evaluate_dmarc("example.com", &TxtLookup::NxDomain, &[]);
        let row = reporting_domain(input(Some(none), None), ReportingMode::Direct, ADDRESS);
        assert_eq!(row.state, ReportingStatus::NoRecord);
        assert_eq!(
            row.record_to_publish.as_deref(),
            Some("v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net")
        );
        let failed = evaluate_dmarc("example.com", &TxtLookup::Timeout, &[]);
        let row = reporting_domain(input(Some(failed), None), ReportingMode::Direct, ADDRESS);
        assert_eq!(row.state, ReportingStatus::CheckFailed);
    }

    #[test]
    fn arriving_reports_need_no_lookup() {
        let mut i = input(None, None);
        i.reporting = ReportingState::Reporting;
        let row = reporting_domain(i, ReportingMode::Direct, ADDRESS);
        assert_eq!(row.state, ReportingStatus::Reporting);
        assert!(row.dmarc.is_none());
    }

    #[test]
    fn sorted_worst_first_then_by_name() {
        let row = |domain: &str, state: ReportingStatus| ReportingDomain {
            id: 0,
            domain: domain.into(),
            last_report_at: None,
            state,
            status: state.status(),
            set_up: state.is_set_up(),
            dmarc: None,
            names_report_address: false,
            other_addresses: vec![],
            mailbox_auth_missing: false,
            refused: vec![],
            record_to_publish: None,
        };
        let mut rows = vec![
            row("b.example", ReportingStatus::Reporting),
            row("c.example", ReportingStatus::NoRecord),
            row("a.example", ReportingStatus::Reporting),
        ];
        sort_domains(&mut rows);
        let names: Vec<_> = rows.iter().map(|r| r.domain.as_str()).collect();
        assert_eq!(names, ["c.example", "a.example", "b.example"]);
    }
}
