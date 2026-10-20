//! The alert email: every event since the last delivery, grouped by domain,
//! worst first, each line in plain words, with a link to the domain when a
//! base URL is set. Pure: alert items in, text and HTML out.

use super::frame::{finish, Body, Content};
use super::text::{escape, format_time, status_colours, Letterhead, ALERTS_FOOTER};
use crate::view::alerts::AlertItem;
use crate::wording::plural;
use dmarc_assess::{worst_first, Status};

/// One domain's events, worst first, and the worst colour among them.
pub(crate) struct DomainGroup<'a> {
    pub domain_id: i64,
    pub domain: &'a str,
    pub worst: Status,
    pub items: Vec<&'a AlertItem>,
}

/// `items` by domain: domains worst first (then by name), each domain's
/// events worst first (then in the order they were found).
pub(crate) fn group_by_domain(items: &[AlertItem]) -> Vec<DomainGroup<'_>> {
    let mut groups: Vec<DomainGroup<'_>> = Vec::new();
    let mut ordered: Vec<&AlertItem> = items.iter().collect();
    ordered.sort_by_key(|i| (i.row.created_at, i.row.id));
    for item in ordered {
        match groups
            .iter_mut()
            .find(|g| g.domain_id == item.row.domain_id)
        {
            Some(g) => g.items.push(item),
            None => groups.push(DomainGroup {
                domain_id: item.row.domain_id,
                domain: &item.row.domain,
                worst: Status::Neutral,
                items: vec![item],
            }),
        }
    }
    for g in &mut groups {
        worst_first(&mut g.items, |i| i.row.event.severity);
        g.worst = Status::worst_of(g.items.iter().map(|i| i.row.event.severity));
    }
    groups.sort_by(|a, b| a.domain.cmp(b.domain));
    worst_first(&mut groups, |g| g.worst);
    groups
}

/// `<base>/domains/<id>`; `None` without a base URL, and for the test
/// email's made-up domain (id 0), which has no page.
pub(crate) fn domain_link(base_url: Option<&str>, domain_id: i64) -> Option<String> {
    base_url
        .filter(|_| domain_id > 0)
        .map(|base| format!("{base}/domains/{domain_id}"))
}

/// One event as a plain-text line: `[Problem] New issue: … (2026-10-13 07:00 UTC)`.
pub(crate) fn text_line(item: &AlertItem) -> String {
    format!(
        "[{}] {} ({})",
        item.severity_label,
        item.text,
        format_time(item.row.created_at)
    )
}

/// One event as an HTML list item, coloured by its severity.
pub(crate) fn html_line(item: &AlertItem) -> String {
    let (fg, bg) = status_colours(item.row.event.severity);
    format!(
        "<li style=\"margin:4px 0;padding:6px 10px;border-left:3px solid {fg};background:{bg};list-style:none\">\
         <strong style=\"color:{fg}\">{}</strong> {} \
         <span style=\"color:#737373;font-size:12px\">{}</span></li>",
        escape(item.severity_label),
        escape(&item.text),
        escape(&format_time(item.row.created_at)),
    )
}

/// The heading of a domain's block: its name, linked when possible.
pub(crate) fn html_domain_heading(domain: &str, link: Option<&str>) -> String {
    match link {
        Some(href) => format!(
            "<h3 style=\"font-size:15px;margin:16px 0 4px\"><a href=\"{}\" style=\"color:#111827\">{}</a></h3>",
            escape(href),
            escape(domain)
        ),
        None => format!(
            "<h3 style=\"font-size:15px;margin:16px 0 4px\">{}</h3>",
            escape(domain)
        ),
    }
}

/// The alert email's title: `3 changes on example.com` / `5 changes on 2 domains`.
pub fn title(items: &[AlertItem]) -> String {
    let groups = group_by_domain(items);
    let changes = plural(items.len(), "change");
    match groups.as_slice() {
        [one] => format!("{changes} on {}", one.domain),
        _ => format!("{changes} on {}", plural(groups.len(), "domain")),
    }
}

/// `Marcan: 3 changes on example.com` / `Marcan: 5 changes on 2 domains`.
pub fn subject(items: &[AlertItem]) -> String {
    format!("Marcan: {}", title(items))
}

/// The alert email for `items` (everything since the last delivery).
/// `base_url` (no trailing slash) adds links to each domain and the alert list.
pub fn render(items: &[AlertItem], base_url: Option<&str>, letterhead: &Letterhead) -> Content {
    let subject = subject(items);
    let title = title(items);
    let groups = group_by_domain(items);

    let mut text = String::from("What changed since the last alert email:\n");
    let mut html = String::from("<p>What changed since the last alert email:</p>");
    for g in &groups {
        let link = domain_link(base_url, g.domain_id);
        text.push('\n');
        text.push_str(g.domain);
        text.push('\n');
        for item in &g.items {
            text.push_str("  ");
            text.push_str(&text_line(item));
            text.push('\n');
        }
        if let Some(link) = &link {
            text.push_str(&format!("  {link}\n"));
        }
        html.push_str(&html_domain_heading(g.domain, link.as_deref()));
        html.push_str("<ul style=\"padding:0;margin:0\">");
        for item in &g.items {
            html.push_str(&html_line(item));
        }
        html.push_str("</ul>");
    }
    if let Some(base) = base_url {
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
    finish(subject, letterhead, &body, ALERTS_FOOTER)
}

/// A made-up set of events for the test email: one domain, a problem, a
/// warning and a fix.
pub fn sample_items(now: i64) -> Vec<AlertItem> {
    use dmarc_assess::{AlertEvent, AlertKind, AlertedIssue, CheckKind, HealthState, RecordDelta};
    let item = |id: i64, kind: AlertKind, severity: Status| {
        AlertItem::from(dmarc_store::AlertEventRow {
            id,
            domain_id: 0,
            domain: "example.com".into(),
            organization_id: 0,
            organization: "Example".into(),
            created_at: now,
            notified_at: None,
            event: AlertEvent { kind, severity },
        })
    };
    vec![
        item(
            1,
            AlertKind::IssueOpened {
                issue: AlertedIssue::DeliveredFailures,
                mail: None,
                stopped: None,
            },
            Status::Fail,
        ),
        item(
            2,
            AlertKind::HealthChanged {
                from: HealthState::Protected,
                to: HealthState::Warning,
                mail: None,
            },
            Status::Warn,
        ),
        item(
            3,
            AlertKind::DnsRecordChanged {
                check: CheckKind::Spf,
                change: RecordDelta::SpfChanged {
                    added: vec!["include:mail.example.net".into()],
                    removed: vec![],
                    lookups_before: 4,
                    lookups_after: 5,
                },
            },
            Status::Neutral,
        ),
        item(
            4,
            AlertKind::IssueResolved {
                issue: AlertedIssue::ReportsStopped,
                mail: None,
                stopped: None,
            },
            Status::Pass,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::InlineImage;
    use dmarc_assess::{AlertEvent, AlertKind, AlertedIssue, PolicyStep};

    const AT: i64 = 1_791_961_200; // 2026-10-14 07:00 UTC

    fn item(id: i64, domain_id: i64, domain: &str, kind: AlertKind, severity: Status) -> AlertItem {
        AlertItem::from(dmarc_store::AlertEventRow {
            id,
            domain_id,
            domain: domain.into(),
            organization_id: 1,
            organization: "Acme".into(),
            created_at: AT + id * 900,
            notified_at: None,
            event: AlertEvent { kind, severity },
        })
    }

    fn items() -> Vec<AlertItem> {
        vec![
            item(
                1,
                7,
                "b.example",
                AlertKind::ReadinessReached {
                    next: PolicyStep::Reject,
                    evidence: None,
                },
                Status::Pass,
            ),
            item(
                2,
                3,
                "a.example",
                AlertKind::IssueResolved {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: None,
                },
                Status::Pass,
            ),
            item(
                3,
                7,
                "b.example",
                AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: None,
                    stopped: None,
                },
                Status::Fail,
            ),
        ]
    }

    #[test]
    fn subject_counts_changes_and_domains() {
        let all = items();
        assert_eq!(subject(&all), "Marcan: 3 changes on 2 domains");
        assert_eq!(subject(&all[2..]), "Marcan: 1 change on b.example");
    }

    #[test]
    fn grouped_by_domain_worst_first_with_links() {
        let mail = render(
            &items(),
            Some("https://marcan.example.com"),
            &Letterhead::default(),
        );
        assert_eq!(
            mail.text,
            concat!("Marcan, free software

3 changes on 2 domains

What changed since the last alert email:

b.example
  [Problem] New issue: delivered failures. (2026-10-14 07:45 UTC)
  [Good] Ready for the next policy step: p=reject. (2026-10-14 07:15 UTC)
  https://marcan.example.com/domains/7

a.example
  [Good] Resolved: reports stopped. (2026-10-14 07:30 UTC)
  https://marcan.example.com/domains/3

Every change: https://marcan.example.com/alerts

--\x20
Recipients, the digest and alert emails are set under Settings → Alerts.

Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "
")
        );
        assert!(mail
            .html
            .contains("<a href=\"https://marcan.example.com/domains/7\""));
        assert!(mail.html.find("b.example").unwrap() < mail.html.find("a.example").unwrap());
        // No external assets: the one image is Marcan's mark, attached inline.
        assert!(!mail.html.contains("http://") && !mail.html.contains("<img src=\"cid:logo"));
        assert_eq!(mail.images, [InlineImage::marcan_mark()]);
        // The notice ends the HTML footer too, its source URL linked.
        assert!(mail.html.contains(concat!(
            "source code: <a href=\"https://github.com/mavgit1/marcan-dmarc/tree/v",
            env!("CARGO_PKG_VERSION"),
            "\""
        )));
    }

    #[test]
    fn without_a_base_url_there_are_no_links() {
        let quiet = Letterhead {
            notice: false,
            ..Letterhead::default()
        };
        let mail = render(&items(), None, &quiet);
        assert!(!mail.text.contains("http"));
        assert!(!mail.html.contains("href"));
        let message = mail.message("ops@example.com".parse().unwrap());
        assert_eq!(message.subject, "Marcan: 3 changes on 2 domains");
        assert!(message.html.is_some());
    }

    #[test]
    fn names_are_escaped_in_html() {
        let odd = vec![item(
            1,
            1,
            "<b>.example",
            AlertKind::IssueOpened {
                issue: AlertedIssue::DeliveredFailures,
                mail: None,
                stopped: None,
            },
            Status::Fail,
        )];
        let mail = render(&odd, None, &Letterhead::default());
        assert!(mail.html.contains("&lt;b&gt;.example"));
        assert!(!mail.html.contains("<b>.example"));
    }

    #[test]
    fn the_operator_name_and_logo_head_the_email() {
        let branded = Letterhead {
            operator_name: Some("Acme <IT>".into()),
            logo: Some(InlineImage {
                cid: super::super::LOGO_CID,
                media_type: "image/png",
                bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
                size: dmarc_store::PixelSize {
                    width: 300,
                    height: 100,
                },
            }),
            notice: true,
        };
        let mail = render(&items(), None, &branded);
        // Without a license Marcan comes first, then the operator, then the title.
        assert!(
            mail.text.starts_with(
                "Marcan, free software\nManaged by Acme <IT>\n\n3 changes on 2 domains\n\nWhat changed since the last alert email:\n"
            ),
            "{}",
            mail.text
        );
        // The plain text has the words only; the HTML the images by their
        // Content-ID, and the message carries them.
        assert!(!mail.text.contains("cid:"));
        assert!(mail.html.contains(
            "<body style=\"font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;font-size:14px;color:#111827;max-width:640px;margin:0 auto;padding:16px\">\
             <p style=\"margin:0 0 12px;font-size:13px;line-height:20px;color:#737373\"><img src=\"cid:marcan-mark\""
        ));
        assert!(mail
            .html
            .contains("<img src=\"cid:logo\" alt=\"\" width=\"96\" height=\"32\" style=\"width:96px;height:32px;"));
        let message = mail.message(super::super::parse_address("a@example.com").unwrap());
        assert_eq!(
            message.images,
            [InlineImage::marcan_mark(), branded.logo.clone().unwrap()]
        );
        // The operator signs the notice.
        assert!(mail.text.ends_with(
            concat!("\n\nSent by Acme <IT> with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "\n")
        ));

        // With the notice off the operator heads the email in bold and signs
        // alone; a name without a logo is a line of text, no image attached.
        let named = Letterhead {
            logo: None,
            notice: false,
            ..branded
        };
        let mail = render(&items(), None, &named);
        assert!(mail
            .text
            .starts_with("Acme <IT>\n\n3 changes on 2 domains\n\nWhat changed"));
        assert!(mail.html.contains(
            "<p style=\"margin:0 0 4px;font-size:20px;font-weight:700\">Acme &lt;IT&gt;</p>"
        ));
        assert!(!mail.html.contains("<img"));
        assert!(mail.images.is_empty());
        assert!(!mail.text.contains("Marcan,") && !mail.text.contains("free software"));
        assert!(!mail.html.contains("free software"));
        assert!(mail.text.ends_with(
            "--\x20\nRecipients, the digest and alert emails are set under Settings → Alerts.\n\nSent by Acme <IT>.\n"
        ));
        assert!(mail.html.contains("<br><br>Sent by Acme &lt;IT&gt;.</p>"));
    }
}
