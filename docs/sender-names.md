# Sender names

Reports identify senders only by IP address. Marcan works out the **operator domain** behind an IP (see [Sender identity](./how-it-works#sender-identity)), for example `amazonses.com` or `hostpoint.ch`, and turns it into a display name with the [parsedmarc](https://github.com/domainaware/parsedmarc) reverse-DNS map that is bundled into the binary.

Names are captions only. They never change a verdict.

<Screenshot name="senders" alt="The senders list, each IP with its sender's name, messages, failed mail and DMARC pass rate" />

## Your own names

To rename an operator, or to name your own servers, edit **Settings → Sender names**:

```csv
crm.example.com, Our CRM
192.0.2.10, Office printer
192.0.2.0/24, Office
hostpoint.ch,
```

One `target, name` per line. The target is one of:

- an **IP address**,
- an **IP range** in CIDR notation,
- an **operator domain**: the registrable domain of the reverse-DNS name, or of the SPF domain,
- a **host name** under an operator domain: `crm.example.com` names that host and the hosts below it.

The most specific match wins: an IP, then the narrowest range, then the longest matching host name, then the operator domain. That way servers without reverse DNS can be named too.

- An empty name hides the name a target would otherwise get.
- `#` starts a comment.
- Further columns are ignored, so a file in parsedmarc's format can be pasted as it is.

Before you save, the editor says what will change: names added, changed and removed. Nothing is saved while a line has an error. Changes apply immediately.
