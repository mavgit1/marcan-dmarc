# Upgrade

Read the [release notes](./releases), then, in your stack's directory:

```bash
docker compose pull && docker compose up -d
```

The stacks run the `latest` image, so a pull is an upgrade, database migrations included. Pull on purpose, never on a timer (cron, Watchtower or the like). When the notes say the stack files changed, fetch them again from your stack's page under [Deployment](./deployment#choose-a-setup) and keep your `.env`. To stay on one version, edit the `image:` line in `docker-compose.yml` to `ghcr.io/mavgit1/marcan:X.Y.Z` (or `X.Y` for the newest patch of a minor version).

## If something goes wrong

You can always start over. Marcan only reads the mailbox, so while the reports stay there, the database can be rebuilt:

1. Start Marcan with an empty data directory.
2. Enter the mailbox.
3. Paste the [four lists](./backup#keep-the-reports-and-the-lists) back.
4. Press **Re-import mailbox** (**Settings → Data**).

This loses the alert history, the record of client reports sent, and reports no longer in the mailbox. For most installations that is fine. The details are in [Backup](./backup#starting-over).

## Going back to an older version

Migrations run on start and cannot be undone; the log says where the schema stands, as in `database schema at migration 2 (1 applied now)`. Going back therefore needs a [database backup](./backup#the-database-file) taken before the upgrade:

1. Edit the `image:` line in `docker-compose.yml` to the old version, such as `ghcr.io/mavgit1/marcan:0.3.0`.
2. Stop Marcan and copy the backup over `marcan.db`, steps 1 and 2 of [Restore](./backup#restore).
3. Start it with `docker compose up -d`, not `docker compose start marcan`: `start` would run the existing container of the new version again, which migrates the restored database once more; `up -d` replaces it with the old version's.

Without the backup, the old version refuses the migrated database: it logs `migration 2 was previously applied but is missing in the resolved migrations` (with the number of the newer migration) and Docker keeps restarting it. Start over on the old version instead, as above.

## After an upgrade

After an upgrade that changes the DNS checks, domains show their last result marked **updating** until they have been checked again, which happens first thing in the background.

## Checking for new versions

Marcan checks for new versions itself. While **Check for new versions daily** is on under **Settings → License** (About), as it is by default, the server fetches a small version file, `https://marcan.kaitiaki.tech/latest.json`, once a day. The docs site is hosted on GitHub Pages, so GitHub sees that request and the server's IP address. The request carries no data about your installation: no version, no identifier, no cookie; the comparison with the running version happens in Marcan. When the file names a newer version, the page says "Marcan X.Y.Z is available" and the sidebar, beside the running version under **Settings**, says "X.Y.Z available", both linking to the release notes. Switched off, nothing is fetched and nothing is shown, and the last answer is forgotten. Your browser never fetches the file.

## Reprocessing old reports

Marcan keeps each report's original XML in the database for as long as it keeps the report, so a later release can process old reports again, even ones the mailbox no longer holds. There is no command for it yet; when a release needs it, its notes will say so.
