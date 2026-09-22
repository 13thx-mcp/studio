use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::{
    supervisor::{ProcessState, Supervisor},
    tunnel::{TunnelHealthProbe, TunnelState, TunnelSupervisor},
    update::{ComponentId, GatewayControlClient, InventoryHealth, InventoryService},
};

pub const DEFAULT_HEALTH_FRESHNESS_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Passed,
    Failed,
    Unknown,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthOwnership {
    StudioProcess,
    StudioSupervisor,
    TunnelSupervisor,
    Gateway,
    FleetBundle,
    RuntimeArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthSnapshot {
    pub component: ComponentId,
    pub observed_at_ms: u64,
    pub freshness_ms: u64,
    pub state: HealthState,
    pub liveness: ProbeState,
    pub readiness: ProbeState,
    pub identity: Option<String>,
    pub ownership: HealthOwnership,
    pub required_checks: Vec<String>,
    pub failed_checks: Vec<String>,
}

impl HealthSnapshot {
    pub fn is_fresh_at(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.observed_at_ms) <= self.freshness_ms
    }

    pub fn supports_automatic_mutation(&self, now_ms: u64) -> bool {
        self.is_fresh_at(now_ms) && self.state == HealthState::Healthy
    }
}

#[derive(Clone)]
pub struct HealthService {
    supervisor: Arc<Supervisor>,
    tunnel: Arc<TunnelSupervisor>,
    inventory: Arc<InventoryService>,
    gateway: GatewayControlClient,
}

impl HealthService {
    pub fn new(
        supervisor: Arc<Supervisor>,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        gateway: GatewayControlClient,
    ) -> Self {
        Self {
            supervisor,
            tunnel,
            inventory,
            gateway,
        }
    }

    pub async fn snapshot(&self, component: ComponentId) -> HealthSnapshot {
        match component {
            ComponentId::Gateway => self.gateway_snapshot().await,
            ComponentId::Tunnel => self.tunnel_snapshot().await,
            ComponentId::Filesystem
            | ComponentId::Git
            | ComponentId::Exec
            | ComponentId::Blender => self.mcp_snapshot(component).await,
            ComponentId::Studio => {
                self.inventory_snapshot(component, HealthOwnership::StudioProcess)
                    .await
            }
            ComponentId::Fleet => {
                self.inventory_snapshot(component, HealthOwnership::FleetBundle)
                    .await
            }
        }
    }

    async fn gateway_snapshot(&self) -> HealthSnapshot {
        let observed_at_ms = now_ms();
        match self.gateway.status().await {
            Ok(status) => {
                let child_capability = status.supports_m8_child_status();
                let enabled_children_healthy = child_capability
                    && status
                        .children
                        .iter()
                        .filter(|child| child.enabled)
                        .all(|child| child.running && child.recovery_state == "HEALTHY");
                let running_state = matches!(status.state.as_str(), "RUNNING" | "DRAINED");
                let mut failed = Vec::new();
                if !running_state {
                    failed.push("gateway_state".into());
                }
                if !child_capability {
                    failed.push("m8_child_status_capability".into());
                }
                if child_capability && !enabled_children_healthy {
                    failed.push("gateway_children".into());
                }
                let state = if !child_capability {
                    HealthState::Unknown
                } else if !running_state {
                    HealthState::Unhealthy
                } else if enabled_children_healthy {
                    HealthState::Healthy
                } else {
                    HealthState::Degraded
                };
                HealthSnapshot {
                    component: ComponentId::Gateway,
                    observed_at_ms,
                    freshness_ms: DEFAULT_HEALTH_FRESHNESS_MS,
                    state,
                    liveness: ProbeState::Passed,
                    readiness: if child_capability && running_state {
                        ProbeState::Passed
                    } else if !child_capability {
                        ProbeState::Unknown
                    } else {
                        ProbeState::Failed
                    },
                    identity: status.instance_id,
                    ownership: HealthOwnership::Gateway,
                    required_checks: vec![
                        "control_socket".into(),
                        "gateway_state".into(),
                        "m8_child_status_capability".into(),
                        "gateway_children".into(),
                    ],
                    failed_checks: failed,
                }
            }
            Err(_) => HealthSnapshot {
                component: ComponentId::Gateway,
                observed_at_ms,
                freshness_ms: DEFAULT_HEALTH_FRESHNESS_MS,
                state: HealthState::Unknown,
                liveness: ProbeState::Unavailable,
                readiness: ProbeState::Unknown,
                identity: None,
                ownership: HealthOwnership::Gateway,
                required_checks: vec!["control_socket".into()],
                failed_checks: vec!["control_socket".into()],
            },
        }
    }

    async fn tunnel_snapshot(&self) -> HealthSnapshot {
        let observed_at_ms = now_ms();
        let status = self.tunnel.status().await;
        let health = self.tunnel.health().await;
        let liveness = map_tunnel_probe(health.liveness);
        let readiness = map_tunnel_probe(health.readiness);
        let mut failed = Vec::new();
        if !status.runtime_available {
            failed.push("runtime_available".into());
        }
        if matches!(liveness, ProbeState::Failed | ProbeState::Unavailable) {
            failed.push("liveness".into());
        }
        if matches!(readiness, ProbeState::Failed | ProbeState::Unavailable) {
            failed.push("readiness".into());
        }

        let state = match status.state {
            TunnelState::Failed => HealthState::Unhealthy,
            TunnelState::Starting | TunnelState::Stopping => HealthState::Unknown,
            TunnelState::Stopped => {
                if status.runtime_available {
                    HealthState::Degraded
                } else {
                    HealthState::Unhealthy
                }
            }
            TunnelState::Running => {
                if !status.runtime_available {
                    HealthState::Unhealthy
                } else if liveness == ProbeState::Passed && readiness == ProbeState::Passed {
                    HealthState::Healthy
                } else if matches!(liveness, ProbeState::Failed)
                    || matches!(readiness, ProbeState::Failed)
                {
                    HealthState::Unhealthy
                } else {
                    HealthState::Degraded
                }
            }
        };

        HealthSnapshot {
            component: ComponentId::Tunnel,
            observed_at_ms,
            freshness_ms: DEFAULT_HEALTH_FRESHNESS_MS,
            state,
            liveness,
            readiness,
            identity: status.pid.map(|pid| format!("pid:{pid}")),
            ownership: HealthOwnership::TunnelSupervisor,
            required_checks: vec![
                "runtime_available".into(),
                "liveness".into(),
                "readiness".into(),
            ],
            failed_checks: failed,
        }
    }

    async fn mcp_snapshot(&self, component: ComponentId) -> HealthSnapshot {
        let observed_at_ms = now_ms();
        let id = component.as_str();
        let status = self.supervisor.status(id).await.ok();
        let inventory = self.inventory.get(component).await.ok();

        let (state, liveness, readiness, ownership) = match status.as_ref() {
            Some(status) => match status.state {
                ProcessState::Running => (
                    HealthState::Healthy,
                    ProbeState::Passed,
                    ProbeState::Passed,
                    HealthOwnership::StudioSupervisor,
                ),
                ProcessState::Failed => (
                    HealthState::Unhealthy,
                    ProbeState::Failed,
                    ProbeState::Failed,
                    HealthOwnership::StudioSupervisor,
                ),
                ProcessState::Starting | ProcessState::Stopping => (
                    HealthState::Unknown,
                    ProbeState::Unknown,
                    ProbeState::Unknown,
                    HealthOwnership::StudioSupervisor,
                ),
                ProcessState::Stopped => (
                    HealthState::Degraded,
                    ProbeState::Unavailable,
                    ProbeState::Unavailable,
                    HealthOwnership::StudioSupervisor,
                ),
            },
            None => {
                let state = match inventory.as_ref().map(|item| item.installation_health) {
                    Some(InventoryHealth::Healthy) => HealthState::Degraded,
                    Some(InventoryHealth::Broken) => HealthState::Unhealthy,
                    _ => HealthState::Unknown,
                };
                (
                    state,
                    ProbeState::Unavailable,
                    ProbeState::Unknown,
                    HealthOwnership::RuntimeArtifact,
                )
            }
        };

        let identity = inventory
            .as_ref()
            .and_then(|item| {
                item.running_version
                    .as_ref()
                    .or(item.installed_version.as_ref())
            })
            .map(ToString::to_string);
        let mut failed = Vec::new();
        if !matches!(state, HealthState::Healthy) {
            failed.push("runtime_state".into());
        }
        if inventory
            .as_ref()
            .is_some_and(|item| item.installation_health != InventoryHealth::Healthy)
        {
            failed.push("installed_identity".into());
        }

        HealthSnapshot {
            component,
            observed_at_ms,
            freshness_ms: DEFAULT_HEALTH_FRESHNESS_MS,
            state,
            liveness,
            readiness,
            identity,
            ownership,
            required_checks: vec!["runtime_state".into(), "installed_identity".into()],
            failed_checks: failed,
        }
    }

    async fn inventory_snapshot(
        &self,
        component: ComponentId,
        ownership: HealthOwnership,
    ) -> HealthSnapshot {
        let observed_at_ms = now_ms();
        match self.inventory.get(component).await {
            Ok(item) => {
                let state = match item.installation_health {
                    InventoryHealth::Healthy => HealthState::Healthy,
                    InventoryHealth::Broken => HealthState::Unhealthy,
                    InventoryHealth::Unknown => HealthState::Unknown,
                };
                HealthSnapshot {
                    component,
                    observed_at_ms,
                    freshness_ms: DEFAULT_HEALTH_FRESHNESS_MS,
                    state,
                    liveness: ProbeState::Unavailable,
                    readiness: if state == HealthState::Healthy {
                        ProbeState::Passed
                    } else {
                        ProbeState::Unknown
                    },
                    identity: item
                        .running_version
                        .as_ref()
                        .or(item.installed_version.as_ref())
                        .map(ToString::to_string),
                    ownership,
                    required_checks: vec!["installed_identity".into()],
                    failed_checks: if state == HealthState::Healthy {
                        Vec::new()
                    } else {
                        vec!["installed_identity".into()]
                    },
                }
            }
            Err(_) => HealthSnapshot {
                component,
                observed_at_ms,
                freshness_ms: DEFAULT_HEALTH_FRESHNESS_MS,
                state: HealthState::Unknown,
                liveness: ProbeState::Unavailable,
                readiness: ProbeState::Unknown,
                identity: None,
                ownership,
                required_checks: vec!["installed_identity".into()],
                failed_checks: vec!["installed_identity".into()],
            },
        }
    }
}

fn map_tunnel_probe(value: TunnelHealthProbe) -> ProbeState {
    match value {
        TunnelHealthProbe::Passed => ProbeState::Passed,
        TunnelHealthProbe::Failed => ProbeState::Failed,
        TunnelHealthProbe::Unknown => ProbeState::Unknown,
        TunnelHealthProbe::Unavailable => ProbeState::Unavailable,
    }
}

fn now_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_freshness_is_bounded() {
        let snapshot = HealthSnapshot {
            component: ComponentId::Git,
            observed_at_ms: 1_000,
            freshness_ms: 5_000,
            state: HealthState::Healthy,
            liveness: ProbeState::Passed,
            readiness: ProbeState::Passed,
            identity: Some("1.0.0".into()),
            ownership: HealthOwnership::StudioSupervisor,
            required_checks: vec!["runtime_state".into()],
            failed_checks: Vec::new(),
        };
        assert!(snapshot.is_fresh_at(6_000));
        assert!(!snapshot.is_fresh_at(6_001));
        assert!(snapshot.supports_automatic_mutation(6_000));
    }

    #[test]
    fn unhealthy_or_unknown_snapshot_never_authorizes_automatic_mutation() {
        for state in [
            HealthState::Degraded,
            HealthState::Unhealthy,
            HealthState::Unknown,
        ] {
            let snapshot = HealthSnapshot {
                component: ComponentId::Gateway,
                observed_at_ms: 1_000,
                freshness_ms: 5_000,
                state,
                liveness: ProbeState::Unknown,
                readiness: ProbeState::Unknown,
                identity: None,
                ownership: HealthOwnership::Gateway,
                required_checks: Vec::new(),
                failed_checks: vec!["gateway".into()],
            };
            assert!(!snapshot.supports_automatic_mutation(1_001));
        }
    }
}
