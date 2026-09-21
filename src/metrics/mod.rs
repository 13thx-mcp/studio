//! Historical metric definitions for M6.
//!
//! These are persisted, replay-safe observations. They never replace the existing
//! live per-process counters exposed by Supervisor/TunnelSupervisor.

use serde::Serialize;

pub const METRIC_DEFINITION_VERSION: i64 = 1;
pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricCode {
    Launches,
    RestartAttempts,
    Crashes,
    ExactSessionDurationMs,
    UpdateChecks,
    UpdatesAvailable,
    PrepareAttempts,
    PrepareFailures,
    InstallAttempts,
    InstallSuccesses,
    InstallFailures,
    InterruptedInstalls,
    VerifiedRollbacks,
    RollbackFailures,
    DriftObservations,
}

impl MetricCode {
    pub const ALL: [Self; 15] = [
        Self::Launches,
        Self::RestartAttempts,
        Self::Crashes,
        Self::ExactSessionDurationMs,
        Self::UpdateChecks,
        Self::UpdatesAvailable,
        Self::PrepareAttempts,
        Self::PrepareFailures,
        Self::InstallAttempts,
        Self::InstallSuccesses,
        Self::InstallFailures,
        Self::InterruptedInstalls,
        Self::VerifiedRollbacks,
        Self::RollbackFailures,
        Self::DriftObservations,
    ];

    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Launches => "launches",
            Self::RestartAttempts => "restart_attempts",
            Self::Crashes => "crashes",
            Self::ExactSessionDurationMs => "exact_session_duration_ms",
            Self::UpdateChecks => "update_checks",
            Self::UpdatesAvailable => "updates_available",
            Self::PrepareAttempts => "prepare_attempts",
            Self::PrepareFailures => "prepare_failures",
            Self::InstallAttempts => "install_attempts",
            Self::InstallSuccesses => "install_successes",
            Self::InstallFailures => "install_failures",
            Self::InterruptedInstalls => "interrupted_installs",
            Self::VerifiedRollbacks => "verified_rollbacks",
            Self::RollbackFailures => "rollback_failures",
            Self::DriftObservations => "drift_observations",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HistoricalMetricTotal {
    pub subject_id: String,
    pub metric: MetricCode,
    pub count: String,
    pub sum: String,
    pub through_seq: String,
    pub coverage_start_ms: i64,
    pub quality: MetricQuality,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HistoricalMetricBucket {
    pub subject_id: String,
    pub metric: MetricCode,
    pub resolution_ms: i64,
    pub bucket_start_ms: i64,
    pub count: String,
    pub sum: String,
    pub min: Option<String>,
    pub max: Option<String>,
    pub through_seq: String,
    pub quality: MetricQuality,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricQuality {
    Complete,
    Partial,
    Unknown,
}

impl MetricQuality {
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "complete" => Some(Self::Complete),
            "partial" => Some(Self::Partial),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}
