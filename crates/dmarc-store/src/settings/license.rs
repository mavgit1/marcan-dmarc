//! The operator license key.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};

/// The operator license, as stored. Nothing is judged here: whether the key
/// is valid, and so whether the free-software notice is off, is the server's.
/// The key is a secret: the API never answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseSettings {
    /// The key as entered, trimmed; `None` without one.
    pub operator_key: Option<String>,
}

impl SettingsGroup for LicenseSettings {
    const KEY: &'static str = "license";
}
