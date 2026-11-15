# Getting started

Marcan reads DMARC aggregate reports from one or more mailboxes (over IMAP, or Microsoft Graph for Microsoft 365), checks your domains' SPF, DKIM and DMARC records live, and shows the results in a web dashboard. It is one Rust binary with the UI built in, and it keeps everything in one SQLite file.

<Screenshot name="dashboard" alt="The Marcan dashboard: message counts, delivered compliance, senders and messages per day by outcome" />

## Two ways to run it

### Try it on your own machine

To look around once, one `docker run` command starts Marcan on your own machine, reachable from that machine only: see [Local trial](./deployment#local-trial).

This is for trying it out, not for keeping: reports keep arriving every day, so Marcan is meant to run all the time.

### Run it permanently

On a server, behind a reverse proxy that handles the login and HTTPS. Marcan has no accounts of its own: you bring the login, from a shared password to single sign-on with two-factor authentication (see [Security model](./security)).

[Deployment](./deployment) has ready-made Docker Compose setups. The default needs a server with Docker, a DNS name and five minutes. They keep the database in a named Docker volume; a directory on the host works too, but is slower on Docker Desktop and some NAS filesystems.

## What you need

- **One or more mailboxes for the reports**, reachable over IMAP with TLS (port 993) or STARTTLS (port 143) with a username and password (a dedicated mailbox, or an app password, is best), or a Microsoft 365 mailbox, read over Microsoft Graph through an app registration in Microsoft Entra (see [Accounts](./accounts#microsoft-365)).
- **Access to your domains' DNS**, for the records below.

Running Marcan for clients, as an agency or MSP? Read [Pricing](./pricing) first: the license asks you to tell clients the software is free, or to hold an operator license. Your own domains need nothing.

## Point your domains at Marcan

Receivers send their reports to the address in each domain's DMARC record. Getting them to one of Marcan's accounts takes up to two DNS records. The setup checks both live and shows the exact values to publish.

**1. On each domain you monitor:** the DMARC record names an account's address, its report address, in `rua=`.

```txt
_dmarc.example.com.  TXT  "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net"
```

If the domain already has a DMARC record, add the address to its `rua=` list; keep everything else as it is.

**2. On the report address's domain, once:** permission to receive reports for other domains. This is needed only when a monitored domain is outside the address's organizational domain, here `example.com` sending reports to `reports.example.net`. Without it, receivers drop the reports silently (RFC 9990 §4). One wildcard record covers every domain:

```txt
*._report._dmarc.reports.example.net.  TXT  "v=DMARC1"
```

A domain may also name an address of its own that forwards to an account: enter it on that account under **Reports forwarded from other addresses** (one row per address, as many as needed). See [The report address](./accounts#the-report-address).

## First start

Until the setup is finished or skipped, Marcan opens it at **/setup**. It shows one step at a time, with **Back** and **Continue**, in a fixed order; the list of steps at the top marks the ones done, the one on screen and the ones to come, and a step you reached can be opened from it again.

<Screenshot name="setup" alt="The setup at its Accounts step: the steps from Organization to Done along the top, then the account form with provider, email address, login, the folder, the servers under Advanced, Test and Save and start import" />

1. **Organization**: the group new domains land in. Rename it, or keep "Default" and add one organization per client later.
2. **Accounts**: the mailboxes the reports arrive in, one or more, in the same form as **Settings → Accounts** ([Accounts](./accounts)): provider (Microsoft 365, or any IMAP/SMTP provider), email address, login, and the servers under **Advanced** for a provider without a preset (a Microsoft 365 account has none). **Test** logs in and lists the folders; pick the one the reports land in, which shows how many messages it holds. **Save and start import** tests the login, saves the account and starts importing the reports already in its folder; nothing is imported before. **Add another account** adds the next one.
3. **Import**: how far the import is, for example "Importing: 1,204 reports from 14 domains so far, checking DNS", then one line per account: the one being read with a progress bar, the others with their last check or error. You can continue at once: the import keeps running behind the next steps, and the header shows it too.
4. **Domains**: the domains the import found, a list that grows while it runs, and a box to add more. Optional: domains also appear on their own when their first report arrives.
5. **Report addresses**: every address reports reach Marcan at (each account's, and the ones forwarded to it), the one new DMARC records name, and the authorization record each address domain still needs, with the real addresses. Nothing to type: an address is changed on its account.
6. **DMARC records**: the record for each domain, checked live, with what it still needs: no DMARC record, authorization missing, report address missing, or waiting for the first report.
7. **Done**: what arrived so far and the optional next steps below. **Finish setup** needs an account that receives reports, not the first report.

While the import runs, the Domains and DMARC records steps say "Still importing": the counts there are not final yet.

Receivers send aggregate reports about once a day, so the first ones usually arrive within 24 to 48 hours. If the folders already hold reports, Marcan imports them at once.

You can skip the setup and open it again under **Settings → Data → Run setup again**. Everything it sets is also under **Settings**. **Guide** (under Tools) shows the principles with your own values and the live status, and opens the setup again, followed by a short DMARC, SPF and DKIM reference.

## Next steps

Optional, and all under **Settings** (the setup's last step lists them too):

- **Turn on alerts**: set up **Sending**, then **Alerts**, for an email when something about a domain changes and a digest. See [Alerts](./alerts).
- **Send clients a monthly report**: list the recipients under **Client reports**, then turn it on there. See [Client reports](./client-reports).
- **Running Marcan for clients?** Add your operator key under **License**, or keep the free-software notice on. See [Pricing](./pricing) and [License](./license).

To read on:

- [How it works](./how-it-works): what Marcan does with a report, and what it decides on its own.
- [Reading the dashboard](./dashboard): health, the attention list and outcomes.
- [Backup](./backup): why keeping the reports in the mailboxes is the simplest backup.
- [Upgrade](./upgrade): moving to a new release, and back.
