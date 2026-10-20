# How it works

## The pipeline

```
mailbox → fetch → parse → store → assess → dashboard
                            ↑
                  live DNS checks
```

1. **Fetch.** Marcan polls one IMAP folder (every 5 minutes by default) and reads new messages. It never changes the mailbox: the folder is opened read-only and messages are read without marking them as seen. Progress is a cursor per mailbox and folder, so the same message is not imported twice, and switching to another mailbox starts that one from its first message.
2. **Parse.** Report attachments may be XML, gzip or zip. Other attachments (office files, TLS-RPT reports) and mail without attachments are skipped silently.
3. **Store.** Each report is stored with its records and the original XML (compressed), so a later release can process them again.
4. **Assess.** Every verdict on the dashboard is decided on the server from the stored figures and the latest DNS check: health, the attention list, DKIM coverage, policy readiness and the DNS grade. The UI only words and colours those verdicts; the [alerts](./alerts) and [client reports](./client-reports) use the same ones.

DNS checks run in the background for every monitored domain, about once an hour, and whenever you press **Re-check DNS**.

## Marcan checks alignment itself

A report states what the receiver concluded, but receivers differ in how they apply DMARC. Marcan recomputes alignment for every record from the raw authentication results, using the Public Suffix List:

- A record **passes DMARC** when SPF or DKIM passed for a domain aligned with the `From:` domain (relaxed or strict, as the domain's `adkim=` / `aspf=` says).
- SPF counts the `MAIL FROM` result. A `HELO` result counts only when the record has no other SPF result (RFC 7489 §4.1).

The receiver's own verdict is kept and shown next to Marcan's (as `reporter_spf_verdict` and `reporter_dkim_verdict` in the [exports](./exports)).

## What happened to the mail

Every record has one of four **outcomes**, and every count, chart, filter and export uses the same four:

| Outcome | Meaning |
| :--- | :--- |
| **DMARC pass** | DMARC passed. |
| **Blocked by policy** | DMARC failed and the receiver quarantined or rejected the mail, as the policy asks. |
| **Forwarded** | DMARC failed in transit, but the receiver delivered it because it trusted the forwarder: the report gives a `forwarded`, `mailing_list` or `trusted_forwarder` reason, or notes a verified ARC chain (`arc=pass`). |
| **Delivered failures** | DMARC failed and the mail was still delivered. These are the ones to look at. |

Separating forwarded mail matters: mailing lists and forwarding break SPF, and without the split they look like spoofing.

## Sender identity

Reports name senders only by IP address. Marcan names the operator behind an IP from evidence, in this order:

1. forward-confirmed reverse DNS (the IP's PTR name resolves back to the IP),
2. a domain whose SPF passed for that IP in your reports,
3. unconfirmed reverse DNS, marked as such.

There is no built-in list of provider IP ranges. Names are captions only and never change a verdict. You can rename operators or name your own servers: see [Sender names](./sender-names).

<Screenshot name="sender" alt="A sender's page: the operator named from reverse DNS, and every domain it sent for" />

## Thresholds

Shares of good mail (delivered compliance, DKIM coverage, a sender's pass rate) are judged by one rule:

| Share | Status |
| :--- | :--- |
| 99% or more | fine |
| 95% to 99% | worth a look |
| below 95% | a problem |

A domain whose reports stop arriving is flagged after the days of the domain warning (**Settings → Mailbox**, 14 by default). The DNS grade is separate and rates DNS configuration only; see [DNS checks](./dns-checks).
