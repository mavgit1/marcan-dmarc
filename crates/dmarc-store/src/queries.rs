//! Helpers shared by the `queries_*` modules: the write transaction, domain
//! name normalization, and the scope every list and total is narrowed by.

use crate::StoreError;
use sqlx::sqlite::SqlitePool;
use sqlx::{QueryBuilder, Sqlite, Transaction};

/// Begin a transaction that reads and then writes.
///
/// `BEGIN IMMEDIATE` takes the write lock up front (waiting out the busy
/// timeout). A plain deferred `BEGIN` starts as a reader, and under WAL its
/// first write fails at once with `SQLITE_BUSY_SNAPSHOT` when another
/// connection (the IMAP import, say) committed since its read: no retry helps.
pub(crate) async fn begin_write(
    pool: &SqlitePool,
) -> Result<Transaction<'static, Sqlite>, StoreError> {
    Ok(pool.begin_with("BEGIN IMMEDIATE").await?)
}

/// A typed list filter as a `LIKE … ESCAPE '\'` pattern that matches any value
/// containing it (`%` and `_` match themselves); `None` when it is blank.
pub(crate) fn contains_pattern(search: Option<&str>) -> Option<String> {
    let search = search.map(str::trim).filter(|s| !s.is_empty())?;
    Some(format!(
        "%{}%",
        search
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    ))
}

/// Ids of the domains shown in Marcan: not ignored.
pub(crate) const VISIBLE_DOMAIN_IDS: &str = "(SELECT id FROM domains WHERE ignored = 0)";

/// What a list or total is narrowed to: an organization, a domain and a
/// `[since, until)` window over `reports.date_begin`. Reports of ignored
/// domains are always left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scope {
    pub organization_id: Option<i64>,
    pub domain_id: Option<i64>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

impl Scope {
    /// Everything (all visible domains, all time).
    pub const ALL: Self = Self {
        organization_id: None,
        domain_id: None,
        since: None,
        until: None,
    };

    /// The usual request scope: organization, domain and a lower time bound.
    pub fn new(organization_id: Option<i64>, domain_id: Option<i64>, since: Option<i64>) -> Self {
        Self {
            organization_id,
            domain_id,
            since,
            until: None,
        }
    }

    /// One domain, from `since` on.
    pub fn domain(domain_id: i64, since: Option<i64>) -> Self {
        Self::new(None, Some(domain_id), since)
    }

    /// The same scope with a different time window.
    pub fn window(self, since: Option<i64>, until: Option<i64>) -> Self {
        Self {
            since,
            until,
            ..self
        }
    }

    /// Append `AND …` clauses onto a query over `reports` aliased `r`.
    pub(crate) fn push_reports(&self, qb: &mut QueryBuilder<'_, Sqlite>, r: &str) {
        qb.push(format!(
            " AND {r}.domain_id IN (SELECT id FROM domains WHERE ignored = 0"
        ));
        if let Some(cid) = self.organization_id {
            qb.push(" AND organization_id = ").push_bind(cid);
        }
        qb.push(")");
        if let Some(did) = self.domain_id {
            qb.push(format!(" AND {r}.domain_id = ")).push_bind(did);
        }
        if let Some(ts) = self.since {
            qb.push(format!(" AND {r}.date_begin >= ")).push_bind(ts);
        }
        if let Some(ts) = self.until {
            qb.push(format!(" AND {r}.date_begin < ")).push_bind(ts);
        }
    }

    /// Append `AND …` clauses onto a query over `domains` aliased `d`: the
    /// visible domains of the organization, or the one domain. The time
    /// window does not apply to domains.
    pub(crate) fn push_domains(&self, qb: &mut QueryBuilder<'_, Sqlite>, d: &str) {
        qb.push(format!(" AND {d}.ignored = 0"));
        if let Some(cid) = self.organization_id {
            qb.push(format!(" AND {d}.organization_id = "))
                .push_bind(cid);
        }
        if let Some(did) = self.domain_id {
            qb.push(format!(" AND {d}.id = ")).push_bind(did);
        }
    }
}
