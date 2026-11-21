//! Where messages go: the [`MailSender`] on `AppState`, the network one
//! (SMTP or Microsoft Graph, as each [`SendConfig`] says) paced by
//! [`PacedSender`]; tests use `MemorySender`.

use super::{login, send, Message, SendConfig, SendError, SmtpServer};
use futures::future::BoxFuture;
use std::fmt;
use std::time::Duration;

/// Where messages go. The server holds one in [`AppState`](crate::AppState):
/// [`NetworkSender`] paced by [`PacedSender`] in production; tests put a
/// `MemorySender` there, so no test opens a socket.
pub trait MailSender: Send + Sync {
    fn send<'a>(
        &'a self,
        config: &'a SendConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>>;

    /// Log in to `server` and leave, sending nothing (the account test's
    /// SMTP line of a password login). A sender that only keeps messages (a
    /// test's) accepts every login.
    fn login<'a>(&'a self, _server: &'a SmtpServer) -> BoxFuture<'a, Result<(), SendError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// [`send`] the way each configuration says: through its SMTP server, or
/// as its Microsoft 365 mailbox over Graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkSender;

impl MailSender for NetworkSender {
    fn send<'a>(
        &'a self,
        config: &'a SendConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>> {
        Box::pin(send(config, message))
    }

    fn login<'a>(&'a self, server: &'a SmtpServer) -> BoxFuture<'a, Result<(), SendError>> {
        Box::pin(login(server))
    }
}

/// The least time between the starts of two sends, process-wide: test sends
/// and previews included. A fixed bound, not a setting: whoever can reach
/// Marcan can press a test button, and this caps how fast that can mail
/// anyone (12 messages a minute). Many recipients take a while; nothing is
/// dropped.
pub const SEND_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// Another [`MailSender`], at most one send per `interval`. Callers queue in
/// the order they arrive (a fair `tokio` mutex held for the whole send), so
/// sends never overlap and none is dropped; each waits until `interval` after
/// the start of the one before, whichever way each leaves. The server's
/// production sender is [`PacedSender::network`].
pub struct PacedSender<S> {
    inner: S,
    interval: Duration,
    /// When the last send started; `None` before the first.
    last_start: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl<S> PacedSender<S> {
    pub fn new(inner: S, interval: Duration) -> Self {
        Self {
            inner,
            interval,
            last_start: tokio::sync::Mutex::new(None),
        }
    }
}

impl PacedSender<NetworkSender> {
    /// [`NetworkSender`] at [`SEND_MIN_INTERVAL`]: what the server sends through.
    pub fn network() -> Self {
        Self::new(NetworkSender, SEND_MIN_INTERVAL)
    }
}

impl<S> fmt::Debug for PacedSender<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PacedSender")
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

impl<S: MailSender> MailSender for PacedSender<S> {
    fn send<'a>(
        &'a self,
        config: &'a SendConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>> {
        Box::pin(async move {
            let mut last_start = self.last_start.lock().await;
            if let Some(last) = *last_start {
                tokio::time::sleep_until(last + self.interval).await;
            }
            *last_start = Some(tokio::time::Instant::now());
            self.inner.send(config, message).await
        })
    }

    /// A login sends no mail, so it is not paced.
    fn login<'a>(&'a self, server: &'a SmtpServer) -> BoxFuture<'a, Result<(), SendError>> {
        self.inner.login(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::config::tests::config;
    use crate::mail::parse_address;
    use std::sync::Mutex;

    /// Records when each message reached it, on tokio's clock.
    #[derive(Default)]
    struct Stamps(Mutex<Vec<(String, tokio::time::Instant)>>);

    impl MailSender for Stamps {
        fn send<'a>(
            &'a self,
            _config: &'a SendConfig,
            message: &'a Message,
        ) -> BoxFuture<'a, Result<(), SendError>> {
            self.0
                .lock()
                .unwrap()
                .push((message.subject.clone(), tokio::time::Instant::now()));
            Box::pin(std::future::ready(Ok(())))
        }
    }

    fn message(subject: &str) -> Message {
        Message {
            to: parse_address("owner@example.org").unwrap(),
            subject: subject.into(),
            text: String::new(),
            html: None,
            images: Vec::new(),
        }
    }

    /// On a paused clock: the first send goes at once, every later one
    /// [`SEND_MIN_INTERVAL`] after the one before, concurrent callers in
    /// the order they came, none dropped.
    #[tokio::test(start_paused = true)]
    async fn sends_are_spaced_and_queued() {
        let config = config();
        let paced = std::sync::Arc::new(PacedSender::new(Stamps::default(), SEND_MIN_INTERVAL));
        let start = tokio::time::Instant::now();

        paced.send(&config, &message("first")).await.unwrap();
        let tasks: Vec<_> = ["second", "third", "fourth"]
            .into_iter()
            .map(|subject| {
                let (paced, config) = (paced.clone(), config.clone());
                // The test runtime has one thread: the tasks reach the queue in spawn order.
                tokio::spawn(async move { paced.send(&config, &message(subject)).await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let stamps = paced.inner.0.lock().unwrap().clone();
        let seconds: Vec<u64> = stamps
            .iter()
            .map(|(_, at)| (*at - start).as_secs())
            .collect();
        assert_eq!(seconds, [0, 5, 10, 15]);
        let order: Vec<&str> = stamps.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(order, ["first", "second", "third", "fourth"]);

        // After a quiet spell longer than the interval, the next goes at once.
        tokio::time::sleep(Duration::from_secs(60)).await;
        let before = tokio::time::Instant::now();
        paced.send(&config, &message("later")).await.unwrap();
        assert_eq!(paced.inner.0.lock().unwrap().last().unwrap().1, before);
    }
}
