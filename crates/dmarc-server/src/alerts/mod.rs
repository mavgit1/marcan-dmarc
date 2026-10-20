//! Alerts: the scheduled evaluation of every monitored domain and the alert
//! emails it leads to.
//!
//! - [`evaluate`] builds each domain's current `dmarc_assess::AlertState`
//!   from the facts the domain overview shows (the assessment and policy
//!   readiness over the readiness window, and the stored DNS result, whose
//!   DMARC record is the policy), compares it with the stored one
//!   (`dmarc_assess::alert_events`, with `dmarc_dns::diff_records` for the
//!   records) and stores the new state and the events in one transaction.
//! - [`job`] runs that on the supervisor's helper after each DNS warm pass
//!   (at most [`job::ALERT_INTERVAL`] apart), and holds it back while the mailbox
//!   imports (`job::ImportStatus::holds`, the hold stored as `alerts_held_at`).
//!
//! Alerts fire on changes only: a state that persists is never sent again.

pub mod evaluate;
pub mod job;
