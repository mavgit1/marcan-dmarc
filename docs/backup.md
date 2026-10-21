# Backup

All of Marcan's state is one SQLite file, `marcan.db`, in its data directory (`/data` inside the container; in the Compose examples a named volume that Docker calls `<project>_marcan_data`, such as `marcan_marcan_data` when the Compose project, by default the directory's name, is `marcan`). Almost all of it can be rebuilt, though, which makes the simplest backup the one you probably already have.

## The simple way: keep the reports in the mailbox

Marcan only ever reads the mailbox. If the reports stay there, every report can be imported again, and the rest of your setup fits in a few short lists. A restore is then a fresh start:

1. **Back up the lists.** Copy the text of each list and keep it somewhere safe:
   - **Domains** (**Settings → Domains**): every domain with its organization, and whether it is parked or ignored.
   - **Sender names** (**Settings → Sender names**): your own names for senders.
   - **DKIM selectors** (**Settings → DNS checks**): the extra selectors to check.
   - **Client report recipients** (**Settings → Client reports**): who gets each organization's monthly report.
2. **To restore**, start Marcan with an empty data directory and in the setup or **Settings**:
   1. enter the mailbox (host, user, password, folder, report address),
   2. paste the lists back in (Domains first: the client report recipients name its organizations),
   3. press **Re-import mailbox** (**Settings → Data**). Every report still in the mailbox is imported again.

What this does not bring back, all of it harmless:

- **Reports no longer in the mailbox.** If you delete or archive old reports, use the database backup below as well.
- **The record of messages that could not be imported.** They are tried again during the re-import.
- **DNS results.** Only the latest result is ever kept, and every domain is checked again within minutes.
- **The alert history and the record of client reports sent.** The **Alerts** list starts empty, and Marcan's first look at each domain records where it stands, so the restore raises no alerts. Which recipients already got this month's client report is forgotten: turned on again within a day of the report time, the month's reports go out once more.
- **Settings you changed from their defaults.** Set them again where they are:
  - the mailbox and domain warnings and the poll interval: **Settings → Mailbox**,
  - Sending, Alerts and Client reports: their own pages under **Settings** (the SMTP password is not in any list),
  - DNS resolvers: **Settings → DNS checks**,
  - the operator key and the version check: **Settings → License**; Branding (name and logo): **Settings → Branding**; Retention: **Settings → Data**.

Keep the lists current when you change domains, sender names or recipients. They change rarely.

## Retention

Marcan deletes reports older than the retention period, **12 months** by default (**Settings → Data → Retention**), once a day, together with their records and original XML, and alerts older than the same period from the **Alerts** list. The reverse-DNS names of sender IPs that no remaining report mentions go in the same clean-up. A database backup holds only what was retained when it was made. A re-import of the mailbox brings older reports back until the next daily clean-up.

## Database backup

A copy of `marcan.db` keeps everything, including reports the mailbox no longer holds. It also holds the IMAP password, stored **unencrypted**, so treat it like a secret.

The image contains no `sqlite3`. Run a throwaway container that mounts the data of the Marcan container (`--volumes-from`) and writes a copy into `./backups`. `.backup` takes a consistent snapshot while Marcan keeps running:

```bash
mkdir -p backups
docker run --rm --volumes-from marcan-app -v "$PWD/backups":/backup alpine \
  sh -c 'apk add --no-cache sqlite >/dev/null && sqlite3 /data/marcan.db ".backup /backup/marcan-$(date +%Y%m%d%H%M%S).db"'
```

`marcan-app` is the container's name in the Compose examples; with your own Compose file, `docker compose ps` shows it. The command works the same whether `/data` is a named volume or a directory on the host. For the one-command trial, use `-v marcan-data:/data` in place of `--volumes-from marcan-app`.

Do not copy `marcan.db` with `cp` while Marcan runs: recent changes may still be in `marcan.db-wal`, and the copy can be inconsistent.

### Restore

1. Stop Marcan: `docker compose stop marcan`.
2. Copy the backup over `marcan.db` in the data volume, delete `marcan.db-wal` and `marcan.db-shm` if they exist, and give the file to uid 10001 (put your backup's file name in place of `marcan-20261019120000.db`):

   ```bash
   docker run --rm --volumes-from marcan-app -v "$PWD/backups":/backup alpine \
     sh -c 'cp /backup/marcan-20261019120000.db /data/marcan.db && rm -f /data/marcan.db-wal /data/marcan.db-shm && chown 10001:10001 /data/marcan.db'
   ```

3. Start Marcan again: `docker compose start marcan`.

To go back to an older version after an upgrade, the order differs: see [Migrations and going back](./upgrade#migrations-and-going-back).
