//! A tiny scripted IMAP server over an in-memory duplex stream, for testing a
//! fetch pass end to end without a network or TLS. It understands just the
//! commands a pass sends: LOGIN, EXAMINE, UID SEARCH, UID FETCH, LOGOUT.
//!
//! Other crates reach it with the `test-support` feature, through
//! [`crate::ImapWorker::sync_with_fake_server`].

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
        }
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
            if upper.starts_with("LOGIN") {
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

/// A logged-in session against `server`, plus its command log.
pub(crate) async fn session(server: FakeServer) -> (async_imap::Session<DuplexStream>, Log) {
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    let log: Log = Arc::default();
    tokio::spawn(server.serve(server_io, log.clone()));
    let mut client = async_imap::Client::new(client_io);
    client.read_response().await.unwrap().expect("greeting");
    let session = client.login("u", "p").await.map_err(|(e, _)| e).unwrap();
    (session, log)
}
