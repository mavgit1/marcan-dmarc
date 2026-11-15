//! `GET /api/reporting-setup`: how the monitored domains' DMARC reports reach
//! the receiving accounts, each domain's [`ReportingStatus`] decided by
//! `dmarc_assess` from the live lookups the endpoint does.
//!
//! The known addresses are every receiving account's address and the
//! addresses forwarded to it, where a receiving account is one set up to read
//! its mailbox (receiving on, its login set, a password login's IMAP server too: the accounts
//! snapshot's `receiving`); a new record names the first one's address in
//! list order (`ReportAddresses::default_address`).

use dmarc_assess::{
    reporting_status, AuthState, DmarcLookup, MailboxAuthorization, ReportingSetupFacts,
    ReportingState, ReportingStatus, RuaFacts, Status,
};
use dmarc_dns::dmarc::{DmarcCheckResult, ExternalAuthorization};
use dmarc_dns::dmarc_publish::{new_record, with_report_address};
use dmarc_domain::{normalize_name, organizational_domain};
use dmarc_store::AccountRow;
use serde::Serialize;
use ts_rs::TS;

/// The reporting setup of the monitored domains.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ReportingSetupResponse {
    /// Every receiving account with its address, in list order; empty while
    /// none receives, and then `domains` is empty too (nothing to check them
    /// against).
    pub addresses: Vec<ReceivingAddress>,
    /// The address a new DMARC record names: the first receiving account's;
    /// empty while none receives.
    pub default_address: String,
    /// The whole list asked for: each known address domain's authorization
    /// of other domains' reports, the default address's first. Empty for one
    /// domain.
    pub authorizations: Vec<AddressAuthorization>,
    /// The domains (or the one asked for), the ones that need a DNS change
    /// first ([`ReportingStatus::rank`]), then by name.
    pub domains: Vec<ReportingDomain>,
}

/// One receiving account's report addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ReceivingAddress {
    pub account_id: i64,
    pub name: String,
    /// Reports sent here land in the account.
    pub address: String,
    /// The other addresses whose reports are forwarded to it, in the order
    /// entered; empty: none.
    pub forwarded_from: Vec<String>,
}

/// Whether one address domain accepts other domains' reports.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct AddressAuthorization {
    /// The address domain, lowercase.
    pub domain: String,
    /// Its organizational domain: domains under it need no authorization.
    pub org_domain: String,
    /// The known addresses on it.
    pub addresses: Vec<String>,
    pub state: MailboxAuthorization,
    pub status: Status,
}

impl AddressAuthorization {
    pub fn new(domain: &str, addresses: Vec<String>, state: MailboxAuthorization) -> Self {
        Self {
            org_domain: organizational_domain(domain).unwrap_or_else(|| domain.to_owned()),
            domain: domain.to_owned(),
            addresses,
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
    /// The known addresses the record's `rua=` names, as published.
    pub named_addresses: Vec<String>,
    /// The record's other `mailto:` addresses, as published.
    pub other_addresses: Vec<String>,
    /// The address domains whose authorization this domain's reports still
    /// need (`<domain>._report._dmarc.<address domain>` is missing): the
    /// domains of the known addresses the record names when every one of
    /// them refuses, or the default address's domain when the record names
    /// none and that domain refuses.
    pub missing_authorizations: Vec<String>,
    /// The other `rua=` destinations that were looked up and refuse this
    /// domain's reports.
    pub refused: Vec<ExternalAuthorization>,
    /// The DMARC record to publish at `_dmarc.<domain>`: a new monitoring
    /// record naming the default address (no record), or the published one
    /// with that address added to `rua=` and every other tag as it is
    /// (address missing); `null` otherwise.
    pub record_to_publish: Option<String>,
}

/// The addresses whose reports reach Marcan, read from the accounts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportAddresses {
    /// Every receiving account (set up to receive, an address set), in list order.
    pub accounts: Vec<ReceivingAddress>,
}

impl ReportAddresses {
    /// The receiving accounts of `accounts` (in their order): the caller
    /// passes the ones set up to read their mailbox (the snapshot's
    /// `receiving`); one with receiving off or no address is left out here.
    pub fn of<'a>(accounts: impl IntoIterator<Item = &'a AccountRow>) -> Self {
        Self {
            accounts: accounts
                .into_iter()
                .filter(|a| a.fields.receive_enabled && !a.fields.address.trim().is_empty())
                .map(|a| ReceivingAddress {
                    account_id: a.id,
                    name: a.fields.name.clone(),
                    address: a.fields.address.trim().to_owned(),
                    forwarded_from: a
                        .fields
                        .forwarded_from
                        .iter()
                        .map(|f| f.trim())
                        .filter(|f| !f.is_empty())
                        .map(str::to_owned)
                        .collect(),
                })
                .collect(),
        }
    }

    /// The address new records name: the first receiving account's; empty
    /// while none receives.
    pub fn default_address(&self) -> &str {
        self.accounts.first().map_or("", |a| a.address.as_str())
    }

    /// The default address's domain; empty while none receives.
    pub fn default_domain(&self) -> String {
        address_domain(self.default_address())
    }

    /// Every known address: each account's, then the ones forwarded to it.
    pub fn known(&self) -> impl Iterator<Item = &str> {
        self.accounts.iter().flat_map(|a| {
            std::iter::once(a.address.as_str()).chain(a.forwarded_from.iter().map(String::as_str))
        })
    }

    /// `address` is a known one (any case).
    pub fn is_known(&self, address: &str) -> bool {
        self.known().any(|k| same_address(k, address))
    }

    /// The known addresses' domains, each once with its addresses, the
    /// default address's first.
    pub fn domains(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        for address in self.known() {
            let domain = address_domain(address);
            if domain.is_empty() {
                continue;
            }
            match out.iter_mut().find(|(d, _)| *d == domain) {
                Some((_, addresses)) => addresses.push(address.to_owned()),
                None => out.push((domain, vec![address.to_owned()])),
            }
        }
        out
    }
}

/// The domain of an address, lowercase; empty without one.
pub fn address_domain(address: &str) -> String {
    address
        .rsplit_once('@')
        .map(|(_, domain)| normalize_name(domain))
        .unwrap_or_default()
}

/// What [`reporting_domain`] is built from.
pub struct ReportingDomainInput {
    pub id: i64,
    pub domain: String,
    pub last_report_at: Option<i64>,
    pub reporting: ReportingState,
    /// `None` when not looked up (only for a domain whose reports arrive).
    pub dmarc: Option<DmarcCheckResult>,
    /// The default address domain's authorization of this domain, when it
    /// was looked up (another organizational domain): what a record that
    /// does not name a known address yet will need.
    pub default_auth: Option<AuthState>,
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

/// A known address the record names, with its domain's authorization of
/// the domain's reports (`None`: none needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedAddress {
    pub address: String,
    pub domain: String,
    pub auth: Option<AuthState>,
}

/// The known addresses `check`'s record names, each with the record's own
/// authorization lookup; empty without a readable record.
pub fn named_known(check: Option<&DmarcCheckResult>, known: &ReportAddresses) -> Vec<NamedAddress> {
    let Some((check, parsed)) = check
        .filter(|c| c.record_count > 0)
        .and_then(|c| c.parsed.as_ref().map(|p| (c, p)))
    else {
        return Vec::new();
    };
    parsed
        .rua
        .iter()
        .filter_map(|uri| {
            let address = uri.address.as_deref()?;
            known.is_known(address).then(|| NamedAddress {
                address: address.to_owned(),
                domain: address_domain(address),
                auth: check
                    .external_authorization
                    .iter()
                    .find(|a| a.uri == uri.raw)
                    .map(|a| a.verdict.state),
            })
        })
        .collect()
}

/// One domain's row: what its record names, and its verdict.
pub fn reporting_domain(input: ReportingDomainInput, known: &ReportAddresses) -> ReportingDomain {
    let published = input
        .dmarc
        .as_ref()
        .filter(|c| c.record_count > 0)
        .and_then(|c| c.parsed.as_ref().map(|p| (c, p)));
    let named = named_known(input.dmarc.as_ref(), known);
    let mut other_addresses = Vec::new();
    let mut refused = Vec::new();
    if let Some((check, parsed)) = published {
        for uri in &parsed.rua {
            let Some(address) = uri.address.as_deref() else {
                continue;
            };
            if known.is_known(address) {
                continue;
            }
            other_addresses.push(address.to_owned());
            if let Some(a) = check
                .external_authorization
                .iter()
                .find(|a| a.uri == uri.raw && a.verdict.state == AuthState::NotAuthorized)
            {
                refused.push(a.clone());
            }
        }
    }
    let dmarc = match (&input.dmarc, published) {
        (_, Some(_)) => DmarcLookup::Published(RuaFacts {
            known: named.iter().map(|n| n.auth).collect(),
            other: other_addresses.len(),
        }),
        (Some(check), None) if check.lookup.is_measured() => DmarcLookup::NotPublished,
        _ => DmarcLookup::Failed,
    };
    let state = reporting_status(&ReportingSetupFacts {
        reporting: input.reporting,
        dmarc,
    });
    let arriving = state == ReportingStatus::Reporting;
    let default = known.default_address();
    let mut missing_authorizations = Vec::new();
    if !arriving {
        if named.is_empty() {
            if input.default_auth == Some(AuthState::NotAuthorized) {
                missing_authorizations.push(known.default_domain());
            }
        } else if state == ReportingStatus::NotAuthorized {
            for n in &named {
                if !missing_authorizations.contains(&n.domain) {
                    missing_authorizations.push(n.domain.clone());
                }
            }
        }
    }
    let record_to_publish = match state {
        _ if default.is_empty() => None,
        ReportingStatus::NoRecord => Some(new_record(default)),
        ReportingStatus::AddressMissing => published
            .and_then(|(check, _)| check.records.first())
            .map(|record| with_report_address(record, default)),
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
        named_addresses: named.into_iter().map(|n| n.address).collect(),
        other_addresses,
        missing_authorizations,
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
    use dmarc_store::AccountFields;

    const ADDRESS: &str = "dmarc@reports.example.net";

    fn account(id: i64, address: &str, forwarded_from: &[&str]) -> AccountRow {
        AccountRow {
            id,
            created_at: 0,
            receiving_since: 0,
            receive_epoch: 0,
            fields: AccountFields {
                name: format!("Account {id}"),
                address: address.into(),
                receive_enabled: true,
                forwarded_from: forwarded_from.iter().map(|&f| f.to_owned()).collect(),
                ..AccountFields::default()
            },
        }
    }

    fn one() -> ReportAddresses {
        ReportAddresses::of(&[account(1, ADDRESS, &[])])
    }

    /// Two accounts: the reports mailbox, and a second one that also gets
    /// the reports forwarded from `dmarc@example.com` and
    /// `dmarc@client.example`.
    fn two() -> ReportAddresses {
        ReportAddresses::of(&[
            account(1, ADDRESS, &[]),
            account(
                2,
                "reports@other.example",
                &["dmarc@example.com", "dmarc@client.example"],
            ),
        ])
    }

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
        default_auth: Option<AuthState>,
    ) -> ReportingDomainInput {
        ReportingDomainInput {
            id: 1,
            domain: "example.com".into(),
            last_report_at: None,
            reporting: ReportingState::NoReports,
            dmarc,
            default_auth,
        }
    }

    fn accepted() -> TxtLookup {
        TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1".into()]))
    }

    #[test]
    fn the_known_addresses_and_their_domains() {
        let mut rows = vec![
            account(1, " Reports@Central.example ", &["dmarc@example.com", " "]),
            account(2, "second@central.example", &[]),
            account(3, "off@example.org", &[]),
            account(4, "", &[]),
        ];
        rows[2].fields.receive_enabled = false;
        let known = ReportAddresses::of(&rows);
        assert_eq!(known.accounts.len(), 2);
        assert_eq!(known.default_address(), "Reports@Central.example");
        assert_eq!(known.default_domain(), "central.example");
        assert!(known.is_known("reports@central.EXAMPLE"));
        assert!(known.is_known("DMARC@example.com"));
        assert!(!known.is_known("off@example.org"));
        assert_eq!(
            known.domains(),
            [
                (
                    "central.example".to_owned(),
                    vec![
                        "Reports@Central.example".to_owned(),
                        "second@central.example".to_owned()
                    ]
                ),
                (
                    "example.com".to_owned(),
                    vec!["dmarc@example.com".to_owned()]
                ),
            ]
        );
        assert_eq!(ReportAddresses::default().default_address(), "");
    }

    #[test]
    fn a_record_naming_the_address_in_any_case_waits() {
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:DMARC@Reports.Example.net",
            &[("example.com._report._dmarc.reports.example.net", accepted())],
        );
        let row = reporting_domain(input(Some(check), Some(AuthState::Authorized)), &one());
        assert_eq!(row.named_addresses, ["DMARC@Reports.Example.net"]);
        assert_eq!(row.state, ReportingStatus::WaitingForFirstReport);
        assert!(row.set_up);
        assert_eq!(row.record_to_publish, None);
        assert!(row.missing_authorizations.is_empty());
        assert!(row.refused.is_empty());
    }

    #[test]
    fn a_refusing_address_domain_is_not_authorized() {
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net",
            &[(
                "example.com._report._dmarc.reports.example.net",
                TxtLookup::NxDomain,
            )],
        );
        let row = reporting_domain(input(Some(check), None), &one());
        assert_eq!(row.state, ReportingStatus::NotAuthorized);
        assert_eq!(row.status, Status::Fail);
        assert_eq!(row.missing_authorizations, ["reports.example.net"]);
        // The refusal is `missing_authorizations`, not a second entry.
        assert!(row.refused.is_empty());
    }

    #[test]
    fn the_forwarded_from_address_counts_and_needs_nothing_on_its_own_domain() {
        let check = record(
            "example.com",
            "v=DMARC1; p=none; rua=mailto:a@vendor.example,mailto:dmarc@example.com",
            &[(
                "example.com._report._dmarc.vendor.example",
                TxtLookup::NxDomain,
            )],
        );
        let row = reporting_domain(input(Some(check.clone()), None), &two());
        assert_eq!(row.named_addresses, ["dmarc@example.com"]);
        assert_eq!(row.other_addresses, ["a@vendor.example"]);
        assert_eq!(row.state, ReportingStatus::WaitingForFirstReport);
        assert_eq!(row.refused.len(), 1);
        assert_eq!(row.refused[0].host, "vendor.example");
        // With one account and no forwarded-from address, the record names
        // no known address: the default address is added to it.
        let row = reporting_domain(input(Some(check), None), &one());
        assert_eq!(row.state, ReportingStatus::AddressMissing);
        assert_eq!(
            row.record_to_publish.as_deref(),
            Some("v=DMARC1; p=none; rua=mailto:a@vendor.example,mailto:dmarc@example.com,mailto:dmarc@reports.example.net")
        );
    }

    #[test]
    fn every_forwarded_address_of_an_account_counts() {
        // Each client domain names its own forwarder; both reach account 2.
        let check = record(
            "client.example",
            "v=DMARC1; p=none; rua=mailto:DMARC@client.example",
            &[],
        );
        let row = reporting_domain(input(Some(check), None), &two());
        assert_eq!(row.named_addresses, ["DMARC@client.example"]);
        assert_eq!(row.state, ReportingStatus::WaitingForFirstReport);
        assert_eq!(
            two().known().collect::<Vec<_>>(),
            [
                ADDRESS,
                "reports@other.example",
                "dmarc@example.com",
                "dmarc@client.example"
            ]
        );
    }

    #[test]
    fn one_accepted_account_is_enough_and_all_refusing_name_each_domain() {
        // Account 1 refuses, account 2 accepts: set up.
        let check = record(
            "example.org",
            "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net,mailto:reports@other.example",
            &[
                (
                    "example.org._report._dmarc.reports.example.net",
                    TxtLookup::NxDomain,
                ),
                ("example.org._report._dmarc.other.example", accepted()),
            ],
        );
        let mut i = input(Some(check), None);
        i.domain = "example.org".into();
        let row = reporting_domain(i, &two());
        assert_eq!(row.state, ReportingStatus::WaitingForFirstReport);
        assert!(row.missing_authorizations.is_empty());
        // Both refuse: each domain is named once.
        let check = record(
            "example.org",
            "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net,mailto:reports@other.example",
            &[
                (
                    "example.org._report._dmarc.reports.example.net",
                    TxtLookup::NxDomain,
                ),
                (
                    "example.org._report._dmarc.other.example",
                    TxtLookup::NxDomain,
                ),
            ],
        );
        let mut i = input(Some(check), None);
        i.domain = "example.org".into();
        let row = reporting_domain(i, &two());
        assert_eq!(row.state, ReportingStatus::NotAuthorized);
        assert_eq!(
            row.missing_authorizations,
            ["reports.example.net", "other.example"]
        );
    }

    #[test]
    fn no_record_names_the_default_and_its_missing_authorization() {
        let none = evaluate_dmarc("example.com", &TxtLookup::NxDomain, &[]);
        let row = reporting_domain(
            input(Some(none.clone()), Some(AuthState::NotAuthorized)),
            &two(),
        );
        assert_eq!(row.state, ReportingStatus::NoRecord);
        assert_eq!(
            row.record_to_publish.as_deref(),
            Some("v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net")
        );
        assert_eq!(row.missing_authorizations, ["reports.example.net"]);
        let failed = evaluate_dmarc("example.com", &TxtLookup::Timeout, &[]);
        let row = reporting_domain(input(Some(failed), None), &one());
        assert_eq!(row.state, ReportingStatus::CheckFailed);
        // Without a receiving account there is no address to publish.
        let row = reporting_domain(input(Some(none), None), &ReportAddresses::default());
        assert_eq!(row.state, ReportingStatus::NoRecord);
        assert_eq!(row.record_to_publish, None);
    }

    #[test]
    fn arriving_reports_need_no_lookup() {
        let mut i = input(None, Some(AuthState::NotAuthorized));
        i.reporting = ReportingState::Reporting;
        let row = reporting_domain(i, &one());
        assert_eq!(row.state, ReportingStatus::Reporting);
        assert!(row.dmarc.is_none());
        assert!(row.missing_authorizations.is_empty());
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
            named_addresses: vec![],
            other_addresses: vec![],
            missing_authorizations: vec![],
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
