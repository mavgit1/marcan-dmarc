---
layout: home

hero:
  name: Marcan
  text: Self-hosted DMARC monitoring
  tagline: Reads your DMARC reports from a mailbox, checks your DNS live, and tells you what to do next. When a domain is ready for the next policy, what changed, which new sender fails.
  image:
    light: /logo-light.svg
    dark: /logo-dark.svg
    alt: Marcan logo
  actions:
    - theme: brand
      text: Get started
      link: /getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/mavgit1/marcan-dmarc

features:
  - title: Policy readiness
    details: For each domain, the next policy step and whether it is ready for it, judged on 30 days of reports. The senders it would affect, and the record to publish.
    link: /readiness
  - title: Alerts on changes
    details: An email when a DNS record or the policy changes, health drops, reports stop or a new sender fails DMARC. A digest that arrives even when nothing changed.
    link: /alerts
  - title: Monthly client reports
    details: One short email per organization about its own domains, under your name and logo. The month against the last, and the next step per domain.
    link: /client-reports
  - title: A setup that checks itself
    details: Mailbox, import, domains and the DNS records to publish, one step at a time. Each record is checked live, with what it still needs.
    link: /getting-started#first-start
  - title: Its own verdicts
    details: Recomputes SPF and DKIM alignment with the Public Suffix List, tells forwarded mail from spoofing, and grades SPF, DKIM, DMARC, MTA-STS, TLS-RPT and BIMI.
    link: /how-it-works
  - title: One binary, one SQLite file
    details: Dashboard, API and IMAP import in one executable. No external database. The login comes from the proxy you put in front.
    link: /deployment
---

Free for your own domains. Running it for clients: see [pricing](./pricing). {.home-note}

<div class="home-screenshot">
  <Screenshot name="dashboard" alt="The Marcan dashboard" />
</div>
