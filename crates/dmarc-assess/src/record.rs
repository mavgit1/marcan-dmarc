//! What happened to one record: how DMARC was decided, what the receiver did
//! with the mail, and what each SPF or DKIM check says.
//!
//! The facts are decided here, in [`RecordFacts`]: whether SPF and DKIM
//! passed for an aligned domain ([`CheckState::Pass`]), DMARC's pass over
//! those two ([`RecordFacts::dmarc_pass`]) and [`trusted_forward`]. Ingest
//! computes them once into plain columns on `records` (`spf_aligned_pass`,
//! `dkim_aligned_pass`, `dmarc_pass`, `forwarded`), and a `CHECK` on the table
//! holds `dmarc_pass` to the two aligned passes. Whether the mail was
//! delivered is read from the stored `disposition` by SQL alone. The four-way
//! [`Outcome`] and the [`DmarcVerdict`] over those columns are decided once,
//! as SQL, in `dmarc_store::outcome` (`OUTCOME_EXPR`, `VERDICT_EXPR`): every
//! count, filter, badge and export reads that text, so there is no Rust copy
//! of the rule to keep in step. The enums here name the result.

use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use ts_rs::TS;

/// A stored token that names no variant of the enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownToken(pub String);

impl fmt::Display for UnknownToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown token '{}'", self.0)
    }
}

impl std::error::Error for UnknownToken {}

/// Snake-case token enum: `as_str`, `FromStr` and `TryFrom<String>` (the last
/// is what `sqlx(try_from)` decodes a TEXT column through).
macro_rules! token_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $token:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// The stored and serialized name.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $token,)+
                }
            }
        }

        impl FromStr for $name {
            type Err = UnknownToken;

            fn from_str(raw: &str) -> Result<Self, UnknownToken> {
                match raw {
                    $($token => Ok(Self::$variant),)+
                    other => Err(UnknownToken(other.to_owned())),
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = UnknownToken;

            fn try_from(raw: String) -> Result<Self, UnknownToken> {
                raw.parse()
            }
        }
    };
}

token_enum!(
    /// How DMARC was decided for a record: which mechanism, if any, passed
    /// for a domain aligned with the From domain. Decided by the store's
    /// `VERDICT_EXPR` over `spf_aligned_pass` / `dkim_aligned_pass`.
    DmarcVerdict {
        /// Neither SPF nor DKIM passed for an aligned domain.
        Fail => "fail",
        PassSpf => "pass_spf",
        PassDkim => "pass_dkim",
        PassBoth => "pass_both",
    }
);

token_enum!(
    /// What happened to a record; the same four outcomes behind every count,
    /// filter, badge and export. Decided by the store's `OUTCOME_EXPR`:
    /// passed if `dmarc_pass`, else blocked unless delivered (no disposition
    /// or `none`), else forwarded if `forwarded`, else delivered.
    Outcome {
        /// Failed DMARC and still delivered, not a trusted forward.
        Delivered => "delivered",
        /// Failed DMARC, delivered because the receiver trusted the forwarder.
        Forwarded => "forwarded",
        /// Failed DMARC and was quarantined or rejected.
        Blocked => "blocked",
        Passed => "passed",
    }
);

impl Outcome {
    /// Every outcome, worst first.
    pub const ALL: [Self; 4] = [
        Self::Delivered,
        Self::Forwarded,
        Self::Blocked,
        Self::Passed,
    ];

    /// Its place worst first (0: a delivered failure, 3: passed): the one
    /// order every list sorts outcomes by (the store's `sort=outcome` reads
    /// it through `OUTCOME_RANK_EXPR`, the UI through each record's
    /// `outcome_rank`).
    pub fn rank(self) -> u8 {
        match self {
            Self::Delivered => 0,
            Self::Forwarded => 1,
            Self::Blocked => 2,
            Self::Passed => 3,
        }
    }
}

/// Override reason types that say the receiver trusted a forwarder.
const FORWARD_REASONS: [&str; 3] = ["forwarded", "mailing_list", "trusted_forwarder"];

/// One policy override says the receiver trusted a forwarder: an explicit
/// forwarding reason, or a verified ARC chain (`arc=pass`, which receivers
/// send as a `local_policy` comment).
pub fn trusted_forward(reason_type: Option<&str>, comment: Option<&str>) -> bool {
    let reason = reason_type.map(str::trim).unwrap_or_default();
    FORWARD_REASONS
        .iter()
        .any(|r| reason.eq_ignore_ascii_case(r))
        || comment.is_some_and(|c| c.to_ascii_lowercase().contains("arc=pass"))
}

/// Which checks of a record lack an aligned pass: the `issue` of
/// `auth-issues.csv`. `None` from [`AuthIssue::of`] when both passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthIssue {
    /// DKIM passed for an aligned domain, SPF did not.
    SpfFailed,
    /// SPF passed for an aligned domain, DKIM did not.
    DkimFailed,
    /// Neither passed for an aligned domain.
    BothFailed,
}

impl AuthIssue {
    /// The issue of a record with these aligned passes; `None` when both passed.
    pub fn of(spf_aligned_pass: bool, dkim_aligned_pass: bool) -> Option<Self> {
        match (spf_aligned_pass, dkim_aligned_pass) {
            (true, true) => None,
            (false, true) => Some(Self::SpfFailed),
            (true, false) => Some(Self::DkimFailed),
            (false, false) => Some(Self::BothFailed),
        }
    }

    /// `spf_failed`, `dkim_failed` or `both_failed`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpfFailed => "spf_failed",
            Self::DkimFailed => "dkim_failed",
            Self::BothFailed => "both_failed",
        }
    }
}

/// The facts of one record that `records` stores as columns, decided once at
/// ingest from its checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordFacts {
    /// An SPF check that counts for DMARC passed for an aligned domain.
    pub spf_aligned_pass: bool,
    /// A DKIM signature passed for an aligned domain.
    pub dkim_aligned_pass: bool,
    /// The receiver said it trusted a forwarder ([`trusted_forward`]).
    pub forwarded: bool,
}

impl RecordFacts {
    /// The facts of a record from its checks: each SPF and DKIM check as its
    /// raw result and whether its domain aligns (and, for SPF, counts for
    /// DMARC), and its override reasons as `(reason type, comment)`.
    pub fn of<'a>(
        spf: impl IntoIterator<Item = (&'a str, bool)>,
        dkim: impl IntoIterator<Item = (&'a str, bool)>,
        overrides: impl IntoIterator<Item = (Option<&'a str>, Option<&'a str>)>,
    ) -> Self {
        let aligned_pass = |(result, aligned): (&str, bool)| {
            CheckState::of(Some(result), aligned) == CheckState::Pass
        };
        Self {
            spf_aligned_pass: spf.into_iter().any(aligned_pass),
            dkim_aligned_pass: dkim.into_iter().any(aligned_pass),
            forwarded: overrides
                .into_iter()
                .any(|(reason, comment)| trusted_forward(reason, comment)),
        }
    }

    /// DMARC passed: SPF or DKIM passed for a domain aligned with the From
    /// domain (RFC 7489 §4.2). The one rule behind `records.dmarc_pass`, which
    /// the table's `CHECK` holds to the two aligned columns.
    pub fn dmarc_pass(self) -> bool {
        self.spf_aligned_pass || self.dkim_aligned_pass
    }
}

/// What one SPF or DKIM check of a record says, most relevant first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// Passed for a domain aligned with the From domain: counts for DMARC.
    Pass,
    /// Passed for another domain (a forwarder, a list, a provider's own
    /// domain): does not count for DMARC, and is no failure either.
    PassOther,
    /// `temperror` (RFC 7208 / 8601): the receiver's DNS lookup failed. No
    /// verdict on the sender.
    Temporary,
    /// Any other result: `fail`, `softfail`, `neutral`, `permerror`, ...
    Fail,
    /// No result reported.
    None,
}

impl CheckState {
    /// The state of a check with raw `result` and the alignment computed at ingest.
    pub fn of(result: Option<&str>, aligned: bool) -> Self {
        let raw = result
            .map(|r| r.trim().to_ascii_lowercase())
            .unwrap_or_default();
        match raw.as_str() {
            "pass" if aligned => Self::Pass,
            "pass" => Self::PassOther,
            "" | "none" => Self::None,
            "temperror" => Self::Temporary,
            _ => Self::Fail,
        }
    }

    /// Its colour: a pass for another domain is neither green nor red.
    pub fn status(self) -> Status {
        match self {
            Self::Pass => Status::Pass,
            Self::Temporary => Status::Warn,
            Self::Fail => Status::Fail,
            Self::PassOther | Self::None => Status::Neutral,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        for v in [
            DmarcVerdict::Fail,
            DmarcVerdict::PassSpf,
            DmarcVerdict::PassDkim,
            DmarcVerdict::PassBoth,
        ] {
            assert_eq!(v.as_str().parse::<DmarcVerdict>(), Ok(v));
        }
        for o in [
            Outcome::Delivered,
            Outcome::Forwarded,
            Outcome::Blocked,
            Outcome::Passed,
        ] {
            assert_eq!(Outcome::try_from(o.as_str().to_owned()), Ok(o));
        }
        assert_eq!(
            "nope".parse::<Outcome>(),
            Err(UnknownToken("nope".to_owned()))
        );
    }

    #[test]
    fn trusted_forward_reasons_and_arc() {
        assert!(trusted_forward(Some("forwarded"), None));
        assert!(trusted_forward(Some(" Mailing_List "), None));
        assert!(trusted_forward(Some("trusted_forwarder"), Some("")));
        assert!(trusted_forward(
            Some("local_policy"),
            Some("ARC=Pass as[1]")
        ));
        assert!(!trusted_forward(Some("local_policy"), Some("arc=fail")));
        assert!(!trusted_forward(Some("sampled_out"), None));
        assert!(!trusted_forward(None, None));
    }

    #[test]
    fn auth_issue_names_the_missing_aligned_pass() {
        assert_eq!(AuthIssue::of(true, true), None);
        assert_eq!(AuthIssue::of(false, true), Some(AuthIssue::SpfFailed));
        assert_eq!(AuthIssue::of(true, false), Some(AuthIssue::DkimFailed));
        assert_eq!(AuthIssue::of(false, false), Some(AuthIssue::BothFailed));
        assert_eq!(AuthIssue::BothFailed.as_str(), "both_failed");
    }

    #[test]
    fn facts_from_checks() {
        let none: [(Option<&str>, Option<&str>); 0] = [];
        // An aligned pass counts; a pass for another domain or an aligned
        // failure does not.
        let facts = RecordFacts::of([(" PASS ", true)], [("pass", false), ("fail", true)], none);
        assert_eq!(
            facts,
            RecordFacts {
                spf_aligned_pass: true,
                dkim_aligned_pass: false,
                forwarded: false,
            }
        );
        assert!(facts.dmarc_pass());
        let dkim_only = RecordFacts::of([("softfail", true)], [("pass", true)], none);
        assert!(!dkim_only.spf_aligned_pass && dkim_only.dkim_aligned_pass);
        assert!(dkim_only.dmarc_pass());
        let forwarded = RecordFacts::of(
            [("pass", false)],
            [("temperror", true)],
            [(Some("local_policy"), Some("arc=pass"))],
        );
        assert!(forwarded.forwarded);
        assert!(!forwarded.dmarc_pass());
        // DMARC passes exactly when one of the two aligned passes holds.
        for (spf, dkim) in [(false, false), (true, false), (false, true), (true, true)] {
            let facts = RecordFacts {
                spf_aligned_pass: spf,
                dkim_aligned_pass: dkim,
                forwarded: false,
            };
            assert_eq!(facts.dmarc_pass(), spf || dkim);
        }
    }

    #[test]
    fn check_states() {
        assert_eq!(CheckState::of(Some("pass"), true), CheckState::Pass);
        assert_eq!(CheckState::of(Some(" PASS "), false), CheckState::PassOther);
        assert_eq!(
            CheckState::of(Some("TempError"), true),
            CheckState::Temporary
        );
        assert_eq!(CheckState::of(Some("softfail"), true), CheckState::Fail);
        assert_eq!(CheckState::of(Some("permerror"), false), CheckState::Fail);
        assert_eq!(CheckState::of(Some("none"), true), CheckState::None);
        assert_eq!(CheckState::of(Some(" "), true), CheckState::None);
        assert_eq!(CheckState::of(None, false), CheckState::None);
    }

    #[test]
    fn check_status() {
        assert_eq!(CheckState::Pass.status(), Status::Pass);
        assert_eq!(CheckState::PassOther.status(), Status::Neutral);
        assert_eq!(CheckState::Temporary.status(), Status::Warn);
        assert_eq!(CheckState::Fail.status(), Status::Fail);
        assert_eq!(CheckState::None.status(), Status::Neutral);
        // Most relevant first.
        assert!(CheckState::Pass < CheckState::PassOther);
        assert!(CheckState::Temporary < CheckState::Fail);
        assert!(CheckState::Fail < CheckState::None);
    }
}
