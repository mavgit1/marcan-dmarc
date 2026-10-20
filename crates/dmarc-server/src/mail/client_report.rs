//! The client report: one organization's month in a short email, for
//! someone who never opens Marcan. Its length follows its content:
//!
//! 1. A verdict for the organization: all domains protected, or how many
//!    need attention ([`report_verdict`]).
//! 2. The calendar month's mail with the change against the month before
//!    ([`Statistics`] over the organization and the month): messages,
//!    passed, blocked by policy, forwarded, failed but delivered.
//! 3. One short block per domain that has something to say: an open issue,
//!    a pending policy step (ready, or raise `pct`), or a change in the
//!    month. The block words the issues, the next step and what it would
//!    affect, the DNS grade when it is not passing, and the changes.
//! 4. The protected and unchanged domains on one line, and the parked
//!    domains on another.
//!
//! A quiet month is the verdict, the mail line, the two domain lines and
//! the sign-off. Only what Marcan is sure of goes in: no sender names or
//! IPs, no reporters; a DNS grade from an older checker is left out. What
//! goes where, and every colour, is `dmarc_assess`'s ([`report_place`],
//! [`report_verdict`], [`parked_line`]); this words and lays it out. Pure:
//! view types in, text and HTML out.

use super::frame::{finish, Body, Content};
use super::text::{escape, format_month, status_colours, Letterhead, CLIENT_REPORT_FOOTER};
use crate::view::alerts::AlertItem;
use crate::view::domain::{needing_attention, DomainSummary};
use crate::view::stats::Statistics;
use crate::wording::alerts::step_name;
use crate::wording::issues::issue_name;
use crate::wording::readiness;
use crate::wording::{
    capitalized, count_noun, format_count, format_pct, join_and, one_or_many, plural,
};
use chrono::{DateTime, Datelike, Months, NaiveDate, TimeZone, Utc};
use dmarc_assess::{
    parked_line, report_place, report_verdict, worst_first, ParkedLine, PolicyReadiness,
    PolicyStep, ReportBlock, ReportDomainFacts, ReportPlace, ReportVerdict, Status, Trend,
    TrendDirection, READINESS_WINDOW_DAYS,
};

/// How many changes a domain's block words; the rest are counted.
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

/// One domain of the report: its figures and issues over the 30 days
/// before sending, its next policy step, and how many senders' mail fails
/// DMARC in the readiness window (what the step would affect).
pub struct ReportDomain {
    pub summary: DomainSummary,
    pub readiness: PolicyReadiness,
    pub failing_senders: i64,
}

impl ReportDomain {
    /// What the report judges the domain by, with its `changes` in the month.
    fn facts(&self, changes: usize) -> ReportDomainFacts<'_> {
        let d = &self.summary;
        ReportDomainFacts {
            assessment: &d.assessment,
            readiness: &self.readiness,
            parked: d.row.parked,
            dns_current: d.row.dns.is_some() && !d.dns_updating,
            changes,
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

/// One line of a domain's block: its label, the value, the value's colour
/// (`None`: not a verdict), and the changes listed under it.
struct Row {
    label: &'static str,
    lead: String,
    rest: String,
    status: Option<Status>,
    items: Vec<String>,
}

impl Row {
    fn new(
        label: &'static str,
        lead: String,
        rest: impl Into<String>,
        status: Option<Status>,
    ) -> Self {
        Self {
            label,
            lead,
            rest: rest.into(),
            status,
            items: Vec::new(),
        }
    }

    fn text(&self) -> String {
        format!("{}{}", self.lead, self.rest)
    }
}

const LABEL_WIDTH: usize = 12;

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

/// The open issues the block names, worst first; `None` without any.
fn issues_row(block: &ReportBlock) -> Option<Row> {
    if block.issues.is_empty() {
        return None;
    }
    let names: Vec<&str> = block.issues.iter().map(|&k| issue_name(k)).collect();
    Some(Row::new(
        "Attention",
        capitalized(&join_and(&names)),
        ".",
        Some(block.issues_status),
    ))
}

/// Delivered compliance, the figure behind delivered failures (the block
/// says when). The count is the mail the rate is over (passed
/// and delivered failures), not blocked or forwarded mail.
fn compliance_row(d: &DomainSummary) -> Option<Row> {
    let rate = d.verdicts.delivered_compliance;
    let pct = rate.pct?;
    Some(Row::new(
        "Compliance",
        format_pct(pct),
        format!(
            " of delivered mail over the last {} days passed DMARC ({}).",
            READINESS_WINDOW_DAYS,
            count_noun(d.row.mail.mail().delivered(), "message")
        ),
        Some(rate.status),
    ))
}

/// The next policy step, in the words of the domain page's verdict
/// ([`readiness::verdict`]).
fn readiness_row(r: &PolicyReadiness) -> Row {
    let verdict = readiness::verdict(r);
    Row::new(
        "Next step",
        verdict.lead,
        format!(": {}", verdict.detail),
        Some(r.status),
    )
}

/// What moving to `next` would act on: the window's mail that fails DMARC
/// and is not forwarded, and how many senders it comes from.
fn impact_row(r: &PolicyReadiness, next: PolicyStep, failing_senders: i64) -> Row {
    let days = READINESS_WINDOW_DAYS;
    let text = if r.window.failed == 0 {
        format!(
            "Moving to {} would affect none of the last {days} days' mail.",
            step_name(next)
        )
    } else {
        format!(
            "Moving to {} would affect {} from {} over the last {days} days.",
            step_name(next),
            count_noun(r.window.failed, "message"),
            count_noun(failing_senders, "sender")
        )
    };
    Row::new("Impact", text, "", None)
}

/// The stored DNS grade, coloured `status` (the block shows only a current
/// grade that does not pass).
fn dns_row(d: &DomainSummary, status: Status) -> Option<Row> {
    let grade = d.row.dns.as_ref()?.grade.clone();
    Some(Row::new(
        "DNS grade",
        grade,
        " for the domain's DMARC, SPF, DKIM and related records.",
        Some(status),
    ))
}

/// The month's changes, the worst three worded; `None` without any.
fn changes_row(events: &[&AlertItem], month: &str) -> Option<Row> {
    if events.is_empty() {
        return None;
    }
    let mut ordered: Vec<&AlertItem> = events.to_vec();
    // Newest first, then worst first (stable): the worst, most recent lead.
    ordered.sort_by_key(|i| std::cmp::Reverse((i.row.created_at, i.row.id)));
    worst_first(&mut ordered, |i| i.row.event.severity);
    let mut row = Row::new(
        "Changes",
        plural(events.len(), "change"),
        format!(
            " in {month}{}",
            if events.len() > CHANGES_SHOWN {
                ", the most important:"
            } else {
                ":"
            }
        ),
        None,
    );
    row.items = ordered
        .iter()
        .take(CHANGES_SHOWN)
        .map(|i| i.text.clone())
        .collect();
    if events.len() > CHANGES_SHOWN {
        row.items
            .push(format!("and {} more.", events.len() - CHANGES_SHOWN));
    }
    Some(row)
}

/// The lines `block` holds, worded.
fn rows(
    domain: &ReportDomain,
    block: &ReportBlock,
    events: &[&AlertItem],
    month: &str,
) -> Vec<Row> {
    let (d, r) = (&domain.summary, &domain.readiness);
    let mut rows = Vec::new();
    rows.extend(issues_row(block));
    rows.extend(compliance_row(d).filter(|_| block.compliance));
    rows.extend(block.next_step.then(|| readiness_row(r)));
    rows.extend(
        block
            .impact
            .map(|next| impact_row(r, next, domain.failing_senders)),
    );
    rows.extend(block.dns.and_then(|status| dns_row(d, status)));
    rows.extend(changes_row(events, month).filter(|_| block.changes));
    rows
}

/// The protected and unchanged domains (no block, not parked) on one line;
/// `None` without any.
fn green_line(names: &[&str]) -> Option<String> {
    match names {
        [] => None,
        [one] => Some(format!("{one} is protected and unchanged.")),
        all => Some(format!(
            "{} domains are protected and unchanged: {}.",
            all.len(),
            all.join(", ")
        )),
    }
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

fn block(domain: &ReportDomain, rows: &[Row], text: &mut Vec<String>, html: &mut String) {
    let d = &domain.summary;
    let name = if d.row.parked {
        format!("{} (parked: sends no mail)", d.row.domain)
    } else {
        d.row.domain.clone()
    };
    let mut lines = vec![name.clone()];
    html.push_str(&format!(
        "<h2 style=\"font-size:15px;margin:20px 0 6px\">{}</h2>\
         <table style=\"border-collapse:collapse;font-size:14px\">",
        escape(&name)
    ));
    for row in rows {
        lines.push(format!(
            "  {:<width$}{}",
            row.label,
            row.text(),
            width = LABEL_WIDTH
        ));
        for item in &row.items {
            lines.push(format!("  {:<width$}- {item}", "", width = LABEL_WIDTH));
        }
        let lead = match row.status {
            Some(status) => format!(
                "<strong style=\"color:{}\">{}</strong>",
                status_colours(status).0,
                escape(&row.lead)
            ),
            None => escape(&row.lead),
        };
        let items = if row.items.is_empty() {
            String::new()
        } else {
            let lis: String = row
                .items
                .iter()
                .map(|i| format!("<li>{}</li>", escape(i)))
                .collect();
            format!("<ul style=\"margin:4px 0;padding-left:18px\">{lis}</ul>")
        };
        html.push_str(&format!(
            "<tr><td style=\"color:#737373;padding:3px 16px 3px 0;vertical-align:top;white-space:nowrap\">{}</td>\
             <td style=\"padding:3px 0\">{lead}{}{items}</td></tr>",
            escape(row.label),
            escape(&row.rest)
        ));
    }
    html.push_str("</table>");
    text.push(lines.join("\n"));
}

pub fn render(input: &ClientReportInput<'_>) -> Content {
    let subject = subject(input);
    let month = input.month.name();
    let mut text: Vec<String> = Vec::new();
    let mut html = String::new();

    let events_of = |domain: &ReportDomain| -> Vec<&AlertItem> {
        let id = domain.summary.row.id;
        input
            .events
            .iter()
            .filter(|e| e.row.domain_id == id)
            .collect()
    };
    let facts: Vec<ReportDomainFacts<'_>> = input
        .domains
        .iter()
        .map(|d| d.facts(events_of(d).len()))
        .collect();

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

        // Blocks in the order the attention list ranks them, then by name.
        let summaries: Vec<DomainSummary> =
            input.domains.iter().map(|d| d.summary.clone()).collect();
        let ranked: Vec<i64> = needing_attention(&summaries)
            .iter()
            .map(|d| d.row.id)
            .collect();
        let mut order: Vec<(&ReportDomain, &ReportDomainFacts<'_>)> =
            input.domains.iter().zip(&facts).collect();
        order.sort_by_key(|(d, _)| {
            ranked
                .iter()
                .position(|&id| id == d.summary.row.id)
                .unwrap_or(usize::MAX)
        });
        let mut green: Vec<&str> = Vec::new();
        for (domain, facts) in order {
            match report_place(facts) {
                ReportPlace::Block(b) => {
                    let rows = rows(domain, &b, &events_of(domain), &month);
                    block(domain, &rows, &mut text, &mut html);
                }
                ReportPlace::Quiet => green.push(&domain.summary.row.domain),
                ReportPlace::ParkedOnly => {}
            }
        }
        green.sort_unstable();
        if let Some(line) = green_line(&green) {
            paragraph(&mut text, &mut html, &line, None);
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
                    failing_senders: 3,
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

    /// A quiet month: the verdict, the mail with its change, the protected
    /// domains, the parked ones, and the sign-off. Nothing else.
    #[test]
    fn a_green_month_is_five_lines() {
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

2 domains are protected and unchanged: acme-shop.example, acme.example.

2 parked domains publish p=reject.

--\x20
This report shows how mail sent from your domains is authenticated (DMARC).

Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "
")
        );
        assert_eq!(body(&report.text).split("\n\n").count(), 4);
        assert!(
            report.html.contains("<p><strong style=\"color:"),
            "{}",
            report.html
        );
        assert!(
            !report.html.contains("<h2"),
            "no domain blocks: {}",
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

    /// A domain with an open issue gets a block: the issue, the figure
    /// behind it, the next step and what it would affect, a failing DNS
    /// grade; the others stay on the protected line.
    #[test]
    fn a_domain_with_an_issue_gets_a_block() {
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
        for line in [
            "acme-shop.example\n  Attention   ",
            "  Compliance  94.0% of delivered mail over the last 30 days passed DMARC (1,000 messages).\n",
            "  Next step   Not ready for p=quarantine yet: 94.0% delivered compliance over 30 days (needs 99%).\n",
            "  Impact      Moving to p=quarantine would affect 60 messages from 3 senders over the last 30 days.\n",
            "  DNS grade   D for the domain's DMARC, SPF, DKIM and related records.",
            "\n\nacme.example is protected and unchanged.\n\nParked domains should publish p=reject; acme.biz does not.",
        ] {
            assert!(text.contains(line), "missing {line:?} in\n{text}");
        }
        assert!(
            text.contains("Delivered failures, failing DNS grade and not enforcing (p=none)."),
            "{text}"
        );
        assert!(
            !text.contains("acme.biz (parked"),
            "no parked block: {text}"
        );
        assert!(report
            .html
            .contains("<h2 style=\"font-size:15px;margin:20px 0 6px\">acme-shop.example</h2>"));
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
        let events = [
            event(
                1,
                AlertKind::IssueResolved {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: None,
                },
                Status::Pass,
            ),
            event(
                2,
                AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: None,
                    stopped: None,
                },
                Status::Fail,
            ),
            event(
                3,
                AlertKind::IssueResolved {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: None,
                },
                Status::Pass,
            ),
            event(
                4,
                AlertKind::IssueOpened {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: None,
                },
                Status::Warn,
            ),
            event(
                5,
                AlertKind::IssueResolved {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: None,
                    stopped: None,
                },
                Status::Pass,
            ),
        ];
        let mail = month_mail();
        let report = render(&input(&domains, &events, &mail, &NOTICE));
        let text = body(&report.text);
        assert!(text.starts_with("acme.example is protected.\n\n"), "{text}");
        assert!(
            text.contains("\n\nacme.example\n  Changes     5 changes in September 2026, the most important:\n              - New issue: delivered failures.\n              - New issue: reports stopped.\n              - Resolved: delivered failures.\n              - and 2 more."),
            "{text}"
        );
        assert!(!text.contains("protected and unchanged"), "{text}");
        assert!(report.html.contains("<li>and 2 more.</li>"));
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

    /// The count beside the rate is the delivered mail it is over: blocked
    /// and forwarded mail stay out of it, as they stay out of the rate.
    #[test]
    fn the_compliance_count_is_the_delivered_mail() {
        let mut r = row(1, "acme.example", "reject", 1_000, 50);
        // 900 passed, 50 failed and were delivered, 20 forwarded, 30 blocked.
        r.mail.compliant_messages = 900;
        r.mail.forwarded_failures = 20;
        let domains = domains(vec![r]);
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        assert!(
            report.text.contains(
                "  Compliance  94.7% of delivered mail over the last 30 days passed DMARC (950 messages).\n"
            ),
            "{}",
            report.text
        );
    }

    /// `under` of 400 messages evaluated under `p=quarantine`, reports under
    /// it for `days`; the rest still under `p=none`.
    fn quarantined(days: i64, under: i64) -> PolicyReadiness {
        dmarc_assess::policy_readiness(
            &dmarc_assess::ReadinessFacts {
                window_mail: dmarc_assess::Mail {
                    total: 400,
                    compliant: 400,
                    ..Default::default()
                },
                policy: Some("quarantine"),
                pct: None,
                parked: false,
                policy_messages: under,
                policy_first_report_at: Some(NOW - days * DAY),
                other_policy: Some("none"),
            },
            NOW,
        )
    }

    fn readiness_text(r: &PolicyReadiness) -> String {
        readiness_row(r).text()
    }

    /// Too little history reads as the days covered under the current
    /// policy so far, like the UI.
    #[test]
    fn short_history_reads_as_days_covered() {
        let text = readiness_text(&quarantined(12, 400));
        assert!(
            text.ends_with(": reports cover 12 of 30 days under p=quarantine so far."),
            "{text}"
        );
        let r = quarantined(12, 400);
        let impact = impact_row(&r, r.next_step().expect("a next step"), 0).text();
        assert_eq!(
            impact,
            "Moving to p=reject would affect none of the last 30 days' mail."
        );
    }

    /// Mail still evaluated under the old policy reads as its share.
    #[test]
    fn old_policy_mail_reads_as_its_share() {
        let text = readiness_text(&quarantined(40, 352));
        assert_eq!(
            text,
            "Not ready for p=reject yet: receivers still apply the old policy to 12.0% of mail."
        );
    }

    /// The client report's next step reads like the domain page's verdict
    /// (`Readiness`, which the UI shows): every state the report words, from
    /// one function.
    #[test]
    fn the_next_step_reads_like_the_domain_page() {
        use crate::view::readiness::Readiness;
        use crate::wording::readiness::tests::{not_ready, readiness};
        use dmarc_assess::{PolicyStep, ReadinessReason, ReadinessState};
        let (q, r) = (PolicyStep::Quarantine, PolicyStep::Reject);
        let history = ReadinessReason::NotEnoughHistory {
            window_full_at: NOW,
        };
        let states = [
            ReadinessState::Ready { next: q },
            ReadinessState::Ready { next: r },
            ReadinessState::RaisePct { pct: 50 },
            not_ready(q, ReadinessReason::TooLittleMail),
            not_ready(r, history),
            not_ready(r, ReadinessReason::OldPolicyApplied),
            not_ready(q, ReadinessReason::BelowThreshold),
        ];
        for state in states {
            for pct in [Some(97.2), None] {
                let judged = readiness(state, 4_120, pct);
                let page = Readiness::new(judged.clone(), false, "v=DMARC1; p=none");
                let row = readiness_row(&judged);
                assert_eq!(row.text(), format!("{}: {}", page.lead, page.detail));
                assert_eq!(row.status, Some(page.judged.status));
            }
        }
        // In a whole report: the block's line is the page's sentence.
        let domains = domains(vec![row(1, "acme.example", "none", 1_000, 60)]);
        let page = Readiness::new(domains[0].readiness.clone(), false, "v=DMARC1; p=none");
        let mail = month_mail();
        let report = render(&input(&domains, &[], &mail, &NOTICE));
        let line = format!("  Next step   {}: {}\n", page.lead, page.detail);
        assert!(report.text.contains(&line), "{line:?} in\n{}", report.text);
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
            !report.text.contains("  DNS grade   "),
            "an outdated or passing grade is not shown: {}",
            report.text
        );
        assert!(
            report.text.contains(
                "old.example\n  Attention   Failing DNS grade and not enforcing (p=none)."
            ),
            "{}",
            report.text
        );
    }
}
