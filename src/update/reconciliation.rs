use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    process::Command,
    sync::RwLock,
    time::{sleep, timeout},
};

use crate::{
    config::LoadedConfigIdentity,
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
    tunnel::{TunnelState, TunnelSupervisor},
};

use super::{
    ComponentCatalog, ComponentId, InventoryService,
    gateway::probe_gateway_tool_names,
    transaction::{now_ms, sanitize_transaction_error},
};

const PLAN_SCHEMA_VERSION: u32 = 1;
const MANAGED_SCHEMA_VERSION: u32 = 1;
const PLAN_TIMEOUT: Duration = Duration::from_secs(15);
const PLAN_MAX_BYTES: usize = 2 * 1024 * 1024;
const GATEWAY_RELOAD_WAIT: Duration = Duration::from_millis(1800);
const TUNNEL_HEALTH_WINDOW: Duration = Duration::from_millis(800);
const MANIFEST_RELATIVE: &str = "fleet/state/reconciliation.json";
const BACKUP_PREFIX: &str = "reconciliation-backup-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationState {
    Unknown,
    Synchronized,
    ManagedSafeDrift,
    UnmanagedConflict,
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationPhase {
    Idle,
    Checking,
    ReconcileReady,
    Snapshotting,
    Rendering,
    Validating,
    ReloadingGateway,
    RestartingTunnel,
    CatalogVerifying,
    Synchronized,
    Failed,
    RollbackFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientFreshness {
    Unknown,
    RefreshPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioConfigActivation {
    Unknown,
    Active,
    RestartRequired,
}

#[derive(Debug, Clone)]
pub struct StudioProcessConfigIdentity {
    pub canonical_path: Option<PathBuf>,
    pub sha256: Option<String>,
    pub process_instance: String,
}

impl StudioProcessConfigIdentity {
    pub fn from_loaded(loaded: LoadedConfigIdentity, process_instance: String) -> Self {
        Self {
            canonical_path: loaded.canonical_path,
            sha256: loaded.sha256,
            process_instance,
        }
    }

    fn unknown() -> Self {
        Self {
            canonical_path: None,
            sha256: None,
            process_instance: "unknown".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationView {
    pub host_id: Option<String>,
    pub state: ReconciliationState,
    pub phase: ReconciliationPhase,
    pub safe_to_reconcile: bool,
    pub affected_surfaces: Vec<String>,
    pub gateway_reload_required: bool,
    pub tunnel_restart_required: bool,
    pub studio_restart_required: bool,
    pub studio_config_activation: StudioConfigActivation,
    pub catalog_fingerprint: Option<String>,
    pub generation: u64,
    pub client_freshness: ClientFreshness,
    pub rollback_succeeded: Option<bool>,
    pub last_error: Option<String>,
    pub checked_at_ms: u128,
}

impl Default for ReconciliationView {
    fn default() -> Self {
        Self {
            host_id: None,
            state: ReconciliationState::Unknown,
            phase: ReconciliationPhase::Idle,
            safe_to_reconcile: false,
            affected_surfaces: Vec::new(),
            gateway_reload_required: false,
            tunnel_restart_required: false,
            studio_restart_required: false,
            studio_config_activation: StudioConfigActivation::Unknown,
            catalog_fingerprint: None,
            generation: 0,
            client_freshness: ClientFreshness::Unknown,
            rollback_succeeded: None,
            last_error: None,
            checked_at_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone)]
struct DesiredSurface {
    surface: String,
    relative_path: PathBuf,
    bytes: Vec<u8>,
    sha256: String,
    effects: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct DesiredPlan {
    host_id: String,
    outputs: BTreeMap<String, DesiredSurface>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRenderPlan {
    schema_version: u32,
    host_id: String,
    runtime_root: String,
    outputs: Vec<RawRenderOutput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRenderOutput {
    surface: String,
    relative_path: String,
    sha256: String,
    content_encoding: String,
    content_b64: String,
    ownership: String,
    effects: Vec<String>,
}

#[async_trait]
trait RenderPlanProvider: Send + Sync {
    async fn render_plan(&self) -> StudioResult<DesiredPlan>;
}

struct FleetRenderPlanProvider {
    catalog: ComponentCatalog,
}

#[async_trait]
impl RenderPlanProvider for FleetRenderPlanProvider {
    async fn render_plan(&self) -> StudioResult<DesiredPlan> {
        let fleet_root = self.catalog.install_path(ComponentId::Fleet)?;
        let host_id = active_host_id(&fleet_root)?;
        let script = fleet_root.join("scripts/fleetctl.py");
        let metadata = fs::symlink_metadata(&script).map_err(|error| {
            StudioError::UpdateTransaction(format!(
                "Fleet render-plan script is unavailable: {error}"
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet render-plan script must be a regular file".into(),
            ));
        }

        let mut command = Command::new("python3");
        command
            .arg(&script)
            .arg("render-plan")
            .arg("--host")
            .arg(&host_id)
            .arg("--json")
            .current_dir(&fleet_root)
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .kill_on_drop(true);

        let output = timeout(PLAN_TIMEOUT, command.output())
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan timed out".into(),
            })?
            .map_err(|error| StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: format!("Fleet render-plan failed to start: {error}"),
            })?;
        if !output.status.success() {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan returned failure".into(),
            });
        }
        if output.stdout.len() > PLAN_MAX_BYTES {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan response exceeded size limit".into(),
            });
        }
        let raw: RawRenderPlan = serde_json::from_slice(&output.stdout).map_err(|error| {
            StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: format!("Fleet render-plan returned invalid JSON: {error}"),
            }
        })?;
        validate_render_plan(raw, &host_id, self.catalog.runtime_root())
    }
}

fn active_host_id(fleet_root: &Path) -> StudioResult<String> {
    let hosts = fleet_root.join("hosts");
    let metadata = fs::symlink_metadata(&hosts)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet hosts directory must be a regular directory".into(),
        ));
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(hosts)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet hosts directory contains a non-regular entry".into(),
            ));
        }
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| StudioError::UpdateTransaction("invalid Fleet host filename".into()))?
            .to_owned();
        if name.ends_with(".toml") && !name.ends_with(".example.toml") {
            profiles.push((name, fs::read_to_string(path)?));
        }
    }
    if profiles.len() != 1 {
        return Err(StudioError::UpdateTransaction(
            "Fleet reconciliation requires exactly one active host profile".into(),
        ));
    }
    let (filename, text) = profiles.pop().expect("one profile");
    let value: toml::Value = toml::from_str(&text)?;
    let host_id = value
        .get("host_id")
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| StudioError::UpdateTransaction("Fleet host_id is missing".into()))?
        .to_owned();
    if filename != format!("{host_id}.toml") {
        return Err(StudioError::UpdateTransaction(
            "Fleet active host filename does not match host_id".into(),
        ));
    }
    Ok(host_id)
}

fn validate_render_plan(
    raw: RawRenderPlan,
    expected_host: &str,
    trusted_runtime_root: &Path,
) -> StudioResult<DesiredPlan> {
    if raw.schema_version != PLAN_SCHEMA_VERSION {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "unsupported Fleet render-plan schema".into(),
        });
    }
    if raw.host_id != expected_host {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "Fleet render-plan host identity mismatch".into(),
        });
    }
    let planned_root = fs::canonicalize(&raw.runtime_root).map_err(|error| {
        StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: format!("Fleet render-plan runtime_root does not resolve: {error}"),
        }
    })?;
    let trusted_root = fs::canonicalize(trusted_runtime_root)?;
    if planned_root != trusted_root {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "Fleet render-plan runtime_root is not the trusted runtime root".into(),
        });
    }

    let expected = expected_surfaces();
    if raw.outputs.len() != expected.len() {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "Fleet render-plan output set is incomplete or excessive".into(),
        });
    }

    let mut outputs = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for output in raw.outputs {
        let Some((expected_path, expected_effects)) = expected.get(output.surface.as_str()) else {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan contains an unknown managed surface".into(),
            });
        };
        if output.ownership != "fleet_managed" || output.content_encoding != "base64" {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan ownership/encoding is invalid".into(),
            });
        }
        let relative = validated_relative_path(&output.relative_path)?;
        if &relative != expected_path || !paths.insert(relative.clone()) {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan destination is unexpected or duplicated".into(),
            });
        }
        let bytes = BASE64.decode(output.content_b64.as_bytes()).map_err(|_| {
            StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan content encoding is invalid".into(),
            }
        })?;
        let sha256 = sha256_bytes(&bytes);
        if sha256 != output.sha256 {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan content digest mismatch".into(),
            });
        }
        let effects = output.effects.into_iter().collect::<BTreeSet<_>>();
        if &effects != expected_effects {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan lifecycle effects mismatch".into(),
            });
        }
        let surface = output.surface.clone();
        if outputs
            .insert(
                surface.clone(),
                DesiredSurface {
                    surface,
                    relative_path: relative,
                    bytes,
                    sha256,
                    effects,
                },
            )
            .is_some()
        {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet render-plan contains duplicate surfaces".into(),
            });
        }
    }

    Ok(DesiredPlan {
        host_id: raw.host_id,
        outputs,
    })
}

fn expected_surfaces() -> BTreeMap<&'static str, (PathBuf, BTreeSet<String>)> {
    BTreeMap::from([
        (
            "gateway.filesystem",
            (
                PathBuf::from("gateway/servers.d/filesystem.yaml"),
                BTreeSet::from(["gateway_reload".into()]),
            ),
        ),
        (
            "gateway.git",
            (
                PathBuf::from("gateway/servers.d/git.yaml"),
                BTreeSet::from(["gateway_reload".into()]),
            ),
        ),
        (
            "gateway.exec",
            (
                PathBuf::from("gateway/servers.d/exec.yaml"),
                BTreeSet::from(["gateway_reload".into()]),
            ),
        ),
        (
            "studio.config",
            (
                PathBuf::from("studio/studio.toml"),
                BTreeSet::from(["studio_restart".into()]),
            ),
        ),
        (
            "tunnel.config",
            (
                PathBuf::from("tunnel-client/config.yaml"),
                BTreeSet::from(["tunnel_restart".into()]),
            ),
        ),
    ])
}

fn validated_relative_path(value: &str) -> StudioResult<PathBuf> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "Fleet render-plan destination is not a safe relative path".into(),
        });
    }
    Ok(path)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedSurface {
    relative_path: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StudioRestartPending {
    sha256: String,
    requested_from_process: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedStateManifest {
    schema_version: u32,
    host_id: String,
    generation: u64,
    surfaces: BTreeMap<String, ManagedSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    studio_restart_pending: Option<StudioRestartPending>,
}

#[derive(Debug, Clone)]
struct SurfaceSnapshot {
    relative_path: PathBuf,
    bytes: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
}

#[derive(Debug)]
struct Backup {
    root: PathBuf,
    entries: Vec<SurfaceSnapshot>,
}

#[derive(Debug)]
struct Evaluation {
    view: ReconciliationView,
    plan: DesiredPlan,
    manifest: Option<ManagedStateManifest>,
    legacy_adopt: bool,
}

#[async_trait]
trait ReconciliationRuntime: Send + Sync {
    async fn tunnel_state(&self) -> StudioResult<TunnelState>;
    async fn reload_gateway(&self) -> StudioResult<()>;
    async fn restart_tunnel(&self) -> StudioResult<()>;
    async fn restore_tunnel_state(&self, previous: TunnelState) -> StudioResult<()>;
    fn validate_tunnel_binding(&self) -> StudioResult<()>;
    async fn catalog_tool_names(&self) -> StudioResult<Vec<String>>;
}

struct StudioReconciliationRuntime {
    catalog: ComponentCatalog,
    tunnel: Arc<TunnelSupervisor>,
    inventory: Arc<InventoryService>,
}

#[async_trait]
impl ReconciliationRuntime for StudioReconciliationRuntime {
    async fn tunnel_state(&self) -> StudioResult<TunnelState> {
        Ok(self.tunnel.status().await.state)
    }

    async fn reload_gateway(&self) -> StudioResult<()> {
        let state = self.tunnel.status().await.state;
        match state {
            TunnelState::Running => {
                sleep(GATEWAY_RELOAD_WAIT).await;
                if self.tunnel.status().await.state != TunnelState::Running {
                    return Err(StudioError::UpdateVerificationFailed {
                        component: "reconciliation".into(),
                        detail: "tunnel owner did not remain running during Gateway reload window"
                            .into(),
                    });
                }
                Ok(())
            }
            TunnelState::Stopped => Ok(()),
            TunnelState::Starting | TunnelState::Stopping | TunnelState::Failed => {
                Err(StudioError::Conflict(
                    "Gateway reload requires stable tunnel ownership state".into(),
                ))
            }
        }
    }

    async fn restart_tunnel(&self) -> StudioResult<()> {
        if self.tunnel.status().await.state != TunnelState::Running {
            return Err(StudioError::Conflict(
                "tunnel restart requires the tunnel to be running".into(),
            ));
        }
        self.tunnel.stop().await?;
        self.tunnel.start().await?;
        sleep(TUNNEL_HEALTH_WINDOW).await;
        if self.tunnel.status().await.state != TunnelState::Running {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "tunnel did not remain running after reconciliation restart".into(),
            });
        }
        Ok(())
    }

    async fn restore_tunnel_state(&self, previous: TunnelState) -> StudioResult<()> {
        let current = self.tunnel.status().await.state;
        match previous {
            TunnelState::Running => {
                if current == TunnelState::Running {
                    return Ok(());
                }
                if matches!(current, TunnelState::Starting | TunnelState::Stopping) {
                    return Err(StudioError::RollbackFailed {
                        component: "reconciliation".into(),
                        detail: "tunnel is transitional during reconciliation rollback".into(),
                    });
                }
                self.tunnel.start().await?;
                sleep(TUNNEL_HEALTH_WINDOW).await;
                if self.tunnel.status().await.state != TunnelState::Running {
                    return Err(StudioError::RollbackFailed {
                        component: "reconciliation".into(),
                        detail: "tunnel rollback restart did not remain running".into(),
                    });
                }
                Ok(())
            }
            TunnelState::Stopped => {
                if matches!(current, TunnelState::Running | TunnelState::Starting) {
                    self.tunnel.stop().await?;
                }
                if self.tunnel.status().await.state != TunnelState::Stopped {
                    return Err(StudioError::RollbackFailed {
                        component: "reconciliation".into(),
                        detail: "tunnel rollback did not restore stopped state".into(),
                    });
                }
                Ok(())
            }
            TunnelState::Starting | TunnelState::Stopping | TunnelState::Failed => {
                Err(StudioError::RollbackFailed {
                    component: "reconciliation".into(),
                    detail: "pre-reconciliation tunnel state was not stable".into(),
                })
            }
        }
    }

    fn validate_tunnel_binding(&self) -> StudioResult<()> {
        self.tunnel.validate_gateway_binding(
            &self.catalog.install_path(ComponentId::Gateway)?,
            &self.catalog.runtime_root().join("gateway/servers.d"),
        )
    }

    async fn catalog_tool_names(&self) -> StudioResult<Vec<String>> {
        let gateway = self.inventory.get(ComponentId::Gateway).await?;
        let version =
            gateway
                .installed_version
                .ok_or_else(|| StudioError::UpdateVerificationFailed {
                    component: "reconciliation".into(),
                    detail: "installed Gateway version is unavailable".into(),
                })?;
        probe_gateway_tool_names(
            &self.catalog.install_path(ComponentId::Gateway)?,
            &self.catalog.runtime_root().join("gateway/servers.d"),
            &version,
        )
        .await
    }
}

trait ReconciliationStateStore: Send + Sync {
    fn persist(&self, runtime_root: &Path, manifest: &ManagedStateManifest) -> StudioResult<()>;
}

struct FileReconciliationStateStore;

impl ReconciliationStateStore for FileReconciliationStateStore {
    fn persist(&self, runtime_root: &Path, manifest: &ManagedStateManifest) -> StudioResult<()> {
        persist_manifest(runtime_root, manifest)
    }
}

#[derive(Clone)]
pub struct RuntimeReconciler {
    catalog: ComponentCatalog,
    plan_provider: Arc<dyn RenderPlanProvider>,
    runtime: Arc<dyn ReconciliationRuntime>,
    events: EventHub,
    status: Arc<RwLock<ReconciliationView>>,
    active: Arc<StdMutex<bool>>,
    state_store: Arc<dyn ReconciliationStateStore>,
    studio_config_identity: StudioProcessConfigIdentity,
}

impl RuntimeReconciler {
    pub fn new(
        catalog: ComponentCatalog,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> StudioResult<Self> {
        Self::new_with_config_identity(
            catalog,
            tunnel,
            inventory,
            events,
            StudioProcessConfigIdentity::unknown(),
        )
    }

    pub fn new_with_config_identity(
        catalog: ComponentCatalog,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
        studio_config_identity: StudioProcessConfigIdentity,
    ) -> StudioResult<Self> {
        Ok(Self {
            plan_provider: Arc::new(FleetRenderPlanProvider {
                catalog: catalog.clone(),
            }),
            runtime: Arc::new(StudioReconciliationRuntime {
                catalog: catalog.clone(),
                tunnel,
                inventory,
            }),
            catalog,
            events,
            status: Arc::new(RwLock::new(ReconciliationView::default())),
            active: Arc::new(StdMutex::new(false)),
            state_store: Arc::new(FileReconciliationStateStore),
            studio_config_identity,
        })
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        plan_provider: Arc<dyn RenderPlanProvider>,
        runtime: Arc<dyn ReconciliationRuntime>,
        events: EventHub,
    ) -> Self {
        Self {
            catalog,
            plan_provider,
            runtime,
            events,
            status: Arc::new(RwLock::new(ReconciliationView::default())),
            active: Arc::new(StdMutex::new(false)),
            state_store: Arc::new(FileReconciliationStateStore),
            studio_config_identity: StudioProcessConfigIdentity::unknown(),
        }
    }

    fn studio_surface<'a>(&self, plan: &'a DesiredPlan) -> Option<&'a DesiredSurface> {
        plan.outputs
            .values()
            .find(|surface| surface.effects.contains("studio_restart"))
    }

    fn loaded_studio_config_proves(&self, expected_sha256: &str) -> bool {
        let Some(loaded_path) = self.studio_config_identity.canonical_path.as_ref() else {
            return false;
        };
        let Some(loaded_sha256) = self.studio_config_identity.sha256.as_deref() else {
            return false;
        };
        if loaded_sha256 != expected_sha256 {
            return false;
        }
        let expected_path = self.catalog.runtime_root().join("studio/studio.toml");
        fs::canonicalize(expected_path)
            .ok()
            .is_some_and(|path| &path == loaded_path)
    }

    fn reconcile_studio_restart_marker(&self, evaluation: &Evaluation) -> StudioResult<bool> {
        let Some(manifest) = evaluation.manifest.as_ref() else {
            return Ok(false);
        };
        let Some(surface) = self.studio_surface(&evaluation.plan) else {
            return Ok(false);
        };
        let mut next = manifest.clone();
        let changed = match next.studio_restart_pending.as_ref() {
            Some(pending)
                if self.loaded_studio_config_proves(&pending.sha256)
                    && self.studio_config_identity.process_instance
                        != pending.requested_from_process =>
            {
                next.studio_restart_pending = None;
                true
            }
            Some(_) => false,
            None if evaluation.view.state == ReconciliationState::Synchronized
                && !self.loaded_studio_config_proves(&surface.sha256) =>
            {
                next.studio_restart_pending = Some(StudioRestartPending {
                    sha256: surface.sha256.clone(),
                    requested_from_process: self.studio_config_identity.process_instance.clone(),
                });
                true
            }
            None => false,
        };
        if changed {
            self.state_store
                .persist(self.catalog.runtime_root(), &next)?;
        }
        Ok(changed)
    }

    fn decorate_studio_activation(&self, view: &mut ReconciliationView, plan: &DesiredPlan) {
        if view.studio_restart_required {
            view.studio_config_activation = StudioConfigActivation::RestartRequired;
            return;
        }
        view.studio_config_activation = self
            .studio_surface(plan)
            .filter(|surface| self.loaded_studio_config_proves(&surface.sha256))
            .map_or(StudioConfigActivation::Unknown, |_| {
                StudioConfigActivation::Active
            });
    }

    pub async fn status(&self) -> ReconciliationView {
        self.status.read().await.clone()
    }

    pub async fn check(&self) -> StudioResult<ReconciliationView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("reconciliation_check")?;
        let _guard = self.acquire()?;
        self.publish_phase(ReconciliationPhase::Checking).await;
        match self.evaluate().await {
            Ok(mut evaluation) => {
                if evaluation.legacy_adopt {
                    self.runtime.validate_tunnel_binding()?;
                    let names = self.runtime.catalog_tool_names().await?;
                    let fingerprint = catalog_fingerprint(&names);
                    let manifest = manifest_for_plan(&evaluation.plan, 1, Some(fingerprint))?;
                    self.state_store
                        .persist(self.catalog.runtime_root(), &manifest)?;
                    evaluation = self.evaluate().await?;
                }
                if self.reconcile_studio_restart_marker(&evaluation)? {
                    evaluation = self.evaluate().await?;
                }
                self.decorate_studio_activation(&mut evaluation.view, &evaluation.plan);
                self.set_status(evaluation.view.clone()).await;
                Ok(evaluation.view)
            }
            Err(error) => {
                let mut view = self.status().await;
                view.state = ReconciliationState::Broken;
                view.phase = ReconciliationPhase::Failed;
                view.safe_to_reconcile = false;
                view.rollback_succeeded = None;
                view.last_error = Some(sanitize_reconciliation_error(&error));
                view.checked_at_ms = now_ms();
                self.set_status(view.clone()).await;
                Ok(view)
            }
        }
    }

    pub async fn adopt(&self) -> StudioResult<ReconciliationView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("reconciliation_adopt")?;
        let _guard = self.acquire()?;
        let plan = self.plan_provider.render_plan().await?;
        validate_launcher_surface(self.catalog.runtime_root(), self.catalog.bin_root())?;

        let mut surfaces = BTreeMap::new();
        for desired in plan.outputs.values() {
            let active =
                read_active_surface(self.catalog.runtime_root(), desired)?.ok_or_else(|| {
                    StudioError::Conflict(
                        "cannot adopt a missing Fleet-managed runtime surface".into(),
                    )
                })?;
            surfaces.insert(
                desired.surface.clone(),
                ManagedSurface {
                    relative_path: path_to_string(&desired.relative_path)?,
                    sha256: sha256_bytes(&active),
                },
            );
        }
        self.runtime.validate_tunnel_binding()?;
        let tool_names = self.runtime.catalog_tool_names().await?;
        let fingerprint = catalog_fingerprint(&tool_names);
        let previous = load_manifest(self.catalog.runtime_root())?;
        let manifest = ManagedStateManifest {
            schema_version: MANAGED_SCHEMA_VERSION,
            host_id: plan.host_id.clone(),
            generation: previous
                .as_ref()
                .map_or(1, |state| state.generation.saturating_add(1)),
            surfaces,
            catalog_fingerprint: Some(fingerprint),
            studio_restart_pending: previous.and_then(|state| state.studio_restart_pending),
        };
        self.state_store
            .persist(self.catalog.runtime_root(), &manifest)?;
        let mut evaluation = self.evaluate().await?;
        if self.reconcile_studio_restart_marker(&evaluation)? {
            evaluation = self.evaluate().await?;
        }
        self.decorate_studio_activation(&mut evaluation.view, &evaluation.plan);
        self.set_status(evaluation.view.clone()).await;
        Ok(evaluation.view)
    }

    pub async fn apply(&self) -> StudioResult<ReconciliationView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("reconciliation_apply")?;
        let _guard = self.acquire()?;
        let evaluation = self.evaluate().await?;
        if evaluation.view.state != ReconciliationState::ManagedSafeDrift
            || !evaluation.view.safe_to_reconcile
        {
            return Err(StudioError::Conflict(
                "runtime reconciliation is not in managed-safe drift state".into(),
            ));
        }
        let manifest = evaluation.manifest.clone().ok_or_else(|| {
            StudioError::UpdateTransaction("managed-safe reconciliation has no baseline".into())
        })?;
        let changed = changed_surfaces(self.catalog.runtime_root(), &evaluation.plan)?;
        if changed.is_empty() {
            return Err(StudioError::Conflict(
                "runtime reconciliation has no changed managed surfaces".into(),
            ));
        }

        self.publish_phase(ReconciliationPhase::Snapshotting).await;
        let backup = create_backup(self.catalog.runtime_root(), &changed)?;
        let previous_tunnel = self.runtime.tunnel_state().await?;
        if !matches!(previous_tunnel, TunnelState::Running | TunnelState::Stopped) {
            remove_backup(&backup)?;
            return Err(StudioError::Conflict(
                "runtime reconciliation requires a stable tunnel state".into(),
            ));
        }

        let gateway_changed = changed
            .iter()
            .any(|surface| surface.effects.contains("gateway_reload"));
        let tunnel_changed = changed
            .iter()
            .any(|surface| surface.effects.contains("tunnel_restart"));
        let studio_changed = changed
            .iter()
            .any(|surface| surface.effects.contains("studio_restart"));

        let apply_result = async {
            validate_manifest_bytes(self.catalog.runtime_root(), &manifest)?;
            self.publish_phase(ReconciliationPhase::Rendering).await;
            for surface in &changed {
                atomic_write_surface(self.catalog.runtime_root(), surface)?;
            }

            self.publish_phase(ReconciliationPhase::Validating).await;
            let rerendered = self.plan_provider.render_plan().await?;
            ensure_same_plan(&evaluation.plan, &rerendered)?;
            verify_active_matches_plan(self.catalog.runtime_root(), &evaluation.plan)?;
            validate_launcher_surface(self.catalog.runtime_root(), self.catalog.bin_root())?;

            if tunnel_changed && previous_tunnel == TunnelState::Running {
                self.publish_phase(ReconciliationPhase::RestartingTunnel)
                    .await;
                self.runtime.restart_tunnel().await?;
            } else if gateway_changed {
                self.publish_phase(ReconciliationPhase::ReloadingGateway)
                    .await;
                self.runtime.reload_gateway().await?;
            }

            self.publish_phase(ReconciliationPhase::CatalogVerifying)
                .await;
            self.runtime.validate_tunnel_binding()?;
            let tool_names = self.runtime.catalog_tool_names().await?;
            let fingerprint = catalog_fingerprint(&tool_names);
            verify_active_matches_plan(self.catalog.runtime_root(), &evaluation.plan)?;
            let rerendered = self.plan_provider.render_plan().await?;
            ensure_same_plan(&evaluation.plan, &rerendered)?;
            let mut next = manifest_for_plan(
                &evaluation.plan,
                manifest.generation.saturating_add(1),
                Some(fingerprint.clone()),
            )?;
            next.studio_restart_pending = manifest.studio_restart_pending.clone();
            if studio_changed {
                let studio_surface = self.studio_surface(&evaluation.plan).ok_or_else(|| {
                    StudioError::UpdateVerificationFailed {
                        component: "reconciliation".into(),
                        detail: "Studio restart effect has no Studio config surface".into(),
                    }
                })?;
                next.studio_restart_pending = Some(StudioRestartPending {
                    sha256: studio_surface.sha256.clone(),
                    requested_from_process: self.studio_config_identity.process_instance.clone(),
                });
            }
            self.state_store
                .persist(self.catalog.runtime_root(), &next)?;
            Ok::<(String, ManagedStateManifest), StudioError>((fingerprint, next))
        }
        .await;

        match apply_result {
            Ok((fingerprint, next)) => {
                let cleanup_warning = remove_backup(&backup).err();
                let mut view = synchronized_view(
                    &evaluation.plan,
                    &next,
                    fingerprint,
                    if gateway_changed || tunnel_changed {
                        ClientFreshness::RefreshPending
                    } else {
                        ClientFreshness::Unknown
                    },
                    studio_changed,
                );
                if cleanup_warning.is_some() {
                    view.last_error = Some("reconciliation_cleanup_warning".into());
                }
                self.decorate_studio_activation(&mut view, &evaluation.plan);
                self.set_status(view.clone()).await;
                Ok(view)
            }
            Err(error) => {
                self.rollback(
                    &backup,
                    &manifest,
                    previous_tunnel,
                    gateway_changed,
                    tunnel_changed,
                    error,
                )
                .await
            }
        }
    }

    async fn rollback(
        &self,
        backup: &Backup,
        manifest: &ManagedStateManifest,
        previous_tunnel: TunnelState,
        gateway_changed: bool,
        tunnel_changed: bool,
        cause: StudioError,
    ) -> StudioResult<ReconciliationView> {
        self.publish_phase(ReconciliationPhase::Validating).await;
        let restore = async {
            restore_backup(self.catalog.runtime_root(), backup)?;
            self.state_store
                .persist(self.catalog.runtime_root(), manifest)?;
            self.runtime.restore_tunnel_state(previous_tunnel).await?;
            if tunnel_changed && previous_tunnel == TunnelState::Running {
                self.runtime.restart_tunnel().await?;
            } else if gateway_changed {
                self.runtime.reload_gateway().await?;
            }
            validate_manifest_bytes(self.catalog.runtime_root(), manifest)?;
            validate_launcher_surface(self.catalog.runtime_root(), self.catalog.bin_root())?;
            self.runtime.validate_tunnel_binding()?;
            let names = self.runtime.catalog_tool_names().await?;
            let fingerprint = catalog_fingerprint(&names);
            if let Some(expected) = &manifest.catalog_fingerprint
                && expected != &fingerprint
            {
                return Err(StudioError::RollbackFailed {
                    component: "reconciliation".into(),
                    detail: "restored Gateway catalog fingerprint mismatch".into(),
                });
            }
            Ok::<(), StudioError>(())
        }
        .await;

        match restore {
            Ok(()) => {
                remove_backup(backup)?;
                let evaluation = self.evaluate().await?;
                let mut view = evaluation.view;
                self.decorate_studio_activation(&mut view, &evaluation.plan);
                view.phase = ReconciliationPhase::Failed;
                view.rollback_succeeded = Some(true);
                view.last_error = Some(sanitize_reconciliation_error(&cause));
                self.set_status(view.clone()).await;
                Err(StudioError::UpdateTransaction(
                    "runtime reconciliation failed and was rolled back".into(),
                ))
            }
            Err(rollback_error) => {
                let mut view = self.status().await;
                view.state = ReconciliationState::Broken;
                view.phase = ReconciliationPhase::RollbackFailed;
                view.safe_to_reconcile = false;
                view.rollback_succeeded = Some(false);
                view.last_error = Some("rollback_failed".into());
                view.checked_at_ms = now_ms();
                self.set_status(view).await;
                Err(StudioError::RollbackFailed {
                    component: "reconciliation".into(),
                    detail: format!("{cause}; rollback verification failed: {rollback_error}"),
                })
            }
        }
    }

    async fn evaluate(&self) -> StudioResult<Evaluation> {
        let plan = self.plan_provider.render_plan().await?;
        let manifest = load_manifest(self.catalog.runtime_root())?;
        let launcher_error =
            validate_launcher_surface(self.catalog.runtime_root(), self.catalog.bin_root()).err();

        if let Some(state) = &manifest
            && (state.schema_version != MANAGED_SCHEMA_VERSION || state.host_id != plan.host_id)
        {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "managed reconciliation state identity/schema mismatch".into(),
            });
        }

        let mut affected = Vec::new();
        let mut unmanaged = launcher_error.is_some();
        if launcher_error.is_some() {
            affected.push("tunnel.launcher".to_owned());
        }

        let mut safe_drift = false;
        for desired in plan.outputs.values() {
            let active = read_active_surface(self.catalog.runtime_root(), desired)?;
            let desired_matches = active
                .as_deref()
                .is_some_and(|bytes| sha256_bytes(bytes) == desired.sha256);
            if desired_matches {
                continue;
            }
            affected.push(desired.surface.clone());

            let Some(state) = &manifest else {
                unmanaged = true;
                continue;
            };
            let Some(previous) = state.surfaces.get(&desired.surface) else {
                if active.is_none() {
                    safe_drift = true;
                } else {
                    unmanaged = true;
                }
                continue;
            };
            if previous.relative_path != path_to_string(&desired.relative_path)? {
                unmanaged = true;
                continue;
            }
            match active {
                Some(bytes) if sha256_bytes(&bytes) == previous.sha256 => safe_drift = true,
                None => safe_drift = true,
                _ => unmanaged = true,
            }
        }
        affected.sort();
        affected.dedup();

        if manifest.is_none() && !unmanaged && affected.is_empty() {
            let candidate = manifest_for_plan(&plan, 0, None)?;
            return Ok(Evaluation {
                view: synchronized_view(
                    &plan,
                    &candidate,
                    String::new(),
                    ClientFreshness::Unknown,
                    false,
                ),
                plan,
                manifest: None,
                legacy_adopt: true,
            });
        }

        if unmanaged {
            let view = ReconciliationView {
                host_id: Some(plan.host_id.clone()),
                state: ReconciliationState::UnmanagedConflict,
                phase: ReconciliationPhase::Failed,
                safe_to_reconcile: false,
                affected_surfaces: affected,
                gateway_reload_required: false,
                tunnel_restart_required: false,
                studio_restart_required: manifest
                    .as_ref()
                    .is_some_and(|state| state.studio_restart_pending.is_some()),
                studio_config_activation: if manifest
                    .as_ref()
                    .is_some_and(|state| state.studio_restart_pending.is_some())
                {
                    StudioConfigActivation::RestartRequired
                } else {
                    StudioConfigActivation::Unknown
                },
                catalog_fingerprint: manifest
                    .as_ref()
                    .and_then(|state| state.catalog_fingerprint.clone()),
                generation: manifest.as_ref().map_or(0, |state| state.generation),
                client_freshness: ClientFreshness::Unknown,
                rollback_succeeded: None,
                last_error: launcher_error
                    .as_ref()
                    .map(sanitize_reconciliation_error)
                    .or_else(|| Some("unmanaged_conflict".into())),
                checked_at_ms: now_ms(),
            };
            return Ok(Evaluation {
                view,
                plan,
                manifest,
                legacy_adopt: false,
            });
        }

        if safe_drift || !affected.is_empty() {
            let gateway_reload_required = plan.outputs.values().any(|surface| {
                affected.contains(&surface.surface) && surface.effects.contains("gateway_reload")
            });
            let tunnel_restart_required = plan.outputs.values().any(|surface| {
                affected.contains(&surface.surface) && surface.effects.contains("tunnel_restart")
            });
            let studio_restart_required = plan.outputs.values().any(|surface| {
                affected.contains(&surface.surface) && surface.effects.contains("studio_restart")
            });
            let view = ReconciliationView {
                host_id: Some(plan.host_id.clone()),
                state: ReconciliationState::ManagedSafeDrift,
                phase: ReconciliationPhase::ReconcileReady,
                safe_to_reconcile: true,
                affected_surfaces: affected,
                gateway_reload_required,
                tunnel_restart_required,
                studio_restart_required: studio_restart_required
                    || manifest
                        .as_ref()
                        .is_some_and(|state| state.studio_restart_pending.is_some()),
                studio_config_activation: if studio_restart_required
                    || manifest
                        .as_ref()
                        .is_some_and(|state| state.studio_restart_pending.is_some())
                {
                    StudioConfigActivation::RestartRequired
                } else {
                    StudioConfigActivation::Unknown
                },
                catalog_fingerprint: manifest
                    .as_ref()
                    .and_then(|state| state.catalog_fingerprint.clone()),
                generation: manifest.as_ref().map_or(0, |state| state.generation),
                client_freshness: ClientFreshness::Unknown,
                rollback_succeeded: None,
                last_error: None,
                checked_at_ms: now_ms(),
            };
            return Ok(Evaluation {
                view,
                plan,
                manifest,
                legacy_adopt: false,
            });
        }

        self.runtime.validate_tunnel_binding()?;
        let names = self.runtime.catalog_tool_names().await?;
        let fingerprint = catalog_fingerprint(&names);
        let state = manifest.ok_or_else(|| StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "synchronized runtime has no managed baseline".into(),
        })?;
        let freshness = if state.catalog_fingerprint.as_ref() == Some(&fingerprint) {
            ClientFreshness::Unknown
        } else {
            ClientFreshness::RefreshPending
        };
        let desired_manifest =
            manifest_for_plan(&plan, state.generation, state.catalog_fingerprint.clone())?;
        if state.surfaces != desired_manifest.surfaces {
            return Ok(Evaluation {
                view: ReconciliationView {
                    host_id: Some(plan.host_id.clone()),
                    state: ReconciliationState::UnmanagedConflict,
                    phase: ReconciliationPhase::Failed,
                    safe_to_reconcile: false,
                    affected_surfaces: vec!["managed_baseline".into()],
                    gateway_reload_required: false,
                    tunnel_restart_required: false,
                    studio_restart_required: state.studio_restart_pending.is_some(),
                    studio_config_activation: if state.studio_restart_pending.is_some() {
                        StudioConfigActivation::RestartRequired
                    } else {
                        StudioConfigActivation::Unknown
                    },
                    catalog_fingerprint: state.catalog_fingerprint.clone(),
                    generation: state.generation,
                    client_freshness: ClientFreshness::Unknown,
                    rollback_succeeded: None,
                    last_error: Some("managed_baseline_mismatch".into()),
                    checked_at_ms: now_ms(),
                },
                plan,
                manifest: Some(state),
                legacy_adopt: false,
            });
        }
        Ok(Evaluation {
            view: synchronized_view(&plan, &state, fingerprint, freshness, false),
            plan,
            manifest: Some(state),
            legacy_adopt: false,
        })
    }

    async fn publish_phase(&self, phase: ReconciliationPhase) {
        let mut view = self.status().await;
        view.phase = phase;
        view.checked_at_ms = now_ms();
        self.set_status(view).await;
    }

    async fn set_status(&self, view: ReconciliationView) {
        *self.status.write().await = view.clone();
        self.events
            .publish(StudioEvent::ReconciliationChanged { status: view });
    }

    fn acquire(&self) -> StudioResult<ReconciliationGuard> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| StudioError::UpdateTransaction("reconciliation lock poisoned".into()))?;
        if *active {
            return Err(StudioError::Conflict(
                "runtime reconciliation already in progress".into(),
            ));
        }
        *active = true;
        Ok(ReconciliationGuard {
            active: self.active.clone(),
        })
    }
}

struct ReconciliationGuard {
    active: Arc<StdMutex<bool>>,
}

impl Drop for ReconciliationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = false;
        }
    }
}

fn synchronized_view(
    plan: &DesiredPlan,
    manifest: &ManagedStateManifest,
    fingerprint: String,
    freshness: ClientFreshness,
    studio_restart_required: bool,
) -> ReconciliationView {
    ReconciliationView {
        host_id: Some(plan.host_id.clone()),
        state: ReconciliationState::Synchronized,
        phase: ReconciliationPhase::Synchronized,
        safe_to_reconcile: false,
        affected_surfaces: Vec::new(),
        gateway_reload_required: false,
        tunnel_restart_required: false,
        studio_restart_required: studio_restart_required
            || manifest.studio_restart_pending.is_some(),
        studio_config_activation: if studio_restart_required
            || manifest.studio_restart_pending.is_some()
        {
            StudioConfigActivation::RestartRequired
        } else {
            StudioConfigActivation::Unknown
        },
        catalog_fingerprint: Some(fingerprint),
        generation: manifest.generation,
        client_freshness: freshness,
        rollback_succeeded: None,
        last_error: None,
        checked_at_ms: now_ms(),
    }
}

fn manifest_for_plan(
    plan: &DesiredPlan,
    generation: u64,
    catalog_fingerprint: Option<String>,
) -> StudioResult<ManagedStateManifest> {
    let mut surfaces = BTreeMap::new();
    for desired in plan.outputs.values() {
        surfaces.insert(
            desired.surface.clone(),
            ManagedSurface {
                relative_path: path_to_string(&desired.relative_path)?,
                sha256: desired.sha256.clone(),
            },
        );
    }
    Ok(ManagedStateManifest {
        schema_version: MANAGED_SCHEMA_VERSION,
        host_id: plan.host_id.clone(),
        generation,
        surfaces,
        catalog_fingerprint,
        studio_restart_pending: None,
    })
}

fn manifest_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join(MANIFEST_RELATIVE)
}

fn load_manifest(runtime_root: &Path) -> StudioResult<Option<ManagedStateManifest>> {
    let path = manifest_path(runtime_root);
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed reconciliation state is not a regular file".into(),
        });
    }
    let manifest: ManagedStateManifest = serde_json::from_slice(&fs::read(path)?)?;
    Ok(Some(manifest))
}

fn persist_manifest(runtime_root: &Path, manifest: &ManagedStateManifest) -> StudioResult<()> {
    let fleet = runtime_root.join("fleet");
    let fleet_meta = fs::symlink_metadata(&fleet)?;
    if fleet_meta.file_type().is_symlink() || !fleet_meta.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet runtime root is not a regular directory".into(),
        ));
    }
    let state = fleet.join("state");
    if !state.exists() {
        fs::create_dir(&state)?;
    }
    let state_meta = fs::symlink_metadata(&state)?;
    if state_meta.file_type().is_symlink() || !state_meta.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet state root is not a regular directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    }

    let destination = manifest_path(runtime_root);
    let temp = state.join(format!(
        ".reconciliation.{}.{}.tmp",
        std::process::id(),
        now_ms()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&temp)?;
    let mut bytes = serde_json::to_vec_pretty(manifest)?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.flush()?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temp, &destination)?;
    File::open(&state)?.sync_all()?;
    Ok(())
}

fn changed_surfaces(runtime_root: &Path, plan: &DesiredPlan) -> StudioResult<Vec<DesiredSurface>> {
    let mut changed = Vec::new();
    for desired in plan.outputs.values() {
        let active = read_active_surface(runtime_root, desired)?;
        if active
            .as_deref()
            .is_none_or(|bytes| sha256_bytes(bytes) != desired.sha256)
        {
            changed.push(desired.clone());
        }
    }
    Ok(changed)
}

fn read_active_surface(
    runtime_root: &Path,
    desired: &DesiredSurface,
) -> StudioResult<Option<Vec<u8>>> {
    let target = trusted_surface_target(runtime_root, &desired.relative_path)?;
    if !target.exists() {
        validate_target_parent(runtime_root, &target)?;
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(&target)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface is not a regular file".into(),
        });
    }
    let canonical_root = fs::canonicalize(runtime_root)?;
    let canonical_target = fs::canonicalize(&target)?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface escapes runtime_root".into(),
        });
    }
    Ok(Some(fs::read(target)?))
}

fn trusted_surface_target(runtime_root: &Path, relative: &Path) -> StudioResult<PathBuf> {
    if relative.is_absolute()
        || relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface path is invalid".into(),
        });
    }
    if !expected_surfaces()
        .values()
        .any(|(expected, _)| expected == relative)
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface is not server-approved".into(),
        });
    }
    Ok(runtime_root.join(relative))
}

fn validate_target_parent(runtime_root: &Path, target: &Path) -> StudioResult<()> {
    let parent = target
        .parent()
        .ok_or_else(|| StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface has no parent".into(),
        })?;
    if !parent.exists() {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface parent is missing".into(),
        });
    }
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface parent is unsafe".into(),
        });
    }
    let root = fs::canonicalize(runtime_root)?;
    let parent = fs::canonicalize(parent)?;
    if !parent.starts_with(root) {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime surface parent escapes runtime_root".into(),
        });
    }
    Ok(())
}

fn atomic_write_surface(runtime_root: &Path, desired: &DesiredSurface) -> StudioResult<()> {
    let target = trusted_surface_target(runtime_root, &desired.relative_path)?;
    validate_target_parent(runtime_root, &target)?;
    let permissions = if target.exists() {
        let metadata = fs::symlink_metadata(&target)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateActivationFailed {
                component: "reconciliation".into(),
                detail: "managed runtime target is unsafe".into(),
            });
        }
        Some(metadata.permissions())
    } else {
        None
    };
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StudioError::UpdateActivationFailed {
            component: "reconciliation".into(),
            detail: "managed runtime target filename is invalid".into(),
        })?;
    let temp = target.with_file_name(format!(".{name}.{}.reconcile", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(&desired.bytes)?;
    file.flush()?;
    file.sync_all()?;
    if let Some(permissions) = permissions {
        fs::set_permissions(&temp, permissions)?;
    }
    fs::rename(&temp, &target)?;
    File::open(target.parent().expect("validated parent"))?.sync_all()?;
    Ok(())
}

fn verify_active_matches_plan(runtime_root: &Path, plan: &DesiredPlan) -> StudioResult<()> {
    for desired in plan.outputs.values() {
        let active = read_active_surface(runtime_root, desired)?.ok_or_else(|| {
            StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "managed runtime surface is missing after reconciliation".into(),
            }
        })?;
        if sha256_bytes(&active) != desired.sha256 {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "managed runtime surface differs after reconciliation".into(),
            });
        }
    }
    Ok(())
}

fn ensure_same_plan(expected: &DesiredPlan, actual: &DesiredPlan) -> StudioResult<()> {
    if expected.host_id != actual.host_id || expected.outputs.len() != actual.outputs.len() {
        return Err(StudioError::UpdateVerificationFailed {
            component: "reconciliation".into(),
            detail: "Fleet desired render changed during reconciliation".into(),
        });
    }
    for (surface, desired) in &expected.outputs {
        let Some(other) = actual.outputs.get(surface) else {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet desired render changed during reconciliation".into(),
            });
        };
        if desired.relative_path != other.relative_path
            || desired.sha256 != other.sha256
            || desired.effects != other.effects
        {
            return Err(StudioError::UpdateVerificationFailed {
                component: "reconciliation".into(),
                detail: "Fleet desired render changed during reconciliation".into(),
            });
        }
    }
    Ok(())
}

fn validate_manifest_bytes(
    runtime_root: &Path,
    manifest: &ManagedStateManifest,
) -> StudioResult<()> {
    for (surface, record) in &manifest.surfaces {
        let surfaces = expected_surfaces();
        let Some((expected_path, _)) = surfaces.get(surface.as_str()) else {
            return Err(StudioError::RollbackFailed {
                component: "reconciliation".into(),
                detail: "rollback manifest contains an unknown surface".into(),
            });
        };
        if record.relative_path != path_to_string(expected_path)? {
            return Err(StudioError::RollbackFailed {
                component: "reconciliation".into(),
                detail: "rollback manifest surface path mismatch".into(),
            });
        }
        let desired = DesiredSurface {
            surface: surface.clone(),
            relative_path: expected_path.clone(),
            bytes: Vec::new(),
            sha256: record.sha256.clone(),
            effects: BTreeSet::new(),
        };
        let bytes = read_active_surface(runtime_root, &desired)?.ok_or_else(|| {
            StudioError::RollbackFailed {
                component: "reconciliation".into(),
                detail: "rollback restored surface is missing".into(),
            }
        })?;
        if sha256_bytes(&bytes) != record.sha256 {
            return Err(StudioError::RollbackFailed {
                component: "reconciliation".into(),
                detail: "rollback restored surface digest mismatch".into(),
            });
        }
    }
    Ok(())
}

fn create_backup(runtime_root: &Path, changed: &[DesiredSurface]) -> StudioResult<Backup> {
    let state = runtime_root.join("fleet/state");
    if !state.exists() {
        fs::create_dir(&state)?;
    }
    let root = state.join(format!(
        "{BACKUP_PREFIX}{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }

    let mut entries = Vec::new();
    for (index, desired) in changed.iter().enumerate() {
        let target = trusted_surface_target(runtime_root, &desired.relative_path)?;
        let (bytes, permissions) = if target.exists() {
            let metadata = fs::symlink_metadata(&target)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StudioError::UpdateTransaction(
                    "cannot snapshot unsafe managed runtime target".into(),
                ));
            }
            (Some(fs::read(&target)?), Some(metadata.permissions()))
        } else {
            (None, None)
        };
        if let Some(bytes) = &bytes {
            let backup_file = root.join(format!("{index}.bin"));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup_file)?;
            file.write_all(bytes)?;
            file.flush()?;
            file.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&backup_file, fs::Permissions::from_mode(0o600))?;
            }
        }
        entries.push(SurfaceSnapshot {
            relative_path: desired.relative_path.clone(),
            bytes,
            permissions,
        });
    }
    File::open(&root)?.sync_all()?;
    Ok(Backup { root, entries })
}

fn restore_backup(runtime_root: &Path, backup: &Backup) -> StudioResult<()> {
    for entry in &backup.entries {
        let target = trusted_surface_target(runtime_root, &entry.relative_path)?;
        match &entry.bytes {
            Some(bytes) => {
                let desired = DesiredSurface {
                    surface: "rollback".into(),
                    relative_path: entry.relative_path.clone(),
                    bytes: bytes.clone(),
                    sha256: sha256_bytes(bytes),
                    effects: BTreeSet::new(),
                };
                atomic_write_surface(runtime_root, &desired)?;
                if let Some(permissions) = &entry.permissions {
                    fs::set_permissions(&target, permissions.clone())?;
                }
            }
            None => {
                if target.exists() {
                    let metadata = fs::symlink_metadata(&target)?;
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(StudioError::RollbackFailed {
                            component: "reconciliation".into(),
                            detail: "rollback target became unsafe".into(),
                        });
                    }
                    fs::remove_file(&target)?;
                    File::open(target.parent().expect("trusted target parent"))?.sync_all()?;
                }
            }
        }
    }
    Ok(())
}

fn remove_backup(backup: &Backup) -> StudioResult<()> {
    if backup.root.exists() {
        let metadata = fs::symlink_metadata(&backup.root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StudioError::UpdateTransaction(
                "reconciliation backup path is unsafe".into(),
            ));
        }
        fs::remove_dir_all(&backup.root)?;
    }
    Ok(())
}

fn validate_launcher_surface(runtime_root: &Path, bin_root: &Path) -> StudioResult<()> {
    let script = runtime_root.join("tunnel-client/run.sh");
    if !script.exists() && !script.is_symlink() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&script)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StudioError::Conflict(
            "validate-only tunnel launcher is not a regular file".into(),
        ));
    }
    let text = fs::read_to_string(&script)?;
    let runtime = runtime_root.join("tunnel-client");
    let expected_absolute = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nRUNTIME=\"{}\"\nCONFIG=\"{}\"\nGATEWAY=\"{}\"\nSERVERS_DIR=\"{}\"\nexec \"$RUNTIME\" run --config \"$CONFIG\" --mcp.command=\"command=$GATEWAY --config-dir $SERVERS_DIR,channel=main\" --control-plane.poll-channel=main\n",
        runtime
            .join("current/tunnel-client-runtime-cloudflared")
            .display(),
        runtime.join("config.yaml").display(),
        bin_root.join("rust-mcp-gateway").display(),
        runtime_root.join("gateway/servers.d").display(),
    );

    let mut supported = vec![expected_absolute];
    if let Some(workspace) = runtime_root.parent()
        && workspace.file_name().and_then(|name| name.to_str()) == Some("mcp-server")
        && bin_root == workspace.join("bin")
        && runtime_root == workspace.join("runtime")
    {
        supported.push(
            "#!/usr/bin/env bash\nset -euo pipefail\n\nSCRIPT_DIR=\"$(cd -- \"$(dirname -- \"${BASH_SOURCE[0]}\")\" && pwd)\"\nWORKSPACE_ROOT=\"$(cd -- \"$SCRIPT_DIR/../../..\" && pwd)\"\nRUNTIME=\"$SCRIPT_DIR/current/tunnel-client-runtime-cloudflared\"\nCONFIG=\"$SCRIPT_DIR/config.yaml\"\nGATEWAY=\"$WORKSPACE_ROOT/mcp-server/bin/rust-mcp-gateway\"\nSERVERS_DIR=\"$WORKSPACE_ROOT/mcp-server/runtime/gateway/servers.d\"\n\nfor path in \"$RUNTIME\" \"$GATEWAY\"; do\n  if [[ ! -x \"$path\" ]]; then\n    echo \"error: executable not found: $path\" >&2\n    exit 1\n  fi\ndone\n\nif [[ ! -f \"$CONFIG\" ]]; then\n  echo \"error: tunnel config not found: $CONFIG\" >&2\n  exit 1\nfi\n\nif [[ ! -d \"$SERVERS_DIR\" ]]; then\n  echo \"error: gateway servers directory not found: $SERVERS_DIR\" >&2\n  exit 1\nfi\n\n# Avoid stale shell/profile settings overriding the one authoritative main binding.\nunset MCP_COMMAND MCP_SERVER_URL CONTROL_PLANE_POLL_CHANNELS\n\nexec \"$RUNTIME\" run \\\n  --config \"$CONFIG\" \\\n  --mcp.command=\"command=$GATEWAY --config-dir $SERVERS_DIR,channel=main\" \\\n  --control-plane.poll-channel=main \\\n  \"$@\"\n"
                .to_owned(),
        );
    }

    if !supported.iter().any(|template| template == &text) {
        return Err(StudioError::Conflict(
            "validate-only tunnel launcher does not match a supported canonical template".into(),
        ));
    }
    Ok(())
}

fn catalog_fingerprint(names: &[String]) -> String {
    let mut names = names.to_vec();
    names.sort();
    names.dedup();
    let mut digest = Sha256::new();
    for name in names {
        digest.update(name.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn path_to_string(path: &Path) -> StudioResult<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| StudioError::UpdateTransaction("runtime path is not UTF-8".into()))
}

fn sanitize_reconciliation_error(error: &StudioError) -> String {
    match error {
        StudioError::Conflict(_) => "unmanaged_conflict".into(),
        StudioError::RollbackFailed { .. } => "rollback_failed".into(),
        StudioError::UpdateVerificationFailed { .. } => "validation_failed".into(),
        StudioError::UpdateActivationFailed { .. } => "activation_failed".into(),
        _ => sanitize_transaction_error(&error.to_string()),
    }
}

fn _unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as TestMutex;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        tunnel::TunnelConfig,
        update::{HostPlatform, HostRuntimeRoots},
    };

    #[derive(Clone)]
    struct FakePlanProvider {
        plan: Arc<TestMutex<DesiredPlan>>,
        fail: Arc<TestMutex<bool>>,
    }

    #[async_trait]
    impl RenderPlanProvider for FakePlanProvider {
        async fn render_plan(&self) -> StudioResult<DesiredPlan> {
            if *self.fail.lock().unwrap() {
                return Err(StudioError::UpdateVerificationFailed {
                    component: "reconciliation".into(),
                    detail: "forced render failure".into(),
                });
            }
            Ok(self.plan.lock().unwrap().clone())
        }
    }

    struct FakeRuntime {
        tunnel_state: TestMutex<TunnelState>,
        reload_count: TestMutex<u64>,
        restart_count: TestMutex<u64>,
        fail_reload: TestMutex<bool>,
        fail_restart: TestMutex<bool>,
        fail_probe_once: TestMutex<bool>,
        tool_names: TestMutex<Vec<String>>,
    }

    impl Default for FakeRuntime {
        fn default() -> Self {
            Self {
                tunnel_state: TestMutex::new(TunnelState::Stopped),
                reload_count: TestMutex::new(0),
                restart_count: TestMutex::new(0),
                fail_reload: TestMutex::new(false),
                fail_restart: TestMutex::new(false),
                fail_probe_once: TestMutex::new(false),
                tool_names: TestMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ReconciliationRuntime for FakeRuntime {
        async fn tunnel_state(&self) -> StudioResult<TunnelState> {
            Ok(*self.tunnel_state.lock().unwrap())
        }

        async fn reload_gateway(&self) -> StudioResult<()> {
            *self.reload_count.lock().unwrap() += 1;
            if *self.fail_reload.lock().unwrap() {
                return Err(StudioError::UpdateVerificationFailed {
                    component: "reconciliation".into(),
                    detail: "forced Gateway reload failure".into(),
                });
            }
            Ok(())
        }

        async fn restart_tunnel(&self) -> StudioResult<()> {
            *self.restart_count.lock().unwrap() += 1;
            if *self.fail_restart.lock().unwrap() {
                return Err(StudioError::UpdateVerificationFailed {
                    component: "reconciliation".into(),
                    detail: "forced tunnel restart failure".into(),
                });
            }
            *self.tunnel_state.lock().unwrap() = TunnelState::Running;
            Ok(())
        }

        async fn restore_tunnel_state(&self, previous: TunnelState) -> StudioResult<()> {
            *self.tunnel_state.lock().unwrap() = previous;
            Ok(())
        }

        fn validate_tunnel_binding(&self) -> StudioResult<()> {
            Ok(())
        }

        async fn catalog_tool_names(&self) -> StudioResult<Vec<String>> {
            let mut fail = self.fail_probe_once.lock().unwrap();
            if *fail {
                *fail = false;
                return Err(StudioError::UpdateVerificationFailed {
                    component: "reconciliation".into(),
                    detail: "forced catalog failure".into(),
                });
            }
            Ok(self.tool_names.lock().unwrap().clone())
        }
    }

    fn plan(root: &Path, suffix: &str) -> DesiredPlan {
        let definitions = [
            (
                "gateway.filesystem",
                "gateway/servers.d/filesystem.yaml",
                ["gateway_reload"].as_slice(),
            ),
            (
                "gateway.git",
                "gateway/servers.d/git.yaml",
                ["gateway_reload"].as_slice(),
            ),
            (
                "gateway.exec",
                "gateway/servers.d/exec.yaml",
                ["gateway_reload"].as_slice(),
            ),
            (
                "studio.config",
                "studio/studio.toml",
                ["studio_restart"].as_slice(),
            ),
            (
                "tunnel.config",
                "tunnel-client/config.yaml",
                ["tunnel_restart"].as_slice(),
            ),
        ];
        let mut outputs = BTreeMap::new();
        for (surface, relative, effects) in definitions {
            let bytes = format!("{surface}:{suffix}\n").into_bytes();
            outputs.insert(
                surface.to_owned(),
                DesiredSurface {
                    surface: surface.to_owned(),
                    relative_path: PathBuf::from(relative),
                    sha256: sha256_bytes(&bytes),
                    bytes,
                    effects: effects.iter().map(|value| (*value).to_owned()).collect(),
                },
            );
        }
        let _ = root;
        DesiredPlan {
            host_id: "aira".into(),
            outputs,
        }
    }

    fn write_plan(root: &Path, plan: &DesiredPlan) {
        for desired in plan.outputs.values() {
            let target = root.join("runtime").join(&desired.relative_path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, &desired.bytes).unwrap();
        }
    }

    fn write_valid_launcher(root: &Path) {
        let script = root.join("runtime/tunnel-client/run.sh");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(
            script,
            format!(
                "#!/usr/bin/env bash\nset -euo pipefail\nRUNTIME=\"{}\"\nCONFIG=\"{}\"\nGATEWAY=\"{}\"\nSERVERS_DIR=\"{}\"\nexec \"$RUNTIME\" run --config \"$CONFIG\" --mcp.command=\"command=$GATEWAY --config-dir $SERVERS_DIR,channel=main\" --control-plane.poll-channel=main\n",
                root.join("runtime/tunnel-client/current/tunnel-client-runtime-cloudflared").display(),
                root.join("runtime/tunnel-client/config.yaml").display(),
                root.join("bin/rust-mcp-gateway").display(),
                root.join("runtime/gateway/servers.d").display(),
            ),
        )
        .unwrap();
    }

    fn fixture() -> (
        TempDir,
        RuntimeReconciler,
        Arc<FakePlanProvider>,
        Arc<FakeRuntime>,
    ) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("runtime/fleet/state")).unwrap();
        fs::create_dir_all(root.join("runtime/fleet/hosts")).unwrap();
        let initial = plan(root, "v1");
        write_plan(root, &initial);
        write_valid_launcher(root);

        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.join("bin"), root.join("runtime")).unwrap(),
        );
        let provider = Arc::new(FakePlanProvider {
            plan: Arc::new(TestMutex::new(initial)),
            fail: Arc::new(TestMutex::new(false)),
        });
        let runtime = Arc::new(FakeRuntime {
            tunnel_state: TestMutex::new(TunnelState::Running),
            tool_names: TestMutex::new(vec!["a".into(), "b".into(), "c".into()]),
            ..FakeRuntime::default()
        });
        let mut reconciler = RuntimeReconciler::with_dependencies(
            catalog,
            provider.clone(),
            runtime.clone(),
            EventHub::default(),
        );
        let studio = reconciler
            .studio_surface(&provider.plan.lock().unwrap())
            .unwrap()
            .clone();
        reconciler.studio_config_identity = StudioProcessConfigIdentity {
            canonical_path: Some(
                fs::canonicalize(root.join("runtime/studio/studio.toml")).unwrap(),
            ),
            sha256: Some(studio.sha256),
            process_instance: "proc-1".into(),
        };
        (temp, reconciler, provider, runtime)
    }

    struct FailingStateStore {
        calls: std::sync::atomic::AtomicUsize,
        fail_calls: BTreeSet<usize>,
    }

    impl FailingStateStore {
        fn new(fail_calls: impl IntoIterator<Item = usize>) -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                fail_calls: fail_calls.into_iter().collect(),
            }
        }
    }

    impl ReconciliationStateStore for FailingStateStore {
        fn persist(
            &self,
            runtime_root: &Path,
            manifest: &ManagedStateManifest,
        ) -> StudioResult<()> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if self.fail_calls.contains(&call) {
                return Err(StudioError::Io(std::io::Error::other(format!(
                    "injected reconciliation manifest write failure #{call}"
                ))));
            }
            persist_manifest(runtime_root, manifest)
        }
    }

    #[tokio::test]
    async fn synchronized_legacy_runtime_is_safely_adopted() {
        let (temp, reconciler, _provider, _runtime) = fixture();
        let view = reconciler.check().await.unwrap();
        assert_eq!(view.state, ReconciliationState::Synchronized);
        assert_eq!(view.generation, 1);
        assert!(
            temp.path()
                .join("runtime/fleet/state/reconciliation.json")
                .is_file()
        );
    }

    #[tokio::test]
    async fn managed_drift_is_repaired_and_effects_are_bounded() {
        let (temp, reconciler, provider, runtime) = fixture();
        assert_eq!(
            reconciler.check().await.unwrap().state,
            ReconciliationState::Synchronized
        );

        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        let drift = reconciler.check().await.unwrap();
        assert_eq!(drift.state, ReconciliationState::ManagedSafeDrift);
        assert!(drift.safe_to_reconcile);
        assert!(drift.gateway_reload_required);
        assert!(drift.tunnel_restart_required);
        assert!(drift.studio_restart_required);

        let result = reconciler.apply().await.unwrap();
        assert_eq!(result.state, ReconciliationState::Synchronized);
        assert_eq!(*runtime.restart_count.lock().unwrap(), 1);
        assert_eq!(*runtime.reload_count.lock().unwrap(), 0);
        assert!(result.studio_restart_required);
        for desired in provider.plan.lock().unwrap().outputs.values() {
            assert_eq!(
                fs::read(temp.path().join("runtime").join(&desired.relative_path)).unwrap(),
                desired.bytes
            );
        }
    }

    #[tokio::test]
    async fn unmanaged_edit_and_launcher_divergence_fail_closed_until_adopted() {
        let (temp, reconciler, provider, _runtime) = fixture();
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        fs::write(
            temp.path().join("runtime/gateway/servers.d/exec.yaml"),
            b"operator edit\n",
        )
        .unwrap();
        let conflict = reconciler.check().await.unwrap();
        assert_eq!(conflict.state, ReconciliationState::UnmanagedConflict);
        assert!(!conflict.safe_to_reconcile);
        assert!(reconciler.apply().await.is_err());

        let launcher = temp.path().join("runtime/tunnel-client/run.sh");
        let text = fs::read_to_string(&launcher).unwrap();
        fs::write(
            &launcher,
            text.replace(
                "/bin/rust-mcp-gateway",
                "/gateway/target/release/rust-mcp-gateway",
            ),
        )
        .unwrap();
        let conflict = reconciler.check().await.unwrap();
        assert_eq!(conflict.state, ReconciliationState::UnmanagedConflict);
        assert!(
            conflict
                .affected_surfaces
                .contains(&"tunnel.launcher".into())
        );
    }

    #[tokio::test]
    async fn explicit_adoption_records_unknown_bytes_without_rewriting() {
        let (temp, reconciler, provider, _runtime) = fixture();
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        let target = temp.path().join("runtime/gateway/servers.d/exec.yaml");
        fs::write(&target, b"operator edit\n").unwrap();
        let before = fs::read(&target).unwrap();
        assert_eq!(
            reconciler.check().await.unwrap().state,
            ReconciliationState::UnmanagedConflict
        );
        let adopted = reconciler.adopt().await.unwrap();
        assert_eq!(adopted.state, ReconciliationState::ManagedSafeDrift);
        assert_eq!(fs::read(&target).unwrap(), before);
    }

    #[tokio::test]
    async fn post_write_catalog_failure_rolls_back_bytes_and_runtime_state() {
        let (temp, reconciler, provider, runtime) = fixture();
        reconciler.check().await.unwrap();
        let old = provider.plan.lock().unwrap().clone();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        *runtime.fail_probe_once.lock().unwrap() = true;
        assert!(reconciler.apply().await.is_err());
        for desired in old.outputs.values() {
            assert_eq!(
                fs::read(temp.path().join("runtime").join(&desired.relative_path)).unwrap(),
                desired.bytes
            );
        }
        assert_eq!(*runtime.tunnel_state.lock().unwrap(), TunnelState::Running);
        let status = reconciler.status().await;
        assert_eq!(status.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn stopped_tunnel_remains_stopped_when_tunnel_config_changes() {
        let (temp, reconciler, provider, runtime) = fixture();
        *runtime.tunnel_state.lock().unwrap() = TunnelState::Stopped;
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        reconciler.apply().await.unwrap();
        assert_eq!(*runtime.restart_count.lock().unwrap(), 0);
        assert_eq!(*runtime.tunnel_state.lock().unwrap(), TunnelState::Stopped);
    }

    #[tokio::test]
    async fn gateway_only_drift_uses_watcher_reload_without_tunnel_restart() {
        let (temp, reconciler, provider, runtime) = fixture();
        assert_eq!(
            reconciler.check().await.unwrap().state,
            ReconciliationState::Synchronized
        );

        let mut next = provider.plan.lock().unwrap().clone();
        let exec = next.outputs.get_mut("gateway.exec").unwrap();
        exec.bytes = b"gateway.exec:v2\n".to_vec();
        exec.sha256 = sha256_bytes(&exec.bytes);
        *provider.plan.lock().unwrap() = next;

        let drift = reconciler.check().await.unwrap();
        assert_eq!(drift.state, ReconciliationState::ManagedSafeDrift);
        assert!(drift.gateway_reload_required);
        assert!(!drift.tunnel_restart_required);
        assert!(!drift.studio_restart_required);

        let result = reconciler.apply().await.unwrap();
        assert_eq!(result.state, ReconciliationState::Synchronized);
        assert_eq!(*runtime.reload_count.lock().unwrap(), 1);
        assert_eq!(*runtime.restart_count.lock().unwrap(), 0);
        assert_eq!(
            fs::read(temp.path().join("runtime/gateway/servers.d/exec.yaml")).unwrap(),
            b"gateway.exec:v2\n"
        );
    }

    #[tokio::test]
    async fn equal_count_catalog_identity_change_is_visible_as_refresh_pending() {
        let (_temp, reconciler, _provider, runtime) = fixture();
        let first = reconciler.check().await.unwrap();
        assert_eq!(first.state, ReconciliationState::Synchronized);
        let first_fingerprint = first.catalog_fingerprint.unwrap();

        *runtime.tool_names.lock().unwrap() = vec!["a".into(), "b".into(), "d".into()];
        let second = reconciler.check().await.unwrap();
        assert_eq!(second.state, ReconciliationState::Synchronized);
        assert_eq!(second.client_freshness, ClientFreshness::RefreshPending);
        assert_ne!(
            second.catalog_fingerprint.as_deref(),
            Some(first_fingerprint.as_str())
        );
    }

    #[tokio::test]
    async fn equal_tool_counts_have_distinct_catalog_fingerprints() {
        let left = vec!["a".into(), "b".into(), "c".into()];
        let right = vec!["a".into(), "b".into(), "d".into()];
        assert_eq!(left.len(), right.len());
        assert_ne!(catalog_fingerprint(&left), catalog_fingerprint(&right));
    }

    #[tokio::test]
    #[ignore = "explicit M5.12 incident-derived reconciliation closure drill"]
    async fn m5_12_incident_reconciliation_closure_drill() {
        let (temp, reconciler, provider, runtime) = fixture();

        // Baseline is a trusted managed legacy generation. The helper launcher is
        // already canonical, so process startup could look healthy even while
        // generated files later become stale.
        let baseline = reconciler.check().await.unwrap();
        assert_eq!(baseline.state, ReconciliationState::Synchronized);
        assert_eq!(baseline.generation, 1);
        let baseline_fingerprint = baseline.catalog_fingerprint.clone().unwrap();
        assert_eq!(*runtime.tunnel_state.lock().unwrap(), TunnelState::Running);

        // Desired Fleet output advances. Active tunnel YAML and Exec child config
        // intentionally remain on the prior managed generation, which models the
        // incident class: stale canonical config masked by a correct launcher.
        let mut desired = provider.plan.lock().unwrap().clone();
        for key in ["tunnel.config", "gateway.exec"] {
            let surface = desired.outputs.get_mut(key).unwrap();
            surface.bytes = format!("{key}:canonical-flat-v2\n").into_bytes();
            surface.sha256 = sha256_bytes(&surface.bytes);
        }
        *provider.plan.lock().unwrap() = desired.clone();

        let drift = reconciler.check().await.unwrap();
        assert_eq!(drift.state, ReconciliationState::ManagedSafeDrift);
        assert!(drift.safe_to_reconcile);
        assert!(drift.gateway_reload_required);
        assert!(drift.tunnel_restart_required);
        assert!(drift.affected_surfaces.contains(&"gateway.exec".into()));
        assert!(drift.affected_surfaces.contains(&"tunnel.config".into()));

        let repaired = reconciler.apply().await.unwrap();
        assert_eq!(repaired.state, ReconciliationState::Synchronized);
        assert_eq!(*runtime.tunnel_state.lock().unwrap(), TunnelState::Running);
        assert_eq!(*runtime.restart_count.lock().unwrap(), 1);
        assert_eq!(
            fs::read(temp.path().join("runtime/gateway/servers.d/exec.yaml")).unwrap(),
            desired.outputs["gateway.exec"].bytes
        );
        assert_eq!(
            fs::read(temp.path().join("runtime/tunnel-client/config.yaml")).unwrap(),
            desired.outputs["tunnel.config"].bytes
        );
        assert!(repaired.catalog_fingerprint.is_some());

        // Equal-count catalog identity changes must not be hidden by the count.
        *runtime.tool_names.lock().unwrap() = vec!["a".into(), "b".into(), "d".into()];
        let identity_changed = reconciler.check().await.unwrap();
        assert_eq!(identity_changed.state, ReconciliationState::Synchronized);
        assert_eq!(
            identity_changed.client_freshness,
            ClientFreshness::RefreshPending
        );
        assert_ne!(
            identity_changed.catalog_fingerprint.as_deref(),
            Some(baseline_fingerprint.as_str())
        );
        *runtime.tool_names.lock().unwrap() = vec!["a".into(), "b".into(), "c".into()];
        let identity_restored = reconciler.check().await.unwrap();
        assert_eq!(identity_restored.state, ReconciliationState::Synchronized);
        assert_eq!(
            identity_restored.catalog_fingerprint.as_deref(),
            Some(baseline_fingerprint.as_str())
        );

        // Force a post-write catalog verification failure. The old managed bytes
        // and running tunnel ownership must be restored before rollback succeeds.
        let repaired_exec =
            fs::read(temp.path().join("runtime/gateway/servers.d/exec.yaml")).unwrap();
        let repaired_tunnel =
            fs::read(temp.path().join("runtime/tunnel-client/config.yaml")).unwrap();
        let mut next = desired.clone();
        for key in ["tunnel.config", "gateway.exec"] {
            let surface = next.outputs.get_mut(key).unwrap();
            surface.bytes = format!("{key}:canonical-flat-v3\n").into_bytes();
            surface.sha256 = sha256_bytes(&surface.bytes);
        }
        *provider.plan.lock().unwrap() = next;
        *runtime.fail_probe_once.lock().unwrap() = true;
        assert!(reconciler.apply().await.is_err());
        assert_eq!(
            fs::read(temp.path().join("runtime/gateway/servers.d/exec.yaml")).unwrap(),
            repaired_exec
        );
        assert_eq!(
            fs::read(temp.path().join("runtime/tunnel-client/config.yaml")).unwrap(),
            repaired_tunnel
        );
        assert_eq!(*runtime.tunnel_state.lock().unwrap(), TunnelState::Running);
        assert_eq!(reconciler.status().await.rollback_succeeded, Some(true));

        // Unknown local edits stay fail-closed and are never silently overwritten.
        let exec_path = temp.path().join("runtime/gateway/servers.d/exec.yaml");
        fs::write(&exec_path, b"operator-owned edit\n").unwrap();
        let before = fs::read(&exec_path).unwrap();
        let conflict = reconciler.check().await.unwrap();
        assert_eq!(conflict.state, ReconciliationState::UnmanagedConflict);
        assert!(!conflict.safe_to_reconcile);
        assert!(reconciler.apply().await.is_err());
        assert_eq!(fs::read(exec_path).unwrap(), before);

        // Stopped ownership is preserved in a separate closure leg.
        let (stopped_temp, stopped_reconciler, stopped_provider, stopped_runtime) = fixture();
        *stopped_runtime.tunnel_state.lock().unwrap() = TunnelState::Stopped;
        assert_eq!(
            stopped_reconciler.check().await.unwrap().state,
            ReconciliationState::Synchronized
        );
        *stopped_provider.plan.lock().unwrap() = plan(stopped_temp.path(), "v2");
        let stopped_result = stopped_reconciler.apply().await.unwrap();
        assert_eq!(stopped_result.state, ReconciliationState::Synchronized);
        assert_eq!(
            *stopped_runtime.tunnel_state.lock().unwrap(),
            TunnelState::Stopped
        );
        assert_eq!(*stopped_runtime.restart_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    #[ignore = "explicit Aira runtime-only reconciliation smoke using copied runtime artifacts"]
    async fn live_aira_runtime_only_reconciliation_smoke() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let bin_root = root.join("bin");
        let runtime_root = root.join("runtime");
        let fleet_root = runtime_root.join("fleet");
        fs::create_dir_all(&bin_root).unwrap();
        fs::create_dir_all(fleet_root.join("scripts")).unwrap();
        fs::create_dir_all(fleet_root.join("hosts")).unwrap();
        fs::create_dir_all(runtime_root.join("gateway/servers.d")).unwrap();
        fs::create_dir_all(runtime_root.join("studio")).unwrap();
        fs::create_dir_all(runtime_root.join("tunnel-client")).unwrap();

        for binary in [
            "rust-mcp-filesystem",
            "rust-mcp-git",
            "rust-mcp-exec",
            "rust-mcp-gateway",
        ] {
            fs::copy(PathBuf::from("../bin").join(binary), bin_root.join(binary)).unwrap();
        }
        fs::copy(
            "../fleet/scripts/fleetctl.py",
            fleet_root.join("scripts/fleetctl.py"),
        )
        .unwrap();
        fs::copy("../fleet/fleet.toml", fleet_root.join("fleet.toml")).unwrap();

        let host_profile = format!(
            r#"schema_version = 1
host_id = "aira"
workspace_root = "{}"
source_root = "{}"
bin_root = "{}"
runtime_root = "{}"

[gateway]
server_dir = "gateway/servers.d"

[servers.filesystem]
enabled = true
timeout_ms = 30000
tool_allowlist = []

[servers.git]
enabled = true
timeout_ms = 30000
extra_args = ["--allow-remote-read"]
tool_allowlist = []

[servers.exec]
enabled = true
timeout_ms = 30000
tool_allowlist = []
"#,
            root.display(),
            root.join("missing-source").display(),
            bin_root.display(),
            runtime_root.display(),
        );
        fs::write(fleet_root.join("hosts/aira.toml"), host_profile).unwrap();

        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(bin_root.clone(), runtime_root.clone()).unwrap(),
        );
        let provider = FleetRenderPlanProvider {
            catalog: catalog.clone(),
        };
        let desired = provider.render_plan().await.unwrap();
        assert_eq!(desired.outputs.len(), 5);
        write_plan(root, &desired);
        write_valid_launcher(root);

        let tunnel_runtime = runtime_root.join("tunnel-client/tunnel-client-runtime-cloudflared");
        fs::copy(
            "../runtime/tunnel-client/current/tunnel-client-runtime-cloudflared",
            &tunnel_runtime,
        )
        .unwrap();

        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.join("missing-source"),
                HostPlatform::from_raw("Darwin", "x86_64")
                    .unwrap()
                    .platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let tunnel = Arc::new(TunnelSupervisor::new(
            TunnelConfig {
                name: "Smoke tunnel".into(),
                runtime: tunnel_runtime,
                working_dir: runtime_root.join("tunnel-client"),
                config_file: runtime_root.join("tunnel-client/config.yaml"),
                env: BTreeMap::new(),
            },
            32,
            Duration::from_millis(100),
            root.to_path_buf(),
            EventHub::default(),
        ));
        let reconciler =
            RuntimeReconciler::new(catalog, tunnel, inventory, EventHub::default()).unwrap();

        assert!(!root.join("missing-source").exists());
        let view = reconciler.check().await.unwrap();
        assert_eq!(view.state, ReconciliationState::Synchronized);
        assert_eq!(view.generation, 1);
        assert!(view.catalog_fingerprint.is_some());
        assert_eq!(view.client_freshness, ClientFreshness::Unknown);
        assert!(
            runtime_root
                .join("fleet/state/reconciliation.json")
                .is_file()
        );
    }

    #[tokio::test]
    async fn render_failure_is_broken_without_mutating_runtime() {
        let (temp, reconciler, provider, _runtime) = fixture();
        let target = temp.path().join("runtime/gateway/servers.d/exec.yaml");
        let before = fs::read(&target).unwrap();
        *provider.fail.lock().unwrap() = true;
        let view = reconciler.check().await.unwrap();
        assert_eq!(view.state, ReconciliationState::Broken);
        assert_eq!(fs::read(&target).unwrap(), before);
    }

    #[tokio::test]
    async fn manifest_commit_failure_restores_files_and_baseline() {
        let (temp, mut reconciler, provider, _runtime) = fixture();
        let baseline = reconciler.check().await.unwrap();
        assert_eq!(baseline.generation, 1);
        let original_manifest = fs::read(manifest_path(&temp.path().join("runtime"))).unwrap();
        let old = provider.plan.lock().unwrap().clone();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        reconciler.state_store = Arc::new(FailingStateStore::new([1]));
        assert!(reconciler.apply().await.is_err());
        assert_eq!(
            fs::read(manifest_path(&temp.path().join("runtime"))).unwrap(),
            original_manifest
        );
        for surface in old.outputs.values() {
            assert_eq!(
                fs::read(temp.path().join("runtime").join(&surface.relative_path)).unwrap(),
                surface.bytes
            );
        }
        assert_eq!(reconciler.status().await.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn rollback_manifest_failure_is_explicit_and_retains_backup() {
        let (temp, mut reconciler, provider, _runtime) = fixture();
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        reconciler.state_store = Arc::new(FailingStateStore::new([1, 2]));
        assert!(matches!(
            reconciler.apply().await.unwrap_err(),
            StudioError::RollbackFailed { .. }
        ));
        assert_eq!(
            reconciler.status().await.phase,
            ReconciliationPhase::RollbackFailed
        );
        let backups = fs::read_dir(temp.path().join("runtime/fleet/state"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(BACKUP_PREFIX)
            })
            .count();
        assert!(
            backups >= 1,
            "rollback diagnostics were removed after rollback failure"
        );
    }

    #[tokio::test]
    async fn unexpected_external_edit_before_mutation_fails_closed() {
        let (temp, reconciler, provider, runtime) = fixture();
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        let mut gated = RuntimeReconciler::with_dependencies(
            reconciler.catalog.clone(),
            provider.clone(),
            runtime,
            EventHub::default(),
        );
        gated.state_store = reconciler.state_store.clone();
        // Reuse the committed manifest while changing a managed byte after evaluation
        // would otherwise classify it from stale input. The apply pre-mutation hash
        // check must reject the edit before overwriting it.
        let target = temp.path().join("runtime/gateway/servers.d/exec.yaml");
        fs::write(&target, b"external edit\n").unwrap();
        assert!(gated.apply().await.is_err());
        assert_eq!(fs::read(target).unwrap(), b"external edit\n");
    }

    #[test]
    fn launcher_contract_rejects_comments_duplicates_wrong_root_and_symlink() {
        let (temp, _, _, _) = fixture();
        let runtime = temp.path().join("runtime");
        let bin = temp.path().join("bin");
        let script = runtime.join("tunnel-client/run.sh");
        let valid = fs::read_to_string(&script).unwrap();

        fs::write(&script, format!("# canonical-looking comment\n{valid}")).unwrap();
        assert!(validate_launcher_surface(&runtime, &bin).is_err());

        fs::write(
            &script,
            valid.replace(
                "--control-plane.poll-channel=main",
                "--config /tmp/override --control-plane.poll-channel=main",
            ),
        )
        .unwrap();
        assert!(validate_launcher_surface(&runtime, &bin).is_err());

        fs::write(
            &script,
            valid.replace(
                &format!("GATEWAY=\"{}\"", bin.join("rust-mcp-gateway").display()),
                "GATEWAY=\"/tmp/wrong-gateway\"",
            ),
        )
        .unwrap();
        assert!(validate_launcher_surface(&runtime, &bin).is_err());

        fs::remove_file(&script).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp/not-a-launcher", &script).unwrap();
            assert!(validate_launcher_surface(&runtime, &bin).is_err());
        }
    }

    #[tokio::test]
    async fn studio_restart_pending_clears_only_after_new_process_loads_expected_config() {
        let (temp, mut reconciler, provider, _) = fixture();
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        let applied = reconciler.apply().await.unwrap();
        assert!(applied.studio_restart_required);
        assert_eq!(
            applied.studio_config_activation,
            StudioConfigActivation::RestartRequired
        );

        // A new process with old bytes cannot clear the pending restart.
        reconciler.studio_config_identity.process_instance = "proc-2".into();
        reconciler.studio_config_identity.sha256 = Some(sha256_bytes(b"studio.config:v1\n"));
        let old_bytes = reconciler.check().await.unwrap();
        assert!(old_bytes.studio_restart_required);

        // Correct bytes from the wrong config path also cannot clear it.
        let wrong = temp.path().join("runtime/studio/other.toml");
        fs::write(&wrong, b"studio.config:v2\n").unwrap();
        reconciler.studio_config_identity.canonical_path = Some(fs::canonicalize(&wrong).unwrap());
        reconciler.studio_config_identity.sha256 = Some(sha256_bytes(b"studio.config:v2\n"));
        let wrong_path = reconciler.check().await.unwrap();
        assert!(wrong_path.studio_restart_required);

        // A new process proving the canonical path and exact reconciled bytes clears it.
        reconciler.studio_config_identity.canonical_path =
            Some(fs::canonicalize(temp.path().join("runtime/studio/studio.toml")).unwrap());
        let cleared = reconciler.check().await.unwrap();
        assert!(!cleared.studio_restart_required);
        assert_eq!(
            cleared.studio_config_activation,
            StudioConfigActivation::Active
        );
    }

    #[tokio::test]
    async fn gateway_only_reconciliation_does_not_create_studio_restart_pending() {
        let (_temp, reconciler, provider, _) = fixture();
        let initial = reconciler.check().await.unwrap();
        assert!(!initial.studio_restart_required);
        let mut next = provider.plan.lock().unwrap().clone();
        let exec = next.outputs.get_mut("gateway.exec").unwrap();
        exec.bytes = b"gateway.exec:gateway-only\n".to_vec();
        exec.sha256 = sha256_bytes(&exec.bytes);
        *provider.plan.lock().unwrap() = next;
        let drift = reconciler.check().await.unwrap();
        assert!(!drift.studio_restart_required);
        let applied = reconciler.apply().await.unwrap();
        assert!(!applied.studio_restart_required);
        assert_eq!(
            applied.studio_config_activation,
            StudioConfigActivation::Active
        );
    }

    #[test]
    fn audit_launcher_shadowing_must_fail_closed() {
        let (temp, _, _, _) = fixture();
        let script = temp.path().join("runtime/tunnel-client/run.sh");
        let text = fs::read_to_string(&script).unwrap();
        fs::write(
            &script,
            text.replace(
                "exec \"$RUNTIME\"",
                "GATEWAY=/usr/bin/false\nexec \"$RUNTIME\"",
            ),
        )
        .unwrap();
        assert!(
            validate_launcher_surface(&temp.path().join("runtime"), &temp.path().join("bin"))
                .is_err(),
            "canonical assignment is shadowed before exec, but launcher validation accepts it"
        );
    }

    #[tokio::test]
    async fn audit_check_must_respect_mutation_guard() {
        let (temp, reconciler, _, _) = fixture();
        let _guard = reconciler.acquire().unwrap();
        assert!(matches!(
            reconciler.check().await,
            Err(StudioError::Conflict(_))
        ));
        assert!(
            !manifest_path(&temp.path().join("runtime")).exists(),
            "check persisted a manifest while the mutation guard was held"
        );
    }

    #[tokio::test]
    async fn audit_studio_restart_flag_must_survive_check() {
        let (temp, reconciler, provider, _) = fixture();
        reconciler.check().await.unwrap();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        assert!(reconciler.apply().await.unwrap().studio_restart_required);
        assert!(
            reconciler.check().await.unwrap().studio_restart_required,
            "no Studio restart occurred, but check cleared restart_required"
        );
    }

    struct AuditGatedRuntime {
        inner: Arc<FakeRuntime>,
        block_once: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait]
    impl ReconciliationRuntime for AuditGatedRuntime {
        async fn tunnel_state(&self) -> StudioResult<TunnelState> {
            self.inner.tunnel_state().await
        }
        async fn reload_gateway(&self) -> StudioResult<()> {
            self.inner.reload_gateway().await
        }
        async fn restart_tunnel(&self) -> StudioResult<()> {
            if self
                .block_once
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                self.entered.notify_one();
                self.release.notified().await;
                return Err(StudioError::Process(
                    "audit injected restart failure after concurrent check".into(),
                ));
            }
            self.inner.restart_tunnel().await
        }
        async fn restore_tunnel_state(&self, state: TunnelState) -> StudioResult<()> {
            self.inner.restore_tunnel_state(state).await
        }
        fn validate_tunnel_binding(&self) -> StudioResult<()> {
            self.inner.validate_tunnel_binding()
        }
        async fn catalog_tool_names(&self) -> StudioResult<Vec<String>> {
            self.inner.catalog_tool_names().await
        }
    }
    #[tokio::test]
    async fn audit_concurrent_check_must_not_poison_rollback_baseline() {
        let (temp, mut reconciler, provider, runtime) = fixture();
        reconciler.check().await.unwrap();
        let old = provider.plan.lock().unwrap().clone();
        *provider.plan.lock().unwrap() = plan(temp.path(), "v2");
        let gated = Arc::new(AuditGatedRuntime {
            inner: runtime,
            block_once: std::sync::atomic::AtomicBool::new(true),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        reconciler.runtime = gated.clone();
        let reconciler = Arc::new(reconciler);
        let worker = reconciler.clone();
        let task = tokio::spawn(async move { worker.apply().await });
        tokio::time::timeout(Duration::from_secs(5), gated.entered.notified())
            .await
            .unwrap();
        assert!(matches!(
            reconciler.check().await,
            Err(StudioError::Conflict(_))
        ));
        gated.release.notify_one();
        assert!(task.await.unwrap().is_err());
        for surface in old.outputs.values() {
            assert_eq!(
                fs::read(temp.path().join("runtime").join(&surface.relative_path)).unwrap(),
                surface.bytes
            );
        }
        assert_eq!(
            reconciler.status().await.state,
            ReconciliationState::ManagedSafeDrift,
            "rollback restored v1 files but concurrent check persisted v2 managed fingerprints"
        );
    }
}
