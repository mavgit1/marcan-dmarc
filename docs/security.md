---
description: Marcan has no login of its own and relies on your reverse proxy; host name checks, credentials encrypted with MARCAN_SECRET_KEY, sending and mailbox access.
---

# Security model

## Bring your own login

Marcan has no user accounts of its own. Signing in is left to the reverse proxy in front of it, which is where your organization's login already lives:

- **Single sign-on** with your identity provider (Entra ID, Okta, Google, Keycloak) through OAuth2-Proxy,
- **two-factor authentication** and separate users through Authelia,
- or **one shared password** with Caddy's basic auth for a small setup.

So Marcan is exactly as well protected as the login you choose, and it gets every improvement of that login (passkeys, conditional access, audit logs) for free. There is no second set of passwords to manage and no login code in Marcan to get wrong. [Deployment](./deployment) has a ready-made setup for each.

The flip side: **Marcan must never be reachable except through that proxy.** Anyone who reaches its port directly can read every report and change every setting.

## How the examples keep it private

- Inside the container Marcan listens on `0.0.0.0:8080`. That means all of the *container's* interfaces, not the server's: without a `ports:` entry, Docker makes the port reachable only from containers on the same Compose network. None of the examples in `deploy/` publish it; only the proxy can reach Marcan.
- If you do publish the port, bind it to the loopback address (`127.0.0.1:8080:8080`). A plain `8080:8080` opens it on every interface of the server, and Docker's published ports bypass host firewalls such as `ufw`.
- Outside a container, the binary listens on `127.0.0.1:8080` by default.

## Host names and frames

A web page you visit can try to reach a Marcan on your own machine by pointing its own name at 127.0.0.1 (DNS rebinding). Marcan answers only requests addressed to a host it knows: when it listens on one address (the binary's default `127.0.0.1:8080`), that address, `localhost` and the names given with `--allowed-hosts` (`ALLOWED_HOSTS`). Anything else gets an error. The image listens on every interface of its container and takes whatever name your proxy forwards, so there it answers any host unless you set `ALLOWED_HOSTS`; the proxy and its login are what protect it.

Every response tells the browser never to show Marcan inside a frame on another page (`X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`), whatever proxy is in front. A site cannot lay Marcan under its own page and trick a signed-in user into clicking.

`/healthz` answers `ok` and nothing else, so the examples leave it without a login for uptime monitors. Everything else, including `/api/system/health`, needs one.

Every signed-in user can change every setting. There are no roles: who may sign in at all is decided at the proxy.

## Account credentials

Each [account](./accounts) holds one credential: a password, or for a Microsoft login a certificate's private key.

- **It cannot be read back.** The UI and the API show the servers, the username, the tenant and client ID and a certificate's thumbprint, never the password or the key, and none of them is written to the logs.
- **It only goes where it was saved for.** A password is sent only to the IMAP and SMTP servers and the username it was saved with: changing a host, port or username asks for it again, so nobody can redirect a stored password to a server of their own. A certificate's private key never leaves Marcan: only assertions signed with it go to Microsoft's login service (`login.microsoftonline.com`), and the token they get goes only to Microsoft Graph.
- **Tokens go to Microsoft Graph only.** The OAuth token of a Microsoft login is sent to Microsoft Graph (`graph.microsoft.com`, to read the mailbox and to send as it), and nowhere else, whatever servers the account's page holds: Marcan follows no redirect and no link from an answer to another host with a token.
- **It is stored unencrypted in the database unless you set a key** ([Secrets at rest](#secrets-at-rest)). The database file is created readable by its owner only, but without a key anyone with the file, or a backup of it, has the credentials.

Use a dedicated mailbox for the reports, or an app password that can be revoked on its own. A Microsoft app registration reaches only the mailboxes its role assignments in Exchange Online are scoped to, and its certificate can be removed in Entra at any time.

## Secrets at rest

The key is 32 random bytes, in base64 (with or without its `=` padding) or as 64 hex digits. Generate it once:

```bash
openssl rand -base64 32
```

Marcan reads it from the environment only, never from a command-line flag (which every user of the machine sees in the process list):

- `MARCAN_SECRET_KEY` holds the key itself. The Compose stacks read it from `.env`, where it is commented out (see [Deployment](./deployment#image-and-options)).
- Or `MARCAN_SECRET_KEY_FILE` names a file that holds it, such as a Docker secret. The environment of a container is not private: `docker inspect` shows it to anyone who may use Docker on the host, so the file is the better place. The Compose stacks have the lines for it, commented out: put the key in `marcan_secret_key` next to `docker-compose.yml`, readable by the container's user only (`sudo chown 10001:10001 marcan_secret_key && sudo chmod 400 marcan_secret_key`), and uncomment them.

Setting both is an error, and so is a file that cannot be read or is empty. An empty `MARCAN_SECRET_KEY` counts as not set.

**What it encrypts.** Every account's stored secret: passwords and Microsoft certificate private keys, each with AES-256-GCM, a random nonce of its own, and the account and the kind of secret as additional data, so a value copied onto another account does not open. Each value records which key sealed it. The rest of the database (reports, settings, addresses, tenant and client IDs, the certificates themselves) is not encrypted.

**Adding a key later.** The first start with the key encrypts every secret stored without one, in one transaction, and logs how many. It then rewrites the database file (SQLite's `VACUUM`) and empties its write-ahead log, because the unencrypted values would otherwise stay readable in the pages they were freed from; this needs as much free disk space as the database takes, once. From then on, whatever Marcan deletes or replaces is overwritten in the file (`secure_delete`). Nothing else is needed. An upgrade from 0.3 also rewrites the database once, at its first start, for the passwords the 0.3 settings held.

**Keep the key apart from the backups.** The key protects a copy of the database that leaks without it, such as a backup, so keep it in a password manager, not on the same disk or in the same archive. It does not help against someone who can read the server's files or Marcan's environment, where the key has to be. Using the [mailboxes as your backup](./backup#keep-the-reports-and-the-lists) instead of copying the database around avoids the leaking copy altogether.

**Changing the key.** Give Marcan the new key and, for one start, the old one:

1. Generate a new key (`openssl rand -base64 32`).
2. Set the old key as `MARCAN_SECRET_KEY_PREVIOUS` (or `MARCAN_SECRET_KEY_PREVIOUS_FILE`) and the new one as `MARCAN_SECRET_KEY`, and restart (`docker compose up -d` applies a changed `.env`).
3. The start encrypts every secret the old key encrypted again with the new one, in one transaction, rewrites the database as above, and logs how many: "encrypted again from MARCAN_SECRET_KEY_PREVIOUS to MARCAN_SECRET_KEY".
4. Remove `MARCAN_SECRET_KEY_PREVIOUS` and restart. (Left in place, it does no harm: the next start logs that no secret uses it any more.)

A secret neither key opens stays [locked](./accounts#locked-accounts). `MARCAN_SECRET_KEY_PREVIOUS` without `MARCAN_SECRET_KEY` stops the start with an error that says so. Backups made before the change still need the old key.

**A missing or changed key.** Marcan still starts, but every account whose secret it cannot open is [locked](./accounts#locked-accounts): it neither receives nor sends, the log names it at start and at each pass or send, and its card under **Settings → Accounts** says why. There are two ways out:

1. Set the original key again and restart.
2. Or enter the account's credentials again (the password, or a new certificate uploaded to the app registration). They are then stored with the current key, or unencrypted without one.

Removing the key therefore does not decrypt anything: to go back to unencrypted storage, remove it and enter the credentials again. A key that is not 32 bytes in base64 or hex stops the start with an error that says so (and never shows the value).

**A damaged value.** A secret that the key it was encrypted with does not open (the database file was damaged, or the value was altered) locks its account too, with its own reason: restore the database from a backup, or enter the credentials again.

**Tokens never touch the disk.** The access tokens of a Microsoft login are kept in memory only and fetched again after a restart. A certificate generated in a form that is not saved yet waits in memory too, for a day at most, so an abandoned form leaves no key behind.

**Without a key, the file is the secret.** Treat `marcan.db` and every copy of it like the passwords it holds.

## Sending mail

Sending is off by default. **With sending on, anyone who can reach Marcan can make it send mail as the sender address**: the UI has no login of its own, so whoever gets past your proxy can use the test button or change the recipient. Turn sending on only when Marcan sits behind your proxy's authentication. The test sends a fixed message and only chooses the test recipient and the account as chosen on the page, so it cannot be used to send arbitrary content; it can still be used to send that message to anyone. The test alert (**Settings → Alerts**) and the [client report](./client-reports) preview go to the saved test recipient, which anyone who can reach Marcan can change, so whoever can reach Marcan can mail made-up changes or that summary of an organization's domains anywhere.

Marcan sends **at most one message every 5 seconds**, whatever asks for it: alerts, digests, client reports, the tests and previews. Messages wait their turn and none is dropped, so a script that keeps pressing a test button can mail at most 12 messages a minute. That bounds the misuse; it does not prevent it, which is what the proxy's login is for.

Marcan only hands mail to your provider: to its SMTP submission server, always logged in and always over TLS or STARTTLS (unencrypted SMTP and port 25 are refused), or for a Microsoft login to Microsoft Graph over HTTPS, as the account's mailbox. It never delivers mail to the recipients' servers itself.

Mail goes out through the account chosen under **Settings → Sending**, with that account's credential and its [rules](#account-credentials).

## The operator key

An operator key changes only how the emails name their sender: see [What a key changes](./license#what-a-key-changes). It unlocks no feature and no access, it changes nothing in Marcan's own pages but the line under its name in the sidebar, and Marcan never contacts anyone to check it. Like the passwords, it is not shown back once saved.

## Mailbox access

Marcan opens each account's report folder read-only and reads messages without marking them as seen. It never moves, flags or deletes mail.

## Untrusted report content

Anyone can send a report to your report addresses, so report content is treated as untrusted:

- Attachments over 25 MB are rejected, and so are reports that unpack to more than 50 MB of XML: the limit is checked while unpacking, so a zip or gzip bomb stops there. The reports in one message may unpack to 50 MB together.
- Implausible values (negative counts, periods in the future, invalid domain names) quarantine the report instead of importing it.
- In CSV exports, values a spreadsheet could read as a formula get a leading `'`.
- A valid report from anyone adds the domain it names to the default organization and starts monitoring it, DNS checks included. Ignore the domain to stop that.

## Limitations

- **One folder per account.** Each account is read from one folder.
- **Microsoft 365 in the global cloud only.** A Microsoft login works with Microsoft Graph's worldwide endpoint (`graph.microsoft.com`); national clouds are not supported. Other providers log in with a password or an app password.
- **IMAP over TLS or STARTTLS only.** No unencrypted IMAP: with STARTTLS, a server that does not offer the upgrade is refused.
- **Aggregate reports only.** Forensic (`ruf=`) reports and TLS-RPT reports are not imported.

See the [roadmap](./roadmap) for what may change.
