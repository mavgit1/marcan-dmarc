//! Live DNS validation for DMARC, SPF, DKIM, MTA-STS, TLS-RPT, and BIMI.
//!
//! This crate measures: it looks records up, parses them and reports what it
//! found as findings, placing each on `dmarc_assess`'s severity scale. The
//! vocabulary (`Finding`, `Severity`), the thresholds, the score, the grade
//! and every verdict come from `dmarc_assess`.
//!
//! Parsers and lookup counters are pure and unit-tested against hardcoded
//! records. Live checks use `hickory-resolver` and `reqwest` with timeouts.

#![forbid(unsafe_code)]

mod checker;
mod diff;
mod dns;
mod lookup;
mod resolver;

pub mod dkim;
pub mod dmarc;
pub mod dmarc_publish;
pub mod policy;
pub mod spf;

pub use checker::{assemble_report, CheckBudget, DnsChecker, DnsReport};
pub use diff::{diff_records, diff_reports, dns_state, record_facts};
pub use dns::{parse_ip, DnsError, ForwardLookup, IpError, PtrLookup};
pub use lookup::{LookupStatus, TxtAnswer, TxtLookup};
pub use resolver::{default_resolver, Resolver, TableResolver};
pub use spf::{spf_ip_match, SpfIpMatch};

/// Version of the checking and scoring logic that produced a grade.
///
/// Bump whenever a change here or in `dmarc_assess`'s DNS judgement would
/// alter the score, grade or a stored verdict for unchanged DNS (reclassifying
/// a severity, or adding a verdict field that older snapshots lack, counts;
/// rewording a message does not). Snapshots of another version are re-checked.
pub const CHECKER_VERSION: u32 = 12;
