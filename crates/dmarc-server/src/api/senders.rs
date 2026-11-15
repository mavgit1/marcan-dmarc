//! `GET /api/senders` — source IPs in a scope, sorted and paged.
//! `GET /api/senders/{ip}` — one IP across domains.

use super::identity::{identities, with_identities, WithIdentity};
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path, Query};
use crate::filters::{parse_pagination, parse_sort, Page, ScopeQuery};
use crate::snapshots::dns::load_complete_snapshots;
use crate::view::sender::{SenderDetail, SenderSummary};
use crate::AppState;
use axum::extract::State;
use dmarc_dns::{spf_ip_match, SpfIpMatch};
use dmarc_identity::SenderIdentity;
use dmarc_store::{
    get_sender_by_ip, resolve_source_ip, source_ips_in_scope, unix_now, Scope, SenderListQuery,
    SenderSort,
};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Deserialize)]
pub struct SendersQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// `total_messages`, `failed_messages`, `compliance_pct`, … ; `-` prefix for descending.
    pub sort: Option<String>,
    /// `true`: only senders with mail that failed DMARC and was not forwarded.
    pub failing: Option<bool>,
    /// Case-insensitive substring of the source IP or the sender's name (the
    /// name shown for it, or its operator's domain).
    pub q: Option<String>,
}

pub async fn list_senders(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ScopeQuery>,
    Query(q): Query<SendersQuery>,
) -> AppResult<Json<Page<WithIdentity<SenderSummary>>>> {
    let (limit, offset) = parse_pagination(q.limit, q.offset)?;
    let scope = scope.scope()?;
    let only_ips = match q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(search) => Some(search_ips(&state, scope, search).await?),
        None => None,
    };
    let query = SenderListQuery {
        scope,
        failing: q.failing.unwrap_or(false),
        only_ips,
        sort: parse_sort(q.sort.as_deref(), SenderSort::DEFAULT)?,
        limit,
        offset,
    };
    let (items, total) = dmarc_store::list_senders(&state.pool, &query).await?;
    let items: Vec<SenderSummary> = items.into_iter().map(Into::into).collect();
    let items = with_identities(&state, items, |s| &s.row.source_ip).await?;
    Ok(Json(Page::new(items, total, limit, offset)))
}

/// The source IPs in `scope` whose address or name contains `search`. Names
/// are resolved here, not stored, so the search runs over the scope's IPs.
async fn search_ips(state: &AppState, scope: Scope, search: &str) -> AppResult<Vec<String>> {
    let ips = source_ips_in_scope(&state.pool, scope).await?;
    let found = identities(state, ips.iter().map(String::as_str)).await?;
    let needle = search.to_lowercase();
    Ok(ips
        .into_iter()
        .filter(|ip| matches_search(ip, found.get(ip), &needle))
        .collect())
}

/// Whether `needle` (lower case) is part of the IP, the sender's name or its
/// operator's domain: what the list shows for the row.
fn matches_search(ip: &str, identity: Option<&SenderIdentity>, needle: &str) -> bool {
    let named = identity.is_some_and(|i| {
        [i.name.as_deref(), i.operator_domain.as_deref()]
            .into_iter()
            .flatten()
            .any(|text| text.to_lowercase().contains(needle))
    });
    named || ip.to_lowercase().contains(needle)
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SenderDetailResponse {
    #[serde(flatten)]
    pub detail: SenderDetail,
    pub identity: SenderIdentity,
    /// Domains whose own SPF lists this IP by address, and where.
    pub spf_listings: Vec<SpfListing>,
}

/// Where one sending domain's SPF (its record or an include) lists the IP.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SpfListing {
    pub domain_id: i64,
    #[serde(flatten)]
    pub listed: SpfIpMatch,
}

pub async fn get_sender(
    State(state): State<Arc<AppState>>,
    Path(ip): Path<String>,
    Query(q): Query<ScopeQuery>,
) -> AppResult<Json<SenderDetailResponse>> {
    let parsed = parse_path_ip(&ip)?;
    let scope = q.scope()?;
    let mut candidates = vec![ip.clone(), parsed.to_string()];
    candidates.sort();
    candidates.dedup();
    let Some(stored_ip) = resolve_source_ip(&state.pool, &candidates, scope).await? else {
        return Err(AppError::not_found(format!("sender {parsed} not found")));
    };
    let detail: SenderDetail = get_sender_by_ip(&state.pool, &stored_ip, scope, unix_now())
        .await?
        .ok_or_else(|| AppError::not_found(format!("sender {parsed} not found")))?
        .into();
    let identity = identities(&state, [stored_ip.as_str()])
        .await?
        .remove(&stored_ip)
        .unwrap_or_default();
    let spf_listings = spf_listings(&state, &detail, parsed).await?;
    Ok(Json(SenderDetailResponse {
        detail,
        identity,
        spf_listings,
    }))
}

/// Match `ip` against each sending domain's last complete SPF check. Only
/// literal `ip4:` / `ip6:` terms can match, so a domain without a listing may
/// still authorize the IP through `a`, `mx` or `exists`.
async fn spf_listings(
    state: &AppState,
    detail: &SenderDetail,
    ip: IpAddr,
) -> AppResult<Vec<SpfListing>> {
    let domains: Vec<String> = detail.items.iter().map(|i| i.row.domain.clone()).collect();
    let reports = load_complete_snapshots(&state.pool, &domains).await?;
    let mut out = Vec::new();
    for item in &detail.items {
        let Some(report) = reports.get(&item.row.domain) else {
            continue;
        };
        if let Some(listed) = spf_ip_match(&report.spf.include_tree, ip) {
            out.push(SpfListing {
                domain_id: item.row.domain_id,
                listed,
            });
        }
    }
    Ok(out)
}

fn parse_path_ip(raw: &str) -> AppResult<IpAddr> {
    let stripped = raw
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(raw);
    stripped
        .parse::<IpAddr>()
        .or_else(|_| {
            let decoded = stripped.replace("%3A", ":").replace("%3a", ":");
            decoded.parse()
        })
        .map_err(|_| AppError::bad_request(format!("invalid IP address: {raw}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_matches_the_ip_the_name_or_the_operator() {
        let google = SenderIdentity {
            name: Some("Google".into()),
            operator_domain: Some("google.com".into()),
            ..SenderIdentity::default()
        };
        assert!(matches_search("209.85.220.41", Some(&google), "goog"));
        assert!(matches_search("209.85.220.41", Some(&google), "google.com"));
        assert!(matches_search("209.85.220.41", None, "209.85"));
        assert!(!matches_search("209.85.220.41", Some(&google), "outlook"));
        assert!(matches_search("2001:DB8::1", None, "2001:db8"));
    }
}
