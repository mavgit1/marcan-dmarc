# Client reports

If you look after domains for others, Marcan can send each [organization](./domains) a short email once a month about its domains: whether they are protected, how their mail fared, what to do next, and what changed. Your clients never need to open Marcan for it, and they see only their own domains.

Client reports are **experimental**: what the email contains, and how it is worded, may still change between versions.

## What the email contains

One email per organization, sent to everyone listed for it. Its length follows what there is to say: a quiet month is four short lines and the sign-off.

1. **The verdict**: "All 12 domains are protected." when no domain has an open issue, else "3 of 12 domains need attention.", in the colour of the worst issue.
2. **The month's mail**, for the previous calendar month, with the change against the month before: "In September 2026, receivers reported 12,480 messages, up 8.0% on August: 12,300 passed DMARC (up 8.2%), 150 were blocked by policy (down 9.6%), 20 were forwarded (unchanged) and 10 failed but were delivered (none the month before)." The changes are left out when the reports do not reach back over the month before.
3. **A block for each domain with something to say**: an open issue, a next policy step that can be taken (ready for it, or raise `pct` to 100 first), or a change in the month. The block has only the lines that apply:

| Line | What it says |
| :--- | :--- |
| Attention | The domain's open issues, such as "Not enforcing (p=none)" or "Failing DNS grade". |
| Compliance | With delivered failures only: the share of delivered mail over the last 30 days that passed DMARC, with the number of delivered messages (blocked and forwarded mail is in neither). |
| Next step | [Policy readiness](./dashboard#when-to-move-to-the-next-policy-step): ready for the next step, not ready yet and why, or raise `pct` to 100 first, in the same words as the domain's **Enforcement** tab. Nothing once a domain is at `p=reject` (or is parked). |
| Impact | What the next step would act on: "Moving to p=quarantine would affect 141 messages from 3 senders over the last 30 days.", the mail that failed DMARC and was not forwarded, and how many sending IP addresses it came from, as the domain's Enforcement tab lists them. |
| DNS grade | The domain's [DNS grade](./dns-checks#the-dns-score), only when it does not pass. Left out until the domain has been checked, and while a newer Marcan version re-checks it. |
| Changes | How many [changes](./alerts) the month brought, with the three that matter most, worst first. |

4. **The other domains on one line**: "9 domains are protected and unchanged: a.example, b.example, …".
5. **The parked domains on one line**: "2 parked domains publish p=reject.", or the ones that do not: "Parked domains should publish p=reject; old.example does not."

Two periods:

- **The month's mail and changes** cover the **previous calendar month**: a report sent on 1 October covers September.
- **Issues, compliance and the next step** are judged on the **30 days before sending**, like the domain page.

The email holds only verdicts Marcan is sure of. It names no senders, no IP addresses and no reporters: the impact line counts senders, it does not list them. It is plain text with a simple HTML version, and it goes out through your [Sending](./mailbox#sending) account.

## Who gets it

**Settings → Client reports → Recipients** holds the recipients as text, one `organization,email` per line:

```csv
Acme,it@acme.example
Acme,ceo@acme.example
Globex,dmarc@globex.example
```

- An organization may have several recipients, and one address may get several organizations' reports (one email each).
- Organizations are matched by name, ignoring case. The organization must already exist: add it in the [Domains list](./domains#domains-as-text) (**Settings → Domains**) or on the Organizations page. A name that matches none is an error with a suggestion ("did you mean Acme?"); it never creates an organization.
- Quote names that contain a comma: `"Acme, Inc",it@acme.example`. An optional `organization,email` header line is skipped, and `#` starts a comment.
- Organizations that are not listed get no report.

**Unlike the Domains list, this list is the full truth.** Saving replaces every recipient, so a line you take out stops that report. Before you save, the editor says what will change, line by line: "Starts sending to it@acme.example for Acme", "Stops sending to old@acme.example for Acme", or "No changes". Nothing is saved while a line has an error.

Deleting an organization removes its recipients too.

## When it is sent

Under **Settings → Client reports**. The monthly reports are **off by default**. The switch at the top of the page, **Send client reports**, is saved with the rest of the page, the recipient list included; switched on, **Save** waits until the list has someone ("Add at least one recipient to the list first."). To empty the list, switch the reports off in the same save. On, every listed organization gets its report once a month. Sending must be on too: while it is off, the line under the switch says so. Off, the fields and the list below the switch are greyed out and keep their values.

| Field | Notes |
| :--- | :--- |
| Day of the month | 1 to 28, so every month has it. The 1st by default. |
| Time (UTC) | The hour; reports go out on the hour. 07:00 by default. |
| Reply-To | Where your clients' replies go. Empty: the Reply-To of Sending (or the sender address). |

Marcan looks every hour. The reports go out on the set day from the set hour, one email every 5 seconds at most (Marcan's limit for all its mail), so many recipients take a few minutes; if Marcan was not running then, or sending was not ready, it tries again until a day later, then skips that month. Marcan records each email as it goes out, so nobody gets a month's report twice, also when Marcan restarts in the middle of a send. Once any report of a month has been tried, the recipients who do not have it yet (Marcan stopped before their turn) are tried on the next hourly run, until the next month's report time; a recipient added to the list in that time gets it on the next hourly run too.

An email that does not go out is recorded with its cause, and what happens next depends on the failure:

| Failure | Examples | What Marcan does |
| :--- | :--- | :--- |
| Passing | The mail server unreachable or busy (a 4xx answer), a timeout, a login to fix under Sending | Tries again every hour for 3 days from the first failure, then gives up for the month. |
| Permanent | The server refused the address or the message for good (a 5xx answer): the mailbox does not exist, the address is refused, a policy refusal | Does not try again this month. Fix the address in the list; the next month's report goes to it. |

Under the send line the page names them until the next report time, in red once one is given up or refused, else in amber: "2 of 4 reports not delivered: x@example.com (mailbox does not exist), ops@example.net (server unreachable, retrying until 4 November)."

If the mailbox has not been read successfully for 3 days when the reports are due, none goes out: with the import broken, every report would say "No reports" for your clients' domains. The page says "Held: the mailbox has not been read since 28 October 2026. Reports go out once it is read again.", and the month's reports go out on the first hourly run after the mailbox is read again, even days after the report day, still once.

The line under the switch says when the reports go out next, to how many organizations, and when one last went out: "Next send: 1 November 2026, 4 organizations. Last sent: 1 October 2026." While some recipients still wait for this month's report (not tried yet, or tried again after a passing failure), the next send is that report time and the count is the organizations still waiting. From three days before, an organization's page says so too, "Client report goes out on 1 November 2026 to 2 recipients.", with a **Preview** button; nothing while the reports or sending are off, or when the organization is not in the list.

## Preview

Each organization has a **Preview** button: it shows that organization's report in Marcan, as it would go out now, with its subject and sender name, the HTML version and the plain text one click away. Nothing is sent, and it works with sending off.

Beside it, a **Send a preview** button: it sends that organization's report, as it would go out now, to the test recipient set under [Sending](./mailbox#sending) (the sender address unless you set one), with "[Preview]" in the subject; the line under the list says where it goes. The organization does not need to be listed. It uses the Reply-To on the page, saved or not; the switch and sending must be on.

## Your name and the notice

The email opens with who it is from, then its title, such as "DMARC report for Acme, September 2026". Who it is from depends on the [operator key](./license#what-a-key-changes):

- Without one, the first line names Marcan as free software, "Marcan, free software" (with Marcan's small mark in the HTML version). Under it, smaller, your letterhead from **Settings → Branding**: your logo, if you uploaded one, and "Managed by Acme IT". Without an operator name that line is left out.
- With one, your letterhead alone: your logo at full size and your name in bold. Nothing names Marcan.

The logo and the mark go in the HTML version, attached to the email; the plain-text version has the words only. The logo keeps its shape and is never enlarged: it is shown at most 200 by 64 pixels with an operator key and 120 by 32 without, so a wide logo with its name in it reads best. Its size is read from the image when you upload it and written into the email, so desktop Outlook, which ignores sizes set by style, shows it at that size too. The sender name follows the same rule: "Acme IT via Marcan" without an operator key, "Acme IT" with one, and "Marcan" without an operator name; the address stays the sender address.

If you run Marcan for clients, the [license](./license) asks you to tell them it is free software they could run themselves, unless you have a license. These emails end with that notice, as every email Marcan sends does, signed with your name: "Sent by Acme IT with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: …". With a valid operator key the notice is off and the report ends in "Sent by Acme IT." (see [The operator key and the notice](./license#the-operator-key-and-the-notice)). The notice in the reports does not replace telling a client at sign-up: the first report goes out with the next month, often weeks after the client signed up, and without a license the client must be told by then.
