//! SQLite store for DMARC aggregate reports: schema, ingest and queries that
//! return plain rows. Verdicts are `dmarc-assess`'s; a row the API sends as-is
//! derives serde/ts-rs here, the server's `view/` adds the verdicts.
//!
//! - [`init`] opens the database (WAL, foreign keys, owner-only file) and runs
//!   the embedded `migrations/`.
//! - `ingest`: idempotent report ingestion; the per-record facts
//!   (`dmarc_assess::RecordFacts`) become columns on `records`.
//! - `outcome`: the four-way outcome and the DMARC verdict as SQL
//!   (`OUTCOME_EXPR`, `VERDICT_EXPR`), read by every count, filter and export;
//!   also [`MailCounts`].
//! - `queries`: the shared helpers and [`Scope`], which narrows every list and
//!   total. `sort`: server-side sort keys mapped to fixed columns.
//! - `queries_*`: one module per area (alerts, analytics, client reports, DKIM
//!   selectors, DNS and identity snapshots, domains, export, IMAP state,
//!   mapping, organizations, reports, senders, system).
//!   `queries_domain_stats/` is split by what it answers: the domain list, the
//!   overview, the policy source rule, the readiness sums, the failures.
//!   `queries_imap`: the IMAP worker's cursor, quarantine, attempts and passes.
//! - `record_checks`: a record's SPF and DKIM checks and override reasons as
//!   JSON, for the report view, the failures and the auth-issues export.
//! - `settings/`: typed [`AppSettings`], one module per group on the
//!   [`SettingsGroup`] trait. Each group is one JSON row in `app_settings`,
//!   read over its defaults (a field it lacks reads as the default) and
//!   written whole by [`save_settings`].

mod ingest;
mod outcome;
mod queries;
mod queries_alerts;
mod queries_analytics;
mod queries_client_reports;
mod queries_dkim;
mod queries_dns;
mod queries_domain_stats;
mod queries_domains;
mod queries_export;
mod queries_identity;
mod queries_imap;
mod queries_mapping;
mod queries_organizations;
mod queries_reports;
mod queries_senders;
mod queries_system;
mod record_checks;
mod settings;
mod sort;

pub use ingest::{ingest_report, ingest_reports, BatchIngestError, IngestResult};
pub use outcome::MailCounts;
pub use queries::Scope;
pub use queries_alerts::{
    alert_events_in, count_alert_events, delete_alert_events_older_than,
    domains_with_reports_stored_since_evaluated, get_alert_states, list_alert_events,
    mark_alert_events_notified, save_alert_evaluations, unnotified_alert_events, AlertEvaluation,
    AlertEventRow,
};
pub use queries_analytics::{
    get_sender_by_ip, list_timeseries, report_span, resolve_source_ip, scope_stats,
    OtherDomainPass, ScopeStats, SenderDetailRow, SenderDomainStatsRow, SenderReceiverBreakdown,
    SenderTargetCount, TimeseriesBucket, TimeseriesRow,
};
pub use queries_client_reports::{
    client_reports_last_sent_at, count_client_report_sends, list_client_report_failures,
    list_client_report_recipients, list_unsent_client_report_recipients,
    preview_client_report_recipients, record_client_report_failure, record_client_report_send,
    replace_client_report_recipients, ClientReportChanges, ClientReportFailure,
    ClientReportRecipient, FailedClientReport,
};
pub use queries_dkim::{
    list_observed_dkim_selectors, ObservedDkimSelector, OBSERVED_SELECTOR_WINDOW_SECS,
};
pub use queries_dns::{
    dns_coverage, dns_snapshot_is_fresh, get_dns_snapshot, get_dns_snapshots,
    list_monitored_domain_ids, list_monitored_domains, upsert_dns_snapshot, DnsCoverage,
    DnsSnapshot, SnapshotDmarc,
};
pub use queries_domain_stats::{
    count_domains, domain_exists, get_domain_overview, get_domain_summary, list_domain_failures,
    list_domain_summaries, page_domain_summaries, readiness_mail, DomainFailureFilter,
    DomainFailureQuery, DomainListQuery, DomainOrganization, DomainOverviewRow, DomainSort,
    DomainSummaryRow, FailureOutcome, FailureRow, FailureSort, PolicySource, PublishedPolicy,
    ReadinessMailRow, StoredDnsGrade,
};
pub use queries_domains::{
    create_domain, find_domain_by_name, get_or_create_domain, ignore_domain, move_domain,
    set_domain_parked, CreateDomainResult, Domain, MoveDomain,
};
pub use queries_export::{
    sender_export_ips, stream_auth_issue_export, stream_dns_snapshot_export, stream_sender_export,
    AuthIssueExportRow, DnsSnapshotExportRow, SenderExportRow,
};
pub use queries_identity::{
    get_identity_snapshots, identity_snapshot_is_fresh, list_sender_ips_by_volume,
    list_sender_names, replace_sender_names, spf_pass_domains, upsert_identity_snapshot,
    IdentitySnapshot,
};
pub use queries_imap::{
    clear_imap_attempts, count_imap_quarantine, get_cursor, get_imap_attempts, imap_pass_status,
    quarantine_imap_message, record_imap_attempt, record_imap_pass, reset_cursor, set_cursor,
    ImapAttemptKey, ImapPassRow, ImapPassStatus, IMAP_PASSES_KEPT,
};
pub use queries_mapping::{
    apply_domain_mapping, domain_mapping_rows, preview_domain_mapping, DomainStatus,
    MappingChanges, MappingEntry, MappingIgnore, MappingMove, MappingPlacement, MappingPlan,
};
pub use queries_organizations::{
    create_organization, delete_organization, get_default_organization, get_or_create_organization,
    get_organization_summary, list_organization_summaries, rename_organization, scope_options,
    DeleteOrganization, Organization, OrganizationSummary, OrganizationWrite, ScopeDomain,
    ScopeOrganization,
};
pub use queries_reports::{
    find_report_by_report_id, get_report_summary, list_records_for_report, list_reports, RecordRow,
    ReportListQuery, ReportSort, ReportSummaryRow, StoredReport,
};
pub use queries_senders::{list_senders, SenderListQuery, SenderSort, SenderSummaryRow};
pub use queries_system::{
    database_size_bytes, delete_reports_older_than, last_ingest_at, report_totals,
    retention_cutoff, ReportTotals,
};
pub use record_checks::{DkimCheckRow, OverrideReason, RecordChecks, SpfCheckRow};
pub use settings::{
    get_alerts_held_at, get_client_reports_held_at, get_custom_dkim_selectors,
    get_demo_dns_records, get_last_digest_at, get_logo, get_logo_info, get_setting, load_settings,
    save_settings, set_alerts_held_at, set_client_reports_held_at, set_custom_dkim_selectors,
    set_demo_dns_records, set_last_digest_at, set_logo, set_setting, AlertSettings, AppSettings,
    BrandingSettings, ClientReportSettings, DigestFrequency, DnsSettings, ImapSecurity,
    LicenseSettings, LogoInfo, LogoType, MailboxSettings, MonitoringSettings, OnboardingSettings,
    OnboardingState, PixelSize, ReportingMode, ReportingSettings, RetentionSettings, SendingGroup,
    SendingLogin, SendingSettings, SettingsGroup, SmtpSecurity, StoredLogo, Weekday,
    CLIENT_REPORT_DAY_RANGE, DEFAULT_CLIENT_REPORT_DAY, DEFAULT_CLIENT_REPORT_HOUR,
    DEFAULT_DIGEST_HOUR, DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS,
    DEFAULT_REPORT_FRESHNESS_WARNING_HOURS, DEFAULT_REPORT_RETENTION_MONTHS,
    DOMAIN_REPORTS_STOPPED_DAYS_RANGE, REPORT_FRESHNESS_WARNING_HOURS_RANGE,
    REPORT_RETENTION_MONTHS_RANGE,
};
pub use sort::{Sort, SortDir, SortKey};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqliteSynchronous};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Current time in unix seconds; `0` if the clock is before the epoch.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Errors from pool setup, migrations, or queries.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("JSON encode error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to prepare database file: {0}")]
    Io(#[from] std::io::Error),
}

/// Open the SQLite database at `path` (creating it and its parent directory
/// if missing), enable WAL + foreign keys, and run migrations.
pub async fn init(path: &Path) -> Result<SqlitePool, StoreError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        create_private_file(path)?;
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        // Every connection in WAL with synchronous=NORMAL: durable across app
        // crashes, only an OS crash can lose the last commits, never corrupt.
        // A commit then needs no fsync, only the checkpoints do.
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        // Negative = KiB. sqlx's default pool is 10 connections, so this caps
        // page cache at ~160 MB worst case while keeping a working set hot.
        .pragma("cache_size", "-16384");

    let pool = SqlitePool::connect_with(options).await?;

    // Path is relative to this crate: crates/dmarc-store → ../../migrations
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}

/// Pre-create the database file as owner-only (0600). The database holds the
/// IMAP password, and SQLite gives the `-wal` / `-shm` files the same mode.
#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => Err(e),
        _ => Ok(()),
    }
}

/// Elsewhere SQLite creates the file with default permissions.
#[cfg(not(unix))]
fn create_private_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
