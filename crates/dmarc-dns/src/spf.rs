//! SPF record parsing and recursive DNS-lookup counting (RFC 7208).

use crate::lookup::{LookupStatus, TxtAnswer, TxtLookup};
use dmarc_assess::{
    no_issue_above_info, spf_all_status, spf_void_lookups_exceeded, CheckKind, Finding,
    SpfLookupLoad, Status, SPF_LOOKUP_LIMIT, SPF_VOID_LIMIT,
};
use dmarc_domain::normalize_name;
use ipnet::IpNet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use ts_rs::TS;

const SPF_MAX_DEPTH: u32 = 20;
pub(crate) const SPF_MAX_WALK: u32 = 50;

/// Finding code for an include walk with unanswered targets: a failed
/// measurement, not a result.
pub const SPF_WALK_INCOMPLETE: &str = "spf_walk_incomplete";

/// What an SPF term does: an RFC 7208 mechanism, the `redirect=` / `exp=`
/// modifier, or something the RFC does not know (kept by name, lowercased).
///
/// On the wire it is the mechanism name (`"include"`), `"unknown"` for
/// [`Mechanism::Unknown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mechanism {
    All,
    Include,
    A,
    Mx,
    Ptr,
    Ip4,
    Ip6,
    Exists,
    Redirect,
    Exp,
    Unknown(String),
}

impl Mechanism {
    const NAMED: [(&'static str, Self); 10] = [
        ("all", Self::All),
        ("include", Self::Include),
        ("a", Self::A),
        ("mx", Self::Mx),
        ("ptr", Self::Ptr),
        ("ip4", Self::Ip4),
        ("ip6", Self::Ip6),
        ("exists", Self::Exists),
        ("redirect", Self::Redirect),
        ("exp", Self::Exp),
    ];

    /// The mechanism or modifier named `name` (case-insensitive).
    pub fn parse(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        Self::NAMED
            .iter()
            .find(|(n, _)| *n == lower)
            .map(|(_, m)| m.clone())
            .unwrap_or(Self::Unknown(lower))
    }

    /// The name as written in a record; `unknown` for [`Self::Unknown`].
    pub fn as_str(&self) -> &'static str {
        Self::NAMED
            .iter()
            .find(|(_, m)| m == self)
            .map_or("unknown", |(n, _)| n)
    }

    /// `a`, `mx`, `ptr`, `exists`, `include` and `redirect` each cost one of
    /// the ten DNS lookups RFC 7208 §4.6.4 allows.
    pub fn counts_as_lookup(&self) -> bool {
        matches!(
            self,
            Self::A | Self::Mx | Self::Ptr | Self::Exists | Self::Include | Self::Redirect
        )
    }
}

impl Serialize for Mechanism {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Mechanism {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self::parse(&s))
    }
}

/// The qualifier in front of a mechanism: what a match means for the sender.
/// `+` (the default when none is written) passes, `-` fails, `~` soft-fails,
/// `?` is neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qualifier {
    Pass,
    Fail,
    SoftFail,
    Neutral,
}

impl Qualifier {
    const ALL: [Self; 4] = [Self::Pass, Self::Fail, Self::SoftFail, Self::Neutral];

    /// The qualifier written as `symbol`, if it is one.
    pub fn parse(symbol: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.as_str() == symbol)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "+",
            Self::Fail => "-",
            Self::SoftFail => "~",
            Self::Neutral => "?",
        }
    }
}

impl Serialize for Qualifier {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Qualifier {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("not an SPF qualifier: {s:?}")))
    }
}

/// `Option<Qualifier>` on the wire as the bare symbol, empty when absent
/// (modifiers have no qualifier), as the UI and stored snapshots expect.
mod qualifier_or_empty {
    use super::{Deserialize, Deserializer, Qualifier, Serializer};

    // serde's `serialize_with` hands the field over by reference.
    #[allow(clippy::ref_option, clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S: Serializer>(q: &Option<Qualifier>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(q.map_or("", Qualifier::as_str))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Qualifier>, D::Error> {
        let s = String::deserialize(d)?;
        if s.is_empty() {
            return Ok(None);
        }
        Qualifier::parse(&s)
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("not an SPF qualifier: {s:?}")))
    }
}

/// One mechanism or modifier from an SPF record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SpfTerm {
    pub raw: String,
    /// `+`, `-`, `~`, `?`, or empty for modifiers.
    #[serde(with = "qualifier_or_empty")]
    #[ts(type = "string")]
    pub qualifier: Option<Qualifier>,
    /// `all`, `include`, `a`, `mx`, `ptr`, `ip4`, `ip6`, `exists`, `redirect`, `exp`, `unknown`.
    #[ts(type = "string")]
    pub kind: Mechanism,
    pub value: Option<String>,
    pub counts_as_lookup: bool,
}

/// Parsed `v=spf1` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParsedSpf {
    pub version: Option<String>,
    pub terms: Vec<SpfTerm>,
    pub has_all: bool,
    #[ts(type = "string | null")]
    pub all_qualifier: Option<Qualifier>,
    pub redirect: Option<String>,
    pub syntax_errors: Vec<String>,
}

/// Why an include/redirect branch has no complete record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncludeError {
    /// Nested deeper than `SPF_MAX_DEPTH`.
    DepthExceeded,
    /// The path loops back to a domain already being evaluated.
    Loop,
    /// The walk stopped at `SPF_MAX_WALK` lookups.
    WalkCapped,
    /// Never answered (the walk timed out or was capped first).
    NotResolved,
    /// NXDOMAIN or no TXT record: an RFC 7208 void lookup.
    Void,
    Timeout,
    Servfail {
        detail: String,
    },
    /// TXT records exist, but none is `v=spf1`.
    NoSpfRecord,
}

/// Node in the expanded include/redirect tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IncludeNode {
    pub domain: String,
    pub record: Option<String>,
    pub lookups: u32,
    pub children: Vec<Self>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<IncludeError>,
}

/// Full SPF check result for one domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SpfCheckResult {
    pub domain: String,
    pub lookup: LookupStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lookup_detail: Option<String>,
    pub records: Vec<String>,
    pub record_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parsed: Option<ParsedSpf>,
    pub lookup_count: u32,
    pub void_lookup_count: u32,
    pub lookup_limit: u32,
    pub void_limit: u32,
    pub exceeds_lookup_limit: bool,
    #[ts(type = "string | null")]
    pub all_qualifier: Option<Qualifier>,
    /// Verdict on the effective `all` (see [`spf_all_status`]); neutral without a record.
    pub all_status: Status,
    /// A complete measurement (see [`SpfCheckResult::is_complete`]); anything
    /// less is a failed check, never a count.
    pub complete: bool,
    pub include_tree: IncludeNode,
    pub findings: Vec<Finding>,
}

impl SpfCheckResult {
    /// Whether this is a complete measurement: the root TXT lookup answered
    /// and every include/redirect target was resolved. Anything less is a
    /// failed attempt and must not stand in for the domain's SPF state.
    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

/// Parse a single SPF TXT value. Does not produce findings.
pub fn parse_spf_record(raw: &str) -> ParsedSpf {
    let mut parsed = ParsedSpf {
        version: None,
        terms: Vec::new(),
        has_all: false,
        all_qualifier: None,
        redirect: None,
        syntax_errors: Vec::new(),
    };
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.is_empty() {
        parsed.syntax_errors.push("empty SPF record".into());
        return parsed;
    }
    parsed.version = Some(tokens[0].to_string());
    if !tokens[0].eq_ignore_ascii_case("v=spf1") {
        parsed
            .syntax_errors
            .push(format!("SPF version is `{}`, not v=spf1", tokens[0]));
    }
    for tok in &tokens[1..] {
        let term = parse_term(tok, &mut parsed.syntax_errors);
        match term.kind {
            Mechanism::All => {
                parsed.has_all = true;
                parsed.all_qualifier = term.qualifier;
            }
            Mechanism::Redirect => {
                if parsed.redirect.is_some() {
                    parsed
                        .syntax_errors
                        .push("multiple redirect= modifiers".into());
                }
                parsed.redirect.clone_from(&term.value);
            }
            _ => {}
        }
        parsed.terms.push(term);
    }
    parsed
}

fn parse_term(tok: &str, syntax_errors: &mut Vec<String>) -> SpfTerm {
    if let Some((name, value)) = tok.split_once('=') {
        // A modifier: `redirect=` and `exp=` are the ones RFC 7208 defines.
        let kind = match Mechanism::parse(name) {
            m @ (Mechanism::Redirect | Mechanism::Exp) => m,
            _ => {
                syntax_errors.push(format!("unknown modifier `{tok}`"));
                Mechanism::Unknown("unknown".into())
            }
        };
        return SpfTerm {
            raw: tok.to_string(),
            qualifier: None,
            counts_as_lookup: kind.counts_as_lookup(),
            kind,
            value: Some(value.to_string()),
        };
    }

    let (qualifier, body) = match tok.get(..1).and_then(Qualifier::parse) {
        Some(q) => (q, &tok[1..]),
        None => (Qualifier::Pass, tok),
    };
    let (name, value) = match body.split_once(':') {
        Some((k, v)) => (k, Some(v.to_string())),
        None => match body.split_once('/') {
            Some((k, cidr)) => (k, Some(format!("/{cidr}"))),
            None => (body, None),
        },
    };
    let kind = match Mechanism::parse(name) {
        // Modifiers are written with `=`; `redirect:x` is no mechanism.
        Mechanism::Redirect | Mechanism::Exp => Mechanism::Unknown(name.to_ascii_lowercase()),
        m => m,
    };
    let value_missing = value.as_ref().is_none_or(String::is_empty);

    match &kind {
        Mechanism::All => {
            if value.is_some() {
                syntax_errors.push(format!("all does not take an argument (`{tok}`)"));
            }
        }
        Mechanism::Include if value_missing => {
            syntax_errors.push(format!("include requires a domain (`{tok}`)"));
        }
        Mechanism::Exists if value_missing => {
            syntax_errors.push(format!("exists requires a domain (`{tok}`)"));
        }
        Mechanism::Ip4 | Mechanism::Ip6 if value_missing => {
            syntax_errors.push(format!("{} requires an address (`{tok}`)", kind.as_str()));
        }
        Mechanism::Unknown(other) => {
            syntax_errors.push(format!("unknown mechanism `{other}` in `{tok}`"));
        }
        _ => {}
    }

    SpfTerm {
        raw: tok.to_string(),
        qualifier: Some(qualifier),
        counts_as_lookup: kind.counts_as_lookup(),
        kind,
        value,
    }
}

fn nested_map(nested: &[(String, TxtLookup)]) -> HashMap<String, TxtLookup> {
    nested
        .iter()
        .map(|(k, v)| (normalize_name(k), v.clone()))
        .collect()
}

fn spf_records_from(lookup: &TxtLookup) -> Vec<String> {
    lookup.records_starting_with("v=spf1")
}

/// Evaluate SPF for `domain`. `nested` supplies TXT lookups for include/redirect targets.
pub fn evaluate_spf(
    domain: &str,
    lookup: &TxtLookup,
    nested: &[(String, TxtLookup)],
) -> SpfCheckResult {
    let map = nested_map(nested);
    let records = spf_records_from(lookup);
    let mut findings = Vec::new();
    let mut result = SpfCheckResult {
        domain: domain.to_string(),
        lookup: lookup.status(),
        lookup_detail: lookup.detail().map(str::to_string),
        records: records.clone(),
        record_count: records.len(),
        parsed: None,
        lookup_count: 0,
        void_lookup_count: 0,
        lookup_limit: SPF_LOOKUP_LIMIT,
        void_limit: SPF_VOID_LIMIT,
        exceeds_lookup_limit: false,
        all_qualifier: None,
        all_status: Status::Neutral,
        // A measured answer with nothing to walk (no record, or none to
        // follow) is complete; the walk below may still find it is not.
        complete: lookup.is_measured(),
        include_tree: IncludeNode {
            domain: domain.to_string(),
            record: records.first().cloned(),
            lookups: 0,
            children: Vec::new(),
            error: None,
        },
        findings: Vec::new(),
    };

    match lookup {
        TxtLookup::Timeout => {
            findings.push(Finding::notice(
                CheckKind::Spf,
                "spf_timeout",
                format!("DNS lookup for {domain} TXT timed out, so SPF could not be measured."),
                "Retry the check. This finding is about an incomplete lookup, not a confirmed SPF defect.",
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::ServFail { detail } => {
            findings.push(Finding::notice(
                CheckKind::Spf,
                "spf_servfail",
                format!(
                    "DNS lookup for {domain} TXT returned SERVFAIL ({detail}), so SPF could not be measured."
                ),
                "Retry the check. This finding is about an incomplete lookup, not a confirmed SPF defect.",
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::NxDomain | TxtLookup::NoRecords => {
            findings.push(Finding::critical(
                CheckKind::Spf,
                "spf_missing",
                format!(
                    "No SPF TXT record was found at {domain}. Receivers cannot authorize senders, so legitimate mail may fail DMARC alignment and spoofed mail is easier to get through."
                ),
                format!(
                    "Publish a TXT record at {domain} such as `v=spf1 mx a include:<your-mail-host> -all` covering every legitimate sending source."
                ),
            ));
            result.findings = findings;
            return result;
        }
        TxtLookup::Records(_) => {}
    }

    if records.is_empty() {
        findings.push(Finding::critical(
            CheckKind::Spf,
            "spf_missing",
            format!("{domain} has TXT records, but none start with v=spf1."),
            "Add a TXT record whose value begins with `v=spf1`.",
        ));
        result.findings = findings;
        return result;
    }
    if records.len() > 1 {
        findings.push(Finding::critical(
            CheckKind::Spf,
            "spf_multiple_records",
            format!(
                "Found {} SPF TXT records at {domain}. RFC 7208 allows only one; receivers must return permerror, so SPF fails for all mail.",
                records.len()
            ),
            "Delete extra v=spf1 TXT records and keep a single SPF record.",
        ));
    }

    if let TxtLookup::Records(ans) = lookup {
        flag_record_size(domain, ans, &mut findings);
    }

    let parsed = parse_spf_record(&records[0]);
    for err in &parsed.syntax_errors {
        findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_syntax",
            format!("SPF syntax issue on {domain}: {err}."),
            "Correct the mechanism or modifier so the record matches RFC 7208.",
        ));
    }

    let mut state = WalkState {
        lookup_count: 0,
        void_lookup_count: 0,
        visiting: HashSet::new(),
        loop_detected: false,
        unresolved: 0,
        walk_capped: false,
    };
    let tree = walk_record(domain, &records[0], &parsed, &map, &mut state, 0);
    result.lookup_count = state.lookup_count;
    result.void_lookup_count = state.void_lookup_count;
    result.exceeds_lookup_limit =
        SpfLookupLoad::of_count(state.lookup_count) == SpfLookupLoad::OverLimit;
    result.all_qualifier = effective_all(&parsed, &tree);
    result.all_status = spf_all_status(result.all_qualifier.map(Qualifier::as_str));
    // Unanswered targets make the measurement incomplete, unless the walk hit
    // its cap first: that is deterministic (see `emit_walk_incomplete`).
    result.complete = state.walk_capped || state.unresolved == 0;
    result.include_tree = tree;
    result.parsed = Some(parsed.clone());

    emit_policy_findings(domain, &parsed, result.all_qualifier, &state, &mut findings);
    let walk_complete = emit_walk_incomplete(domain, &state, &mut findings);

    // `spf_ok` quotes the lookup count as final, so it needs the full tree.
    if walk_complete && no_issue_above_info(&findings) {
        findings.push(Finding::ok(
            CheckKind::Spf,
            "spf_ok",
            format!(
                "SPF record is valid with {} DNS lookup(s) (limit {SPF_LOOKUP_LIMIT}) and all={}.",
                state.lookup_count,
                result.all_qualifier.map_or("missing", Qualifier::as_str)
            ),
            "Keep the record within 10 DNS lookups and prefer -all once all senders are listed.",
        ));
    }

    result.findings = findings;
    result
}

struct WalkState {
    lookup_count: u32,
    void_lookup_count: u32,
    visiting: HashSet<String>,
    loop_detected: bool,
    /// Include/redirect targets we never got an answer for (walk timed out or
    /// was capped before reaching them, or their lookup timed out / SERVFAILed).
    /// Their subtrees are unknown, so the lookup counts are minimums.
    unresolved: u32,
    /// Evaluation stopped at `SPF_MAX_WALK` or `SPF_MAX_DEPTH` before visiting
    /// every term.
    walk_capped: bool,
}

fn walk_record(
    domain: &str,
    record: &str,
    parsed: &ParsedSpf,
    map: &HashMap<String, TxtLookup>,
    state: &mut WalkState,
    depth: u32,
) -> IncludeNode {
    let key = normalize_name(domain);
    let mut node = IncludeNode {
        domain: key.clone(),
        record: Some(record.to_string()),
        lookups: 0,
        children: Vec::new(),
        error: None,
    };
    if depth > SPF_MAX_DEPTH {
        node.error = Some(IncludeError::DepthExceeded);
        state.walk_capped = true;
        return node;
    }
    if !state.visiting.insert(key.clone()) {
        state.loop_detected = true;
        node.error = Some(IncludeError::Loop);
        return node;
    }

    for term in &parsed.terms {
        // `redirect=` is followed last, and only without an `all` (below).
        if term.kind == Mechanism::Redirect || !term.counts_as_lookup {
            continue;
        }
        if state.lookup_count >= SPF_MAX_WALK {
            node.error = Some(IncludeError::WalkCapped);
            state.walk_capped = true;
            break;
        }
        state.lookup_count += 1;
        node.lookups += 1;
        if term.kind == Mechanism::Include {
            if let Some(target) = term.value.as_deref() {
                node.children
                    .push(walk_include(target, map, state, depth + 1));
            }
        }
    }

    if !parsed.has_all {
        if let Some(target) = parsed.redirect.as_deref() {
            if state.lookup_count < SPF_MAX_WALK {
                state.lookup_count += 1;
                node.lookups += 1;
                node.children
                    .push(walk_include(target, map, state, depth + 1));
            } else {
                state.walk_capped = true;
            }
        }
    }

    state.visiting.remove(&key);
    node
}

fn walk_include(
    target: &str,
    map: &HashMap<String, TxtLookup>,
    state: &mut WalkState,
    depth: u32,
) -> IncludeNode {
    let key = normalize_name(target);
    // A name missing from the map was never resolved (walk timed out or was
    // capped). That is unknown, not a void lookup.
    let Some(lookup) = map.get(&key).cloned() else {
        state.unresolved += 1;
        return IncludeNode {
            domain: key,
            record: None,
            lookups: 0,
            children: Vec::new(),
            error: Some(IncludeError::NotResolved),
        };
    };
    match &lookup {
        TxtLookup::NxDomain | TxtLookup::NoRecords => {
            state.void_lookup_count += 1;
            IncludeNode {
                domain: key,
                record: None,
                lookups: 0,
                children: Vec::new(),
                error: Some(IncludeError::Void),
            }
        }
        TxtLookup::Timeout => {
            state.unresolved += 1;
            IncludeNode {
                domain: key,
                record: None,
                lookups: 0,
                children: Vec::new(),
                error: Some(IncludeError::Timeout),
            }
        }
        TxtLookup::ServFail { detail } => {
            state.unresolved += 1;
            IncludeNode {
                domain: key,
                record: None,
                lookups: 0,
                children: Vec::new(),
                error: Some(IncludeError::Servfail {
                    detail: detail.clone(),
                }),
            }
        }
        TxtLookup::Records(_) => {
            let records = spf_records_from(&lookup);
            if records.is_empty() {
                state.void_lookup_count += 1;
                IncludeNode {
                    domain: key,
                    record: None,
                    lookups: 0,
                    children: Vec::new(),
                    error: Some(IncludeError::NoSpfRecord),
                }
            } else {
                let parsed = parse_spf_record(&records[0]);
                walk_record(&key, &records[0], &parsed, map, state, depth)
            }
        }
    }
}

fn flag_record_size(domain: &str, ans: &TxtAnswer, findings: &mut Vec<Finding>) {
    for (i, lens) in ans.string_lengths.iter().enumerate() {
        if lens.iter().any(|n| *n > 255) {
            findings.push(Finding::warning(
                CheckKind::Spf,
                "spf_string_too_long",
                format!(
                    "SPF TXT string #{i} at {domain} exceeds 255 octets, which DNS does not allow in a single character-string."
                ),
                "Split the TXT RR into multiple character-strings of at most 255 octets each.",
            ));
        }
    }
    if let Some(rec) = ans
        .records
        .iter()
        .find(|r| r.trim().to_ascii_lowercase().starts_with("v=spf1"))
    {
        if rec.len() > 512 {
            findings.push(Finding::warning(
                CheckKind::Spf,
                "spf_record_too_long",
                format!(
                    "SPF record at {domain} is {} octets. Values over 512 often fail on UDP DNS and some receivers." ,
                    rec.len()
                ),
                "Shorten the record by flattening or dropping unused includes so it stays near 255–450 octets.",
            ));
        }
    }
}

/// Flag an include walk that did not finish. Returns true when it did.
///
/// Two different gaps, two codes:
/// - `spf_walk_incomplete`: include/redirect targets went unanswered (timeout,
///   SERVFAIL, walk budget). That is a failed measurement: callers must not
///   treat the result as the domain's SPF state (see
///   [`SpfCheckResult::is_complete`]).
/// - `spf_walk_capped`: the record needs more than [`SPF_MAX_WALK`] lookups
///   (or nests deeper than `SPF_MAX_DEPTH`). That is deterministic, and the
///   record is far over the limit anyway, so the result stands.
///
/// Both are `Notice`s: neither costs points.
fn emit_walk_incomplete(domain: &str, state: &WalkState, findings: &mut Vec<Finding>) -> bool {
    if state.walk_capped {
        findings.push(Finding::notice(
            CheckKind::Spf,
            "spf_walk_capped",
            format!(
                "SPF evaluation for {domain} stopped at the walk limit ({SPF_MAX_WALK} lookups, depth {SPF_MAX_DEPTH}); the record needs at least {} lookups.",
                state.lookup_count
            ),
            "Flatten or remove includes so the record needs at most 10 DNS lookups.",
        ));
        return false;
    }
    if state.unresolved == 0 {
        return true;
    }
    findings.push(Finding::notice(
        CheckKind::Spf,
        SPF_WALK_INCOMPLETE,
        format!(
            "{} SPF include/redirect target(s) for {domain} could not be resolved, so the SPF check is incomplete.",
            state.unresolved
        ),
        "Retry the check. This finding is about an incomplete lookup, not a confirmed SPF defect.",
    ));
    false
}

/// The `all` qualifier that actually ends evaluation: the record's own, or the one
/// reached by following `redirect=` (possibly chained) through the walked tree.
fn effective_all(parsed: &ParsedSpf, tree: &IncludeNode) -> Option<Qualifier> {
    let (mut parsed, mut node) = (parsed.clone(), tree);
    loop {
        if parsed.all_qualifier.is_some() {
            return parsed.all_qualifier;
        }
        // Node domains are normalized the same way, so compare canonical forms.
        let target = normalize_name(parsed.redirect.as_deref()?);
        node = node.children.iter().find(|c| c.domain == target)?;
        parsed = parse_spf_record(node.record.as_deref()?);
    }
}

fn emit_policy_findings(
    domain: &str,
    parsed: &ParsedSpf,
    effective_all: Option<Qualifier>,
    state: &WalkState,
    findings: &mut Vec<Finding>,
) {
    if state.loop_detected {
        findings.push(Finding::critical(
            CheckKind::Spf,
            "spf_include_loop",
            format!("SPF include/redirect graph for {domain} contains a loop. Receivers will hit permerror and SPF will fail."),
            "Remove the cyclic include or redirect so each domain appears only once in the evaluation chain.",
        ));
    }
    match SpfLookupLoad::of_count(state.lookup_count) {
        SpfLookupLoad::OverLimit => findings.push(Finding::critical(
            CheckKind::Spf,
            "spf_lookup_limit",
            format!(
                "SPF requires {} DNS lookups, exceeding the RFC 7208 limit of {SPF_LOOKUP_LIMIT}; receivers return permerror for any mail not matched before the limit is reached, so SPF fails for some or all senders.",
                state.lookup_count
            ),
            format!("Flatten or remove unused include: mechanisms (and avoid nested third-party includes) so evaluation stays at {SPF_LOOKUP_LIMIT} lookups or fewer."),
        )),
        SpfLookupLoad::NearLimit => {
            let spare = SPF_LOOKUP_LIMIT.saturating_sub(state.lookup_count);
            findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_lookup_high",
            format!(
                "SPF for {domain} uses {} of the {SPF_LOOKUP_LIMIT} DNS lookups RFC 7208 allows, leaving {spare} spare. Adding one include can cost more than one lookup, because the mechanisms inside the included record count toward your total; once evaluation passes {SPF_LOOKUP_LIMIT}, receivers return permerror and SPF fails for every message.",
                state.lookup_count
            ),
            "Check which include contributes the most lookups and flatten it to ip4/ip6 ranges, or drop senders you no longer use.",
        ));
        }
        SpfLookupLoad::Comfortable => {}
    }
    if spf_void_lookups_exceeded(state.void_lookup_count) {
        findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_void_limit",
            format!(
                "SPF evaluation hits {} void lookups (NXDOMAIN/empty), above the recommended limit of {SPF_VOID_LIMIT}. Some receivers treat this as permerror.",
                state.void_lookup_count
            ),
            "Remove include/a/mx/exists terms that point at names with no DNS data.",
        ));
    }

    // Judge the qualifier that actually applies, including one inherited via redirect=.
    let via = match (&parsed.all_qualifier, &parsed.redirect) {
        (None, Some(target)) => format!(" (inherited via redirect={target})"),
        _ => String::new(),
    };
    match effective_all {
        Some(Qualifier::Pass) => findings.push(Finding::critical(
            CheckKind::Spf,
            "spf_plus_all",
            format!("SPF on {domain} ends with +all{via}, which authorizes every host on the internet to send mail as this domain."),
            "Replace +all with -all (fail) after listing legitimate senders. +all makes SPF useless.",
        )),
        Some(Qualifier::SoftFail) => findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_softfail_all",
            format!("SPF on {domain} uses ~all (softfail){via}. Receivers typically accept the mail and only mark it; -all is stronger and expected with an enforced DMARC policy."),
            "Switch ~all to -all once every legitimate sending service is listed in the SPF record.",
        )),
        Some(Qualifier::Neutral) => findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_neutral_all",
            format!("SPF on {domain} uses ?all (neutral){via}, which makes no assertion about unauthorized senders."),
            "Use -all so unauthorized hosts fail SPF.",
        )),
        // Ok, not Info: -all is the outcome every other branch of this match is
        // telling the operator to reach. Deducting for it would penalise the
        // strongest possible SPF terminator.
        Some(Qualifier::Fail) => findings.push(Finding::ok(
            CheckKind::Spf,
            "spf_fail_all",
            format!("SPF on {domain} uses -all{via}, the strongest terminator: hosts not listed fail SPF."),
            "Keep -all; only add new mechanisms when you onboard a sending service.",
        )),
        None if parsed.redirect.is_none() => findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_missing_all",
            format!("SPF on {domain} has no all mechanism and no redirect=. Default is ?all (neutral), so unauthorized senders are not failed."),
            "End the record with `-all` (or `~all` during a cautious rollout).",
        )),
        None => {}
    }

    if parsed.has_all && parsed.redirect.is_some() {
        // Redirect is dead when all is present; SPF evaluation is unchanged.
        findings.push(Finding::notice(
            CheckKind::Spf,
            "spf_redirect_ignored",
            format!("SPF on {domain} has both all and redirect=; RFC 7208 never evaluates redirect when a record matches, and all always matches, so redirect is dead."),
            "Remove redirect= or remove all, not both.",
        ));
    }

    if parsed.terms.iter().any(|t| t.kind == Mechanism::Ptr) {
        findings.push(Finding::warning(
            CheckKind::Spf,
            "spf_ptr_deprecated",
            format!("SPF on {domain} uses the ptr mechanism, which RFC 7208 marks as not recommended (slow, unreliable, extra DNS)."),
            "Replace ptr with ip4/ip6, a, or include: terms for the actual sending hosts.",
        ));
    }
}

/// Where a domain's SPF lists an IP outright.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SpfIpMatch {
    /// The domain whose record holds the term: the checked domain itself or
    /// one of its includes / redirect targets.
    pub domain: String,
    /// The matching `ip4:` / `ip6:` term as published.
    pub term: String,
}

/// The first `+ip4:` / `+ip6:` term in `tree` (depth first, record order)
/// whose network contains `ip`.
///
/// Only literal networks are matched: `a`, `mx`, `exists` and macros are not
/// resolved, so `None` means "not listed by address", never "not authorized".
pub fn spf_ip_match(tree: &IncludeNode, ip: IpAddr) -> Option<SpfIpMatch> {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    };
    let listed = tree.record.as_deref().and_then(|record| {
        parse_spf_record(record).terms.into_iter().find(|term| {
            matches!(term.kind, Mechanism::Ip4 | Mechanism::Ip6)
                && term.qualifier == Some(Qualifier::Pass)
                && term
                    .value
                    .as_deref()
                    .and_then(parse_network)
                    .is_some_and(|net| net.contains(&ip))
        })
    });
    if let Some(term) = listed {
        return Some(SpfIpMatch {
            domain: tree.domain.clone(),
            term: term.raw,
        });
    }
    tree.children
        .iter()
        .find_map(|child| spf_ip_match(child, ip))
}

/// `192.0.2.0/24`, `2001:db8::/32` or a bare address (a host route).
fn parse_network(value: &str) -> Option<IpNet> {
    value
        .parse::<IpNet>()
        .ok()
        .or_else(|| value.parse::<IpAddr>().ok().map(IpNet::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_assess::Severity;

    fn rec(s: &str) -> TxtLookup {
        TxtLookup::Records(TxtAnswer::from_records(vec![s.to_string()]))
    }

    fn nested(pairs: &[(&str, &str)]) -> Vec<(String, TxtLookup)> {
        pairs
            .iter()
            .map(|(d, r)| ((*d).to_string(), rec(r)))
            .collect()
    }

    #[test]
    fn parses_mechanisms_and_qualifiers() {
        let p = parse_spf_record(
            "v=spf1 +a mx include:_spf.google.com ip4:192.0.2.0/24 ip6:2001:db8::/32 exists:%{i}.list -all",
        );
        assert_eq!(p.version.as_deref(), Some("v=spf1"));
        assert!(p.has_all);
        assert_eq!(p.all_qualifier, Some(Qualifier::Fail));
        let kinds: Vec<_> = p.terms.iter().map(|t| t.kind.as_str()).collect();
        assert_eq!(kinds, ["a", "mx", "include", "ip4", "ip6", "exists", "all"]);
        assert_eq!(p.terms[2].value.as_deref(), Some("_spf.google.com"));
        assert!(p.terms[2].counts_as_lookup);
        assert!(!p.terms[3].counts_as_lookup);
        assert_eq!(p.terms[6].qualifier, Some(Qualifier::Fail));
    }

    /// Terms keep their stored and API shape: the qualifier symbol (empty for
    /// a modifier) and the mechanism name as strings.
    #[test]
    fn terms_serialize_as_strings() {
        let p = parse_spf_record("v=spf1 ~mx redirect=_spf.example.net foo=bar");
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["all_qualifier"], serde_json::Value::Null);
        assert_eq!(json["terms"][0]["qualifier"], "~");
        assert_eq!(json["terms"][0]["kind"], "mx");
        assert_eq!(json["terms"][1]["qualifier"], "");
        assert_eq!(json["terms"][1]["kind"], "redirect");
        assert_eq!(json["terms"][2]["kind"], "unknown");
        let back: ParsedSpf = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);

        let all = parse_spf_record("v=spf1 -all");
        assert_eq!(serde_json::to_value(&all).unwrap()["all_qualifier"], "-");
    }

    #[test]
    fn missing_and_multiple_records() {
        let miss = evaluate_spf("example.com", &TxtLookup::NxDomain, &[]);
        assert!(miss
            .findings
            .iter()
            .any(|f| f.code == "spf_missing" && f.severity == Severity::Critical));
        let multi = evaluate_spf(
            "example.com",
            &TxtLookup::Records(TxtAnswer::from_records(vec![
                "v=spf1 -all".into(),
                "v=spf1 +all".into(),
            ])),
            &[],
        );
        assert!(multi
            .findings
            .iter()
            .any(|f| f.code == "spf_multiple_records"));
        assert_eq!(multi.record_count, 2);
    }

    #[test]
    fn plus_all_missing_all_and_ptr() {
        let plus = evaluate_spf("example.com", &rec("v=spf1 ip4:192.0.2.1 +all"), &[]);
        assert!(plus.findings.iter().any(|f| f.code == "spf_plus_all"));
        let none = evaluate_spf("example.com", &rec("v=spf1 ip4:192.0.2.1"), &[]);
        assert!(none.findings.iter().any(|f| f.code == "spf_missing_all"));
        let ptr = evaluate_spf("example.com", &rec("v=spf1 ptr -all"), &[]);
        assert!(ptr.findings.iter().any(|f| f.code == "spf_ptr_deprecated"));
        assert_eq!(ptr.lookup_count, 1);
        let tilde = evaluate_spf("example.com", &rec("v=spf1 ~all"), &[]);
        assert!(tilde.findings.iter().any(|f| f.code == "spf_softfail_all"));
    }

    /// `-all` is what every other terminator branch tells the operator to reach,
    /// so reaching it must be free.
    #[test]
    fn fail_all_is_ok_and_scores_a_hundred() {
        let r = evaluate_spf("example.com", &rec("v=spf1 ip4:192.0.2.1 -all"), &[]);
        let fail_all = r
            .findings
            .iter()
            .find(|f| f.code == "spf_fail_all")
            .expect("spf_fail_all finding");
        assert_eq!(fail_all.severity, Severity::Ok);
        assert_eq!(dmarc_assess::score_from_findings(&r.findings), 100);
    }

    #[test]
    fn nested_includes_count_lookups() {
        let nested_recs = nested(&[
            ("a.test", "v=spf1 include:c.test -all"),
            ("b.test", "v=spf1 a -all"),
            ("c.test", "v=spf1 mx -all"),
        ]);
        let r = evaluate_spf(
            "example.com",
            &rec("v=spf1 include:a.test include:b.test -all"),
            &nested_recs,
        );
        // include a, include b, include c, a, mx = 5
        assert_eq!(r.lookup_count, 5);
        assert!(!r.exceeds_lookup_limit);
        assert_eq!(r.include_tree.children.len(), 2);
        assert_eq!(r.include_tree.children[0].domain, "a.test");
        assert_eq!(r.include_tree.children[0].children[0].domain, "c.test");
        assert!(r.findings.iter().any(|f| f.code == "spf_fail_all"));
    }

    #[test]
    fn redirect_counts_and_is_ignored_when_all_present() {
        let nested_recs = nested(&[("other.test", "v=spf1 a mx -all")]);
        let with_all = evaluate_spf(
            "example.com",
            &rec("v=spf1 ip4:192.0.2.1 -all redirect=other.test"),
            &nested_recs,
        );
        assert_eq!(with_all.lookup_count, 0);
        assert!(with_all
            .findings
            .iter()
            .any(|f| f.code == "spf_redirect_ignored"));

        let only_redirect = evaluate_spf(
            "example.com",
            &rec("v=spf1 redirect=other.test"),
            &nested_recs,
        );
        assert_eq!(only_redirect.lookup_count, 3); // redirect + a + mx
        assert_eq!(only_redirect.include_tree.children[0].domain, "other.test");
        assert_eq!(only_redirect.all_qualifier, Some(Qualifier::Fail));
    }

    /// The inherited qualifier is what receivers apply, so it is what gets judged,
    /// across chained redirects too.
    #[test]
    fn redirect_inherited_all_is_judged() {
        let neutral = evaluate_spf(
            "example.com",
            &rec("v=spf1 redirect=spf.example.net"),
            &nested(&[("spf.example.net", "v=spf1 a ?all")]),
        );
        assert_eq!(neutral.all_qualifier, Some(Qualifier::Neutral));
        let f = neutral
            .findings
            .iter()
            .find(|f| f.code == "spf_neutral_all")
            .expect("inherited ?all is flagged");
        assert!(f.message.contains("redirect=spf.example.net"));

        let chained = evaluate_spf(
            "example.com",
            &rec("v=spf1 redirect=a.example.net"),
            &nested(&[
                ("a.example.net", "v=spf1 redirect=b.example.net"),
                ("b.example.net", "v=spf1 ip4:192.0.2.1 ~all"),
            ]),
        );
        assert_eq!(chained.all_qualifier, Some(Qualifier::SoftFail));
        assert!(chained
            .findings
            .iter()
            .any(|f| f.code == "spf_softfail_all"));
    }

    #[test]
    fn over_limit_and_void_lookups() {
        let terms: String = (1..=11)
            .map(|i| format!("include:n{i}.test"))
            .collect::<Vec<_>>()
            .join(" ");
        let recs: Vec<(String, TxtLookup)> = (1..=11)
            .map(|i| (format!("n{i}.test"), rec("v=spf1 -all")))
            .collect();
        let r = evaluate_spf("example.com", &rec(&format!("v=spf1 {terms} -all")), &recs);
        assert_eq!(r.lookup_count, 11);
        assert!(r.exceeds_lookup_limit);
        assert!(r.findings.iter().any(|f| f.code == "spf_lookup_limit"));

        let root =
            rec("v=spf1 include:missing1.test include:missing2.test include:missing3.test -all");
        let nx: Vec<(String, TxtLookup)> = (1..=3)
            .map(|i| (format!("missing{i}.test"), TxtLookup::NxDomain))
            .collect();
        let voids = evaluate_spf("example.com", &root, &nx);
        assert_eq!(voids.void_lookup_count, 3);
        assert!(voids.findings.iter().any(|f| f.code == "spf_void_limit"));

        // Includes that were never resolved (walk timed out) are not void.
        let unresolved = evaluate_spf("example.com", &root, &[]);
        assert_eq!(unresolved.void_lookup_count, 0);
        assert!(!unresolved
            .findings
            .iter()
            .any(|f| f.code == "spf_void_limit"));
    }

    #[test]
    fn unresolved_includes_make_the_walk_incomplete() {
        let root = rec("v=spf1 include:a.test include:b.test include:c.test -all");
        // b.test resolved, a.test timed out, c.test never reached.
        let partial = vec![
            ("a.test".to_string(), TxtLookup::Timeout),
            ("b.test".to_string(), rec("v=spf1 mx -all")),
        ];
        let r = evaluate_spf("example.com", &root, &partial);
        assert_eq!(r.lookup_count, 4); // 3 includes + mx; a/c subtrees unknown
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "spf_walk_incomplete")
            .expect("incomplete-walk finding");
        assert_eq!(f.severity, Severity::Notice);
        assert!(f.message.starts_with("2 SPF include/redirect target(s)"));
        assert!(!r.is_complete());
        assert!(!r.findings.iter().any(|f| f.code == "spf_ok"));
        // Our measurement gap must not cost the domain points.
        assert_eq!(dmarc_assess::score_from_findings(&r.findings), 100);

        let complete = evaluate_spf(
            "example.com",
            &root,
            &nested(&[
                ("a.test", "v=spf1 -all"),
                ("b.test", "v=spf1 mx -all"),
                ("c.test", "v=spf1 -all"),
            ]),
        );
        assert!(!complete
            .findings
            .iter()
            .any(|f| f.code == "spf_walk_incomplete"));
        assert!(complete.findings.iter().any(|f| f.code == "spf_ok"));
        assert!(complete.is_complete());
    }

    #[test]
    fn capped_walk_is_flagged_but_still_a_complete_measurement() {
        let terms: String = (0..60)
            .map(|i| format!("include:n{i}.test"))
            .collect::<Vec<_>>()
            .join(" ");
        let recs: Vec<(String, TxtLookup)> = (0..60)
            .map(|i| (format!("n{i}.test"), rec("v=spf1 -all")))
            .collect();
        let r = evaluate_spf("example.com", &rec(&format!("v=spf1 {terms} -all")), &recs);
        assert_eq!(r.lookup_count, SPF_MAX_WALK);
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "spf_walk_capped" && f.message.contains("walk limit")));
        assert!(r.exceeds_lookup_limit);
        assert!(r.is_complete());
    }

    #[test]
    fn include_loop_is_flagged() {
        let nested_recs = nested(&[
            ("a.test", "v=spf1 include:b.test -all"),
            ("b.test", "v=spf1 include:a.test -all"),
        ]);
        let r = evaluate_spf(
            "example.com",
            &rec("v=spf1 include:a.test -all"),
            &nested_recs,
        );
        assert!(r.findings.iter().any(|f| f.code == "spf_include_loop"));
        fn has_loop_error(n: &IncludeNode) -> bool {
            n.error == Some(IncludeError::Loop) || n.children.iter().any(has_loop_error)
        }
        assert!(has_loop_error(&r.include_tree));
    }

    #[test]
    fn eight_lookups_is_a_warning() {
        let terms: String = (1..=8)
            .map(|i| format!("include:n{i}.test"))
            .collect::<Vec<_>>()
            .join(" ");
        let recs: Vec<(String, TxtLookup)> = (1..=8)
            .map(|i| (format!("n{i}.test"), rec("v=spf1 -all")))
            .collect();
        let r = evaluate_spf("example.com", &rec(&format!("v=spf1 {terms} -all")), &recs);
        assert_eq!(r.lookup_count, 8);
        assert!(!r.exceeds_lookup_limit);
        assert!(r.findings.iter().any(|f| f.code == "spf_lookup_high"));
    }

    #[test]
    fn timeout_is_notice_and_unmeasured() {
        let r = evaluate_spf("example.com", &TxtLookup::Timeout, &[]);
        assert!(!r.lookup.is_measured());
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "spf_timeout")
            .expect("timeout finding");
        assert_eq!(f.severity, Severity::Notice);
    }

    #[test]
    fn servfail_is_notice_and_unmeasured() {
        let r = evaluate_spf(
            "example.com",
            &TxtLookup::ServFail {
                detail: "refused".into(),
            },
            &[],
        );
        assert!(!r.lookup.is_measured());
        let f = r
            .findings
            .iter()
            .find(|f| f.code == "spf_servfail")
            .expect("servfail finding");
        assert_eq!(f.severity, Severity::Notice);
    }

    fn node(domain: &str, record: &str, children: Vec<IncludeNode>) -> IncludeNode {
        IncludeNode {
            domain: domain.into(),
            record: Some(record.into()),
            lookups: 0,
            children,
            error: None,
        }
    }

    #[test]
    fn ip_match_names_the_listing_include() {
        let tree = node(
            "example.com",
            "v=spf1 ip4:192.0.2.10 include:_spf.vendor.example -all",
            vec![node(
                "_spf.vendor.example",
                "v=spf1 -ip4:198.51.100.0/24 ip4:198.51.100.0/25 ip6:2001:db8::/32 ~all",
                vec![],
            )],
        );
        let hit = |ip: &str| spf_ip_match(&tree, ip.parse().unwrap());
        assert_eq!(
            hit("192.0.2.10"),
            Some(SpfIpMatch {
                domain: "example.com".into(),
                term: "ip4:192.0.2.10".into()
            })
        );
        assert_eq!(
            hit("198.51.100.7"),
            Some(SpfIpMatch {
                domain: "_spf.vendor.example".into(),
                term: "ip4:198.51.100.0/25".into()
            })
        );
        assert_eq!(
            hit("::ffff:198.51.100.7").map(|m| m.domain),
            Some("_spf.vendor.example".into())
        );
        assert_eq!(
            hit("2001:db8::25").map(|m| m.term),
            Some("ip6:2001:db8::/32".into())
        );
        // Only the -ip4 covers it: listed as denied, not authorized.
        assert_eq!(hit("198.51.100.200"), None);
        assert_eq!(hit("203.0.113.1"), None);
    }
}
