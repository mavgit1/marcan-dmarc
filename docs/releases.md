# Releases

Newest first. Each release is tagged `vX.Y.Z` and the image `ghcr.io/mavgit1/marcan` carries the same tag; how to move to a new one is in [Backup & upgrades](./backup#upgrade). An entry says what changed and what, if anything, to do before upgrading.

## 0.2.1

First published release. 0.2.0 was tagged the same day but never built: two advisories against rustls-webpki and webpki-roots appeared between the tag and the public check, and this release carries the fixed versions. Otherwise identical.

## 0.2.0

Tagged, never built (see 0.2.1). First public release: IMAP import of aggregate reports, live SPF, DKIM, DMARC, MTA-STS, TLS-RPT and BIMI checks, policy readiness per domain, alerts and monthly client reports (both experimental), CSV exports.

Nothing to do before upgrading: there is no earlier release.
