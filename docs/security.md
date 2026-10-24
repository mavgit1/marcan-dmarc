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

## The mailbox password

- **It cannot be read back.** The UI and the API show the host, user name and folder, never the password, and it is never written to logs.
- **It is only ever sent to the mailbox it was saved for.** Changing the host, port or user name asks for it again, so nobody can redirect the stored password to a server of their own.
- **It is stored unencrypted in the database.** The database file is created readable by its owner only, but anyone with the file, or a backup of it, has the password.

Why not encrypted? The key would have to be on the same server, readable by Marcan, so anyone who can read the database can almost always read the key as well: it would look safer without being safer. The one case it helps is a backup that leaks without the key. You can avoid that case entirely by using the [mailbox as your backup](./backup#keep-the-reports-and-the-lists) instead of copying the database around.

Use a dedicated mailbox for the reports, or an app password that can be revoked on its own.

## Sending mail

Sending is off by default. **With sending on, anyone who can reach Marcan can make it send mail as the sender address**: the UI has no login of its own, so whoever gets past your proxy can use the test button or change the recipient. Turn sending on only when Marcan sits behind your proxy's authentication. The test sends a fixed message and only chooses the test recipient and the account as typed on the page (a stored password only ever goes to the server and username it was saved for), so it cannot be used to send arbitrary content; it can still be used to send that message to anyone. The test alert (**Settings → Alerts**) and the [client report](./client-reports) preview go to the saved test recipient, which anyone who can reach Marcan can change, so whoever can reach Marcan can mail made-up changes or that summary of an organization's domains anywhere.

Marcan sends **at most one message every 5 seconds**, whatever asks for it: alerts, digests, client reports, the tests and previews. Messages wait their turn and none is dropped, so a script that keeps pressing a test button can mail at most 12 messages a minute. That bounds the misuse; it does not prevent it, which is what the proxy's login is for.

Marcan only hands mail to your provider's SMTP submission server, always logged in and always over TLS or STARTTLS: unencrypted SMTP and port 25 are refused, and it never delivers mail to the recipients' servers itself.

The SMTP password follows the mailbox password's rules: it cannot be read back, it is only sent to the server and username it was saved for, and it is stored unencrypted. The mailbox login is only used for sending when the SMTP server belongs to the mailbox's own provider.

## The operator key

An operator key changes only how the emails name their sender: see [What a key changes](./license#what-a-key-changes). It unlocks no feature and no access, it changes nothing in Marcan's own pages but the line under its name in the sidebar, and Marcan never contacts anyone to check it. Like the passwords, it is not shown back once saved.

## Mailbox access

Marcan opens the report folder read-only and reads messages without marking them as seen. It never moves, flags or deletes mail.

## Untrusted report content

Anyone can send a report to your mailbox, so report content is treated as untrusted:

- Attachments over 25 MB are rejected, and so are reports that unpack to more than 50 MB of XML: the limit is checked while unpacking, so a zip or gzip bomb stops there. The reports in one message may unpack to 50 MB together.
- Implausible values (negative counts, periods in the future, invalid domain names) quarantine the report instead of importing it.
- In CSV exports, values a spreadsheet could read as a formula get a leading `'`.
- A valid report from anyone adds the domain it names to the default organization and starts monitoring it, DNS checks included. Ignore the domain to stop that.

## Limitations

- **One mailbox.** All reports must arrive in one IMAP mailbox and folder.
- **Password login to the mailbox.** Microsoft 365 / Exchange Online mailboxes cannot connect, because Microsoft has turned off IMAP password logins there and Marcan does not support OAuth2 yet.
- **IMAP over TLS or STARTTLS only.** No unencrypted IMAP: with STARTTLS, a server that does not offer the upgrade is refused.
- **Aggregate reports only.** Forensic (`ruf=`) reports and TLS-RPT reports are not imported.

See the [roadmap](./roadmap) for what may change.
