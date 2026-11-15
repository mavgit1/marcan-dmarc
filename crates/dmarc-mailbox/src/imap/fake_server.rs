//! A tiny scripted IMAP server over an in-memory duplex stream, for testing a
//! fetch pass end to end without a network or TLS. It understands just the
//! commands a pass and the connection test send: LOGIN, LIST, EXAMINE, UID
//! SEARCH, UID FETCH, LOGOUT.
//!
//! Other crates reach it with the `test-support` feature, through
//! [`crate::MailboxWorker::sync_with_fake_server`] and [`probe`].

use crate::imap::config::ImapConfig;
use crate::imap::connect::{authenticate, probe_session, ConnectError, MailboxProbe};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

/// Commands received, without their tags, in order.
pub type Log = Arc<Mutex<Vec<String>>>;

/// One message in the fake mailbox.
#[derive(Clone)]
pub struct FakeMessage {
    uid: u32,
    body: Vec<u8>,
    announced: Option<u32>,
}

impl FakeMessage {
    /// Its raw RFC 822 mail (the fake Graph server serves the same ones).
    pub fn raw(&self) -> &[u8] {
        &self.body
    }

    /// A message with one valid aggregate report whose `report_id` is unique to `uid`.
    pub fn report(uid: u32) -> Self {
        Self::report_with_id(uid, &format!("fake-{uid}"))
    }

    /// The report of <code>[Self::report](original)</code> again, in message `uid`:
    /// a duplicate the import skips.
    pub fn duplicate_of(uid: u32, original: u32) -> Self {
        Self::report_with_id(uid, &format!("fake-{original}"))
    }

    fn report_with_id(uid: u32, report_id: &str) -> Self {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" ?>
<feedback>
  <report_metadata>
    <org_name>google.com</org_name>
    <email>noreply@google.com</email>
    <report_id>{report_id}</report_id>
    <date_range><begin>1704067200</begin><end>1704153599</end></date_range>
  </report_metadata>
  <policy_published>
    <domain>example.com</domain>
    <p>none</p>
  </policy_published>
  <record>
    <row>
      <source_ip>192.0.2.10</source_ip>
      <count>1</count>
      <policy_evaluated><disposition>none</disposition><dkim>pass</dkim><spf>pass</spf></policy_evaluated>
    </row>
    <identifiers><header_from>example.com</header_from></identifiers>
    <auth_results><spf><domain>example.com</domain><result>pass</result></spf></auth_results>
  </record>
</feedback>
"#
        );
        Self::with_xml_attachment(uid, &xml)
    }

    /// A message whose XML attachment is not a valid aggregate report.
    pub fn broken_report(uid: u32) -> Self {
        Self::with_xml_attachment(uid, "<feedback><report_metadata>truncated")
    }

    /// A message with an XML attachment that is some other document.
    pub fn other_xml(uid: u32) -> Self {
        Self::with_xml_attachment(uid, r#"<?xml version="1.0"?><Types xmlns="x"/>"#)
    }

    fn with_xml_attachment(uid: u32, xml: &str) -> Self {
        let b64 = STANDARD.encode(xml.as_bytes());
        let body = format!(
            "From: reporter@example.com\r\n\
             To: dmarc@example.com\r\n\
             Subject: Report Domain: example.com\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"bound\"\r\n\
             \r\n\
             --bound\r\n\
             Content-Type: text/plain\r\n\
             \r\n\
             Report.\r\n\
             --bound\r\n\
             Content-Type: application/xml\r\n\
             Content-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"google.com!example.com!{uid}.xml\"\r\n\
             \r\n\
             {b64}\r\n\
             --bound--\r\n"
        );
        Self {
            uid,
            body: body.into_bytes(),
            announced: None,
        }
    }

    /// Ordinary mail without attachments.
    pub fn plain(uid: u32) -> Self {
        Self {
            uid,
            body: b"From: a@b.c\r\nTo: d@e.f\r\nSubject: hi\r\n\r\nJust text\r\n".to_vec(),
            announced: None,
        }
    }

    /// Report this RFC822.SIZE instead of the real body length.
    pub fn announced_size(mut self, size: u32) -> Self {
        self.announced = Some(size);
        self
    }

    fn size(&self) -> u32 {
        self.announced.unwrap_or(self.body.len() as u32)
    }
}

/// A mailbox folder: its UIDVALIDITY, its messages, and the faults to script.
pub struct FakeServer {
    uid_validity: u32,
    messages: Vec<FakeMessage>,
    /// 1-based index of the body FETCH that never gets an answer.
    hang_on_body_fetch: Option<usize>,
    /// Deleted when the first body FETCH arrives.
    expunge_before_bodies: Option<u32>,
    /// Silently left out of body FETCH answers, while still existing.
    omit_body: Option<u32>,
    /// Never answer a body FETCH that includes this UID.
    hang_on_uid: Option<u32>,
    /// The only password `LOGIN` accepts; `None` accepts any.
    secret: Option<String>,
    /// How long each body FETCH takes to answer (a slow server, in tokio
    /// time, so a paused test clock skips it).
    body_fetch_delay: Option<std::time::Duration>,
    /// The folders `LIST` names; only these can be examined. Empty: just
    /// `INBOX`, and any folder can be examined.
    folders: Vec<String>,
}

impl FakeServer {
    pub fn new(uid_validity: u32, mut messages: Vec<FakeMessage>) -> Self {
        messages.sort_by_key(|m| m.uid);
        Self {
            uid_validity,
            messages,
            hang_on_body_fetch: None,
            expunge_before_bodies: None,
            omit_body: None,
            hang_on_uid: None,
            secret: None,
            body_fetch_delay: None,
            folders: Vec::new(),
        }
    }

    /// Name these folders in `LIST`; examining any other one is refused.
    pub fn with_folders(mut self, folders: &[&str]) -> Self {
        self.folders = folders.iter().map(|f| (*f).to_owned()).collect();
        self
    }

    fn has_folder(&self, name: &str) -> bool {
        self.folders.is_empty() || self.folders.iter().any(|f| f == name)
    }

    /// Take `delay` to answer each body FETCH.
    pub fn delay_body_fetch(mut self, delay: std::time::Duration) -> Self {
        self.body_fetch_delay = Some(delay);
        self
    }

    /// Accept only this password; anything else is answered `NO`.
    pub fn expect_secret(mut self, secret: &str) -> Self {
        self.secret = Some(secret.to_owned());
        self
    }

    fn accepts(&self, secret: &str) -> bool {
        self.secret.as_deref().is_none_or(|s| s == secret)
    }

    pub fn hang_on_body_fetch(mut self, nth: usize) -> Self {
        self.hang_on_body_fetch = Some(nth);
        self
    }

    pub fn expunge_after_sizes(mut self, uid: u32) -> Self {
        self.expunge_before_bodies = Some(uid);
        self
    }

    pub fn omit_body(mut self, uid: u32) -> Self {
        self.omit_body = Some(uid);
        self
    }

    pub fn hang_on_uid(mut self, uid: u32) -> Self {
        self.hang_on_uid = Some(uid);
        self
    }

    fn matching(&self, set: &str) -> Vec<(usize, &FakeMessage)> {
        let max = self.messages.last().map_or(0, |m| m.uid);
        let ranges: Vec<(u32, u32)> = set
            .split(',')
            .map(|part| {
                let bound = |s: &str| if s == "*" { max } else { s.parse().unwrap() };
                match part.split_once(':') {
                    Some((a, b)) => {
                        let (a, b) = (bound(a), bound(b));
                        (a.min(b), a.max(b))
                    }
                    None => (bound(part), bound(part)),
                }
            })
            .collect();
        self.messages
            .iter()
            .enumerate()
            .filter(|(_, m)| ranges.iter().any(|&(a, b)| m.uid >= a && m.uid <= b))
            .map(|(i, m)| (i + 1, m))
            .collect()
    }

    async fn serve(mut self, io: DuplexStream, log: Log) {
        let (read, mut write) = tokio::io::split(io);
        let mut lines = BufReader::new(read).lines();
        let mut body_fetches = 0usize;
        write.write_all(b"* OK fake IMAP ready\r\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            let Some((tag, command)) = line.split_once(' ') else {
                continue;
            };
            log.lock().unwrap().push(command.to_string());
            let upper = command.to_ascii_uppercase();
            let mut out: Vec<u8> = Vec::new();
            if upper.starts_with("LOGIN ") {
                let password = command
                    .split_whitespace()
                    .nth(2)
                    .unwrap_or_default()
                    .trim_matches('"');
                if !self.accepts(password) {
                    let refused =
                        format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n");
                    if write.write_all(refused.as_bytes()).await.is_err() {
                        return;
                    }
                    continue;
                }
            } else if upper.starts_with("LIST ") {
                let names = if self.folders.is_empty() {
                    vec!["INBOX".to_owned()]
                } else {
                    self.folders.clone()
                };
                for name in names {
                    out.extend(format!("* LIST () \"/\" \"{name}\"\r\n").bytes());
                }
            } else if (upper.starts_with("SELECT ") || upper.starts_with("EXAMINE "))
                && !self.has_folder(
                    command
                        .split_once(' ')
                        .map_or("", |(_, name)| name.trim().trim_matches('"')),
                )
            {
                let refused = format!("{tag} NO [NONEXISTENT] No such folder\r\n");
                if write.write_all(refused.as_bytes()).await.is_err() {
                    return;
                }
                continue;
            } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                out.extend(
                    format!(
                        "* {} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY {}] UIDs valid\r\n* FLAGS (\\Seen)\r\n",
                        self.messages.len(),
                        self.uid_validity
                    )
                    .bytes(),
                );
            } else if let Some(rest) = upper.strip_prefix("UID SEARCH UID ") {
                let uids: Vec<String> = self
                    .matching(rest.trim())
                    .iter()
                    .map(|(_, m)| m.uid.to_string())
                    .collect();
                out.extend(format!("* SEARCH {}\r\n", uids.join(" ")).bytes());
            } else if upper.starts_with("UID FETCH ") {
                let rest = &command["UID FETCH ".len()..];
                let (set, query) = rest.split_once(' ').unwrap();
                if query.to_ascii_uppercase().contains("RFC822.SIZE") {
                    for (seq, m) in self.matching(set) {
                        out.extend(
                            format!("* {seq} FETCH (UID {} RFC822.SIZE {})\r\n", m.uid, m.size())
                                .bytes(),
                        );
                    }
                } else {
                    body_fetches += 1;
                    let hang_uid = self.hang_on_uid;
                    if self.hang_on_body_fetch == Some(body_fetches)
                        || self
                            .matching(set)
                            .iter()
                            .any(|(_, m)| Some(m.uid) == hang_uid)
                    {
                        std::future::pending::<()>().await;
                    }
                    if let Some(delay) = self.body_fetch_delay {
                        tokio::time::sleep(delay).await;
                    }
                    if let Some(uid) = self.expunge_before_bodies.take() {
                        self.messages.retain(|m| m.uid != uid);
                    }
                    for (seq, m) in self.matching(set) {
                        if Some(m.uid) == self.omit_body {
                            continue;
                        }
                        out.extend(
                            format!(
                                "* {seq} FETCH (UID {} BODY[] {{{}}}\r\n",
                                m.uid,
                                m.body.len()
                            )
                            .bytes(),
                        );
                        out.extend(&m.body);
                        out.extend(b")\r\n");
                    }
                }
            } else if upper.starts_with("LOGOUT") {
                out.extend(b"* BYE\r\n");
            } else {
                out.extend(format!("{tag} BAD unknown command\r\n").bytes());
                write.write_all(&out).await.unwrap();
                continue;
            }
            out.extend(format!("{tag} OK done\r\n").bytes());
            if write.write_all(&out).await.is_err() {
                return;
            }
        }
    }
}

/// A session against `server`, logged in the way `cfg` says (through the
/// same [`authenticate`] as the network), plus its command log.
pub(crate) async fn session_as(
    server: FakeServer,
    cfg: &ImapConfig,
) -> (Result<async_imap::Session<DuplexStream>, ConnectError>, Log) {
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    let log: Log = Arc::default();
    tokio::spawn(server.serve(server_io, log.clone()));
    let mut client = async_imap::Client::new(client_io);
    client.read_response().await.unwrap().expect("greeting");
    (authenticate(client, cfg).await, log)
}

/// The connection test ([`crate::connect_mailbox`]) against `server`: log
/// in the way `cfg` says, list the folders and examine `cfg.folder`.
pub async fn probe(server: FakeServer, cfg: &ImapConfig) -> Result<MailboxProbe, ConnectError> {
    let (session, _log) = session_as(server, cfg).await;
    probe_session(session?, cfg).await
}

/// A session against `server` logged in with a password, plus its command log.
#[cfg(test)]
pub(crate) async fn session(server: FakeServer) -> (async_imap::Session<DuplexStream>, Log) {
    use crate::imap::config::ImapSecurity;
    let cfg = ImapConfig {
        id: "fake".into(),
        epoch: 0,
        host: "fake".into(),
        port: 993,
        security: ImapSecurity::Tls,
        username: "u".into(),
        password: "p".into(),
        folder: "INBOX".into(),
    };
    let (session, log) = session_as(server, &cfg).await;
    (session.expect("the fake server accepts the login"), log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::config::ImapSecurity;
    use crate::imap::connect::ConnectStage;

    fn cfg(password: String) -> ImapConfig {
        ImapConfig {
            id: "m1".into(),
            epoch: 0,
            host: "fake".into(),
            port: 993,
            security: ImapSecurity::Tls,
            username: "reports@example.com".into(),
            password,
            folder: "INBOX".into(),
        }
    }

    #[tokio::test]
    async fn password_login_sends_login() {
        let server = FakeServer::new(1, vec![]).expect_secret("hunter2");
        let auth = "hunter2".into();
        let (session, log) = session_as(server, &cfg(auth)).await;
        session.expect("logged in").logout().await.unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log[0], r#"LOGIN "reports@example.com" "hunter2""#);
    }

    #[tokio::test]
    async fn a_wrong_password_is_an_auth_failure() {
        let server = FakeServer::new(1, vec![]).expect_secret("hunter2");
        let auth = "wrong".into();
        let (session, _log) = session_as(server, &cfg(auth)).await;
        let err = session.expect_err("refused");
        assert_eq!(err.stage, ConnectStage::Auth);
        assert!(err.message.contains("username and password"), "{err}");
        assert!(!err.to_string().contains("wrong"), "{err}");
    }

    #[tokio::test]
    async fn the_probe_lists_the_folders_and_examines_one() {
        let server = || {
            FakeServer::new(1, vec![FakeMessage::report(1), FakeMessage::report(2)])
                .with_folders(&["INBOX", "DMARC"])
                .expect_secret("hunter2")
        };
        let mut c = cfg("hunter2".into());
        c.folder = "DMARC".into();
        let found = probe(server(), &c).await.expect("logged in");
        let names: Vec<_> = found.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["INBOX", "DMARC"]);
        assert_eq!(found.selected.messages, Some(2));
        assert!(found.selected.exists);

        c.folder = "Missing".into();
        let missing = probe(server(), &c).await.expect("logged in");
        assert!(!missing.selected.exists);

        c.password = "wrong".into();
        let err = probe(server(), &c).await.expect_err("refused");
        assert_eq!(err.stage, ConnectStage::Auth);
    }
}
