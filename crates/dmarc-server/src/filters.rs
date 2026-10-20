//! Shared query-parameter helpers and the paginated response envelope.

use crate::error::{AppError, AppResult};
use crate::since::parse_since;
use dmarc_store::{Scope, Sort, SortKey};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Common organization / domain / since filters used by several endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScopeQuery {
    pub organization_id: Option<i64>,
    pub domain_id: Option<i64>,
    pub since: Option<String>,
}

impl ScopeQuery {
    /// Resolve `since` into a unix lower bound (or `None`).
    pub fn since_ts(&self) -> AppResult<Option<i64>> {
        parse_since(self.since.as_deref())
    }

    /// The store scope these parameters name.
    pub fn scope(&self) -> AppResult<Scope> {
        Ok(Scope::new(
            self.organization_id,
            self.domain_id,
            self.since_ts()?,
        ))
    }
}

/// Envelope of every paginated list: one page of `items` plus the total row count.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: i64, limit: i64, offset: i64) -> Self {
        Self {
            items,
            total,
            limit,
            offset,
        }
    }
}

/// Envelope of every unpaged list.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Items<T> {
    pub items: Vec<T>,
}

impl<T> Items<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self { items }
    }
}

/// Parse pagination: `limit` (default 50, max 500) and `offset` (default 0).
pub fn parse_pagination(limit: Option<i64>, offset: Option<i64>) -> AppResult<(i64, i64)> {
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);
    if !(1..=500).contains(&limit) {
        return Err(AppError::bad_request(
            "limit must be between 1 and 500".to_owned(),
        ));
    }
    if offset < 0 {
        return Err(AppError::bad_request(
            "offset must be non-negative".to_owned(),
        ));
    }
    Ok((limit, offset))
}

/// Parse `sort` (`key` ascending, `-key` descending); `default` when absent.
pub fn parse_sort<K: SortKey>(raw: Option<&str>, default: Sort<K>) -> AppResult<Sort<K>> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(value) => Sort::parse(value)
            .ok_or_else(|| AppError::bad_request(format!("invalid sort '{value}'"))),
    }
}
