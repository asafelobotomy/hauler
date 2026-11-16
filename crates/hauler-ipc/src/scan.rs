//! What oysterAV said about a finished download.

use serde::{Deserialize, Serialize};

/// Where a scan of a finished download stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanState {
    /// oysterAV is looking at it (or waiting for its turn).
    Scanning,
    Clean,
    /// oysterAV found something; `detail` names it.
    Threat,
    /// The scan could not run or finish; `detail` says why.
    Error,
}

impl ScanState {
    pub fn as_str(self) -> &'static str {
        match self {
            ScanState::Scanning => "scanning",
            ScanState::Clean => "clean",
            ScanState::Threat => "threat",
            ScanState::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "scanning" => ScanState::Scanning,
            "clean" => ScanState::Clean,
            "threat" => ScanState::Threat,
            "error" => ScanState::Error,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub state: ScanState,
    /// The threat names found, or why the scan failed; empty when clean.
    #[serde(default)]
    pub detail: String,
    /// Unix seconds the scan started (`scanning`) or ended.
    pub at: i64,
}
