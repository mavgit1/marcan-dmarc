//! The mail a domain's policy readiness is judged from: plain sums over a
//! window, split by the policy each report's receiver applied.

use crate::outcome::{MailCounts, MAIL_COUNTS_SELECT};
use crate::queries::Scope;
use crate::StoreError;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder};

/// One domain's mail over a readiness window, split by the policy each
/// report's receiver applied (`reports.policy_p`, its `policy_published`):
/// what policy readiness is judged from. Plain sums and stored values;
/// whether they are enough is `dmarc_assess::policy_readiness`'s call.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow)]
pub struct ReadinessMailRow {
    /// Every message of the window, whatever policy was applied.
    #[sqlx(flatten)]
    pub mail: MailCounts,
    /// Messages of the window from reports that applied the given policy.
    pub policy_messages: i64,
    /// `date_begin` of the oldest report that applied the given policy since
    /// the newest report under another one (or none) that began before the
    /// window; any time, so it can lie before the window. Reports under
    /// another policy inside the window do not move it. `None` without one.
    pub policy_first_report_at: Option<i64>,
    /// The other applied policy (lowercase) with the most messages in the
    /// window; `None` when every message was under the given one, or the
    /// rest came from reports that name none.
    pub other_policy: Option<String>,
}

/// `LOWER(TRIM(<r>.policy_p))`: a report's applied `p=`, compared like the
/// DNS record's (which the DNS parser stores lowercase).
fn applied_p(r: &str) -> String {
    format!("LOWER(TRIM({r}.policy_p))")
}

/// [`ReadinessMailRow`] of domain `domain_id` from `since` on, split by
/// `policy` (the DNS record's `p=`; `None` matches no report).
pub async fn readiness_mail(
    pool: &SqlitePool,
    domain_id: i64,
    policy: Option<&str>,
    since: i64,
) -> Result<ReadinessMailRow, StoreError> {
    let policy = policy.map(|p| p.trim().to_ascii_lowercase());
    let (r_p, f_p, o_p, x_p) = (
        applied_p("r"),
        applied_p("f"),
        applied_p("o"),
        applied_p("x"),
    );
    let mut qb = QueryBuilder::new(format!(
        "SELECT {MAIL_COUNTS_SELECT}, \
         COALESCE(SUM(CASE WHEN {r_p} = "
    ));
    qb.push_bind(policy.clone());
    qb.push(
        " THEN rec.count END), 0) AS policy_messages, \
         (SELECT MIN(f.date_begin) FROM reports f WHERE f.domain_id = ",
    );
    qb.push_bind(domain_id);
    qb.push(format!(" AND {f_p} = "));
    qb.push_bind(policy.clone());
    qb.push(
        " AND f.date_begin > IFNULL((SELECT MAX(o.date_begin) FROM reports o \
         WHERE o.domain_id = ",
    );
    qb.push_bind(domain_id);
    qb.push(" AND o.date_begin < ");
    qb.push_bind(since);
    qb.push(format!(" AND {o_p} IS NOT "));
    qb.push_bind(policy.clone());
    qb.push(
        "), f.date_begin - 1)) AS policy_first_report_at, \
         (SELECT ",
    );
    qb.push(format!(
        "{x_p} FROM reports x INNER JOIN records xr ON xr.report_id = x.id \
         WHERE x.domain_id = "
    ));
    qb.push_bind(domain_id);
    qb.push(" AND x.date_begin >= ");
    qb.push_bind(since);
    qb.push(format!(" AND {x_p} IS NOT NULL AND {x_p} IS NOT "));
    qb.push_bind(policy);
    qb.push(format!(
        " GROUP BY {x_p} ORDER BY SUM(xr.count) DESC, {x_p} LIMIT 1) AS other_policy \
         FROM reports r LEFT JOIN records rec ON rec.report_id = r.id WHERE 1=1"
    ));
    Scope::domain(domain_id, Some(since)).push_reports(&mut qb, "r");
    Ok(qb
        .build_query_as::<ReadinessMailRow>()
        .fetch_one(pool)
        .await?)
}
