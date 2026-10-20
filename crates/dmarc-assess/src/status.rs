//! The verdict colours, each with one meaning everywhere.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A verdict. `info` (blue) means spoofing was stopped; `forward` (violet)
/// means mail failed in transit and the receiver trusted the forward: neither
/// a warning nor a pass. `neutral` states nothing (no data, not applicable).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Info,
    Forward,
    #[default]
    Neutral,
}

impl Status {
    /// Higher is worse: fail > warn > forward > info > pass > neutral.
    pub fn rank(self) -> u8 {
        match self {
            Self::Fail => 5,
            Self::Warn => 4,
            Self::Forward => 3,
            Self::Info => 2,
            Self::Pass => 1,
            Self::Neutral => 0,
        }
    }

    /// The worse of two verdicts; `self` on a tie.
    pub fn worst(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    /// Worst of `statuses`; `Neutral` when empty.
    pub fn worst_of(statuses: impl IntoIterator<Item = Self>) -> Self {
        statuses.into_iter().fold(Self::Neutral, Self::worst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worst_follows_the_colour_order() {
        assert_eq!(Status::Pass.worst(Status::Info), Status::Info);
        assert_eq!(Status::Forward.worst(Status::Info), Status::Forward);
        assert_eq!(Status::Warn.worst(Status::Fail), Status::Fail);
        assert_eq!(Status::worst_of([]), Status::Neutral);
        assert_eq!(
            Status::worst_of([Status::Pass, Status::Warn, Status::Info]),
            Status::Warn
        );
    }
}
