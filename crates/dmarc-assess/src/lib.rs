//! What Marcan concludes from the figures: the one place for every threshold
//! and verdict the UI shows.
//!
//! The store counts messages; this crate turns counts into judgements (a
//! [`Status`] and a machine-readable state or kind). The UI only words and
//! colours them, so a rule changes here and nowhere else.
//!
//! Pure: no I/O, no clock (callers pass `now`).

mod alerts;
mod client_report;
mod dns;
mod domain;
mod mail;
mod pipeline;
mod readiness;
mod record;
mod reporting_setup;
mod status;
mod trend;

pub use alerts::{
    alert_events, alert_state, catalogue_entry, catalogue_id, worst_first, AlertEvent,
    AlertFigures, AlertGroup, AlertKind, AlertState, AlertedIssue, BimiRecordFacts, CatalogueKind,
    DeliveredFigures, DkimKeyFacts, DkimSelectorFacts, DmarcRecordFacts, DnsRecords, DnsState,
    MtaStsPolicyFacts, MtaStsRecordFacts, NewSender, Observed, OpenIssue, ReadinessEvidence,
    RecordChange, RecordDelta, SpfRecordFacts, StoppedFigures, TlsRptRecordFacts, ALERT_CATALOGUE,
    ALERT_FORMAT, NEW_SENDER_MIN_FAILURES,
};
pub use client_report::{
    parked_line, report_line, report_place, report_verdict, ParkedLine, ReportDomainFacts,
    ReportLine, ReportPlace, ReportVerdict,
};
pub use dns::{
    assign_points, authorization_summary, authorization_verdict, critical_count, dkim_summary,
    dmarc_record_status, dns_grade_status, dns_status, grade_from_score, key_status,
    mta_sts_fetch_status, mta_sts_max_age_short, mta_sts_mode_status, no_issue_above_info,
    pct_status, score_from_findings, selector_verdict, severity_points, sort_findings,
    spf_all_status, spf_void_lookups_exceeded, testing_status, worst_check, AuthState, AuthVerdict,
    CheckKind, DnsStatus, Finding, KeyStrength, SelectorFacts, SelectorState, SelectorVerdict,
    Severity, SpfLookupLoad, GRADE_BANDS, MTA_STS_GOOD_MAX_AGE, MTA_STS_MIN_MAX_AGE, RSA_GOOD_BITS,
    RSA_MIN_BITS, SPF_LOOKUP_LIMIT, SPF_LOOKUP_WARN_AT, SPF_VOID_LIMIT,
};
pub use domain::{
    assess_domain, attention_ranks, organization_attention, reporting, reports_stopped_days,
    sender_check, sender_dkim_check, sender_domain_verdict, spoofing_status, AttentionCandidate,
    DomainAssessment, DomainFacts, Issue, IssueKind, IssueSeverity, OrganizationAttention,
    Reporting, ReportingState, SenderCheck, SenderCheckState, SenderDomainVerdict,
    SenderVerdictKind, WorstDns,
};
pub use mail::{
    dkim_coverage, enforcement, grade_status, pass_rate, rate_status, CoverageState, DkimCoverage,
    Enforcement, EnforcementState, Health, HealthState, Mail, MailVerdicts, Rate,
    RATE_PASS_MIN_PCT, RATE_WARN_MIN_PCT,
};
pub use pipeline::{
    client_report_hold, client_report_retry, is_bulk_import, last_poll, pipeline_status,
    ClientReportHold, ClientReportRetry, LastPoll, MailboxState, Pipeline, PipelineFacts,
    PipelineHeadline, ReportsState, BULK_IMPORT_MIN_MESSAGES, CLIENT_REPORT_HOLD_AFTER_DAYS,
    CLIENT_REPORT_RETRY_DAYS,
};
pub use readiness::{
    policy_readiness, readiness_window_start, NotApplicableReason, PolicyReadiness, PolicyStep,
    PolicyTarget, ReadinessCriteria, ReadinessFacts, ReadinessReason, ReadinessState,
    ReadinessWindow, READINESS_MIN_MESSAGES, READINESS_MIN_POLICY_SHARE_PCT, READINESS_WINDOW_DAYS,
};
pub use record::{
    trusted_forward, AuthIssue, CheckState, DmarcVerdict, Outcome, RecordFacts, UnknownToken,
};
pub use reporting_setup::{
    mailbox_authorization, reporting_status, DmarcLookup, MailboxAuthorization,
    ReportingSetupFacts, ReportingStatus, RuaFacts,
};
pub use status::Status;
pub use trend::{
    count_trend, covers_previous, points_trend, previous_window, Trend, TrendDirection, TrendSense,
};
