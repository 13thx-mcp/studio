use std::sync::Arc;

use crate::{
    config::{RestartAutomationConfig, RestartAutomationPolicy},
    update::RuntimeOperationCoordinator,
};

#[derive(Clone)]
pub struct RestartOwnerContext {
    pub policy: RestartPolicy,
    pub runtime_operations: Arc<RuntimeOperationCoordinator>,
}

impl RestartOwnerContext {
    pub fn new(
        policy: RestartPolicy,
        runtime_operations: Arc<RuntimeOperationCoordinator>,
    ) -> Self {
        Self {
            policy,
            runtime_operations,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    pub enabled: bool,
    pub max_attempts: u32,
    pub stability_window_ms: u64,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub cooldown_ms: u64,
}

impl RestartPolicy {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            max_attempts: 1,
            stability_window_ms: 30_000,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 30_000,
            cooldown_ms: 60_000,
        }
    }

    pub fn for_mcp(config: &RestartAutomationConfig) -> Self {
        Self::from_config(config, config.mcp)
    }

    pub fn for_tunnel(config: &RestartAutomationConfig) -> Self {
        Self::from_config(config, config.tunnel)
    }

    fn from_config(config: &RestartAutomationConfig, mode: RestartAutomationPolicy) -> Self {
        Self {
            enabled: mode == RestartAutomationPolicy::OnFailure,
            max_attempts: config.max_attempts,
            stability_window_ms: config.stability_window_seconds.saturating_mul(1000),
            initial_backoff_ms: config.initial_backoff_seconds.saturating_mul(1000),
            max_backoff_ms: config.max_backoff_seconds.saturating_mul(1000),
            cooldown_ms: config.circuit_cooldown_seconds.saturating_mul(1000),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPhase {
    Healthy,
    BackingOff,
    CircuitOpen,
}

impl RestartPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::BackingOff => "backing_off",
            Self::CircuitOpen => "circuit_open",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartEpisode {
    pub phase: RestartPhase,
    pub consecutive_failures: u32,
    pub retry_at_ms: Option<u64>,
}

impl Default for RestartEpisode {
    fn default() -> Self {
        Self {
            phase: RestartPhase::Healthy,
            consecutive_failures: 0,
            retry_at_ms: None,
        }
    }
}

impl RestartEpisode {
    pub fn on_started(&mut self) {
        self.phase = RestartPhase::Healthy;
        self.retry_at_ms = None;
    }

    pub fn record_failure(
        &mut self,
        now_ms: u64,
        uptime_ms: Option<u64>,
        policy: RestartPolicy,
    ) -> Option<u64> {
        if !policy.enabled {
            self.phase = RestartPhase::Healthy;
            self.retry_at_ms = None;
            return None;
        }
        if uptime_ms.is_some_and(|uptime| uptime >= policy.stability_window_ms) {
            self.consecutive_failures = 0;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);

        let delay = if self.consecutive_failures >= policy.max_attempts {
            self.phase = RestartPhase::CircuitOpen;
            policy.cooldown_ms
        } else {
            self.phase = RestartPhase::BackingOff;
            backoff_delay(self.consecutive_failures, policy)
        };
        let retry = now_ms.saturating_add(delay);
        self.retry_at_ms = Some(retry);
        Some(retry)
    }

    pub fn defer_without_failure(&mut self, now_ms: u64, policy: RestartPolicy) -> u64 {
        let retry = now_ms.saturating_add(policy.initial_backoff_ms.max(1));
        self.retry_at_ms = Some(retry);
        retry
    }

    pub fn ready(&self, now_ms: u64) -> bool {
        self.retry_at_ms.is_some_and(|retry| now_ms >= retry)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

fn backoff_delay(attempt: u32, policy: RestartPolicy) -> u64 {
    let exponent = attempt.saturating_sub(1).min(31);
    policy
        .initial_backoff_ms
        .saturating_mul(1_u64.checked_shl(exponent).unwrap_or(u64::MAX))
        .min(policy.max_backoff_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RestartPolicy {
        RestartPolicy {
            enabled: true,
            max_attempts: 3,
            stability_window_ms: 30_000,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 30_000,
            cooldown_ms: 60_000,
        }
    }

    #[test]
    fn crash_loop_backoff_is_bounded_and_opens_circuit() {
        let mut episode = RestartEpisode::default();
        assert_eq!(episode.record_failure(0, Some(100), policy()), Some(1_000));
        assert_eq!(
            episode.record_failure(1_000, Some(100), policy()),
            Some(3_000)
        );
        assert_eq!(
            episode.record_failure(3_000, Some(100), policy()),
            Some(63_000)
        );
        assert_eq!(episode.phase, RestartPhase::CircuitOpen);
        assert_eq!(episode.consecutive_failures, 3);
    }

    #[test]
    fn stability_window_resets_failure_episode_only_after_sustained_health() {
        let mut episode = RestartEpisode::default();
        episode.record_failure(0, Some(100), policy());
        episode.record_failure(1_000, Some(29_999), policy());
        assert_eq!(episode.consecutive_failures, 2);

        episode.record_failure(3_000, Some(30_000), policy());
        assert_eq!(episode.consecutive_failures, 1);
        assert_eq!(episode.phase, RestartPhase::BackingOff);
    }

    #[test]
    fn coordinator_deferral_does_not_increment_failures() {
        let mut episode = RestartEpisode::default();
        episode.record_failure(0, Some(100), policy());
        let failures = episode.consecutive_failures;
        assert_eq!(episode.defer_without_failure(1_000, policy()), 2_000);
        assert_eq!(episode.consecutive_failures, failures);
    }

    #[test]
    fn disabled_policy_never_schedules_restart() {
        let mut disabled = policy();
        disabled.enabled = false;
        let mut episode = RestartEpisode::default();
        assert_eq!(episode.record_failure(0, Some(1), disabled), None);
        assert_eq!(episode.consecutive_failures, 0);
    }
}
