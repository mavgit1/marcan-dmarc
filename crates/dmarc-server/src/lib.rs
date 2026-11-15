//! Marcan's HTTP API and background work as a library: the binary (`main.rs`,
//! flags `--bind`, `--data-dir`, `--allowed-hosts`, and `MARCAN_SECRET_KEY`)
//! and the integration tests
//! (`tests/common/` is their harness) build the same [`AppState`] and router.
//!
//! - `api/`: the axum routes under `/api`, one module per area; `api/settings/`
//!   answers every settings group and saves one per `PUT`: a typed body (a
//!   missing field is refused), checked or clamped before it is stored.
//! - `view/`: the wire types, store rows plus `dmarc_assess` verdicts, exported
//!   to the UI by ts-rs. Other ts-rs exports come only from the assess, dns and
//!   identity vocabularies and the store's verdict-free values.
//! - `wording/`: the server's sentences, written once for the emails and the
//!   UI: alert events, issues, policy readiness, undelivered client reports.
//! - `background.rs`: the supervisor: the mailbox loop, the DNS and identity warm
//!   loops, the alert job, the client report job, the version check and the
//!   retention clean-up on one `every` helper under one cancellation token,
//!   with graceful shutdown.
//! - `snapshots/`: persisted DNS and reverse-DNS results; [`DnsService`] on
//!   [`AppState`] holds the checker, the attempt backoff and the refreshes in
//!   flight (no statics).
//! - `alerts/`: the evaluation after each DNS warm round (at most
//!   `ALERT_INTERVAL` apart, held while the mailbox imports), then the alert
//!   emails and the digest.
//! - `client_reports/`: the recipient list parser, one organization's report
//!   read through a `Scope` pinned to it, and the hourly job that sends once a
//!   month.
//! - `mail/`: sending, as an SMTP submission client (465 or 587, never port
//!   25) or as a Microsoft 365 mailbox over Graph, behind the `MailSender`
//!   on [`AppState`], each failure a `SendError` by step and
//!   permanent or not; the alert, digest and client report emails, rendered
//!   from view types in one frame that ends in `footer()`.
//! - `updates.rs`: the daily version check against the docs site's
//!   `latest.json` (on by default; nothing is fetched while it is off).
//! - `license.rs`: operator keys (Ed25519, verified offline), the free-software
//!   notice and `SOURCE_URL`.
//! - `accounts/`: the mail accounts behind receiving and sending ([`Accounts`]
//!   on [`AppState`]): how each reads (IMAP or Graph) and sends (SMTP or
//!   Graph), the secrets sealed
//!   under `MARCAN_SECRET_KEY`, the providers' presets, and Microsoft logins'
//!   tokens (client credentials, from a token endpoint given to
//!   [`AppState`], never a setting) and certificates.
//! - `mailbox_backend.rs`: the mailbox worker's `Backend` over the store, one
//!   mailbox per receiving account.
//! - `http_policy.rs` (the `Host` check, frame headers), `extract.rs` and
//!   `error.rs` (every rejection a JSON error), `filters.rs` and `since.rs`
//!   (shared query parameters, the page envelope), `csv_text.rs`,
//!   `static_assets.rs` (the embedded UI).

pub mod accounts;
pub mod alerts;
pub mod api;
pub mod background;
pub mod client_reports;
mod csv_text;
pub mod error;
pub mod extract;
pub mod filters;
pub mod http_policy;
pub mod license;
pub mod mail;
pub mod mailbox_backend;
pub mod since;
mod snapshots;
pub mod static_assets;
pub mod updates;
pub mod view;
pub mod wording;

pub use accounts::{Accounts, SecretKey, SecretKeys};
pub use background::Background;
pub use http_policy::AllowedHosts;
pub use mailbox_backend::StoreBackend;
pub use snapshots::dns::{demo_resolver, dns_snapshot, DnsService};

use axum::middleware;
use axum::Router;
use dmarc_identity::SenderNames;
use dmarc_mailbox::MailboxWorker;
use mail::{MailSender, PacedSender};
use sqlx::SqlitePool;
use std::sync::{Arc, RwLock};
use tokio::sync::Notify;
use tower_http::trace::TraceLayer;

/// Everything a request handler or background task works with, built once in
/// `main` (or a test) and shared.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    /// Runs the mailbox passes (IMAP or Graph); shared by the polling loop and
    /// manual syncs.
    pub mailboxes: MailboxWorker,
    /// The DNS checker for the configured resolvers, this process's record of
    /// check attempts, and the refreshes in flight.
    pub dns: Arc<DnsService>,
    /// Display names for sender operators (bundled map plus the overrides from
    /// Settings); swapped whole when the overrides are saved.
    names: Arc<RwLock<Arc<SenderNames>>>,
    /// Where outgoing mail goes: the sending account in Settings (a recorder in tests).
    pub mail: Arc<dyn MailSender>,
    /// The mail accounts and the key their secrets are sealed with.
    pub accounts: Arc<Accounts>,
    /// Where the account test of a password login logs in to IMAP: the
    /// network, or a test's fake server. (A Microsoft login's test reads
    /// the accounts' Graph endpoint.)
    pub imap_prober: Arc<dyn accounts::connection::ImapProber>,
    /// The `Host` names requests may carry (any, unless `main` narrows it).
    pub allowed_hosts: Arc<AllowedHosts>,
    /// Wakes the version check (`updates.rs`) when it is switched on, so the
    /// first answer does not wait for the hourly look.
    pub update_check_wake: Arc<Notify>,
}

impl AppState {
    /// State with the bundled sender names only.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            accounts: Arc::new(Accounts::new(pool.clone(), None)),
            imap_prober: Arc::new(accounts::connection::NetworkProber),
            pool,
            mailboxes: MailboxWorker::new(),
            dns: Arc::new(DnsService::default()),
            names: Arc::new(RwLock::new(Arc::new(SenderNames::bundled()))),
            mail: Arc::new(PacedSender::network()),
            allowed_hosts: Arc::new(AllowedHosts::any()),
            update_check_wake: Arc::new(Notify::new()),
        }
    }

    /// State that answers only requests for `hosts` (see [`AllowedHosts`]).
    pub fn with_allowed_hosts(self, hosts: AllowedHosts) -> Self {
        Self {
            allowed_hosts: Arc::new(hosts),
            ..self
        }
    }

    /// State whose account secrets are sealed with `key` (`None`: stored
    /// plain), as `MARCAN_SECRET_KEY` gives it.
    pub fn with_secret_key(self, key: Option<SecretKey>) -> Self {
        self.with_secret_keys(SecretKeys {
            current: key,
            previous: None,
        })
    }

    /// State whose account secrets are sealed with `keys.current`, and
    /// whose start moves the values of `keys.previous` to it.
    pub fn with_secret_keys(self, keys: SecretKeys) -> Self {
        Self {
            accounts: Arc::new(
                Accounts::new(self.pool.clone(), keys.current)
                    .with_previous_key(keys.previous)
                    .with_tokens(self.accounts.tokens().clone())
                    .with_graph(self.accounts.graph().clone()),
            ),
            ..self
        }
    }

    /// State whose Microsoft logins get their tokens from `tokens` (tests:
    /// a fake token endpoint) instead of Microsoft's identity platform.
    pub fn with_token_source(self, tokens: accounts::oauth::TokenSource) -> Self {
        Self {
            accounts: Arc::new(self.accounts.with_token_source(Arc::new(tokens))),
            ..self
        }
    }

    /// State whose Microsoft logins read Graph at `graph` (tests: a fake
    /// Graph) instead of Microsoft Graph.
    pub fn with_graph_api(self, graph: dmarc_mailbox::GraphApi) -> Self {
        Self {
            accounts: Arc::new(self.accounts.with_graph_api(graph)),
            ..self
        }
    }

    /// State that sends mail through `sender` instead of the network (tests).
    pub fn with_mail_sender(self, sender: Arc<dyn MailSender>) -> Self {
        Self {
            mail: sender,
            ..self
        }
    }

    /// State whose DNS checks look everything up through `resolver` instead
    /// of the stored resolvers (tests: a stub that answers at once).
    pub fn with_imap_prober(self, prober: Arc<dyn accounts::connection::ImapProber>) -> Self {
        Self {
            imap_prober: prober,
            ..self
        }
    }

    pub fn with_dns_resolver(self, resolver: Arc<dyn dmarc_dns::Resolver>) -> Self {
        Self {
            dns: Arc::new(DnsService::with_resolver(resolver)),
            ..self
        }
    }

    /// State with `names` (e.g. the stored overrides) instead of the bundled map alone.
    pub fn with_sender_names(self, names: SenderNames) -> Self {
        self.set_names(names);
        self
    }

    /// The sender names in effect.
    pub fn names(&self) -> Arc<SenderNames> {
        Arc::clone(&self.names.read().expect("sender names lock"))
    }

    /// Put new sender names in effect for every later request.
    pub fn set_names(&self, names: SenderNames) {
        *self.names.write().expect("sender names lock") = Arc::new(names);
    }

    /// What the mailbox worker reads and writes: the store behind this state.
    pub fn mailbox_backend(&self) -> StoreBackend {
        StoreBackend::new(self.pool.clone(), self.dns.clone(), self.accounts.clone())
    }
}

/// Build the axum router with tracing middleware, the `Host` check, the
/// anti-framing headers and the static SPA fallback.
pub fn router(pool: SqlitePool) -> Router {
    router_with_state(AppState::new(pool))
}

/// [`router`] over existing state, so the binary can share its [`MailboxWorker`]
/// and [`DnsService`] with the background tasks.
pub fn router_with_state(state: AppState) -> Router {
    let allowed_hosts = Arc::clone(&state.allowed_hosts);
    api::routes(Arc::new(state))
        .fallback(static_assets::serve_asset)
        .layer(middleware::from_fn_with_state(
            allowed_hosts,
            http_policy::check_host,
        ))
        .layer(middleware::from_fn(http_policy::frame_headers))
        .layer(TraceLayer::new_for_http())
}
