//! Where messages go: the [`MailSender`] on `AppState`, the SMTP one paced by
//! [`PacedSender`], and [`MemorySender`] for tests.

use super::{send, Message, SendError, SmtpConfig};
use futures::future::BoxFuture;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

/// Where messages go. The server holds one in [`AppState`](crate::AppState):
/// [`SmtpSender`] paced by [`PacedSender`] in production; tests put a
/// [`MemorySender`] there, so no test opens a socket.
pub trait MailSender: Send + Sync {
    fn send<'a>(
        &'a self,
        config: &'a SmtpConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>>;
}

/// [`send`] through the configured submission server.
#[derive(Debug, Clone, Copy, Default)]
pub struct SmtpSender;

impl MailSender for SmtpSender {
    fn send<'a>(
        &'a self,
        config: &'a SmtpConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>> {
        Box::pin(send(config, message))
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
/// the start of the one before. The server's production sender is
/// [`PacedSender::smtp`].
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

impl PacedSender<SmtpSender> {
    /// [`SmtpSender`] at [`SEND_MIN_INTERVAL`]: what the server sends through.
    pub fn smtp() -> Self {
        Self::new(SmtpSender, SEND_MIN_INTERVAL)
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
        config: &'a SmtpConfig,
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
}

/// Keeps every message instead of sending it, and answers each send with
/// `Ok` or the error it was given ([`MemorySender::failing`]). For tests.
#[derive(Debug, Default)]
pub struct MemorySender {
    sent: Mutex<Vec<Message>>,
    /// The account each kept message went out through.
    configs: Mutex<Vec<SmtpConfig>>,
    error: Option<SendError>,
}

impl MemorySender {
    /// A sender whose every send fails with `error` (nothing is kept).
    pub fn failing(error: SendError) -> Self {
        Self {
            sent: Mutex::default(),
            configs: Mutex::default(),
            error: Some(error),
        }
    }

    /// The messages sent so far, oldest first.
    pub fn sent(&self) -> Vec<Message> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The account each message in [`MemorySender::sent`] went out through.
    pub fn configs(&self) -> Vec<SmtpConfig> {
        self.configs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The `From` display name of each message in [`MemorySender::sent`].
    pub fn from_names(&self) -> Vec<String> {
        self.configs().into_iter().map(|c| c.sender_name).collect()
    }
}

impl MailSender for MemorySender {
    fn send<'a>(
        &'a self,
        config: &'a SmtpConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>> {
        let result = match &self.error {
            Some(error) => Err(error.clone()),
            None => {
                self.sent
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(message.clone());
                self.configs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(config.clone());
                Ok(())
            }
        };
        Box::pin(std::future::ready(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::config::tests::settings;
    use crate::mail::parse_address;

    /// Records when each message reached it, on tokio's clock.
    #[derive(Default)]
    struct Stamps(Mutex<Vec<(String, tokio::time::Instant)>>);

    impl MailSender for Stamps {
        fn send<'a>(
            &'a self,
            _config: &'a SmtpConfig,
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
        let config = SmtpConfig::from_settings(&settings()).unwrap();
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
