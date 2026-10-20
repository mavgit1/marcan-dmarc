//! Domain figures: the domain list ([`list`]), one domain's overview
//! ([`overview`]), the mail its policy readiness is judged from
//! ([`readiness`]) and its failures ([`failures`]); the policy a domain shows
//! is picked by one rule ([`policy`]).
//!
//! Message counts come from the fact columns on `records` (see `outcome.rs`);
//! the per-check child tables are only read as JSON arrays per record, so a
//! record with three DKIM auth results cannot triple-count its messages.

mod failures;
mod list;
mod overview;
mod policy;
mod readiness;

pub use failures::{
    list_domain_failures, DomainFailureFilter, DomainFailureQuery, FailureOutcome, FailureRow,
    FailureSort,
};
pub use list::{
    count_domains, get_domain_summary, list_domain_summaries, page_domain_summaries,
    DomainListQuery, DomainSort, DomainSummaryRow,
};
pub use overview::{domain_exists, get_domain_overview, DomainOverviewRow};
pub use policy::{PolicySource, PublishedPolicy};
pub use readiness::{readiness_mail, ReadinessMailRow};

use serde::Serialize;
use sqlx::FromRow;
use ts_rs::TS;

// The columns the domain list and the overview both read.

/// The organization a domain belongs to. Read from the `organization_id`
/// and `organization_name` columns.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct DomainOrganization {
    #[sqlx(rename = "organization_id")]
    pub id: i64,
    #[sqlx(rename = "organization_name")]
    pub name: String,
}

/// A domain's stored DNS check as the domain list and page show it: its
/// grade and score, how many critical findings, and when. The whole result
/// is a [`DnsSnapshot`](crate::DnsSnapshot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct StoredDnsGrade {
    pub grade: String,
    pub score: i64,
    pub critical_count: i64,
    pub checked_at: i64,
    /// Checker version of the stored result; the API says `dns_updating` instead.
    #[serde(skip)]
    #[ts(skip)]
    pub checker_version: i64,
}

/// The `dns_<field>` columns of a `LEFT JOIN dns_snapshots s`: all null
/// without a stored check, else none is.
#[derive(Debug, FromRow)]
struct StoredDnsColumns {
    dns_grade: Option<String>,
    dns_score: Option<i64>,
    dns_critical_count: Option<i64>,
    dns_checked_at: Option<i64>,
    dns_checker_version: Option<i64>,
}

/// `s.grade AS dns_grade, …`: the [`StoredDnsColumns`] of `dns_snapshots s`.
const STORED_DNS_COLUMNS: &str = "s.grade AS dns_grade, s.score AS dns_score, \
     s.critical_count AS dns_critical_count, s.checked_at AS dns_checked_at, \
     s.checker_version AS dns_checker_version";

impl StoredDnsColumns {
    fn stored(self) -> Option<StoredDnsGrade> {
        Some(StoredDnsGrade {
            grade: self.dns_grade?,
            score: self.dns_score?,
            critical_count: self.dns_critical_count?,
            checked_at: self.dns_checked_at?,
            checker_version: self.dns_checker_version?,
        })
    }
}
