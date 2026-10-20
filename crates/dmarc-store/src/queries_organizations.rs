//! Organizations: create, rename, delete, list, and the scope filter's options.

use crate::queries::begin_write;
use crate::StoreError;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;
use ts_rs::TS;

/// A row from the `organizations` table.
#[derive(Debug, Clone, FromRow)]
pub struct Organization {
    pub id: i64,
    pub name: String,
    pub is_default: bool,
}

/// Ensure an organization named `name` exists (`INSERT OR IGNORE`) and return it.
pub async fn get_or_create_organization(
    pool: &SqlitePool,
    name: &str,
) -> Result<Organization, StoreError> {
    sqlx::query("INSERT OR IGNORE INTO organizations (name) VALUES (?)")
        .bind(name)
        .execute(pool)
        .await?;

    let organization = sqlx::query_as::<_, Organization>(
        "SELECT id, name, is_default FROM organizations WHERE name = ?",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(organization)
}

/// The default organization: where domains first seen in reports are created.
/// Seeded by the initial migration, so it always exists.
pub async fn get_default_organization(pool: &SqlitePool) -> Result<Organization, StoreError> {
    let organization = sqlx::query_as::<_, Organization>(
        "SELECT id, name, is_default FROM organizations WHERE is_default = 1",
    )
    .fetch_one(pool)
    .await?;
    Ok(organization)
}

/// An organization with its domain and report counts, as the API lists it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct OrganizationSummary {
    pub id: i64,
    pub name: String,
    /// Domains first seen in reports are created in this organization. Exactly one
    /// organization is the default; it cannot be deleted.
    pub is_default: bool,
    pub domain_count: i64,
    pub report_count: i64,
}

/// `SELECT` for [`OrganizationSummary`] with organizations aliased `org`; callers append the rest.
const ORGANIZATION_SUMMARY_SELECT: &str = r#"
    SELECT
        org.id,
        org.name,
        org.is_default,
        (SELECT COUNT(*) FROM domains d
         WHERE d.organization_id = org.id AND d.ignored = 0) AS domain_count,
        (SELECT COUNT(*)
         FROM reports r
         INNER JOIN domains d ON d.id = r.domain_id
         WHERE d.organization_id = org.id AND d.ignored = 0) AS report_count
    FROM organizations org
"#;

/// Every organization, default first, then by name.
pub async fn list_organization_summaries(
    pool: &SqlitePool,
) -> Result<Vec<OrganizationSummary>, StoreError> {
    let rows = sqlx::query_as::<_, OrganizationSummary>(&format!(
        "{ORGANIZATION_SUMMARY_SELECT} ORDER BY org.is_default DESC, org.name COLLATE NOCASE"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One organization, or `None` when `id` is unknown.
pub async fn get_organization_summary(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<OrganizationSummary>, StoreError> {
    let row = sqlx::query_as::<_, OrganizationSummary>(&format!(
        "{ORGANIZATION_SUMMARY_SELECT} WHERE org.id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Outcome of creating or renaming an organization.
#[derive(Debug, Clone)]
pub enum OrganizationWrite {
    Saved(OrganizationSummary),
    /// Another organization already has this name, ignoring case (names are
    /// unique, and the domain mapping matches them case-insensitively).
    NameTaken,
    NotFound,
}

/// Create an organization named `name` (already trimmed). A taken name is a conflict,
/// not a silent get-or-create.
pub async fn create_organization(
    pool: &SqlitePool,
    name: &str,
) -> Result<OrganizationWrite, StoreError> {
    if name_taken(pool, name, None).await? {
        return Ok(OrganizationWrite::NameTaken);
    }
    let insert = sqlx::query("INSERT INTO organizations (name) VALUES (?)")
        .bind(name)
        .execute(pool)
        .await;
    match insert {
        Ok(done) => organization_write(pool, done.last_insert_rowid()).await,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            Ok(OrganizationWrite::NameTaken)
        }
        Err(err) => Err(err.into()),
    }
}

/// Rename organization `id` to `name` (already trimmed).
pub async fn rename_organization(
    pool: &SqlitePool,
    id: i64,
    name: &str,
) -> Result<OrganizationWrite, StoreError> {
    if name_taken(pool, name, Some(id)).await? {
        return Ok(OrganizationWrite::NameTaken);
    }
    let update = sqlx::query("UPDATE organizations SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await;
    match update {
        Ok(done) if done.rows_affected() == 0 => Ok(OrganizationWrite::NotFound),
        Ok(_) => organization_write(pool, id).await,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            Ok(OrganizationWrite::NameTaken)
        }
        Err(err) => Err(err.into()),
    }
}

/// Whether an organization other than `except` is named `name`, ignoring case
/// (Unicode, unlike SQLite's `NOCASE`).
async fn name_taken(
    pool: &SqlitePool,
    name: &str,
    except: Option<i64>,
) -> Result<bool, StoreError> {
    let names: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM organizations")
        .fetch_all(pool)
        .await?;
    let lower = name.to_lowercase();
    Ok(names
        .iter()
        .any(|(id, n)| Some(*id) != except && n.to_lowercase() == lower))
}

async fn organization_write(pool: &SqlitePool, id: i64) -> Result<OrganizationWrite, StoreError> {
    Ok(get_organization_summary(pool, id)
        .await?
        .map_or(OrganizationWrite::NotFound, OrganizationWrite::Saved))
}

/// Outcome of [`delete_organization`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOrganization {
    Deleted,
    NotFound,
    /// The default organization cannot be deleted.
    IsDefault,
}

/// Delete a non-default organization. Its domains (and their reports) move to the
/// default organization, so deleting an organization never deletes report data.
pub async fn delete_organization(
    pool: &SqlitePool,
    id: i64,
) -> Result<DeleteOrganization, StoreError> {
    let mut tx = begin_write(pool).await?;
    let is_default: Option<bool> =
        sqlx::query_scalar("SELECT is_default FROM organizations WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    match is_default {
        None => return Ok(DeleteOrganization::NotFound),
        Some(true) => return Ok(DeleteOrganization::IsDefault),
        Some(false) => {}
    }
    sqlx::query(
        "UPDATE domains SET organization_id = (SELECT id FROM organizations WHERE is_default = 1) \
         WHERE organization_id = ?",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM organizations WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(DeleteOrganization::Deleted)
}

/// An organization as the scope filter lists it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct ScopeOrganization {
    pub id: i64,
    pub name: String,
}

/// A monitored domain as the scope filter lists it (and the setup's
/// Domains step, which follows an import with it).
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct ScopeDomain {
    pub id: i64,
    pub domain: String,
    pub organization_id: i64,
    /// A report is stored for it (a domain without one is ignored without
    /// losing anything).
    pub has_reports: bool,
}

/// Every organization (default first, then by name) and every monitored
/// domain (by name), with ids and names only: no report aggregates (whether
/// a domain has a report is one indexed lookup).
pub async fn scope_options(
    pool: &SqlitePool,
) -> Result<(Vec<ScopeOrganization>, Vec<ScopeDomain>), StoreError> {
    let organizations = sqlx::query_as::<_, ScopeOrganization>(
        "SELECT id, name FROM organizations ORDER BY is_default DESC, name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    let domains = sqlx::query_as::<_, ScopeDomain>(
        "SELECT d.id, d.domain, d.organization_id, \
                EXISTS (SELECT 1 FROM reports r WHERE r.domain_id = d.id) AS has_reports \
         FROM domains d WHERE d.ignored = 0 \
         ORDER BY d.domain COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok((organizations, domains))
}
