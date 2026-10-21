# Alerts

Marcan already decides what needs attention. Alerts tell you when that **changes**, and only in the ways you act on: a record in DNS is edited, the DMARC policy in DNS changes, a domain's health changes, delivered failures open or clear, reports stop or resume, a domain becomes ready for its next policy step, you mark a domain as parked, or a sender new to a domain fails DMARC as it.

Alerts are **experimental**: which changes are alerted, and how they are worded, may still change between versions.

Marcan always looks for changes and lists every one under **Alerts** in the sidebar, newest first, narrowed by the organization, domain and time range in the header like every other list. You do not need to set up sending or switch anything on to see them. Each domain page shows its last five changes under **Recent changes**. The settings under **Settings → Alerts** (the **Send alerts** switch and the alert kinds) decide only what is emailed.

<Screenshot name="alerts" alt="The Alerts list, newest first: a DMARC record published, a DMARC tag changed and an SPF include added, each with the domain and when it was found" />

## Changes only

An alert is always "this changed": Marcan compares each domain with how it stood the last time it looked. A state that stays the same is never alerted again, however long it lasts: a domain at `p=none` is alerted once, when it drops to `p=none`, not every day it stays there.

- Marcan looks at every monitored domain **every 15 minutes**, right after the DNS checks of that round (also after the quicker round that checks a new domain). At startup it first waits for the first round, at most 10 minutes.
- **Not while the mailbox imports.** While a mailbox pass runs (or before the first one since startup has ended), Marcan does not look: reports land in any order then. The first look after an import takes the reports of the domains that got reports since Marcan last looked at them as their new starting point, so an import never raises "opened" or "resolved"; DNS changes still count. Marcan stores that it held back, so this also holds when it restarts in the middle of an import.
- The **first look at a domain records where it stands** and raises nothing, so installing Marcan or adding a domain never floods you. The same goes for a domain's first reports and its first DNS result: something that was never measured cannot have changed.
- Report figures are judged over the **last 30 days**, like policy readiness, whatever time range the page shows.
- A DNS lookup that times out is never a change, and neither is a DNS result of a newer Marcan version, whose rules may score the same records differently: that result becomes the new starting point.
- **Marking a domain as parked** (or no longer parked) is one alert, "Marked as parked" or "No longer parked". The domain did not change, only what it is judged against: a parked domain has other issues (`p=reject` expected, no passing mail) than one that sends mail (`p=none`, DKIM coverage). Its issues, health and readiness start over from that look instead of each raising an alert of their own. Changes to its records in DNS still count.
- The DMARC policy is read from DNS only. The policy in a report is what one receiver saw at the time, and receivers report the same record differently (one fills in `sp`, another leaves it out), so two reports that disagree are never a change.

## What is alerted

Every kind of alert, as **Settings → Alerts → Alert kinds** lists it (the wording is Marcan's own, from the same list):

| Group | Alert | When |
| :--- | :--- | :--- |
| Issues | New issue: delivered failures | An issue opened: mail that failed DMARC is delivered at a warning or failing rate. |
|  | Resolved: delivered failures | That issue cleared: failing mail is no longer delivered at that rate. |
|  | New issue: reports stopped | An issue opened: reports for the domain arrived before and then none for the days of the domain warning (Settings → Mailbox). |
|  | Resolved: reports stopped | That issue cleared: reports for the domain arrive again. |
| Health | Health changed | The domain's health moved between two measured states (to or from no data is not alerted). |
| DNS records | DMARC record changed | A DMARC record was published or removed, or its rua or alignment changed (a new p, sp or pct is a DMARC policy change). |
|  | SPF record changed | An SPF record was published or removed, terms were added or removed, or the lookup count moved. |
|  | DKIM key changed | A DKIM key was published, changed, revoked or removed under a selector checked before. |
|  | MTA-STS changed | An MTA-STS record was published or removed, its id changed, or the policy's mode or MX hosts changed. |
|  | TLS-RPT record changed | A TLS-RPT record was published or removed, or its report address changed. |
|  | BIMI record changed | A BIMI record was published or removed, or its logo changed or was declined. |
| Policy | DMARC policy changed | The DMARC record in DNS changed its p, sp or pct (other tags changed with them are named too). |
| Readiness | Ready for the next policy step | The domain became ready for its next policy step: quarantine, then reject. |
| Parked | Marked as parked | The domain was marked as parked: one alert, and its issues, health and readiness start over from there instead of each raising one. |
|  | No longer parked | The domain is no longer marked as parked: one alert, and its issues, health and readiness start over from there instead of each raising one. |
| Senders | New failing sender | A sender that sent nothing as the domain before the last 30 days had at least 10 messages fail DMARC in them (forwarded mail not counted). |

Each change is one sentence that names the numbers it is based on, the same in the app and in the email, and has a colour, the colour of where it led:

| Change | Example | Colour |
| :--- | :--- | :--- |
| Delivered failures open | 12 delivered failures in the last 30 days (1.4% of delivered mail, threshold 1%). | The issue's own: amber above 1% of delivered mail, red above 5% |
| They clear | Delivered failures resolved: 1 delivered failure in the last 30 days (0.1% of delivered mail, threshold 1%). | Green |
| Reports stop | No reports for 9 days. (Threshold 7 days.) | Amber |
| Reports resume | Reports are arriving again after 12 days without any. (Threshold 7 days.) | Green |
| Health changes | Health changed from Protected to Critical: 30 delivered failures in the last 30 days (7.5% of delivered mail, threshold 5%). | The new health |
| A DNS record changes | SPF record changed: added include:mailgun.org; DNS lookups 7 → 8. | The check's worst finding when it is a warning or worse, otherwise none |
| The DMARC policy in DNS changes | DMARC policy changed: p reject → none, pct 100 → 50. | Green when tightened, amber when loosened, red when spoofed mail would now be delivered |
| Ready for the next policy step | Ready for p=quarantine: 30 days and 1,204 messages under p=none, 99.6% delivered compliance. | Green |
| Marked as parked, or no longer | Marked as parked. | None |
| A new sender fails | New sender 203.0.113.5 (Example Mail) failed DMARC with 42 messages in the last 30 days (12 delivered) and sent nothing as the domain before. A new system sending as the domain, or spoofing: check its sender page. | Red when any of its failing mail was delivered, amber when the policy blocked all of it |

The mail figures cover the last 30 days, the window [policy readiness](./readiness) is judged on; the threshold is the share of delivered mail failing DMARC above which the colour applies (delivered compliance below 99% is amber, below 95% red). The days without reports are measured against the domain warning under **Settings → Mailbox** (14 days by default). A DNS record change names what changed in the record (the tags, terms, selector or address before and after).

Two issues of the dashboard's attention list are alerted when they open and clear: delivered failures and stopped reports. The others (a failing DNS grade, `p=none`, a parked domain without `p=reject` or with passing mail, missing DKIM coverage) stay on the dashboard and in the digest's open issues, without alerts of their own: most follow from a change that is alerted itself (a DNS record, the policy, marking the domain parked). The DNS grade moving is not alerted either: health and the record changes cover it. Health moving to or from "no data" is not alerted; the reports-stopped issue covers that.

## New failing senders

A sender is a source IP, as on the [Senders page](./dashboard#pages). It is **new** for a domain when none of the domain's reports that began before the last 30 days has mail from it; it alerts once it has **at least 10 messages that failed DMARC** for the domain in those 30 days. Forwarded mail is not counted (it is legitimate mail broken on the way, not spoofing); mail blocked by policy is. The sentence names the sender (its name when Marcan knows one), how many messages failed and how many of them were delivered, and the alert email links to its sender page when the base URL is set.

- **Once per domain and sender.** A sender that keeps failing is not alerted again. After 30 days it is no longer new, and the alert has done its job; no "resolved" follows.
- **No flood.** Like every report-derived alert, the senders start from a baseline: the first look at a domain, its first reports and the first look after a mailbox import take the new senders as they are, without an alert each. So does the first look after upgrading to the version that added this alert.
- It may mean a new system sending as the domain that is not set up yet (a newsletter tool, a CRM, a new mail provider: add it to SPF and sign with DKIM), or someone spoofing the domain. Its sender page helps tell which: who operates the IP, every domain it sent as, and how SPF and DKIM fared.
- Client reports leave it out: it points to Marcan's sender pages, which your clients do not see.

DNS record changes compare the parsed records, so a record rewritten with other spacing, case or term order is the same record:

| Check | What counts as a change |
| :--- | :--- |
| DMARC | Published or removed; `rua` or the alignment modes changed (each shown before and after). A change of `p`, `sp` or `pct` is one DMARC policy change instead, with the other tags that changed along with it. |
| SPF | Published or removed; terms added or removed, or the total DNS lookups changed (an include you use changing its own record shows here too). |
| DKIM | For each selector checked both times: a key published, changed, revoked or removed. A selector checked for the first time is not a change. |
| MTA-STS | Published or removed; the record's `id` changed, or the policy's mode or MX hosts changed (only when the policy file was read both times). |
| TLS-RPT | Published or removed; the report address changed. |
| BIMI | Published or removed; the logo changed or was declined. |

## Alert emails

Under **Settings → Alerts**. Alert emails are **off by default**. The switch at the top of the page, **Send alerts**, is saved with the rest of the page; switched on, **Save** waits until there is a recipient ("Add at least one recipient first."). On, every round that finds changes sends **one email** with every change since the last one, grouped by domain, worst first, and the digest goes out when it is set. Off, no alert email of any kind goes out, and the fields below the switch are greyed out and keep their values. Mail goes out through the [Sending](./mailbox#sending) account, so sending must be on too: while it is off, the line under the switch says so.

| Field | Notes |
| :--- | :--- |
| Recipients | One address per line. Each recipient gets their own copy; Marcan sends at most one message every 5 seconds, so many recipients take a while. |
| Digest | Off, daily or weekly, at the hour you choose (UTC), on the day you choose for weekly. Off by default; the hour is 07:00 and the day Monday by default. |
| Base URL | The address you open Marcan at, such as `https://marcan.example.com`. With it, the emails link to each domain and to the alert list; without it, they have no links. |
| Alert kinds | Every kind of alert above by group, each with an **Email** checkbox (and Email all / Email none per group); all are on by default, and a kind added in a later version arrives on. Folded to one line ("All 16 emailed", "13 of 16 emailed") until you open it; the alert list's **What creates an alert** link opens it. An unticked kind is left out of the alert emails and of the changes in the digest; the **Alerts** list still shows it. |

**Send a test** sends made-up changes on `example.com` to the test recipient set under [Sending](./mailbox#sending) (the sender address unless you set one), not to the recipients, with the base URL on the page, saved or not, through the Sending account (the switch and sending must be on). The line under the button says where it goes. When it fails it says why, like the sending test.

A change is mailed once. When sending fails (the server cannot be reached, the login is rejected), the changes wait for the next round; changes more than a day old are no longer mailed, so turning alerts on, or fixing sending after a long outage, never sends a backlog. They stay in the list either way, until they are older than the [retention period](./backup#retention).

## The digest

The digest lists what changed since the last digest and every domain's open issues now. It goes out **even when nothing changed**: "nothing changed" is worth knowing too, and a digest that stops arriving tells you something is wrong with Marcan or its mail.

Switching the digest on starts its period: the first one goes out at the next digest time, not at once. The digest is an alert email too: with alert emails off, no digest goes out.

Alert emails and digests open like every email Marcan sends: "Marcan, free software" and under it "Managed by Acme IT" with your logo (**Settings → Branding**). Then comes the title, such as "3 changes on example.com" or "Daily digest for 2026-10-14". They end with the [free-software notice](./license), "Sent by Acme IT with Marcan, free software …", as every email Marcan sends does. An operator key changes the top, the sender name and the last line: see [What a key changes](./license#what-a-key-changes).

## Security

The emails contain domain names, figures and DNS records of your domains, nothing else. Like every mail Marcan sends, they go through your own SMTP account. With sending on, anyone who can open Marcan can make it send the test alert to the test recipient: see [Security](./security#sending-mail).
