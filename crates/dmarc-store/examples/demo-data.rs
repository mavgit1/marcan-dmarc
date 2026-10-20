//! Fill a fresh database with made-up but realistic DMARC reports, for the
//! docs' screenshots and for trying Marcan without a mailbox.
//!
//! ```text
//! cargo run -p dmarc-store --example demo-data -- .demo
//! cargo run -p dmarc-server -- --data-dir .demo --bind 127.0.0.1:8090
//! ```
//!
//! Domains are the names reserved for examples (RFC 2606). Their DNS is made
//! up too and stored with the data ([`demo_dns_records`]): while it is, the
//! server answers every TXT lookup from it instead of the network, and its
//! first DNS pass grades the domains from those records with the real
//! checker. So the records agree with the reports: `p=reject` with strict
//! alignment for example.com and example.net, `p=none` for example.org (all
//! its own mail passes, so it is ready to enforce), and nothing at all for
//! the parked legacy.example.net, which sends no mail and so has no reports.
//! Legitimate senders are public addresses of large mail providers (their
//! reverse DNS, still looked up live, names them); spoofers use documentation
//! addresses (RFC 5737). The 30 days of reports end yesterday. Reports go
//! through the real parser and ingest, so the database is what a mailbox
//! import would produce.

use dmarc_store::{
    create_organization, get_or_create_domain, ingest_report, init, move_domain,
    rename_organization, save_settings, set_demo_dns_records, set_domain_parked,
    OnboardingSettings, OnboardingState, OrganizationWrite, ReportingMode, ReportingSettings,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

const DAYS: i64 = 30;
const DAY_SECS: i64 = 86_400;

/// Who sends the reports, and the share of each domain's mail they see.
const REPORTERS: [(&str, &str, f64); 3] = [
    ("google.com", "noreply-dmarc-support@google.com", 0.55),
    ("Enterprise Outlook", "dmarcreport@microsoft.com", 0.30),
    ("Yahoo", "dmarchelp@yahooinc.com", 0.15),
];

/// One kind of mail for a domain: who sends it and what receivers saw.
struct Stream {
    ip: &'static str,
    /// Messages on an average weekday, across all reporters.
    daily: f64,
    /// Share of days this stream appears at all (spoofing comes and goes).
    presence: f64,
    envelope_from: &'static str,
    spf: (&'static str, &'static str),
    /// `(domain, selector, result)` signatures.
    dkim: &'static [(&'static str, &'static str, &'static str)],
    /// What DMARC said, as the reporter evaluated it: `(dkim, spf)`.
    evaluated: (&'static str, &'static str),
    disposition: &'static str,
    /// A policy override the reporter gives: `(type, comment)`.
    reason: Option<(&'static str, &'static str)>,
}

struct DemoDomain {
    name: &'static str,
    organization: &'static str,
    parked: bool,
    /// The published DMARC policy, as the reports state it: `p=` and
    /// whether alignment is strict.
    policy: &'static str,
    strict: bool,
    streams: Vec<Stream>,
}

/// A 2048-bit RSA public key (SPKI, Base64) for the demo's DKIM selectors.
const DKIM_KEY: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA4Qe0CpUby/aK3HZKigFMjyMEaAyfPtYhzV+RxsvT8g9CGY0YaovG0Tz5xutPzVrIbvySrncyrV0MOTmJyf12ZLtKW9RsKmw+uV5xq476f6mLItAw0I/++qbGEwjCTVVXvX8ROJV35pO+afp6q2S1v/Tze1eOMlD8qkYt64Q676ljtF3TPqRbop87YAUoh8JnJISq6bccdZ6Dj9wnv9vEAs1OvYB7xOf8tdMdrxyhd5H3cO7iSOgopiJZ4sW+xZPBYcjOf8aLxLeaxplxoNqBam9fkvF7f2lSG4RAi60Bc+R/VIo90ooMDGcxiIwcwT0VmnpWBQhf+HqOnO0PPcnJ6QIDAQAB";

/// The demo's DNS: the TXT records of the demo domains and of the providers
/// their SPF records include, by name. Every other TXT name does not exist.
fn demo_dns_records() -> BTreeMap<String, Vec<String>> {
    let dkim = format!("v=DKIM1; k=rsa; p={DKIM_KEY}");
    let records: Vec<(&str, String)> = vec![
        // example.com: enforced, signed by every sender, TLS reports on.
        (
            "_dmarc.example.com",
            "v=DMARC1; p=reject; sp=reject; adkim=s; aspf=s; rua=mailto:dmarc@example.com".into(),
        ),
        ("example.com", "v=spf1 include:_spf.google.com -all".into()),
        ("google._domainkey.example.com", dkim.clone()),
        ("ses2024._domainkey.example.com", dkim.clone()),
        ("s1._domainkey.example.com", dkim.clone()),
        (
            "_smtp._tls.example.com",
            "v=TLSRPTv1; rua=mailto:tls-reports@example.com".into(),
        ),
        // The report address accepts every domain's reports.
        ("*._report._dmarc.example.com", "v=DMARC1".into()),
        // example.net: enforced; Mandrill never signs for it.
        (
            "_dmarc.example.net",
            "v=DMARC1; p=reject; sp=reject; adkim=s; aspf=s; rua=mailto:dmarc@example.com".into(),
        ),
        (
            "example.net",
            "v=spf1 include:spf.protection.outlook.com -all".into(),
        ),
        ("selector1._domainkey.example.net", dkim),
        // example.org: monitoring only, and ready to enforce.
        (
            "_dmarc.example.org",
            "v=DMARC1; p=none; rua=mailto:dmarc@example.com".into(),
        ),
        ("example.org", "v=spf1 include:_spf.google.com -all".into()),
        // The providers' SPF, cut down to the ranges the demo's senders use.
        (
            "_spf.google.com",
            "v=spf1 ip4:209.85.128.0/17 ip4:74.125.0.0/16 ~all".into(),
        ),
        (
            "spf.protection.outlook.com",
            "v=spf1 ip4:40.92.0.0/15 ip4:40.107.0.0/16 ip4:52.100.0.0/14 -all".into(),
        ),
    ];
    let mut table = BTreeMap::new();
    for (name, record) in records {
        table
            .entry(name.to_owned())
            .or_insert_with(Vec::new)
            .push(record);
    }
    table
}

fn domains() -> Vec<DemoDomain> {
    vec![
        DemoDomain {
            name: "example.com",
            organization: "Example Corp",
            parked: false,
            policy: "reject",
            strict: true,
            streams: vec![
                // Staff mail through Google Workspace: SPF and DKIM aligned.
                Stream {
                    ip: "209.85.220.41",
                    daily: 420.0,
                    presence: 1.0,
                    envelope_from: "example.com",
                    spf: ("example.com", "pass"),
                    dkim: &[("example.com", "google", "pass")],
                    evaluated: ("pass", "pass"),
                    disposition: "none",
                    reason: None,
                },
                // App notifications through Amazon SES: SPF is SES's own domain,
                // DKIM is the domain's, so DMARC passes on DKIM.
                Stream {
                    ip: "54.240.8.1",
                    daily: 260.0,
                    presence: 1.0,
                    envelope_from: "amazonses.com",
                    spf: ("amazonses.com", "pass"),
                    dkim: &[
                        ("example.com", "ses2024", "pass"),
                        ("amazonses.com", "224i4yxa5dv7c2xz3womw6peuasteono", "pass"),
                    ],
                    evaluated: ("pass", "fail"),
                    disposition: "none",
                    reason: None,
                },
                // The newsletter through SendGrid, signed for the domain.
                Stream {
                    ip: "167.89.115.54",
                    daily: 130.0,
                    presence: 0.7,
                    envelope_from: "em4821.example.com",
                    spf: ("em4821.example.com", "pass"),
                    dkim: &[("example.com", "s1", "pass")],
                    evaluated: ("pass", "fail"),
                    disposition: "none",
                    reason: None,
                },
                // A mailing list that rewrites the body: DKIM breaks, the
                // receiver trusts the list.
                Stream {
                    ip: "209.85.160.41",
                    daily: 9.0,
                    presence: 0.8,
                    envelope_from: "googlegroups.com",
                    spf: ("googlegroups.com", "pass"),
                    dkim: &[("example.com", "google", "fail")],
                    evaluated: ("fail", "fail"),
                    disposition: "none",
                    reason: Some(("mailing_list", "")),
                },
                Stream {
                    ip: "203.0.113.77",
                    daily: 18.0,
                    presence: 0.5,
                    envelope_from: "example.com",
                    spf: ("example.com", "fail"),
                    dkim: &[],
                    evaluated: ("fail", "fail"),
                    disposition: "reject",
                    reason: None,
                },
            ],
        },
        DemoDomain {
            name: "example.net",
            organization: "Example Corp",
            parked: false,
            policy: "reject",
            strict: true,
            streams: vec![
                // Microsoft 365.
                Stream {
                    ip: "40.107.22.50",
                    daily: 310.0,
                    presence: 1.0,
                    envelope_from: "example.net",
                    spf: ("example.net", "pass"),
                    dkim: &[("example.net", "selector1", "pass")],
                    evaluated: ("pass", "pass"),
                    disposition: "none",
                    reason: None,
                },
                // Transactional mail through Mandrill, never set up to sign for
                // the domain: everything is rejected.
                Stream {
                    ip: "198.2.128.1",
                    daily: 55.0,
                    presence: 1.0,
                    envelope_from: "mandrillapp.com",
                    spf: ("mandrillapp.com", "pass"),
                    dkim: &[("mandrillapp.com", "mandrill", "pass")],
                    evaluated: ("fail", "fail"),
                    disposition: "reject",
                    reason: None,
                },
                Stream {
                    ip: "198.51.100.23",
                    daily: 40.0,
                    presence: 0.6,
                    envelope_from: "example.net",
                    spf: ("example.net", "fail"),
                    dkim: &[],
                    evaluated: ("fail", "fail"),
                    disposition: "reject",
                    reason: None,
                },
            ],
        },
        DemoDomain {
            name: "example.org",
            organization: "Example Labs",
            parked: false,
            policy: "none",
            strict: false,
            streams: vec![
                // Google Workspace without DKIM set up: passes on SPF alone.
                Stream {
                    ip: "74.125.82.41",
                    daily: 160.0,
                    presence: 1.0,
                    envelope_from: "example.org",
                    spf: ("example.org", "pass"),
                    dkim: &[],
                    evaluated: ("fail", "pass"),
                    disposition: "none",
                    reason: None,
                },
                // Forwarded by a university mailbox: SPF breaks, the receiver
                // trusted the forwarder.
                Stream {
                    ip: "209.85.218.41",
                    daily: 5.0,
                    presence: 0.7,
                    envelope_from: "example.org",
                    spf: ("example.org", "fail"),
                    dkim: &[],
                    evaluated: ("fail", "fail"),
                    disposition: "none",
                    reason: Some(("forwarded", "")),
                },
                // Spoofing, delivered: the policy asks for nothing yet.
                Stream {
                    ip: "198.51.100.140",
                    daily: 2.0,
                    presence: 0.4,
                    envelope_from: "example.org",
                    spf: ("example.org", "softfail"),
                    dkim: &[],
                    evaluated: ("fail", "fail"),
                    disposition: "none",
                    reason: None,
                },
            ],
        },
        // Parked: it sends no mail, publishes nothing yet, and so gets no
        // reports.
        DemoDomain {
            name: "legacy.example.net",
            organization: "Example Corp",
            parked: true,
            policy: "none",
            strict: false,
            streams: Vec::new(),
        },
    ]
}

/// A small deterministic generator, so every run produces the same data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn record_xml(stream: &Stream, domain: &str, count: i64) -> String {
    let dkim: String = stream
        .dkim
        .iter()
        .map(|(d, selector, result)| {
            format!(
                "      <dkim><domain>{d}</domain><selector>{selector}</selector><result>{result}</result></dkim>\n"
            )
        })
        .collect();
    let reason = stream
        .reason
        .map(|(kind, comment)| {
            format!("        <reason><type>{kind}</type><comment>{comment}</comment></reason>\n")
        })
        .unwrap_or_default();
    format!(
        r#"  <record>
    <row>
      <source_ip>{ip}</source_ip>
      <count>{count}</count>
      <policy_evaluated>
        <disposition>{disposition}</disposition>
        <dkim>{eval_dkim}</dkim>
        <spf>{eval_spf}</spf>
{reason}      </policy_evaluated>
    </row>
    <identifiers>
      <envelope_from>{envelope}</envelope_from>
      <header_from>{domain}</header_from>
    </identifiers>
    <auth_results>
{dkim}      <spf><domain>{spf_domain}</domain><scope>mfrom</scope><result>{spf_result}</result></spf>
    </auth_results>
  </record>
"#,
        ip = stream.ip,
        disposition = stream.disposition,
        eval_dkim = stream.evaluated.0,
        eval_spf = stream.evaluated.1,
        envelope = stream.envelope_from,
        spf_domain = stream.spf.0,
        spf_result = stream.spf.1,
    )
}

fn report_xml(
    reporter: (&str, &str),
    report_id: &str,
    domain: &DemoDomain,
    begin: i64,
    records: &str,
) -> String {
    let (org, email) = reporter;
    let name = domain.name;
    let p = domain.policy;
    let alignment = if domain.strict {
        "    <adkim>s</adkim>\n    <aspf>s</aspf>\n"
    } else {
        "    <adkim>r</adkim>\n    <aspf>r</aspf>\n"
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feedback>
  <report_metadata>
    <org_name>{org}</org_name>
    <email>{email}</email>
    <report_id>{report_id}</report_id>
    <date_range><begin>{begin}</begin><end>{end}</end></date_range>
  </report_metadata>
  <policy_published>
    <domain>{name}</domain>
{alignment}    <p>{p}</p>
    <sp>{p}</sp>
    <pct>100</pct>
  </policy_published>
{records}</feedback>
"#,
        end = begin + DAY_SECS - 1,
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".demo".into()));
    let path = dir.join("marcan.db");
    anyhow::ensure!(
        !path.exists(),
        "{} exists; demo data goes into a fresh database only",
        path.display()
    );
    let pool = init(&path).await?;

    let today = dmarc_store::unix_now() / DAY_SECS * DAY_SECS;
    let mut rng = Rng(0x00ab_cdef_2026);
    let mut reports = 0;
    for (d, domain) in domains().iter().enumerate() {
        for day in 1..=DAYS {
            let begin = today - day * DAY_SECS;
            // Less mail at weekends (1970-01-01 was a Thursday).
            let weekday = (begin / DAY_SECS + 3) % 7;
            let volume = if weekday >= 5 { 0.35 } else { 1.0 };
            for (r, (org, email, share)) in REPORTERS.iter().enumerate() {
                let mut records = String::new();
                for stream in &domain.streams {
                    if rng.next() > stream.presence {
                        continue;
                    }
                    let jitter = 0.75 + rng.next() * 0.5;
                    let count = (stream.daily * share * volume * jitter).round() as i64;
                    if count > 0 {
                        records.push_str(&record_xml(stream, domain.name, count));
                    }
                }
                if records.is_empty() {
                    continue;
                }
                // Unique per reporter, as ingest requires.
                let report_id = format!("{}{r}{d}", begin / DAY_SECS);
                let xml = report_xml((org, email), &report_id, domain, begin, &records);
                let report = dmarc_parser::parse_report(xml.as_bytes(), Some("report.xml"))?;
                ingest_report(&pool, &report).await?;
                reports += 1;
            }
        }
    }

    // Organizations: the default one becomes the first company.
    if matches!(
        rename_organization(&pool, 1, "Example Corp").await?,
        OrganizationWrite::NotFound
    ) {
        anyhow::bail!("default organization missing");
    }
    let labs = match create_organization(&pool, "Example Labs").await? {
        OrganizationWrite::Saved(org) => org.id,
        _ => anyhow::bail!("could not create Example Labs"),
    };
    for domain in domains() {
        let row = get_or_create_domain(&pool, 1, domain.name).await?;
        if domain.organization == "Example Labs" {
            move_domain(&pool, row.id, labs).await?;
        }
        if domain.parked {
            set_domain_parked(&pool, row.id, true).await?;
        }
    }

    save_settings(
        &pool,
        &ReportingSettings {
            report_address: Some("dmarc@example.com".to_owned()),
            reporting_mode: Some(ReportingMode::Direct),
        },
    )
    .await?;
    save_settings(
        &pool,
        &OnboardingSettings {
            state: Some(OnboardingState::Completed),
        },
    )
    .await?;
    set_demo_dns_records(&pool, &demo_dns_records()).await?;

    println!("{reports} reports written to {}", path.display());
    Ok(())
}
