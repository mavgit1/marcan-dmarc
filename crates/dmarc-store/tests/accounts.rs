//! The accounts and their secrets as stored: written and read back, listed in
//! id order, the receive state forgotten and the epoch moved on when a save
//! replaces the mailbox, plain secrets sealed in one go, and everything of
//! an account gone with it.

use dmarc_store::{
    account_with_address, count_receive_quarantine, create_account, delete_account, get_account,
    get_account_secret, get_cursor, get_receive_attempts, init, insert_account,
    list_account_secrets, list_accounts, quarantine_receive_message, receive_pass_status,
    record_receive_attempt, record_receive_pass, reseal_secrets, set_account_address, set_cursor,
    update_account, update_account_receiving, AccountFields, AccountProvider, ImapSecurity,
    LoginKind, ReceiveAttemptKey, ReceiveChange, ReceivePassRow, SecretKind, SecretRow,
    SecretWrite, SmtpSecurity,
};
use sqlx::SqlitePool;

async fn pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = init(&dir.path().join("t.db")).await.expect("init");
    (dir, pool)
}

fn mailbox() -> AccountFields {
    AccountFields {
        name: "Reports".into(),
        address: "dmarc@example.com".into(),
        username: "reports@example.com".into(),
        imap_host: "imap.example.com".into(),
        imap_port: 143,
        imap_security: ImapSecurity::StartTls,
        smtp_host: "smtp.example.com".into(),
        smtp_port: Some(587),
        smtp_security: SmtpSecurity::StartTls,
        folder: "DMARC".into(),
        forwarded_from: vec!["old@example.net".into(), "dmarc@client.example".into()],
        ..AccountFields::default()
    }
}

/// A cursor, a quarantined message and a pass for `id`.
async fn receive_state(pool: &SqlitePool, id: i64, folder: &str) {
    set_cursor(pool, id, folder, "7", "42", 0).await.unwrap();
    quarantine_receive_message(pool, id, folder, "7", "3", "bad", 0, 0)
        .await
        .unwrap();
    record_receive_pass(
        pool,
        &ReceivePassRow {
            account_id: id,
            finished_at: 10,
            checked_at: Some(9),
            error: None,
            error_detail: None,
            yielded: false,
        },
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn accounts_read_back_what_was_written_in_id_order() {
    let (_dir, pool) = pool().await;
    assert!(list_accounts(&pool).await.unwrap().is_empty());

    let first = insert_account(&pool, None, &mailbox(), &[], 100)
        .await
        .unwrap();
    let sender = AccountFields {
        name: "Sending".into(),
        provider: AccountProvider::Microsoft365,
        login_kind: LoginKind::Microsoft,
        receive_enabled: false,
        ..AccountFields::default()
    };
    let second = insert_account(&pool, Some(7), &sender, &[], 200)
        .await
        .unwrap();
    assert_eq!(second, 7);

    let all = list_accounts(&pool).await.unwrap();
    assert_eq!(all.iter().map(|a| a.id).collect::<Vec<_>>(), [first, 7]);
    assert_eq!(all[0].fields, mailbox());
    assert_eq!(all[0].created_at, 100);
    assert_eq!(all[1].fields, sender);
    assert_eq!(all[1].fields.smtp_port, None);
    assert_eq!(get_account(&pool, 7).await.unwrap().unwrap().fields, sender);
    assert_eq!(get_account(&pool, 99).await.unwrap(), None);

    // Addresses are unique, case folded; an empty one is none.
    assert_eq!(
        account_with_address(&pool, "DMARC@example.com")
            .await
            .unwrap(),
        Some(first)
    );
    assert_eq!(account_with_address(&pool, "").await.unwrap(), None);
    // A forwarded address belongs to its account too, in either column's
    // place, case folded; its order is the order written.
    assert_eq!(
        account_with_address(&pool, "DMARC@client.EXAMPLE")
            .await
            .unwrap(),
        Some(first)
    );
    let taken = AccountFields {
        forwarded_from: vec!["Old@Example.net".into()],
        ..sender.clone()
    };
    assert!(insert_account(&pool, None, &taken, &[], 0).await.is_err());
    let mut reordered = mailbox();
    reordered.forwarded_from = vec!["dmarc@client.example".into(), "new@example.net".into()];
    assert!(update_account(&pool, first, &reordered, &[]).await.unwrap());
    assert_eq!(
        get_account(&pool, first).await.unwrap().unwrap().fields,
        reordered
    );
    assert_eq!(
        account_with_address(&pool, "old@example.net")
            .await
            .unwrap(),
        None
    );
    update_account(&pool, first, &mailbox(), &[]).await.unwrap();
    let clash = AccountFields {
        address: "Dmarc@Example.com".into(),
        ..sender.clone()
    };
    assert!(insert_account(&pool, None, &clash, &[], 0).await.is_err());
    set_account_address(&pool, first, "").await.unwrap();
    insert_account(&pool, None, &clash, &[], 0).await.unwrap();
    assert!(!update_account(&pool, 99, &sender, &[]).await.unwrap());
}

#[tokio::test]
async fn secrets_are_set_kept_and_cleared_per_kind() {
    let (_dir, pool) = pool().await;
    let password = SecretRow::plain(SecretKind::Password, " spaced ");
    let id = insert_account(&pool, None, &mailbox(), std::slice::from_ref(&password), 0)
        .await
        .unwrap();
    assert_eq!(
        get_account_secret(&pool, id, SecretKind::Password)
            .await
            .unwrap(),
        Some(password.clone())
    );
    assert!(!format!("{password:?}").contains("spaced"));

    let sealed = SecretRow {
        kind: SecretKind::PrivateKey,
        value: vec![1, 2, 3],
        nonce: Some(vec![0; 12]),
        key_id: Some(vec![9; 8]),
    };
    update_account(
        &pool,
        id,
        &mailbox(),
        &[
            (SecretKind::Password, SecretWrite::Keep),
            (SecretKind::PrivateKey, SecretWrite::Set(sealed.clone())),
        ],
    )
    .await
    .unwrap();
    // Plain and sealed side by side (by kind name).
    assert_eq!(
        list_account_secrets(&pool).await.unwrap(),
        [(id, password.clone()), (id, sealed.clone())]
    );

    update_account(
        &pool,
        id,
        &mailbox(),
        &[(SecretKind::Password, SecretWrite::Clear)],
    )
    .await
    .unwrap();
    assert_eq!(list_account_secrets(&pool).await.unwrap(), [(id, sealed)]);

    // A sealed value needs its nonce and key: the table refuses half of one.
    let half = SecretRow {
        kind: SecretKind::PrivateKey,
        value: vec![1],
        nonce: Some(vec![0; 11]),
        key_id: Some(vec![1]),
    };
    assert!(update_account(
        &pool,
        id,
        &mailbox(),
        &[(SecretKind::PrivateKey, SecretWrite::Set(half))]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn resealing_offers_every_secret_and_replaces_the_ones_returned() {
    let (_dir, pool) = pool().await;
    let a = insert_account(
        &pool,
        None,
        &mailbox(),
        &[SecretRow::plain(SecretKind::Password, "one")],
        0,
    )
    .await
    .unwrap();
    let sealed = SecretRow {
        kind: SecretKind::Password,
        value: b"sealed".to_vec(),
        nonce: Some(vec![1; 12]),
        key_id: Some(b"old".to_vec()),
    };
    let b = insert_account(
        &pool,
        None,
        &AccountFields::default(),
        std::slice::from_ref(&sealed),
        0,
    )
    .await
    .unwrap();

    let mut seen = Vec::new();
    let count = reseal_secrets(&pool, |id, stored| {
        seen.push((id, stored.value.clone()));
        // Only the plain one is replaced.
        (!stored.is_encrypted()).then(|| SecretRow {
            kind: stored.kind,
            value: stored.value.iter().rev().copied().collect(),
            nonce: Some(vec![2; 12]),
            key_id: Some(b"new".to_vec()),
        })
    })
    .await
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(seen, [(a, b"one".to_vec()), (b, b"sealed".to_vec())]);
    let stored = get_account_secret(&pool, a, SecretKind::Password)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.value, b"eno");
    assert_eq!(stored.key_id.as_deref(), Some(&b"new"[..]));
    // The one left alone is as it was.
    assert_eq!(
        get_account_secret(&pool, b, SecretKind::Password)
            .await
            .unwrap(),
        Some(sealed)
    );
    // `None` keeps every value.
    assert_eq!(reseal_secrets(&pool, |_, _| None).await.unwrap(), 0);
}

/// An update writes the columns and leaves the receive state and the epoch
/// alone, even with another server (the server decides whether the
/// account reads another mailbox); a replacing save forgets one account's
/// cursor, quarantine, retry counts and passes and moves only its epoch
/// on, and nothing of another account.
#[tokio::test]
async fn the_receive_state_is_forgotten_only_when_replaced_and_per_account() {
    let (_dir, pool) = pool().await;
    let id = insert_account(&pool, None, &mailbox(), &[], 0)
        .await
        .unwrap();
    let other = insert_account(&pool, None, &AccountFields::default(), &[], 0)
        .await
        .unwrap();
    receive_state(&pool, id, "DMARC").await;
    receive_state(&pool, other, "INBOX").await;
    let epoch = |pool: SqlitePool, id: i64| async move {
        get_account(&pool, id).await.unwrap().unwrap().receive_epoch
    };

    let moved = AccountFields {
        imap_host: "imap.example.net".into(),
        ..mailbox()
    };
    assert!(update_account(&pool, id, &moved, &[]).await.unwrap());
    assert_eq!(
        get_cursor(&pool, id, "DMARC").await.unwrap(),
        Some(("7".into(), "42".into()))
    );
    assert_eq!(epoch(pool.clone(), id).await, 0);

    let replaced = ReceiveChange::Replaced { at: 1 };
    update_account_receiving(&pool, id, &moved, &[], replaced)
        .await
        .unwrap();
    assert_eq!(get_cursor(&pool, id, "DMARC").await.unwrap(), None);
    assert_eq!(
        count_receive_quarantine(&pool, Some(id), None)
            .await
            .unwrap(),
        0
    );
    // Its passes go too: they read the mailbox it no longer reads.
    assert_eq!(
        receive_pass_status(&pool, id).await.unwrap(),
        dmarc_store::ReceivePassStatus::default()
    );
    assert_eq!(epoch(pool.clone(), id).await, 1);
    // Another account keeps its state and its epoch.
    assert_eq!(
        get_cursor(&pool, other, "INBOX").await.unwrap(),
        Some(("7".into(), "42".into()))
    );
    assert_eq!(
        receive_pass_status(&pool, other)
            .await
            .unwrap()
            .last_check_at,
        Some(9)
    );
    assert_eq!(epoch(pool.clone(), other).await, 0);
}

#[tokio::test]
async fn deleting_an_account_deletes_its_secrets_and_receive_state() {
    let (_dir, pool) = pool().await;
    let id = insert_account(
        &pool,
        None,
        &mailbox(),
        &[SecretRow::plain(SecretKind::Password, "p")],
        0,
    )
    .await
    .unwrap();
    receive_state(&pool, id, "DMARC").await;
    assert!(delete_account(&pool, id).await.unwrap());
    assert!(!delete_account(&pool, id).await.unwrap());
    assert!(list_account_secrets(&pool).await.unwrap().is_empty());
    assert_eq!(
        count_receive_quarantine(&pool, None, None).await.unwrap(),
        0
    );
    assert_eq!(
        receive_pass_status(&pool, id).await.unwrap().last_check_at,
        None
    );
    let cursors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM receive_cursor")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cursors, 0);
    // Its forwarded addresses go with it.
    assert_eq!(
        account_with_address(&pool, "old@example.net")
            .await
            .unwrap(),
        None
    );
}

/// A save that makes the account read another mailbox forgets its receive
/// state and moves its epoch on in the save's own transaction; one that
/// keeps the mailbox keeps both.
#[tokio::test]
async fn a_replacing_save_forgets_the_receive_state_in_its_transaction() {
    let (_dir, pool) = pool().await;
    let id = insert_account(&pool, None, &mailbox(), &[], 0)
        .await
        .unwrap();
    receive_state(&pool, id, "DMARC").await;
    let renamed = AccountFields {
        name: "Renamed".into(),
        ..mailbox()
    };
    let keep = ReceiveChange::Keep;
    assert!(update_account_receiving(&pool, id, &renamed, &[], keep)
        .await
        .unwrap());
    assert_eq!(
        get_cursor(&pool, id, "DMARC").await.unwrap(),
        Some(("7".into(), "42".into()))
    );
    let row = get_account(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.receive_epoch, 0);

    let moved = AccountFields {
        folder: "Other".into(),
        ..mailbox()
    };
    let replaced = ReceiveChange::Replaced { at: 7 };
    assert!(update_account_receiving(&pool, id, &moved, &[], replaced)
        .await
        .unwrap());
    assert_eq!(get_cursor(&pool, id, "DMARC").await.unwrap(), None);
    assert_eq!(
        count_receive_quarantine(&pool, Some(id), None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        receive_pass_status(&pool, id).await.unwrap(),
        dmarc_store::ReceivePassStatus::default()
    );
    let row = get_account(&pool, id).await.unwrap().unwrap();
    assert_eq!(
        (row.fields, row.receiving_since, row.receive_epoch),
        (moved, 7, 1)
    );
}

/// An account reads its mailbox from when it was added; switching its
/// receiving on, or another mailbox, starts that over; any other save
/// keeps it.
#[tokio::test]
async fn receiving_since_starts_with_the_account_and_its_mailbox() {
    let (_dir, pool) = pool().await;
    let id = insert_account(&pool, None, &mailbox(), &[], 5)
        .await
        .unwrap();
    let since = |pool: SqlitePool| async move {
        get_account(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .receiving_since
    };
    assert_eq!(since(pool.clone()).await, 5);
    update_account(&pool, id, &mailbox(), &[]).await.unwrap();
    assert_eq!(since(pool.clone()).await, 5);
    let started = ReceiveChange::Started { at: 9 };
    update_account_receiving(&pool, id, &mailbox(), &[], started)
        .await
        .unwrap();
    assert_eq!(since(pool.clone()).await, 9);
    // Switching receiving on keeps the epoch: the same mailbox.
    let row = get_account(&pool, id).await.unwrap().unwrap();
    assert_eq!(row.receive_epoch, 0);
    let created = create_account(&pool, &AccountFields::default(), 11, |_| vec![])
        .await
        .unwrap();
    let row = get_account(&pool, created).await.unwrap().unwrap();
    assert_eq!((row.created_at, row.receiving_since), (11, 11));
}

/// A pass's writes land only while the account still has the epoch the
/// pass was prepared under, and never fail on an account that is gone.
#[tokio::test]
async fn writes_of_a_pass_land_only_under_the_accounts_epoch() {
    let (_dir, pool) = pool().await;
    let id = insert_account(&pool, None, &mailbox(), &[], 0)
        .await
        .unwrap();
    let key = ReceiveAttemptKey {
        account_id: id,
        folder: "DMARC",
        generation: "7",
        kind: "message",
        message_id: "3",
    };
    let pass = ReceivePassRow {
        account_id: id,
        finished_at: 10,
        checked_at: Some(9),
        error: None,
        error_detail: None,
        yielded: false,
    };
    // An epoch the account does not have (yet): nothing lands.
    assert!(!set_cursor(&pool, id, "DMARC", "7", "42", 1).await.unwrap());
    assert!(
        !quarantine_receive_message(&pool, id, "DMARC", "7", "3", "bad", 0, 1)
            .await
            .unwrap()
    );
    assert_eq!(record_receive_attempt(&pool, &key, 0, 1).await.unwrap(), 0);
    assert!(!record_receive_pass(&pool, &pass, Some(1)).await.unwrap());
    assert_eq!(get_cursor(&pool, id, "DMARC").await.unwrap(), None);
    assert_eq!(get_receive_attempts(&pool, &key).await.unwrap(), 0);
    assert_eq!(
        receive_pass_status(&pool, id).await.unwrap().last_check_at,
        None
    );

    // Its own: everything lands.
    assert!(set_cursor(&pool, id, "DMARC", "7", "42", 0).await.unwrap());
    assert!(
        quarantine_receive_message(&pool, id, "DMARC", "7", "3", "bad", 0, 0)
            .await
            .unwrap()
    );
    assert_eq!(record_receive_attempt(&pool, &key, 0, 0).await.unwrap(), 1);
    assert!(record_receive_pass(&pool, &pass, Some(0)).await.unwrap());
    assert_eq!(
        get_cursor(&pool, id, "DMARC").await.unwrap(),
        Some(("7".into(), "42".into()))
    );

    // A save that keeps the mailbox keeps the epoch.
    let microsoft = AccountFields {
        login_kind: LoginKind::Microsoft,
        username: String::new(),
        ..mailbox()
    };
    assert!(update_account(&pool, id, &microsoft, &[]).await.unwrap());
    assert!(set_cursor(&pool, id, "DMARC", "7", "43", 0).await.unwrap());

    // A replacing save: the pass prepared before writes nothing more, the
    // next one (prepared under epoch 1) does.
    let replaced = ReceiveChange::Replaced { at: 1 };
    update_account_receiving(&pool, id, &microsoft, &[], replaced)
        .await
        .unwrap();
    assert!(!set_cursor(&pool, id, "DMARC", "7", "44", 0).await.unwrap());
    assert_eq!(record_receive_attempt(&pool, &key, 0, 0).await.unwrap(), 0);
    assert!(!record_receive_pass(&pool, &pass, Some(0)).await.unwrap());
    assert_eq!(get_cursor(&pool, id, "DMARC").await.unwrap(), None);
    assert!(set_cursor(&pool, id, "DMARC", "1", "2", 1).await.unwrap());
    assert_eq!(
        get_cursor(&pool, id, "DMARC").await.unwrap(),
        Some(("1".into(), "2".into()))
    );
    // A pass that never had a configuration needs only the account.
    assert!(record_receive_pass(&pool, &pass, None).await.unwrap());

    // Gone: nothing written, and no foreign-key error.
    assert!(delete_account(&pool, id).await.unwrap());
    assert!(!set_cursor(&pool, id, "DMARC", "1", "3", 1).await.unwrap());
    assert!(
        !quarantine_receive_message(&pool, id, "DMARC", "1", "4", "bad", 0, 1)
            .await
            .unwrap()
    );
    assert_eq!(record_receive_attempt(&pool, &key, 0, 1).await.unwrap(), 0);
    assert!(!record_receive_pass(&pool, &pass, Some(1)).await.unwrap());
    assert!(!record_receive_pass(&pool, &pass, None).await.unwrap());
}

/// Creating an account writes it and the secrets made for its new id in one
/// transaction.
#[tokio::test]
async fn an_account_is_created_with_the_secrets_made_for_its_id() {
    let (_dir, pool) = pool().await;
    insert_account(&pool, None, &AccountFields::default(), &[], 0)
        .await
        .unwrap();
    let mut seen = None;
    let id = create_account(&pool, &mailbox(), 5, |id| {
        seen = Some(id);
        vec![
            (
                SecretKind::Password,
                SecretWrite::Set(SecretRow::plain(SecretKind::Password, &format!("p{id}"))),
            ),
            (SecretKind::PrivateKey, SecretWrite::Clear),
        ]
    })
    .await
    .unwrap();
    assert_eq!(seen, Some(id));
    assert_eq!(id, 2);
    let row = get_account(&pool, id).await.unwrap().unwrap();
    assert_eq!((row.created_at, row.fields), (5, mailbox()));
    let password = get_account_secret(&pool, id, SecretKind::Password)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(password.value, b"p2");
    assert_eq!(list_account_secrets(&pool).await.unwrap().len(), 1);
}
