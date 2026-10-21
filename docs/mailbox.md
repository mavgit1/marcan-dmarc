# Mailbox & reporting setup

Marcan reads reports from **one IMAP mailbox and one folder**. Every domain you monitor has to send its aggregate reports there, directly or through forwarding.

## Connect the mailbox

Under **Settings → Mailbox** (or in the setup):

| Field | Notes |
| :--- | :--- |
| Host, port | The IMAP server. Port 993 by default, 143 with STARTTLS. |
| Security | **TLS** (TLS from the first byte, usually port 993) or **STARTTLS** (a plain connection upgraded to TLS before logging in, usually port 143). Unencrypted IMAP is not supported: a server that does not offer STARTTLS is an error, never a fallback. |
| Username, password | A plain IMAP login. Use a dedicated mailbox or an app password. |
| Report address | The address DMARC `rua=` tags point at. Only needed when it differs from the login, for example a shared mailbox you log in to with your own account. |
| Folder | Where the reports land. `INBOX` by default. |
| Delivery | Under **Reports**: how reports reach the mailbox, also asked in the setup's **Report delivery** step. **Central mailbox** (the default): every domain's `rua=` names the report address. **Forwarded or central**: a domain may name an address of its own that forwards to it. The choice decides what the setup asks you to publish and how each domain's reporting status is judged; see [Choose how reports arrive](#choose-how-reports-arrive). |
| Checks | Under **Schedule**: how often Marcan looks for new mail, in whole minutes. Every 5 minutes by default, at least every minute. |
| Mailbox warning | Under **Report arrival**: how many hours without any new report, for the mailbox as a whole, before the status in the header warns (and goes red after twice as long). 4, 12, 24, 48 or 72 hours; 24 by default. |
| Domain warning | Under **Report arrival**: how many days without a report before a domain that had reports is flagged with the **reports stopped** issue, which is also an [alert](./alerts). 2, 3, 5, 7, 10, 14 or 30 days; 14 by default. A parked domain is not flagged. |

How long reports are kept is set apart, under **Settings → Data → Retention** (see [Backup](./backup#retention)).

Switching the security moves the port to its usual number, unless you typed a port of your own. Changing the host, port or username asks for the password again. The stored password is only ever sent to the mailbox it was saved for.

The password is never returned by the API, and it is stored **unencrypted** in the database. Protect the data directory and your backups accordingly.

Marcan never changes the mailbox: it opens the folder read-only and reads messages without marking them as seen. You can keep reading the same mailbox in a mail client, or clean it up with your own rules.

## Choose how reports arrive

The choice is one setting for all domains: **Delivery** under **Settings → Mailbox → Reports**, or the setup's **Report delivery** step. It has two values, **Central mailbox** (the default) and **Forwarded or central**. A domain's reports count once they arrive, whatever its record names: it then reads **Reporting** in either mode.

### Central mailbox (recommended)

Every domain's DMARC record names the report address itself:

```txt
_dmarc.example.com.  TXT  "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net"
```

When a domain is outside the report address's organizational domain (here `example.com` reporting to `reports.example.net`), receivers only send reports after the mailbox's domain agrees to receive them (RFC 7489 §7.1). Publish one wildcard record on the mailbox's domain and every domain is covered:

```txt
*._report._dmarc.reports.example.net.  TXT  "v=DMARC1"
```

Without it, reports for those domains are dropped silently. Marcan checks this record for each domain and shows the exact records to publish; while the mailbox's domain refuses a domain's reports, that domain reads **Authorization missing**. A record per domain (`example.com._report._dmarc.reports.example.net`) works as well: when every domain has its own, the setup reads **Authorized per domain** and mentions the wildcard only as a way to cover domains added later.

In this mode every `rua=` must name the report address. A domain whose record names only an address of its own (`rua=mailto:dmarc@example.com`, forwarded or not) reads **Report address missing** until its first report arrives, and Marcan shows the record with the report address added. Once the records are right, a domain reads **Waiting for first report** until its first report arrives.

### Forwarded or central

Each domain may keep its own report address (for example `dmarc@example.com`) that forwards to Marcan's mailbox. Use this when domains must not name a shared address.

Marcan cannot see where an address forwards to, so it does not ask for the report address in `rua=`. A domain whose record names the report address is judged as under Central mailbox, so the two kinds of domain can be mixed in this mode. For the others:

- A record without any `mailto:` address reads **Report address missing**.
- A forwarding address outside the domain's organizational domain must accept the domain's reports too (RFC 7489 §7.1), with a record on the forwarding address's domain, such as `example.com._report._dmarc.forwarder.example.net` for `rua=mailto:dmarc@forwarder.example.net`. While every address in `rua=` refuses them, the domain reads **Authorization missing**.
- Otherwise the domain reads **Waiting for first report** until its first report arrives, then **Reporting**.

## Sync and re-import

- **Sync now** (in the header's status and in the setup's Import step) polls the mailbox immediately.
- **Re-import mailbox** (**Settings → Data**, Actions) reads every message in the folder again and adds the reports that are missing. Reports already stored are skipped. Use it after taking a domain off the ignore list, or when you have moved old reports into the folder.

## When a message cannot be imported

A message is **quarantined**, recorded and skipped, so it never stalls the import, when it is not a valid report. For example: its domain is not a valid hostname, a record count is below 0 or above 1,000,000,000, the report period ends before it begins or begins more than a day in the future, or the attachment is larger than 25 MB.

A message that fails for another reason (the server returns nothing for it, the download times out, the database rejects it) is retried on the next passes and quarantined after 5 attempts. The header's status counts skipped reports.

## Sending

Marcan can send mail through an SMTP account, set up under **Settings → Sending**. It is **off by default**. The switch at the top of the page, **Sending**, is saved with the rest of the page: while it is off, the fields below it are greyed out and keep their values, and Marcan sends no mail at all. Switched on, **Save** waits until a server is entered ("Enter a server first."), and saving checks that the account can send. Marcan uses it for [alert emails and the digest](./alerts) and for [client reports](./client-reports), each on its own page after it.

<Screenshot name="sending" alt="Settings, Sending: the switch on, the SMTP server and login, the sender address with its domain's DMARC policy and SPF ending, Reply-To and the test recipient" />

| Field | Notes |
| :--- | :--- |
| Host, port | Your provider's SMTP submission server. The port starts at the usual one for the security: **465** with TLS, **587** with STARTTLS. Port 25 is refused. |
| Security | **TLS** (TLS from the first byte, usually port 465) or **STARTTLS** (a plain connection upgraded to TLS before logging in, usually port 587). Unencrypted SMTP is not supported: a server that does not offer STARTTLS is an error, and Marcan disconnects before logging in. |
| Same login as the mailbox | On by default: Marcan logs in with the mailbox's username and password. Only allowed when the SMTP server belongs to the mailbox's provider (the same host, or the same domain, such as `imap.example.com` and `smtp.example.com`), so the mailbox password never goes anywhere else. |
| Username, password | The SMTP login when it is not the mailbox's. Like the mailbox password, it is never returned by the API, stored unencrypted, and only sent to the server and username it was saved for. A stored password shows as **Stored** with **Replace**. |
| Sender address | The From address. With the same login it may stay empty, and mail goes out from the mailbox address. |
| Reply-To | Where replies go. Empty: the sender address. |
| Test recipient | Where test emails and previews go: this page's **Send a test**, the test alert under **Alerts** and the client report previews. Empty: the sender address. |

Marcan is an SMTP submission client and nothing more: it always logs in, it never delivers mail to the recipients' servers itself, and it never signs mail with DKIM. Whether the mail is trusted is up to your provider and your DNS.

**Send through your report mailbox's provider, so SPF and DKIM are its job.** The provider then signs the mail with DKIM and its servers are already in your SPF record, so the sender's domain passes DMARC without any work. A separate SMTP account (a transactional mail service, another provider) needs its own SPF and DKIM set up for the sender's domain first. Once a sender is saved, the page shows that domain's DMARC policy and SPF `all` from Marcan's DNS check, so a sender without SPF stands out at once.

**Send a test** sends a short message ("Marcan can send mail from this address.") with the values on the page, saved or not, while the switch is on, to the test recipient on the page (the sender itself when it is empty); the line under the button says where it goes. A stored password is only used for the server and username it was saved for. When it fails, it says which step failed and why: the host does not resolve, the server cannot be reached, the TLS connection or STARTTLS failed, the login was rejected, or the server refused the message (with its reply).

With sending on, anyone who can open Marcan can make it send mail as the sender. See [Security](./security#sending-mail).

## Microsoft 365 and Exchange Online

Marcan logs in with a username and password only. Microsoft has turned off password logins for IMAP in Exchange Online, and Marcan does not support OAuth2, so these mailboxes cannot connect. Use a mailbox elsewhere (your own mail server, or any provider that accepts a password or an app password) and point your domains' `rua=` at it, or forward the reports there.
