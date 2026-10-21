# Reading the dashboard

The scope filter at the top narrows a page to an organization, a domain and a time range, as far as the page takes them:

- The dashboard, **Reports**, **Alerts** and **Senders** take all three.
- **Domains**, a domain's page and a sender's page take the organization and the time range.
- **Organizations** and an organization's page take the time range only; on an organization's page, the organization and domain filters open another page.
- Settings, the tools (such as the DNS inspector), a single report and the setup take none.

Exports follow the same scope. The scope is part of the page address (for example `?org=2&since=7d`), so a reload or a bookmark opens the page with the same scope, and the browser's back button goes back to the scope you had.

<Screenshot name="dashboard" alt="The dashboard with its figures, outcome counts and volume per day" />

## Pages

| Page | What it shows |
| :--- | :--- |
| **Dashboard** | Messages, delivered compliance and senders with their change against the previous range; volume over time with the four [outcomes](#outcomes); the attention list. |
| **Reports** | Every imported report, with its records. |
| **Alerts** | Every [change](./alerts) Marcan found about your domains, newest first. |
| **Domains** | Each domain's DNS grade, policy, messages, delivered compliance, delivered failures and last report, 50 at a time; the box above the list filters by domain or organization name (see [The domain list](./domains#the-domain-list)). The policy is the one in the domain's DMARC record, as the last DNS check found it; a new domain is checked within about a minute. Before that it reads **Not checked yet**, and without a DMARC record **No record**. The policy in a report is only what one receiver saw at the time (receivers disagree and lag behind DNS), so it is shown with that report and never as the domain's. A domain's page has an **Overview** tab (messages per day with the four outcomes and their counts for the time range under the chart, DNS, recent changes), a **Senders** tab, a **Failures** tab (how many messages failed DMARC in the time range and what became of them, then every record that failed SPF or DKIM; a **Show** list ticks the kinds to include, each with its message count in the time range: delivered failures, forwarded, blocked by policy, and mail that passed via DKIM only or via SPF only, all ticked by default) and an **Enforcement** tab (whether the next policy step is safe, and why: see [Policy readiness](./readiness)). |
| **Senders** | Every sending IP with its identity, volume, failed mail and pass rate. Failed means failed DMARC and delivered anyway or blocked by policy; forwarded mail is counted apart. A sender's page shows each domain it sent for, and where a domain's SPF record lists it. |
| **Organizations** | Domains grouped by organization: for each, its domains, reports and messages, a bar of its mail by outcome, how many of its domains need attention and the worst DNS grade. |
| **DNS inspector** | Live checks for any domain, monitored or not. |
| **Setup** | The [setup](./getting-started#first-start)'s steps, read-only, over what is set up now (each domain's DMARC record and what it still needs among them), and a short DMARC, SPF and DKIM reference. |

At the bottom of the sidebar, under **Settings**, one line shows the running version and a link to these docs (**Docs**), and "X.Y.Z available" when the [version check](./upgrade#checking-for-new-versions) found a newer release.

The long lists (reports, alerts, domains, senders, a domain's failures) are paged, 50 rows at a time with **Previous** and **Next**. A list scrolls with the page, never in a box of its own, and on a wide window its column headers stay in view under the header.

A domain's page puts its mail and its DNS side by side:

<Screenshot name="domain" alt="A domain's overview: at p=none and ready for p=quarantine, messages per day with the four outcomes, the DNS grade and the checks behind it" />

The **Failures** tab lists every record that failed SPF or DKIM, with what the receiver did with it. Its first line counts every message that failed DMARC in the range, forwarded mail included, and splits it into delivered, forwarded and blocked; everywhere else a bare "failed" count leaves forwarded mail out:

<Screenshot name="failures" alt="The Failures tab of a domain: the messages that failed DMARC and what became of them, then each failing record, shown by kind" />

## Health

Health answers one question: **is the delivered mail authentic?** It is based on *delivered compliance*: the share of delivered mail that passed DMARC. Mail the receiver blocked does not count against it.

| Health | Meaning |
| :--- | :--- |
| **Protected** | 99% or more of delivered mail passed DMARC. |
| **Warning** | 95% to 99%. |
| **Critical** | Below 95%. |
| **All blocked** | Mail was seen, nothing was delivered: every message failed DMARC and was blocked by policy (or forwarded). Delivered mail that passed, with every failure blocked, is **Protected**. |
| **No data** | No reports in the selected time range. |

A report's page shows its health as a badge. The dashboard shows the delivered compliance it is based on, and the attention list names the domains whose delivered failures reach a warning or failing rate.

Health says nothing about DNS. A domain can be **Protected** and still have a poor DNS grade, for example with `p=none`. The attention list brings both together.

## When to move to the next policy step

A domain's page says under its name whether the domain is ready for the next DMARC policy, from `p=none` to `p=quarantine` and then to `p=reject`, and its **Enforcement** tab gives the reason and the record to publish. How the verdict is reached, and what `pct=` and `t=y` change about it, is on [Policy readiness](./readiness).

## The attention list

One row per domain with every open issue, ranked by severity rather than by mail volume, so a problem on a small domain is not hidden behind a large one. The dashboard shows the first 10; with more, a line says how many more need attention and opens the Domains page on the same list: only the domains with an open issue, in the same order, with the issues first and every page of them. A **Needs attention** chip above the table says so; clear it for the whole list.

| Issue | When it is raised |
| :--- | :--- |
| `delivered_failures` | Mail failed DMARC and was still delivered, at a warning or failing rate. |
| `dns_grade` | The DNS grade fails, or there is a critical DNS finding. |
| `not_enforcing` | The policy is `p=none`: spoofed mail would be delivered. |
| `parked_not_rejecting` | A parked domain does not publish `p=reject`. |
| `parked_passing_mail` | Mail passed DMARC for a parked domain: something sends as it. |
| `dkim_coverage` | Too little of the passing mail is signed by the domain itself. |
| `reports_stopped` | Reports arrived before and then stopped: none for the days of the domain warning (**Settings → Mailbox**, 14 by default). |

A parked domain's `parked_not_rejecting` badge names the policy it has, or **No record**. The same issue names appear in the `issues` column of `domains.csv` (see [Exports](./exports)).

<Screenshot name="domains" alt="The domains list with DNS grade (A to F), policy, messages, delivered compliance, delivered failures and last report per domain" />

## DKIM coverage

The share of DMARC-passing mail that is signed by the domain itself. The rest passed on SPF alone, unsigned or signed only by another domain, and SPF fails as soon as mail is forwarded. Coverage is judged like every other share: from 99% fine, from 95% worth a look, below that a problem. It is shown per domain and per sending IP.

## Outcomes

Counts and charts split mail into four outcomes: **DMARC pass**, **Blocked by policy**, **Forwarded** and **Delivered failures**. They always appear as the same legend, the four with their message counts: under every chart of messages per day (on the dashboard with the change against the previous range), under a report's health and in each domain opened on a sender's page. The Organizations list shows them as one bar per organization, with the counts in its tooltips. See [What happened to the mail](./how-it-works#what-happened-to-the-mail).

## The status in the header

The header shows the state of the import: when the mailbox was last polled, whether the last poll failed, how many reports were skipped, and whether reports have stopped arriving for longer than the mailbox warning (**Settings → Mailbox → Report arrival**). It also has **Sync now**.
