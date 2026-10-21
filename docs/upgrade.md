# Upgrade

In your stack's directory:

```bash
# set MARCAN_VERSION=<new version> in .env
docker compose pull
docker compose up -d
```

The [Releases](./releases) page lists every release, what changed and what to do before upgrading; the `vX.Y.Z` tag on GitHub carries the same notes.

## Image tags

The image has three tags per release: `X.Y.Z` is exactly that release, `X.Y` the latest patch release of that minor version, `latest` the newest release. Pin `X.Y.Z` for anything you rely on, as the Compose examples do, so nothing changes until you change the version.

## Before upgrading

1. Take a [database backup](./backup#database-backup) first, or make sure your [lists are saved](./backup#the-simple-way-keep-the-reports-in-the-mailbox). The backup is the only way back to the old version (see below).
2. If the release changed the Compose files, fetch your stack's files again with the new tag, as on its page under [Deployment](./deployment#choose-a-setup) (in a clone: `git fetch --tags && git checkout vX.Y.Z`). Keep your `.env`. The [release notes](./releases) say when a file moved: in 0.3.0 the default stack moved from `deploy/` to `deploy/caddy/`.

## Migrations and going back

Database migrations run on start and cannot be undone; the log says where the schema stands, as in `database schema at migration 2 (1 applied now)`. A database migrated by a newer version cannot be opened by an older one: the older version refuses to start, logs `migration 2 was previously applied but is missing in the resolved migrations` (with the number of the newer migration) and Docker keeps restarting it. To go back, restore the backup you took before the upgrade and start the old version:

1. Set `MARCAN_VERSION` back to the old version in `.env`.
2. Stop Marcan and copy the backup over `marcan.db`, steps 1 and 2 of [Restore](./backup#restore).
3. Start it with `docker compose up -d`, not `docker compose start marcan`: `start` would run the existing container of the new version again, which migrates the restored database once more; `up -d` replaces it with the old version's.

After an upgrade that changes the DNS checks, domains show their last result marked **updating** until they have been checked again, which happens first thing in the background.

## Checking for new versions

Marcan checks for new versions itself. While **Check for new versions daily** is on under **Settings → License** (About), as it is by default, the server fetches a small version file, `https://marcan.kaitiaki.tech/latest.json`, once a day. The docs site is hosted on GitHub Pages, so GitHub sees that request and the server's IP address. The request carries no data about your installation: no version, no identifier, no cookie; the comparison with the running version happens in Marcan. When the file names a newer version, the page says "Marcan X.Y.Z is available" and the sidebar, beside the running version under **Settings**, says "X.Y.Z available", both linking to the release notes. Switched off, nothing is fetched and nothing is shown, and the last answer is forgotten. Your browser never fetches the file.

## Reprocessing old reports

Marcan keeps each report's original XML in the database for as long as it keeps the report, so a later release can process old reports again, even ones the mailbox no longer holds. There is no command for it yet; when a release needs it, its notes will say so.
