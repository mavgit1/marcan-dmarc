# Alerts

Marcan already decides what needs attention. Alerts tell you when that **changes**, and only in the ways you act on: a record in DNS is edited, the DMARC policy in DNS changes, a domain's health changes, delivered failures open or clear, reports stop or resume, a domain becomes ready for its next policy step, you mark a domain as parked, or a sender new to a domain fails DMARC as it. Three are about Marcan itself: a newer release, a mail account's Microsoft 365 certificate about to expire, and a receiving account that has not been read for 3 days.

Alerts are **experimental**: which changes are alerted, and how they are worded, may still change between versions.

Marcan always looks for changes and lists every one under **Alerts** in the sidebar, newest first, narrowed by the organization, domain and time range in the header like every other list. You do not need to set up sending or switch anything on to see them. Each domain page shows its last five changes under **Recent changes**. The settings under **Settings → Alerts** (the **Send alerts** switch and the alert kinds) decide only what is emailed.

<Screenshot name="alerts" alt="The Alerts list, newest first: reports stopping, a new sender failing DMARC, health and DNS changes, each with the domain, its weight and when it was found" />

## Changes only

An alert is always "this changed": Marcan compares each domain with how it stood the last time it looked. A state that stays the same is never alerted again, however long it lasts: a domain at `p=none` is alerted once, when it drops to `p=none`, not every day it stays there.

- Marcan looks at every monitored domain **every 15 minutes**, right after the DNS checks of that round (also after the quicker round that checks a new domain). At startup it first waits for the first round, at most 10 minutes.
- **Not while an account imports.** While a pass over one of the [accounts](./accounts) runs for more than a minute, while an import is unfinished (a long import gave way to another account, was stopped by a settings change, or failed part-way and goes on shortly; a re-import waits for its pass), or before every receiving account has had its first pass since startup, Marcan does not look: reports land in any order then. The first look after an import takes the reports of the domains that got reports since Marcan last looked at them as their new starting point, so an import never raises "opened" or "resolved"; DNS changes still count. Marcan stores that it held back, so this also holds when it restarts in the middle of an import.
- The **first look at a domain records where it stands** and raises nothing, so installing Marcan or adding a domain never floods you. The same goes for a domain's first reports and its first DNS result: something that was never measured cannot have changed.
- Report figures are judged over the **last 30 days**, like policy readiness, whatever time range the page shows.
- A DNS lookup that times out is never a change, and neither is a DNS result of a newer Marcan version, whose rules may score the same records differently: that result becomes the new starting point.
- **Marking a domain as parked** (or no longer parked) is one alert, "Marked as parked" or "No longer parked". The domain did not change, only what it is judged against: a parked domain has other issues (`p=reject` expected, no passing mail) than one that sends mail (`p=none`, DKIM coverage). Its issues, health and readiness start over from that look instead of each raising an alert of their own. Changes to its records in DNS still count.
- The DMARC policy is read from DNS only. The policy in a report is what one receiver saw at the time, and receivers report the same record differently (one fills in `sp`, another leaves it out), so two reports that disagree are never a change.

## What is alerted

Every kind of alert, as **Settings → Alerts → Alert kinds** lists it (the wording is Marcan's own, from the same list). How each is counted and kept from repeating is told below the tables and in the sections after them.

| Group | Alert | When |
| :--- | :--- | :--- |
| Issues | New issue: delivered failures | More than 1% of the domain's delivered mail fails DMARC. |
|  | Resolved: delivered failures | Delivered failures fall back under 1% of delivered mail. |
|  | New issue: reports stopped | Reports for the domain stop for longer than the domain warning under Settings → Accounts. |
|  | Resolved: reports stopped | Reports for the domain arrive again. |
| Health | Health changed | The domain's health moves to another state. |
| DNS records | DMARC record changed | A DMARC record is published or removed, or its rua or alignment changes. |
|  | SPF record changed | An SPF record is published or removed, or its terms or lookup count change. |
|  | DKIM key changed | A DKIM key is published, changed, revoked or removed. |
|  | MTA-STS changed | An MTA-STS record or policy is published, removed or changed. |
|  | TLS-RPT record changed | A TLS-RPT record is published or removed, or its report address changes. |
|  | BIMI record changed | A BIMI record is published or removed, or its logo changes. |
| Policy | DMARC policy changed | The DMARC policy in DNS changes: p, sp, pct or testing mode (t=y). |
| Enforcement | Ready for the next policy step | The domain becomes ready for p=quarantine, then for p=reject. |
| Parked | Marked as parked | The domain is marked as parked. |
|  | No longer parked | The domain is no longer marked as parked. |
| Senders | New failing sender | A sender new to the domain fails DMARC with at least 10 messages in the last 30 days. |
| Updates | Update available | The daily version check finds a newer Marcan release. |
| Accounts | Microsoft certificate expiring | An account's Microsoft 365 certificate expires within 30 days, or has expired. |
|  | Account not read | A receiving account has not been read for 3 days. |

Each change is one sentence that names the numbers it is based on, the same in the app and in the email, and has a colour, the colour of where it led:

| Change | Example | Colour |
| :--- | :--- | :--- |
| Delivered failures open | 12 delivered failures in the last 30 days (1.4% of delivered mail, threshold 1%). | The issue's own: amber above 1% of delivered mail, red above 5% |
| They clear | Delivered failures back under 1%: 1 in the last 30 days (0.1% of delivered mail). | Green |
| Reports stop | No reports for 9 days (threshold 7 days). | Amber |
| Reports resume | Reports are arriving again after 12 days without any. | Green |
| Health changes | Health changed from Healthy to Critical: 30 delivered failures in the last 30 days (7.5% of delivered mail, threshold 5%). | The new health |
| A DNS record changes | SPF record changed: added include:mailgun.org; DNS lookups 7 → 8. | The check's worst finding when it is a warning or worse, otherwise none |
| The DMARC policy in DNS changes | DMARC policy changed: p=reject → p=none, pct=100 → pct=50. | Green when tightened, amber when loosened, red when spoofed mail would now be delivered |
| Testing mode turns on or off | DMARC policy changed: testing mode on (t=y). | Like a policy change: `t=y` under quarantine or reject loosens it as `pct=` below 100 does (amber), removing it tightens it (green) |
| Ready for the next policy step | Ready for p=quarantine: 30 days and 1,204 messages under p=none, 99.6% delivered compliance. | Green |
| Marked as parked, or no longer | Marked as parked. | None |
| A new sender fails | New sender 203.0.113.5 (Example Mail) failed DMARC with 42 messages in the last 30 days (12 delivered) and sent nothing as the domain before. A new system sending as the domain, or spoofing: check its sender page. | Red when any of its failing mail was delivered, amber when the policy blocked all of it |
| A newer release | Marcan 0.3.2 is available. | Blue (information) |
| A Microsoft certificate expires | The Microsoft certificate of account “Reports” expires on 20 November 2026. | Blue (information) |
| A Microsoft certificate has expired | The Microsoft certificate of account “Reports” expired on 20 November 2026. | Amber |
| An account is not read | Account “Reports” has not been read since 20 November 2026. The last attempt failed: connection refused. | Amber |

The mail figures cover the last 30 days, the window [policy readiness](./readiness) is judged on; the threshold is the share of delivered mail failing DMARC above which the colour applies (delivered compliance below 99% is amber, below 95% red). The days without reports are measured against the domain warning under **Settings → Accounts** (14 days by default), and only for a domain that had reports before. When delivered failures clear, the sentence says where they stand now (back under the threshold), not the problem again. A DMARC policy change names each tag as the record writes it (`p=none → p=quarantine`, `no sp` for a tag the record leaves out). A DNS record change names what changed in the record (the tags, terms, selector or address before and after).

Two issues of the dashboard's attention list are alerted when they open and clear: delivered failures and stopped reports. The others (a failing DNS grade, `p=none`, a parked domain without `p=reject` or with passing mail, missing DKIM coverage) stay on the dashboard and in the digest's open issues, without alerts of their own: most follow from a change that is alerted itself (a DNS record, the policy, marking the domain parked). The DNS grade moving is not alerted either: health and the record changes cover it. Health moving to or from "no data" is not alerted; the reports-stopped issue covers that.

Delivered failures open and clear on the same delivered compliance health is judged on, so the two move together. When they do, **Health changed** is the one alert (it names the same figures), not a second "New issue" or "Resolved" beside it; delivered failures are alerted on their own only when health moved to or from "no data" (the first mail after a quiet month, say). If you mute **Health changed** but not the delivered-failures alerts, you are emailed about delivered failures only in that case.

## New failing senders

A sender is a source IP, as on the [Senders page](./dashboard#pages). It is **new** for a domain when none of the domain's reports that began before the last 30 days has mail from its **network**: the /24 of an IPv4 address, the /48 of an IPv6 one (large providers send from many neighbouring addresses, so a new address of a network that has sent for the domain for years is not new). It alerts once it has **at least 10 messages that failed DMARC** for the domain in those 30 days. Forwarded mail is not counted (it is legitimate mail broken on the way, not spoofing); mail blocked by policy is. The sentence names the sender (its name when Marcan knows one), how many messages failed and how many of them were delivered, and the alert email links to its sender page when the base URL is set.

- **Once per domain and network.** Several new addresses of one network failing are one alert, for the address with the most failures (its sentence names that address's own figures); the others, and any address of that network later, are not alerted again. A sender that keeps failing is not alerted again either. After 30 days it is no longer new, and the alert has done its job; no "resolved" follows.
- **No flood.** Like every report-derived alert, the senders start from a baseline: the first look at a domain, its first reports and the first look after an import take the new senders as they are, without an alert each. So does the first look after upgrading to the version that added this alert.
- It may mean a new system sending as the domain that is not set up yet (a newsletter tool, a CRM, a new mail provider: add it to SPF and sign with DKIM), or someone spoofing the domain. Its sender page helps tell which: who operates the IP, every domain it sent as, and how SPF and DKIM fared.
- Client reports leave it out: it points to Marcan's sender pages, which your clients do not see.

## New releases

The [version check](./upgrade#checking-for-new-versions) asks once a day for the newest release; when it names one newer than the release running, that is one **Update available** alert, linking its release notes in the list and in the emails. It is about the installation, not a domain: its Domain cell in the list is empty (—), the emails put it last under "Marcan", and narrowing the list to an organization or domain leaves it out, as do client reports.

- **Once per version.** The same answer the next day raises nothing; a later release raises a new alert. A release no newer than the running one never alerts, and neither does going back to an older image.
- Nothing while the check is off. The first look after upgrading to the version that added this alert records the newest release as its starting point, like the first look at a domain, so a release the header already names is not alerted.

## Microsoft certificates expiring

A Microsoft 365 account logs in with a certificate Marcan generated, valid for two years. Within **30 days** of its expiry date it raises one **Microsoft certificate expiring** alert (blue), and once the date has passed one more (amber).

- **Per account and expiry date.** Each stage alerts once for an account and a date. A new certificate with a later date starts over, so its own expiry is alerted again in time.
- Like a newer release, it is about Marcan itself: its Domain cell in the list is empty, and client reports leave it out.

## Accounts not read

A receiving [account](./accounts) that has not been read without error for **3 days** raises one **Account not read** alert. It names the account, since when it has not been read, and why, as far as Marcan knows: its credentials cannot be read ([locked](./accounts#locked-accounts)), or the error of its last attempt. A new account counts from when it was added, and so does one saved with another server, login or folder, or whose receiving was switched back on. Like a newer release, it is about Marcan itself, not a domain.

- **Once per outage.** It is not raised again while the account stays unread; once a pass reads the account, a later outage raises a new alert.
- The same 3 days hold the [client reports](./client-reports#when-it-is-sent): while any receiving account is not read, they wait, so the two always agree.

DNS record changes compare the parsed records, so a record rewritten with other spacing, case or term order is the same record:

| Check | What counts as a change |
| :--- | :--- |
| DMARC | Published or removed; `rua` or the alignment modes changed (each shown before and after). A change of `p`, `sp`, `pct` or `t` (testing mode on or off; `t=n` is the default) is one DMARC policy change instead, with the other tags that changed along with it. |
| SPF | Published or removed; terms added or removed, or the total DNS lookups changed (an include you use changing its own record shows here too). |
| DKIM | For each selector checked both times: a key published, changed, revoked or removed. A selector checked for the first time is not a change. |
| MTA-STS | Published or removed; the record's `id` changed, or the policy's mode or MX hosts changed (only when the policy file was read both times). |
| TLS-RPT | Published or removed; the report address changed. |
| BIMI | Published or removed; the logo changed or was declined. |

## Alert emails

Under **Settings → Alerts**. Alert emails are **off by default**. The switch at the top of the page, **Send alerts**, is saved with the rest of the page; switched on, **Save** waits until there is a recipient ("Add at least one recipient first."). On, every round that finds changes sends **one email** with every change since the last one, grouped by domain, worst first (a newer release last), and the digest goes out when it is set. Off, no alert email of any kind goes out, and the fields below the switch are greyed out and keep their values. Mail goes out through the [Sending](./accounts#sending) account, so sending must be on too: while it is off, the line under the switch says so.

| Field | Notes |
| :--- | :--- |
| Recipients | One address per line. Each recipient gets their own copy; Marcan sends at most one message every 5 seconds, so many recipients take a while. |
| Digest | Off, daily or weekly, at the hour you choose (UTC), on the day you choose for weekly. Off by default; the hour is 07:00 and the day Monday by default. |
| Base URL | The address you open Marcan at, such as `https://marcan.example.com`. With it, the emails link to each domain and to the alert list; without it, they have no links. |
| Alert kinds | Every kind of alert above by group, each with an **Email** checkbox (and Email all / Email none per group); all are on by default, and a kind added in a later version arrives on. Folded to one line ("All 19 emailed", "16 of 19 emailed") until you open it; the alert list's **What creates an alert** link opens it. An unticked kind is left out of the alert emails and of the changes in the digest; the **Alerts** list still shows it. |

**Send a test** sends made-up changes on `example.com` to the test recipient set under [Sending](./accounts#sending) (the sender address unless you set one), not to the recipients, with the base URL on the page, saved or not, through the Sending account (the switch and sending must be on). The line under the button says where it goes. When it fails it says why, like the sending test.

A change is mailed once. When sending fails (the server cannot be reached, the login is rejected), the changes wait for the next round; changes more than a day old are no longer mailed, so turning alerts on, or fixing sending after a long outage, never sends a backlog. They stay in the list either way, until they are older than the [retention period](./backup#retention).

## The digest

The digest lists what changed since the last digest and every domain's open issues now. It goes out **even when nothing changed**: "nothing changed" is worth knowing too, and a digest that stops arriving tells you something is wrong with Marcan or its mail.

Switching the digest on starts its period: the first one goes out at the next digest time, not at once. The digest is an alert email too: with alert emails off, no digest goes out.

Alert emails and digests open like every email Marcan sends: "Marcan, free software" and under it "Managed by Acme IT" with your logo (**Settings → Branding**). Then comes the title, such as "3 changes on example.com" or "Daily digest for 2026-10-14". They end with the [free-software notice](./license), "Sent by Acme IT with Marcan, free software …", as every email Marcan sends does. An operator key changes the top, the sender name and the last line: see [What a key changes](./license#what-a-key-changes).

## Security

The emails contain domain names, figures and DNS records of your domains, nothing else. Like every mail Marcan sends, they go through your own sending account (its SMTP server, or Microsoft Graph for a Microsoft login). With sending on, anyone who can open Marcan can make it send the test alert to the test recipient: see [Security](./security#sending-mail).
