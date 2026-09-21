use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    registry::{RegistryEntryView, RuntimeKind},
    update::{
        ClientFreshness, ReconciliationPhase, ReconciliationState, ReconciliationView,
        StudioConfigActivation,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigProvenance {
    Loaded,
    Committed,
}

impl ConfigProvenance {
    pub(crate) const fn as_db(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Committed => "committed",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConfigRevisionObservation {
    pub revision_id: String,
    pub surface_code: String,
    pub exact_bytes_sha256: Option<String>,
    pub safe_projection_sha256: String,
    pub schema_code: Option<String>,
    pub provenance: ConfigProvenance,
    pub change_categories_json: String,
    pub operation_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub relation: &'static str,
}

impl ConfigRevisionObservation {
    pub(crate) fn loaded_studio(digest: &str, run_id: Uuid) -> Self {
        Self {
            revision_id: Uuid::new_v4().to_string(),
            surface_code: "studio.config".into(),
            exact_bytes_sha256: Some(digest.to_owned()),
            safe_projection_sha256: digest.to_owned(),
            schema_code: Some("studio.toml".into()),
            provenance: ConfigProvenance::Loaded,
            change_categories_json: r#"["loaded"]"#.into(),
            operation_id: None,
            run_id: Some(run_id),
            relation: "loaded_by",
        }
    }

    pub(crate) fn registry(
        view: &RegistryEntryView,
        present: bool,
        operation_id: Uuid,
        run_id: Option<Uuid>,
    ) -> (Self, RegistryEntryProjection) {
        let key_digest = digest_text(&view.id);
        let runtime_kind = match view.runtime {
            RuntimeKind::Rust => "rust",
            RuntimeKind::Node => "node",
            RuntimeKind::Python => "python",
        };
        let safe = serde_json::json!({
            "registry_key_digest": key_digest,
            "present": present,
            "enabled": present.then_some(view.enabled),
            "runtime_kind": present.then_some(runtime_kind),
        })
        .to_string();
        let observation = Self {
            revision_id: Uuid::new_v4().to_string(),
            surface_code: "registry.entry".into(),
            exact_bytes_sha256: None,
            safe_projection_sha256: digest_text(&safe),
            schema_code: Some("registry.v1.safe".into()),
            provenance: ConfigProvenance::Committed,
            change_categories_json: if present {
                r#"["registry_commit"]"#.into()
            } else {
                r#"["registry_remove"]"#.into()
            },
            operation_id: Some(operation_id),
            run_id,
            relation: "committed_by",
        };
        (
            observation,
            RegistryEntryProjection {
                subject_id: format!("registry-{}", &key_digest[..32]),
                registry_key_digest: key_digest,
                present,
                enabled: present.then_some(view.enabled),
                runtime_kind: present.then_some(runtime_kind),
            },
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RegistryEntryProjection {
    pub subject_id: String,
    pub registry_key_digest: String,
    pub present: bool,
    pub enabled: Option<bool>,
    pub runtime_kind: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct DriftObservation {
    pub observation_id: String,
    pub source_stream: String,
    pub operation_id: Uuid,
    pub host_key_digest: Option<String>,
    pub generation: u64,
    pub manifest_digest: Option<String>,
    pub state: &'static str,
    pub phase: &'static str,
    pub studio_config_activation: &'static str,
    pub client_freshness: &'static str,
    pub rollback_outcome: &'static str,
    pub observation_kind: &'static str,
    pub surfaces: Vec<String>,
    pub run_id: Option<Uuid>,
}

impl DriftObservation {
    pub(crate) fn from_view(
        view: &ReconciliationView,
        operation_id: Uuid,
        observation_kind: &'static str,
        run_id: Option<Uuid>,
    ) -> Self {
        Self {
            observation_id: Uuid::new_v4().to_string(),
            source_stream: format!("drift/{operation_id}"),
            operation_id,
            host_key_digest: view.host_id.as_deref().map(digest_text),
            generation: view.generation,
            manifest_digest: view.catalog_fingerprint.as_deref().map(digest_text),
            state: reconciliation_state(view.state),
            phase: reconciliation_phase(view.phase),
            studio_config_activation: activation(view.studio_config_activation),
            client_freshness: freshness(view.client_freshness),
            rollback_outcome: match view.rollback_succeeded {
                Some(true) => "succeeded",
                Some(false) => "failed",
                None => "not_needed",
            },
            observation_kind,
            surfaces: view.affected_surfaces.clone(),
            run_id,
        }
    }
}

fn digest_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn reconciliation_state(value: ReconciliationState) -> &'static str {
    match value {
        ReconciliationState::Unknown => "unknown",
        ReconciliationState::Synchronized => "synchronized",
        ReconciliationState::ManagedSafeDrift => "managed_safe_drift",
        ReconciliationState::UnmanagedConflict => "unmanaged_conflict",
        ReconciliationState::Broken => "broken",
    }
}

fn reconciliation_phase(value: ReconciliationPhase) -> &'static str {
    match value {
        ReconciliationPhase::Idle => "idle",
        ReconciliationPhase::Checking => "checking",
        ReconciliationPhase::ReconcileReady => "reconcile_ready",
        ReconciliationPhase::Snapshotting => "snapshotting",
        ReconciliationPhase::Rendering => "rendering",
        ReconciliationPhase::Validating => "validating",
        ReconciliationPhase::ReloadingGateway => "reloading_gateway",
        ReconciliationPhase::RestartingTunnel => "restarting_tunnel",
        ReconciliationPhase::CatalogVerifying => "catalog_verifying",
        ReconciliationPhase::Synchronized => "synchronized",
        ReconciliationPhase::Failed => "failed",
        ReconciliationPhase::RollbackFailed => "rollback_failed",
    }
}

fn activation(value: StudioConfigActivation) -> &'static str {
    match value {
        StudioConfigActivation::Unknown => "unknown",
        StudioConfigActivation::Active => "active",
        StudioConfigActivation::RestartRequired => "restart_required",
    }
}

fn freshness(value: ClientFreshness) -> &'static str {
    match value {
        ClientFreshness::Unknown => "unknown",
        ClientFreshness::RefreshPending => "refresh_pending",
    }
}
