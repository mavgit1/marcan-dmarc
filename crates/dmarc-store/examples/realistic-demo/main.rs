//! A demo database that looks like a busy real installation and holds
//! nothing real: for screenshots and videos.
//!
//! ```text
//! cargo run -p dmarc-store --example realistic-demo -- <out-dir> [--baseline <db>] [--seed <n>]
//! cargo run -p dmarc-server -- --data-dir <out-dir> --bind 127.0.0.1:8091
//! ```
//!
//! `--baseline` is a copy of a real database (made with sqlite's `.backup`)
//! the demo takes its shape from: per domain the mail volume, the share of
//! each outcome, the number of sending addresses, since when it reports and
//! its DNS grade; the weekend dip; the receivers' shares (see `profile`).
//! Only those aggregates are read; no name, address, id or record of it
//! reaches the demo. Without it the built-in defaults are used, a baseline
//! reading rounded. `--print-profile` prints the reading instead.
//!
//! What it writes, from the seed (the same seed gives the same database):
//! nine invented organizations with 36 domains shaped like the baseline,
//! four in a clean state (p=reject, grade A), three parked and two ignored,
//! all on `.example` (RFC 2606); 90 days of aggregate reports ending
//! yesterday, through the real parser and ingest, from the generic receivers
//! and invented smaller ones; senders on documentation addresses only (RFC
//! 5737, RFC 3849), named by stored reverse DNS after the generic platforms
//! (Microsoft 365, Google, security gateways, mailing services) or invented
//! hosts; made-up DNS records the server's checks answer from (as for the
//! `demo-data` example) so each domain earns its grade; six weeks of alert
//! events; client report recipients and their sent months; branding
//! "Example IT" with a drawn logo; the mailbox and sending filled in with
//! example values, sending off and the mailbox without a password, so the
//! server sends nothing and connects nowhere.

mod alerts;
mod catalogue;
mod dns;
mod extras;
mod ingest;
mod profile;
mod rng;
mod services;
mod traffic;

use catalogue::{IGNORED, ORGS};
use dmarc_store::{
    create_organization, get_or_create_domain, ignore_domain, init, rename_organization,
    set_demo_dns_records, set_domain_parked, unix_now, OrganizationWrite,
};
use profile::{DnsClass, Profile};
use rng::Rng;
use services::Services;
use sqlx::SqlitePool;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use traffic::{Plan, Role, Traffic, DAY};

const DEFAULT_SEED: u64 = 2026;
/// Domains shaped like the baseline.
const SHAPED: usize = 36;
/// Messages over the window from which a domain counts as large.
const LARGE_DOMAIN: f64 = 300.0;
/// Days a large domain has reported for, at least.
const ESTABLISHED_DAYS: i64 = 50;

#[derive(Debug)]
pub struct Summary {
    pub source: String,
    pub organizations: i64,
    pub domains: i64,
    pub parked: i64,
    pub ignored: i64,
    pub reports: i64,
    pub rows: i64,
    pub messages: i64,
    pub senders: i64,
    pub alert_events: i64,
    pub recipients: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut out, mut baseline, mut seed, mut print) = (None, None, DEFAULT_SEED, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--baseline" => baseline = Some(PathBuf::from(args.next().ok_or_else(usage)?)),
            "--seed" => seed = args.next().ok_or_else(usage)?.parse()?,
            "--print-profile" => print = true,
            _ if !arg.starts_with('-') && out.is_none() => out = Some(PathBuf::from(arg)),
            _ => return Err(usage()),
        }
    }
    if print {
        return match baseline {
            Some(path) => {
                profile::from_baseline(&path).await?.print();
                Ok(())
            }
            None => Err(usage()),
        };
    }
    let out = out.ok_or_else(usage)?;
    let s = generate(&out, baseline.as_deref(), seed).await?;
    println!(
        "{} written from {} (seed {seed}):\n  {} organizations, {} domains ({} parked) and {} ignored\n  \
         {} reports, {} rows, {} messages from {} senders\n  {} alert events, {} client report recipients",
        out.join("marcan.db").display(),
        s.source,
        s.organizations,
        s.domains,
        s.parked,
        s.ignored,
        s.reports,
        s.rows,
        s.messages,
        s.senders,
        s.alert_events,
        s.recipients
    );
    Ok(())
}

fn usage() -> anyhow::Error {
    anyhow::anyhow!(
        "usage: realistic-demo <out-dir> [--baseline <db>] [--seed <n>]\n       \
         realistic-demo --print-profile --baseline <db>"
    )
}

/// Write the demo database into `dir` (which must not hold one yet).
pub async fn generate(dir: &Path, baseline: Option<&Path>, seed: u64) -> anyhow::Result<Summary> {
    let (profile, source) = match baseline {
        Some(path) => (
            profile::from_baseline(path).await?,
            format!("baseline {}", path.display()),
        ),
        None => (profile::defaults(), "built-in defaults".to_owned()),
    };
    let path = dir.join("marcan.db");
    anyhow::ensure!(
        !path.exists(),
        "{} exists; the demo goes into a fresh database only",
        path.display()
    );
    let pool = init(&path).await?;
    let now = unix_now();
    let today = now / DAY * DAY;
    let mut rng = Rng::new(seed);
    let services = Services::new(&mut rng);
    let mut plans = plans(&mut rng, &services, &profile);

    // Organizations: the default one becomes the first.
    let mut org_ids = Vec::new();
    for (i, spec) in ORGS.iter().enumerate() {
        let written = if i == 0 {
            rename_organization(&pool, 1, spec.name).await?
        } else {
            create_organization(&pool, spec.name).await?
        };
        match written {
            OrganizationWrite::Saved(org) => org_ids.push(org.id),
            _ => anyhow::bail!("could not write organization {}", spec.name),
        }
    }
    let mut domain_ids = Vec::new();
    for plan in &plans {
        let row = get_or_create_domain(&pool, org_ids[plan.org], plan.name).await?;
        if plan.role == Role::Parked {
            set_domain_parked(&pool, row.id, true).await?;
        }
        domain_ids.push(row.id);
    }

    // Reports, through the real parser and ingest.
    let mut traffic: Vec<Traffic> = Vec::new();
    for plan in &mut plans {
        traffic.push(traffic::traffic(&mut rng, &services, &profile, plan, today));
    }
    let names: Vec<&str> = plans.iter().map(|p| p.name).collect();
    let xml: Vec<String> = traffic
        .iter_mut()
        .flat_map(|t| std::mem::take(&mut t.xml))
        .collect();
    ingest::ingest(&pool, dir, &names, xml, 16).await?;
    for (plan, id) in plans.iter().zip(&domain_ids) {
        if plan.role == Role::Ignored {
            ignore_domain(&pool, *id).await?;
        }
    }
    extras::arrival_times(&pool, now).await?;
    let used: BTreeSet<String> = traffic.iter().flat_map(|t| t.ips.iter().cloned()).collect();
    extras::identities(&pool, &services, &used, now).await?;
    set_demo_dns_records(&pool, &dns::records(&services, &plans)).await?;

    let events = alerts::history(&mut rng, &services, &plans, &traffic, today, now);
    store_alert_events(&pool, &domain_ids, &events).await?;
    let orgs: Vec<(i64, &catalogue::OrgSpec)> = org_ids.iter().copied().zip(ORGS.iter()).collect();
    let recipients = extras::client_reports(&pool, &orgs, now).await?;
    extras::settings(&pool).await?;

    let count = |sql: &'static str| {
        let pool = pool.clone();
        async move { sqlx::query_scalar::<_, i64>(sql).fetch_one(&pool).await }
    };
    let summary = Summary {
        source,
        organizations: count("SELECT COUNT(*) FROM organizations").await?,
        domains: count("SELECT COUNT(*) FROM domains WHERE ignored = 0").await?,
        parked: count("SELECT COUNT(*) FROM domains WHERE parked = 1").await?,
        ignored: count("SELECT COUNT(*) FROM domains WHERE ignored = 1").await?,
        reports: count("SELECT COUNT(*) FROM reports").await?,
        rows: count("SELECT COUNT(*) FROM records").await?,
        messages: count("SELECT COALESCE(SUM(count), 0) FROM records").await?,
        senders: count("SELECT COUNT(DISTINCT source_ip) FROM records").await?,
        alert_events: count("SELECT COUNT(*) FROM alert_events").await?,
        recipients,
    };
    // Fold the WAL into the file so the database is one file at rest.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(summary)
}

/// Who sends each domain's mail: the baseline's shapes on the client
/// domains (main domains get the largest), then the clean, parked and
/// ignored ones; and a few histories (a tightened policy, reports that
/// stopped).
fn plans(rng: &mut Rng, services: &Services, profile: &Profile) -> Vec<Plan> {
    let mut slots = Vec::new();
    let longest = ORGS.iter().map(|o| o.shaped.len()).max().unwrap_or(0);
    for k in 0..longest {
        for (org, spec) in ORGS.iter().enumerate() {
            if let Some(name) = spec.shaped.get(k) {
                slots.push((org, *name));
            }
        }
    }
    // A baseline that took on most of its domains a few weeks ago would show
    // trends of thousands of percent; the demo is an installation that has
    // been running, so its large domains report for at least seven weeks.
    let mut shapes = profile.spread(SHAPED.min(slots.len()));
    for shape in shapes.iter_mut().filter(|s| s.messages >= LARGE_DOMAIN) {
        shape.first_ago = shape.first_ago.max(ESTABLISHED_DAYS);
    }
    let mut plans: Vec<Plan> = slots
        .into_iter()
        .zip(shapes)
        .map(|((org, name), shape)| {
            traffic::plan(rng, services, profile, name, org, Role::Shaped, shape)
        })
        .collect();
    let mut rank = 0;
    for (org, spec) in ORGS.iter().enumerate() {
        for name in spec.clean {
            let shape = traffic::clean_shape(rank);
            rank += 1;
            plans.push(traffic::plan(
                rng,
                services,
                profile,
                name,
                org,
                Role::Clean,
                shape,
            ));
        }
    }
    for (org, spec) in ORGS.iter().enumerate() {
        for name in spec.parked {
            plans.push(traffic::plan(
                rng,
                services,
                profile,
                name,
                org,
                Role::Parked,
                traffic::parked_shape(),
            ));
        }
    }
    for name in IGNORED {
        plans.push(traffic::plan(
            rng,
            services,
            profile,
            name,
            0,
            Role::Ignored,
            traffic::parked_shape(),
        ));
    }

    // The quarantine domain enforced six weeks ago; the first clean domain
    // went from quarantine to reject recently.
    if let Some(p) = plans
        .iter_mut()
        .find(|p| p.shape.dns == DnsClass::Quarantine && p.shape.first_ago > 45)
    {
        p.policy_before = Some((40, "none"));
    }
    if let Some(p) = plans.iter_mut().find(|p| p.role == Role::Clean) {
        p.policy_before = Some((18, "quarantine"));
    }
    // One domain whose reports stopped two and a half weeks ago.
    if let Some(p) = plans.iter_mut().find(|p| {
        p.role == Role::Shaped
            && p.shape.dns == DnsClass::None
            && p.shape.first_ago >= 60
            && (40.0..2000.0).contains(&p.shape.messages)
    }) {
        p.stopped_ago = Some(17);
    }
    plans
}

async fn store_alert_events(
    pool: &SqlitePool,
    domain_ids: &[i64],
    events: &[alerts::Stored],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for (i, (d, at, event)) in events.iter().enumerate() {
        let severity = serde_json::to_value(event.severity)?;
        sqlx::query(
            "INSERT INTO alert_events (domain_id, created_at, kind, severity, format, params, notified_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(domain_ids[*d])
        .bind(at)
        .bind(event.kind.name())
        .bind(severity.as_str().unwrap_or_default())
        .bind(dmarc_assess::ALERT_FORMAT)
        .bind(serde_json::to_string(&event.kind)?)
        // Alert emails were on, so each went out a few minutes later.
        .bind(at + 120 + (i as i64 * 53) % 480)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    /// RFC 5737 and RFC 3849: the only addresses the demo may use.
    fn documentation(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                let [a, b, c, _] = v4.octets();
                matches!((a, b, c), (192, 0, 2) | (198, 51, 100) | (203, 0, 113))
            }
            IpAddr::V6(v6) => v6.segments()[..2] == [0x2001, 0x0db8],
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn defaults_make_a_full_installation_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let s = generate(dir.path(), None, DEFAULT_SEED).await.unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");

        assert_eq!(s.organizations, 9);
        assert_eq!(s.domains, 43);
        assert_eq!(s.parked, 3);
        assert_eq!(s.ignored, 2);
        assert!((2_000..6_000).contains(&s.reports), "{s:?}");
        assert!((40_000..90_000).contains(&s.messages), "{s:?}");
        assert!((500..1_100).contains(&s.senders), "{s:?}");
        assert!((15..120).contains(&s.alert_events), "{s:?}");
        assert_eq!(s.recipients, 6);

        // Nothing real: invented domains, documentation addresses, known
        // receivers only.
        let pool = init(&dir.path().join("marcan.db")).await.unwrap();
        let domains: Vec<String> = sqlx::query_scalar("SELECT domain FROM domains")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            domains.iter().all(|d| d.ends_with(".example")),
            "{domains:?}"
        );
        let ips: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT source_ip FROM records UNION SELECT ip FROM identity_snapshots",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(
            ips.iter().all(|ip| documentation(ip.parse().unwrap())),
            "{ips:?}"
        );
        let orgs: Vec<String> = sqlx::query_scalar("SELECT DISTINCT org_name FROM reports")
            .fetch_all(&pool)
            .await
            .unwrap();
        let known = traffic::reporters();
        assert!(
            orgs.iter().all(|o| known.iter().any(|r| r.org == o)),
            "{orgs:?}"
        );
        // Every client domain with reports has its DNS.
        let dns = dmarc_store::get_demo_dns_records(&pool)
            .await
            .unwrap()
            .unwrap();
        for d in domains.iter().filter(|d| !IGNORED.contains(&d.as_str())) {
            assert!(dns.contains_key(&format!("_dmarc.{d}")), "{d}");
        }
    }

    #[tokio::test]
    async fn the_same_seed_gives_the_same_shape() {
        let a = plans(
            &mut Rng::new(7),
            &Services::new(&mut Rng::new(7)),
            &profile::defaults(),
        );
        let b = plans(
            &mut Rng::new(7),
            &Services::new(&mut Rng::new(7)),
            &profile::defaults(),
        );
        let key = |p: &[Plan]| {
            p.iter()
                .map(|p| (p.name, p.platform_ips.len(), p.spoof_ips.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&a), key(&b));
    }
}
