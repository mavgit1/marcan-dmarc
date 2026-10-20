//! Wire types: what the API answers, built from `dmarc_store` rows and
//! `dmarc_assess` verdicts. Every type here is exported to the UI by ts-rs.
//! A store value the API carries unchanged (`MailCounts`, the mapping and
//! organization types, the settings and filter enums) is not copied here: it
//! derives serde/ts-rs in the store, which judges nothing either way. A type
//! here embeds its store row (`#[serde(flatten)]`) and adds only the verdicts,
//! so a new column is one edit in the store.

pub mod alerts;
pub mod domain;
pub mod readiness;
pub mod report;
pub mod reporting_setup;
pub mod sender;
pub mod settings;
pub mod stats;
pub mod system;

pub use dmarc_store::MailCounts;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A list edited as text, one entry per line: the domain mapping, the sender
/// names, the extra DKIM selectors and the client report recipients. Each
/// has the same three requests: `GET` answers the stored list as this,
/// `POST …/preview` takes a draft and answers what saving it would change and
/// its line errors (writing nothing), and `PUT` takes the draft and saves it,
/// answering the same summary; a draft with a line error is refused whole
/// (`errors` on the error answer) and nothing is written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ListText {
    pub text: String,
}

/// One line of a list edited as text ([`ListText`]) that cannot be saved,
/// and why.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct LineError {
    /// 1-based line number in the submitted text.
    pub line: usize,
    pub message: String,
}
