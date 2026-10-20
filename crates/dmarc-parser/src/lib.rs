//! DMARC aggregate report XML parser.
//!
//! Parses the standard `feedback` document and auto-detects plain XML,
//! gzip (`.xml.gz`), or zip archives containing a report XML file.

mod alignment;
mod decompress;
mod types;
mod xml;

pub use alignment::{dkim_auth_aligned, spf_auth_aligned, spf_counts_for_dmarc, AlignmentMode};
pub use types::*;

use thiserror::Error;

/// Errors produced while decompressing or parsing a DMARC report.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("failed to decompress report: {0}")]
    Decompress(String),
    #[error("no XML entry found in zip archive")]
    EmptyZip,
    #[error("invalid UTF-8 in report XML: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("failed to parse DMARC XML: {0}")]
    Xml(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid report: {0}")]
    Invalid(String),
    /// Decodes fine but is not a DMARC aggregate report (no `<feedback>`
    /// document): an office file, a TLS-RPT report. Not a failure.
    #[error("not a DMARC aggregate report")]
    NotDmarc,
}

/// Parse raw report bytes using an optional filename or content-type hint.
///
/// Detection order:
/// 1. Magic bytes (gzip `1f 8b`, zip `PK`)
/// 2. Filename / content-type hint (`.gz`, `.zip`, `gzip`, `zip`)
/// 3. Treat as plain XML
pub fn parse_report(data: &[u8], hint: Option<&str>) -> Result<Report, ParseError> {
    let xml_bytes = decompress::to_xml_bytes(data, hint)?;
    if !looks_like_feedback(&xml_bytes) {
        return Err(ParseError::NotDmarc);
    }
    let raw_xml = String::from_utf8(xml_bytes)?;
    xml::parse_feedback_xml(&raw_xml)
}

/// Whether decoded bytes carry a `<feedback>` document (searched within the
/// first 4 KiB, past any XML declaration, comments or BOM).
fn looks_like_feedback(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    head.windows(b"<feedback".len()).any(|w| w == b"<feedback")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    /// Realistic synthetic aggregate report covering two records.
    pub(crate) const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<feedback>
  <report_metadata>
    <org_name>google.com</org_name>
    <email>noreply-dmarc-support@google.com</email>
    <extra_contact_info>https://support.google.com/a/answer/2466580</extra_contact_info>
    <report_id>12345678901234567890</report_id>
    <date_range>
      <begin>1704067200</begin>
      <end>1704153599</end>
    </date_range>
  </report_metadata>
  <policy_published>
    <domain>example.com</domain>
    <adkim>r</adkim>
    <aspf>r</aspf>
    <p>quarantine</p>
    <sp>none</sp>
    <pct>100</pct>
  </policy_published>
  <record>
    <row>
      <source_ip>192.0.2.10</source_ip>
      <count>42</count>
      <policy_evaluated>
        <disposition>none</disposition>
        <dkim>pass</dkim>
        <spf>pass</spf>
      </policy_evaluated>
    </row>
    <identifiers>
      <header_from>example.com</header_from>
      <envelope_from>mail.example.com</envelope_from>
    </identifiers>
    <auth_results>
      <dkim>
        <domain>example.com</domain>
        <selector>google</selector>
        <result>pass</result>
      </dkim>
      <spf>
        <domain>example.com</domain>
        <result>pass</result>
      </spf>
    </auth_results>
  </record>
  <record>
    <row>
      <source_ip>198.51.100.7</source_ip>
      <count>3</count>
      <policy_evaluated>
        <disposition>quarantine</disposition>
        <dkim>fail</dkim>
        <spf>fail</spf>
      </policy_evaluated>
    </row>
    <identifiers>
      <header_from>example.com</header_from>
    </identifiers>
    <auth_results>
      <dkim>
        <domain>evil.example</domain>
        <result>fail</result>
      </dkim>
      <spf>
        <domain>evil.example</domain>
        <result>fail</result>
      </spf>
    </auth_results>
  </record>
</feedback>
"#;

    fn assert_sample_report(report: &Report) {
        assert_eq!(report.report_id, "12345678901234567890");
        assert_eq!(report.org_name, "google.com");
        assert_eq!(report.email, "noreply-dmarc-support@google.com");
        assert_eq!(report.date_begin, 1_704_067_200);
        assert_eq!(report.date_end, 1_704_153_599);
        assert_eq!(report.policy.domain, "example.com");
        assert_eq!(report.policy.adkim.as_deref(), Some("r"));
        assert_eq!(report.policy.aspf.as_deref(), Some("r"));
        assert_eq!(report.policy.p.as_deref(), Some("quarantine"));
        assert_eq!(report.policy.sp.as_deref(), Some("none"));
        assert_eq!(report.policy.pct, Some(100));
        assert_eq!(report.records.len(), 2);

        let r0 = &report.records[0];
        assert_eq!(r0.source_ip, "192.0.2.10");
        assert_eq!(r0.count, 42);
        assert_eq!(r0.disposition.as_deref(), Some("none"));
        assert_eq!(r0.dkim_result.as_deref(), Some("pass"));
        assert_eq!(r0.spf_result.as_deref(), Some("pass"));
        assert_eq!(r0.header_from.as_deref(), Some("example.com"));
        assert_eq!(r0.envelope_from.as_deref(), Some("mail.example.com"));
        assert_eq!(r0.dkim_auth.len(), 1);
        assert_eq!(r0.dkim_auth[0].domain, "example.com");
        assert_eq!(r0.dkim_auth[0].selector.as_deref(), Some("google"));
        assert_eq!(r0.dkim_auth[0].result, "pass");
        assert_eq!(r0.spf_auth.len(), 1);

        let r1 = &report.records[1];
        assert_eq!(r1.source_ip, "198.51.100.7");
        assert_eq!(r1.count, 3);
        assert_eq!(r1.disposition.as_deref(), Some("quarantine"));
        assert!(r1.envelope_from.is_none());
        assert!(r1.dkim_auth[0].selector.is_none());
    }

    #[test]
    fn parses_plain_xml() {
        let report = parse_report(SAMPLE_XML.as_bytes(), Some("report.xml")).unwrap();
        assert_sample_report(&report);
        assert!(report.raw_xml.contains("<feedback>"));
    }

    #[test]
    fn parses_gzip_compressed_xml() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(SAMPLE_XML.as_bytes()).unwrap();
        let gz = encoder.finish().unwrap();

        let report = parse_report(&gz, Some("report.xml.gz")).unwrap();
        assert_sample_report(&report);
    }

    #[test]
    fn parses_gzip_by_magic_bytes_without_hint() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(SAMPLE_XML.as_bytes()).unwrap();
        let gz = encoder.finish().unwrap();

        let report = parse_report(&gz, None).unwrap();
        assert_eq!(report.report_id, "12345678901234567890");
    }

    #[test]
    fn parses_zip_archive() {
        use std::io::Cursor;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let cursor = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(cursor);
        zip.start_file(
            "google.com!example.com!1704067200!1704153599.xml",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(SAMPLE_XML.as_bytes()).unwrap();
        let cursor = zip.finish().unwrap();
        let bytes = cursor.into_inner();

        let report = parse_report(&bytes, Some("report.zip")).unwrap();
        assert_sample_report(&report);
    }

    #[test]
    fn gzip_bomb_rejected_by_50mb_limit() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let chunk = [0u8; 1024 * 1024]; // 1MB of zeros
        for _ in 0..51 {
            encoder.write_all(&chunk).unwrap();
        }
        let gz = encoder.finish().unwrap();

        let err = parse_report(&gz, Some("bomb.xml.gz")).unwrap_err();
        match err {
            ParseError::Decompress(msg) => assert!(msg.contains("exceeds 50 MB limit")),
            other => panic!("expected Decompress error, got {other:?}"),
        }
    }

    fn edited(from: &str, to: &str) -> String {
        assert!(SAMPLE_XML.contains(from), "{from}");
        SAMPLE_XML.replacen(from, to, 1)
    }

    #[test]
    fn empty_optional_fields_do_not_reject_the_report() {
        let xml = edited("<pct>100</pct>", "<pct/>");
        assert_eq!(parse_report(xml.as_bytes(), None).unwrap().policy.pct, None);
        let xml = edited(
            "<domain>example.com</domain>\n        <selector>google</selector>",
            "<selector>google</selector>",
        );
        let report = parse_report(xml.as_bytes(), None).unwrap();
        assert_eq!(report.records[0].dkim_auth[0].domain, "");
    }

    #[test]
    fn untrusted_values_are_validated_or_canonicalized() {
        for (from, to) in [
            (
                "<domain>example.com</domain>\n    <adkim>",
                "<domain>evil.com/../x</domain>\n    <adkim>",
            ),
            ("<count>42</count>", "<count>-5</count>"),
            ("<count>42</count>", "<count>1000000001</count>"),
            (
                "<source_ip>192.0.2.10</source_ip>",
                "<source_ip>not-an-ip</source_ip>",
            ),
        ] {
            let err = parse_report(edited(from, to).as_bytes(), None).unwrap_err();
            assert!(matches!(err, ParseError::Invalid(_)), "{to}: {err:?}");
        }
        let xml = edited(
            "<source_ip>192.0.2.10</source_ip>",
            "<source_ip> 2001:0DB8:0000::0001 </source_ip>",
        );
        assert_eq!(
            parse_report(xml.as_bytes(), None).unwrap().records[0].source_ip,
            "2001:db8::1"
        );
        let xml = edited(
            "<domain>example.com</domain>\n    <adkim>",
            "<domain>Example.COM.</domain>\n    <adkim>",
        );
        assert_eq!(
            parse_report(xml.as_bytes(), None).unwrap().policy.domain,
            "example.com"
        );
    }

    #[test]
    fn message_count_limit_is_inclusive() {
        let xml = edited("<count>42</count>", "<count>1000000000</count>");
        let report = parse_report(xml.as_bytes(), None).unwrap();
        assert_eq!(report.records[0].count, xml::MAX_RECORD_COUNT);
        // Beyond i64 the XML itself is rejected.
        let xml = edited("<count>42</count>", "<count>99999999999999999999</count>");
        assert!(parse_report(xml.as_bytes(), None).is_err());
    }

    #[test]
    fn implausible_date_ranges_are_rejected() {
        let backwards = edited("<end>1704153599</end>", "<end>1704067199</end>");
        let err = parse_report(backwards.as_bytes(), None).unwrap_err();
        assert!(
            matches!(err, ParseError::Invalid(ref m) if m.contains("before")),
            "{err:?}"
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let range = |begin: i64| {
            edited(
                "<begin>1704067200</begin>\n      <end>1704153599</end>",
                &format!("<begin>{begin}</begin><end>{}</end>", begin + 86_399),
            )
        };
        let future = range(now + 2 * 86_400);
        let err = parse_report(future.as_bytes(), None).unwrap_err();
        assert!(
            matches!(err, ParseError::Invalid(ref m) if m.contains("future")),
            "{err:?}"
        );

        // Today's report, or one from a reporter a few hours ahead, is fine.
        assert!(parse_report(range(now + 3_600).as_bytes(), None).is_ok());
        // A zero-length period is not backwards.
        let instant = edited("<end>1704153599</end>", "<end>1704067200</end>");
        assert!(parse_report(instant.as_bytes(), None).is_ok());
    }

    #[test]
    fn other_documents_are_not_dmarc() {
        let tls_rpt = br#"{"organization-name":"Google Inc.","policies":[]}"#;
        assert!(matches!(
            parse_report(tls_rpt, None),
            Err(ParseError::NotDmarc)
        ));
        let other_xml = b"<?xml version=\"1.0\"?><Types xmlns=\"x\"/>";
        assert!(matches!(
            parse_report(other_xml, Some("[Content_Types].xml")),
            Err(ParseError::NotDmarc)
        ));
    }
}
