//! Raw attachment bytes through `parse_report` (magic-byte detection, gzip,
//! zip, XML), once per filename hint the IMAP worker can pass, then the
//! alignment checks ingest runs on a parsed report.
#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;

const HINTS: [Option<&str>; 5] = [
    Some("report.xml"),
    Some("report.gz"),
    Some("report.zip"),
    Some("report.xml.gz"),
    None,
];

fuzz_target!(|data: &[u8]| {
    for hint in HINTS {
        if let Ok(report) = dmarc_parser::parse_report(data, hint) {
            common::align_all(&report);
        }
    }
});
