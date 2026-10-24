//! The resolvers every DNS check uses, the extra DKIM selectors the checker
//! probes (`dkim_selectors`), and the demo database's own DNS records.

use super::SettingsGroup;
use crate::queries::begin_write;
use crate::StoreError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::collections::BTreeMap;
use std::net::IpAddr;
use ts_rs::TS;

/// Setting key for the made-up DNS records of a demo database (JSON: TXT
/// records by name). Only the demo examples (`demo-data`, `realistic-demo`)
/// write it; while it is stored, every check answers TXT lookups from it
/// instead of the network.
const DEMO_DNS_RECORDS_KEY: &str = "demo_dns_records";

/// The TXT records a demo database's checks answer from, by name; `None`
/// for every real database.
pub async fn get_demo_dns_records(
    pool: &SqlitePool,
) -> Result<Option<BTreeMap<String, Vec<String>>>, StoreError> {
    match super::get_setting(pool, DEMO_DNS_RECORDS_KEY).await? {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

/// Store a demo database's TXT records (the demo examples only).
pub async fn set_demo_dns_records(
    pool: &SqlitePool,
    records: &BTreeMap<String, Vec<String>>,
) -> Result<(), StoreError> {
    super::set_setting(pool, DEMO_DNS_RECORDS_KEY, &serde_json::to_string(records)?).await
}

/// The resolvers every DNS check uses, as stored and as `PUT
/// /api/settings/dns` takes them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DnsSettings {
    /// IP addresses, at most 4; blank entries are dropped when saved, and
    /// each is checked. Empty: the built-in resolvers (`defaults`).
    pub nameservers: Vec<String>,
}

impl SettingsGroup for DnsSettings {
    const KEY: &'static str = "dns";
}

impl DnsSettings {
    /// The resolvers as addresses, skipping any that does not read as one.
    pub fn addresses(&self) -> Vec<IpAddr> {
        self.nameservers
            .iter()
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    }
}

/// One selector name as stored: trimmed and lowercase; `None` when blank.
fn normalize_selector(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    (!s.is_empty()).then_some(s)
}

/// The custom DKIM selectors the checker probes for every domain, sorted.
pub async fn get_custom_dkim_selectors(pool: &SqlitePool) -> Result<Vec<String>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT selector FROM dkim_selectors ORDER BY selector")
            .fetch_all(pool)
            .await?,
    )
}

/// Replace the custom DKIM selectors with `selectors` (normalized, blanks
/// dropped) in one transaction, and return the stored list.
pub async fn set_custom_dkim_selectors(
    pool: &SqlitePool,
    selectors: &[String],
) -> Result<Vec<String>, StoreError> {
    let mut tx = begin_write(pool).await?;
    sqlx::query("DELETE FROM dkim_selectors")
        .execute(&mut *tx)
        .await?;
    for selector in selectors.iter().filter_map(|s| normalize_selector(s)) {
        sqlx::query("INSERT OR IGNORE INTO dkim_selectors (selector) VALUES (?)")
            .bind(selector)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    get_custom_dkim_selectors(pool).await
}
