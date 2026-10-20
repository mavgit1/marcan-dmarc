//! DKIM selectors observed in aggregate reports.
//!
//! Selectors cannot be guessed reliably (ESPs and tenants pick their own names),
//! but every DKIM auth result in an aggregate report carries the selector that
//! signed the message. This turns those into the DNS checker's probe list.

use crate::StoreError;
use sqlx::sqlite::SqlitePool;

/// Most selectors any real domain uses at once is a handful; this only exists so
/// a pathological domain cannot turn one grade into hundreds of DNS lookups.
pub const OBSERVED_SELECTOR_LIMIT: i64 = 25;

/// Only selectors seen within this many seconds of the domain's newest report
/// count as in use. Anchored to the newest report rather than the clock so a
/// domain whose reports paused is still assessed, while a selector retired
/// after a key rotation stops being probed (and stops warning) a month later.
pub const OBSERVED_SELECTOR_WINDOW_SECS: i64 = 30 * 86_400;

const OBSERVED_SELECTORS_SQL: &str = r#"
        SELECT LOWER(TRIM(auth.selector)) AS selector,
               COALESCE(SUM(CASE WHEN auth.result = 'pass' THEN rec.count ELSE 0 END), 0) AS pass,
               COALESCE(SUM(CASE WHEN auth.result = 'pass' THEN 0 ELSE rec.count END), 0) AS fail
        FROM domains
        INNER JOIN reports ON reports.domain_id = domains.id
        INNER JOIN records rec ON rec.report_id = reports.id
        INNER JOIN record_dkim_auth auth ON auth.record_id = rec.id
        WHERE domains.domain = ?
          AND reports.date_begin >= (
                SELECT MAX(latest.date_begin) FROM reports latest
                WHERE latest.domain_id = domains.id
              ) - ?
          AND auth.selector IS NOT NULL
          AND TRIM(auth.selector) <> ''
          AND (
                auth.domain = ? COLLATE NOCASE
             OR auth.domain LIKE ? COLLATE NOCASE
          )
        GROUP BY 1
        ORDER BY SUM(rec.count) DESC, 1
        LIMIT ?
        "#;

/// One DKIM selector seen signing mail for a domain, with report volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedDkimSelector {
    /// Lowercased, trimmed selector name.
    pub selector: String,
    /// Messages whose signature with this selector verified.
    pub pass_messages: i64,
    /// Messages whose signature with this selector did not verify (any
    /// non-`pass` result).
    pub fail_messages: i64,
}

/// Distinct DKIM selectors seen signing mail for `domain`, most used first.
///
/// Only signatures whose `d=` is the domain itself or a subdomain count; a third
/// party signing under its own zone publishes that key there, not in ours.
///
/// Limited to [`OBSERVED_SELECTOR_WINDOW_SECS`] before the domain's newest
/// report, then ranked by volume and capped.
///
/// Domains are stored normalized (lowercase, no trailing dot), so the lookup
/// is an exact match that can use the `domains.domain` unique index.
pub async fn list_observed_dkim_selectors(
    pool: &SqlitePool,
    domain: &str,
) -> Result<Vec<ObservedDkimSelector>, StoreError> {
    let domain = dmarc_domain::normalize_name(domain);
    let rows = sqlx::query_as::<_, (String, i64, i64)>(OBSERVED_SELECTORS_SQL)
        .bind(&domain)
        .bind(OBSERVED_SELECTOR_WINDOW_SECS)
        .bind(&domain)
        .bind(format!("%.{domain}"))
        .bind(OBSERVED_SELECTOR_LIMIT)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(selector, pass_messages, fail_messages)| ObservedDkimSelector {
                selector,
                pass_messages,
                fail_messages,
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    const NEWEST: i64 = 100 * 86_400;

    async fn seeded() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrate");

        // Organization 1 (the default) is seeded by the migration.
        sqlx::query(
            "INSERT INTO domains (id, organization_id, domain) VALUES (1, 1, 'example.com'), (2, 1, 'example.org')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Report 3 is example.com's, but far older than its newest report.
        for (id, domain_id, begin) in [
            (1i64, 1i64, NEWEST),
            (2, 2, NEWEST),
            (3, 1, NEWEST - OBSERVED_SELECTOR_WINDOW_SECS - 86_400),
        ] {
            sqlx::query(
                r#"INSERT INTO reports (id, report_id, org_name, email, domain_id, date_begin,
                   date_end, total_messages, compliant_messages)
                   VALUES (?, ?, 'google.com', 'x@y', ?, ?, ?, 1, 1)"#,
            )
            .bind(id)
            .bind(format!("r{id}"))
            .bind(domain_id)
            .bind(begin)
            .bind(begin + 86_400)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    #[allow(clippy::too_many_arguments)]
    async fn add_result(
        pool: &SqlitePool,
        id: i64,
        report: i64,
        count: i64,
        d: &str,
        selector: &str,
        result: &str,
    ) {
        sqlx::query(
            "INSERT INTO records (id, report_id, source_ip, count) VALUES (?, ?, '192.0.2.1', ?)",
        )
        .bind(id)
        .bind(report)
        .bind(count)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO record_dkim_auth (record_id, domain, selector, result) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(d)
        .bind(selector)
        .bind(result)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn add(pool: &SqlitePool, id: i64, report: i64, count: i64, d: &str, selector: &str) {
        add_result(pool, id, report, count, d, selector, "pass").await;
    }

    fn names(rows: &[ObservedDkimSelector]) -> Vec<&str> {
        rows.iter().map(|r| r.selector.as_str()).collect()
    }

    #[tokio::test]
    async fn ranks_by_volume_and_ignores_other_zones() {
        let pool = seeded().await;
        add(&pool, 1, 1, 900, "example.com", "news").await;
        add(&pool, 2, 1, 10, "example.com", "selector1").await;
        add(&pool, 3, 1, 5, "mail.example.com", "k1").await;
        // Signed by a third party under its own zone: not our key to publish.
        add(&pool, 4, 1, 5000, "sendgrid.net", "smtpapi").await;
        // Another domain's report entirely.
        add(&pool, 5, 2, 5000, "example.org", "somethingelse").await;

        let got = list_observed_dkim_selectors(&pool, "example.com")
            .await
            .unwrap();
        assert_eq!(names(&got), vec!["news", "selector1", "k1"]);
    }

    #[tokio::test]
    async fn folds_case_and_skips_blanks() {
        let pool = seeded().await;
        add(&pool, 1, 1, 5, "example.com", "News").await;
        add(&pool, 2, 1, 5, "Example.COM", "news").await;
        add(&pool, 3, 1, 900, "example.com", "  ").await;

        let got = list_observed_dkim_selectors(&pool, " EXAMPLE.com. ")
            .await
            .unwrap();
        assert_eq!(names(&got), vec!["news"]);
        assert_eq!(got[0].pass_messages, 10);
    }

    #[tokio::test]
    async fn counts_pass_and_fail_per_selector() {
        let pool = seeded().await;
        add_result(&pool, 1, 1, 40, "example.com", "s1", "pass").await;
        add_result(&pool, 2, 1, 7, "example.com", "s1", "fail").await;
        add_result(&pool, 3, 1, 3, "example.com", "s1", "temperror").await;

        let got = list_observed_dkim_selectors(&pool, "example.com")
            .await
            .unwrap();
        assert_eq!(
            got,
            vec![ObservedDkimSelector {
                selector: "s1".into(),
                pass_messages: 40,
                fail_messages: 10,
            }]
        );
    }

    /// A selector last seen long before the newest report was rotated out; it
    /// must not be probed (and warned about) forever.
    #[tokio::test]
    async fn retired_selectors_fall_out_of_the_window() {
        let pool = seeded().await;
        add(&pool, 1, 1, 5, "example.com", "current").await;
        add(&pool, 2, 3, 9000, "example.com", "retired").await;

        let got = list_observed_dkim_selectors(&pool, "example.com")
            .await
            .unwrap();
        assert_eq!(names(&got), vec!["current"]);
    }

    #[tokio::test]
    async fn unknown_domain_yields_nothing() {
        let pool = seeded().await;
        assert!(list_observed_dkim_selectors(&pool, "nope.example")
            .await
            .unwrap()
            .is_empty());
    }

    /// The domain filter must drive the plan through the `domains.domain`
    /// unique index, never a scan of every DKIM auth row.
    #[tokio::test]
    async fn plan_starts_from_the_domain_index() {
        let pool = seeded().await;
        let sql = format!("EXPLAIN QUERY PLAN {OBSERVED_SELECTORS_SQL}");
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(&sql)
            .bind("example.com")
            .bind(OBSERVED_SELECTOR_WINDOW_SECS)
            .bind("example.com")
            .bind("%.example.com")
            .bind(OBSERVED_SELECTOR_LIMIT)
            .fetch_all(&pool)
            .await
            .unwrap();
        let detail: Vec<&str> = plan.iter().map(|r| r.3.as_str()).collect();
        assert!(
            detail.iter().any(|d| d.contains("SEARCH domains USING")),
            "{detail:?}"
        );
        assert!(!detail.iter().any(|d| d.starts_with("SCAN")), "{detail:?}");
    }
}
