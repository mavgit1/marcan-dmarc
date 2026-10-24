-- Alert events about the installation instead of a domain (a newer Marcan
-- release, dmarc_assess::AlertKind::UpdateAvailable): alert_events.domain_id
-- becomes nullable, NULL for such an event. SQLite cannot drop a NOT NULL,
-- so the table is rebuilt with the same columns, rows and indexes. No table
-- references alert_events. An event with a domain still goes with it.
CREATE TABLE alert_events_new (
    id INTEGER PRIMARY KEY,
    domain_id INTEGER REFERENCES domains(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    format INTEGER NOT NULL,
    params TEXT NOT NULL,
    notified_at INTEGER
);

INSERT INTO alert_events_new (id, domain_id, created_at, kind, severity, format, params, notified_at)
SELECT id, domain_id, created_at, kind, severity, format, params, notified_at FROM alert_events;

DROP TABLE alert_events;
ALTER TABLE alert_events_new RENAME TO alert_events;

CREATE INDEX idx_alert_events_created ON alert_events(created_at DESC);
CREATE INDEX idx_alert_events_domain ON alert_events(domain_id, created_at DESC);
CREATE INDEX idx_alert_events_unnotified ON alert_events(created_at, id) WHERE notified_at IS NULL;
