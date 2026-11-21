//! [`MemorySender`], the [`MailSender`] of the tests: built for them and
//! for the `test-support` feature only.

use super::{MailSender, Message, SendConfig, SendError, SmtpServer};
use futures::future::BoxFuture;
use std::sync::Mutex;

/// Keeps every message instead of sending it, and answers each send with
/// `Ok` or the error it was given ([`MemorySender::failing`]). For tests.
#[derive(Debug, Default)]
pub struct MemorySender {
    sent: Mutex<Vec<Message>>,
    /// The account each kept message went out through.
    configs: Mutex<Vec<SendConfig>>,
    /// The server of each login test.
    logins: Mutex<Vec<SmtpServer>>,
    error: Option<SendError>,
}

impl MemorySender {
    /// A sender whose every send fails with `error` (nothing is kept).
    pub fn failing(error: SendError) -> Self {
        Self {
            sent: Mutex::default(),
            configs: Mutex::default(),
            logins: Mutex::default(),
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
    pub fn configs(&self) -> Vec<SendConfig> {
        self.configs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The server of each login test so far, oldest first.
    pub fn logins(&self) -> Vec<SmtpServer> {
        self.logins
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
        config: &'a SendConfig,
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

    /// Kept, then answered `Ok` or with the error it was given.
    fn login<'a>(&'a self, server: &'a SmtpServer) -> BoxFuture<'a, Result<(), SendError>> {
        self.logins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(server.clone());
        let result = self.error.clone().map_or(Ok(()), Err);
        Box::pin(std::future::ready(result))
    }
}
