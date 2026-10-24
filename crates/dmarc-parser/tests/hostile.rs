//! Crafted attachments: anyone can mail the report mailbox, so each of these
//! must end in a plain error (or a report) in bounded time and memory, never
//! a panic, a hang or an abort. The fuzz targets in `fuzz/` search for more.

use dmarc_parser::{parse_report, ParseError};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0"?><feedback><report_metadata><org_name>x</org_name><email>a@b.c</email><report_id>1</report_id><date_range><begin>1704067200</begin><end>1704153599</end></date_range></report_metadata><policy_published><domain>example.com</domain></policy_published>"#;
const RECORD: &str = "<record><row><source_ip>192.0.2.1</source_ip><count>1</count></row></record>";
const MIB: usize = 1024 * 1024;

fn report(records: usize) -> String {
    format!("{HEAD}{}</feedback>", RECORD.repeat(records))
}

/// `head`, then `chunk` `times` over, then `tail`, gzipped without holding the whole.
fn gzip_stream(head: &[u8], chunk: &[u8], times: usize, tail: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(head).unwrap();
    for _ in 0..times {
        gz.write_all(chunk).unwrap();
    }
    gz.write_all(tail).unwrap();
    gz.finish().unwrap()
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn assert_too_large(result: Result<dmarc_parser::Report, ParseError>) {
    match result {
        Err(ParseError::TooLarge(limit)) => assert_eq!(limit, dmarc_parser::MAX_REPORT_BYTES),
        other => panic!("expected the size limit, got {other:?}"),
    }
}

#[test]
fn a_million_records_are_cut_off_by_the_size_limit() {
    // 75 MB of records in a 1.3 MB gzip: refused at 50 MB, not parsed.
    let gz = gzip_stream(
        HEAD.as_bytes(),
        RECORD.as_bytes(),
        1_000_000,
        b"</feedback>",
    );
    assert!(gz.len() < 2 * MIB, "{}", gz.len());
    assert_too_large(parse_report(&gz, Some("bomb.xml.gz")));
}

#[test]
fn a_zip_bomb_is_cut_off_by_the_size_limit() {
    // One entry inflating to 200 MB.
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));
    zip.start_file("report.xml", options).unwrap();
    zip.write_all(HEAD.as_bytes()).unwrap();
    let spaces = vec![b' '; MIB];
    for _ in 0..200 {
        zip.write_all(&spaces).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert_too_large(parse_report(&bytes, Some("bomb.zip")));
    // The same bomb gzipped inside a zip.
    let inner = gzip_stream(HEAD.as_bytes(), &spaces, 200, b"");
    let nested = zip_stored("report.xml.gz", &inner);
    assert_too_large(parse_report(&nested, Some("bomb.zip")));
}

fn zip_stored(name: &str, data: &[u8]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file(name, options).unwrap();
    zip.write_all(data).unwrap();
    zip.finish().unwrap().into_inner()
}

/// Runs `f` on a thread with the stack of a tokio blocking-pool thread
/// (2 MiB), where the IMAP worker parses: a stack overflow aborts the whole
/// process, it is no panic the worker could catch.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 * MIB)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    on_small_stack(|| {
        const DEPTH: usize = 1_000_000;
        // In an element the report does not know (skipped), closed and unclosed.
        let closed = format!(
            "{HEAD}{}{}</feedback>",
            "<a>".repeat(DEPTH),
            "</a>".repeat(DEPTH)
        );
        let report = parse_report(closed.as_bytes(), None).unwrap();
        assert!(report.records.is_empty());
        let unclosed = format!("{HEAD}{}", "<a>".repeat(DEPTH));
        assert!(matches!(
            parse_report(unclosed.as_bytes(), None),
            Err(ParseError::Xml(_))
        ));
        // In a field that expects text.
        let in_field = HEAD.replace(
            "<org_name>x</org_name>",
            &format!(
                "<org_name>{}{}</org_name>",
                "<a>".repeat(DEPTH),
                "</a>".repeat(DEPTH)
            ),
        ) + "</feedback>";
        assert!(matches!(
            parse_report(in_field.as_bytes(), None),
            Err(ParseError::Xml(_))
        ));
    });
}

#[test]
fn a_huge_text_node_is_one_string_or_too_large() {
    let name = "x".repeat(20 * MIB);
    let xml = HEAD.replace(
        "<org_name>x</org_name>",
        &format!("<org_name>{name}</org_name>"),
    ) + "</feedback>";
    assert_eq!(
        parse_report(xml.as_bytes(), None).unwrap().org_name.len(),
        20 * MIB
    );
    let (head, tail) = HEAD.split_once("x</org_name>").unwrap();
    let gz = gzip_stream(
        head.as_bytes(),
        &vec![b'x'; MIB],
        60,
        format!("</org_name>{tail}</feedback>").as_bytes(),
    );
    assert_too_large(parse_report(&gz, Some("big.xml.gz")));
}

#[test]
fn zip_entries_other_than_the_report_are_passed_over() {
    let xml = report(2);
    // A non-XML entry first, then the report.
    let bytes = zip(&[("readme.txt", b"hello"), ("report.xml", xml.as_bytes())]);
    assert_eq!(
        parse_report(&bytes, Some("r.zip")).unwrap().records.len(),
        2
    );
    // Only a non-XML entry: not a report, not a failure.
    let bytes = zip(&[("readme.txt", b"hello")]);
    assert!(matches!(
        parse_report(&bytes, Some("r.zip")),
        Err(ParseError::NotDmarc)
    ));
    // No entries at all.
    assert!(matches!(
        parse_report(&zip(&[]), Some("r.zip")),
        Err(ParseError::EmptyZip)
    ));
    // Twenty thousand empty entries before the report.
    let names: Vec<String> = (0..20_000).map(|i| format!("{i}.txt")).collect();
    let mut entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
    entries.push(("report.xml", xml.as_bytes()));
    let bytes = zip(&entries);
    assert_eq!(
        parse_report(&bytes, Some("r.zip")).unwrap().records.len(),
        2
    );
}
