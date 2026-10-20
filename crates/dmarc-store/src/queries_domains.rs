//! Domains: create, move, park, ignore, and lookups by name.

use crate::queries::begin_write;
use crate::StoreError;
use dmarc_domain::normalize_name;
use sqlx::sqlite::{SqliteConnection, SqlitePool};
use sqlx::FromRow;

/// A row from the `domains` table.
#[derive(Debug, Clone, FromRow)]
pub struct Domain {
    pub id: i64,
    pub organization_id: i64,
    pub domain: String,
}

/// Look up a domain by name across all organizations. `domain` is normalized first.
pub async fn find_domain_by_name(
    pool: &SqlitePool,
    domain: &str,
) -> Result<Option<Domain>, StoreError> {
    let row = sqlx::query_as::<_, Domain>(
        "SELECT id, organization_id, domain FROM domains WHERE domain = ?",
    )
    .bind(normalize_name(domain))
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Return the domain named `domain` wherever it lives; create it under
/// `organization_id` only when no organization has it yet. Domain names are unique
/// across organizations, so the returned row may belong to another organization, and it
/// may be ignored: ingest drops its reports.
pub async fn get_or_create_domain(
    pool: &SqlitePool,
    organization_id: i64,
    domain: &str,
) -> Result<Domain, StoreError> {
    let domain = normalize_name(domain);
    sqlx::query("INSERT OR IGNORE INTO domains (organization_id, domain) VALUES (?, ?)")
        .bind(organization_id)
        .bind(&domain)
        .execute(pool)
        .await?;

    let row = sqlx::query_as::<_, Domain>(
        "SELECT id, organization_id, domain FROM domains WHERE domain = ?",
    )
    .bind(&domain)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Outcome of an explicit domain create (not ingest). Distinct from
/// `get_or_create_domain`: a duplicate is a conflict the caller must surface,
/// not a silent no-op. Domains are allowed to exist with zero reports.
#[derive(Debug, Clone)]
pub enum CreateDomainResult {
    Created(Domain),
    /// The name belonged to an ignored (removed) domain: it is monitored again,
    /// in the requested organization, without history until the mailbox is re-imported.
    Unignored(Domain),
    /// The name is taken (in any organization) by a visible domain. Carries the
    /// existing row and the name of the organization that holds it.
    AlreadyExists {
        domain: Domain,
        organization_name: String,
    },
    OrganizationNotFound,
}

/// Insert a domain under an existing organization. Does not require any reports.
/// `domain` is normalized before it is stored.
pub async fn create_domain(
    pool: &SqlitePool,
    organization_id: i64,
    domain: &str,
) -> Result<CreateDomainResult, StoreError> {
    let organization: Option<i64> = sqlx::query_scalar("SELECT id FROM organizations WHERE id = ?")
        .bind(organization_id)
        .fetch_optional(pool)
        .await?;
    if organization.is_none() {
        return Ok(CreateDomainResult::OrganizationNotFound);
    }

    let domain = normalize_name(domain);
    let insert = sqlx::query("INSERT INTO domains (organization_id, domain) VALUES (?, ?)")
        .bind(organization_id)
        .bind(&domain)
        .execute(pool)
        .await;

    match insert {
        Ok(done) => Ok(CreateDomainResult::Created(Domain {
            id: done.last_insert_rowid(),
            organization_id,
            domain,
        })),
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            let unignored = sqlx::query(
                "UPDATE domains SET ignored = 0, organization_id = ? WHERE domain = ? AND ignored = 1",
            )
            .bind(organization_id)
            .bind(&domain)
            .execute(pool)
            .await?;
            let (existing, organization_name) = domain_with_organization(pool, &domain).await?;
            if unignored.rows_affected() > 0 {
                return Ok(CreateDomainResult::Unignored(existing));
            }
            Ok(CreateDomainResult::AlreadyExists {
                domain: existing,
                organization_name,
            })
        }
        Err(err) => Err(err.into()),
    }
}

/// Outcome of [`move_domain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDomain {
    Moved,
    DomainNotFound,
    OrganizationNotFound,
}

/// Move domain `id` to organization `organization_id`. Names are unique across organizations,
/// so a move cannot collide. An ignored domain counts as not found.
pub async fn move_domain(
    pool: &SqlitePool,
    id: i64,
    organization_id: i64,
) -> Result<MoveDomain, StoreError> {
    let organization: Option<i64> = sqlx::query_scalar("SELECT id FROM organizations WHERE id = ?")
        .bind(organization_id)
        .fetch_optional(pool)
        .await?;
    if organization.is_none() {
        return Ok(MoveDomain::OrganizationNotFound);
    }
    let moved = sqlx::query("UPDATE domains SET organization_id = ? WHERE id = ? AND ignored = 0")
        .bind(organization_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(if moved.rows_affected() == 0 {
        MoveDomain::DomainNotFound
    } else {
        MoveDomain::Moved
    })
}

/// Remove domain `id`: delete its reports and DNS result, and ignore it, so
/// ingest drops its future reports. Listing it without `ignored` in the domain
/// mapping, or adding the name again, un-ignores it and re-imports the mailbox.
/// Returns `false` when `id` is unknown or already ignored.
pub async fn ignore_domain(pool: &SqlitePool, id: i64) -> Result<bool, StoreError> {
    let mut tx = begin_write(pool).await?;
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM domains WHERE id = ? AND ignored = 0")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if found.is_none() {
        return Ok(false);
    }
    ignore_domain_in(&mut tx, id).await?;
    tx.commit().await?;
    Ok(true)
}

/// [`ignore_domain`] inside the caller's transaction, for domain `id`.
pub(crate) async fn ignore_domain_in(
    conn: &mut SqliteConnection,
    id: i64,
) -> Result<(), StoreError> {
    // Records, their auth results and the raw XML go with it (ON DELETE CASCADE).
    sqlx::query("DELETE FROM reports WHERE domain_id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM dns_snapshots WHERE domain_id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    // Its alerts go too: un-ignored, it starts from a new baseline.
    for table in ["alert_events", "alert_state"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE domain_id = ?"))
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("UPDATE domains SET ignored = 1, parked = 0 WHERE id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Mark domain `id` as parked (sends no mail of its own) or not. Returns `false`
/// when `id` is unknown or blocked.
pub async fn set_domain_parked(
    pool: &SqlitePool,
    id: i64,
    parked: bool,
) -> Result<bool, StoreError> {
    let done = sqlx::query("UPDATE domains SET parked = ? WHERE id = ? AND ignored = 0")
        .bind(parked)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The domain row named `domain` (normalized first) and its organization's name.
pub async fn domain_with_organization(
    pool: &SqlitePool,
    domain: &str,
) -> Result<(Domain, String), StoreError> {
    let (id, organization_id, name, organization_name): (i64, i64, String, String) =
        sqlx::query_as(
            r#"
        SELECT d.id, d.organization_id, d.domain, org.name
        FROM domains d INNER JOIN organizations org ON org.id = d.organization_id
        WHERE d.domain = ?
        "#,
        )
        .bind(normalize_name(domain))
        .fetch_one(pool)
        .await?;
    Ok((
        Domain {
            id,
            organization_id,
            domain: name,
        },
        organization_name,
    ))
}
