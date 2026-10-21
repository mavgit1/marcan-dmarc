//! One organization's report, read for rendering. Everything goes through a
//! `Scope` pinned to the organization, the same path the UI's organization
//! page takes: its domains with their figures and issues over the 30 days
//! before sending, the readiness facts, the calendar
//! month's mail against the month before (`Statistics`, as `GET
//! /api/statistics` computes it) and the month's changes. Nothing is read
//! for all domains and filtered here.

use crate::error::AppResult;
use crate::mail::client_report::{self, ClientReportInput, ReportDomain, ReportMonth};
use crate::mail::Content;
use crate::mail::{parse_address, Letterhead, SendError, SmtpConfig};
use crate::view::alerts::AlertItem;
use crate::view::domain::{domain_summaries, summary_readiness, Judging};
use crate::view::stats::{statistics, Statistics};
use dmarc_assess::{covers_previous, readiness_window_start};
use dmarc_store::{
    alert_events_in, count_domains, list_domain_summaries, readiness_mail, report_span,
    scope_stats, AppSettings, Scope,
};
use sqlx::SqlitePool;

/// The report of organization `organization_id` (named `organization`) as
/// of `now`: figures over the readiness window before `now`, changes in the
/// calendar month before `now`'s; under `letterhead`.
pub async fn organization_report(
    pool: &SqlitePool,
    organization_id: i64,
    organization: &str,
    now: i64,
    letterhead: &Letterhead,
) -> AppResult<Content> {
    let scope = Scope::new(Some(organization_id), None, None);
    let window = readiness_window_start(now);
    let month = ReportMonth::before(now);
    let rows = list_domain_summaries(pool, scope.window(Some(window), None)).await?;
    let events = alert_events_in(pool, scope.window(Some(month.start), Some(month.end))).await?;
    let mut domains: Vec<ReportDomain> = Vec::new();
    for summary in domain_summaries(rows, Judging::load(pool, now).await?) {
        let row = &summary.row;
        let mail = readiness_mail(
            pool,
            row.id,
            row.policy.as_ref().and_then(|p| p.p.as_deref()),
            window,
        )
        .await?;
        let readiness = summary_readiness(row, &mail, now);
        domains.push(ReportDomain { summary, readiness });
    }
    let mail = month_mail(pool, scope, &month).await?;
    // A new failing sender is the operator's to look into (its line points
    // to Marcan's sender page): the client's report leaves it out.
    let events: Vec<AlertItem> = events
        .into_iter()
        .map(AlertItem::from)
        .filter(|item| item.sender_ip.is_none())
        .collect();
    Ok(client_report::render(&ClientReportInput {
        organization,
        now,
        month,
        domains: &domains,
        events: &events,
        mail: &mail,
        letterhead,
    }))
}

/// The organization's mail in `month`, against the month before when its
/// reports reach back that far.
async fn month_mail(pool: &SqlitePool, scope: Scope, month: &ReportMonth) -> AppResult<Statistics> {
    let current = scope_stats(pool, scope.window(Some(month.start), Some(month.end))).await?;
    let previous = month.previous();
    let (first, _) = report_span(pool, scope).await?;
    let previous = if covers_previous(first, previous.start) {
        Some(scope_stats(pool, scope.window(Some(previous.start), Some(previous.end))).await?)
    } else {
        None
    };
    let domain_count = count_domains(pool, scope).await?;
    Ok(statistics(current, previous, domain_count))
}

/// The sending account for client reports: the stored one, with the client
/// reports' own Reply-To when one is set.
pub fn report_config(settings: &AppSettings) -> Result<SmtpConfig, SendError> {
    let mut config = SmtpConfig::from_settings(settings)?;
    if let Some(raw) = Some(&settings.client_reports.reply_to).filter(|r| !r.is_empty()) {
        config.reply_to = Some(parse_address(raw).ok_or_else(|| {
            SendError::Incomplete(format!(
                "The client reports' Reply-To address {raw} is not a valid email address."
            ))
        })?);
    }
    Ok(config)
}
