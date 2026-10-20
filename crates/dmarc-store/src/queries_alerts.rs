//! Alerts: the judged state each domain was last evaluated in, and the events
//! (what changed) every evaluation found. The comparison is
//! `dmarc_assess::alert_events`; this module only keeps its input and output.
//!
//! Events are what the UI's alert list shows and what the alert email sends;
//! `notified_at` says which were mailed. Both tables follow their domain:
//! deleted with it, and cleared when it is ignored.

use crate::queries::{begin_write, Scope};
use crate::StoreError;
use dmarc_assess::{AlertEvent, AlertKind, AlertState, Status, ALERT_FORMAT};
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder, Sqlite};
use std::collections::{HashMap, HashSet};
use ts_rs::TS;

/// One domain's evaluation: its new state and what changed since the last one.
#[derive(Debug, Clone)]
pub struct AlertEvaluation {
    pub domain_id: i64,
    pub state: AlertState,
    pub events: Vec<AlertEvent>,
}

/// One stored event with its domain.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct AlertEventRow {
    pub id: i64,
    pub domain_id: i64,
    pub domain: String,
    pub organization_id: i64,
    pub organization: String,
    /// When the evaluation that found it ran (unix seconds).
    pub created_at: i64,
    /// When an alert email carried it; `null` until then (or with alert
    /// emails off).
    pub notified_at: Option<i64>,
    /// What changed and its colour.
    pub event: AlertEvent,
}

#[derive(FromRow)]
struct EventRow {
    id: i64,
    domain_id: i64,
    domain: String,
    organization_id: i64,
    organization: String,
    created_at: i64,
    notified_at: Option<i64>,
    severity: String,
    params: String,
}

impl EventRow {
    /// The stored event; `None` (logged) for a row this version cannot read.
    fn parse(self) -> Option<AlertEventRow> {
        let kind = serde_json::from_str::<AlertKind>(&self.params);
        let severity =
            serde_json::from_value::<Status>(serde_json::Value::String(self.severity.clone()));
        match (kind, severity) {
            (Ok(kind), Ok(severity)) => Some(AlertEventRow {
                id: self.id,
                domain_id: self.domain_id,
                domain: self.domain,
                organization_id: self.organization_id,
                organization: self.organization,
                created_at: self.created_at,
                notified_at: self.notified_at,
                event: AlertEvent { kind, severity },
            }),
            _ => {
                tracing::warn!(
                    id = self.id,
                    "stored alert event is not readable; skipping it"
                );
                None
            }
        }
    }
}

/// The stored state of every visible domain that has one, by domain id. A
/// state this version cannot read (or of another [`ALERT_FORMAT`]) is left
/// out: the next evaluation is then a new baseline for that domain.
pub async fn get_alert_states(pool: &SqlitePool) -> Result<HashMap<i64, AlertState>, StoreError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT s.domain_id, s.state FROM alert_state s \
         INNER JOIN domains d ON d.id = s.domain_id WHERE d.ignored = 0 AND s.format = ?",
    )
    .bind(ALERT_FORMAT)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(id, json)| match serde_json::from_str::<AlertState>(&json) {
                Ok(state) => Some((id, state)),
                Err(e) => {
                    tracing::warn!(domain_id = id, error = %e, "stored alert state is not readable; starting a new baseline");
                    None
                }
            },
        )
        .collect())
}

/// Ids of the evaluated domains last evaluated at or before `held_at` that
/// got a report stored (ingested, not dated) since that evaluation: the ones
/// a mailbox import that held the evaluation back at `held_at` may have
/// touched. A domain evaluated after `held_at` is not among them.
pub async fn domains_with_reports_stored_since_evaluated(
    pool: &SqlitePool,
    held_at: i64,
) -> Result<HashSet<i64>, StoreError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT s.domain_id FROM alert_state s \
         WHERE s.evaluated_at <= ? \
         AND EXISTS (SELECT 1 FROM reports r \
             WHERE r.domain_id = s.domain_id AND r.created_at >= s.evaluated_at)",
    )
    .bind(held_at)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().collect())
}

/// Store one evaluation run in one transaction: every domain's new state and
/// its events, all stamped `evaluated_at`. A domain ignored meanwhile keeps
/// nothing. Returns the number of events stored.
pub async fn save_alert_evaluations(
    pool: &SqlitePool,
    evaluated_at: i64,
    evaluations: &[AlertEvaluation],
) -> Result<usize, StoreError> {
    let mut tx = begin_write(pool).await?;
    let mut stored = 0;
    for e in evaluations {
        let state = serde_json::to_string(&e.state)?;
        let written = sqlx::query(
            r#"
            INSERT INTO alert_state (domain_id, format, state, evaluated_at)
            SELECT ?1, ?2, ?3, ?4
            WHERE EXISTS (SELECT 1 FROM domains WHERE id = ?1 AND ignored = 0)
            ON CONFLICT(domain_id) DO UPDATE SET
                format = excluded.format,
                state = excluded.state,
                evaluated_at = excluded.evaluated_at
            "#,
        )
        .bind(e.domain_id)
        .bind(ALERT_FORMAT)
        .bind(&state)
        .bind(evaluated_at)
        .execute(&mut *tx)
        .await?;
        if written.rows_affected() == 0 {
            continue;
        }
        for event in &e.events {
            let severity = serde_json::to_value(event.severity)?;
            sqlx::query(
                "INSERT INTO alert_events (domain_id, created_at, kind, severity, format, params) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(e.domain_id)
            .bind(evaluated_at)
            .bind(event.kind.name())
            .bind(severity.as_str().unwrap_or_default())
            .bind(ALERT_FORMAT)
            .bind(serde_json::to_string(&event.kind)?)
            .execute(&mut *tx)
            .await?;
            stored += 1;
        }
    }
    tx.commit().await?;
    Ok(stored)
}

const EVENT_SELECT: &str = "SELECT e.id, e.domain_id, d.domain, d.organization_id, \
     org.name AS organization, e.created_at, e.notified_at, e.severity, e.params \
     FROM alert_events e \
     INNER JOIN domains d ON d.id = e.domain_id \
     INNER JOIN organizations org ON org.id = d.organization_id \
     WHERE d.ignored = 0";

/// [`EVENT_SELECT`] over the events of this [`ALERT_FORMAT`].
fn event_query() -> QueryBuilder<'static, Sqlite> {
    let mut qb = QueryBuilder::new(EVENT_SELECT);
    qb.push(" AND e.format = ").push_bind(ALERT_FORMAT);
    qb
}

/// Narrow events to `scope`: its organization and domain, and its time
/// window over `created_at`.
fn push_scope(qb: &mut QueryBuilder<'_, Sqlite>, scope: &Scope) {
    if let Some(id) = scope.organization_id {
        qb.push(" AND d.organization_id = ").push_bind(id);
    }
    if let Some(id) = scope.domain_id {
        qb.push(" AND e.domain_id = ").push_bind(id);
    }
    if let Some(ts) = scope.since {
        qb.push(" AND e.created_at >= ").push_bind(ts);
    }
    if let Some(ts) = scope.until {
        qb.push(" AND e.created_at < ").push_bind(ts);
    }
}

async fn fetch_events(
    pool: &SqlitePool,
    mut qb: QueryBuilder<'_, Sqlite>,
) -> Result<Vec<AlertEventRow>, StoreError> {
    Ok(qb
        .build_query_as::<EventRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(EventRow::parse)
        .collect())
}

/// One page of the events in `scope`, newest first, and the total.
pub async fn list_alert_events(
    pool: &SqlitePool,
    scope: Scope,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AlertEventRow>, i64), StoreError> {
    let mut qb = event_query();
    push_scope(&mut qb, &scope);
    qb.push(" ORDER BY e.created_at DESC, e.id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let items = fetch_events(pool, qb).await?;
    Ok((items, count_alert_events(pool, scope).await?))
}

/// Every event in `scope`, oldest first (a digest's list).
pub async fn alert_events_in(
    pool: &SqlitePool,
    scope: Scope,
) -> Result<Vec<AlertEventRow>, StoreError> {
    let mut qb = event_query();
    push_scope(&mut qb, &scope);
    qb.push(" ORDER BY e.created_at, e.id");
    fetch_events(pool, qb).await
}

/// The events no alert email has carried yet, created at or after `since`,
/// oldest first, at most `limit` (read through the partial index
/// `idx_alert_events_unnotified`).
pub async fn unnotified_alert_events(
    pool: &SqlitePool,
    since: i64,
    limit: i64,
) -> Result<Vec<AlertEventRow>, StoreError> {
    let mut qb = event_query();
    qb.push(" AND e.notified_at IS NULL AND e.created_at >= ")
        .push_bind(since)
        .push(" ORDER BY e.created_at, e.id LIMIT ")
        .push_bind(limit);
    fetch_events(pool, qb).await
}

/// Delete up to `limit` of the alert events created before `cutoff` (oldest
/// first), in one write transaction; returns how many.
pub async fn delete_alert_events_older_than(
    pool: &SqlitePool,
    cutoff: i64,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut tx = begin_write(pool).await?;
    let done = sqlx::query(
        "DELETE FROM alert_events WHERE id IN \
         (SELECT id FROM alert_events WHERE created_at < ? ORDER BY created_at, id LIMIT ?)",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(done.rows_affected())
}

/// Record that the events `ids` were sent at `at`.
pub async fn mark_alert_events_notified(
    pool: &SqlitePool,
    ids: &[i64],
    at: i64,
) -> Result<(), StoreError> {
    let mut tx = begin_write(pool).await?;
    for chunk in ids.chunks(500) {
        let mut qb = QueryBuilder::<Sqlite>::new("UPDATE alert_events SET notified_at = ");
        qb.push_bind(at).push(" WHERE id IN (");
        let mut sep = qb.separated(", ");
        for id in chunk {
            sep.push_bind(id);
        }
        qb.push(")");
        qb.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// How many events `scope` holds (its window over `created_at`).
pub async fn count_alert_events(pool: &SqlitePool, scope: Scope) -> Result<i64, StoreError> {
    let mut qb = QueryBuilder::new(
        "SELECT COUNT(*) FROM alert_events e \
         INNER JOIN domains d ON d.id = e.domain_id WHERE d.ignored = 0 AND e.format = ",
    );
    qb.push_bind(ALERT_FORMAT);
    push_scope(&mut qb, &scope);
    Ok(qb.build_query_scalar::<i64>().fetch_one(pool).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_domain, ignore_domain, init, CreateDomainResult};
    use dmarc_assess::{AlertedIssue, HealthState};

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&dir.path().join("test.db")).await.unwrap();
        (dir, pool)
    }

    async fn domain(pool: &SqlitePool, organization_id: i64, name: &str) -> i64 {
        match create_domain(pool, organization_id, name).await.unwrap() {
            CreateDomainResult::Created(d) => d.id,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn opened(issue: AlertedIssue) -> AlertEvent {
        AlertEvent {
            kind: AlertKind::IssueOpened {
                issue,
                mail: None,
                stopped: None,
            },
            severity: Status::Fail,
        }
    }

    fn resolved(issue: AlertedIssue) -> AlertEvent {
        AlertEvent {
            kind: AlertKind::IssueResolved {
                issue,
                mail: None,
                stopped: None,
            },
            severity: Status::Pass,
        }
    }

    fn state(health: HealthState) -> AlertState {
        AlertState {
            has_reports: true,
            health,
            ..AlertState::default()
        }
    }

    #[tokio::test]
    async fn states_and_events_round_trip_in_one_run() {
        let (_dir, pool) = pool().await;
        let a = domain(&pool, 1, "a.example").await;
        let b = domain(&pool, 1, "b.example").await;
        assert!(get_alert_states(&pool).await.unwrap().is_empty());

        let run = [
            AlertEvaluation {
                domain_id: a,
                state: state(HealthState::Protected),
                events: vec![],
            },
            AlertEvaluation {
                domain_id: b,
                state: state(HealthState::Critical),
                events: vec![
                    opened(AlertedIssue::DeliveredFailures),
                    opened(AlertedIssue::ReportsStopped),
                ],
            },
        ];
        assert_eq!(save_alert_evaluations(&pool, 1_000, &run).await.unwrap(), 2);
        let states = get_alert_states(&pool).await.unwrap();
        assert_eq!(states[&a].health, HealthState::Protected);
        assert_eq!(states[&b].health, HealthState::Critical);

        // A later run replaces the state and adds events.
        let next = [AlertEvaluation {
            domain_id: a,
            state: state(HealthState::Warning),
            events: vec![resolved(AlertedIssue::DeliveredFailures)],
        }];
        save_alert_evaluations(&pool, 2_000, &next).await.unwrap();
        assert_eq!(
            get_alert_states(&pool).await.unwrap()[&a].health,
            HealthState::Warning
        );

        let (page, total) = list_alert_events(&pool, Scope::ALL, 2, 0).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(page.len(), 2);
        // Newest first; ties by id, newest first.
        assert_eq!(page[0].domain, "a.example");
        assert_eq!(page[0].created_at, 2_000);
        assert_eq!(page[0].event, resolved(AlertedIssue::DeliveredFailures));
        assert_eq!(page[1].event, opened(AlertedIssue::ReportsStopped));
        let (rest, _) = list_alert_events(&pool, Scope::ALL, 2, 2).await.unwrap();
        assert_eq!(rest[0].event, opened(AlertedIssue::DeliveredFailures));
    }

    #[tokio::test]
    async fn lists_follow_the_scope() {
        let (_dir, pool) = pool().await;
        let other_id = crate::get_or_create_organization(&pool, "Other")
            .await
            .unwrap()
            .id;
        let a = domain(&pool, 1, "a.example").await;
        let b = domain(&pool, other_id, "b.example").await;
        for (at, id) in [(100, a), (200, b), (300, a)] {
            let run = [AlertEvaluation {
                domain_id: id,
                state: AlertState::default(),
                events: vec![opened(AlertedIssue::ReportsStopped)],
            }];
            save_alert_evaluations(&pool, at, &run).await.unwrap();
        }
        let count = |scope| {
            let pool = pool.clone();
            async move { count_alert_events(&pool, scope).await.unwrap() }
        };
        assert_eq!(count(Scope::ALL).await, 3);
        assert_eq!(count(Scope::new(Some(other_id), None, None)).await, 1);
        assert_eq!(count(Scope::domain(a, None)).await, 2);
        assert_eq!(count(Scope::domain(a, Some(200))).await, 1);
        assert_eq!(count(Scope::ALL.window(Some(100), Some(300))).await, 2);
        let (items, _) = list_alert_events(&pool, Scope::new(Some(other_id), None, None), 50, 0)
            .await
            .unwrap();
        assert_eq!(items[0].organization, "Other");
        let window = alert_events_in(&pool, Scope::ALL.window(Some(150), None))
            .await
            .unwrap();
        let times: Vec<i64> = window.iter().map(|e| e.created_at).collect();
        assert_eq!(times, [200, 300]);
    }

    #[tokio::test]
    async fn notified_events_are_marked() {
        let (_dir, pool) = pool().await;
        let a = domain(&pool, 1, "a.example").await;
        let run = [AlertEvaluation {
            domain_id: a,
            state: AlertState::default(),
            events: vec![
                opened(AlertedIssue::ReportsStopped),
                resolved(AlertedIssue::DeliveredFailures),
            ],
        }];
        save_alert_evaluations(&pool, 100, &run).await.unwrap();
        let pending = unnotified_alert_events(&pool, 0, 50).await.unwrap();
        assert_eq!(pending.len(), 2);
        mark_alert_events_notified(&pool, &[pending[0].id], 150)
            .await
            .unwrap();
        let pending = unnotified_alert_events(&pool, 0, 50).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event, resolved(AlertedIssue::DeliveredFailures));
        let (all, _) = list_alert_events(&pool, Scope::ALL, 50, 0).await.unwrap();
        assert_eq!(all[1].notified_at, Some(150));
    }

    /// The unsent events are read from `since` on, oldest first, at most
    /// `limit`, through the partial index; old events are deleted.
    #[tokio::test]
    async fn unsent_events_are_bounded_and_old_ones_pruned() {
        let (_dir, pool) = pool().await;
        let a = domain(&pool, 1, "a.example").await;
        for at in [100, 200, 300, 400] {
            let run = [AlertEvaluation {
                domain_id: a,
                state: AlertState::default(),
                events: vec![opened(AlertedIssue::ReportsStopped)],
            }];
            save_alert_evaluations(&pool, at, &run).await.unwrap();
        }
        let times =
            |rows: Vec<AlertEventRow>| -> Vec<i64> { rows.iter().map(|e| e.created_at).collect() };
        let pending = unnotified_alert_events(&pool, 200, 2).await.unwrap();
        assert_eq!(times(pending), [200, 300]);
        let plan: Vec<String> = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT id FROM alert_events \
             WHERE notified_at IS NULL AND created_at >= 200 ORDER BY created_at, id",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.3)
        .collect();
        assert!(
            plan.iter()
                .any(|p| p.contains("idx_alert_events_unnotified")),
            "{plan:?}"
        );

        // Two events before the cutoff, one per call with a limit of one.
        assert_eq!(
            delete_alert_events_older_than(&pool, 300, 1).await.unwrap(),
            1
        );
        assert_eq!(
            delete_alert_events_older_than(&pool, 300, 1).await.unwrap(),
            1
        );
        assert_eq!(
            delete_alert_events_older_than(&pool, 300, 1).await.unwrap(),
            0
        );
        let pending = unnotified_alert_events(&pool, 0, 50).await.unwrap();
        assert_eq!(times(pending), [300, 400]);
    }

    #[tokio::test]
    async fn an_ignored_domain_keeps_nothing() {
        let (_dir, pool) = pool().await;
        let a = domain(&pool, 1, "a.example").await;
        let run = [AlertEvaluation {
            domain_id: a,
            state: AlertState::default(),
            events: vec![opened(AlertedIssue::ReportsStopped)],
        }];
        save_alert_evaluations(&pool, 100, &run).await.unwrap();
        assert!(ignore_domain(&pool, a).await.unwrap());
        assert!(get_alert_states(&pool).await.unwrap().is_empty());
        assert_eq!(count_alert_events(&pool, Scope::ALL).await.unwrap(), 0);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
        // Evaluating it after it was ignored stores nothing.
        assert_eq!(save_alert_evaluations(&pool, 200, &run).await.unwrap(), 0);
        let states: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_state")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(states, 0);
    }

    #[tokio::test]
    async fn rows_go_with_a_deleted_domain() {
        let (_dir, pool) = pool().await;
        let a = domain(&pool, 1, "a.example").await;
        let run = [AlertEvaluation {
            domain_id: a,
            state: AlertState::default(),
            events: vec![opened(AlertedIssue::ReportsStopped)],
        }];
        save_alert_evaluations(&pool, 100, &run).await.unwrap();
        sqlx::query("DELETE FROM domains WHERE id = ?")
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        let rows: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM alert_events) + (SELECT COUNT(*) FROM alert_state)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 0);
    }
}
