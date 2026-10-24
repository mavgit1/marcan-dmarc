//! The shape the demo follows: per domain its mail volume over the window,
//! the share of each outcome, how many sending addresses, since when it
//! reports and what its DNS looks like; across domains the weekend dip, the
//! receivers' shares and how many messages a report row holds.
//!
//! Read from a baseline database (aggregates only: no name, address or id
//! leaves it), or the built-in defaults, which are such a reading rounded.

use crate::catalogue::GENERIC_REPORTERS;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use sqlx::Row;
use std::path::Path;

/// Days of reports, ending yesterday.
pub const DAYS: i64 = 90;

/// What a domain's DNS publishes, named by the grade it earns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsClass {
    /// p=reject, -all, 2048-bit keys, TLS reporting: no issue at all.
    Clean,
    /// p=reject, -all: A.
    Reject,
    /// p=reject with SPF ~all: B.
    RejectSoftfail,
    /// p=reject without rua: B, and no reports.
    RejectNoRua,
    /// p=quarantine, SPF near the lookup limit, a 1024-bit key: B.
    Quarantine,
    /// p=none, otherwise sound: D.
    None,
    /// p=none with SPF ?all: D.
    NoneNeutral,
    /// p=none with a 1024-bit key: D.
    NoneDated,
    /// p=none, and a selector seen in reports is not published: D.
    NoneMissingSelector,
    /// p=none with SPF over the lookup limit and ~all: F.
    NoneOverLimit,
}

impl DnsClass {
    pub fn policy(self) -> &'static str {
        match self {
            Self::Clean | Self::Reject | Self::RejectSoftfail | Self::RejectNoRua => "reject",
            Self::Quarantine => "quarantine",
            _ => "none",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Clean => "Clean",
            Self::Reject => "Reject",
            Self::RejectSoftfail => "RejectSoftfail",
            Self::RejectNoRua => "RejectNoRua",
            Self::Quarantine => "Quarantine",
            Self::None => "None",
            Self::NoneNeutral => "NoneNeutral",
            Self::NoneDated => "NoneDated",
            Self::NoneMissingSelector => "NoneMissingSelector",
            Self::NoneOverLimit => "NoneOverLimit",
        }
    }

    /// From a stored DNS check: its policy and the finding codes it raised.
    fn of(policy: Option<&str>, codes: &str) -> Self {
        let has = |code: &str| codes.split(',').any(|c| c == code);
        match policy {
            Some("reject" | "quarantine") if has("dmarc_no_rua") => Self::RejectNoRua,
            Some("quarantine") => Self::Quarantine,
            Some("reject") if has("spf_softfail_all") => Self::RejectSoftfail,
            Some("reject") => Self::Reject,
            _ if has("spf_lookup_limit") => Self::NoneOverLimit,
            _ if has("spf_neutral_all") => Self::NoneNeutral,
            _ if has("dkim_key_dated") => Self::NoneDated,
            _ if has("dkim_observed_selector_missing") => Self::NoneMissingSelector,
            _ => Self::None,
        }
    }
}

/// One domain's traffic, as shares of its own mail.
#[derive(Clone, Debug)]
pub struct Shape {
    /// Messages over [`DAYS`].
    pub messages: f64,
    pub passed: f64,
    pub blocked: f64,
    pub forwarded: f64,
    pub delivered: f64,
    /// Distinct sending addresses over [`DAYS`].
    pub ips: usize,
    /// Days since its first report (at most [`DAYS`]).
    pub first_ago: i64,
    pub dns: DnsClass,
}

pub struct Profile {
    /// Largest first.
    pub shapes: Vec<Shape>,
    /// A weekend day's mail relative to a weekday's.
    pub weekend: f64,
    /// Share of report rows per [`GENERIC_REPORTERS`] entry, then the tail.
    pub reporters: Vec<f64>,
    /// Messages per report row.
    pub messages_per_row: f64,
}

/// `(messages, passed %, blocked %, forwarded %, delivered %, addresses,
/// first report days ago, DNS)`, a baseline reading rounded.
type Row8 = (f64, f64, f64, f64, f64, usize, i64, DnsClass);

const DEFAULT_SHAPES: [Row8; 42] = {
    use DnsClass::*;
    [
        (18700.0, 99.1, 0.0, 0.1, 0.8, 422, 89, None),
        (13300.0, 90.0, 0.0, 7.9, 2.1, 459, 24, NoneMissingSelector),
        (11700.0, 65.9, 34.1, 0.0, 0.0, 174, 89, Quarantine),
        (2530.0, 99.8, 0.1, 0.0, 0.1, 159, 89, Reject),
        (2110.0, 97.4, 0.0, 0.0, 2.6, 218, 24, None),
        (1710.0, 93.7, 0.0, 2.6, 3.8, 83, 23, NoneDated),
        (1300.0, 88.9, 0.0, 1.5, 9.6, 31, 89, None),
        (1050.0, 99.7, 0.3, 0.0, 0.0, 42, 88, RejectSoftfail),
        (845.0, 99.4, 0.0, 0.0, 0.6, 104, 24, None),
        (706.0, 93.3, 0.0, 1.1, 5.5, 178, 24, None),
        (559.0, 100.0, 0.0, 0.0, 0.0, 41, 89, None),
        (415.0, 97.6, 0.0, 0.0, 2.4, 26, 89, None),
        (318.0, 88.4, 0.0, 0.0, 11.6, 19, 89, NoneNeutral),
        (317.0, 100.0, 0.0, 0.0, 0.0, 20, 23, None),
        (254.0, 97.2, 2.8, 0.0, 0.0, 24, 57, Reject),
        (229.0, 92.1, 0.0, 0.0, 7.9, 21, 24, None),
        (167.0, 74.9, 0.0, 0.0, 25.1, 27, 23, None),
        (141.0, 46.1, 53.9, 0.0, 0.0, 6, 86, Reject),
        (102.0, 96.1, 0.0, 1.0, 2.9, 18, 24, NoneNeutral),
        (91.0, 73.6, 0.0, 0.0, 26.4, 42, 24, None),
        (88.0, 100.0, 0.0, 0.0, 0.0, 8, 24, NoneDated),
        (72.0, 100.0, 0.0, 0.0, 0.0, 7, 80, None),
        (57.0, 98.2, 0.0, 0.0, 1.8, 25, 23, None),
        (45.0, 100.0, 0.0, 0.0, 0.0, 4, 70, NoneDated),
        (45.0, 75.6, 0.0, 2.2, 22.2, 15, 24, NoneOverLimit),
        (39.0, 84.6, 0.0, 12.8, 2.6, 13, 89, NoneNeutral),
        (34.0, 100.0, 0.0, 0.0, 0.0, 10, 20, None),
        (34.0, 100.0, 0.0, 0.0, 0.0, 8, 18, NoneNeutral),
        (28.0, 0.0, 0.0, 0.0, 100.0, 6, 88, None),
        (23.0, 82.6, 0.0, 0.0, 17.4, 15, 16, None),
        (19.0, 100.0, 0.0, 0.0, 0.0, 10, 87, None),
        (17.0, 41.2, 0.0, 0.0, 58.8, 15, 24, None),
        (14.0, 0.0, 0.0, 0.0, 100.0, 4, 24, None),
        (9.0, 88.9, 0.0, 0.0, 11.1, 7, 22, NoneNeutral),
        (8.0, 0.0, 0.0, 0.0, 100.0, 4, 22, None),
        (8.0, 100.0, 0.0, 0.0, 0.0, 5, 20, None),
        (5.0, 80.0, 0.0, 0.0, 20.0, 3, 22, NoneDated),
        (3.0, 0.0, 0.0, 0.0, 100.0, 2, 23, NoneNeutral),
        (3.0, 100.0, 0.0, 0.0, 0.0, 3, 10, None),
        (2.0, 0.0, 0.0, 0.0, 100.0, 2, 37, None),
        (2.0, 0.0, 0.0, 0.0, 100.0, 2, 18, None),
        (0.0, 0.0, 0.0, 0.0, 0.0, 0, 0, RejectNoRua),
    ]
};

/// Report-row shares of the receivers, in [`GENERIC_REPORTERS`] order, then
/// everyone else.
const DEFAULT_REPORTERS: [f64; 10] = [
    0.815, 0.06, 0.018, 0.024, 0.012, 0.006, 0.001, 0.002, 0.002, 0.06,
];

pub fn defaults() -> Profile {
    let shapes = DEFAULT_SHAPES
        .iter()
        .map(|&(messages, p, b, f, d, ips, first_ago, dns)| Shape {
            messages,
            passed: p / 100.0,
            blocked: b / 100.0,
            forwarded: f / 100.0,
            delivered: d / 100.0,
            ips,
            first_ago,
            dns,
        })
        .collect();
    Profile {
        shapes,
        weekend: 0.24,
        reporters: DEFAULT_REPORTERS.to_vec(),
        messages_per_row: 1.76,
    }
}

/// Read the shape of the database at `path`, opened read-only and immutable
/// (nothing is written to it, not even a lock). The window is the
/// [`DAYS`] before its newest report.
pub async fn from_baseline(path: &Path) -> anyhow::Result<Profile> {
    anyhow::ensure!(path.is_file(), "baseline {} not found", path.display());
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .immutable(true);
    let pool = SqlitePool::connect_with(options).await?;
    let end: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(date_end), 0) FROM reports")
        .fetch_one(&pool)
        .await?;
    anyhow::ensure!(end > 0, "the baseline holds no reports");
    let start = end - DAYS * 86_400;

    let outcome = "(CASE WHEN rec.dmarc_pass = 1 THEN 'p' \
         WHEN NOT (rec.disposition IS NULL OR rec.disposition = 'none') THEN 'b' \
         WHEN rec.forwarded = 1 THEN 'f' ELSE 'd' END)";
    let sql = format!(
        r#"
        WITH o AS (
            SELECT r.domain_id, r.date_begin, rec.count AS c, rec.source_ip AS ip, {outcome} AS oc
            FROM records rec JOIN reports r ON r.id = rec.report_id
            WHERE r.date_begin >= ?1
        )
        SELECT
            COALESCE(SUM(o.c), 0) AS total,
            COALESCE(SUM(CASE WHEN o.oc = 'p' THEN o.c END), 0) AS p,
            COALESCE(SUM(CASE WHEN o.oc = 'b' THEN o.c END), 0) AS b,
            COALESCE(SUM(CASE WHEN o.oc = 'f' THEN o.c END), 0) AS f,
            COALESCE(SUM(CASE WHEN o.oc = 'd' THEN o.c END), 0) AS d,
            COUNT(DISTINCT o.ip) AS ips,
            MIN(o.date_begin) AS first_begin,
            s.policy_p AS policy,
            (SELECT GROUP_CONCAT(json_extract(f.value, '$.code'))
             FROM json_each(s.report_json, '$.findings') f) AS codes
        FROM domains dom
        LEFT JOIN dns_snapshots s ON s.domain_id = dom.id
        LEFT JOIN o ON o.domain_id = dom.id
        WHERE dom.ignored = 0 AND dom.parked = 0
        GROUP BY dom.id
        "#
    );
    let rows = sqlx::query(&sql).bind(start).fetch_all(&pool).await?;
    let mut shapes: Vec<Shape> = rows
        .iter()
        .map(|row| {
            let total: i64 = row.get("total");
            let share = |name: &str| {
                let n: i64 = row.get(name);
                if total > 0 {
                    n as f64 / total as f64
                } else {
                    0.0
                }
            };
            let first: Option<i64> = row.get("first_begin");
            let policy: Option<String> = row.get("policy");
            let codes: Option<String> = row.get("codes");
            Shape {
                messages: total as f64,
                passed: share("p"),
                blocked: share("b"),
                forwarded: share("f"),
                delivered: share("d"),
                ips: row.get::<i64, _>("ips") as usize,
                first_ago: first.map_or(0, |f| ((end - f) / 86_400).clamp(1, DAYS)),
                dns: DnsClass::of(policy.as_deref(), codes.as_deref().unwrap_or("")),
            }
        })
        .collect();
    shapes.sort_by(|a, b| b.messages.total_cmp(&a.messages));
    anyhow::ensure!(!shapes.is_empty(), "the baseline holds no domains");

    // Weekend dip: the average weekend day against the average weekday.
    let days: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT date_begin / 86400 AS day, SUM(total_messages) FROM reports \
         WHERE date_begin >= ? GROUP BY day",
    )
    .bind(start)
    .fetch_all(&pool)
    .await?;
    let mean = |weekend: bool| {
        let v: Vec<f64> = days
            .iter()
            .filter(|(day, _)| ((day + 3) % 7 >= 5) == weekend)
            .map(|(_, n)| *n as f64)
            .collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    let weekend = (mean(true) / mean(false).max(1.0)).clamp(0.05, 1.0);

    // Receivers: report rows by org_name, the generic ones by name.
    let by_reporter: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT LOWER(r.org_name), COUNT(*), SUM(rec.count) FROM records rec \
         JOIN reports r ON r.id = rec.report_id WHERE r.date_begin >= ? GROUP BY 1",
    )
    .bind(start)
    .fetch_all(&pool)
    .await?;
    let mut reporters = vec![0.0; GENERIC_REPORTERS.len() + 1];
    let (mut rows_total, mut messages_total) = (0i64, 0i64);
    for (name, rows, messages) in &by_reporter {
        let slot = GENERIC_REPORTERS
            .iter()
            .position(|r| r.org.to_lowercase() == *name)
            .unwrap_or(GENERIC_REPORTERS.len());
        reporters[slot] += *rows as f64;
        rows_total += rows;
        messages_total += messages;
    }
    let messages_per_row = messages_total as f64 / rows_total.max(1) as f64;
    for share in &mut reporters {
        *share /= rows_total.max(1) as f64;
    }
    pool.close().await;
    Ok(Profile {
        shapes,
        weekend,
        reporters,
        messages_per_row: messages_per_row.max(1.0),
    })
}

impl Profile {
    /// The profile as Rust source for [`DEFAULT_SHAPES`] (`--print-profile`).
    pub fn print(&self) {
        println!(
            "weekend: {:.2}, messages per row: {:.2}",
            self.weekend, self.messages_per_row
        );
        println!(
            "reporters: {:?}",
            self.reporters
                .iter()
                .map(|r| (r * 1000.0).round() / 1000.0)
                .collect::<Vec<_>>()
        );
        for s in &self.shapes {
            println!(
                "({:.0}.0, {:.1}, {:.1}, {:.1}, {:.1}, {}, {}, {}),",
                round_sig(s.messages),
                s.passed * 100.0,
                s.blocked * 100.0,
                s.forwarded * 100.0,
                s.delivered * 100.0,
                s.ips,
                s.first_ago,
                s.dns.code()
            );
        }
    }

    /// `count` shapes spread over this profile's range, largest first: the
    /// baseline resampled to the demo's number of domains.
    pub fn spread(&self, count: usize) -> Vec<Shape> {
        let n = self.shapes.len();
        (0..count)
            .map(|i| self.shapes[(i * n / count).min(n - 1)].clone())
            .collect()
    }
}

/// Three significant digits.
fn round_sig(x: f64) -> f64 {
    if x < 1000.0 {
        return x.round();
    }
    let scale = 10f64.powi(x.log10().floor() as i32 - 2);
    (x / scale).round() * scale
}
