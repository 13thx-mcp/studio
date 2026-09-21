use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex as StdMutex},
};

use async_trait::async_trait;
use semver::Version as SemverVersion;
use serde::{Deserialize, Serialize};

use crate::error::{StudioError, StudioResult};

mod fleet;
mod gateway;
mod github_http;
mod inventory;
mod openai;
mod platform;
mod reconciliation;
mod self_update;
mod staging;
mod thirteenthx;
mod transaction;
mod tunnel_update;
pub use fleet::FleetUpdateManager;
pub use gateway::GatewayUpdateManager;
pub use inventory::{
    CheckStatus, HostMode, InventoryEntry, InventoryHealth, InventoryService, InventoryView,
    ReleaseCheckView, RunningIdentityProvider, RuntimeInstalledIdentityProvider,
    StudioRunningIdentityProvider,
};
pub use openai::{
    OpenAiTunnelReleaseProvider, TUNNEL_RUNTIME_ASSET_PREFIX, TUNNEL_RUNTIME_BINARY_NAME,
};
pub use platform::{
    HostPlatform, expected_asset_name, normalize_arch, normalize_os, select_release_asset,
};
pub use reconciliation::{
    ClientFreshness, ReconciliationPhase, ReconciliationState, ReconciliationView,
    RuntimeReconciler, StudioConfigActivation, StudioProcessConfigIdentity,
};
pub use self_update::{SelfUpdateManager, write_activation_ready_proof_from_env};
pub use staging::{ArtifactStager, StagedArtifact};
pub use thirteenthx::ThirteenthXReleaseProvider;
pub(crate) use transaction::ArtifactHistoryIdentity;
pub use transaction::{
    ApplyMcpUpdateRequest, McpUpdateManager, McpUpdatePhase, McpUpdateTransactionView,
    PrepareMcpUpdateRequest,
};
pub use tunnel_update::TunnelUpdateManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentId {
    Filesystem,
    Git,
    Exec,
    Gateway,
    Blender,
    Studio,
    Fleet,
    Tunnel,
}

impl ComponentId {
    pub const ALL: [Self; 8] = [
        Self::Filesystem,
        Self::Git,
        Self::Exec,
        Self::Gateway,
        Self::Blender,
        Self::Studio,
        Self::Fleet,
        Self::Tunnel,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Filesystem => "filesystem",
            Self::Git => "git",
            Self::Exec => "exec",
            Self::Gateway => "gateway",
            Self::Blender => "blender",
            Self::Studio => "studio",
            Self::Fleet => "fleet",
            Self::Tunnel => "tunnel",
        }
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ComponentId {
    type Err = StudioError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "filesystem" => Ok(Self::Filesystem),
            "git" => Ok(Self::Git),
            "exec" => Ok(Self::Exec),
            "gateway" => Ok(Self::Gateway),
            "blender" => Ok(Self::Blender),
            "studio" => Ok(Self::Studio),
            "fleet" => Ok(Self::Fleet),
            "tunnel" => Ok(Self::Tunnel),
            _ => Err(StudioError::UnknownComponent(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentClass {
    McpBinary,
    Service,
    ControlBundle,
    UpstreamRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReleaseProviderId {
    #[serde(rename = "github_13thx")]
    ThirteenthXGitHub,
    #[serde(rename = "github_openai")]
    OpenAiGitHub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseSource {
    pub owner: &'static str,
    pub repository: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallTarget {
    BinRootBinary { binary: &'static str },
    RuntimeBundle { directory: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseAssetKind {
    PlatformTarGz,
    ArchitectureIndependentTarGz,
    PlatformZip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseAssetPolicy {
    pub stem: &'static str,
    pub kind: ReleaseAssetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    PreserveMcpState,
    ExternalSupervisor,
    NoManagedRestart,
    PreserveTunnelState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentPolicy {
    pub id: ComponentId,
    pub display_name: &'static str,
    pub class: ComponentClass,
    pub provider: ReleaseProviderId,
    pub source: ReleaseSource,
    pub install_target: InstallTarget,
    pub asset: ReleaseAssetPolicy,
    pub channel: UpdateChannel,
    pub restart_policy: RestartPolicy,
}

/// Server-side roots supplied by trusted host/runtime configuration. Browser
/// requests never carry either root or a derived install path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRuntimeRoots {
    pub bin_root: PathBuf,
    pub runtime_root: PathBuf,
}

impl HostRuntimeRoots {
    pub fn new(bin_root: PathBuf, runtime_root: PathBuf) -> StudioResult<Self> {
        if bin_root.as_os_str().is_empty() || runtime_root.as_os_str().is_empty() {
            return Err(StudioError::Config(
                "component bin_root and runtime_root must not be empty".into(),
            ));
        }
        Ok(Self {
            bin_root,
            runtime_root,
        })
    }

    fn install_path(&self, policy: &ComponentPolicy) -> PathBuf {
        match policy.install_target {
            InstallTarget::BinRootBinary { binary } => self.bin_root.join(binary),
            InstallTarget::RuntimeBundle { directory } => self.runtime_root.join(directory),
        }
    }
}

/// Trusted component policy catalog. It is intentionally constructed on the
/// server and contains the only repository/provider/install-target mappings
/// accepted by the update domain.
#[derive(Debug, Default)]
struct RuntimeOperationState {
    control_owner: Option<&'static str>,
    components: BTreeMap<ComponentId, &'static str>,
}

/// Shared in-process coordinator for runtime mutations. Acquisition order is
/// fixed: coordinator lease first, then a manager-local transaction lease.
/// Control-path operations are exclusive with every component mutation;
/// ordinary MCP component updates may proceed concurrently for distinct IDs.
#[derive(Debug, Default)]
pub struct RuntimeOperationCoordinator {
    state: StdMutex<RuntimeOperationState>,
}

impl RuntimeOperationCoordinator {
    pub fn acquire_control(
        self: &Arc<Self>,
        owner: &'static str,
    ) -> StudioResult<RuntimeOperationGuard> {
        let mut state = self.state.lock().map_err(|_| {
            StudioError::UpdateTransaction("runtime operation lock poisoned".into())
        })?;
        if let Some(active) = state.control_owner {
            return Err(StudioError::Conflict(format!(
                "runtime control operation busy: {active}"
            )));
        }
        if !state.components.is_empty() {
            return Err(StudioError::Conflict(
                "runtime component update is in progress".into(),
            ));
        }
        state.control_owner = Some(owner);
        Ok(RuntimeOperationGuard {
            coordinator: self.clone(),
            lease: RuntimeOperationLease::Control(owner),
        })
    }

    pub fn acquire_component(
        self: &Arc<Self>,
        component: ComponentId,
        owner: &'static str,
    ) -> StudioResult<RuntimeOperationGuard> {
        let mut state = self.state.lock().map_err(|_| {
            StudioError::UpdateTransaction("runtime operation lock poisoned".into())
        })?;
        if let Some(active) = state.control_owner {
            return Err(StudioError::Conflict(format!(
                "runtime control operation busy: {active}"
            )));
        }
        if state.components.contains_key(&component) {
            return Err(StudioError::Conflict(format!(
                "{component}: runtime component update already in progress"
            )));
        }
        state.components.insert(component, owner);
        Ok(RuntimeOperationGuard {
            coordinator: self.clone(),
            lease: RuntimeOperationLease::Component(component),
        })
    }
}

#[derive(Debug)]
enum RuntimeOperationLease {
    Control(&'static str),
    Component(ComponentId),
}

#[derive(Debug)]
pub struct RuntimeOperationGuard {
    coordinator: Arc<RuntimeOperationCoordinator>,
    lease: RuntimeOperationLease,
}

impl Drop for RuntimeOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.coordinator.state.lock() {
            match self.lease {
                RuntimeOperationLease::Control(owner) => {
                    if state.control_owner == Some(owner) {
                        state.control_owner = None;
                    }
                }
                RuntimeOperationLease::Component(component) => {
                    state.components.remove(&component);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComponentCatalog {
    roots: HostRuntimeRoots,
    components: BTreeMap<ComponentId, ComponentPolicy>,
    runtime_operations: Arc<RuntimeOperationCoordinator>,
}

impl ComponentCatalog {
    pub fn new(roots: HostRuntimeRoots) -> Self {
        let components = component_policies()
            .into_iter()
            .map(|policy| (policy.id, policy))
            .collect();
        Self {
            roots,
            components,
            runtime_operations: Arc::new(RuntimeOperationCoordinator::default()),
        }
    }

    pub fn component(&self, id: ComponentId) -> StudioResult<&ComponentPolicy> {
        self.components
            .get(&id)
            .ok_or_else(|| StudioError::UnknownComponent(id.to_string()))
    }

    pub fn component_by_str(&self, id: &str) -> StudioResult<&ComponentPolicy> {
        self.component(id.parse()?)
    }

    pub fn list_components(&self) -> impl Iterator<Item = &ComponentPolicy> {
        self.components.values()
    }

    pub fn install_path(&self, id: ComponentId) -> StudioResult<PathBuf> {
        Ok(self.roots.install_path(self.component(id)?))
    }

    pub fn bin_root(&self) -> &Path {
        &self.roots.bin_root
    }

    pub fn runtime_root(&self) -> &Path {
        &self.roots.runtime_root
    }

    pub fn runtime_operations(&self) -> Arc<RuntimeOperationCoordinator> {
        self.runtime_operations.clone()
    }

    pub fn provider_for(&self, id: ComponentId) -> StudioResult<ReleaseProviderId> {
        Ok(self.component(id)?.provider)
    }

    pub fn public_view(&self, id: ComponentId) -> StudioResult<ComponentView> {
        Ok(ComponentView::from(self.component(id)?))
    }
}

fn component_policies() -> [ComponentPolicy; 8] {
    [
        project_mcp(
            ComponentId::Filesystem,
            "Filesystem MCP",
            "filesystem",
            "rust-mcp-filesystem",
        ),
        project_mcp(ComponentId::Git, "Git MCP", "git", "rust-mcp-git"),
        project_mcp(ComponentId::Exec, "Exec MCP", "exec", "rust-mcp-exec"),
        project_mcp(
            ComponentId::Gateway,
            "Gateway MCP",
            "gateway",
            "rust-mcp-gateway",
        ),
        project_mcp(
            ComponentId::Blender,
            "Blender MCP",
            "blender",
            "rust-mcp-blender",
        ),
        ComponentPolicy {
            id: ComponentId::Studio,
            display_name: "MCP Studio",
            class: ComponentClass::Service,
            provider: ReleaseProviderId::ThirteenthXGitHub,
            source: ReleaseSource {
                owner: "13thx-mcp",
                repository: "studio",
            },
            install_target: InstallTarget::RuntimeBundle {
                directory: "studio",
            },
            asset: ReleaseAssetPolicy {
                stem: "mcp-studio",
                kind: ReleaseAssetKind::PlatformTarGz,
            },
            channel: UpdateChannel::Stable,
            restart_policy: RestartPolicy::ExternalSupervisor,
        },
        ComponentPolicy {
            id: ComponentId::Fleet,
            display_name: "Fleet control bundle",
            class: ComponentClass::ControlBundle,
            provider: ReleaseProviderId::ThirteenthXGitHub,
            source: ReleaseSource {
                owner: "13thx-mcp",
                repository: "fleet",
            },
            install_target: InstallTarget::RuntimeBundle { directory: "fleet" },
            asset: ReleaseAssetPolicy {
                stem: "mcp-fleet",
                kind: ReleaseAssetKind::ArchitectureIndependentTarGz,
            },
            channel: UpdateChannel::Stable,
            restart_policy: RestartPolicy::NoManagedRestart,
        },
        ComponentPolicy {
            id: ComponentId::Tunnel,
            display_name: "OpenAI tunnel-client",
            class: ComponentClass::UpstreamRuntime,
            provider: ReleaseProviderId::OpenAiGitHub,
            source: ReleaseSource {
                owner: "openai",
                repository: "tunnel-client",
            },
            install_target: InstallTarget::RuntimeBundle {
                directory: "tunnel-client",
            },
            asset: ReleaseAssetPolicy {
                stem: TUNNEL_RUNTIME_ASSET_PREFIX,
                kind: ReleaseAssetKind::PlatformZip,
            },
            channel: UpdateChannel::Stable,
            restart_policy: RestartPolicy::PreserveTunnelState,
        },
    ]
}

fn project_mcp(
    id: ComponentId,
    display_name: &'static str,
    repository: &'static str,
    binary: &'static str,
) -> ComponentPolicy {
    ComponentPolicy {
        id,
        display_name,
        class: ComponentClass::McpBinary,
        provider: ReleaseProviderId::ThirteenthXGitHub,
        source: ReleaseSource {
            owner: "13thx-mcp",
            repository,
        },
        install_target: InstallTarget::BinRootBinary { binary },
        asset: ReleaseAssetPolicy {
            stem: binary,
            kind: ReleaseAssetKind::PlatformTarGz,
        },
        channel: UpdateChannel::Stable,
        restart_policy: RestartPolicy::PreserveMcpState,
    }
}

/// Validated semantic-version identity. Release tags may be parsed through
/// `parse_tag`, which accepts the repository convention `vX.Y.Z`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Version(SemverVersion);

impl Version {
    pub fn parse(value: &str) -> StudioResult<Self> {
        SemverVersion::parse(value).map(Self).map_err(|error| {
            StudioError::Config(format!("invalid semantic version {value:?}: {error}"))
        })
    }

    pub fn parse_tag(value: &str) -> StudioResult<Self> {
        Self::parse(value.strip_prefix('v').unwrap_or(value))
    }

    pub fn as_semver(&self) -> &SemverVersion {
        &self.0
    }

    pub fn cmp_precedence(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp_precedence(&other.0)
    }

    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.cmp_precedence(other).is_gt()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for Version {
    type Err = StudioError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingSystem {
    Darwin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    Arm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub os: OperatingSystem,
    pub arch: Architecture,
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let os = match self.os {
            OperatingSystem::Darwin => "darwin",
        };
        let arch = match self.arch {
            Architecture::Arm64 => "arm64",
        };
        write!(formatter, "{os}-{arch}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledArtifactIdentity {
    pub component: ComponentId,
    pub version: Version,
    pub platform: Option<Platform>,
    pub sha256: Option<String>,
    pub install_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableRelease {
    pub component: ComponentId,
    pub version: Version,
    pub tag: String,
    pub assets: Vec<ReleaseAsset>,
    pub checksum_manifest_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackMetadata {
    pub previous: InstalledArtifactIdentity,
    pub prepared_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Idle,
    Checking,
    Available,
    Downloading,
    Verifying,
    Staging,
    PreparingRuntime,
    Activating,
    HealthVerifying,
    Current,
    DownloadFailed,
    VerifyFailed,
    StageFailed,
    StopFailed,
    ActivateFailed,
    HealthFailed,
    RollbackRequired,
    RollbackFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateTransaction {
    pub component: ComponentId,
    pub state: UpdateState,
    pub source_version: Option<Version>,
    pub target_version: Option<Version>,
    pub rollback: Option<RollbackMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftState {
    Current,
    UpdateAvailable,
    InstalledRestartRequired,
    Drifted,
    Unknown,
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationHealth {
    Healthy,
    Unknown,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDimensions {
    pub installed_version: Option<Version>,
    pub running_version: Option<Version>,
    pub desired_version: Option<Version>,
    pub latest_version: Option<Version>,
    pub installation_health: InstallationHealth,
}

impl Default for VersionDimensions {
    fn default() -> Self {
        Self {
            installed_version: None,
            running_version: None,
            desired_version: None,
            latest_version: None,
            installation_health: InstallationHealth::Unknown,
        }
    }
}

impl VersionDimensions {
    pub fn drift_state(&self) -> DriftState {
        match self.installation_health {
            InstallationHealth::Broken => return DriftState::Broken,
            InstallationHealth::Unknown => return DriftState::Unknown,
            InstallationHealth::Healthy => {}
        }

        let (Some(installed), Some(running), Some(desired)) = (
            self.installed_version.as_ref(),
            self.running_version.as_ref(),
            self.desired_version.as_ref(),
        ) else {
            return DriftState::Unknown;
        };

        if installed != desired {
            return DriftState::Drifted;
        }
        if running != installed {
            return DriftState::InstalledRestartRequired;
        }
        if self
            .latest_version
            .as_ref()
            .is_some_and(|latest| latest.is_newer_than(installed))
        {
            return DriftState::UpdateAvailable;
        }
        DriftState::Current
    }
}

/// Deliberately browser-safe projection. Internal repository ownership,
/// release URLs, checksum URLs, install paths, and binary names stay server-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentView {
    pub id: ComponentId,
    pub display_name: &'static str,
    pub class: ComponentClass,
    pub provider: ReleaseProviderId,
    pub channel: UpdateChannel,
    pub restart_policy: RestartPolicy,
}

impl From<&ComponentPolicy> for ComponentView {
    fn from(policy: &ComponentPolicy) -> Self {
        Self {
            id: policy.id,
            display_name: policy.display_name,
            class: policy.class,
            provider: policy.provider,
            channel: policy.channel,
            restart_policy: policy.restart_policy,
        }
    }
}

/// Minimal browser mutation selector for future update actions. Unknown fields
/// are rejected so request payloads cannot smuggle release/path authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRequest {
    pub component: ComponentId,
}

/// Read-only provider boundary. M5.2/M5.3 supply concrete implementations;
/// M5.1 intentionally performs no network access.
#[async_trait]
pub trait ReleaseProvider: Send + Sync {
    fn provider_id(&self) -> ReleaseProviderId;

    async fn latest_release(&self, component: &ComponentPolicy) -> StudioResult<AvailableRelease>;

    async fn release(
        &self,
        component: &ComponentPolicy,
        version: &Version,
    ) -> StudioResult<AvailableRelease>;

    fn select_asset<'a>(
        &self,
        release: &'a AvailableRelease,
        platform: Platform,
    ) -> StudioResult<&'a ReleaseAsset>;

    async fn checksum_manifest(&self, release: &AvailableRelease) -> StudioResult<Vec<u8>>;
}

/// Installed identity is also abstracted from the catalog so future inventory
/// logic can inspect artifacts/runtime state without accepting caller paths.
#[async_trait]
pub trait InstalledIdentityProvider: Send + Sync {
    async fn installed_identity(
        &self,
        component: &ComponentPolicy,
        install_path: &Path,
    ) -> StudioResult<Option<InstalledArtifactIdentity>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ComponentCatalog {
        ComponentCatalog::new(
            HostRuntimeRoots::new(
                PathBuf::from("/trusted/bin"),
                PathBuf::from("/trusted/runtime"),
            )
            .unwrap(),
        )
    }

    #[test]
    fn runtime_operation_coordinator_serializes_control_and_preserves_component_concurrency() {
        let catalog = catalog();
        let coordinator = catalog.runtime_operations();
        let git = coordinator
            .acquire_component(ComponentId::Git, "git_test")
            .unwrap();
        let exec = coordinator
            .acquire_component(ComponentId::Exec, "exec_test")
            .unwrap();
        assert!(
            coordinator
                .acquire_component(ComponentId::Git, "duplicate")
                .is_err()
        );
        assert!(coordinator.acquire_control("control").is_err());
        drop(exec);
        drop(git);
        let control = coordinator.acquire_control("control").unwrap();
        assert!(
            coordinator
                .acquire_component(ComponentId::Git, "blocked")
                .is_err()
        );
        assert!(coordinator.acquire_control("duplicate_control").is_err());
        drop(control);
        assert!(
            coordinator
                .acquire_component(ComponentId::Git, "after_control")
                .is_ok()
        );
    }

    #[test]
    fn catalog_contains_expected_current_components() {
        let catalog = catalog();
        let ids = catalog
            .list_components()
            .map(|component| component.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ComponentId::ALL);
    }

    #[test]
    fn unknown_component_is_rejected() {
        let error = catalog().component_by_str("unknown").unwrap_err();
        assert!(matches!(error, StudioError::UnknownComponent(id) if id == "unknown"));
    }

    #[test]
    fn component_class_mapping_is_stable() {
        let catalog = catalog();
        for id in [
            ComponentId::Filesystem,
            ComponentId::Git,
            ComponentId::Exec,
            ComponentId::Gateway,
            ComponentId::Blender,
        ] {
            assert_eq!(
                catalog.component(id).unwrap().class,
                ComponentClass::McpBinary
            );
        }
        assert_eq!(
            catalog.component(ComponentId::Studio).unwrap().class,
            ComponentClass::Service
        );
        assert_eq!(
            catalog.component(ComponentId::Fleet).unwrap().class,
            ComponentClass::ControlBundle
        );
        assert_eq!(
            catalog.component(ComponentId::Tunnel).unwrap().class,
            ComponentClass::UpstreamRuntime
        );
    }

    #[test]
    fn semantic_versions_parse_and_order_by_semver_rules() {
        let stable = Version::parse_tag("v1.2.3").unwrap();
        assert_eq!(stable.to_string(), "1.2.3");
        assert!(
            Version::parse("1.10.0")
                .unwrap()
                .is_newer_than(&Version::parse("1.9.9").unwrap())
        );
        assert!(stable.is_newer_than(&Version::parse("1.2.3-rc.1").unwrap()));
        assert!(
            Version::parse("1.2.3-rc.10")
                .unwrap()
                .is_newer_than(&Version::parse("1.2.3-rc.2").unwrap())
        );
        assert_eq!(
            Version::parse("1.2.3+build.2")
                .unwrap()
                .cmp_precedence(&Version::parse("1.2.3+build.1").unwrap()),
            std::cmp::Ordering::Equal
        );
        assert!(Version::parse("1.02.3").is_err());
        assert!(Version::parse("1.2").is_err());
    }

    #[test]
    fn update_state_serializes_with_stable_names() {
        assert_eq!(
            serde_json::to_string(&UpdateState::HealthVerifying).unwrap(),
            "\"health_verifying\""
        );
        assert_eq!(
            serde_json::to_string(&UpdateState::RollbackRequired).unwrap(),
            "\"rollback_required\""
        );
    }

    #[test]
    fn drift_state_derivation_is_deterministic() {
        let v1 = Version::parse("1.0.0").unwrap();
        let v2 = Version::parse("1.1.0").unwrap();

        assert_eq!(
            VersionDimensions {
                installed_version: Some(v1.clone()),
                running_version: Some(v1.clone()),
                desired_version: Some(v1.clone()),
                latest_version: Some(v1.clone()),
                installation_health: InstallationHealth::Healthy,
            }
            .drift_state(),
            DriftState::Current
        );
        assert_eq!(
            VersionDimensions {
                installed_version: Some(v1.clone()),
                running_version: Some(v1.clone()),
                desired_version: Some(v1.clone()),
                latest_version: Some(v2.clone()),
                installation_health: InstallationHealth::Healthy,
            }
            .drift_state(),
            DriftState::UpdateAvailable
        );
        assert_eq!(
            VersionDimensions {
                installed_version: Some(v2.clone()),
                running_version: Some(v1.clone()),
                desired_version: Some(v2.clone()),
                latest_version: Some(v2.clone()),
                installation_health: InstallationHealth::Healthy,
            }
            .drift_state(),
            DriftState::InstalledRestartRequired
        );
        assert_eq!(
            VersionDimensions {
                installed_version: Some(v1.clone()),
                running_version: Some(v1.clone()),
                desired_version: Some(v2.clone()),
                latest_version: Some(v2.clone()),
                installation_health: InstallationHealth::Healthy,
            }
            .drift_state(),
            DriftState::Drifted
        );
        assert_eq!(
            VersionDimensions {
                installed_version: Some(v1.clone()),
                running_version: Some(v1),
                desired_version: Some(v2.clone()),
                latest_version: Some(v2),
                installation_health: InstallationHealth::Broken,
            }
            .drift_state(),
            DriftState::Broken
        );
        assert_eq!(
            VersionDimensions::default().drift_state(),
            DriftState::Unknown
        );
    }

    #[test]
    fn provider_selection_comes_from_trusted_catalog() {
        let catalog = catalog();
        assert_eq!(
            catalog.provider_for(ComponentId::Filesystem).unwrap(),
            ReleaseProviderId::ThirteenthXGitHub
        );
        assert_eq!(
            catalog.provider_for(ComponentId::Tunnel).unwrap(),
            ReleaseProviderId::OpenAiGitHub
        );
        assert_eq!(
            catalog.component(ComponentId::Tunnel).unwrap().source,
            ReleaseSource {
                owner: "openai",
                repository: "tunnel-client"
            }
        );
    }

    #[test]
    fn trusted_roots_derive_flat_and_runtime_install_paths() {
        let catalog = catalog();
        assert_eq!(
            catalog.install_path(ComponentId::Git).unwrap(),
            PathBuf::from("/trusted/bin/rust-mcp-git")
        );
        assert_eq!(
            catalog.install_path(ComponentId::Studio).unwrap(),
            PathBuf::from("/trusted/runtime/studio")
        );
        assert_eq!(
            catalog.install_path(ComponentId::Tunnel).unwrap(),
            PathBuf::from("/trusted/runtime/tunnel-client")
        );
    }

    #[test]
    fn browser_component_request_accepts_only_known_component_identity() {
        let request: ComponentRequest = serde_json::from_str(r#"{"component":"git"}"#).unwrap();
        assert_eq!(request.component, ComponentId::Git);
        assert!(serde_json::from_str::<ComponentRequest>(r#"{"component":"unknown"}"#).is_err());
        assert!(
            serde_json::from_str::<ComponentRequest>(
                r#"{"component":"git","download_url":"https://example.invalid/a"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ComponentRequest>(
                r#"{"component":"git","install_path":"/tmp/evil"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn browser_safe_component_view_omits_internal_urls_paths_and_secrets() {
        let catalog = catalog();
        let json = serde_json::to_value(catalog.public_view(ComponentId::Tunnel).unwrap()).unwrap();
        let object = json.as_object().unwrap();
        for forbidden in [
            "repository",
            "owner",
            "download_url",
            "checksum_manifest_url",
            "install_path",
            "binary",
            "secret",
        ] {
            assert!(
                !object.contains_key(forbidden),
                "unexpected {forbidden} field"
            );
        }
        assert_eq!(object.get("id").unwrap(), "tunnel");
        assert_eq!(object.get("provider").unwrap(), "github_openai");
    }

    #[test]
    fn platform_identity_has_stable_target_names() {
        assert_eq!(
            Platform {
                os: OperatingSystem::Darwin,
                arch: Architecture::Arm64,
            }
            .to_string(),
            "darwin-arm64"
        );
    }
}
