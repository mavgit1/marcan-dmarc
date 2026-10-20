---
layout: home

hero:
  name: Marcan
  text: Self-hosted DMARC report processing
  tagline: Ingests DMARC aggregate reports from an IMAP mailbox, checks your domains' live SPF, DKIM and DMARC records, and shows the results in a web dashboard.
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
  - title: Single binary
    details: API, embedded dashboard and IMAP ingestion worker in one executable, storing everything in SQLite. No external database or Node runtime.
  - title: Independent alignment checks
    details: Recomputes SPF and DKIM alignment (RFC 7489) with the Public Suffix List instead of trusting each reporter's verdict.
  - title: Live DNS checks
    details: SPF include trees and lookup limits, DMARC policy and report authorization, DKIM keys, MTA-STS, TLS-RPT and BIMI, graded by a DNS score.
  - title: Forwarding-aware failures
    details: Splits DMARC failures into blocked by policy, forwarded and real delivered failures, so mailing lists are not mistaken for spoofing.
  - title: Attention list
    details: One row per domain with every open issue, ranked by severity rather than mail volume, so low-volume domains are not hidden.
  - title: Sender identity from evidence
    details: Names senders from forward-confirmed reverse DNS and passing SPF, not from a list of provider IP ranges.
---

<div class="home-screenshot">
  <Screenshot name="dashboard" alt="The Marcan dashboard" />
</div>
