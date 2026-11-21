---
description: What is planned for Marcan and what has shipped, without dates, from alerts, client reports and accounts to TLS-RPT reports and a stable 1.0.
---

# Roadmap

Ideas and plans, not promises. Nothing here has a date. If one of them matters to you, say so in an [issue](https://github.com/mavgit1/marcan-dmarc/issues): that decides the order.

## Alerts

**Shipped, experimental**: which changes are alerted, and how they are worded, may still change. Marcan alerts when something changes about a domain (a DNS record or the DMARC policy in DNS changes, health changes, delivered failures open or clear, reports stop or resume, a domain becomes ready for its next policy step, a domain is marked as parked), never again while a state persists. Changes are listed in the UI and sent by email through the account chosen under **Settings → Sending**, with an optional daily or weekly digest. See [Alerts](./alerts).

- **A read-only status endpoint** for monitoring tools (open alerts, health per domain, pipeline status), served apart from the admin UI.

## Client reports

**Shipped, experimental**: what the email contains, and how it is worded, may still change. Each organization can get a short monthly email about its own domains, with your name and logo in the letterhead (under Marcan's line without an [operator key](./license#what-a-key-changes)). See [Client reports](./client-reports).

## Accounts

**Shipped.** Reports arrive in any number of mail accounts, each with its own address, folder and any number of forwarded addresses, and mail goes out through one of them. Microsoft 365 / Exchange Online mailboxes are read and sent from over Microsoft Graph with an app registration (OAuth 2.0 with a certificate). See [Accounts](./accounts).

## Reports

- **TLS-RPT reports** (RFC 8460): failures of encrypted delivery to your domains, next to the DMARC data.
- **Reprocessing** stored reports from their original XML when processing improves.

## Retention

**Shipped.** Reports and alerts older than the retention period (12 months by default, **Settings → Data → Retention**) are deleted once a day. See [Retention](./backup#retention).

## Stability

- **1.0**: a stable database schema with supported upgrades only, and a documented, stable JSON API.
