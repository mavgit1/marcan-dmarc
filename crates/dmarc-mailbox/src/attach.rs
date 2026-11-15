//! Extract DMARC aggregate report payloads from MIME messages.

use crate::batch::format_bytes;
use mail_parser::{Message, MessageParser, MessagePart, MimeHeaders};

/// Report payloads larger than this are refused (and the message quarantined).
const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// A candidate DMARC report payload extracted from an email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportAttachment {
    /// Filename hint (e.g. `report.xml.gz`), if present.
    pub filename: Option<String>,
    /// MIME content type string (e.g. `application/gzip`), if present.
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

impl ReportAttachment {
    /// Hint string suitable for [`dmarc_parser::parse_report`].
    pub fn parse_hint(&self) -> Option<String> {
        match (&self.filename, &self.content_type) {
            (Some(name), Some(ct)) => Some(format!("{name}; {ct}")),
            (Some(name), None) => Some(name.clone()),
            (None, Some(ct)) => Some(ct.clone()),
            (None, None) => None,
        }
    }
}

/// One report-looking part of a message: its payload, or why it was refused.
pub type Candidate = Result<ReportAttachment, String>;

/// Parse a raw RFC822 message and collect likely DMARC report attachments.
/// A report-looking part over [`MAX_ATTACHMENT_BYTES`] is an `Err` with the
/// reason, so the caller can quarantine the message instead of losing it.
///
/// Detection order for each MIME part:
/// 1. Filename ends with `.xml`, `.gz` (incl. `.xml.gz`), or `.zip`
/// 2. Content-Type is xml / gzip / zip
/// 3. Magic bytes (gzip `1f 8b`, zip `PK`)
///
/// Attachment parts are preferred; if none match, binary / non-text body parts
/// are scanned with the same heuristics (some senders put the report in the body).
pub fn extract_report_attachments(raw_email: &[u8]) -> Vec<Candidate> {
    let Some(message) = MessageParser::default()
        .with_mime_headers()
        .parse(raw_email)
    else {
        return Vec::new();
    };

    let mut out = Vec::new();

    for part in message.attachments() {
        if let Some(att) = candidate_from_part(part) {
            out.push(att);
        }
    }

    if out.is_empty() {
        // Fallback: scan non-attachment parts (inline body payloads).
        for part in &message.parts {
            if part.is_multipart() || part.is_message() || part.is_text() {
                continue;
            }
            if let Some(att) = candidate_from_part(part) {
                out.push(att);
            }
        }
    }

    // Last resort: if the whole message is a single binary body with magic bytes.
    if out.is_empty() {
        if let Some(att) = candidate_from_root_body(&message) {
            out.push(att);
        }
    }

    out
}

fn candidate_from_part(part: &MessagePart<'_>) -> Option<Candidate> {
    let data = part.contents();
    if data.is_empty() {
        return None;
    }

    let filename = part.attachment_name().map(ToOwned::to_owned);
    let content_type = part.content_type().map(|ct| match ct.subtype() {
        Some(sub) => format!("{}/{}", ct.ctype(), sub),
        None => ct.ctype().to_owned(),
    });

    if !is_dmarc_payload(filename.as_deref(), content_type.as_deref(), data) {
        return None;
    }
    if data.len() > MAX_ATTACHMENT_BYTES {
        return Some(Err(format!(
            "attachment {} is {} and exceeds the {} limit; it was not imported",
            filename.as_deref().unwrap_or("(unnamed)"),
            format_bytes(data.len() as u64),
            format_bytes(MAX_ATTACHMENT_BYTES as u64)
        )));
    }
    Some(Ok(ReportAttachment {
        filename,
        content_type,
        data: data.to_vec(),
    }))
}

fn candidate_from_root_body(message: &Message<'_>) -> Option<Candidate> {
    let root = message.root_part();
    if root.is_multipart() || root.is_message() {
        return None;
    }
    candidate_from_part(root)
}

/// Returns true if this part looks like a DMARC aggregate report payload.
pub fn is_dmarc_payload(filename: Option<&str>, content_type: Option<&str>, data: &[u8]) -> bool {
    let name = filename.unwrap_or("").to_ascii_lowercase();
    let ct = content_type.unwrap_or("").to_ascii_lowercase();

    name_looks_like_report(&name) || content_type_looks_like_report(&ct) || has_compress_magic(data)
}

fn name_looks_like_report(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.ends_with(".xml") || name.ends_with(".gz") || name.ends_with(".zip")
}

fn content_type_looks_like_report(ct: &str) -> bool {
    if ct.is_empty() {
        return false;
    }
    ct.contains("xml") || ct.contains("gzip") || ct.contains("zip")
}

fn has_compress_magic(data: &[u8]) -> bool {
    data.len() >= 2
        && ((data[0] == 0x1f && data[1] == 0x8b) || (data[0] == b'P' && data[1] == b'K'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    const SAMPLE_XML: &str = r#"<?xml version="1.0"?>
<feedback>
  <report_metadata>
    <org_name>google.com</org_name>
    <email>noreply@google.com</email>
    <report_id>abc-123</report_id>
    <date_range><begin>1</begin><end>2</end></date_range>
  </report_metadata>
  <policy_published>
    <domain>example.com</domain>
    <p>none</p>
  </policy_published>
</feedback>
"#;

    fn gzip_bytes(plain: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(plain).unwrap();
        enc.finish().unwrap()
    }

    fn build_multipart_email(filename: &str, content_type: &str, payload: &[u8]) -> Vec<u8> {
        use std::fmt::Write as _;
        let encoded = STANDARD.encode(payload);
        let b64 = encoded
            .as_bytes()
            .chunks(76)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n");

        let mut mail = String::new();
        write!(
            &mut mail,
            "From: reporter@example.com\r\n\
             To: dmarc@example.com\r\n\
             Subject: Report Domain: example.com\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"bound\"\r\n\
             \r\n\
             --bound\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             \r\n\
             This is a DMARC aggregate report.\r\n\
             --bound\r\n\
             Content-Type: {content_type}\r\n\
             Content-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"{filename}\"\r\n\
             \r\n\
             {b64}\r\n\
             --bound--\r\n"
        )
        .unwrap();
        mail.into_bytes()
    }

    #[test]
    fn detects_xml_gz_attachment_by_filename() {
        let gz = gzip_bytes(SAMPLE_XML.as_bytes());
        let email =
            build_multipart_email("google.com!example.com!1!2.xml.gz", "application/gzip", &gz);
        let found = extract_report_attachments(&email);
        assert_eq!(found.len(), 1);
        let found = found[0].as_ref().unwrap();
        assert_eq!(
            found.filename.as_deref(),
            Some("google.com!example.com!1!2.xml.gz")
        );
        assert_eq!(found.data, gz);
    }

    #[test]
    fn detects_zip_by_magic_bytes_without_name() {
        assert!(is_dmarc_payload(None, None, &[b'P', b'K', 0x03, 0x04]));
        assert!(is_dmarc_payload(None, None, &[0x1f, 0x8b, 0x08]));
        assert!(!is_dmarc_payload(None, None, b"<html>"));
    }

    #[test]
    fn name_and_content_type_heuristics() {
        assert!(is_dmarc_payload(Some("r.xml"), None, b"x"));
        assert!(is_dmarc_payload(Some("r.zip"), None, b"x"));
        assert!(is_dmarc_payload(None, Some("application/xml"), b"x"));
        assert!(is_dmarc_payload(None, Some("text/xml"), b"x"));
        assert!(!is_dmarc_payload(
            None,
            Some("application/octet-stream"),
            b"hello"
        ));
        assert!(is_dmarc_payload(
            Some("report.bin"),
            Some("application/octet-stream"),
            &[0x1f, 0x8b]
        ));
    }

    #[test]
    fn parse_hint_combines_filename_and_type() {
        let att = ReportAttachment {
            filename: Some("a.xml.gz".into()),
            content_type: Some("application/gzip".into()),
            data: vec![],
        };
        assert_eq!(
            att.parse_hint().as_deref(),
            Some("a.xml.gz; application/gzip")
        );
    }

    #[test]
    fn refuses_report_attachments_exceeding_25mb() {
        let huge = vec![0u8; 26 * 1024 * 1024];
        let email = build_multipart_email("large.xml.gz", "application/gzip", &huge);
        let found = extract_report_attachments(&email);
        assert_eq!(found.len(), 1);
        let reason = found[0].as_ref().unwrap_err();
        assert!(reason.contains("large.xml.gz"), "{reason}");
        assert!(reason.contains("26.0 MB"), "{reason}");
        assert!(reason.contains("25.0 MB limit"), "{reason}");
    }

    #[test]
    fn large_attachments_that_are_not_reports_are_ignored() {
        let huge = vec![b'a'; 26 * 1024 * 1024];
        let email = build_multipart_email("slides.pdf", "application/pdf", &huge);
        assert!(extract_report_attachments(&email).is_empty());
    }

    #[test]
    fn ignores_plain_text_only_email() {
        let email = b"From: a@b.c\r\nTo: d@e.f\r\nSubject: hi\r\n\r\nJust text\r\n";
        assert!(extract_report_attachments(email).is_empty());
    }
}
