//! The XML stage alone (quick-xml, serde and the field checks), without
//! decompression or the `<feedback>` sniff in front of it, then the
//! alignment checks ingest runs on a parsed report. Seeded from
//! `seeds/feedback_xml`.
#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(xml) = std::str::from_utf8(data) {
        if let Ok(report) = dmarc_parser::fuzzing::parse_feedback_xml(xml.to_owned()) {
            common::align_all(&report);
        }
    }
});
