---
description: How Marcan fetches, parses and stores DMARC aggregate reports, decides alignment itself, sorts mail into four outcomes and names senders from reverse DNS.
---

# How it works

## The pipeline

```
accounts → fetch → parse → store → assess → dashboard
                             ↑
                   live DNS checks
```

1. **Fetch.** Reports reach Marcan at the addresses of its [accounts](./accounts): each receiving account's address, and the addresses forwarded to it. Marcan checks each account's folder over IMAP, or a Microsoft 365 mailbox over Microsoft Graph (every 5 minutes by default), one account at a time, and reads new messages. It never changes a mailbox: over IMAP the folder is opened read-only and messages are read without marking them as seen, and over Microsoft Graph it may only read. Progress is a cursor per account and folder, so the same message is not imported twice, and changing an account's server, username or folder (a Microsoft 365 account's tenant, address or folder) starts it from its first message.
2. **Parse.** Report attachments may be XML, gzip or zip. Other attachments (office files, TLS-RPT reports) and mail without attachments are skipped silently.
3. **Store.** Each report is stored with its records and the original XML (compressed), so a later release can process them again.
4. **Assess.** Every verdict on the dashboard is decided on the server from the stored figures and the latest DNS check: health, the attention list, DKIM coverage, policy readiness and the DNS grade. The UI only words and colours those verdicts; the [alerts](./alerts) and [client reports](./client-reports) use the same ones.

DNS checks run in the background for every monitored domain, about once an hour, and whenever you press **Re-check DNS**.

## Marcan decides each record from the evidence

A report carries two things per record: the receiver's verdict, and the raw authentication results it rests on (which domain each DKIM signature was for, what SPF said for which domain). Marcan needs the raw results anyway to explain a record ("passed via DKIM only", "SPF passed for the forwarder's domain, not yours"), so it decides the verdict from the same evidence, with one rule for every reporter, using the Public Suffix List. The numbers then follow one definition instead of each receiver's reading, and the rare reporter mistake does not reach your counts:

- A record **passes DMARC** when SPF or DKIM passed for a domain aligned with the `From:` domain (relaxed or strict, as the domain's `adkim=` / `aspf=` says; RFC 9989 §4.4).
- SPF counts the `MAIL FROM` result (RFC 9989 §4.4.2). A `HELO` result counts only when the record has no other SPF result, as for a bounce: its `MAIL FROM` is empty, and SPF then checks `postmaster@` the HELO name (RFC 7208 §2.4).

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

A domain whose reports stop arriving is flagged after the days of the domain warning (**Settings → Accounts**, 14 by default). The DNS grade is separate and rates DNS configuration only; see [DNS checks](./dns-checks).

## Standards

| Document | What Marcan takes from it |
| :--- | :--- |
| RFC 9989 (DMARC) | Identifier alignment (§4.4) and the record's tags, `np=`, `t=` and `psd=` included (§4.7). |
| RFC 7489 (the DMARC it replaces) | `pct=`, `rf=` and `ri=`, as receivers that have not moved on still read them, and the organizational domain from the Public Suffix List. RFC 9989 finds it with a DNS tree walk instead (§4.10); Marcan keeps the PSL on purpose, since the two differ only for hosting and registry edge cases. |
| RFC 9990 (aggregate reports) | The report format (RFC 7489's older one is read too) and the authorization of report addresses on another organizational domain (§4). |
| RFC 9991 (failure reports) | The same authorization for `ruf=` addresses (§5). Failure reports themselves are not imported. |
| RFC 7208 (SPF) | The `MAIL FROM` identity (§2.4) and the limits of 10 DNS lookups and 2 void lookups (§4.6.4). |
| RFC 6376, 8301, 8463 (DKIM) | The key record, the minimum RSA key size (1024 bits, 2048 advised) and Ed25519 keys. |
| RFC 8461 (MTA-STS), RFC 8460 (TLS-RPT) | The DNS records and the MTA-STS policy file, fetched without redirects (RFC 8461 §3.3). |
| BIMI (Internet-Draft) | The assertion record, including the empty `l=` that declines BIMI. |
| RFC 6749 (OAuth 2.0), RFC 7523 (JWT client assertions) | A [Microsoft login](./accounts#microsoft-365): the client credentials grant (RFC 6749 §4.4), proved with an assertion signed by the account's certificate (RFC 7523 §2.2); the token goes to Microsoft Graph as a bearer token (RFC 6750) to read the mailbox and to send as it. |
