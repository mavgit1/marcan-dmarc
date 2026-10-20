//! Shared DNS lookup outcome types. Lookup failures are data, not errors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Outcome of a single DNS TXT (or similar) query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LookupStatus {
    Ok,
    Nxdomain,
    NoRecords,
    Timeout,
    Servfail,
}

impl LookupStatus {
    /// True when the lookup finished and we observed a real DNS answer.
    ///
    /// `Timeout` and `Servfail` mean we never measured the name - the result is
    /// about our resolver, not the domain. `Nxdomain` / `NoRecords` / `Ok` are
    /// successful measurements of absence or presence.
    pub fn is_measured(self) -> bool {
        !matches!(self, Self::Timeout | Self::Servfail)
    }
}

/// Raw TXT records from one DNS name, plus per-string lengths for size checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtAnswer {
    /// Concatenated character-strings for each TXT RR (RFC 7208 §3.3).
    pub records: Vec<String>,
    /// Length of each character-string inside each RR (parallel to `records`).
    pub string_lengths: Vec<Vec<usize>>,
}

impl TxtAnswer {
    pub fn from_records(records: Vec<String>) -> Self {
        let string_lengths = records.iter().map(|r| vec![r.len()]).collect();
        Self {
            records,
            string_lengths,
        }
    }
}

/// Result of looking up TXT records at a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxtLookup {
    Records(TxtAnswer),
    NxDomain,
    NoRecords,
    Timeout,
    ServFail { detail: String },
}

impl TxtLookup {
    pub fn status(&self) -> LookupStatus {
        match self {
            Self::Records(_) => LookupStatus::Ok,
            Self::NxDomain => LookupStatus::Nxdomain,
            Self::NoRecords => LookupStatus::NoRecords,
            Self::Timeout => LookupStatus::Timeout,
            Self::ServFail { .. } => LookupStatus::Servfail,
        }
    }

    pub fn is_measured(&self) -> bool {
        self.status().is_measured()
    }

    pub fn records(&self) -> &[String] {
        match self {
            Self::Records(a) => &a.records,
            _ => &[],
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::ServFail { detail } => Some(detail),
            _ => None,
        }
    }

    /// TXT RRs whose concatenated value starts with `prefix` (case-insensitive).
    pub fn records_starting_with(&self, prefix: &str) -> Vec<String> {
        self.records()
            .iter()
            .filter(|r| {
                // Byte-wise: a record may hold multi-byte text (or U+FFFD from
                // lossy decoding), so slicing it at `prefix.len()` could panic.
                r.trim()
                    .as_bytes()
                    .get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
            })
            .cloned()
            .collect()
    }
}

/// Split a DNS-style tag list (`v=DMARC1; p=reject; ...`) into
/// `(key, value, has_eq)` triples. Keys are lowercased. Parts without `=` are
/// returned with an empty value and `has_eq == false` so callers can flag
/// syntax errors.
pub fn parse_tag_list(raw: &str) -> Vec<(String, String, bool)> {
    raw.split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            match part.split_once('=') {
                Some((k, v)) => Some((k.trim().to_ascii_lowercase(), v.trim().to_string(), true)),
                None => Some((part.to_ascii_lowercase(), String::new(), false)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TXT data is arbitrary bytes: a record whose byte at the prefix length is
    /// inside a multi-byte character must be skipped, not panic the check.
    #[test]
    fn prefix_match_survives_multibyte_records() {
        let lookup = TxtLookup::Records(TxtAnswer::from_records(vec![
            "🔥🔥🔥".into(),
            String::from_utf8_lossy(b"v=DMAR\xff").into_owned(),
            " V=SPF1 -all ".into(),
        ]));
        assert_eq!(
            lookup.records_starting_with("v=spf1"),
            vec![" V=SPF1 -all "]
        );
        assert!(lookup.records_starting_with("v=dmarc1").is_empty());
    }

    #[test]
    fn is_measured_only_for_finished_lookups() {
        assert!(LookupStatus::Ok.is_measured());
        assert!(LookupStatus::Nxdomain.is_measured());
        assert!(LookupStatus::NoRecords.is_measured());
        assert!(!LookupStatus::Timeout.is_measured());
        assert!(!LookupStatus::Servfail.is_measured());
    }
}
