use uuid::Uuid;

use crate::update::{
    ArtifactHistoryIdentity, CheckStatus, ComponentId, InventoryView, McpUpdatePhase,
    McpUpdateTransactionView,
};

#[derive(Debug, Clone)]
pub(crate) struct UpdateTransactionObservation {
    pub attempt_id: String,
    pub subject_id: String,
    pub transaction_id: String,
    pub component_code: String,
    pub domain: &'static str,
    pub source_version: Option<String>,
    pub target_version: String,
    pub phase: &'static str,
    pub source_revision: u128,
    pub install_outcome: &'static str,
    pub rollback_outcome: &'static str,
    pub verification_scope: &'static str,
    pub was_running: Option<bool>,
    pub error_code: Option<&'static str>,
    pub terminal: bool,
    pub owner_verified: bool,
    pub operation_id: Option<Uuid>,
    pub operation_kind: OperationKind,
    pub artifact: Option<ArtifactHistoryIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Prepare,
    Apply,
    Observation,
}

impl UpdateTransactionObservation {
    pub(crate) fn from_view(
        view: &McpUpdateTransactionView,
        operation_id: Option<Uuid>,
        operation_kind: OperationKind,
    ) -> Self {
        let component_code = view.component.to_string();
        let domain = domain_for(view.component);
        let phase = phase_code(view.phase);
        let terminal = matches!(
            view.phase,
            McpUpdatePhase::Completed
                | McpUpdatePhase::Failed
                | McpUpdatePhase::RolledBack
                | McpUpdatePhase::RollbackFailed
        );
        let install_outcome =
            if operation_kind == OperationKind::Prepare && view.phase == McpUpdatePhase::Failed {
                "not_attempted"
            } else {
                match view.phase {
                    McpUpdatePhase::Preparing | McpUpdatePhase::Staged => "not_attempted",
                    McpUpdatePhase::Completed => "succeeded",
                    McpUpdatePhase::Failed
                    | McpUpdatePhase::RolledBack
                    | McpUpdatePhase::RollbackFailed => "failed",
                    _ => "pending",
                }
            };
        let rollback_outcome = match view.phase {
            McpUpdatePhase::RollingBack => "pending",
            McpUpdatePhase::RolledBack => "succeeded",
            McpUpdatePhase::RollbackFailed => "failed",
            _ => match view.rollback_succeeded {
                Some(true) => "succeeded",
                Some(false) => "failed",
                None => "not_needed",
            },
        };
        let verification_scope = match view.phase {
            McpUpdatePhase::Staged => "artifact_probe",
            McpUpdatePhase::Completed | McpUpdatePhase::RolledBack => match view.component {
                ComponentId::Gateway => "catalog_probe",
                ComponentId::Studio => "external_fleet",
                _ => "owned_runtime",
            },
            McpUpdatePhase::Failed if view.rollback_succeeded == Some(true) => match view.component
            {
                ComponentId::Gateway => "catalog_probe",
                ComponentId::Studio => "external_fleet",
                _ => "owned_runtime",
            },
            _ => "none",
        };
        let attempt_key = format!("{domain}:{}", view.transaction_id);
        let subject_key = format!("component:{component_code}");
        Self {
            attempt_id: opaque_id("attempt", &attempt_key),
            subject_id: opaque_id("component", &subject_key),
            transaction_id: view.transaction_id.clone(),
            component_code,
            domain,
            source_version: view.source_version.as_ref().map(ToString::to_string),
            target_version: view.target_version.to_string(),
            phase,
            source_revision: view.updated_at_ms,
            install_outcome,
            rollback_outcome,
            verification_scope,
            was_running: view.was_running,
            error_code: view.error.as_ref().map(|_| "update_failed"),
            terminal,
            owner_verified: terminal,
            operation_id,
            operation_kind,
            artifact: None,
        }
    }

    pub(crate) fn from_view_with_artifact(
        view: &McpUpdateTransactionView,
        operation_id: Option<Uuid>,
        operation_kind: OperationKind,
        artifact: Option<ArtifactHistoryIdentity>,
    ) -> Self {
        let mut observation = Self::from_view(view, operation_id, operation_kind);
        observation.artifact = artifact;
        observation
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UpdateCheckObservation {
    pub subject_id: String,
    pub component_code: String,
    pub source_stream: String,
    pub operation_id: Uuid,
    pub check_outcome: &'static str,
    pub latest_version: Option<String>,
    pub installed_version: Option<String>,
    pub update_available: Option<bool>,
    pub error_code: Option<&'static str>,
}

impl UpdateCheckObservation {
    pub(crate) fn from_view(view: &InventoryView, operation_id: Uuid) -> Self {
        let component_code = view.component.to_string();
        let status = view.last_check.status;
        let fresh = status == CheckStatus::Ok;
        Self {
            subject_id: opaque_id("component", &format!("component:{component_code}")),
            component_code: component_code.clone(),
            source_stream: format!("update-check/{operation_id}/{component_code}"),
            operation_id,
            check_outcome: match status {
                CheckStatus::Ok => "ok",
                CheckStatus::Error => "error",
                CheckStatus::Never => "cancelled",
            },
            latest_version: fresh
                .then(|| view.latest_version.as_ref().map(ToString::to_string))
                .flatten(),
            installed_version: view.installed_version.as_ref().map(ToString::to_string),
            update_available: fresh.then_some(view.update_available),
            error_code: (status == CheckStatus::Error).then_some("provider_error"),
        }
    }
}

fn domain_for(component: ComponentId) -> &'static str {
    match component {
        ComponentId::Gateway => "gateway",
        ComponentId::Fleet => "fleet",
        ComponentId::Studio => "studio",
        ComponentId::Tunnel => "tunnel",
        _ => "mcp",
    }
}

fn phase_code(phase: McpUpdatePhase) -> &'static str {
    match phase {
        McpUpdatePhase::Preparing => "preparing",
        McpUpdatePhase::Staged => "staged",
        McpUpdatePhase::Stopping => "stopping",
        McpUpdatePhase::Activating => "activating",
        McpUpdatePhase::Starting => "starting",
        McpUpdatePhase::Verifying => "verifying",
        McpUpdatePhase::GatewayStopping => "gateway_stopping",
        McpUpdatePhase::GatewayRestarting => "gateway_restarting",
        McpUpdatePhase::GatewayReconnecting => "gateway_reconnecting",
        McpUpdatePhase::GatewayCatalogVerifying => "gateway_catalog_verifying",
        McpUpdatePhase::ActivationPending => "activation_pending",
        McpUpdatePhase::ExternalActivating => "external_activating",
        McpUpdatePhase::HealthVerifying => "health_verifying",
        McpUpdatePhase::RollingBack => "rolling_back",
        McpUpdatePhase::RolledBack => "rolled_back",
        McpUpdatePhase::Completed => "completed",
        McpUpdatePhase::Failed => "failed",
        McpUpdatePhase::RollbackFailed => "rollback_failed",
    }
}

fn opaque_id(prefix: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!("{prefix}-{}", &digest[..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{
        Architecture, ComponentClass, DriftState, HostMode, InventoryHealth, OperatingSystem,
        Platform, ReleaseCheckView, ReleaseProviderId, Version,
    };

    fn view(phase: McpUpdatePhase, rollback_succeeded: Option<bool>) -> McpUpdateTransactionView {
        McpUpdateTransactionView {
            transaction_id: "txn-history-test".into(),
            component: ComponentId::Git,
            source_version: Some(Version::parse("1.0.0").unwrap()),
            target_version: Version::parse("1.1.0").unwrap(),
            phase,
            was_running: Some(true),
            rollback_succeeded,
            error: (phase == McpUpdatePhase::Failed).then_some("owner failure".into()),
            updated_at_ms: 1,
        }
    }

    #[test]
    fn prepare_failure_is_not_install_failure() {
        let observation = UpdateTransactionObservation::from_view(
            &view(McpUpdatePhase::Failed, None),
            Some(Uuid::new_v4()),
            OperationKind::Prepare,
        );
        assert_eq!(observation.install_outcome, "not_attempted");
        assert_eq!(observation.rollback_outcome, "not_needed");
        assert!(observation.terminal);
    }

    #[test]
    fn rollback_success_is_not_target_install_success() {
        let observation = UpdateTransactionObservation::from_view(
            &view(McpUpdatePhase::Failed, Some(true)),
            Some(Uuid::new_v4()),
            OperationKind::Apply,
        );
        assert_eq!(observation.install_outcome, "failed");
        assert_eq!(observation.rollback_outcome, "succeeded");
        assert_eq!(observation.verification_scope, "owned_runtime");
    }

    #[test]
    fn failed_check_cached_latest_is_not_fresh_availability() {
        let inventory = InventoryView {
            component: ComponentId::Git,
            display_name: "Git MCP",
            class: ComponentClass::McpBinary,
            provider: ReleaseProviderId::ThirteenthXGitHub,
            platform: Platform {
                os: OperatingSystem::Darwin,
                arch: Architecture::Arm64,
            },
            host_mode: HostMode::RuntimeOnly,
            source_present: false,
            installed_version: Some(Version::parse("1.0.0").unwrap()),
            running_version: Some(Version::parse("1.0.0").unwrap()),
            desired_version: Some(Version::parse("1.0.0").unwrap()),
            desired_pinned: false,
            latest_version: Some(Version::parse("1.1.0").unwrap()),
            update_available: true,
            installation_health: InventoryHealth::Healthy,
            drift: DriftState::UpdateAvailable,
            last_check: ReleaseCheckView {
                checked_at_ms: Some(1),
                status: CheckStatus::Error,
                error: Some("provider failure".into()),
            },
        };
        let observation = UpdateCheckObservation::from_view(&inventory, Uuid::new_v4());
        assert_eq!(observation.check_outcome, "error");
        assert_eq!(observation.latest_version, None);
        assert_eq!(observation.update_available, None);
        assert_eq!(observation.error_code, Some("provider_error"));
    }
}
