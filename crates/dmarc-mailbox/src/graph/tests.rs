//! Graph passes against the fake Graph server: paging and links, resuming,
//! expired links, a recreated folder, deleted, failing, unparsable and
//! oversized messages, the token, throttling, foreign links, the refusals
//! and their words, the folder lookup and the connection test; and sending
//! (`sendMail` in MIME form, its token, throttling and refusals).

use super::fake_server::{Expiry, FakeGraph, UNKNOWN_USER};
use super::source::{GraphId, ROUND_PAGES};
use super::*;
use crate::backend::memory::{MemoryBackend, Quarantined};
use crate::backend::{Cursor, MailboxConfig};
use crate::batch::Limits;
use crate::imap::fake_server::FakeMessage;
use crate::pass::{
    default_parser, FailureKind, FetchSummary, Parser, Pass, PassResult, MAX_ATTEMPTS,
};
use crate::progress::SharedProgress;
use crate::source::MessageId;
use crate::ConnectStage;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

const MAILBOX: &str = "dmarc@example.com";

/// Tokens handed out in order: each refusal moves to the next.
struct Tokens {
    tokens: Vec<String>,
    at: AtomicUsize,
    calls: AtomicUsize,
    refused: AtomicUsize,
}

impl Tokens {
    fn new(tokens: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            tokens: tokens.iter().map(|t| (*t).to_owned()).collect(),
            at: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            refused: AtomicUsize::new(0),
        })
    }
}

impl GraphToken for Tokens {
    fn token(&self) -> BoxFuture<'_, Result<String, TokenFailure>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let at = self.at.load(Ordering::SeqCst).min(self.tokens.len() - 1);
            Ok(self.tokens[at].clone())
        })
    }

    fn refused(&self) {
        self.refused.fetch_add(1, Ordering::SeqCst);
        self.at.fetch_add(1, Ordering::SeqCst);
    }
}

/// The problem itself, so the tests see which one it was.
fn words(problem: &GraphProblem<'_>) -> String {
    format!("{problem:?}")
}

/// A docs section for the missing role only, so the tests see it passed on.
fn docs(problem: &GraphProblem<'_>) -> Option<&'static str> {
    matches!(problem, GraphProblem::NoAccess { .. }).then_some("role")
}

fn config(fake: &FakeGraph, address: &str, folder: &str, tokens: Arc<Tokens>) -> GraphConfig {
    GraphConfig {
        id: "1".into(),
        epoch: 0,
        address: address.into(),
        folder: folder.into(),
        api: GraphApi::new(fake.base.clone()),
        token: tokens,
        say: words,
        docs,
    }
}

fn limits() -> Limits {
    Limits {
        batch_max_messages: 2,
        batch_max_bytes: 1_000_000,
        max_message_bytes: 1_000_000,
        max_pending_xml_bytes: Limits::DEFAULT.max_pending_xml_bytes,
        size_chunk: 3,
        command_timeout: Duration::from_secs(5),
        batch_timeout_base: Duration::from_secs(5),
        batch_bytes_per_extra_sec: u64::MAX,
    }
}

struct Run {
    result: PassResult<()>,
    summary: FetchSummary,
}

async fn run_with(
    cfg: &GraphConfig,
    backend: &MemoryBackend,
    limits: &Limits,
    cancel: &CancellationToken,
    parser: Parser,
) -> Run {
    let mailbox = MailboxConfig::Graph(cfg.clone());
    let progress = SharedProgress::default();
    let mut pass = Pass::new(backend, &mailbox, &progress, cancel, limits, &parser);
    let mut source = GraphSource::new(cfg, *limits);
    let result = pass.import(&mut source).await;
    Run {
        result,
        summary: pass.summary,
    }
}

async fn run(cfg: &GraphConfig, backend: &MemoryBackend) -> Run {
    run_with(
        cfg,
        backend,
        &limits(),
        &CancellationToken::new(),
        default_parser(),
    )
    .await
}

/// A fake with `n` reports in its inbox, and the configuration reading it.
async fn inbox_with(n: u32) -> (FakeGraph, GraphConfig, Vec<String>) {
    let fake = FakeGraph::start(MAILBOX).await;
    let ids = fake.deliver_reports(&fake.inbox(), &(1..=n).collect::<Vec<_>>());
    let cfg = config(&fake, MAILBOX, "INBOX", Tokens::new(&["t"]));
    (fake, cfg, ids)
}

fn cursor(backend: &MemoryBackend) -> Option<Cursor> {
    backend.cursor_of("INBOX")
}

fn quarantine(backend: &MemoryBackend) -> Vec<Quarantined> {
    backend.state().quarantine.clone()
}

#[tokio::test]
async fn a_first_import_pages_through_the_delta_query() {
    let (fake, cfg, ids) = inbox_with(5).await;
    let backend = MemoryBackend::default();
    let run = run(&cfg, &backend).await;
    run.result.unwrap();
    assert_eq!(run.summary.reports_ingested, 5);
    assert_eq!(run.summary.messages_seen, 5);
    assert!(run.summary.progressed);
    assert_eq!(run.summary.start_position, None);
    assert_eq!(backend.report_count(), 5);

    let deltas = fake.delta_requests();
    assert_eq!(deltas.len(), 3, "{deltas:?}");
    assert!(
        deltas[0].ends_with("/messages/delta?changeType=created&$select=id"),
        "{deltas:?}"
    );
    assert!(deltas[1].contains("$skiptoken="), "{deltas:?}");
    assert!(fake
        .requests()
        .iter()
        .filter(|r| r.target.contains("/messages/delta"))
        .all(|r| r.prefer == "odata.maxpagesize=2" && r.auth == "Bearer t"));
    assert_eq!(fake.downloads(), ids);

    let stored = cursor(&backend).unwrap();
    assert_eq!(stored.generation, fake.inbox());
    assert!(stored.position.contains("$deltatoken="), "{stored:?}");
    assert!(
        stored.position.starts_with(fake.base.as_str()),
        "{stored:?}"
    );
}

#[tokio::test]
async fn an_empty_pass_saves_the_new_delta_link_and_then_reads_new_mail_only() {
    let (fake, cfg, _) = inbox_with(3).await;
    let backend = MemoryBackend::default();
    run(&cfg, &backend).await.result.unwrap();
    let first = cursor(&backend).unwrap();
    fake.clear_log();

    // Nothing new: the stored delta link is asked, its successor stored.
    let empty = run(&cfg, &backend).await;
    empty.result.unwrap();
    assert_eq!(empty.summary.messages_seen, 0);
    assert!(fake.downloads().is_empty());
    let deltas = fake.delta_requests();
    assert_eq!(deltas.len(), 1);
    assert!(first.position.ends_with(&deltas[0]), "{deltas:?} {first:?}");
    let second = cursor(&backend).unwrap();
    assert_ne!(second, first);
    assert!(second.position.contains("$deltatoken="));

    // New mail: only it is read.
    fake.clear_log();
    let new = fake.deliver_reports(&fake.inbox(), &[4, 5]);
    let more = run(&cfg, &backend).await;
    more.result.unwrap();
    assert_eq!(more.summary.reports_ingested, 2);
    assert_eq!(fake.downloads(), new);
    assert!(second.position.ends_with(&fake.delta_requests()[0]));
    assert_eq!(backend.report_count(), 5);
}

/// The settings are saved while the first batch is parsed: the pass stops
/// after it, and the next pass goes on from the next page, reading no page
/// or message of the first again.
#[tokio::test]
async fn an_interrupted_import_resumes_after_its_last_finished_page() {
    let (fake, cfg, ids) = inbox_with(6).await;
    let backend = MemoryBackend::default();
    let cancel = CancellationToken::new();
    let cancelling: Parser = {
        let cancel = cancel.clone();
        let parse = default_parser();
        Arc::new(move |raw: &[u8]| {
            cancel.cancel();
            parse(raw)
        })
    };
    let first = run_with(&cfg, &backend, &limits(), &cancel, cancelling).await;
    first.result.unwrap();
    assert!(first.summary.interrupted);
    assert_eq!(first.summary.reports_ingested, 2);
    assert!(first.summary.progressed);
    let stored = cursor(&backend).unwrap();
    assert!(stored.position.contains("$skiptoken="), "{stored:?}");

    fake.clear_log();
    let second = run(&cfg, &backend).await;
    second.result.unwrap();
    assert_eq!(second.summary.reports_ingested, 4);
    assert_eq!(second.summary.reports_duplicate, 0);
    assert_eq!(fake.downloads(), ids[2..].to_vec());
    let deltas = fake.delta_requests();
    assert!(stored.position.ends_with(&deltas[0]), "{deltas:?}");
    assert!(deltas.iter().all(|d| d.contains("token=")), "{deltas:?}");
}

/// Graph's pages are larger than a batch: a pass stopped in the middle of
/// a page asks for that page again and skips what it handled.
#[tokio::test]
async fn a_page_left_in_its_middle_is_read_from_the_next_message() {
    let (fake, cfg, ids) = inbox_with(5).await;
    fake.page_size(3);
    let backend = MemoryBackend::default();
    let cancel = CancellationToken::new();
    let cancelling: Parser = {
        let cancel = cancel.clone();
        let parse = default_parser();
        Arc::new(move |raw: &[u8]| {
            cancel.cancel();
            parse(raw)
        })
    };
    run_with(&cfg, &backend, &limits(), &cancel, cancelling)
        .await
        .result
        .unwrap();
    let stored = cursor(&backend).unwrap();
    let (link, after) = stored
        .position
        .split_once('\n')
        .expect("a page and a message");
    assert_eq!(after, ids[1]);
    assert!(
        link.ends_with("/messages/delta?changeType=created&$select=id"),
        "{link}"
    );

    fake.clear_log();
    let second = run(&cfg, &backend).await;
    second.result.unwrap();
    assert_eq!(fake.downloads(), ids[2..].to_vec());
    assert_eq!(second.summary.reports_duplicate, 0);
    assert_eq!(backend.report_count(), 5);
}

#[tokio::test]
async fn an_expired_link_lists_the_folder_from_its_first_message_again() {
    for expiry in [Expiry::Gone, Expiry::LostState] {
        let (fake, cfg, _) = inbox_with(2).await;
        let backend = MemoryBackend::default();
        run(&cfg, &backend).await.result.unwrap();
        fake.expire_links(expiry);
        let new = fake.deliver_reports(&fake.inbox(), &[3]);
        fake.clear_log();

        let again = run(&cfg, &backend).await;
        again.result.unwrap();
        assert_eq!(again.summary.reports_ingested, 1, "{expiry:?}");
        assert_eq!(again.summary.reports_duplicate, 2, "{expiry:?}");
        assert_eq!(fake.downloads().len(), 3);
        assert!(fake.downloads().ends_with(&new));
        let deltas = fake.delta_requests();
        assert!(
            deltas[1].ends_with("changeType=created&$select=id"),
            "{deltas:?}"
        );
        let stored = cursor(&backend).unwrap();
        assert!(stored.position.contains("$deltatoken=1."), "{stored:?}");
        assert!(quarantine(&backend).is_empty());
    }
}

#[tokio::test]
async fn a_folder_created_again_is_read_from_the_start_and_its_quarantine_dropped() {
    let fake = FakeGraph::start(MAILBOX).await;
    let inbox = fake.inbox();
    fake.deliver(&inbox, FakeMessage::broken_report(1).raw());
    fake.deliver_reports(&inbox, &[2]);
    let cfg = config(&fake, MAILBOX, "INBOX", Tokens::new(&["t"]));
    let backend = MemoryBackend::default();
    let first = run(&cfg, &backend).await;
    first.result.unwrap();
    assert_eq!(first.summary.messages_quarantined, 1);
    assert_eq!(quarantine(&backend).len(), 1);

    let recreated = fake.recreate(&inbox);
    let new = fake.deliver_reports(&recreated, &[3, 4]);
    fake.clear_log();
    let second = run(&cfg, &backend).await;
    second.result.unwrap();
    assert_eq!(second.summary.reports_ingested, 2);
    assert_eq!(fake.downloads(), new);
    assert_eq!(cursor(&backend).unwrap().generation, recreated);
    assert!(quarantine(&backend).is_empty());
    assert!(fake.delta_requests()[0].contains(&recreated));
}

#[tokio::test]
async fn a_message_deleted_while_it_is_read_is_skipped() {
    let (fake, cfg, ids) = inbox_with(3).await;
    fake.vanish_on_read(&ids[1]);
    let backend = MemoryBackend::default();
    let run = run(&cfg, &backend).await;
    run.result.unwrap();
    assert_eq!(run.summary.reports_ingested, 2);
    assert_eq!(run.summary.messages_seen, 3);
    assert_eq!(run.summary.messages_quarantined, 0);
    assert_eq!(run.summary.errors, 0);
    assert!(cursor(&backend).unwrap().position.contains("$deltatoken="));
}

#[tokio::test]
async fn an_unparsable_message_is_quarantined_once_and_not_read_again() {
    let fake = FakeGraph::start(MAILBOX).await;
    let inbox = fake.inbox();
    fake.deliver_reports(&inbox, &[1]);
    let broken = fake.deliver(&inbox, FakeMessage::broken_report(2).raw());
    fake.deliver_reports(&inbox, &[3]);
    let cfg = config(&fake, MAILBOX, "INBOX", Tokens::new(&["t"]));
    let backend = MemoryBackend::default();
    let first = run(&cfg, &backend).await;
    first.result.unwrap();
    assert_eq!(first.summary.reports_ingested, 2);
    assert_eq!(first.summary.messages_quarantined, 1);
    let q = quarantine(&backend);
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].message_id, broken);
    assert_eq!(q[0].generation, inbox);

    fake.clear_log();
    let second = run(&cfg, &backend).await;
    second.result.unwrap();
    assert!(fake.downloads().is_empty());
    assert_eq!(quarantine(&backend).len(), 1);
}

#[tokio::test]
async fn a_message_graph_keeps_failing_on_is_quarantined_after_max_attempts() {
    let (fake, cfg, ids) = inbox_with(3).await;
    fake.fail_message(&ids[1], true);
    let backend = MemoryBackend::default();
    for attempt in 1..MAX_ATTEMPTS {
        let failed = run(&cfg, &backend).await;
        let err = failed.result.unwrap_err();
        assert_eq!(err.kind, FailureKind::Server);
        assert!(
            err.message
                .contains(&format!("attempt {attempt} of {MAX_ATTEMPTS}")),
            "{err}"
        );
        assert!(err.message.contains(&ids[1]), "{err}");
        assert!(cursor(&backend).is_none());
    }
    let last = run(&cfg, &backend).await;
    last.result.unwrap();
    assert_eq!(last.summary.messages_quarantined, 1);
    assert_eq!(backend.report_count(), 2);
    assert!(backend.state().attempts.is_empty());

    fake.clear_log();
    run(&cfg, &backend).await.result.unwrap();
    assert!(fake.downloads().is_empty());
}

#[tokio::test]
async fn a_batch_graph_fails_on_entirely_fails_the_pass_without_attempts() {
    let (fake, cfg, ids) = inbox_with(2).await;
    for id in &ids {
        fake.fail_message(id, true);
    }
    let backend = MemoryBackend::default();
    let err = run(&cfg, &backend).await.result.unwrap_err();
    assert_eq!(err.kind, FailureKind::Server);
    assert!(err.message.contains("failed to serve all 2"), "{err}");
    assert!(backend.state().attempts.is_empty());
}

#[tokio::test]
async fn a_message_over_the_size_cap_is_quarantined_while_it_is_read() {
    let (fake, cfg, ids) = inbox_with(2).await;
    let big = fake.deliver(&fake.inbox(), &vec![b'x'; 5_000]);
    let backend = MemoryBackend::default();
    let small = Limits {
        max_message_bytes: 4_000,
        ..limits()
    };
    let run = run_with(
        &cfg,
        &backend,
        &small,
        &CancellationToken::new(),
        default_parser(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(run.summary.reports_ingested, 2);
    assert_eq!(run.summary.messages_quarantined, 1);
    let q = quarantine(&backend);
    assert_eq!(q[0].message_id, big);
    assert!(q[0].error.contains("import limit"), "{}", q[0].error);
    assert_eq!(fake.downloads().len(), ids.len() + 1);
}

#[tokio::test]
async fn a_refused_token_is_fetched_anew_once() {
    let (fake, _, _) = inbox_with(1).await;
    fake.accept_tokens(&["new"]);
    let tokens = Tokens::new(&["old", "new"]);
    let cfg = config(&fake, MAILBOX, "INBOX", Arc::clone(&tokens));
    let backend = MemoryBackend::default();
    let run = run(&cfg, &backend).await;
    run.result.unwrap();
    assert_eq!(run.summary.reports_ingested, 1);
    assert_eq!(tokens.refused.load(Ordering::SeqCst), 1);
    assert_eq!(tokens.calls.load(Ordering::SeqCst), 2);

    // Refused again with the new one: a login failure, said once.
    let tokens = Tokens::new(&["old", "stale"]);
    let cfg = config(&fake, MAILBOX, "INBOX", Arc::clone(&tokens));
    let err = super::tests::run(&cfg, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Connect(ConnectStage::Auth));
    assert!(err.message.starts_with("TokenRefused"), "{err}");
    assert!(
        err.detail.starts_with("InvalidAuthenticationToken"),
        "{err}"
    );
    assert_eq!(tokens.refused.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn throttling_is_waited_out_within_a_bound() {
    let (fake, cfg, _) = inbox_with(1).await;
    fake.throttle(1, 1);
    let backend = MemoryBackend::default();
    let started = std::time::Instant::now();
    run(&cfg, &backend).await.result.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(backend.report_count(), 1);

    // Throttled on and on: the pass fails, and its back-off applies.
    let (fake, cfg, _) = inbox_with(1).await;
    fake.throttle(100, 0);
    let err = run(&cfg, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Server);
    assert!(err.message.starts_with("Throttled"), "{err}");
    assert!(err.detail.starts_with("ApplicationThrottled"), "{err}");
    assert_eq!(fake.requests().len(), 5);
}

#[tokio::test]
async fn no_token_reaches_an_error() {
    let token = "secret-token-123";
    let fake = FakeGraph::start(MAILBOX).await;
    fake.forbid(true);
    fake.echo_auth(true);
    let cfg = config(&fake, MAILBOX, "INBOX", Tokens::new(&[token]));
    let err = run(&cfg, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    // The sentence is the host's; Graph's answer stays beside it, masked.
    assert!(err.message.starts_with("NoAccess"), "{err}");
    assert!(err.detail.contains("Access is denied for"), "{err}");
    assert!(!err.detail.contains(token), "{err}");
    assert!(!err.to_string().contains(token), "{err}");
    assert!(!format!("{cfg:?}").contains(token));

    let probe = probe_graph(&cfg).await.unwrap_err();
    assert!(!probe.to_string().contains(token), "{probe}");

    let jwt = "eyJ0eXAiOiJKV1QifQ.eyJhdWQiOiJ4In0.c2ln";
    assert_eq!(
        cfg.redact(&format!("refused Bearer {jwt} and {jwt}, not eyJust")),
        "refused Bearer *** and ***, not eyJust"
    );
}

#[tokio::test]
async fn links_to_another_host_are_neither_followed_nor_stored() {
    let (fake, cfg, _) = inbox_with(3).await;
    fake.foreign_links(true);
    let backend = MemoryBackend::default();
    let err = run(&cfg, &backend).await.result.unwrap_err();
    assert_eq!(err.kind, FailureKind::Server);
    assert!(err.message.contains("graph.evil.example"), "{err}");
    assert!(err.message.contains("only to 127.0.0.1"), "{err}");
    assert_eq!(fake.delta_requests().len(), 1);
    assert!(fake.downloads().is_empty());
    assert!(cursor(&backend).is_none());

    // A stored position elsewhere is not asked either: the folder is read
    // from its first message.
    fake.foreign_links(false);
    backend
        .save_cursor(
            "1",
            0,
            "INBOX",
            Cursor {
                generation: fake.inbox(),
                position: "http://graph.evil.example/v1.0/users/x/messages/delta?$deltatoken=0.0.1"
                    .into(),
            },
        )
        .await
        .unwrap();
    fake.clear_log();
    run(&cfg, &backend).await.result.unwrap();
    assert!(fake.delta_requests()[0].ends_with("changeType=created&$select=id"));
    assert_eq!(backend.report_count(), 3);
}

#[tokio::test]
async fn a_listing_longer_than_a_round_is_imported_round_by_round() {
    let fake = FakeGraph::start(MAILBOX).await;
    let inbox = fake.inbox();
    let n = u32::try_from(ROUND_PAGES).unwrap() + 3;
    fake.deliver_reports(&inbox, &(1..=n).collect::<Vec<_>>());
    fake.page_size(1);
    let cfg = config(&fake, MAILBOX, "INBOX", Tokens::new(&["t"]));
    let backend = MemoryBackend::default();
    let run = run(&cfg, &backend).await;
    run.result.unwrap();
    assert_eq!(run.summary.reports_ingested, n);
    assert!(cursor(&backend).unwrap().position.contains("$deltatoken="));
}

#[tokio::test]
async fn refusals_are_said_as_the_host_words_them() {
    let fake = FakeGraph::start(MAILBOX).await;
    let tokens = Tokens::new(&["t"]);

    let other = config(&fake, "other@example.com", "INBOX", Arc::clone(&tokens));
    let err = run(&other, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Connect(ConnectStage::Auth));
    // The host's sentence alone; Graph's answer is recorded apart.
    assert_eq!(
        err.message, r#"NoAccess { mailbox: "other@example.com" }"#,
        "{err}"
    );
    assert!(err.detail.starts_with("ErrorAccessDenied: "), "{err}");

    let unknown = config(&fake, UNKNOWN_USER, "INBOX", Arc::clone(&tokens));
    let err = run(&unknown, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Connect(ConnectStage::Auth));
    assert!(err.message.starts_with("NoMailbox"), "{err}");
    assert!(err.detail.contains("ErrorInvalidUser"), "{err}");

    let missing = config(&fake, MAILBOX, "Reports", Arc::clone(&tokens));
    let err = run(&missing, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Server);
    assert_eq!(
        err.message,
        r#"NoFolder { mailbox: "dmarc@example.com", folder: "Reports" }"#
    );

    let unreachable = GraphConfig {
        api: GraphApi::new(Url::parse("http://127.0.0.1:9").unwrap()),
        ..config(&fake, MAILBOX, "INBOX", tokens)
    };
    let err = run(&unreachable, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Connect(ConnectStage::Tcp));
    assert!(err.message.starts_with("Unreachable"), "{err}");
}

#[tokio::test]
async fn nested_folders_are_found_by_their_path_without_case() {
    let fake = FakeGraph::start(MAILBOX).await;
    let inbox = fake.inbox();
    let dmarc = fake.add_folder(Some(&inbox), "DMARC");
    let archive = fake.add_folder(None, "Archive");
    let reports = fake.add_folder(Some(&archive), "Reports 2026");
    fake.deliver_reports(&dmarc, &[1]);
    fake.deliver_reports(&reports, &[2, 3]);
    let tokens = Tokens::new(&["t"]);
    for (path, id, count) in [
        ("inbox/dmarc", &dmarc, 1),
        ("Archive/reports 2026/", &reports, 2),
    ] {
        let backend = MemoryBackend::default();
        let cfg = config(&fake, MAILBOX, path, Arc::clone(&tokens));
        let run = run(&cfg, &backend).await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, count, "{path}");
        assert_eq!(&backend.cursor_of(path).unwrap().generation, id);
    }
    let nowhere = config(&fake, MAILBOX, "Archive/Missing", tokens);
    let err = run(&nowhere, &MemoryBackend::default())
        .await
        .result
        .unwrap_err();
    assert!(err.message.starts_with("NoFolder"), "{err}");
}

#[tokio::test]
async fn the_connection_test_lists_every_folder_as_a_path() {
    let fake = FakeGraph::start(MAILBOX).await;
    let inbox = fake.inbox();
    let dmarc = fake.add_folder(Some(&inbox), "dmarc");
    fake.add_folder(Some(&dmarc), "2026");
    let archive = fake.add_folder(None, "Archive");
    fake.add_folder(None, "Sent Items");
    fake.deliver_reports(&dmarc, &[1, 2]);
    fake.deliver_reports(&archive, &[3]);
    fake.folder_page(1);
    let tokens = Tokens::new(&["t"]);

    let probe = probe_graph(&config(&fake, MAILBOX, "INBOX/dmarc", Arc::clone(&tokens)))
        .await
        .unwrap();
    let names: Vec<&str> = probe.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "INBOX",
            "Archive",
            "INBOX/dmarc",
            "INBOX/dmarc/2026",
            "Sent Items"
        ]
    );
    assert!(probe
        .folders
        .iter()
        .all(|f| f.delimiter.as_deref() == Some("/")));
    assert!(probe.selected.exists);
    assert_eq!(probe.selected.messages, Some(2));

    let probe = probe_graph(&config(&fake, MAILBOX, "Nope", Arc::clone(&tokens)))
        .await
        .unwrap();
    assert!(!probe.selected.exists);
    assert_eq!(probe.selected.messages, None);

    fake.forbid(true);
    let err = probe_graph(&config(&fake, MAILBOX, "INBOX", tokens))
        .await
        .unwrap_err();
    assert_eq!(err.stage, ConnectStage::Auth);
    assert!(err.message.starts_with("NoAccess"), "{err}");
    assert!(err.detail.starts_with("ErrorAccessDenied"), "{err}");
    assert_eq!(err.docs, Some("role"));
}

#[test]
fn graph_ids_are_named_by_their_id_and_kept_in_listing_order() {
    let a = GraphId::test("AAMk=b", 1);
    let b = GraphId::test("AAMk=a", 2);
    assert!(a < b);
    assert_eq!(a.key(), "AAMk=b");
    assert_eq!(a.label(), "message AAMk=b");
    assert_eq!(GraphId::span(&a, &b), "messages AAMk=b to AAMk=a");
    assert_eq!(GraphId::span(&a, &a), "message AAMk=b");
    let many: Vec<GraphId> = (1..=5)
        .map(|n| GraphId::test(&format!("m{n}"), n))
        .collect();
    assert_eq!(GraphId::list(&many), "messages m1, m2, m3 and 2 more");
    assert_eq!(a.mark(), None);
}

#[test]
fn only_links_to_the_endpoint_are_its_own() {
    let api = GraphApi::new(Url::parse("https://graph.microsoft.com").unwrap());
    let owns = |link: &str| api.owns(&Url::parse(link).unwrap());
    assert!(owns(
        "https://graph.microsoft.com/v1.0/users/a/messages/delta?$deltatoken=x"
    ));
    assert!(owns("https://graph.microsoft.com:443/v1.0/users"));
    assert!(!owns("http://graph.microsoft.com/v1.0/users"));
    assert!(!owns("https://graph.microsoft.com.evil.example/v1.0/users"));
    assert!(!owns("https://graph.microsoft.com:8443/v1.0/users"));
    assert!(!owns("https://graph.microsoft.com/beta/users"));
    let url = api.url(
        &["users", "a b@example.com", "messages", "AA/k=", "$value"],
        None,
    );
    assert_eq!(
        url.as_str(),
        "https://graph.microsoft.com/v1.0/users/a%20b@example.com/messages/AA%2Fk=/$value"
    );
}

fn send_config(fake: &FakeGraph, address: &str, tokens: Arc<Tokens>) -> GraphSendConfig {
    GraphSendConfig {
        address: address.into(),
        api: GraphApi::new(fake.base.clone()),
        token: tokens,
    }
}

fn mime(from: &str) -> Vec<u8> {
    format!(
        "From: Marcan <{from}>\r\nTo: owner@example.org\r\nSubject: Test\r\nMessage-ID: <1@example.com>\r\n\r\nHello \u{e9}\r\n"
    )
    .into_bytes()
}

/// The message goes to the mailbox's `sendMail` as base64 `text/plain`
/// with the bearer token, and arrives byte for byte.
#[tokio::test]
async fn a_message_is_sent_as_the_mailbox_in_mime_form() {
    let fake = FakeGraph::start(MAILBOX).await;
    fake.accept_tokens(&["t1"]);
    let cfg = send_config(&fake, MAILBOX, Tokens::new(&["t1"]));
    let mime = mime(MAILBOX);
    send_mime(&cfg, &mime).await.unwrap();
    let sent = fake.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].user, MAILBOX);
    assert_eq!(sent[0].mime, mime);
    let request = &fake.requests()[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/v1.0/users/dmarc@example.com/sendMail");
    assert_eq!(request.content_type, "text/plain");
    assert_eq!(request.auth, "Bearer t1");
}

/// A refused token is fetched anew once; refused again, the send fails
/// with Graph's words and nothing went out.
#[tokio::test]
async fn a_send_with_a_refused_token_fetches_one_new_token() {
    let fake = FakeGraph::start(MAILBOX).await;
    fake.accept_tokens(&["fresh"]);
    let tokens = Tokens::new(&["stale", "fresh"]);
    send_mime(&send_config(&fake, MAILBOX, tokens.clone()), &mime(MAILBOX))
        .await
        .unwrap();
    assert_eq!(tokens.refused.load(Ordering::SeqCst), 1);
    assert_eq!(fake.sent().len(), 1);

    let tokens = Tokens::new(&["stale", "staler"]);
    let err = send_mime(&send_config(&fake, MAILBOX, tokens.clone()), &mime(MAILBOX))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SendFailure::TokenRefused { detail } if detail.contains("InvalidAuthenticationToken")),
        "{err:?}"
    );
    assert_eq!(tokens.refused.load(Ordering::SeqCst), 1);
    assert_eq!(fake.sent().len(), 1, "nothing more went out");
}

/// Throttling is waited out within the send's bound; past it the send fails.
#[tokio::test]
async fn a_throttled_send_waits_then_gives_up() {
    let fake = FakeGraph::start(MAILBOX).await;
    fake.throttle(1, 1);
    let cfg = send_config(&fake, MAILBOX, Tokens::new(&["t"]));
    let started = std::time::Instant::now();
    send_mime(&cfg, &mime(MAILBOX)).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(fake.sent().len(), 1);

    fake.throttle(100, 0);
    let err = send_mime(&cfg, &mime(MAILBOX)).await.unwrap_err();
    assert!(
        matches!(&err, SendFailure::Throttled { detail } if detail.contains("ApplicationThrottled")),
        "{err:?}"
    );
    assert_eq!(fake.sent().len(), 1);
}

/// Graph's refusals come back with their status and code, a message too
/// large is not sent, and no token reaches a text.
#[tokio::test]
async fn a_refused_send_keeps_graphs_status_and_code() {
    let token = "secret-token-123";
    let fake = FakeGraph::start(MAILBOX).await;
    let cfg = send_config(&fake, MAILBOX, Tokens::new(&[token]));
    let refused = |err: SendFailure| match err {
        SendFailure::Refused {
            status,
            code,
            detail,
        } => (status, code, detail),
        other => panic!("{other:?}"),
    };

    fake.forbid(true);
    fake.echo_auth(true);
    let (status, code, detail) = refused(send_mime(&cfg, &mime(MAILBOX)).await.unwrap_err());
    assert_eq!((status, code.as_str()), (403, "ErrorAccessDenied"));
    assert!(!detail.contains(token), "{detail}");
    fake.forbid(false);

    let (status, code, _) = refused(
        send_mime(&cfg, &mime("alerts@example.com"))
            .await
            .unwrap_err(),
    );
    assert_eq!((status, code.as_str()), (403, "ErrorSendAsDenied"));

    let unknown = send_config(&fake, UNKNOWN_USER, Tokens::new(&[token]));
    let (status, code, _) = refused(send_mime(&unknown, &mime(UNKNOWN_USER)).await.unwrap_err());
    assert_eq!((status, code.as_str()), (404, "ErrorInvalidUser"));

    fake.refuse_send(Some((
        reqwest::StatusCode::BAD_REQUEST,
        "ErrorMimeContentInvalidBase64String",
        "Invalid base64 string for MIME content.",
    )));
    let (status, code, detail) = refused(send_mime(&cfg, &mime(MAILBOX)).await.unwrap_err());
    assert_eq!(
        (status, code.as_str()),
        (400, "ErrorMimeContentInvalidBase64String")
    );
    assert_eq!(
        detail,
        "ErrorMimeContentInvalidBase64String: Invalid base64 string for MIME content."
    );
    fake.refuse_send(None);

    let sent = fake.sent().len();
    let large = vec![b'a'; MAX_SEND_BODY_BYTES];
    let err = send_mime(&cfg, &large).await.unwrap_err();
    assert!(matches!(err, SendFailure::TooLarge { .. }), "{err:?}");
    assert_eq!(fake.sent().len(), sent, "not sent");
}
