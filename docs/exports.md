# Exports & API

## CSV exports

Every **Export** button downloads a CSV for the page's current scope (organization, domain, time range), without a row limit:

| Export | Contents |
| :--- | :--- |
| `/api/export/domains.csv` | One row per domain: organization, DNS grade, policy, parked, message totals (delivered failures, forwarded, blocked by policy, passing without aligned DKIM), `health` (`no_data`, `all_blocked`, `protected`, `warning`, `critical`) and `issues`, the open attention issues separated by `;` (`delivered_failures`, `dns_grade`, `not_enforcing`, `parked_not_rejecting`, `parked_passing_mail`, `dkim_coverage`, `reports_stopped`) |
| `/api/export/senders.csv` | One row per sending IP and domain, including passing messages without aligned DKIM. `sender_name`, `operator_domain` and `ptr_hostname` are optional enrichment from stored evidence and may be empty; they change when reverse DNS or the name map does |
| `/api/export/auth-issues.csv` | Report records where SPF or DKIM did not pass for the domain, with an `outcome` column (`delivered`, `forwarded`, `blocked`, `passed`); filterable like the domain's Failures tab with `outcome=all_failures\|delivered\|forwarded\|blocked\|dkim_only\|spf_only` (one value, or several separated by commas: `outcome=blocked,dkim_only` lists records of either kind; `all_failures` is every DMARC failure, delivered, forwarded and blocked) and `source_ip` |
| `/api/export/dns-findings.csv` | The findings of each domain's latest DNS check |

### Columns

In order:

- **domains.csv**: `organization`, `domain`, `policy_p`, `policy_sp`, `policy_pct`, `total_messages`, `compliant_messages`, `delivered_compliance_pct`, `delivered_failures`, `forwarded_failures`, `blocked_by_policy`, `no_aligned_dkim_messages`, `parked`, `reporter_count`, `report_count`, `last_report_at`, `dns_grade`, `dns_score`, `dns_critical_count`, `dns_checked_at`, `health`, `issues`, `policy_source`, `readiness`
- **senders.csv**: `organization`, `domain`, `source_ip`, `sender_name`, `operator_domain`, `ptr_hostname`, `total_messages`, `compliant_messages`, `compliance_pct`, `spf_aligned_pass_messages`, `dkim_aligned_pass_messages`, `no_aligned_dkim_messages`, `delivered_failures`, `forwarded_failures`, `blocked_by_policy`, `envelope_from`, `dkim_selectors`, `reporters`, `report_count`, `first_seen`, `last_seen`
- **auth-issues.csv**: `report_id`, `record_id`, `date_begin`, `date_end`, `reporter`, `organization`, `domain`, `source_ip`, `messages`, `dmarc_pass`, `spf_aligned_pass`, `dkim_aligned_pass`, `outcome`, `issue`, `disposition`, `delivered`, `header_from`, `envelope_from`, `spf_domain`, `dkim_domain`, `dkim_selector`, `reporter_spf_verdict`, `reporter_dkim_verdict`, `policy_override`
- **dns-findings.csv**: `organization`, `domain`, `checked_at`, `grade`, `score`, `check`, `severity`, `code`, `points`, `message`, `suggestion`

In **domains.csv**, `policy_p`, `policy_sp` and `policy_pct` are the policy of the domain's DMARC record as its last DNS check found it. The fields are empty for a domain not checked yet or without a DMARC record; the policy in its reports is not used. `policy_source` says which: `dns` (the record's policy), `not_published` (the check found no DMARC record), `not_checked` (no check yet) or `check_failed` (no check stored and the latest attempt failed). `readiness` is the domain's [policy readiness](./readiness) now, judged on the last 30 days whatever the export's time range: `ready`, `not_ready`, `raise_pct` (raise `pct` to 100 first), `enforced` (`p=reject` at 100%) or `not_applicable` (no published policy, or a parked domain). `delivered_compliance_pct` is the share of delivered mail that passed DMARC, `compliant_messages / (compliant_messages + delivered_failures)`, the rate `health` in the same row is judged on and the Domains page shows; blocked and forwarded mail is not in it. It is empty when nothing was delivered.

In **senders.csv**, `compliance_pct` is a sender's raw pass rate, `compliant_messages / total_messages`: blocked mail from a legitimate sender is lost mail, so it counts against the sender.

In **auth-issues.csv**, `issue` says which check lacks a pass for a domain aligned with the From domain: `spf_failed` (DKIM passed), `dkim_failed` (SPF passed) or `both_failed`.

### Format

Files start with a UTF-8 BOM and use CRLF line ends. Report content is written by whoever sends a report, so a value a spreadsheet could read as a formula (starting with `=`, `+`, `-`, `@`, a tab or a carriage return) gets a leading `'`, and so does each such item of a `;`-separated list. Nothing else is changed or dropped.

### Automating

Each export is a plain `GET` request, so a script behind the same proxy can fetch it with the scope as query parameters, for example `?organization_id=2&since=30d`. To get the exact URL, copy the download link from the export dialog in the UI.

## JSON API

The JSON API under `/api` is what the UI uses. It is not a stable interface in 0.x releases and may change between versions. Use the CSV exports for anything you automate.

`GET /healthz` answers `ok` and needs no login in the example deployments, for uptime monitors. `GET /api/system/health` has the import status in detail and needs a login like everything else.

