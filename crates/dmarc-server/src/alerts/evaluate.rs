//! One evaluation run: every monitored domain's current state against the
//! stored one.

use crate::api::{identities_named, load_sender_names};
use crate::error::AppResult;
use crate::snapshots::dns::complete_report;
use crate::view::domain::{domain_overview, Judging};
use dmarc_assess::{
    alert_events, alert_state, readiness_window_start, AlertState, NewSender,
    NEW_SENDER_MIN_FAILURES,
};
use dmarc_identity::SenderNames;
use dmarc_store::{
    domains_with_reports_stored_since_evaluated, get_alert_states, get_alerts_held_at,
    get_dns_snapshots, get_domain_overview, list_monitored_domain_ids, new_failing_senders,
    readiness_mail, save_alert_evaluations, AlertEvaluation, DnsSnapshot,
};
use sqlx::SqlitePool;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

/// What one run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvaluationSummary {
    /// Domains evaluated (a domain's first evaluation counts, as a baseline).
    pub domains: usize,
    /// Events stored.
    pub events: usize,
    /// Domains whose reports were taken as a new baseline after an import.
    pub rebaselined: usize,
    /// Domains whose state could not be read this run, by name: they keep
    /// their stored state and are tried again next run, and the others are
    /// evaluated and stored as usual.
    pub skipped: Vec<String>,
}

/// The sender names of one run, loaded when the first domain needs them:
/// most runs find no new failing sender.
#[derive(Default)]
pub struct RunNames(Option<SenderNames>);

impl RunNames {
    async fn get(&mut self, pool: &SqlitePool) -> AppResult<&SenderNames> {
        if self.0.is_none() {
            self.0 = Some(load_sender_names(pool).await?);
        }
        Ok(self.0.as_ref().expect("loaded"))
    }
}

/// The senders new to domain `id` in the window from `since` with at least
/// [`NEW_SENDER_MIN_FAILURES`] failing messages, named as the sender pages
/// name them.
async fn new_senders(
    pool: &SqlitePool,
    id: i64,
    since: i64,
    names: &mut RunNames,
) -> AppResult<Vec<NewSender>> {
    let rows = new_failing_senders(pool, id, since, NEW_SENDER_MIN_FAILURES).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let names = names.get(pool).await?;
    let mut identities =
        identities_named(pool, names, rows.iter().map(|r| r.source_ip.as_str())).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let identity = identities.remove(&row.source_ip).unwrap_or_default();
            NewSender {
                name: identity.name.or(identity.operator_domain),
                source_ip: row.source_ip,
                failed: row.failed_messages,
                delivered_failures: row.delivered_failures,
            }
        })
        .collect())
}

/// The current [`AlertState`] of domain `id`, judged by `judging` (at its
/// `now`), or `None` when it is no longer monitored. `snapshot` is its
/// stored DNS result; without a complete one the state has no DNS part.
/// `names` names the new failing senders.
pub async fn current_state(
    pool: &SqlitePool,
    id: i64,
    snapshot: Option<&DnsSnapshot>,
    judging: Judging,
    names: &mut RunNames,
) -> AppResult<Option<AlertState>> {
    let since = readiness_window_start(judging.now);
    let Some(row) = get_domain_overview(pool, id, Some(since)).await? else {
        return Ok(None);
    };
    let has_reports = row.last_report_at.is_some();
    let policy = row.policy.as_ref().and_then(|p| p.p.as_deref());
    let readiness = readiness_mail(pool, id, policy, since).await?;
    let overview = domain_overview(row, &readiness, judging);
    let senders = new_senders(pool, id, since, names).await?;
    let dns = snapshot.and_then(|snap| {
        let report = complete_report(snap)?;
        let version = u32::try_from(snap.checker_version).unwrap_or_default();
        Some(dmarc_dns::dns_state(&report, version))
    });
    Ok(Some(alert_state(
        overview.verdicts.health.state,
        &overview.assessment,
        &overview.policy_readiness.judged,
        has_reports,
        overview.row.parked,
        dns,
        senders,
    )))
}

/// Evaluate every monitored domain at `now` and store the states and events
/// in one transaction. A domain evaluated for the first time is stored as
/// its baseline, with no events. Stops between domains once `token` is
/// cancelled, storing nothing of an unfinished run. A domain whose state
/// cannot be read is logged with its name and skipped
/// ([`EvaluationSummary::skipped`]), so one domain never stops the alerts of
/// the others.
///
/// After the evaluation was held back for a mailbox import (the stored
/// `alerts_held_at`), a domain last evaluated before that hold that got
/// reports stored since its evaluation compares only its DNS, and its
/// report-derived parts become a new baseline
/// (`AlertState::reports_rebaselined`). Both times are stored, so the rule
/// holds across a restart and until an evaluation succeeds.
pub async fn evaluate_all(
    pool: &SqlitePool,
    now: i64,
    token: &CancellationToken,
) -> AppResult<EvaluationSummary> {
    let domains = list_monitored_domain_ids(pool).await?;
    if domains.is_empty() {
        return Ok(EvaluationSummary::default());
    }
    let imported = match get_alerts_held_at(pool).await? {
        Some(held_at) => domains_with_reports_stored_since_evaluated(pool, held_at).await?,
        None => HashSet::new(),
    };
    let mut previous = get_alert_states(pool).await?;
    for (id, state) in &mut previous {
        if imported.contains(id) {
            *state = state.reports_rebaselined();
        }
    }
    let names: Vec<String> = domains.iter().map(|(_, name)| name.clone()).collect();
    let snapshots = get_dns_snapshots(pool, &names).await?;
    let judging = Judging::load(pool, now).await?;

    let mut names = RunNames::default();
    let mut evaluations = Vec::with_capacity(domains.len());
    let mut skipped = Vec::new();
    for (id, name) in &domains {
        if token.is_cancelled() {
            return Ok(EvaluationSummary::default());
        }
        let state = match current_state(pool, *id, snapshots.get(name), judging, &mut names).await {
            Ok(Some(state)) => state,
            Ok(None) => continue,
            Err(e) => {
                tracing::info!(domain = %name, error = %e, "alerts: could not evaluate this domain; skipped this run");
                skipped.push(name.clone());
                continue;
            }
        };
        let before = previous.get(id);
        let changes = match (before.and_then(|s| s.dns.as_ref()), state.dns.as_ref()) {
            (Some(a), Some(b)) => dmarc_dns::diff_records(&a.records, &b.records),
            _ => Vec::new(),
        };
        let events = alert_events(before, &state, &changes);
        evaluations.push(AlertEvaluation {
            domain_id: *id,
            state,
            events,
        });
    }
    let events = save_alert_evaluations(pool, now, &evaluations).await?;
    Ok(EvaluationSummary {
        domains: evaluations.len(),
        events,
        rebaselined: evaluations
            .iter()
            .filter(|e| imported.contains(&e.domain_id))
            .count(),
        skipped,
    })
}
