# Policy readiness

Marcan tells you when a domain can move to the next DMARC policy, from `p=none` to `p=quarantine`, then to `p=reject`, and what to publish when it can. The verdict is decided on the server from the last 30 days of reports and the latest DNS check; the domain's page, the [client reports](./client-reports), the `readiness` column of [`domains.csv`](./exports#columns) and the [**Ready for the next policy step**](./alerts#what-is-alerted) alert all use the same one.

<Screenshot name="domain" alt="A domain's overview: at p=none and ready for p=quarantine, messages per day with the four outcomes, the DNS grade and the checks behind it" />

## When a domain is ready

A domain's page says, under its name, whether the domain is ready for the next DMARC policy: from `p=none` to `p=quarantine`, then to `p=reject`. A domain is **ready** when, over the **last 30 days**:

- at least **200 messages** were reported under its current policy, so a domain with a handful of messages is never called ready,
- reports under its current policy cover all 30 days,
- at least **95%** of the mail was evaluated under its current policy, and
- 99% or more of its delivered mail passed DMARC (delivered compliance, as for health: forwarded and blocked mail do not count against it).

The current policy is the `p=` of the DMARC record in DNS. Each report says which policy the receiver applied, and only mail evaluated under the current one counts toward the 200 messages and the 30 days: the evidence that a domain is ready for `p=quarantine` says nothing about `p=reject`. So after you publish `p=quarantine`, the verdict for `p=reject` starts again at 0 of 30 days and waits for reports under the new policy. Some receivers keep applying the old policy for a while (they read DNS before the change, or report late); as long as that is under 5% of the mail it does not hold the verdict back. Delivered compliance counts all of the mail, since whether a message passes DMARC does not depend on the policy applied. For a domain at `p=none` all reports carry `p=none`, so all of its mail counts.

## When it is not ready yet

The 30 days are always the last 30, whatever time range the page shows, so the verdict does not change with the range. When a domain is not ready, the Enforcement tab gives one reason, the most fundamental first: too little mail under the current policy (how many of the 200), not enough history under it (how many of the 30 days its reports cover so far), receivers still applying the old policy (to how much of the mail), or delivered compliance below 99%.

## pct, testing and enforced

A policy with `pct=` below 100 applies to only part of the failing mail: the page asks you to raise it to 100 before the next step. A policy published with `t=y` (testing, [RFC 9989](./dns-checks#dmarcbis-rfc-9989)) counts like `pct=0`, since receivers that follow RFC 9989 do not apply it: the page asks you to remove `t=y` first, and the records it suggests leave it out. `p=reject` at 100% reads **Enforced**: there is nothing left to tighten. Parked domains and domains without a published policy get no readiness line; a parked domain should publish `p=reject` straight away.

## The Enforcement tab

The line under the name is the verdict alone (**Ready for p=quarantine**, **Not ready for p=reject yet**, **Enforced**) with a **Details** link to the domain's **Enforcement** tab, which gives the reason and shows what the verdict was based on. Opening it checks nothing new; it shows the figures the verdict used:

- **The verdict** in one sentence, with the reason (including **Enforced**, and why a domain without a policy is not judged). A domain at `p=quarantine` that is ready for `p=reject` also gets a line with how many messages were blocked by policy in the 30 days, since `p=reject` refuses that mail instead of sending it to spam.
- **What the verdict is based on:** the current policy (the DMARC record in DNS; a domain without one is not judged), and each rule as what the last 30 days showed against what it needs, met or not: messages ("140 under p=quarantine, needs 200"), report history ("0 of 30 days under p=quarantine"), the mail evaluated under the current policy ("88.0%, needs 95%", with how much was still evaluated under the old one when it is below) and delivered compliance ("98.2%, needs 99%"). The verdict names only the first unmet rule as the reason; the list shows all four.
- **Who would be affected:** the senders whose mail failed DMARC in the window, with how many messages failed, the largest first. Failed means delivered anyway or blocked by policy, the same count as on the Senders page. Forwarded mail is left out, since receivers deliver it when they trust the forwarder; a line says how much there was.
- **What to publish:** the `_dmarc` TXT record as the last DNS check found it, and the next records in order (`p=quarantine`, then `p=reject`, each at the full `pct`), written into your current record so every other tag stays as it is, each with a copy button. When the domain is ready, the record it is ready for stands out with a **Copy record** button; the others are smaller. When it is not ready yet, the steps read **Once ready** and none stands out. A step after the first waits until the domain is ready again. An enforced domain shows only its current record.

## Parked domains

A parked domain (marked in the [menu at the top right of its page](./domains#the-domain-menu), or with **Mark as parked** beside a "too little mail" verdict) has its own layout: **Publish p=reject now** with that record first, a new `v=DMARC1; p=reject; rua=mailto:<report address>` when the domain has no DMARC record yet (**Enforced** once it is published), then every message sent in its name in the 30 days, failing or not, since a parked domain should send none.
