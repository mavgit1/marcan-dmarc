//! SQL for what happened to a record, over `records rec`: the one text behind
//! every count, filter, badge and export.
//!
//! The facts are decided in `dmarc_assess::RecordFacts` and computed at ingest
//! into plain columns (`dmarc_pass`, `spf_aligned_pass`, `dkim_aligned_pass`,
//! `forwarded`; `disposition` normalized; a `CHECK` holds `dmarc_pass` to the
//! two aligned passes, so the outcome and the verdict cannot disagree), so these expressions need no joins
//! and cannot fan out `records.count`. The four-way outcome and the DMARC
//! verdict over those columns are decided here, once, as SQL; the enums in
//! `dmarc_assess` only name the result.

use dmarc_assess::{Mail, Outcome};
use serde::Serialize;
use sqlx::FromRow;
use ts_rs::TS;

/// The receiver delivered the record (no disposition counts as delivered).
macro_rules! delivered_cond {
    () => {
        "(rec.disposition IS NULL OR rec.disposition = 'none')"
    };
}

/// Whether the receiver delivered `records rec` (no disposition counts as
/// delivered), as a boolean column.
pub(crate) const DELIVERED_EXPR: &str = delivered_cond!();

/// `dmarc_assess::Outcome` of `records rec`: the rule itself, in SQL.
pub(crate) const OUTCOME_EXPR: &str = concat!(
    "(CASE WHEN rec.dmarc_pass = 1 THEN 'passed' WHEN NOT ",
    delivered_cond!(),
    " THEN 'blocked' WHEN rec.forwarded = 1 THEN 'forwarded' ELSE 'delivered' END)"
);

/// `column` (an outcome's stored name) as its place worst first: a sort key
/// built from `dmarc_assess::Outcome::rank`, so the store sorts outcomes as
/// every other list does.
pub(crate) fn outcome_rank_expr(column: &str) -> String {
    let arms: String = Outcome::ALL
        .iter()
        .map(|o| format!(" WHEN '{}' THEN {}", o.as_str(), o.rank()))
        .collect();
    format!("(CASE {column}{arms} END)")
}

/// The DMARC verdict of `records rec`, as a literal for `concat!`.
macro_rules! verdict_expr {
    () => {
        "(CASE \
         WHEN rec.spf_aligned_pass = 1 AND rec.dkim_aligned_pass = 1 THEN 'pass_both' \
         WHEN rec.dkim_aligned_pass = 1 THEN 'pass_dkim' \
         WHEN rec.spf_aligned_pass = 1 THEN 'pass_spf' \
         ELSE 'fail' END)"
    };
}

/// `dmarc_assess::DmarcVerdict` of `records rec`: the rule itself, in SQL.
pub(crate) const VERDICT_EXPR: &str = verdict_expr!();

/// `SUM(rec.count)` over the records matching `$cond`; 0 (never NULL) over
/// an empty set, so it also works as a scalar subquery.
macro_rules! sum_where {
    ($cond:expr) => {
        concat!(
            "COALESCE(SUM(CASE WHEN ",
            $cond,
            " THEN rec.count ELSE 0 END), 0)"
        )
    };
}

// Message counts per outcome, shared by every screen. Each condition is
// spelled once; the standalone expressions and [`MAIL_COUNTS_SELECT`] are
// built from the same text.

macro_rules! compliant_cond {
    () => {
        "rec.dmarc_pass = 1"
    };
}
macro_rules! delivered_failure_cond {
    () => {
        concat!(
            "rec.dmarc_pass = 0 AND rec.forwarded = 0 AND ",
            delivered_cond!()
        )
    };
}
macro_rules! forwarded_failure_cond {
    () => {
        concat!(
            "rec.dmarc_pass = 0 AND rec.forwarded = 1 AND ",
            delivered_cond!()
        )
    };
}

/// Messages that passed DMARC.
pub(crate) const COMPLIANT_MESSAGES_EXPR: &str = sum_where!(compliant_cond!());
/// Messages that failed DMARC and were not a trusted forward: delivered
/// failures plus blocked by policy. The one meaning of a bare "failed" count
/// (the senders' `failed_messages` and their failing filter); forwarded mail
/// is legitimate mail broken in transit and counted apart
/// ([`FORWARDED_FAILURES_EXPR`]). A count or filter that takes forwarded mail
/// in says so in its name: the domain overview's `dmarc_failed_incl_forwarded`,
/// the Failures filter `all_failures`.
pub(crate) const FAILED_MESSAGES_EXPR: &str = sum_where!(concat!(
    "rec.dmarc_pass = 0 AND NOT (",
    forwarded_failure_cond!(),
    ")"
));
/// Delivered failures. Blocked by policy is `total - compliant - delivered - forwarded`.
pub(crate) const DELIVERED_FAILURES_EXPR: &str = sum_where!(delivered_failure_cond!());
/// Forwarded failures: legitimate mail broken in transit, not spoofing.
pub(crate) const FORWARDED_FAILURES_EXPR: &str = sum_where!(forwarded_failure_cond!());
/// Messages with an aligned SPF pass. The one SPF pass definition: raw
/// `records.spf_result` is alignment-blind.
pub(crate) const SPF_PASS_MESSAGES_EXPR: &str = sum_where!("rec.spf_aligned_pass = 1");
/// Messages with an aligned DKIM pass.
pub(crate) const DKIM_PASS_MESSAGES_EXPR: &str = sum_where!("rec.dkim_aligned_pass = 1");
/// Messages that passed DMARC without an aligned DKIM pass: SPF alone carried
/// them, so forwarding breaks them. Unsigned and signed-by-another-domain count alike.
pub(crate) const NO_ALIGNED_DKIM_MESSAGES_EXPR: &str =
    sum_where!("rec.dmarc_pass = 1 AND rec.dkim_aligned_pass = 0");
/// Messages of the verdict `pass_dkim` ([`VERDICT_EXPR`]): passed via DKIM
/// only, the Failures tab's "Passed via DKIM only".
pub(crate) const DKIM_ONLY_MESSAGES_EXPR: &str =
    sum_where!(concat!(verdict_expr!(), " = 'pass_dkim'"));
/// Messages of the verdict `pass_spf` ([`VERDICT_EXPR`]): passed via SPF
/// only, the Failures tab's "Passed via SPF only".
pub(crate) const SPF_ONLY_MESSAGES_EXPR: &str =
    sum_where!(concat!(verdict_expr!(), " = 'pass_spf'"));

/// Delivered compliance (`dmarc_assess::Mail::delivered_compliance`) as a
/// sort key over a row's `compliant` and `delivered_failures` columns (SQL
/// expressions): nothing delivered sorts below every real percentage. Only
/// for `ORDER BY`; the shown rate is the Rust one.
pub(crate) fn delivered_compliance_sort(compliant: &str, delivered_failures: &str) -> String {
    format!("COALESCE({compliant} * 100.0 / NULLIF({compliant} + {delivered_failures}, 0), -1)")
}

/// A sender's raw pass rate (`dmarc_assess::Mail::pass_rate`) as a sort key
/// over its `compliant` and `total` (SQL expressions); 0 without mail. Only
/// for `ORDER BY`; the shown rate is the Rust one.
pub(crate) fn pass_rate_sort(compliant: &str, total: &str) -> String {
    format!("COALESCE({compliant} * 100.0 / NULLIF({total}, 0), 0.0)")
}

/// The four [`MailCounts`] columns, for a `SELECT` over `records rec`.
pub(crate) const MAIL_COUNTS_SELECT: &str = concat!(
    "COALESCE(SUM(rec.count), 0) AS total_messages, ",
    sum_where!(compliant_cond!()),
    " AS compliant_messages, ",
    sum_where!(delivered_failure_cond!()),
    " AS delivered_failures, ",
    sum_where!(forwarded_failure_cond!()),
    " AS forwarded_failures"
);

/// Message totals of a scope by outcome, as every list row and total carries
/// them (flattened into the row). Blocked by policy is derived, not stored.
///
/// Carried by the API as-is: it is a fact, not a judgement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct MailCounts {
    /// `SUM(records.count)` in scope.
    pub total_messages: i64,
    /// Passed DMARC (`records.dmarc_pass`).
    pub compliant_messages: i64,
    /// Failed DMARC and delivered, not a trusted forward.
    pub delivered_failures: i64,
    /// Failed DMARC, delivered because the receiver trusted the forwarder.
    pub forwarded_failures: i64,
}

impl MailCounts {
    /// The same figures as the assessment input.
    pub fn mail(&self) -> Mail {
        Mail {
            total: self.total_messages,
            compliant: self.compliant_messages,
            delivered_failures: self.delivered_failures,
            forwarded_failures: self.forwarded_failures,
        }
    }

    /// Failures the receiver quarantined or rejected.
    pub fn blocked(&self) -> i64 {
        self.mail().blocked()
    }

    pub(crate) fn add(&mut self, other: &Self) {
        self.total_messages += other.total_messages;
        self.compliant_messages += other.compliant_messages;
        self.delivered_failures += other.delivered_failures;
        self.forwarded_failures += other.forwarded_failures;
    }
}

impl<'a> std::iter::Sum<&'a Self> for MailCounts {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        let mut total = Self::default();
        for counts in iter {
            total.add(counts);
        }
        total
    }
}

impl From<MailCounts> for Mail {
    fn from(counts: MailCounts) -> Self {
        counts.mail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sort key ranks every outcome as `Outcome::rank` does.
    #[tokio::test]
    async fn outcomes_sort_worst_first() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        for outcome in Outcome::ALL {
            let rank: i64 = sqlx::query_scalar(&format!(
                "SELECT {} FROM (SELECT ? AS kind)",
                outcome_rank_expr("kind")
            ))
            .bind(outcome.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(rank, i64::from(outcome.rank()), "{outcome:?}");
        }
    }

    /// The rule itself, through SQLite: every outcome and verdict the fact
    /// columns can produce, and the four counts over the same rows.
    #[tokio::test]
    async fn outcome_verdict_and_counts_truth_table() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        // (n, dmarc_pass, disposition, forwarded, spf_aligned_pass, dkim_aligned_pass, count)
        // Only rows the `records` CHECK admits: dmarc_pass is either aligned pass.
        let rec = "WITH rec(n, dmarc_pass, disposition, forwarded, spf_aligned_pass, \
                   dkim_aligned_pass, count) AS (VALUES \
                   (1, 1, NULL, 0, 1, 1, 10), \
                   (2, 1, 'reject', 1, 0, 1, 20), \
                   (3, 1, 'quarantine', 0, 1, 0, 1), \
                   (4, 0, NULL, 0, 0, 0, 3), \
                   (5, 0, 'none', 1, 0, 0, 2), \
                   (6, 0, 'quarantine', 1, 0, 0, 4), \
                   (7, 0, 'reject', 0, 0, 0, 8))";
        let impossible: i64 = sqlx::query_scalar(&format!(
            "{rec} SELECT COUNT(*) FROM rec \
             WHERE dmarc_pass != (spf_aligned_pass OR dkim_aligned_pass)"
        ))
        .fetch_one(&pool)
        .await
        .expect("check");
        assert_eq!(impossible, 0);
        let rows: Vec<(String, String)> = sqlx::query_as(&format!(
            "{rec} SELECT {OUTCOME_EXPR}, {VERDICT_EXPR} FROM rec ORDER BY n"
        ))
        .fetch_all(&pool)
        .await
        .expect("rows");
        let expect = |o: &str, v: &str| (o.to_owned(), v.to_owned());
        assert_eq!(
            rows,
            vec![
                expect("passed", "pass_both"),
                // Passed: the disposition and the forward do not matter.
                expect("passed", "pass_dkim"),
                expect("passed", "pass_spf"),
                // Failed and delivered: no disposition counts as delivered.
                expect("delivered", "fail"),
                expect("forwarded", "fail"),
                // Not delivered: blocked, forwarded or not.
                expect("blocked", "fail"),
                expect("blocked", "fail"),
            ]
        );
        let counts: MailCounts =
            sqlx::query_as(&format!("{rec} SELECT {MAIL_COUNTS_SELECT} FROM rec"))
                .fetch_one(&pool)
                .await
                .expect("counts");
        assert_eq!(
            counts,
            MailCounts {
                total_messages: 48,
                compliant_messages: 31,
                delivered_failures: 3,
                forwarded_failures: 2,
            }
        );
        assert_eq!(counts.blocked(), 12);
        // "Failed" leaves out the trusted forward: delivered failures plus blocked.
        let failed: i64 =
            sqlx::query_scalar(&format!("{rec} SELECT {FAILED_MESSAGES_EXPR} FROM rec"))
                .fetch_one(&pool)
                .await
                .expect("failed");
        assert_eq!(failed, counts.delivered_failures + counts.blocked());
        assert_eq!(failed, 15);
        // The two partial passes count the rows of their verdict: row 2
        // (`pass_dkim`, 20) and row 3 (`pass_spf`, 1).
        let (dkim_only, spf_only): (i64, i64) = sqlx::query_as(&format!(
            "{rec} SELECT {DKIM_ONLY_MESSAGES_EXPR}, {SPF_ONLY_MESSAGES_EXPR} FROM rec"
        ))
        .fetch_one(&pool)
        .await
        .expect("partial passes");
        assert_eq!((dkim_only, spf_only), (20, 1));
    }

    #[test]
    fn counts_sum_and_derive_blocked() {
        let a = MailCounts {
            total_messages: 10,
            compliant_messages: 6,
            delivered_failures: 1,
            forwarded_failures: 1,
        };
        let total: MailCounts = [a, a].iter().sum();
        assert_eq!(total.total_messages, 20);
        assert_eq!(total.blocked(), 4);
    }
}
