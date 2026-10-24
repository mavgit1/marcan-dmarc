//! The reports go through the real parser and ingest, several parts at
//! once: each statement waits on a round trip to its connection's thread, so
//! one database takes half a minute for a baseline's worth of reports while
//! the CPU idles. Each part is ingested into a database of its own (the same
//! domains, in the same order, so their ids agree), then copied into the
//! demo's table by table, its ids moved past the rows already there. The
//! reports are dealt out in turn, so every part gets big and small ones.

use dmarc_store::{get_or_create_domain, ingest_reports, init};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// A parent column and the table it points into.
type Parent = Option<(&'static str, &'static str)>;

/// Tables a part adds rows to, parents first: `(table, id column, parent)`.
const TABLES: [(&str, &str, Parent); 6] = [
    ("reports", "id", None),
    ("report_xml", "report_id", None),
    ("records", "id", Some(("report_id", "reports"))),
    ("record_dkim_auth", "id", Some(("record_id", "records"))),
    ("record_spf_auth", "id", Some(("record_id", "records"))),
    ("record_overrides", "id", Some(("record_id", "records"))),
];

pub async fn ingest(
    pool: &SqlitePool,
    dir: &Path,
    domains: &[&str],
    xml: Vec<String>,
    parts: usize,
) -> anyhow::Result<()> {
    let work = dir.join(".realistic-demo-parts");
    std::fs::create_dir_all(&work)?;
    let domains: Vec<String> = domains.iter().map(|d| (*d).to_owned()).collect();
    let parts = parts.clamp(1, xml.len().max(1));
    let mut dealt: Vec<Vec<String>> = vec![Vec::new(); parts];
    for (i, x) in xml.into_iter().enumerate() {
        dealt[i % parts].push(x);
    }
    let mut tasks = Vec::new();
    for (k, chunk) in dealt.into_iter().enumerate() {
        let path = work.join(format!("part-{k}.db"));
        let domains = domains.clone();
        tasks.push(tokio::spawn(async move {
            ingest_part(path, &domains, &chunk).await
        }));
    }
    let mut paths = Vec::new();
    for task in tasks {
        paths.push(task.await??);
    }
    for path in &paths {
        merge(pool, path).await?;
    }
    std::fs::remove_dir_all(&work)?;
    Ok(())
}

async fn ingest_part(path: PathBuf, domains: &[String], xml: &[String]) -> anyhow::Result<PathBuf> {
    let pool = init(&path).await?;
    for d in domains {
        get_or_create_domain(&pool, 1, d).await?;
    }
    let mut parsed = Vec::with_capacity(xml.len());
    for x in xml {
        parsed.push(dmarc_parser::parse_report(
            x.as_bytes(),
            Some("report.xml"),
        )?);
    }
    for chunk in parsed.chunks(500) {
        let refs: Vec<_> = chunk.iter().collect();
        ingest_reports(&pool, &refs)
            .await
            .map_err(|e| anyhow::anyhow!("ingest failed at {:?}: {}", e.index, e.error))?;
    }
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(path)
}

/// Copy a part's rows into the demo database, each id (and each parent
/// column) moved past the demo's largest id of that table. The columns are
/// read from the schema, so a column a later migration adds is copied too.
async fn merge(pool: &SqlitePool, part: &Path) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("ATTACH DATABASE ? AS part")
        .bind(part.to_string_lossy().into_owned())
        .execute(&mut *conn)
        .await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    let mut offsets = std::collections::HashMap::new();
    for (table, id, _) in TABLES {
        let max: i64 =
            sqlx::query_scalar(&format!("SELECT COALESCE(MAX({id}), 0) FROM main.{table}"))
                .fetch_one(&mut *conn)
                .await?;
        offsets.insert(table, max);
    }
    // report_xml is keyed by its report: it moves with the reports.
    offsets.insert("report_xml", offsets["reports"]);
    for (table, id, parent) in TABLES {
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
            .bind(table)
            .fetch_all(&mut *conn)
            .await?;
        let values: Vec<String> = columns
            .iter()
            .map(|c| match parent {
                _ if c == id => format!("{c} + {}", offsets[table]),
                Some((column, parent_table)) if c == column => {
                    format!("{c} + {}", offsets[parent_table])
                }
                _ => c.clone(),
            })
            .collect();
        sqlx::query(&format!(
            "INSERT INTO main.{table} ({}) SELECT {} FROM part.{table}",
            columns.join(", "),
            values.join(", ")
        ))
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query("COMMIT").execute(&mut *conn).await?;
    sqlx::query("DETACH DATABASE part")
        .execute(&mut *conn)
        .await?;
    Ok(())
}
