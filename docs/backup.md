# Backup

Marcan only reads the mailbox. As long as the reports stay there, the database is a cache: every report can be imported again. You may not need a backup.

## Keep the reports and the lists

Two things are enough:

- **The reports in the mailbox.** Do not delete or archive them.
- **A copy of the four lists.** Each is plain text on its settings page; copy it out when it changes:
  - **Domains** (**Settings → Domains**): every domain with its organization, and whether it is parked or ignored.
  - **Sender names** (**Settings → Sender names**): your own names for senders.
  - **DKIM selectors** (**Settings → DNS checks**): the extra selectors to check.
  - **Client report recipients** (**Settings → Client reports**): who gets each organization's monthly report.

## Starting over

Start Marcan with an empty data directory, then, in the setup or **Settings**:

1. Enter the mailbox (host, user, password, folder, report address).
2. Paste the lists back, Domains first: the client report recipients name its organizations.
3. Press **Re-import mailbox** (**Settings → Data**). Every report still in the mailbox is imported again.

This does not bring back:

- **Reports no longer in the mailbox.**
- **The alert history and the record of client reports sent.** The **Alerts** list starts empty. Marcan's first look at each domain records where it stands, so starting over raises no alerts. Client reports turned on again within a day of the report time go out once more for the month.
- **Settings you changed from their defaults**, such as Sending (the SMTP password is in no list), Alerts, the poll interval, DNS resolvers, the operator key, Branding and Retention. Set them again on their pages under **Settings**.

Messages that could not be imported are tried again, and every domain's DNS is checked again within minutes.

## The database file

All of Marcan's state is one SQLite file, `marcan.db`, in its data directory (`/data` inside the container; in the Compose examples a named volume that Docker calls `<project>_marcan_data`, such as `marcan_marcan_data` when the Compose project, by default the directory's name, is `marcan`). Back it up when:

- you delete or archive reports from the mailbox,
- you want to keep the alert history and the record of client reports sent,
- you want to be able to [go back to an older version](./upgrade#going-back-to-an-older-version) after an upgrade.

The file also holds the IMAP password, stored **unencrypted**, so treat a copy like a secret.

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

To go back to an older version after an upgrade, the order differs: see [Going back to an older version](./upgrade#going-back-to-an-older-version).

## Retention

Marcan deletes reports older than the retention period, **12 months** by default (**Settings → Data → Retention**), once a day, together with their records and original XML, and alerts older than the same period from the **Alerts** list. The reverse-DNS names of sender IPs that no remaining report mentions go in the same clean-up. A database backup holds only what was retained when it was made. A re-import of the mailbox brings older reports back until the next daily clean-up.
