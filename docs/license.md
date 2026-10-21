# License

All of Marcan's code is public, and nothing is held back for a paid edition. What you owe depends only on whose domains you monitor.

## Your own domains: free

A company, an association or a person monitoring **its own domains** uses Marcan for free, with no duties beyond the usual ones for open code: keep the license with copies you pass on, and if you run a modified version, offer its source to the people who use it. That includes:

- domains of companies in the same group (parent company, subsidiaries),
- every user and employee who signs in.

## Domains of clients: tell them, or buy a license

You run Marcan for clients when it monitors **other people's domains** as a service you provide, paid or unpaid. That includes:

- running it as an MSP, hoster or consultant on your own servers,
- installing, configuring or administering it **on a client's own server** for them,
- monitoring domains you register, hold or control **on a client's behalf**.

Then you choose:

**Option 1: tell your clients.** Let each client know, when they sign up or their first domain is added, that the service runs on software whose source code is freely available, that they may run it themselves, and where to get the source of the version you run. Keep that information available to them as long as they are your client. That's all; it costs nothing.

An example you can use in a welcome e-mail or your terms:

> This service runs on Marcan, software whose source code is freely available under the Beneficiary Copyleft License 1.0: you may run, study, modify and share it yourself. Source code of the version we run: https://example.com/source

**Option 2: buy a license.** It removes the duty to tell your clients, for as long as it is active, and lets you keep your own changes to Marcan private. The two plans, the 30-day trial and how client domains are counted are on [Pricing](./pricing). Buying a license means agreeing to the [Marcan Operator License v1](/licenses/operator-v1).

### The operator key and the notice

Marcan shows the **free-software notice** by default, one sentence that says what the license asks you to tell your clients, with the link to Marcan's source code:

- as the last line of every email it sends (alerts, digests and client reports), signed with your operator name when one is set under **Settings → Branding**:

  > Sent by Acme IT with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v0.2.0

  Without an operator name it reads "Sent by Marcan, free software …". The link names the release you run, here 0.2.0, so it leads to the source of exactly that version.
- as a small line "Free software · source · license" under "Marcan" in the sidebar, linking to the source code and to this page.

It is never added to the CSV exports: they are a documented format that other tools read.

The notice helps, but it does not replace telling a client at sign-up (Option 1 above): the first email that carries it may arrive weeks after the client signed up or its first domain was added, with the first alert or the first monthly report, and the license asks for the information by then.

When you subscribe (see [Pricing](./pricing)), you receive an **operator key**. Enter it under **Settings → License → Operator key**. Saving a valid key turns the notice off at once, and it stays off as long as the key is stored: the emails then end in "Sent by Acme IT." (or "Sent by Marcan." without an operator name). Removing the key brings the notice back. With the notice off, the sidebar shows a small "Operator license" badge in place of the free-software line.

### What a key changes

The emails are what your clients see, so they carry the attribution. A key changes three things in them: the sender name, the top of the email and its last line. Without a key Marcan comes first in each and your branding second; with one, those three name you only. For an operator named Acme IT:

| | Without an operator key | With a valid operator key |
| :--- | :--- | :--- |
| Sender name | Acme IT via Marcan | Acme IT |
| Top of the email | "Marcan, free software" with Marcan's small mark, then your logo (at most 120 by 32 px) with "Managed by Acme IT" beside it | Your logo (at most 200 by 64 px) with "Acme IT" in bold beside it |
| Last line | Sent by Acme IT with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: … | Sent by Acme IT. |

Without an operator name, the sender name is "Marcan" either way; without a key the top is Marcan's line alone and the last line "Sent by Marcan, free software …", with one the top is your logo alone (nothing without a logo) and the last line is "Sent by Marcan.". The email's title (such as "DMARC report for Acme, September 2026") follows the top in both. The subjects stay the same with or without a key: alert emails and digests still begin with Marcan ("Marcan: 3 changes on example.com", "Marcan daily digest: …"), and client reports name the organization ("Acme: DMARC report for September 2026"). That is all the key changes, besides the sidebar line above: every feature works the same with or without it, nothing is hidden or blocked without one, and Marcan never asks you for one.

**Settings → License** opens with the duty in one sentence and says that the alert and client report emails carry the notice, with a link to **Branding**. Under it, once a key is stored, the license state reads "Operator license active. Licensed to you@acme.example. Client emails carry your name, not the free-software notice.", marked green, with two links: **Manage or cancel a subscription** (the billing link on [Pricing](./pricing#good-to-know)) and **Branding**. The key field then shows whose key it is and how it ends, such as "you@acme.example · …GPL7CQ", never the whole key, with **Replace** and **Remove**. Without a key the page reads "No operator license. Client emails carry the free-software notice."; a stored key that does not verify reads "The stored operator key is not valid.", and the notice stays on. In every state a line "Plans and prices: marcan.kaitiaki.tech/pricing" links to [Pricing](./pricing), which lists the plans, the trial and their checkout links; the page itself shows no plans. How the emails look is shown under **Settings → Branding** (see [Branding](#branding)).

Without an operator license the notice stays on: the license requires telling clients the software is free, and the notice keeps that visible. An operator license replaces that duty, so with one the notice is off.

The key is checked inside Marcan, never online, and it does not expire. The license is a subscription, though: when it ends, the duty to tell your clients comes back whatever the key says (see below), so remove the key to bring the notice back.

### When a license ends

If you go on running Marcan for clients, you inform your existing clients within 30 days, and new clients as before. If you run a modified version, its source must then be offered to them as well.

### Branding

Your name and logo (**Settings → Branding**) are shown in the emails only: at the top of every email, and your name in the sender name: "Acme IT via Marcan" without an operator key, "Acme IT" with one. Marcan's own pages show neither; the emails are what your clients see. Branding needs no key. The logo is an image you upload there: PNG, JPEG or WebP, at most 200 KB, stored in Marcan's database (nothing is fetched from the web). The HTML emails carry it as an inline attachment, so mail clients that strip embedded images still show it; the plain-text version has your name only. Without an operator key and without an operator name, the logo is not shown: the "Managed by" line it belongs to needs a name.

Under the name and logo, **How your emails look** shows a client report as it goes out now: the sender name, the top of the email (see [What a key changes](#what-a-key-changes)), its title and its last line. Under it, the same email in the other state: with an operator license while you have none, without one while you do. It follows what you type before you save. Without an operator name, emails go out as Marcan.

## The license

Marcan is licensed under the **Beneficiary Copyleft License 1.0**: the terms of the GNU Affero General Public License v3 (AGPLv3), plus the duty to tell clients described above, under Swiss law. The full text is in [`LICENSE`](https://github.com/mavgit1/marcan-dmarc/blob/main/LICENSE) and, unchanged for good, at [/licenses/bcl-1.0](/licenses/bcl-1.0). The AGPLv3 it builds on is in [`AGPL-3.0.txt`](https://github.com/mavgit1/marcan-dmarc/blob/main/AGPL-3.0.txt).

Because of that extra duty, this is not an OSI-approved open source license, even though all the code is open. If your organization's license policy needs a review for that reason, the questions below and the license text are short.

## Questions

**We monitor our own domains and those of a few clients. Do we need a license?**
Only for the client domains: tell those clients, or buy a license sized for them. Your own domains never count.

**We set Marcan up on a client's own server and look after it. Does that count?**
Yes. Administering it for a client counts as running it for them, even when it runs on their server.

**May I modify Marcan?**
Yes. If you run a modified version for others over a network, its source must be available to them, unless you hold a license.

**May I share Marcan's code?**
Yes, with the license. Whoever receives it gets the same terms.
