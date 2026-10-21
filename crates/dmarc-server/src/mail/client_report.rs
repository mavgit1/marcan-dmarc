//! The client report: one organization's month in a short email, for
//! someone who never opens Marcan. The figures carry it:
//!
//! 1. A verdict for the organization: all domains protected, or how many
//!    need attention ([`report_verdict`]).
//! 2. The calendar month's mail with the change against the month before
//!    ([`Statistics`] over the organization and the month): messages,
//!    passed, blocked by policy, forwarded, failed but delivered.
//! 3. One line per domain over the 30 days before sending
//!    ([`report_place`]): its policy, the step it can take now when there
//!    is one ("ready for p=quarantine"), its messages, the share that
//!    passed DMARC, and the failures delivered, blocked and forwarded when
//!    there are any; then the open issues no figure shows, as the badges of
//!    the domain list read them ("DNS grade D").
//! 4. The month's changes, per domain, and the parked domains on one line.
//!
//! Only what Marcan is sure of goes in: no sender names or IPs, no
//! reporters; a DNS grade from an older checker is left out. What goes
//! where, and every colour, is `dmarc_assess`'s ([`report_place`],
//! [`report_verdict`], [`parked_line`]); the words are `wording`'s; this
//! lays them out. Pure: view types in, text and HTML out.

use super::frame::{finish, Body, Content};
use super::text::{escape, format_month, status_colours, Letterhead, CLIENT_REPORT_FOOTER};
use crate::view::alerts::AlertItem;
use crate::view::domain::{needing_attention, DomainSummary};
use crate::view::stats::Statistics;
use crate::wording::client_reports::{line_figures, note_words, policy_words, step_words, Figure};
use crate::wording::{count_noun, format_count, format_pct, join_and, one_or_many, plural};
use chrono::{DateTime, Datelike, Months, NaiveDate, TimeZone, Utc};
use dmarc_assess::{
    parked_line, report_place, report_verdict, worst_first, ParkedLine, PolicyReadiness,
    ReportDomainFacts, ReportLine, ReportPlace, ReportVerdict, Status, Trend, TrendDirection,
    READINESS_WINDOW_DAYS,
};

/// How many changes a domain's list words; the rest are counted.
pub const CHANGES_SHOWN: usize = 3;

/// The calendar month a report covers: `[start, end)`, unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportMonth {
    pub start: i64,
    pub end: i64,
}

impl ReportMonth {
    /// The calendar month (UTC) before the one `now` is in.
    pub fn before(now: i64) -> Self {
        let today = DateTime::<Utc>::from_timestamp(now, 0)
            .unwrap_or_default()
            .date_naive();
        let this = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
        let previous = this.checked_sub_months(Months::new(1)).unwrap_or(this);
        let midnight = |d: NaiveDate| {
            Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default())
                .timestamp()
        };
        Self {
            start: midnight(previous),
            end: midnight(this),
        }
    }

    /// The calendar month before this one.
    pub fn previous(&self) -> Self {
        Self::before(self.start)
    }

    /// `September 2026`.
    pub fn name(&self) -> String {
        format_month(self.start)
    }

    /// `September`.
    fn month_name(&self) -> String {
        DateTime::<Utc>::from_timestamp(self.start, 0)
            .map(|t| t.format("%B").to_string())
            .unwrap_or_default()
    }
}

/// One domain of the report: its issues over the 30 days before sending,
/// and its next policy step with the figures of the same 30 days.
pub struct ReportDomain {
    pub summary: DomainSummary,
    pub readiness: PolicyReadiness,
}

impl ReportDomain {
    /// What the report judges the domain by.
    fn facts(&self) -> ReportDomainFacts<'_> {
        let d = &self.summary;
        ReportDomainFacts {
            assessment: &d.assessment,
            readiness: &self.readiness,
            parked: d.row.parked,
            dns_current: d.row.dns.is_some() && !d.dns_updating,
        }
    }
}

/// What one organization's report covers.
pub struct ClientReportInput<'a> {
    pub organization: &'a str,
    pub now: i64,
    pub month: ReportMonth,
    /// The organization's domains, by name.
    pub domains: &'a [ReportDomain],
    /// The organization's changes in `month`.
    pub events: &'a [AlertItem],
    /// The organization's mail in `month`, against the month before.
    pub mail: &'a Statistics,
    pub letterhead: &'a Letterhead,
}

/// How a piece of a line is shown in the HTML version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    /// The domain's name, in bold.
    Name,
    /// A figure or word in a verdict's colour.
    Coloured(Status),
}

/// A line in pieces, so that the plain text and the HTML say the same.
#[derive(Default)]
struct Pieces(Vec<(String, Look)>);

impl Pieces {
    fn push(&mut self, text: impl Into<String>, look: Look) {
        self.0.push((text.into(), look));
    }

    fn plain(&mut self, text: impl Into<String>) {
        self.push(text, Look::Plain);
    }

    fn text(&self) -> String {
        self.0.iter().map(|(t, _)| t.as_str()).collect()
    }

    fn html(&self) -> String {
        self.0
            .iter()
            .map(|(t, look)| match look {
                Look::Plain | Look::Coloured(Status::Neutral) => escape(t),
                Look::Name => format!("<strong>{}</strong>", escape(t)),
                Look::Coloured(status) => format!(
                    "<strong style=\"color:{}\">{}</strong>",
                    status_colours(*status).0,
                    escape(t)
                ),
            })
            .collect()
    }
}

/// The organization's verdict in words: every domain protected (no open
/// issue), or how many need attention.
fn verdict_line(verdict: &ReportVerdict, domains: &[ReportDomain], organization: &str) -> String {
    let only = match domains {
        [one] => Some(one.summary.row.domain.as_str()),
        _ => None,
    };
    match (verdict.domains, only, verdict.needing_attention) {
        (0, _, _) => format!("No domains are monitored for {organization} yet."),
        (_, Some(one), 0) => format!("{one} is protected."),
        (_, Some(one), _) => format!("{one} needs attention."),
        (all, None, 0) => format!("All {all} domains are protected."),
        (all, None, 1) => format!("1 of {all} domains needs attention."),
        (all, None, n) => format!("{n} of {all} domains need attention."),
    }
}

/// ` (up 7.9%)`; empty without a previous month to compare with.
fn change(trend: Option<Trend>) -> String {
    trend.map_or_else(String::new, |t| format!(" ({})", trend_words(&t)))
}

/// `up 8.0%`, `down 12.0%`, `unchanged`, `none the month before`.
fn trend_words(t: &Trend) -> String {
    let delta = format_pct(t.delta.unwrap_or(0.0).abs());
    match t.direction {
        TrendDirection::Up => format!("up {delta}"),
        TrendDirection::Down => format!("down {delta}"),
        TrendDirection::Flat => "unchanged".to_owned(),
        TrendDirection::New => "none the month before".to_owned(),
    }
}

/// The month's mail in one sentence, each figure with its change against
/// the month before when reports cover it.
fn mail_line(mail: &Statistics, month: &ReportMonth) -> String {
    let counts = &mail.row.mail;
    if counts.total_messages == 0 {
        return format!(
            "No mail from these domains was reported in {}.",
            month.name()
        );
    }
    let total = match mail.total_messages_trend {
        Some(t) => format!(", {} on {}", trend_words(&t), month.previous().month_name()),
        None => String::new(),
    };
    format!(
        "In {}, receivers reported {}{total}: {} passed DMARC{}, {} {} blocked by policy{}, {} {} forwarded{} and {} failed but {} delivered{}.",
        month.name(),
        count_noun(counts.total_messages, "message"),
        format_count(counts.compliant_messages),
        change(mail.compliant_messages_trend),
        format_count(mail.verdicts.blocked_messages),
        were(mail.verdicts.blocked_messages),
        change(mail.blocked_messages_trend),
        format_count(counts.forwarded_failures),
        were(counts.forwarded_failures),
        change(mail.forwarded_failures_trend),
        format_count(counts.delivered_failures),
        were(counts.delivered_failures),
        change(mail.delivered_failures_trend),
    )
}

fn were(n: i64) -> &'static str {
    one_or_many(n, "was", "were")
}

/// A domain's line: `novatraffic.ch, p=none: 9,973 messages, 97.7% passed
/// DMARC, 224 failed but were delivered, 5 forwarded. DNS grade D.`
fn domain_line(domain: &ReportDomain, line: &ReportLine) -> Pieces {
    let d = &domain.summary;
    let mut pieces = Pieces::default();
    let name = if d.row.parked {
        format!("{} (parked)", d.row.domain)
    } else {
        d.row.domain.clone()
    };
    pieces.push(name, Look::Name);
    pieces.plain(format!(", {}", policy_words(&d.policy_label())));
    if let Some(status) = line.step {
        pieces.plain(", ");
        pieces.push(step_words(&domain.readiness), Look::Coloured(status));
    }
    pieces.plain(": ");
    for (i, (figure, words)) in line_figures(line).into_iter().enumerate() {
        if i > 0 {
            pieces.plain(", ");
        }
        match figure {
            Figure::DeliveredFailures => {
                pieces.push(words, Look::Coloured(line.delivered_failures_status));
            }
            Figure::Messages | Figure::Passed | Figure::Blocked | Figure::Forwarded => {
                pieces.plain(words);
            }
        }
    }
    pieces.plain(".");
    let labels: Vec<&str> = line
        .notes
        .iter()
        .filter_map(|&kind| {
            d.assessment
                .worded_issues
                .iter()
                .find(|w| w.issue.kind == kind)
                .map(|w| w.label.as_str())
        })
        .collect();
    if !labels.is_empty() {
        pieces.plain(format!(" {}.", note_words(&labels)));
    }
    pieces
}

/// A domain's changes in the month, the worst three worded: the line over
/// them and the list.
fn changes(domain: &str, events: &[&AlertItem], month: &str) -> (String, Vec<String>) {
    let mut ordered: Vec<&AlertItem> = events.to_vec();
    // Newest first, then worst first (stable): the worst, most recent lead.
    ordered.sort_by_key(|i| std::cmp::Reverse((i.row.created_at, i.row.id)));
    worst_first(&mut ordered, |i| i.row.event.severity);
    let lead = format!(
        "{domain}: {} in {month}{}",
        plural(events.len(), "change"),
        if events.len() > CHANGES_SHOWN {
            ", the most important:"
        } else {
            ":"
        }
    );
    let mut items: Vec<String> = ordered
        .iter()
        .take(CHANGES_SHOWN)
        .map(|i| i.text.clone())
        .collect();
    if events.len() > CHANGES_SHOWN {
        items.push(format!("and {} more.", events.len() - CHANGES_SHOWN));
    }
    (lead, items)
}

/// The parked line in words: all publish `p=reject`, or the ones that do
/// not are named.
fn parked_words(line: &ParkedLine, domains: &[ReportDomain]) -> String {
    if line.not_rejecting.is_empty() {
        return match line.parked {
            1 => "1 parked domain publishes p=reject.".to_owned(),
            n => format!("{n} parked domains publish p=reject."),
        };
    }
    let names: Vec<&str> = line
        .not_rejecting
        .iter()
        .map(|&i| domains[i].summary.row.domain.as_str())
        .collect();
    let verb = if names.len() == 1 { "does" } else { "do" };
    format!(
        "Parked domains should publish p=reject; {} {verb} not.",
        join_and(&names)
    )
}

/// The report's title: `DMARC report for Acme, September 2026`.
pub fn title(organization: &str, month: &ReportMonth) -> String {
    format!("DMARC report for {organization}, {}", month.name())
}

/// `Acme: DMARC report for September 2026`.
pub fn subject(input: &ClientReportInput<'_>) -> String {
    format!(
        "{}: DMARC report for {}",
        input.organization,
        input.month.name()
    )
}

/// A paragraph in both versions; `status` colours the HTML.
fn paragraph(text: &mut Vec<String>, html: &mut String, line: &str, status: Option<Status>) {
    text.push(line.to_owned());
    match status.filter(|&s| s != Status::Neutral) {
        Some(status) => html.push_str(&format!(
            "<p><strong style=\"color:{}\">{}</strong></p>",
            status_colours(status).0,
            escape(line)
        )),
        None => html.push_str(&format!("<p>{}</p>", escape(line))),
    }
}

/// A paragraph that leads into a list, in both versions: `- item` lines in
/// the text, a `<ul>` in the HTML.
fn list(text: &mut Vec<String>, html: &mut String, lead: &str, items: &[Pieces]) {
    let mut lines = vec![lead.to_owned()];
    lines.extend(items.iter().map(|i| format!("- {}", i.text())));
    text.push(lines.join("\n"));
    let lis: String = items
        .iter()
        .map(|i| format!("<li style=\"margin:4px 0\">{}</li>", i.html()))
        .collect();
    html.push_str(&format!(
        "<p style=\"margin-bottom:4px\">{}</p><ul style=\"margin:0 0 12px;padding-left:20px\">{lis}</ul>",
        escape(lead)
    ));
}

pub fn render(input: &ClientReportInput<'_>) -> Content {
    let subject = subject(input);
    let month = input.month.name();
    let mut text: Vec<String> = Vec::new();
    let mut html = String::new();

    let facts: Vec<ReportDomainFacts<'_>> = input.domains.iter().map(ReportDomain::facts).collect();

    let verdict = report_verdict(&facts);
    paragraph(
        &mut text,
        &mut html,
        &verdict_line(&verdict, input.domains, input.organization),
        Some(verdict.status),
    );

    if !input.domains.is_empty() {
        paragraph(
            &mut text,
            &mut html,
            &mail_line(input.mail, &input.month),
            None,
        );

        // Lines in the order the attention list ranks them, then by name.
        let summaries: Vec<DomainSummary> =
            input.domains.iter().map(|d| d.summary.clone()).collect();
        let ranked: Vec<i64> = needing_attention(&summaries)
            .iter()
            .map(|d| d.row.id)
            .collect();
        let mut order: Vec<(&ReportDomain, &ReportDomainFacts<'_>)> =
            input.domains.iter().zip(&facts).collect();
        order.sort_by_cached_key(|(d, _)| {
            let rank = ranked.iter().position(|&id| id == d.summary.row.id);
            (rank.unwrap_or(usize::MAX), d.summary.row.domain.clone())
        });
        let lines: Vec<Pieces> = order
            .iter()
            .filter_map(|(domain, facts)| match report_place(facts) {
                ReportPlace::Line(line) => Some(domain_line(domain, &line)),
                ReportPlace::ParkedOnly => None,
            })
            .collect();
        if !lines.is_empty() {
            let lead = format!("Over the last {READINESS_WINDOW_DAYS} days:");
            list(&mut text, &mut html, &lead, &lines);
        }

        for (domain, _) in &order {
            let id = domain.summary.row.id;
            let events: Vec<&AlertItem> = input
                .events
                .iter()
                .filter(|e| e.row.domain_id == id)
                .collect();
            if events.is_empty() {
                continue;
            }
            let (lead, items) = changes(&domain.summary.row.domain, &events, &month);
            let items: Vec<Pieces> = items
                .into_iter()
                .map(|i| {
                    let mut p = Pieces::default();
                    p.plain(i);
                    p
                })
                .collect();
            list(&mut text, &mut html, &lead, &items);
        }

        if let Some(line) = parked_line(&facts) {
            paragraph(
                &mut text,
                &mut html,
                &parked_words(&line, input.domains),
                Some(line.status),
            );
        }
    }

    let mut text = text.join("\n\n");
    text.push('\n');
    let heading = title(input.organization, &input.month);
    let body = Body {
        title: &heading,
        text: &text,
        html: &html,
    };
    finish(subject, input.letterhead, &body, CLIENT_REPORT_FOOTER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::domain::{domain_summaries, summary_readiness};
    use crate::view::stats::statistics;
    use dmarc_assess::{AlertEvent, AlertKind, AlertedIssue};
    use dmarc_store::{
        DomainOrganization, DomainSummaryRow, MailCounts, PublishedPolicy, ReadinessMailRow,
        ScopeStats, StoredDnsGrade,
    };

    const DAY: i64 = 86_400;
    /// 2026-10-01 07:00 UTC.
    const NOW: i64 = 1_790_838_000;

    fn row(id: i64, domain: &str, policy: &str, total: i64, failures: i64) -> DomainSummaryRow {
        DomainSummaryRow {
            id,
            organization: DomainOrganization {
                id: 2,
                name: "Acme".into(),
            },
            domain: domain.into(),
            report_count: 40,
            last_report_at: Some(NOW - DAY),
            policy: Some(PublishedPolicy {
                p: Some(policy.into()),
                ..PublishedPolicy::default()
            }),
            policy_source: dmarc_store::PolicySource::Dns,
            reporter_count: 2,
            mail: MailCounts {
                total_messages: total,
                compliant_messages: total - failures,
                delivered_failures: failures,
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

    fn parked(id: i64, domain: &str, policy: &str) -> DomainSummaryRow {
        DomainSummaryRow {
            parked: true,
            ..row(id, domain, policy, 0, 0)
        }
    }

    fn domains(rows: Vec<DomainSummaryRow>) -> Vec<ReportDomain> {
        domain_summaries(rows, crate::view::domain::Judging::defaults_at(NOW))
            .into_iter()
            .map(|summary| {
                // Every message under the DNS policy, reports under it for 90 days.
                let mail = ReadinessMailRow {
                    mail: summary.row.mail,
                    policy_messages: summary.row.mail.total_messages,
                    policy_first_report_at: Some(NOW - 90 * DAY),
                    other_policy: None,
                };
                ReportDomain {
                    readiness: summary_readiness(&summary.row, &mail, NOW),
                    summary,
                }
            })
            .collect()
    }

    fn counts(total: i64, compliant: i64, delivered: i64, forwarded: i64) -> ScopeStats {
        ScopeStats {
            mail: MailCounts {
                total_messages: total,
                compliant_messages: compliant,
                delivered_failures: delivered,
                forwarded_failures: forwarded,
            },
            ..Default::default()
        }
    }

    /// 12,480 messages in September, 11,556 in August.
    fn month_mail() -> Statistics {
        statistics(
            counts(12_480, 12_300, 10, 20),
            Some(counts(11_556, 11_370, 0, 20)),
            4,
        )
    }

    fn input<'a>(
        domains: &'a [ReportDomain],
        events: &'a [AlertItem],
        mail: &'a Statistics,
        letterhead: &'a Letterhead,
    ) -> ClientReportInput<'a> {
        ClientReportInput {
            organization: "Acme",
            now: NOW,
            month: ReportMonth::before(NOW),
            domains,
            events,
            mail,
            letterhead,
        }
    }

    static NOTICE: Letterhead = Letterhead {
        operator_name: None,
        logo: None,
        notice: true,
    };

    /// What comes between the title and the footer.
    fn body(text: &str) -> &str {
        let start = text.find("2026\n\n").expect("a title") + "2026\n\n".len();
        let end = text.find("\n-- \n").expect("a footer");
        &text[start..end]
    }

    #[test]
    fn the_month_is_the_calendar_month_before() {
        let month = ReportMonth::before(NOW);
        assert_eq!(month.name(), "September 2026");
        assert_eq!(month.previous().month_name(), "August");
        assert_eq!(month.end - month.start, 30 * DAY);
        assert_eq!(
            super::super::text::format_time(month.start),
            "2026-09-01 00:00 UTC"
        );
        // January reaches back into the previous year.
        let january = ReportMonth::before(NOW + 100 * DAY);
        assert_eq!(january.name(), "December 2026");
    }

    /// A quiet month: the verdict, the mail with its change, a line of
    /// figures per domain, the parked ones, and the sign-off.
    #[test]
    fn a_green_month_is_its_figures() {
        let domains = domains(vec![
            row(1, "acme.example", "reject", 12_000, 0),
            row(2, "acme-shop.example", "reject", 480, 0),
            parked(3, "acme.biz", "reject"),
            parked(4, "acme.info", "reject"),
        ]);
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        assert_eq!(report.subject, "Acme: DMARC report for September 2026");
        assert_eq!(
            report.text,
            concat!("Marcan, free software

DMARC report for Acme, September 2026

All 4 domains are protected.

In September 2026, receivers reported 12,480 messages, up 8.0% on August: 12,300 passed DMARC (up 8.2%), 150 were blocked by policy (down 9.6%), 20 were forwarded (unchanged) and 10 failed but were delivered (none the month before).

Over the last 30 days:
- acme-shop.example, p=reject: 480 messages, 100.0% passed DMARC.
- acme.example, p=reject: 12,000 messages, 100.0% passed DMARC.

2 parked domains publish p=reject.

--\x20
This report shows how mail sent from your domains is authenticated (DMARC).

Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "
")
        );
        assert!(
            report.html.contains("<p><strong style=\"color:"),
            "{}",
            report.html
        );
        assert!(
            report
                .html
                .contains("<li style=\"margin:4px 0\"><strong>acme.example</strong>, p=reject: 12,000 messages, 100.0% passed DMARC.</li>"),
            "{}",
            report.html
        );
    }

    /// Without reports covering the month before, the mail has no changes.
    #[test]
    fn the_mail_line_compares_only_with_a_covered_month() {
        let mail = statistics(counts(400, 400, 0, 0), None, 1);
        assert_eq!(
            mail_line(&mail, &ReportMonth::before(NOW)),
            "In September 2026, receivers reported 400 messages: 400 passed DMARC, 0 were blocked by policy, 0 were forwarded and 0 failed but were delivered."
        );
        let none = statistics(counts(0, 0, 0, 0), None, 1);
        assert_eq!(
            mail_line(&none, &ReportMonth::before(NOW)),
            "No mail from these domains was reported in September 2026."
        );
    }

    /// A domain with an open issue leads, its figures saying what the issue
    /// names: no status word stands without a number behind it.
    #[test]
    fn a_domain_with_an_issue_reads_as_its_figures() {
        let mut shop = row(2, "acme-shop.example", "none", 1_000, 60);
        let dns = shop.dns.as_mut().unwrap();
        dns.grade = "D".into();
        dns.score = 55;
        let domains = domains(vec![
            row(1, "acme.example", "reject", 12_000, 0),
            shop,
            parked(3, "acme.biz", "none"),
        ]);
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        let text = body(&report.text);
        assert!(
            text.starts_with("2 of 3 domains need attention.\n\n"),
            "{text}"
        );
        assert!(
            text.contains(concat!(
                "Over the last 30 days:\n",
                "- acme-shop.example, p=none: 1,000 messages, 94.0% passed DMARC, 60 failed but were delivered. DNS grade D.\n",
                "- acme.example, p=reject: 12,000 messages, 100.0% passed DMARC.\n\n",
                "Parked domains should publish p=reject; acme.biz does not."
            )),
            "{text}"
        );
        for gone in [
            "Attention",
            "not enforcing",
            "Not ready",
            "failing DNS grade",
        ] {
            assert!(!text.contains(gone), "{gone:?} in\n{text}");
        }
        // The failures carry the issue's colour; nothing else on the line
        // is coloured.
        assert!(
            report.html.contains(
                "<strong>acme-shop.example</strong>, p=none: 1,000 messages, 94.0% passed DMARC, <strong style=\"color:#b91c1c\">60 failed but were delivered</strong>. DNS grade D.</li>"
            ),
            "{}",
            report.html
        );
    }

    /// A step that can be taken now is said beside the policy, coloured,
    /// in the domain page's words.
    #[test]
    fn a_step_to_take_now_is_said_beside_the_policy() {
        let domains = domains(vec![row(1, "acme.example", "none", 1_000, 0)]);
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        assert!(
            report.text.contains(
                "- acme.example, p=none, ready for p=quarantine: 1,000 messages, 100.0% passed DMARC.\n"
            ),
            "{}",
            report.text
        );
        assert!(
            report.html.contains(
                "p=none, <strong style=\"color:#15803d\">ready for p=quarantine</strong>: "
            ),
            "{}",
            report.html
        );
        let page = crate::wording::readiness::verdict(&domains[0].readiness);
        assert_eq!(page.lead, "Ready for p=quarantine");
    }

    #[test]
    fn the_worst_three_changes_are_worded_and_the_rest_counted() {
        let domains = domains(vec![row(1, "acme.example", "reject", 150, 0)]);
        let event = |id: i64, kind: AlertKind, severity: Status| {
            AlertItem::from(dmarc_store::AlertEventRow {
                id,
                domain_id: 1,
                domain: "acme.example".into(),
                organization_id: 2,
                organization: "Acme".into(),
                created_at: NOW - 20 * DAY + id,
                notified_at: None,
                event: AlertEvent { kind, severity },
            })
        };
        let opened = |issue| AlertKind::IssueOpened {
            issue,
            mail: None,
            stopped: None,
        };
        let resolved = |issue| AlertKind::IssueResolved {
            issue,
            mail: None,
            stopped: None,
        };
        let events = [
            event(1, resolved(AlertedIssue::ReportsStopped), Status::Pass),
            event(2, opened(AlertedIssue::DeliveredFailures), Status::Fail),
            event(3, resolved(AlertedIssue::ReportsStopped), Status::Pass),
            event(4, opened(AlertedIssue::ReportsStopped), Status::Warn),
            event(5, resolved(AlertedIssue::DeliveredFailures), Status::Pass),
        ];
        let mail = month_mail();
        let report = render(&input(&domains, &events, &mail, &NOTICE));
        let text = body(&report.text);
        assert!(text.starts_with("acme.example is protected.\n\n"), "{text}");
        assert!(
            text.contains("\n\nacme.example: 5 changes in September 2026, the most important:\n- New issue: delivered failures.\n- New issue: reports stopped.\n- Resolved: delivered failures.\n- and 2 more."),
            "{text}"
        );
        assert!(report.html.contains(">and 2 more.</li>"));
    }

    /// With an operator key nothing names Marcan, the sign-off included.
    #[test]
    fn a_licensed_report_does_not_name_marcan() {
        let domains = domains(vec![row(1, "acme.example", "reject", 12_000, 0)]);
        let mail = month_mail();
        let licensed = Letterhead {
            operator_name: Some("Acme IT".into()),
            logo: None,
            notice: false,
        };
        let report = render(&input(&domains, &[], &mail, &licensed));
        assert!(!report.text.contains("Marcan"), "{}", report.text);
        assert!(!report.html.contains("Marcan"), "{}", report.html);
        assert!(!report.subject.contains("Marcan"));
        assert!(report.text.starts_with("Acme IT\n\nDMARC report for Acme, September 2026\n\nacme.example is protected.\n\n"), "{}", report.text);
    }

    /// The line's figures add up: passed, delivered failures, blocked and
    /// forwarded are the messages.
    #[test]
    fn the_figures_add_up_to_the_messages() {
        let mut r = row(1, "acme.example", "reject", 1_000, 50);
        // 900 passed, 50 failed and were delivered, 20 forwarded, 30 blocked.
        r.mail.compliant_messages = 900;
        r.mail.forwarded_failures = 20;
        let domains = domains(vec![r]);
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        assert!(
            report.text.contains(
                "- acme.example, p=reject: 1,000 messages, 90.0% passed DMARC, 50 failed but were delivered, 30 blocked, 20 forwarded.\n"
            ),
            "{}",
            report.text
        );
    }

    #[test]
    fn an_outdated_grade_and_a_passing_one_are_left_out() {
        let mut outdated = row(1, "old.example", "none", 300, 0);
        let dns = outdated.dns.as_mut().unwrap();
        dns.checker_version = 0;
        dns.grade = "F".into();
        let domains = domains(vec![outdated, row(2, "new.example", "none", 300, 0)]);
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        assert!(
            !report.text.contains("DNS grade"),
            "an outdated or passing grade is not shown: {}",
            report.text
        );
        assert!(
            report
                .text
                .contains("- old.example, p=none, ready for p=quarantine: 300 messages, 100.0% passed DMARC.\n"),
            "{}",
            report.text
        );
    }
}
