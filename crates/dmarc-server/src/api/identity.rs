//! Sender identity in responses: who operates a source IP.
//!
//! Evidence is gathered on the server and named by [`dmarc_identity::resolve`]:
//! reverse DNS (persisted in `identity_snapshots` by the warm loop in
//! `snapshots::identity`, so a page load never waits on live DNS) and the SPF
//! passes in reports. Every response row with a `source_ip` carries its
//! `identity`; there is no separate identity endpoint.

use crate::AppState;
use dmarc_identity::{resolve, Evidence, SenderIdentity};
use dmarc_store::{get_identity_snapshots, spf_pass_domains, StoreError};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use ts_rs::TS;

/// A response row plus the identity of its `source_ip`.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct WithIdentity<T> {
    #[serde(flatten)]
    pub row: T,
    pub identity: SenderIdentity,
}

/// Identities for `ips` (stored `source_ip` strings), keyed by the same strings.
pub async fn identities<'a>(
    state: &AppState,
    ips: impl IntoIterator<Item = &'a str>,
) -> Result<HashMap<String, SenderIdentity>, StoreError> {
    let mut seen = HashSet::new();
    let ips: Vec<String> = ips
        .into_iter()
        .filter(|ip| seen.insert(*ip))
        .map(str::to_owned)
        .collect();
    let mut snapshots = get_identity_snapshots(&state.pool, &ips).await?;
    let mut spf = spf_pass_domains(&state.pool, &ips).await?;
    let names = state.names();
    Ok(ips
        .into_iter()
        .map(|raw| {
            let snap = snapshots.remove(&raw);
            let evidence = Evidence {
                ptr_hostname: snap.as_ref().and_then(|s| s.ptr_hostname.clone()),
                ptr_confirmed: snap.and_then(|s| s.ptr_confirmed),
                spf_domain: spf.remove(&raw),
            };
            let identity = resolve(raw.parse().ok(), evidence, &names);
            (raw, identity)
        })
        .collect())
}

/// Attach each row's identity, looked up by `ip_of`.
pub async fn with_identities<T>(
    state: &AppState,
    rows: Vec<T>,
    ip_of: impl Fn(&T) -> &str,
) -> Result<Vec<WithIdentity<T>>, StoreError> {
    let found = identities(state, rows.iter().map(&ip_of)).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let identity = found.get(ip_of(&row)).cloned().unwrap_or_default();
            WithIdentity { row, identity }
        })
        .collect())
}
