//! Bulk organization ↔ domain mapping: the current mapping, and a plan to move,
//! add, (un)park or (un)ignore listed domains, previewed or applied in one
//! transaction.
//!
//! Same rules as the single-domain paths in `queries.rs`: names are stored
//! normalized, and a listed domain gets exactly the status its entry gives.
//! Ignoring is removing ([`crate::ignore_domain`]: its reports are deleted and
//! new ones dropped); listing an ignored domain without `ignored` monitors it
//! again. Domains that are not listed are left alone, and organizations left
//! without domains are kept.

use crate::queries::begin_write;
use crate::queries_domains::ignore_domain_in;
use crate::StoreError;
use dmarc_domain::normalize_name;
use serde::Serialize;
use sqlx::sqlite::{SqliteConnection, SqlitePool};
use std::collections::HashMap;
use ts_rs::TS;

/// What Marcan does with a domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainStatus {
    Monitored,
    /// Sends no mail of its own.
    Parked,
    /// Removed (ignored): no reports are kept for it.
    Ignored,
}

/// One listed domain, the organization (by name) it should belong to, and its
/// status. `domain` must already be validated; it is normalized again before use.
#[derive(Debug, Clone)]
pub struct MappingEntry {
    pub domain: String,
    pub org: String,
    pub status: DomainStatus,
}

/// A visible domain that changes organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MappingMove {
    pub domain: String,
    pub from: String,
    pub to: String,
}

/// A domain added to, or monitored again in, an organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MappingPlacement {
    pub domain: String,
    pub org: String,
}

/// A domain that becomes ignored, and how many of its reports that deletes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MappingIgnore {
    pub domain: String,
    pub reports: i64,
}

/// What a domain mapping changes, as the preview and apply answers list it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MappingChanges {
    #[serde(rename = "move")]
    pub moves: Vec<MappingMove>,
    pub add: Vec<MappingPlacement>,
    /// Domains that become ignored (new ones included).
    pub ignore: Vec<MappingIgnore>,
    /// Ignored domains that are monitored again; the mailbox is re-imported for
    /// their history.
    pub unignore: Vec<MappingPlacement>,
    /// Organizations that do not exist yet, with the name they will get.
    pub create_orgs: Vec<String>,
    /// Domains that become parked (new ones included).
    pub park: Vec<String>,
    /// Domains that stop being parked.
    pub unpark: Vec<String>,
}

/// What a mapping changes, and how many listed domains are already in place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MappingPlan {
    pub changes: MappingChanges,
    pub unchanged_count: usize,
}

impl MappingPlan {
    pub fn is_empty(&self) -> bool {
        let c = &self.changes;
        c.moves.is_empty()
            && c.add.is_empty()
            && c.ignore.is_empty()
            && c.unignore.is_empty()
            && c.create_orgs.is_empty()
            && c.park.is_empty()
            && c.unpark.is_empty()
    }
}

struct Snapshot {
    /// `(id, name)` of every organization.
    organizations: Vec<(i64, String)>,
    /// Every domain, ignored ones included, by normalized name.
    domains: HashMap<String, SnapshotDomain>,
}

struct SnapshotDomain {
    id: i64,
    organization_id: i64,
    ignored: bool,
    parked: bool,
    reports: i64,
}

async fn load_snapshot(conn: &mut SqliteConnection) -> Result<Snapshot, StoreError> {
    let organizations: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM organizations ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    let rows: Vec<(i64, String, i64, bool, bool, i64)> = sqlx::query_as(
        "SELECT d.id, d.domain, d.organization_id, d.ignored, d.parked, \
         (SELECT COUNT(*) FROM reports r WHERE r.domain_id = d.id) FROM domains d",
    )
    .fetch_all(&mut *conn)
    .await?;
    let domains = rows
        .into_iter()
        .map(|(id, domain, organization_id, ignored, parked, reports)| {
            (
                domain,
                SnapshotDomain {
                    id,
                    organization_id,
                    ignored,
                    parked,
                    reports,
                },
            )
        })
        .collect();
    Ok(Snapshot {
        organizations,
        domains,
    })
}

/// Target of a listed organization name.
#[derive(Clone, Copy)]
enum Target {
    Existing(i64),
    /// Index into `create_orgs`.
    New(usize),
}

enum Op {
    Move {
        domain: String,
        to: Target,
    },
    Unignore {
        domain: String,
        to: Target,
        parked: bool,
    },
    Add {
        domain: String,
        to: Target,
        parked: bool,
        ignored: bool,
    },
    Park {
        domain: String,
        parked: bool,
    },
    Block {
        id: i64,
        to: Target,
    },
}

/// Match a name to an organization: exact name first, then case-insensitive (lowest id).
fn find_organization(organizations: &[(i64, String)], name: &str) -> Option<(i64, String)> {
    if let Some((id, n)) = organizations.iter().find(|(_, n)| n == name) {
        return Some((*id, n.clone()));
    }
    let lower = name.to_lowercase();
    organizations
        .iter()
        .find(|(_, n)| n.to_lowercase() == lower)
        .map(|(id, n)| (*id, n.clone()))
}

/// Planned writes: the summary, the operations, and the names of the
/// organizations to create in `Target::New` index order.
struct Planned {
    summary: MappingPlan,
    ops: Vec<Op>,
    create_order: Vec<String>,
}

fn plan(snapshot: &Snapshot, entries: &[MappingEntry]) -> Planned {
    let mut changes = MappingChanges::default();
    let mut unchanged_count = 0;
    let mut ops = Vec::new();
    let mut create_order: Vec<String> = Vec::new();
    // Listed name (lowercased) → target and the name shown in the summary.
    let mut targets: HashMap<String, (Target, String)> = HashMap::new();
    let organization_name = |id: i64| -> String {
        snapshot
            .organizations
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_default()
    };

    for entry in entries {
        let org = entry.org.trim();
        let (target, org_name) = targets
            .entry(org.to_lowercase())
            .or_insert_with(|| match find_organization(&snapshot.organizations, org) {
                Some((id, name)) => (Target::Existing(id), name),
                None => {
                    create_order.push(org.to_owned());
                    (Target::New(create_order.len() - 1), org.to_owned())
                }
            })
            .clone();

        let domain = normalize_name(&entry.domain);
        let parked = entry.status == DomainStatus::Parked;
        let ignored = entry.status == DomainStatus::Ignored;
        let existing = snapshot.domains.get(&domain);
        let was_parked = existing.is_some_and(|d| d.parked);
        if !ignored && parked != was_parked {
            if parked {
                changes.park.push(domain.clone());
            } else {
                changes.unpark.push(domain.clone());
            }
        }
        let in_org =
            |e: &SnapshotDomain| matches!(target, Target::Existing(id) if id == e.organization_id);
        match existing {
            None if ignored => {
                changes.ignore.push(MappingIgnore {
                    domain: domain.clone(),
                    reports: 0,
                });
                ops.push(Op::Add {
                    domain,
                    to: target,
                    parked: false,
                    ignored: true,
                });
            }
            None => {
                changes.add.push(MappingPlacement {
                    domain: domain.clone(),
                    org: org_name,
                });
                ops.push(Op::Add {
                    domain,
                    to: target,
                    parked,
                    ignored: false,
                });
            }
            Some(existing) if existing.ignored && ignored => {
                if in_org(existing) {
                    unchanged_count += 1;
                } else {
                    changes.moves.push(MappingMove {
                        domain: domain.clone(),
                        from: organization_name(existing.organization_id),
                        to: org_name,
                    });
                    ops.push(Op::Move { domain, to: target });
                }
            }
            Some(existing) if existing.ignored => {
                changes.unignore.push(MappingPlacement {
                    domain: domain.clone(),
                    org: org_name,
                });
                ops.push(Op::Unignore {
                    domain,
                    to: target,
                    parked,
                });
            }
            Some(existing) if ignored => {
                changes.ignore.push(MappingIgnore {
                    domain: domain.clone(),
                    reports: existing.reports,
                });
                ops.push(Op::Block {
                    id: existing.id,
                    to: target,
                });
            }
            Some(existing) => {
                let same_org = in_org(existing);
                if !same_org {
                    changes.moves.push(MappingMove {
                        domain: domain.clone(),
                        from: organization_name(existing.organization_id),
                        to: org_name,
                    });
                    ops.push(Op::Move {
                        domain: domain.clone(),
                        to: target,
                    });
                }
                if parked != existing.parked {
                    ops.push(Op::Park { domain, parked });
                } else if same_org {
                    unchanged_count += 1;
                }
            }
        }
    }

    changes.moves.sort_by(|a, b| a.domain.cmp(&b.domain));
    changes.add.sort_by(|a, b| a.domain.cmp(&b.domain));
    changes.ignore.sort_by(|a, b| a.domain.cmp(&b.domain));
    changes.unignore.sort_by(|a, b| a.domain.cmp(&b.domain));
    changes.park.sort();
    changes.unpark.sort();
    changes.create_orgs.clone_from(&create_order);
    changes.create_orgs.sort_by(|a, b| by_name(a, b));
    Planned {
        summary: MappingPlan {
            changes,
            unchanged_count,
        },
        ops,
        create_order,
    }
}

/// Case-insensitive name order, exact order as tie-break.
fn by_name(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
}

/// What `entries` would change, without writing anything.
pub async fn preview_domain_mapping(
    pool: &SqlitePool,
    entries: &[MappingEntry],
) -> Result<MappingPlan, StoreError> {
    let mut conn = pool.acquire().await?;
    let snapshot = load_snapshot(&mut conn).await?;
    Ok(plan(&snapshot, entries).summary)
}

/// Apply `entries` in one transaction: every change is written, or none is.
/// The plan is computed inside the transaction, so it describes exactly what
/// was written.
pub async fn apply_domain_mapping(
    pool: &SqlitePool,
    entries: &[MappingEntry],
) -> Result<MappingPlan, StoreError> {
    let mut tx = begin_write(pool).await?;
    let snapshot = load_snapshot(&mut tx).await?;
    let Planned {
        summary,
        ops,
        create_order,
    } = plan(&snapshot, entries);

    let mut new_ids = Vec::with_capacity(create_order.len());
    for name in &create_order {
        let done = sqlx::query("INSERT INTO organizations (name) VALUES (?)")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        new_ids.push(done.last_insert_rowid());
    }
    let resolve = |t: Target| match t {
        Target::Existing(id) => id,
        Target::New(i) => new_ids[i],
    };

    for op in &ops {
        if let Op::Move { domain, to } = op {
            sqlx::query("UPDATE domains SET organization_id = ? WHERE domain = ?")
                .bind(resolve(*to))
                .bind(domain)
                .execute(&mut *tx)
                .await?;
        }
    }
    for op in &ops {
        if let Op::Park { domain, parked } = op {
            sqlx::query("UPDATE domains SET parked = ? WHERE domain = ? AND ignored = 0")
                .bind(parked)
                .bind(domain)
                .execute(&mut *tx)
                .await?;
        }
    }
    for op in &ops {
        if let Op::Unignore { domain, to, parked } = op {
            sqlx::query(
                "UPDATE domains SET ignored = 0, organization_id = ?, parked = ? WHERE domain = ? AND ignored = 1",
            )
            .bind(resolve(*to))
            .bind(parked)
            .bind(domain)
            .execute(&mut *tx)
            .await?;
        }
    }
    for op in &ops {
        if let Op::Add {
            domain,
            to,
            parked,
            ignored,
        } = op
        {
            sqlx::query(
                "INSERT INTO domains (organization_id, domain, parked, ignored) VALUES (?, ?, ?, ?)",
            )
            .bind(resolve(*to))
            .bind(domain)
            .bind(parked)
            .bind(ignored)
            .execute(&mut *tx)
            .await?;
        }
    }
    for op in &ops {
        if let Op::Block { id, to } = op {
            sqlx::query("UPDATE domains SET organization_id = ? WHERE id = ?")
                .bind(resolve(*to))
                .bind(id)
                .execute(&mut *tx)
                .await?;
            ignore_domain_in(&mut tx, *id).await?;
        }
    }

    tx.commit().await?;
    Ok(summary)
}

/// The current mapping: every domain as `(domain, organization, status)`,
/// ignored ones last, each part sorted by organization name (case-insensitive),
/// then domain.
pub async fn domain_mapping_rows(
    pool: &SqlitePool,
) -> Result<Vec<(String, String, DomainStatus)>, StoreError> {
    let mut rows: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT d.domain, org.name, d.parked, d.ignored FROM domains d \
         JOIN organizations org ON org.id = d.organization_id",
    )
    .fetch_all(pool)
    .await?;
    rows.sort_by(|(da, oa, _, ba), (db, ob, _, bb)| {
        ba.cmp(bb)
            .then_with(|| by_name(oa, ob))
            .then_with(|| da.cmp(db))
    });
    Ok(rows
        .into_iter()
        .map(|(domain, org, parked, ignored)| {
            let status = if ignored {
                DomainStatus::Ignored
            } else if parked {
                DomainStatus::Parked
            } else {
                DomainStatus::Monitored
            };
            (domain, org, status)
        })
        .collect())
}
