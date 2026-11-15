//! The accounts over a real database: the start pass and every key
//! transition, the locked state in receiving and sending, and the sending
//! configuration an account resolves to.

use super::secrets::tests::key;
use super::*;
use crate::license::test_keys::TEST_KEY;
use dmarc_store::{
    get_account_secret, init, insert_account, update_account, SecretWrite, SmtpSecurity,
};

/// A complete submission account, as the sending account resolves to.
pub(crate) fn config() -> SendConfig {
    send_config_for(
        &sender(),
        SendLogin::Password(SecretState::Readable("smtp secret".into())),
        &SendingSettings {
            enabled: true,
            ..SendingSettings::default()
        },
        &AppSettings::default(),
    )
    .expect("a complete account")
}

/// An account that receives and sends, with a password login.
fn sender() -> AccountFields {
    AccountFields {
        name: "Reports".into(),
        address: "dmarc@example.com".into(),
        username: "reports@example.com".into(),
        imap_host: "imap.example.com".into(),
        smtp_host: "smtp.example.com".into(),
        smtp_port: Some(465),
        ..AccountFields::default()
    }
}

async fn pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    (dir, pool)
}

/// An account with `password` stored plain (as 0.3 and the conversion left it).
async fn plain_account(pool: &SqlitePool, password: &str) -> i64 {
    insert_account(
        pool,
        None,
        &sender(),
        &[SecretRow::plain(SecretKind::Password, password)],
        0,
    )
    .await
    .unwrap()
}

async fn stored(pool: &SqlitePool, id: i64) -> SecretRow {
    get_account_secret(pool, id, SecretKind::Password)
        .await
        .unwrap()
        .unwrap()
}

fn sending_through(id: i64) -> AppSettings {
    let mut settings = AppSettings::default();
    settings.sending.enabled = true;
    settings.sending.account_id = Some(id);
    settings
}

const POLL: Duration = Duration::from_secs(300);

#[tokio::test]
async fn no_key_to_a_key_encrypts_every_plain_secret_at_start() {
    let (_dir, pool) = pool().await;
    let id = plain_account(&pool, " spaced ").await;
    // Without a key, nothing changes.
    Accounts::new(pool.clone(), None).start().await.unwrap();
    assert!(!stored(&pool, id).await.is_encrypted());

    let k = key(1);
    let k_id = k.id().to_vec();
    let accounts = Accounts::new(pool.clone(), Some(k));
    accounts.start().await.unwrap();
    let row = stored(&pool, id).await;
    assert!(row.is_encrypted());
    assert_eq!(row.key_id.as_deref(), Some(k_id.as_slice()));
    assert!(!row.value.windows(6).any(|w| w == b"spaced"));
    assert_eq!(
        accounts.secret(id, SecretKind::Password).await.unwrap(),
        SecretState::Readable(" spaced ".into())
    );
    let snapshot = accounts.snapshot().await.unwrap();
    assert_eq!(snapshot.lock(id), None);
    assert!(snapshot.ready_to_receive(snapshot.get(id).unwrap()));
    let cfg = accounts.mailbox_config(id, POLL).await.unwrap().unwrap();
    let cfg = cfg.as_imap().unwrap();
    assert_eq!(cfg.password, " spaced ");
    assert_eq!(cfg.id, id.to_string());
    assert_eq!(cfg.folder, "INBOX");
}

#[tokio::test]
async fn the_same_key_again_changes_nothing() {
    let (_dir, pool) = pool().await;
    let id = plain_account(&pool, "secret").await;
    Accounts::new(pool.clone(), Some(key(1)))
        .start()
        .await
        .unwrap();
    let sealed = stored(&pool, id).await;
    let accounts = Accounts::new(pool.clone(), Some(key(1)));
    accounts.start().await.unwrap();
    assert_eq!(stored(&pool, id).await, sealed);
    assert_eq!(
        accounts.secret(id, SecretKind::Password).await.unwrap(),
        SecretState::Readable("secret".into())
    );
}

#[tokio::test]
async fn another_key_locks_the_account_until_its_credentials_are_entered_again() {
    let (_dir, pool) = pool().await;
    let id = plain_account(&pool, "secret").await;
    Accounts::new(pool.clone(), Some(key(1)))
        .start()
        .await
        .unwrap();
    let sealed = stored(&pool, id).await;

    let accounts = Accounts::new(pool.clone(), Some(key(2)));
    // The start does not fail, and leaves the value for the old key.
    accounts.start().await.unwrap();
    assert_eq!(stored(&pool, id).await, sealed);
    assert_eq!(
        accounts.secret(id, SecretKind::Password).await.unwrap(),
        SecretState::Locked(LockReason::KeyChanged)
    );
    let snapshot = accounts.snapshot().await.unwrap();
    let account = snapshot.get(id).unwrap();
    assert_eq!(snapshot.lock(id), Some(LockReason::KeyChanged));
    // Listed (its login is complete) but neither read nor sent through.
    assert!(snapshot.can_receive(account));
    assert!(!snapshot.ready_to_receive(account));
    assert!(!snapshot.ready_to_send(account));
    assert_eq!(accounts.mailbox_config(id, POLL).await.unwrap(), None);
    match accounts.send_config(&sending_through(id)).await {
        Err(SendError::Locked { account, reason }) => {
            assert_eq!(
                (account.as_str(), reason),
                ("Reports", LockReason::KeyChanged)
            );
        }
        other => panic!("expected Locked, got {other:?}"),
    }

    // Entered again: stored with the current key, readable at once.
    let row = accounts.store(id, SecretKind::Password, "new secret");
    update_account(
        &pool,
        id,
        &sender(),
        &[(SecretKind::Password, SecretWrite::Set(row))],
    )
    .await
    .unwrap();
    assert_eq!(stored(&pool, id).await.key_id.as_deref(), Some(key(2).id()));
    assert_eq!(accounts.snapshot().await.unwrap().lock(id), None);
    let config = accounts.send_config(&sending_through(id)).await.unwrap();
    assert_eq!(config.smtp().unwrap().password, "new secret");
}

#[tokio::test]
async fn no_key_after_one_locks_the_account_and_stores_plain_again() {
    let (_dir, pool) = pool().await;
    let id = plain_account(&pool, "secret").await;
    Accounts::new(pool.clone(), Some(key(1)))
        .start()
        .await
        .unwrap();

    let accounts = Accounts::new(pool.clone(), None);
    accounts.start().await.unwrap();
    assert!(stored(&pool, id).await.is_encrypted());
    assert_eq!(
        accounts.snapshot().await.unwrap().lock(id),
        Some(LockReason::KeyMissing)
    );
    assert_eq!(accounts.mailbox_config(id, POLL).await.unwrap(), None);
    assert!(matches!(
        accounts.send_config(&sending_through(id)).await,
        Err(SendError::Locked {
            reason: LockReason::KeyMissing,
            ..
        })
    ));

    let row = accounts.store(id, SecretKind::Password, "typed again");
    assert!(!row.is_encrypted());
    update_account(
        &pool,
        id,
        &sender(),
        &[(SecretKind::Password, SecretWrite::Set(row))],
    )
    .await
    .unwrap();
    assert_eq!(
        accounts.secret(id, SecretKind::Password).await.unwrap(),
        SecretState::Readable("typed again".into())
    );
}

/// What `start` logs, as text.
async fn start_log(accounts: &Accounts) -> String {
    log_of(async { accounts.start().await.unwrap() }).await
}

/// What `work` logs, as text.
async fn log_of<T>(work: impl std::future::Future<Output = T>) -> String {
    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = {
        let buffer = buffer.clone();
        move || Log(buffer.clone())
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .finish();
    {
        let _guard = tracing::subscriber::set_default(subscriber);
        work.await;
    }
    let bytes = buffer.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

struct Log(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const UNENCRYPTED: &str = "MARCAN_SECRET_KEY is not set: account secrets are stored unencrypted";

#[tokio::test]
async fn without_a_key_the_unencrypted_notice_while_a_secret_is_plain() {
    let (_dir, pool) = pool().await;
    let id = plain_account(&pool, "secret").await;
    let log = start_log(&Accounts::new(pool.clone(), None)).await;
    assert!(log.contains(UNENCRYPTED), "{log}");
    assert!(!log.contains("account locked"), "{log}");

    // Sealed, then started without the key: only the locked account is
    // logged, once, and its first pass does not log it again.
    Accounts::new(pool.clone(), Some(key(1)))
        .start()
        .await
        .unwrap();
    let keyless = Accounts::new(pool.clone(), None);
    let log = start_log(&keyless).await;
    assert!(!log.contains(UNENCRYPTED), "{log}");
    assert_eq!(log.matches("account locked").count(), 1, "{log}");
    assert!(
        log.contains("MARCAN_SECRET_KEY is not set. Restore"),
        "{log}"
    );
    let pass = log_of(keyless.mailbox_config(id, POLL)).await;
    assert!(!pass.contains("account locked"), "{pass}");

    // Beside it, one stored plain: the notice is back.
    insert_account(
        &pool,
        None,
        &AccountFields {
            address: String::new(),
            ..sender()
        },
        &[SecretRow::plain(SecretKind::Password, "second")],
        0,
    )
    .await
    .unwrap();
    let log = start_log(&Accounts::new(pool.clone(), None)).await;
    assert!(log.contains(UNENCRYPTED), "{log}");
    assert!(log.contains("account locked"), "{log}");
}

#[tokio::test]
async fn plain_and_encrypted_secrets_side_by_side() {
    let (_dir, pool) = pool().await;
    let sealed_id = plain_account(&pool, "first").await;
    let k = key(1);
    let accounts = Accounts::new(pool.clone(), Some(key(1)));
    accounts.start().await.unwrap();
    // Written plain after the start (a key added while another process ran).
    let plain_id = insert_account(
        &pool,
        None,
        &AccountFields {
            address: String::new(),
            ..sender()
        },
        &[SecretRow::plain(SecretKind::Password, "second")],
        0,
    )
    .await
    .unwrap();
    assert!(stored(&pool, sealed_id).await.is_encrypted());
    assert!(!stored(&pool, plain_id).await.is_encrypted());
    for (id, text) in [(sealed_id, "first"), (plain_id, "second")] {
        assert_eq!(
            accounts.secret(id, SecretKind::Password).await.unwrap(),
            SecretState::Readable(text.into())
        );
    }
    let snapshot = accounts.snapshot().await.unwrap();
    assert_eq!(
        (snapshot.lock(sealed_id), snapshot.lock(plain_id)),
        (None, None)
    );

    // Without the key the plain one still reads, the sealed one is locked.
    let keyless = Accounts::new(pool.clone(), None);
    let snapshot = keyless.snapshot().await.unwrap();
    assert_eq!(snapshot.lock(sealed_id), Some(LockReason::KeyMissing));
    assert_eq!(snapshot.lock(plain_id), None);
    assert!(keyless
        .mailbox_config(plain_id, POLL)
        .await
        .unwrap()
        .is_some());

    // The next start with the key seals the plain one too.
    Accounts::new(pool.clone(), Some(k)).start().await.unwrap();
    assert!(stored(&pool, plain_id).await.is_encrypted());
}

#[tokio::test]
async fn sending_needs_an_account_and_says_so() {
    let (_dir, pool) = pool().await;
    let accounts = Accounts::new(pool.clone(), None);
    let mut settings = AppSettings::default();
    assert_eq!(
        accounts.send_config(&settings).await,
        Err(SendError::Disabled)
    );
    settings.sending.enabled = true;
    let choose = Err(SendError::Incomplete(NO_SENDING_ACCOUNT.to_owned()));
    assert_eq!(accounts.send_config(&settings).await, choose);
    settings.sending.account_id = Some(9);
    assert_eq!(accounts.send_config(&settings).await, choose);
    let id = plain_account(&pool, "smtp secret").await;
    let config = accounts.send_config(&sending_through(id)).await.unwrap();
    let server = config.smtp().expect("sent over SMTP");
    assert_eq!(server.host, "smtp.example.com");
    assert_eq!(server.username, "reports@example.com");
    assert_eq!(config.sender.to_string(), "dmarc@example.com");
}

fn incomplete(account: &AccountFields, password: SecretState, sending: &SendingSettings) -> String {
    match send_config_for(
        account,
        SendLogin::Password(password),
        sending,
        &AppSettings::default(),
    ) {
        Err(SendError::Incomplete(why)) => why,
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

/// Mail comes from the account's address, whatever the login's username.
#[test]
fn the_account_s_address_is_the_sender() {
    let config = config();
    let server = config.smtp().expect("sent over SMTP");
    assert_eq!(server.username, "reports@example.com");
    assert_eq!(server.password, "smtp secret");
    assert_eq!(config.sender.to_string(), "dmarc@example.com");
    assert_eq!(server.security, SmtpSecurity::Tls);
    assert_eq!(config.reply_to, None);
    assert_eq!(config.sender_name, "Marcan");
    assert!(!format!("{config:?}").contains("secret"));

    let sending = SendingSettings {
        enabled: true,
        reply_to: "help@example.com".into(),
        ..SendingSettings::default()
    };
    let readable = || SecretState::Readable("p".into());
    let config = send_config_for(
        &sender(),
        SendLogin::Password(readable()),
        &sending,
        &AppSettings::default(),
    )
    .unwrap();
    assert_eq!(config.sender.to_string(), "dmarc@example.com");
    assert_eq!(config.reply_to.unwrap().to_string(), "help@example.com");
    let to_me = SendingSettings {
        test_recipient: "me@example.org".into(),
        ..SendingSettings::default()
    };
    assert_eq!(test_recipient(&to_me, Some(&sender())), "me@example.org");
    assert_eq!(
        test_recipient(&SendingSettings::default(), Some(&sender())),
        "dmarc@example.com"
    );
    assert_eq!(test_recipient(&SendingSettings::default(), None), "");
}

/// Without an operator key the From name says Marcan too; with one the
/// operator alone; the address stays the sender's either way.
#[test]
fn the_from_name_follows_the_letterhead() {
    let sending = SendingSettings::default();
    let config_with = |settings: &AppSettings| {
        send_config_for(
            &sender(),
            SendLogin::Password(SecretState::Readable("p".into())),
            &sending,
            settings,
        )
        .unwrap()
    };
    let mut s = AppSettings::default();
    s.branding.operator_name = "Acme IT".into();
    assert_eq!(config_with(&s).sender_name, "Acme IT via Marcan");
    // A key that does not verify changes nothing.
    s.license.operator_key = Some("test@example.com.AAAA".into());
    assert_eq!(config_with(&s).sender_name, "Acme IT via Marcan");
    s.license.operator_key = Some(TEST_KEY.into());
    let config = config_with(&s);
    assert_eq!(config.sender_name, "Acme IT");
    assert_eq!(config.sender.to_string(), "dmarc@example.com");
    assert_eq!(config.sender_name, Letterhead::new(&s).from_name());
    s.branding.operator_name.clear();
    assert_eq!(config_with(&s).sender_name, "Marcan");
}

#[test]
fn missing_pieces_are_named() {
    let on = SendingSettings::default();
    let readable = || SecretState::Readable("p".into());
    let with = |change: fn(&mut AccountFields)| {
        let mut a = sender();
        change(&mut a);
        a
    };
    assert_eq!(
        incomplete(&with(|a| a.smtp_host.clear()), readable(), &on),
        "Enter the SMTP server's host name."
    );
    // An unset port is the provider's usual one, as when it is saved.
    for unset in [None, Some(0)] {
        let mut account = sender();
        account.smtp_port = unset;
        let config = send_config_for(
            &account,
            SendLogin::Password(readable()),
            &on,
            &AppSettings::default(),
        )
        .expect("ready");
        assert_eq!(config.smtp().unwrap().port, 465);
    }
    assert!(
        incomplete(&with(|a| a.smtp_port = Some(25)), readable(), &on)
            .starts_with("Port 25 is for delivery between mail servers.")
    );
    // A Microsoft login sends over Graph as its address: no SMTP server,
    // no username.
    let microsoft = with(|a| {
        a.login_kind = LoginKind::Microsoft;
        a.smtp_host.clear();
        a.username.clear();
    });
    let token = || {
        MicrosoftToken::Ready(Arc::new(oauth::GraphAccountToken::new(
            Arc::new(oauth::TokenSource::microsoft()),
            oauth::tests::certificate_request(),
        )))
    };
    let graph = |token| SendLogin::Microsoft {
        api: GraphApi::microsoft(),
        token,
    };
    let config =
        send_config_for(&microsoft, graph(token()), &on, &AppSettings::default()).expect("ready");
    let mailbox = config.graph().expect("sent over Graph");
    assert_eq!(mailbox.address, "dmarc@example.com");
    assert_eq!(mailbox.api, GraphApi::microsoft());
    let no_mailbox = AccountFields {
        address: String::new(),
        ..microsoft.clone()
    };
    match send_config_for(&no_mailbox, graph(token()), &on, &AppSettings::default()) {
        Err(SendError::Incomplete(why)) => {
            assert_eq!(why, crate::wording::microsoft::ENTER_MAILBOX);
        }
        other => panic!("expected Incomplete, got {other:?}"),
    }
    let locked = graph(MicrosoftToken::Locked(LockReason::KeyMissing));
    assert!(matches!(
        send_config_for(&microsoft, locked, &on, &AppSettings::default()),
        Err(SendError::Locked { .. })
    ));
    assert_eq!(
        incomplete(&with(|a| a.username.clear()), readable(), &on),
        "Enter the SMTP username."
    );
    assert_eq!(
        incomplete(&sender(), SecretState::Missing, &on),
        "Enter the SMTP password."
    );
    assert_eq!(
        incomplete(&with(|a| a.address.clear()), readable(), &on),
        "Reports has no email address to send from. Enter one under Accounts."
    );
    assert_eq!(
        incomplete(&with(|a| a.address = "dmarc".into()), readable(), &on),
        "The email address dmarc of Reports is not a valid email address."
    );
    let bad_reply = SendingSettings {
        reply_to: "nobody".into(),
        ..SendingSettings::default()
    };
    assert_eq!(
        incomplete(&sender(), readable(), &bad_reply),
        "The Reply-To address nobody is not a valid email address."
    );
}

/// Every file of the database at `dir` (the main file, the WAL, the shared
/// memory), as bytes.
fn database_bytes(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    ["marcan.db", "marcan.db-wal", "marcan.db-shm"]
        .into_iter()
        .filter_map(|name| Some((name.to_owned(), std::fs::read(dir.join(name)).ok()?)))
        .collect()
}

fn holds(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// The 0.3.2 fixture (one mailbox, its password and the sending password in
/// `app_settings`) copied into a temp dir as `marcan.db`, and the two
/// passwords.
fn v0_3_2() -> (tempfile::TempDir, std::path::PathBuf, [&'static str; 2]) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dmarc-store/tests/fixtures/v0.3.2.db");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marcan.db");
    std::fs::copy(&fixture, &path).unwrap();
    let old = ["fixture-imap", "fixture-smtp"];
    let before = std::fs::read(&path).unwrap();
    assert!(
        old.iter().all(|p| holds(&before, p)),
        "the fixture holds both"
    );
    (dir, path, old)
}

/// Started with `key(n)` on the database at `path`: every secret is
/// sealed, and none of `old` is anywhere in the files (not in a freed page,
/// not in the WAL) while it runs or after it stopped; the sealed ones open.
async fn sealed_and_gone(path: &std::path::Path, old: [&str; 2], n: u8) {
    let dir = path.parent().unwrap();
    let pool = init(path).await.unwrap();
    Accounts::new(pool.clone(), Some(key(n)))
        .start()
        .await
        .unwrap();
    let secrets = dmarc_store::list_account_secrets(&pool).await.unwrap();
    assert_eq!(secrets.len(), 2);
    assert!(secrets.iter().all(|(_, row)| row.is_encrypted()));
    for (name, bytes) in database_bytes(dir) {
        for password in old {
            assert!(!holds(&bytes, password), "{password} in {name}");
        }
    }
    pool.close().await;
    for (name, bytes) in database_bytes(dir) {
        for password in old {
            assert!(
                !holds(&bytes, password),
                "{password} in {name} after closing"
            );
        }
    }
    let pool = init(path).await.unwrap();
    let (id, _) = secrets[0];
    assert_eq!(
        Accounts::new(pool, Some(key(n)))
            .secret(id, SecretKind::Password)
            .await
            .unwrap(),
        SecretState::Readable("fixture-imap".into())
    );
}

/// Upgraded from 0.3.2 and started with a key at once.
#[tokio::test]
async fn the_first_start_with_a_key_leaves_no_plain_password_in_the_files() {
    let (_dir, path, old) = v0_3_2();
    sealed_and_gone(&path, old, 1).await;
}

/// Upgraded from 0.3.2 without a key (the passwords stay plain, the
/// password typed again once), and the key added at a later start.
#[tokio::test]
async fn a_key_added_later_leaves_no_plain_password_in_the_files() {
    let (_dir, path, old) = v0_3_2();
    let pool = init(&path).await.unwrap();
    Accounts::new(pool.clone(), None).start().await.unwrap();
    let id = dmarc_store::list_account_secrets(&pool).await.unwrap()[0].0;
    let account = get_account(&pool, id).await.unwrap().unwrap();
    update_account(
        &pool,
        id,
        &account.fields,
        &[(
            SecretKind::Password,
            SecretWrite::Set(SecretRow::plain(SecretKind::Password, "fixture-imap")),
        )],
    )
    .await
    .unwrap();
    pool.close().await;
    sealed_and_gone(&path, old, 2).await;
}

/// Moved from the previous key to a new one: the values the previous key
/// sealed are gone from the files.
#[tokio::test]
async fn a_rotated_key_leaves_no_value_of_the_previous_one_in_the_files() {
    let (dir, pool) = pool().await;
    let id = plain_account(&pool, "secret").await;
    Accounts::new(pool.clone(), Some(key(1)))
        .start()
        .await
        .unwrap();
    let old = stored(&pool, id).await;
    Accounts::new(pool.clone(), Some(key(2)))
        .with_previous_key(Some(key(1)))
        .start()
        .await
        .unwrap();
    assert_eq!(stored(&pool, id).await.key_id.as_deref(), Some(key(2).id()));
    for name in ["t.db", "t.db-wal"] {
        let Ok(bytes) = std::fs::read(dir.path().join(name)) else {
            continue;
        };
        assert!(
            !bytes.windows(old.value.len()).any(|w| w == old.value),
            "the previous key's value in {name}"
        );
    }
}

/// With the previous key, its values are sealed again with the new one at
/// start and read at once; a value neither key opens stays locked; the
/// previous key alone is refused before any of this (`SecretKeys`).
#[tokio::test]
async fn the_previous_key_s_values_move_to_the_new_one_at_start() {
    let (_dir, pool) = pool().await;
    let a = plain_account(&pool, "first").await;
    let other = AccountFields {
        address: String::new(),
        ..sender()
    };
    let b = insert_account(
        &pool,
        None,
        &other,
        &[SecretRow::plain(SecretKind::Password, "second")],
        0,
    )
    .await
    .unwrap();
    Accounts::new(pool.clone(), Some(key(1)))
        .start()
        .await
        .unwrap();
    // `b` sealed with a third key, which is not given.
    let row = key(3).seal(b, SecretKind::Password, "second");
    update_account(
        &pool,
        b,
        &other,
        &[(SecretKind::Password, SecretWrite::Set(row.clone()))],
    )
    .await
    .unwrap();

    let accounts = Accounts::new(pool.clone(), Some(key(2))).with_previous_key(Some(key(1)));
    let log = start_log(&accounts).await;
    assert!(
        log.contains("1 encrypted again from MARCAN_SECRET_KEY_PREVIOUS to MARCAN_SECRET_KEY"),
        "{log}"
    );
    assert_eq!(stored(&pool, a).await.key_id.as_deref(), Some(key(2).id()));
    assert_eq!(
        accounts.secret(a, SecretKind::Password).await.unwrap(),
        SecretState::Readable("first".into())
    );
    assert_eq!(stored(&pool, b).await, row);
    assert_eq!(
        accounts.snapshot().await.unwrap().lock(b),
        Some(LockReason::KeyChanged)
    );

    // Started again with both: nothing left to move.
    let log = start_log(&accounts).await;
    assert!(
        log.contains("no stored secret is encrypted with MARCAN_SECRET_KEY_PREVIOUS"),
        "{log}"
    );
}

/// A value its own key does not open is damaged, not "another key"; it
/// locks its account like the others.
#[tokio::test]
async fn a_damaged_value_locks_its_account_with_its_own_reason() {
    let (_dir, pool) = pool().await;
    let id = plain_account(&pool, "secret").await;
    let accounts = Accounts::new(pool.clone(), Some(key(1)));
    accounts.start().await.unwrap();
    let mut row = stored(&pool, id).await;
    row.value[0] ^= 1;
    update_account(
        &pool,
        id,
        &sender(),
        &[(SecretKind::Password, SecretWrite::Set(row))],
    )
    .await
    .unwrap();
    assert_eq!(
        accounts.secret(id, SecretKind::Password).await.unwrap(),
        SecretState::Locked(LockReason::Damaged)
    );
    assert_eq!(accounts.mailbox_config(id, POLL).await.unwrap(), None);
    assert!(matches!(
        accounts.send_config(&sending_through(id)).await,
        Err(SendError::Locked {
            reason: LockReason::Damaged,
            ..
        })
    ));
}
