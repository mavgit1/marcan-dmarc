//! Client reports: one short monthly email per listed organization,
//! summarising its domains for someone who never opens Marcan.
//!
//! - [`list`] parses the recipient list (`organization,email` per line) that
//!   Settings → Client reports (Recipients) edits; the store
//!   keeps it in `client_report_recipients`.
//! - [`report`] reads one organization's report through a `Scope` pinned to
//!   it, and `mail::client_report` renders it.
//! - [`job`] sends every listed organization's report on the set day and
//!   hour, once a month, on the supervisor's helper.

pub mod job;
pub mod list;
pub mod report;
