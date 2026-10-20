//! Detect and decompress gzip / zip / plain XML report payloads.

use crate::ParseError;
use flate2::read::GzDecoder;
use std::io::Read;
use zip::ZipArchive;

enum PayloadKind {
    Gzip,
    Zip,
    Xml,
}

/// Return decompressed XML bytes from a raw payload.
pub(crate) fn to_xml_bytes(data: &[u8], hint: Option<&str>) -> Result<Vec<u8>, ParseError> {
    match detect_kind(data, hint) {
        PayloadKind::Gzip => gunzip(data),
        PayloadKind::Zip => unzip_first_xml(data),
        PayloadKind::Xml => Ok(data.to_vec()),
    }
}

fn detect_kind(data: &[u8], hint: Option<&str>) -> PayloadKind {
    if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        return PayloadKind::Gzip;
    }
    if data.len() >= 2 && data[0] == b'P' && data[1] == b'K' {
        return PayloadKind::Zip;
    }

    let hint = hint.unwrap_or("").to_ascii_lowercase();
    if hint.contains(".gz") || hint.contains("gzip") {
        return PayloadKind::Gzip;
    }
    if hint.contains(".zip") || hint.contains("application/zip") {
        return PayloadKind::Zip;
    }

    PayloadKind::Xml
}

const MAX_DECOMPRESSED_BYTES: u64 = 50 * 1024 * 1024; // 50 MB

fn gunzip(data: &[u8]) -> Result<Vec<u8>, ParseError> {
    let decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .take(MAX_DECOMPRESSED_BYTES)
        .read_to_end(&mut out)
        .map_err(|e| ParseError::Decompress(e.to_string()))?;
    if out.len() as u64 >= MAX_DECOMPRESSED_BYTES {
        return Err(ParseError::Decompress(
            "decompressed payload exceeds 50 MB limit".into(),
        ));
    }
    Ok(out)
}

fn unzip_first_xml(data: &[u8]) -> Result<Vec<u8>, ParseError> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = ZipArchive::new(cursor).map_err(|e| ParseError::Decompress(e.to_string()))?;

    let mut chosen: Option<usize> = None;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| ParseError::Decompress(e.to_string()))?;
        // A name the archive cannot decode is no `.xml` name; the entry still counts as a file.
        let name = file
            .name()
            .map(|n| n.to_ascii_lowercase())
            .unwrap_or_default();
        if file.is_file() && (name.ends_with(".xml") || name.ends_with(".xml.gz")) {
            chosen = Some(i);
            break;
        }
    }

    if chosen.is_none() {
        for i in 0..archive.len() {
            let file = archive
                .by_index(i)
                .map_err(|e| ParseError::Decompress(e.to_string()))?;
            if file.is_file() {
                chosen = Some(i);
                break;
            }
        }
    }

    let idx = chosen.ok_or(ParseError::EmptyZip)?;
    let mut file = archive
        .by_index(idx)
        .map_err(|e| ParseError::Decompress(e.to_string()))?;
    let name = file
        .name()
        .map(|n| n.to_ascii_lowercase())
        .unwrap_or_default();
    let mut buf = Vec::new();
    file.by_ref()
        .take(MAX_DECOMPRESSED_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| ParseError::Decompress(e.to_string()))?;
    if buf.len() as u64 >= MAX_DECOMPRESSED_BYTES {
        return Err(ParseError::Decompress(
            "decompressed payload exceeds 50 MB limit".into(),
        ));
    }

    if name.ends_with(".gz") || (buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b) {
        return gunzip(&buf);
    }
    Ok(buf)
}
