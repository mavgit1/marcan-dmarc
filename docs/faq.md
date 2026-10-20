# FAQ

## No reports arrive. What should I check?

1. **Is the mailbox connected?** The status in the header shows the last poll and any error. **Sync now** polls at once.
2. **Is the folder right?** Reports must land in the folder set under **Settings → Mailbox**. Check that mail filters do not move them elsewhere.
3. **Does the domain's DMARC record name the right address?** The domain's page and the [DNS inspector](./dns-checks) show its `rua=`.
4. **Is the report address authorized?** If the mailbox is on another organizational domain, receivers drop reports silently unless the mailbox's domain publishes an authorization record. See [Central mailbox](./mailbox#central-mailbox-recommended).
5. **Has it been long enough?** Receivers send reports about once a day; the first ones usually arrive within 24 to 48 hours.

## Why does my domain show red although all mail passes?

The DNS grade rates DNS configuration, not mail. `p=none` is a critical finding, because with it spoofed mail is still delivered. Health, which rates the mail, can be **Protected** at the same time. Move to `p=quarantine` or `p=reject` once every legitimate sender passes: the domain's page says when it is ready (see [When to move to the next policy step](./dashboard#when-to-move-to-the-next-policy-step)).

## Why do Marcan's numbers differ from the reporter's?

Marcan recomputes DMARC alignment for every record instead of trusting each receiver's verdict, and it separates forwarded mail from real failures. See [How it works](./how-it-works).

## What are "forwarded" failures?

Mail that failed DMARC in transit (forwarding and mailing lists break SPF and sometimes DKIM) but that the receiver delivered because it trusted the forwarder. They are counted apart from real delivered failures so they are not mistaken for spoofing. Signing with DKIM keeps most forwarded mail passing.

## A domain that is not mine shows up. How do I get rid of it?

Somebody sent mail as that domain with your report address in its DMARC record, or a receiver mixed up reports. Mark it `ignored`: its reports are deleted and new ones are dropped. See [Domain status](./domains#domain-status).

## Can I use a Microsoft 365 mailbox?

No. Microsoft has turned off IMAP password logins for Exchange Online, and Marcan does not support OAuth2. Use another mailbox and forward or point the reports there.

## Does Marcan delete or move mail?

No. It reads the folder without changing anything, not even the read flag.

## How much space does it need?

Little. Aggregate reports are small, and the database holds the parsed data plus each report's compressed XML. **Settings → Data** shows the database size. Reports older than the retention period, 12 months by default, are deleted (see [Backup](./backup#retention)).

## Can several people use it?

Yes. The login comes from the reverse proxy: Authelia or OAuth2-Proxy give everyone their own login, with two-factor authentication or your single sign-on. Marcan has no roles, so everyone who can sign in can change every setting; decide at the proxy who may sign in.

## Is Marcan free?

For your own domains, yes, completely. If you run it for clients, including administering it on a client's own server, you tell them it is software they could run themselves, or buy a license instead. See [License & pricing](./license).

## What will Marcan not do?

- **Forensic (failure) reports.** The large mailbox providers do not send them, and they contain personal data: headers and sometimes content of individual messages.
- **BIMI certificate (VMC) checks.** Marcan reads the BIMI record but does not validate the mark certificate behind it: that certificate is between you and the authority that issued it, and it says nothing about whether your mail authenticates.
- **Hosting or editing your DNS records.** Marcan never touches your DNS; it only reads it and tells you what to change.

## How do I report a bug?

Open an issue on [GitHub](https://github.com/mavgit1/marcan-dmarc/issues). Include the version (shown under **Settings → License**) and the relevant log lines.
