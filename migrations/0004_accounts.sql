-- Accounts: the mail accounts Marcan receives reports in and sends through,
-- in place of the one mailbox, the one sending login and the one report
-- address of 0.3. The settings groups are converted here, in SQL: `mailbox`
-- becomes account 1 and `receiving` (the poll interval for all accounts);
-- `sending` with its own login becomes account 2 and keeps what is not an
-- account; `reporting` becomes account 1's address and, in forwarded mode,
-- its forwarded addresses. The receive state is keyed by account and in
-- text, under names for any source (receive_*).
--
-- accounts: no secrets. address is the report address and the From of the
-- mail sent through the account (empty: none; unique, case folded, where
-- set). An empty imap_host means the account has no IMAP server, an empty
-- smtp_host that it cannot send; smtp_port NULL: not set. Ids are never
-- reused (AUTOINCREMENT): what is kept per account id elsewhere cannot pass
-- to a later account. login_kind 'password' logs in as username with the
-- password in account_secrets; 'microsoft' (OAuth 2.0 client credentials,
-- over Microsoft Graph) has no IMAP or SMTP server and proves itself with
-- the certificate in the ms_cert_* columns, its private key in
-- account_secrets.
--
-- receiving_since: when the account started reading the mailbox it reads
-- now (added, its receiving switched on, or a save made it read another
-- server, login or folder). An account not read successfully since counts
-- from here for the client-report hold and the "Account not read" alert, so
-- a new or replaced mailbox gets the same days as one that stopped being
-- read. A converted account counts from the upgrade.
--
-- receive_epoch: which mailbox's receive state the account holds. A save
-- that makes it read another mailbox forgets its cursor, quarantine, retry
-- counts and passes and adds one to this, in the same transaction. A pass
-- carries the value it was prepared under and writes only while it is still
-- the account's, so a pass running during such a save writes nothing more.
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'generic' CHECK (provider IN ('microsoft365', 'generic')),
    address TEXT NOT NULL DEFAULT '',
    login_kind TEXT NOT NULL DEFAULT 'password' CHECK (login_kind IN ('password', 'microsoft')),
    username TEXT NOT NULL DEFAULT '',
    ms_tenant_id TEXT,
    ms_client_id TEXT,
    ms_cert_der BLOB,
    ms_cert_thumbprint TEXT,
    ms_cert_expires_at INTEGER,
    imap_host TEXT NOT NULL DEFAULT '',
    imap_port INTEGER NOT NULL DEFAULT 993 CHECK (imap_port BETWEEN 1 AND 65535),
    imap_security TEXT NOT NULL DEFAULT 'tls' CHECK (imap_security IN ('tls', 'starttls')),
    smtp_host TEXT NOT NULL DEFAULT '',
    smtp_port INTEGER CHECK (smtp_port BETWEEN 1 AND 65535),
    smtp_security TEXT NOT NULL DEFAULT 'tls' CHECK (smtp_security IN ('tls', 'starttls')),
    receive_enabled INTEGER NOT NULL DEFAULT 1 CHECK (receive_enabled IN (0, 1)),
    folder TEXT NOT NULL DEFAULT 'INBOX',
    created_at INTEGER NOT NULL,
    receiving_since INTEGER NOT NULL DEFAULT 0,
    receive_epoch INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX idx_accounts_address ON accounts(lower(address)) WHERE address <> '';

-- An account's secrets, one per kind: the password, or a Microsoft login's
-- private key. value is plain (encrypted = 0: the UTF-8 text, nonce and
-- key_id NULL) or AES-256-GCM sealed (encrypted = 1: ciphertext and tag, the
-- 12-byte nonce, and the fingerprint of the key it was sealed with). Both
-- kinds of rows can be present at once.
CREATE TABLE account_secrets (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('password', 'private_key')),
    value BLOB NOT NULL,
    encrypted INTEGER NOT NULL CHECK (encrypted IN (0, 1)),
    nonce BLOB,
    key_id BLOB,
    PRIMARY KEY (account_id, kind),
    CHECK (
        (encrypted = 0 AND nonce IS NULL AND key_id IS NULL)
        OR (encrypted = 1 AND length(nonce) = 12 AND length(key_id) > 0)
    )
);

-- The other addresses whose reports reach an account, in the order they
-- were entered (rowid). An address belongs to one account (unique, case
-- folded); that it is no account's own address is the server's check.
CREATE TABLE account_forwarded_addresses (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    address TEXT NOT NULL CHECK (address <> '')
);

CREATE UNIQUE INDEX idx_account_forwarded_addresses_address
    ON account_forwarded_addresses(lower(address));
CREATE INDEX idx_account_forwarded_addresses_account
    ON account_forwarded_addresses(account_id);

-- The groups as 0.3 stored them, read once (dropped at the end). A missing
-- field reads as 0.3's default; NULL port or security as the usual ones, so
-- every converted server has its port.
CREATE TEMP TABLE m4_mailbox AS
SELECT
    trim(COALESCE(json_extract(value, '$.host'), '')) AS host,
    COALESCE(json_extract(value, '$.security'), 'tls') AS security,
    COALESCE(
        json_extract(value, '$.port'),
        CASE COALESCE(json_extract(value, '$.security'), 'tls') WHEN 'starttls' THEN 143 ELSE 993 END
    ) AS port,
    COALESCE(json_extract(value, '$.username'), '') AS username,
    COALESCE(json_extract(value, '$.password'), '') AS password,
    COALESCE(NULLIF(json_extract(value, '$.folder'), ''), 'INBOX') AS folder,
    json_extract(value, '$.poll_interval_secs') AS poll_interval_secs
FROM app_settings WHERE key = 'mailbox';

CREATE TEMP TABLE m4_reporting AS
SELECT
    NULLIF(trim(COALESCE(json_extract(value, '$.report_address'), '')), '') AS report_address,
    json_extract(value, '$.reporting_mode') AS reporting_mode
FROM app_settings WHERE key = 'reporting';

CREATE TEMP TABLE m4_sending AS
SELECT
    COALESCE(json_extract(value, '$.enabled'), 0) AS enabled,
    trim(COALESCE(json_extract(value, '$.host'), '')) AS host,
    -- 0.3 sent through the usual port for the security when none was set.
    COALESCE(
        NULLIF(json_extract(value, '$.port'), 0),
        CASE COALESCE(json_extract(value, '$.security'), 'tls') WHEN 'starttls' THEN 587 ELSE 465 END
    ) AS port,
    COALESCE(json_extract(value, '$.security'), 'tls') AS security,
    COALESCE(json_extract(value, '$.same_login_as_mailbox'), 1) AS same_login,
    COALESCE(json_extract(value, '$.username'), '') AS username,
    COALESCE(json_extract(value, '$.sender'), '') AS sender,
    COALESCE(json_extract(value, '$.reply_to'), '') AS reply_to,
    COALESCE(json_extract(value, '$.test_recipient'), '') AS test_recipient,
    COALESCE(json_extract(value, '$.password'), '') AS password
FROM app_settings WHERE key = 'sending';

-- Account 1, the mailbox: created when a mailbox host is set, or when only a
-- report address is (then with no login, which the worker skips). Its
-- address is the report address, else the username when that is an address
-- (0.3 sent from it); its name the address, else the username.
CREATE TEMP TABLE m4_account1 AS
SELECT
    COALESCE(m.host, '') AS host,
    COALESCE(m.port, 993) AS port,
    COALESCE(m.security, 'tls') AS security,
    COALESCE(m.username, '') AS username,
    COALESCE(m.password, '') AS password,
    COALESCE(m.folder, 'INBOX') AS folder,
    COALESCE(
        r.report_address,
        CASE
            WHEN instr(m.username, '@') > 1
                AND instr(substr(m.username, instr(m.username, '@') + 1), '.') > 0
            THEN m.username
        END,
        ''
    ) AS address,
    COALESCE(m.host, '') <> '' OR r.report_address IS NOT NULL AS created
FROM (SELECT 1) AS one
LEFT JOIN m4_mailbox AS m ON 1
LEFT JOIN m4_reporting AS r ON 1;

INSERT INTO accounts (
    id, name, address, username, imap_host, imap_port, imap_security,
    receive_enabled, folder, created_at, receiving_since
)
SELECT
    1, COALESCE(NULLIF(address, ''), NULLIF(username, ''), 'Mailbox'), address, username,
    host, port, security, 1, folder,
    CAST(strftime('%s', 'now') AS INTEGER), CAST(strftime('%s', 'now') AS INTEGER)
FROM m4_account1 WHERE created;

INSERT INTO account_secrets (account_id, kind, value, encrypted)
SELECT 1, 'password', CAST(password AS BLOB), 0
FROM m4_account1 WHERE created AND password <> '';

-- Sending with the mailbox's login: its server goes onto account 1 (as it
-- was, whatever its host).
UPDATE accounts SET
    smtp_host = (SELECT host FROM m4_sending),
    smtp_port = (SELECT port FROM m4_sending),
    smtp_security = (SELECT security FROM m4_sending)
WHERE id = 1 AND EXISTS (SELECT 1 FROM m4_sending WHERE same_login AND host <> '');

-- Sending with its own login: account 2, "Sending", which receives nothing.
-- Its address is the sender, else the username when that is an address,
-- unless another account already has it.
INSERT INTO accounts (
    id, name, address, username, smtp_host, smtp_port, smtp_security,
    receive_enabled, created_at, receiving_since
)
SELECT
    2, 'Sending',
    CASE
        WHEN lower(candidate) IN (SELECT lower(address) FROM accounts WHERE address <> '') THEN ''
        ELSE candidate
    END,
    username, host, port, security, 0,
    CAST(strftime('%s', 'now') AS INTEGER), CAST(strftime('%s', 'now') AS INTEGER)
FROM (
    SELECT *, CASE
        WHEN sender <> '' THEN sender
        WHEN instr(username, '@') > 1
            AND instr(substr(username, instr(username, '@') + 1), '.') > 0
        THEN username
        ELSE ''
    END AS candidate
    FROM m4_sending
) WHERE NOT same_login AND host <> '';

INSERT INTO account_secrets (account_id, kind, value, encrypted)
SELECT 2, 'password', CAST(password AS BLOB), 0
FROM m4_sending WHERE NOT same_login AND host <> '' AND password <> '';

-- What `sending` keeps: on or off, the account (none without a server), the
-- reply-to and the test recipient. Mail always comes from the sending
-- account's address, so 0.3's From goes: with its own login it became
-- account 2's address above; with the mailbox's login a different From is
-- dropped (whoever wants it adds an account with that address).
UPDATE app_settings SET value = (
    SELECT json_object(
        'enabled', json(CASE WHEN s.enabled THEN 'true' ELSE 'false' END),
        'account_id', a.id,
        'reply_to', s.reply_to,
        'test_recipient', s.test_recipient
    )
    FROM m4_sending AS s
    LEFT JOIN accounts AS a ON a.id = CASE
        WHEN s.host = '' THEN NULL
        WHEN s.same_login THEN 1
        ELSE 2
    END
)
WHERE key = 'sending';

-- `mailbox` becomes `receiving`: the poll interval, now for every account.
INSERT INTO app_settings (key, value)
SELECT 'receiving', CASE
    WHEN poll_interval_secs IS NULL THEN '{}'
    ELSE json_object('poll_interval_secs', poll_interval_secs)
END
FROM m4_mailbox;

-- 0.3's forwarded mode: each domain's record named an address of its own
-- that forwards to the one mailbox. Those addresses become account 1's (the
-- converted mailbox) forwarded addresses: every valid mailto: address that
-- the latest DNS check of a domain not ignored found in its rua=, other than
-- an account's own address, once each (case folded, the first spelling in
-- address order). 0.3's verdict counted the same addresses. Direct mode, or
-- no account 1: nothing.
INSERT INTO account_forwarded_addresses (account_id, address)
SELECT 1, MIN(address)
FROM (
    SELECT trim(json_extract(rua.value, '$.address')) AS address
    FROM dns_snapshots AS s
    JOIN domains AS d ON d.id = s.domain_id AND d.ignored = 0
    JOIN json_each(
        CASE WHEN json_valid(s.report_json) THEN s.report_json ELSE '{}' END,
        '$.dmarc.parsed.rua'
    ) AS rua
    WHERE json_type(rua.value) = 'object'
        AND json_extract(rua.value, '$.scheme') = 'mailto'
        AND json_extract(rua.value, '$.valid') = 1
        AND json_type(rua.value, '$.address') = 'text'
)
WHERE address <> ''
    AND EXISTS (SELECT 1 FROM accounts WHERE id = 1)
    AND EXISTS (SELECT 1 FROM m4_reporting WHERE reporting_mode = 'forwarded')
    AND lower(address) NOT IN (SELECT lower(address) FROM accounts WHERE address <> '')
GROUP BY lower(address)
ORDER BY lower(address);

DELETE FROM app_settings WHERE key IN ('mailbox', 'reporting');

-- The receive state, keyed by account instead of "host|port|username": the
-- rows of the converted mailbox go to account 1, every other row (a mailbox
-- configured before it) is dropped.
--
-- Keyed in text, so a mailbox read over another protocol than IMAP fits the
-- same tables: the folder generation (when it changes, the folder's
-- position, skipped messages and retry counts are void) and the position
-- and message ids are what the reading source writes, opaque to the store.
-- IMAP writes its numbers in decimal: the UIDVALIDITY as the generation,
-- the last handled UID as the position, a UID as the message id. The 0.3
-- rows are carried over in that spelling, so the mailbox keeps its
-- position, its skipped messages and its retry counts.
CREATE TEMP TABLE m4_key AS
SELECT lower(host) || '|' || port || '|' || username AS mailbox
FROM m4_account1 WHERE created AND host <> '';

CREATE TABLE receive_cursor (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder TEXT NOT NULL,
    generation TEXT NOT NULL,
    position TEXT NOT NULL,
    PRIMARY KEY (account_id, folder)
);
INSERT INTO receive_cursor (account_id, folder, generation, position)
SELECT 1, folder, CAST(uid_validity AS TEXT), CAST(last_uid AS TEXT) FROM imap_cursor
WHERE mailbox IN (SELECT mailbox FROM m4_key);
DROP TABLE imap_cursor;

CREATE TABLE receive_quarantine (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder TEXT NOT NULL,
    generation TEXT NOT NULL,
    message_id TEXT NOT NULL,
    last_failed_at INTEGER NOT NULL,
    error TEXT NOT NULL,
    PRIMARY KEY (account_id, folder, generation, message_id)
);
INSERT INTO receive_quarantine
    (account_id, folder, generation, message_id, last_failed_at, error)
SELECT 1, folder, CAST(uid_validity AS TEXT), CAST(uid AS TEXT), last_failed_at, error
FROM imap_quarantine
WHERE mailbox IN (SELECT mailbox FROM m4_key);
DROP TABLE imap_quarantine;

-- kind: 'message' (message_id names it) or 'batch' (named by the id of its
-- last message).
CREATE TABLE receive_attempts (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder TEXT NOT NULL,
    generation TEXT NOT NULL,
    kind TEXT NOT NULL,
    message_id TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    last_failed_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, folder, generation, kind, message_id)
);
INSERT INTO receive_attempts
    (account_id, folder, generation, kind, message_id, attempts, last_failed_at)
SELECT 1, folder, CAST(uid_validity AS TEXT), kind, CAST(uid AS TEXT), attempts, last_failed_at
FROM imap_attempts
WHERE mailbox IN (SELECT mailbox FROM m4_key);
DROP TABLE imap_attempts;

-- The recorded passes were the one mailbox's: account 1's, or gone with it.
-- yielded: the pass gave way to another account's before reading everything
-- (a check of the mailbox, not a success). error is Marcan's sentence;
-- error_detail the server's own answer behind it (Microsoft Graph's code and
-- message), when it gave one.
ALTER TABLE imap_passes RENAME TO receive_passes;
ALTER TABLE receive_passes ADD COLUMN account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE receive_passes ADD COLUMN yielded INTEGER NOT NULL DEFAULT 0 CHECK (yielded IN (0, 1));
ALTER TABLE receive_passes ADD COLUMN error_detail TEXT;
UPDATE receive_passes SET account_id = 1 WHERE EXISTS (SELECT 1 FROM accounts WHERE id = 1);
DELETE FROM receive_passes WHERE account_id IS NULL;
CREATE INDEX idx_receive_passes_account ON receive_passes(account_id, id);

DROP TABLE m4_key;
DROP TABLE m4_account1;
DROP TABLE m4_sending;
DROP TABLE m4_reporting;
DROP TABLE m4_mailbox;
