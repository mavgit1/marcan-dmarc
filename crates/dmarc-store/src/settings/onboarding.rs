//! How the setup ended.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How the setup ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum OnboardingState {
    Completed,
    Skipped,
}

/// The setup's end, as stored and as `PUT /api/settings/onboarding` takes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OnboardingSettings {
    /// `null` until the setup is finished or skipped; saving `null` opens it again.
    pub state: Option<OnboardingState>,
}

impl SettingsGroup for OnboardingSettings {
    const KEY: &'static str = "onboarding";
}
