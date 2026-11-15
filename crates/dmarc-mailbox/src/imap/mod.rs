//! Reading a folder over IMAP: the settings ([`config`]), connecting and
//! logging in with plain-language errors and the settings UI's connection
//! test ([`connect`]), and the [`Source`](crate::source::Source) a pass
//! reads through ([`ImapSource`]).
//!
//! The folder is opened with `EXAMINE` and read with `BODY.PEEK[]`, so the
//! mailbox is never changed and no `\Seen` flag is set: other clients
//! reading the same mailbox cannot starve the pass of mail by marking it
//! seen. The cursor is the folder's UIDVALIDITY (its generation) and the
//! last handled UID, in decimal; new messages are those with a higher UID.

pub(crate) mod config;
pub(crate) mod connect;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_server;
mod source;

pub(crate) use source::ImapSource;
#[cfg(test)]
pub(crate) use source::Uid;

use crate::backend::Backend;
use crate::pass::{FailureKind, FetchError, FetchSummary, Pass};
use config::ImapConfig;
use connect::{connect_and_login, ConnectError};
use tracing::{info, warn};

/// How a pass reaches the mailbox.
pub(crate) enum Transport {
    /// TCP and TLS (or STARTTLS) to the configured server.
    Network,
    /// The scripted in-memory server, for tests.
    #[cfg(any(test, feature = "test-support"))]
    Fake(fake_server::FakeServer),
}

/// Connect to `cfg`'s server and log in, run `pass` over the session, log
/// out after a pass that went well.
pub(crate) async fn run<B: Backend>(
    pass: &mut Pass<'_, B>,
    cfg: &ImapConfig,
    transport: Transport,
) -> Result<FetchSummary, FetchError> {
    let limits = *pass.limits;
    match transport {
        Transport::Network => {}
        #[cfg(any(test, feature = "test-support"))]
        Transport::Fake(server) => {
            return match fake_server::session_as(server, cfg).await {
                (Ok(session), _log) => pass.run_on(ImapSource::new(session, cfg, limits)).await,
                (Err(e), _log) => Err(connect_failed(pass, cfg, e)),
            };
        }
    }
    info!(host = %cfg.host, port = cfg.port, folder = %cfg.folder, "connecting to IMAP");
    let connected = tokio::time::timeout(limits.command_timeout, connect_and_login(cfg)).await;
    let session = match connected {
        Ok(Ok(session)) => session,
        Ok(Err(e)) => return Err(connect_failed(pass, cfg, e)),
        Err(_) => {
            return Err(pass.failed(
                FailureKind::Timeout,
                format!(
                    "connecting to {}:{} timed out after {} seconds",
                    cfg.host,
                    cfg.port,
                    limits.command_timeout.as_secs()
                ),
            ))
        }
    };
    pass.run_on(ImapSource::new(session, cfg, limits)).await
}

/// The mailbox could not be reached: the plain-language sentence is what
/// the pass records (the UI and the alerts show it); the system's own error
/// (`failed to lookup address information: …`) goes to the log only.
fn connect_failed<B: Backend>(pass: &Pass<'_, B>, cfg: &ImapConfig, e: ConnectError) -> FetchError {
    warn!(
        host = %cfg.host,
        port = cfg.port,
        detail = %e.detail,
        "{}",
        e.message
    );
    pass.failed(FailureKind::Connect(e.stage), e.message)
}
