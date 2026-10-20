//! Where a domain's DMARC policy comes from: the one rule
//! ([`POLICY_SOURCE_EXPR`]) and the stored DNS check's policy columns the
//! domain list and the overview read through it.

use serde::Serialize;
use sqlx::FromRow;
use ts_rs::TS;

/// Where a domain's DMARC policy comes from, or why it has none. The policy
/// is the DMARC record in DNS, as the stored check found it; never a report's
/// `policy_published`, which is one receiver's view at the time (receivers
/// disagree, one leaves `sp` out where another fills it in, and all of them
/// lag DNS). A report's policy is shown only with that report. Picked by
/// [`POLICY_SOURCE_EXPR`], the one rule for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    /// The DMARC record of the stored DNS check.
    Dns,
    /// The stored DNS check found no DMARC record: no policy.
    NotPublished,
    /// No DNS check is stored yet: no policy known.
    NotChecked,
    /// No DNS check stored, and the server's latest attempt failed. Never
    /// read from the database: the server sets it in place of
    /// [`PolicySource::NotChecked`].
    CheckFailed,
}

impl TryFrom<String> for PolicySource {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        match raw.as_str() {
            "dns" => Ok(Self::Dns),
            "not_published" => Ok(Self::NotPublished),
            "not_checked" => Ok(Self::NotChecked),
            _ => Err(format!("unknown policy source '{raw}'")),
        }
    }
}

/// Where a domain's policy comes from, over its stored DNS check `s`: the
/// DNS record when the check found one, else why there is none. The domain
/// list, its sort and the overview (and so policy readiness) all read the
/// policy through it, so they never disagree. It picks a stored value and
/// judges nothing.
const POLICY_SOURCE_EXPR: &str = "(CASE \
        WHEN s.dmarc_record_count > 0 THEN 'dns' \
        WHEN s.dmarc_record_count = 0 THEN 'not_published' \
        ELSE 'not_checked' END)";

/// `s.policy_<tag> AS policy_<tag>, …, policy_source`: each tag of the stored
/// DNS check's parsed DMARC record (none without one), and [`POLICY_SOURCE_EXPR`].
pub(super) fn applied_policy_columns(tags: &[&str]) -> String {
    let mut columns: Vec<String> = tags
        .iter()
        .map(|tag| format!("s.policy_{tag} AS policy_{tag}"))
        .collect();
    columns.push(format!("{POLICY_SOURCE_EXPR} AS policy_source"));
    columns.join(",\n        ")
}

/// The DMARC policy a domain shows (see [`PolicySource`]): the tags of the
/// stored DNS check's DMARC record. Read from the `policy_<tag>` columns.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct PublishedPolicy {
    #[sqlx(rename = "policy_p")]
    pub p: Option<String>,
    #[sqlx(rename = "policy_sp")]
    pub sp: Option<String>,
    #[sqlx(rename = "policy_pct")]
    pub pct: Option<i64>,
    #[sqlx(rename = "policy_adkim")]
    pub adkim: Option<String>,
    #[sqlx(rename = "policy_aspf")]
    pub aspf: Option<String>,
}

impl PublishedPolicy {
    /// The policy, or `None` when no tag is stored (no record).
    pub(super) fn published(self) -> Option<Self> {
        (self != Self::default()).then_some(self)
    }
}
