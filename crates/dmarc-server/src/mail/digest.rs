//! The digest email (daily or weekly): what changed since the last digest,
//! grouped like the alert email, and every domain's open issues now. Sent
//! even when nothing changed: "nothing changed" is news too. Pure: view
//! types in, text and HTML out.

use super::alerts::{domain_link, group_by_domain, html_domain_heading, html_line, text_line};
use super::frame::{finish, Body, Content};
use super::text::{escape, format_date, format_time, Letterhead, ALERTS_FOOTER};
use crate::view::alerts::AlertItem;
use crate::view::domain::{needing_attention, DomainSummary};
use crate::wording::issues::issue_name;
use crate::wording::plural;
use dmarc_store::DigestFrequency;

/// What one digest covers.
pub struct DigestInput<'a> {
    pub frequency: DigestFrequency,
    /// Start of the period (the last digest, or one period back).
    pub since: i64,
    pub now: i64,
    /// Every monitored domain, judged now.
    pub domains: &'a [DomainSummary],
    /// The events found since `since`.
    pub events: &'a [AlertItem],
    /// No trailing slash; `None`: no links.
    pub base_url: Option<&'a str>,
    pub letterhead: &'a Letterhead,
}

fn period_name(frequency: DigestFrequency) -> &'static str {
    match frequency {
        DigestFrequency::Weekly => "weekly",
        DigestFrequency::Daily | DigestFrequency::Off => "daily",
    }
}

/// `Marcan daily digest: 3 changes, 2 domains need attention`.
pub fn subject(input: &DigestInput<'_>) -> String {
    let changes = if input.events.is_empty() {
        "nothing changed".to_owned()
    } else {
        plural(input.events.len(), "change")
    };
    let attention = match needing_attention(input.domains).len() {
        0 => "no domain needs attention".to_owned(),
        1 => "1 domain needs attention".to_owned(),
        n => format!("{n} domains need attention"),
    };
    format!(
        "Marcan {} digest: {changes}, {attention}",
        period_name(input.frequency)
    )
}

/// The digest's title: `Daily digest for 2026-10-14`.
pub fn title(input: &DigestInput<'_>) -> String {
    let period = match input.frequency {
        DigestFrequency::Weekly => "Weekly",
        DigestFrequency::Daily | DigestFrequency::Off => "Daily",
    };
    format!("{period} digest for {}", format_date(input.now))
}

pub fn render(input: &DigestInput<'_>) -> Content {
    let subject = subject(input);
    let title = title(input);
    let heading = format!(
        "What changed from {} to {}, and the open issues now.",
        format_time(input.since),
        format_time(input.now)
    );
    let mut text = format!("{heading}\n\nWhat changed\n");
    let mut html = format!(
        "<p>{}</p><h2 style=\"font-size:16px;margin:16px 0 4px\">What changed</h2>",
        escape(&heading)
    );

    if input.events.is_empty() {
        text.push_str("  Nothing changed.\n");
        html.push_str("<p>Nothing changed.</p>");
    }
    for g in group_by_domain(input.events) {
        let link = domain_link(input.base_url, g.domain_id);
        text.push_str(&format!("\n  {}\n", g.domain));
        for item in &g.items {
            text.push_str(&format!("    {}\n", text_line(item, input.base_url)));
        }
        html.push_str(&html_domain_heading(g.domain, link.as_deref()));
        html.push_str("<ul style=\"padding:0;margin:0\">");
        for item in &g.items {
            html.push_str(&html_line(item, input.base_url));
        }
        html.push_str("</ul>");
    }

    text.push_str("\nOpen issues\n");
    html.push_str("<h2 style=\"font-size:16px;margin:16px 0 4px\">Open issues</h2>");
    let open = needing_attention(input.domains);
    if open.is_empty() {
        text.push_str("  No domain needs attention.\n");
        html.push_str("<p>No domain needs attention.</p>");
    } else {
        html.push_str("<ul style=\"padding-left:18px;margin:0\">");
    }
    for d in &open {
        let issues: Vec<&str> = d
            .assessment
            .issues
            .iter()
            .map(|i| issue_name(i.kind))
            .collect();
        let link = domain_link(input.base_url, Some(d.row.id));
        text.push_str(&format!("  {}: {}\n", d.row.domain, issues.join("; ")));
        if let Some(link) = &link {
            text.push_str(&format!("    {link}\n"));
        }
        let name = match &link {
            Some(href) => format!("<a href=\"{}\">{}</a>", escape(href), escape(&d.row.domain)),
            None => escape(&d.row.domain),
        };
        html.push_str(&format!(
            "<li style=\"margin:4px 0\"><strong>{name}</strong>: {}</li>",
            escape(&issues.join("; "))
        ));
    }
    if !open.is_empty() {
        html.push_str("</ul>");
    }

    if let Some(base) = input.base_url {
        text.push_str(&format!("\nEvery change: {base}/alerts\n"));
        html.push_str(&format!(
            "<p style=\"margin-top:16px\"><a href=\"{0}/alerts\">Every change</a></p>",
            escape(base)
        ));
    }
    let body = Body {
        title: &title,
        text: &text,
        html: &html,
    };
    finish(subject, input.letterhead, &body, ALERTS_FOOTER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::domain::domain_summaries;
    use dmarc_assess::{AlertEvent, AlertKind, AlertedIssue, Status};
    use dmarc_store::{
        DomainOrganization, DomainSummaryRow, MailCounts, PublishedPolicy, StoredDnsGrade,
    };

    const NOW: i64 = 1_791_961_200; // 2026-10-14 07:00 UTC
    const DAY: i64 = 86_400;

    fn row(id: i64, domain: &str, policy: &str) -> DomainSummaryRow {
        DomainSummaryRow {
            id,
            organization: DomainOrganization {
                id: 1,
                name: "Acme".into(),
            },
            domain: domain.into(),
            report_count: 3,
            last_report_at: Some(NOW - DAY),
            policy: Some(PublishedPolicy {
                p: Some(policy.into()),
                ..PublishedPolicy::default()
            }),
            policy_source: dmarc_store::PolicySource::Dns,
            reporter_count: 1,
            mail: MailCounts {
                total_messages: 100,
                compliant_messages: 100,
                delivered_failures: 0,
                forwarded_failures: 0,
            },
            no_aligned_dkim_messages: 0,
            parked: false,
            dns: Some(StoredDnsGrade {
                grade: "A".into(),
                score: 95,
                critical_count: 0,
                checked_at: NOW,
                checker_version: i64::from(dmarc_dns::CHECKER_VERSION),
            }),
        }
    }

    fn input<'a>(domains: &'a [DomainSummary], events: &'a [AlertItem]) -> DigestInput<'a> {
        DigestInput {
            frequency: DigestFrequency::Daily,
            since: NOW - DAY,
            now: NOW,
            domains,
            events,
            base_url: None,
            letterhead: &NOTICE,
        }
    }

    static NOTICE: Letterhead = Letterhead {
        operator_name: None,
        logo: None,
        notice: true,
    };

    #[test]
    fn nothing_changed_is_a_digest_too() {
        let domains = domain_summaries(
            vec![row(1, "fine.example", "reject")],
            crate::view::domain::Judging::defaults_at(NOW),
        );
        let mail = render(&input(&domains, &[]));
        assert_eq!(
            mail.subject,
            "Marcan daily digest: nothing changed, no domain needs attention"
        );
        assert_eq!(
            mail.text,
            concat!("Marcan, free software

Daily digest for 2026-10-14

What changed from 2026-10-13 07:00 UTC to 2026-10-14 07:00 UTC, and the open issues now.

What changed
  Nothing changed.

Open issues
  No domain needs attention.

--\x20
Recipients, the digest and alert emails are set under Settings → Alerts.

Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "
")
        );
        assert!(mail.html.contains("<p>Nothing changed.</p>"));
    }

    #[test]
    fn changes_and_open_issues_with_links() {
        let domains = domain_summaries(
            vec![
                row(1, "fine.example", "reject"),
                row(2, "open.example", "none"),
            ],
            crate::view::domain::Judging::defaults_at(NOW),
        );
        let events = [AlertItem::from(dmarc_store::AlertEventRow {
            id: 9,
            domain_id: Some(2),
            domain: Some("open.example".into()),
            organization_id: Some(1),
            organization: Some("Acme".into()),
            created_at: NOW - 3600,
            notified_at: Some(NOW - 3000),
            event: AlertEvent {
                kind: AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: None,
                    stopped: None,
                },
                severity: Status::Fail,
            },
        })];
        let digest = DigestInput {
            frequency: DigestFrequency::Weekly,
            base_url: Some("https://marcan.example.com"),
            since: NOW - 7 * DAY,
            ..input(&domains, &events)
        };
        let mail = render(&digest);
        assert_eq!(
            mail.subject,
            "Marcan weekly digest: 1 change, 1 domain needs attention"
        );
        assert_eq!(
            mail.text,
            concat!("Marcan, free software

Weekly digest for 2026-10-14

What changed from 2026-10-07 07:00 UTC to 2026-10-14 07:00 UTC, and the open issues now.

What changed

  open.example
    [Problem] New issue: delivered failures. (2026-10-14 06:00 UTC)

Open issues
  open.example: not enforcing (p=none)
    https://marcan.example.com/domains/2

Every change: https://marcan.example.com/alerts

--\x20
Recipients, the digest and alert emails are set under Settings → Alerts.

Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "
")
        );
        assert!(mail.html.contains("https://marcan.example.com/domains/2"));
    }
}
