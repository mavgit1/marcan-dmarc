# Releases

Newest first. Each release is tagged `vX.Y.Z` and the image `ghcr.io/mavgit1/marcan` carries the same tag; how to move to a new one is in [Upgrade](./upgrade). An entry says what changed and what, if anything, to do before upgrading.

## 0.3.0

Before upgrading: take a backup. This release has the first database migrations (new columns on the DNS snapshots) and re-checks every domain once; the default Compose stack moved to `deploy/caddy/`, see the first note. A database opened by 0.3.0 cannot be opened by 0.2.1 again.

- **The default Compose stack moved from `deploy/` to `deploy/caddy/`**, beside `deploy/authelia/` and `deploy/oauth2-proxy/`; the files changed only in their comments. If you downloaded the files into a directory of your own, as the guide says, nothing changes: fetch them from `deploy/caddy/` from now on. If you ran the stack from `deploy/` in a clone, Compose would take the new folder's name as the project name and start with a new, empty volume: before `docker compose up -d` in `deploy/caddy/`, move your `.env` (and `data/`, with a bind mount) there and add `COMPOSE_PROJECT_NAME=deploy` to `.env`, so the volume `deploy_marcan_data` is found again. See [Caddy with basic auth](./deploy-caddy#upgrade).
- The docs have a page per deployment stack ([Caddy](./deploy-caddy), [Authelia](./deploy-authelia), [OAuth2-Proxy](./deploy-oauth2-proxy)), each with every file, the `.env`, the login and the first start, and a page for [Policy readiness](./readiness), which moved out of Reading the dashboard. New screenshots of the setup, Settings, the Alerts list and a client report.
- A [Pricing](./pricing) page in the docs, with the two operator plans, the trial and what a key changes; the License page keeps the license and the key's mechanics.
- The emails' letterhead puts your logo left of your name in one row, vertically centred, instead of above it (with an operator key) or inline with the text (without one).
- DMARC checks read the tags of RFC 9989 (DMARCbis): `t=` and `np=` no longer show as unknown tags. `t=y` with an enforcing policy is critical, like `pct=0`, and policy readiness asks you to remove it before the next step. `pct=` below 100 and the obsolete `rf=` and `ri=` get an Info finding each. See [DNS checks](./dns-checks#dmarcbis-rfc-9989).
- Every domain is re-checked once after the upgrade (shown as **updating** until then). The database gains two columns; nothing to do before upgrading.
- `domains.csv` has two new columns at the end: `policy_source` (why the policy columns are empty, if they are) and `readiness` (the domain's policy readiness). The existing columns keep their positions. See [Exports](./exports#columns).
- A daily check for new versions under **Settings → License** (About), on by default: once a day the server fetches a small version file from marcan.kaitiaki.tech (GitHub Pages), carrying no data about the installation, and the page and the sidebar name a newer release. A database from 0.2.1 has no setting for it yet, so the check is on after the upgrade; switch it off there if the server should not reach the internet for it. See [Upgrade](./upgrade#checking-for-new-versions).
- New alert, **New failing sender** (Settings → Alerts, emailed by default like every new kind): a sender new to a domain fails DMARC for it with at least 10 messages in the last 30 days, forwarded mail not counted. Once per domain and sender; the first look after upgrading is its starting point, so the upgrade raises none. See [New failing senders](./alerts#new-failing-senders).
- Client reports read as figures: one line per domain with its policy, its messages, the share that passed DMARC and the failures delivered, blocked and forwarded, plus the step it can take now ("ready for p=quarantine") and the issues no figure shows. The per-domain blocks, the impact line and the "protected and unchanged" line are gone. See [What the email contains](./client-reports#what-the-email-contains).
- The client report recipients list works like the Domains list: it opens with every organization, `Acme,none` for one without a recipient, and saving changes only the organizations in the text (`none` stops an organization's reports). Before, saving replaced the whole list, so a line left out stopped that report. See [Who gets it](./client-reports#who-gets-it).
- The setup's first step opens with how reports reach Marcan, in four sentences. **Setup guide** under Tools is now **Setup**, and the foot of the sidebar shows the running version with a link to the docs.
- Settings → Client reports says, while Branding has no name and no logo, that the reports carry Marcan's letterhead until you set them, with a link to Branding.
- The dashboard's "more need attention" link opens the Domains page filtered to the domains with an open issue, in the attention list's order with their issues first, under a clearable **Needs attention** chip, instead of the list sorted by DNS grade. See [The attention list](./dashboard#the-attention-list).

## 0.2.1

First published release. 0.2.0 was tagged the same day but never built: two advisories against rustls-webpki and webpki-roots appeared between the tag and the public check, and this release carries the fixed versions. Otherwise identical.

## 0.2.0

Tagged, never built (see 0.2.1). First public release: IMAP import of aggregate reports, live SPF, DKIM, DMARC, MTA-STS, TLS-RPT and BIMI checks, policy readiness per domain, alerts and monthly client reports (both experimental), CSV exports.

Nothing to do before upgrading: there is no earlier release.
