-- Marcan's schema. Squashed before v0.1.0; from v0.1.0 on, an applied
-- migration is never edited: changes go into a new migrations/NNNN_*.sql.

-- Exactly one organization is the default: domains first seen in reports land there.
CREATE TABLE organizations (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    is_default INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX idx_organizations_single_default ON organizations(is_default) WHERE is_default = 1;

INSERT INTO organizations (id, name, is_default) VALUES (1, 'Default', 1);

-- A domain belongs to exactly one organization. Names are stored normalized
-- (trimmed, lowercase, no trailing dot) so the global UNIQUE holds.
-- parked: the domain sends no mail of its own.
-- ignored: not ours (removed). Its reports are deleted and new ones dropped at
-- import; un-ignoring it and re-importing the mailbox brings the history back.
CREATE TABLE domains (
    id INTEGER PRIMARY KEY,
    organization_id INTEGER NOT NULL REFERENCES organizations(id),
    domain TEXT NOT NULL UNIQUE,
    parked INTEGER NOT NULL DEFAULT 0,
    ignored INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_domains_organization ON domains(organization_id);

-- compliant_messages is the sum of record counts with dmarc_pass = 1
-- (auth-result pass + PSL alignment), not the reporter's policy_evaluated.
-- report_id is only unique per reporting organization (RFC 7489 §7.2.1.1), so
-- two reporters may use the same id. The raw XML lives in report_xml.
-- policy_p / policy_sp / policy_pct are the receiver's policy_published, shown
-- with that report and read by policy readiness (the policy each receiver
-- applied); a domain's own policy comes from its DNS check (dns_snapshots).
CREATE TABLE reports (
    id INTEGER PRIMARY KEY,
    report_id TEXT NOT NULL,
    org_name TEXT NOT NULL,
    email TEXT NOT NULL,
    domain_id INTEGER NOT NULL REFERENCES domains(id) ON DELETE CASCADE,
    date_begin INTEGER NOT NULL,
    date_end INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    policy_p TEXT,
    policy_sp TEXT,
    policy_pct INTEGER,
    total_messages INTEGER NOT NULL,
    compliant_messages INTEGER NOT NULL,
    UNIQUE (org_name, report_id)
);

CREATE INDEX idx_reports_domain_date ON reports(domain_id, date_begin DESC);
-- total_messages makes this a covering index for the health endpoint's
-- whole-table message total.
CREATE INDEX idx_reports_date ON reports(date_begin DESC, total_messages);
-- The health poll's newest ingest time.
CREATE INDEX idx_reports_created ON reports(created_at);
-- Retention's DELETE by date_end, and each domain's newest report.
CREATE INDEX idx_reports_date_end ON reports(date_end);
CREATE INDEX idx_reports_domain_date_end ON reports(domain_id, date_end);

-- Each report's original XML, gzip-compressed, kept apart so the hot reports
-- table stays small. Nothing reads it today; it keeps old reports
-- reprocessable when parsing changes, whatever the mailbox still holds.
CREATE TABLE report_xml (
    report_id INTEGER PRIMARY KEY REFERENCES reports(id) ON DELETE CASCADE,
    xml_gz BLOB NOT NULL
);

-- What happened to the record, computed at ingest (dmarc_assess::RecordFacts)
-- so every screen reads plain columns: spf_aligned_pass / dkim_aligned_pass
-- say which mechanism passed for a domain aligned with the From domain (PSL),
-- dmarc_pass is either of them (the CHECK holds it there), and
-- forwarded says the receiver trusted a forwarder (an override reason, or an
-- `arc=pass` comment). disposition is stored trimmed and lowercase, like auth
-- results and override reason types; the per-check rows are kept in
-- record_spf_auth / record_dkim_auth / record_overrides for display.
CREATE TABLE records (
    id INTEGER PRIMARY KEY,
    report_id INTEGER NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
    source_ip TEXT NOT NULL,
    count INTEGER NOT NULL,
    disposition TEXT,
    dkim_result TEXT,
    spf_result TEXT,
    header_from TEXT,
    envelope_from TEXT,
    envelope_to TEXT,
    dmarc_pass INTEGER NOT NULL DEFAULT 0,
    spf_aligned_pass INTEGER NOT NULL DEFAULT 0,
    dkim_aligned_pass INTEGER NOT NULL DEFAULT 0,
    forwarded INTEGER NOT NULL DEFAULT 0,
    CHECK (dmarc_pass = (spf_aligned_pass OR dkim_aligned_pass))
);

CREATE INDEX idx_records_report ON records(report_id);
-- Everything keyed by source IP: sender detail, the senders list's cross-domain
-- count, IP lookups. Also serves plain `source_ip` lookups (leftmost column).
CREATE INDEX idx_records_ip_report ON records(source_ip, report_id);

-- Every SPF / DKIM check and override reason of a record, as the reporter
-- listed them, with the alignment computed at ingest (PSL, relaxed/strict per
-- the report's aspf / adkim). An SPF result scoped `helo` is only aligned when
-- the record has no other SPF result (RFC 7489 §4.1). Shown per record and
-- used to name senders; the counts come from the records columns.
CREATE TABLE record_dkim_auth (
    id INTEGER PRIMARY KEY,
    record_id INTEGER NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    domain TEXT,
    selector TEXT,
    result TEXT,
    aligned INTEGER
);

CREATE TABLE record_spf_auth (
    id INTEGER PRIMARY KEY,
    record_id INTEGER NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    domain TEXT,
    scope TEXT,
    result TEXT,
    aligned INTEGER
);

CREATE TABLE record_overrides (
    id INTEGER PRIMARY KEY,
    record_id INTEGER NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    reason_type TEXT,
    comment TEXT
);

-- Child-table lookups from a record.
CREATE INDEX idx_record_dkim_auth_record_id ON record_dkim_auth(record_id);
CREATE INDEX idx_record_spf_auth_record_id ON record_spf_auth(record_id);
CREATE INDEX idx_record_overrides_record_id ON record_overrides(record_id);

-- Passes for a domain other than the aligned one (sender identity, sender detail).
CREATE INDEX idx_spf_auth_other_pass ON record_spf_auth(record_id, domain)
  WHERE result = 'pass' AND aligned = 0;

-- IMAP UID cursor per mailbox ("host|port|username", see ImapConfig::mailbox_key)
-- and folder, so new mail is fetched without relying on \Seen. When UIDVALIDITY
-- changes, UIDs are invalidated and the import restarts from last_uid = 0.
CREATE TABLE imap_cursor (
    mailbox TEXT NOT NULL,
    folder TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    last_uid INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (mailbox, folder)
);

-- Messages the import gave up on (not a report, unreadable, too large, or a
-- repeated error). The cursor skips past them; this keeps the loss countable.
-- Stores UID + error only, not the message. Keyed by uid_validity because UIDs
-- repeat across generations.
CREATE TABLE imap_quarantine (
    mailbox TEXT NOT NULL,
    folder TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    uid INTEGER NOT NULL,
    last_failed_at INTEGER NOT NULL,
    error TEXT NOT NULL,
    PRIMARY KEY (mailbox, folder, uid_validity, uid)
);

-- Consecutive failed attempts on one message (kind 'message') or one download
-- batch (kind 'batch', keyed by its last UID), so a message that keeps failing
-- is quarantined after a fixed number of passes even across restarts. Cleared
-- on success or quarantine, and with the cursor's generation.
CREATE TABLE imap_attempts (
    mailbox TEXT NOT NULL,
    folder TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    kind TEXT NOT NULL,
    uid INTEGER NOT NULL,
    attempts INTEGER NOT NULL,
    last_failed_at INTEGER NOT NULL,
    PRIMARY KEY (mailbox, folder, uid_validity, kind, uid)
);

-- One row per finished mailbox pass (scheduled or manual), for the status
-- display: when the mailbox was last checked, when a pass last succeeded, and
-- the error of the latest pass if it failed. checked_at is NULL when the pass
-- never reached the mailbox search (could not connect, say).
CREATE TABLE imap_passes (
    id INTEGER PRIMARY KEY,
    finished_at INTEGER NOT NULL,
    checked_at INTEGER,
    error TEXT
);

-- Last complete live DNS check per monitored domain, so page loads do not wait
-- on the public internet. checker_version is the scoring logic that produced
-- the row; an older one is shown marked "updating" and re-checked first.
-- grade, score and critical_count are the check's verdicts cached beside it,
-- valid for that checker_version. The DMARC record the check found is kept as
-- columns, written with the row: dmarc_record_count records were published,
-- dmarc_record is the record when there is exactly one, and the policy_* tags
-- are its parsed tags (NULL without a record). Every list, sort and readiness
-- reads the policy from these columns, never from report_json, which only
-- the server reads back as a whole.
CREATE TABLE dns_snapshots (
    domain_id INTEGER PRIMARY KEY REFERENCES domains(id) ON DELETE CASCADE,
    checked_at INTEGER NOT NULL,
    grade TEXT NOT NULL,
    score INTEGER NOT NULL,
    critical_count INTEGER NOT NULL,
    dmarc_record_count INTEGER NOT NULL,
    dmarc_record TEXT,
    policy_p TEXT,
    policy_sp TEXT,
    policy_pct INTEGER,
    policy_adkim TEXT,
    policy_aspf TEXT,
    report_json TEXT NOT NULL,
    checker_version INTEGER NOT NULL
);

-- Persisted reverse-DNS identity per source IP, refreshed by a background loop.
-- ptr_confirmed is tri-state: NULL when there was no PTR to confirm, 1 when
-- forward confirmation passed, 0 when it failed. Do not collapse NULL into 0.
CREATE TABLE identity_snapshots (
    ip TEXT PRIMARY KEY,
    checked_at INTEGER NOT NULL,
    ptr_hostname TEXT,
    ptr_confirmed INTEGER
);

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- The operator's uploaded logo (Settings → Branding), kept apart so loading
-- the settings never reads the image. At most one row. media_type is
-- image/png, image/jpeg or image/webp, as the server sniffed it; width and
-- height are its size in pixels, read from its header at upload (a logo
-- whose size cannot be read is refused), from which the emails size it.
CREATE TABLE branding_logo (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    media_type TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    image BLOB NOT NULL
);

-- DKIM selectors the DNS checker probes for every domain in addition to the
-- selectors seen in reports. Stored lowercase and trimmed.
CREATE TABLE dkim_selectors (
    selector TEXT PRIMARY KEY
);

-- Sender display-name overrides edited in Settings. A target is an IP, an IP
-- range (CIDR) or a domain; a NULL name hides the bundled (parsedmarc) name.
CREATE TABLE sender_names (
    target TEXT PRIMARY KEY,
    name TEXT
);

-- Alerts. alert_state is the judged state each domain was last evaluated in
-- (JSON, dmarc_assess::AlertState); the next evaluation is compared with it.
-- A domain without a row has not been evaluated: its first evaluation is a
-- baseline and raises nothing. format is the JSON shape's version
-- (dmarc_assess::ALERT_FORMAT) in both tables: a migration that changes the
-- shape bumps it and converts the stored rows, and a row of another format is
-- not read.
CREATE TABLE alert_state (
    domain_id INTEGER PRIMARY KEY REFERENCES domains(id) ON DELETE CASCADE,
    format INTEGER NOT NULL,
    state TEXT NOT NULL,
    evaluated_at INTEGER NOT NULL
);

-- What each evaluation found changed: kind is the event's name
-- (dmarc_assess::AlertKind), severity its colour (dmarc_assess::Status), params
-- the whole event as JSON. notified_at is set once an alert email carried it.
-- The UI's alert list reads this table, and so would a status endpoint.
CREATE TABLE alert_events (
    id INTEGER PRIMARY KEY,
    domain_id INTEGER NOT NULL REFERENCES domains(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    format INTEGER NOT NULL,
    params TEXT NOT NULL,
    notified_at INTEGER
);

CREATE INDEX idx_alert_events_created ON alert_events(created_at DESC);
CREATE INDEX idx_alert_events_domain ON alert_events(domain_id, created_at DESC);
-- The delivery run's "not mailed yet" since a time, oldest first.
-- Events older than the report retention are deleted with the reports.
CREATE INDEX idx_alert_events_unnotified ON alert_events(created_at, id) WHERE notified_at IS NULL;

-- Client reports: who gets each organization's monthly summary email, as the
-- Client reports list under Settings → Lists saves it (the whole list is
-- replaced on every save). Deleting an organization drops its recipients.
CREATE TABLE client_report_recipients (
    organization_id INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    email TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (organization_id, email)
);

-- Client reports sent: one row per recipient and month (the month of the
-- report time, 'YYYY-MM'), written as each email goes out. A run that stops
-- part way (a restart, a refusing server) leaves the rest unsent, and the
-- next hourly run sends only those; nobody gets a month's report twice.
-- Settings → Client reports' "Last sent" is the newest sent_at.
CREATE TABLE client_report_sends (
    organization_id INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    email TEXT NOT NULL COLLATE NOCASE,
    month TEXT NOT NULL,
    sent_at INTEGER NOT NULL,
    PRIMARY KEY (organization_id, email, month)
);

-- Client reports not delivered: one row per recipient and month whose email
-- failed, as the server classified the failure: permanent (a 5xx answer, an
-- invalid address: not tried again this month) or not (tried again on each
-- hourly run for a few days from first_failed_at). cause is the failure in a
-- few words ("mailbox does not exist"), detail the server's whole answer. The
-- row is deleted once the email goes out. Settings → Client reports lists
-- the month's rows of listed recipients under the send line.
CREATE TABLE client_report_failures (
    organization_id INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    email TEXT NOT NULL COLLATE NOCASE,
    month TEXT NOT NULL,
    permanent INTEGER NOT NULL CHECK (permanent IN (0, 1)),
    cause TEXT NOT NULL,
    detail TEXT NOT NULL,
    first_failed_at INTEGER NOT NULL,
    last_failed_at INTEGER NOT NULL,
    PRIMARY KEY (organization_id, email, month)
);
