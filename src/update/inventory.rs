use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::Serialize;
use tokio::{process::Command, sync::RwLock, task::JoinSet, time::timeout};

use crate::error::{StudioError, StudioResult};

use super::{
    ComponentCatalog, ComponentClass, ComponentId, ComponentPolicy, DriftState, InstallationHealth,
    InstalledArtifactIdentity, InstalledIdentityProvider, OpenAiTunnelReleaseProvider, Platform,
    ReleaseProvider, ReleaseProviderId, ThirteenthXReleaseProvider, Version, VersionDimensions,
};

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostMode {
    SourcePresent,
    RuntimeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub component: ComponentId,
    pub display_name: &'static str,
    pub class: ComponentClass,
    pub provider: ReleaseProviderId,
    pub repository_owner: &'static str,
    pub repository_name: &'static str,
    pub install_path: PathBuf,
    pub platform: Platform,
    pub source_present: bool,
    pub installed_version: Option<Version>,
    pub running_version: Option<Version>,
    pub desired_version: Option<Version>,
    pub latest_version: Option<Version>,
    pub installation_health: InstallationHealth,
    pub drift: DriftState,
    pub last_check: ReleaseCheckState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReleaseCheckState {
    pub checked_at_ms: Option<u128>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseCheckView {
    pub checked_at_ms: Option<u128>,
    pub status: CheckStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Never,
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InventoryView {
    pub component: ComponentId,
    pub display_name: &'static str,
    pub class: ComponentClass,
    pub provider: ReleaseProviderId,
    pub platform: Platform,
    pub host_mode: HostMode,
    pub source_present: bool,
    pub installed_version: Option<Version>,
    pub running_version: Option<Version>,
    pub desired_version: Option<Version>,
    pub latest_version: Option<Version>,
    pub update_available: bool,
    pub installation_health: InventoryHealth,
    pub drift: DriftState,
    pub last_check: ReleaseCheckView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryHealth {
    Healthy,
    Broken,
    Unknown,
}

impl From<InstallationHealth> for InventoryHealth {
    fn from(value: InstallationHealth) -> Self {
        match value {
            InstallationHealth::Healthy => Self::Healthy,
            InstallationHealth::Broken => Self::Broken,
            InstallationHealth::Unknown => Self::Unknown,
        }
    }
}

impl From<&InventoryEntry> for InventoryView {
    fn from(entry: &InventoryEntry) -> Self {
        let status = match (&entry.last_check.checked_at_ms, &entry.last_check.error) {
            (None, _) => CheckStatus::Never,
            (Some(_), None) => CheckStatus::Ok,
            (Some(_), Some(_)) => CheckStatus::Error,
        };
        let update_available = entry
            .installed_version
            .as_ref()
            .zip(entry.latest_version.as_ref())
            .is_some_and(|(installed, latest)| latest.is_newer_than(installed));
        Self {
            component: entry.component,
            display_name: entry.display_name,
            class: entry.class,
            provider: entry.provider,
            platform: entry.platform,
            host_mode: if entry.source_present {
                HostMode::SourcePresent
            } else {
                HostMode::RuntimeOnly
            },
            source_present: entry.source_present,
            installed_version: entry.installed_version.clone(),
            running_version: entry.running_version.clone(),
            desired_version: entry.desired_version.clone(),
            latest_version: entry.latest_version.clone(),
            update_available,
            installation_health: entry.installation_health.into(),
            drift: entry.drift,
            last_check: ReleaseCheckView {
                checked_at_ms: entry.last_check.checked_at_ms,
                status,
                error: entry.last_check.error.clone(),
            },
        }
    }
}

#[async_trait]
pub trait RunningIdentityProvider: Send + Sync {
    async fn running_version(&self, component: &ComponentPolicy) -> StudioResult<Option<Version>>;
}

#[derive(Default)]
pub struct StudioRunningIdentityProvider;

#[async_trait]
impl RunningIdentityProvider for StudioRunningIdentityProvider {
    async fn running_version(&self, component: &ComponentPolicy) -> StudioResult<Option<Version>> {
        if component.id == ComponentId::Studio {
            return Ok(Some(Version::parse(env!("CARGO_PKG_VERSION"))?));
        }
        Ok(None)
    }
}

#[derive(Default)]
pub struct RuntimeInstalledIdentityProvider;

#[async_trait]
impl InstalledIdentityProvider for RuntimeInstalledIdentityProvider {
    async fn installed_identity(
        &self,
        component: &ComponentPolicy,
        install_path: &Path,
    ) -> StudioResult<Option<InstalledArtifactIdentity>> {
        let version = match component.id {
            ComponentId::Filesystem
            | ComponentId::Git
            | ComponentId::Exec
            | ComponentId::Gateway
            | ComponentId::Blender => {
                probe_binary_version(install_path, BinaryVersionFormat::Named).await?
            }
            ComponentId::Studio => probe_studio_identity(install_path).await?,
            ComponentId::Fleet => probe_fleet_version(install_path)?,
            ComponentId::Tunnel => probe_tunnel_identity(install_path).await?,
        };
        Ok(version.map(|version| InstalledArtifactIdentity {
            component: component.id,
            version,
            platform: None,
            sha256: None,
            install_path: install_path.to_owned(),
        }))
    }
}

#[derive(Clone, Copy)]
enum BinaryVersionFormat {
    Named,
    Leading,
}

async fn probe_binary_version(
    path: &Path,
    format: BinaryVersionFormat,
) -> StudioResult<Option<Version>> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(StudioError::InstalledIdentity(format!(
            "installed artifact is not a file: {}",
            path.display()
        )));
    }
    let mut command = Command::new(path);
    command.arg("--version").env_clear().kill_on_drop(true);
    let output = timeout(VERSION_PROBE_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            StudioError::InstalledIdentity(format!("version probe timed out: {}", path.display()))
        })?
        .map_err(|error| {
            StudioError::InstalledIdentity(format!(
                "version probe failed for {}: {error}",
                path.display()
            ))
        })?;
    if !output.status.success() {
        return Err(StudioError::InstalledIdentity(format!(
            "version probe failed for {} with status {}",
            path.display(),
            output.status
        )));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|_| {
        StudioError::InstalledIdentity(format!(
            "version probe output is not UTF-8: {}",
            path.display()
        ))
    })?;
    let value = stdout.trim();
    let token = match format {
        BinaryVersionFormat::Named => value.split_whitespace().last(),
        BinaryVersionFormat::Leading => value.split_whitespace().next(),
    }
    .ok_or_else(|| {
        StudioError::InstalledIdentity(format!("version probe output is empty: {}", path.display()))
    })?;
    Version::parse(token).map(Some).map_err(|error| {
        StudioError::InstalledIdentity(format!(
            "invalid version output from {}: {error}",
            path.display()
        ))
    })
}

async fn probe_studio_identity(install_path: &Path) -> StudioResult<Option<Version>> {
    if !install_path.exists() {
        return Ok(None);
    }
    let current = install_path.join("current");
    if current.exists() || current.is_symlink() {
        if !current.is_symlink() {
            return Err(StudioError::InstalledIdentity(
                "Studio current activation pointer is not a symlink".into(),
            ));
        }
        let target = fs::read_link(&current)?;
        if target.is_absolute() {
            return Err(StudioError::InstalledIdentity(
                "Studio current activation pointer must be relative".into(),
            ));
        }
        let components = target.components().collect::<Vec<_>>();
        if components.len() != 2
            || components[0].as_os_str() != "releases"
            || !components[1]
                .as_os_str()
                .to_str()
                .is_some_and(|value| value.starts_with('v'))
        {
            return Err(StudioError::InstalledIdentity(
                "Studio current activation pointer has an invalid release target".into(),
            ));
        }
        let canonical_install = fs::canonicalize(install_path)?;
        let canonical_current = fs::canonicalize(&current)?;
        let releases = canonical_install.join("releases");
        if canonical_current.parent() != Some(releases.as_path()) {
            return Err(StudioError::InstalledIdentity(
                "Studio current release escapes releases root".into(),
            ));
        }
        let binary = canonical_current.join("mcp-studio");
        let version = probe_binary_version(&binary, BinaryVersionFormat::Named)
            .await?
            .ok_or_else(|| {
                StudioError::InstalledIdentity("Studio current binary is missing".into())
            })?;
        let directory = canonical_current
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|value| value.strip_prefix('v'))
            .ok_or_else(|| {
                StudioError::InstalledIdentity("Studio current release directory is invalid".into())
            })?;
        let directory_version = Version::parse(directory).map_err(|error| {
            StudioError::InstalledIdentity(format!(
                "invalid Studio current release version: {error}"
            ))
        })?;
        if directory_version != version {
            return Err(StudioError::InstalledIdentity(format!(
                "Studio current release directory {directory_version} does not match binary {version}"
            )));
        }
        if !canonical_current.join("web/dist/index.html").is_file() {
            return Err(StudioError::InstalledIdentity(
                "Studio current release is missing web/dist/index.html".into(),
            ));
        }
        return Ok(Some(version));
    }
    probe_binary_version(&install_path.join("mcp-studio"), BinaryVersionFormat::Named).await
}

fn probe_fleet_version(install_path: &Path) -> StudioResult<Option<Version>> {
    if !install_path.exists() {
        return Ok(None);
    }
    let version_path = install_path.join("VERSION");
    if !version_path.is_file() {
        return Err(StudioError::InstalledIdentity(format!(
            "Fleet VERSION is missing: {}",
            version_path.display()
        )));
    }
    let value = fs::read_to_string(&version_path)?;
    Version::parse(value.trim())
        .map(Some)
        .map_err(|error| StudioError::InstalledIdentity(format!("invalid Fleet VERSION: {error}")))
}

async fn probe_tunnel_identity(install_path: &Path) -> StudioResult<Option<Version>> {
    if !install_path.exists() {
        return Ok(None);
    }
    let current = install_path.join("current");
    if !current.exists() {
        return Err(StudioError::InstalledIdentity(format!(
            "tunnel current release is missing: {}",
            current.display()
        )));
    }
    let canonical_install = fs::canonicalize(install_path)?;
    let canonical_current = fs::canonicalize(&current)?;
    if !canonical_current.starts_with(&canonical_install) {
        return Err(StudioError::InstalledIdentity(
            "tunnel current release escapes install root".into(),
        ));
    }
    let binary = canonical_current.join("tunnel-client-runtime-cloudflared");
    let version = probe_binary_version(&binary, BinaryVersionFormat::Leading)
        .await?
        .ok_or_else(|| StudioError::InstalledIdentity("tunnel runtime binary is missing".into()))?;
    if let Some(dir_name) = canonical_current
        .file_name()
        .and_then(|value| value.to_str())
        && let Some(tag) = dir_name.strip_prefix('v')
    {
        let directory_version = Version::parse(tag).map_err(|error| {
            StudioError::InstalledIdentity(format!(
                "invalid tunnel release directory {dir_name}: {error}"
            ))
        })?;
        if directory_version != version {
            return Err(StudioError::InstalledIdentity(format!(
                "tunnel current release directory {directory_version} does not match binary {version}"
            )));
        }
    }
    Ok(Some(version))
}

#[derive(Debug, Clone)]
struct CachedReleaseCheck {
    latest: Option<Version>,
    state: ReleaseCheckState,
}

#[derive(Clone)]
pub struct InventoryService {
    catalog: ComponentCatalog,
    source_root: PathBuf,
    platform: Platform,
    desired: Arc<RwLock<BTreeMap<ComponentId, Version>>>,
    installed: Arc<dyn InstalledIdentityProvider>,
    running: Arc<dyn RunningIdentityProvider>,
    providers: Arc<BTreeMap<ReleaseProviderId, Arc<dyn ReleaseProvider>>>,
    checks: Arc<RwLock<BTreeMap<ComponentId, CachedReleaseCheck>>>,
}

impl InventoryService {
    pub fn new(
        catalog: ComponentCatalog,
        source_root: PathBuf,
        platform: Platform,
        desired: BTreeMap<ComponentId, Version>,
    ) -> StudioResult<Self> {
        let mut providers: BTreeMap<ReleaseProviderId, Arc<dyn ReleaseProvider>> = BTreeMap::new();
        providers.insert(
            ReleaseProviderId::ThirteenthXGitHub,
            Arc::new(ThirteenthXReleaseProvider::new(catalog.clone())?),
        );
        providers.insert(
            ReleaseProviderId::OpenAiGitHub,
            Arc::new(OpenAiTunnelReleaseProvider::new(catalog.clone())?),
        );
        Ok(Self::with_dependencies(
            catalog,
            source_root,
            platform,
            desired,
            Arc::new(RuntimeInstalledIdentityProvider),
            Arc::new(StudioRunningIdentityProvider),
            providers,
        ))
    }

    fn with_dependencies(
        catalog: ComponentCatalog,
        source_root: PathBuf,
        platform: Platform,
        desired: BTreeMap<ComponentId, Version>,
        installed: Arc<dyn InstalledIdentityProvider>,
        running: Arc<dyn RunningIdentityProvider>,
        providers: BTreeMap<ReleaseProviderId, Arc<dyn ReleaseProvider>>,
    ) -> Self {
        Self {
            catalog,
            source_root,
            platform,
            desired: Arc::new(RwLock::new(desired)),
            installed,
            running,
            providers: Arc::new(providers),
            checks: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub async fn list(&self) -> Vec<InventoryView> {
        let mut views = Vec::with_capacity(ComponentId::ALL.len());
        for id in ComponentId::ALL {
            if let Ok(entry) = self.entry(id).await {
                views.push(InventoryView::from(&entry));
            }
        }
        views
    }

    pub async fn get(&self, id: ComponentId) -> StudioResult<InventoryView> {
        Ok(InventoryView::from(&self.entry(id).await?))
    }

    pub const fn platform(&self) -> Platform {
        self.platform
    }

    pub async fn set_desired(&self, id: ComponentId, version: Version) {
        self.desired.write().await.insert(id, version);
    }

    pub async fn check(&self) -> Vec<InventoryView> {
        let previous = self.checks.read().await.clone();
        let mut tasks = JoinSet::new();

        for id in ComponentId::ALL {
            let Ok(policy) = self.catalog.component(id).cloned() else {
                continue;
            };
            let provider = self.providers.get(&policy.provider).cloned();
            let previous_latest = previous.get(&id).and_then(|entry| entry.latest.clone());

            tasks.spawn(async move {
                let checked_at_ms = now_ms();
                let (latest, error) = match provider {
                    Some(provider) => match provider.latest_release(&policy).await {
                        Ok(release) => (Some(release.version), None),
                        Err(error) => (previous_latest, Some(safe_release_check_error(&error))),
                    },
                    None => (
                        previous_latest,
                        Some(format!(
                            "release provider {:?} is unavailable",
                            policy.provider
                        )),
                    ),
                };
                (
                    id,
                    CachedReleaseCheck {
                        latest,
                        state: ReleaseCheckState {
                            checked_at_ms: Some(checked_at_ms),
                            error,
                        },
                    },
                )
            });
        }

        let mut checked = BTreeMap::new();
        while let Some(result) = tasks.join_next().await {
            if let Ok((id, entry)) = result {
                checked.insert(id, entry);
            }
        }
        self.checks.write().await.extend(checked);
        self.list().await
    }

    async fn entry(&self, id: ComponentId) -> StudioResult<InventoryEntry> {
        let policy = self.catalog.component(id)?;
        let install_path = self.catalog.install_path(id)?;
        let installed_result = self
            .installed
            .installed_identity(policy, &install_path)
            .await;
        let (installed_version, installation_health) = match installed_result {
            Ok(Some(identity)) => (Some(identity.version), InstallationHealth::Healthy),
            Ok(None) => (None, InstallationHealth::Broken),
            Err(_) => (None, InstallationHealth::Broken),
        };
        let running_version = self.running.running_version(policy).await.unwrap_or(None);
        let check = self.checks.read().await.get(&id).cloned();
        let latest_version = check.as_ref().and_then(|entry| entry.latest.clone());
        let last_check = check.map(|entry| entry.state).unwrap_or_default();
        let desired_version = self
            .desired
            .read()
            .await
            .get(&id)
            .cloned()
            .or_else(|| installed_version.clone());
        let drift_running_version = if policy.class == ComponentClass::ControlBundle {
            installed_version.clone()
        } else {
            running_version.clone()
        };
        let dimensions = VersionDimensions {
            installed_version: installed_version.clone(),
            running_version: drift_running_version,
            desired_version: desired_version.clone(),
            latest_version: latest_version.clone(),
            installation_health,
        };
        Ok(InventoryEntry {
            component: id,
            display_name: policy.display_name,
            class: policy.class,
            provider: policy.provider,
            repository_owner: policy.source.owner,
            repository_name: policy.source.repository,
            install_path,
            platform: self.platform,
            source_present: self.source_path(policy).is_dir(),
            installed_version,
            running_version,
            desired_version,
            latest_version,
            installation_health,
            drift: dimensions.drift_state(),
            last_check,
        })
    }

    fn source_path(&self, policy: &ComponentPolicy) -> PathBuf {
        self.source_root.join(policy.source.repository)
    }
}

fn safe_release_check_error(error: &StudioError) -> String {
    match error {
        StudioError::ReleaseProviderUnreachable { .. } => "provider_unreachable".into(),
        StudioError::ReleaseProviderHttp { status, .. } => format!("http_{status}"),
        StudioError::ReleaseProviderResponseTooLarge { .. } => "response_too_large".into(),
        StudioError::MalformedReleaseMetadata { .. } => "malformed_release_metadata".into(),
        StudioError::InvalidReleaseTag { .. } => "invalid_release_tag".into(),
        StudioError::NoStableRelease { .. } => "no_stable_release".into(),
        StudioError::ReleaseVersionNotFound { .. } => "release_version_not_found".into(),
        StudioError::ReleaseNotEligible { .. } => "release_not_eligible".into(),
        StudioError::MissingChecksumManifest { .. } => "missing_checksum_manifest".into(),
        StudioError::DuplicateChecksumManifest { .. } => "duplicate_checksum_manifest".into(),
        StudioError::UntrustedReleaseAssetUrl { .. } => "untrusted_release_asset_url".into(),
        StudioError::MissingReleaseAssetFamily { .. } => "missing_release_asset_family".into(),
        StudioError::AmbiguousReleaseAssetFamily { .. } => "ambiguous_release_asset_family".into(),
        _ => "release_check_failed".into(),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use super::*;
    use crate::update::{
        Architecture, AvailableRelease, HostRuntimeRoots, OperatingSystem, ReleaseAsset,
    };

    #[derive(Default)]
    struct FakeInstalled {
        versions: Mutex<BTreeMap<ComponentId, Result<Option<Version>, String>>>,
    }

    #[async_trait]
    impl InstalledIdentityProvider for FakeInstalled {
        async fn installed_identity(
            &self,
            component: &ComponentPolicy,
            install_path: &Path,
        ) -> StudioResult<Option<InstalledArtifactIdentity>> {
            match self.versions.lock().unwrap().get(&component.id).cloned() {
                Some(Ok(version)) => Ok(version.map(|version| InstalledArtifactIdentity {
                    component: component.id,
                    version,
                    platform: None,
                    sha256: None,
                    install_path: install_path.to_owned(),
                })),
                Some(Err(error)) => Err(StudioError::InstalledIdentity(error)),
                None => Ok(None),
            }
        }
    }

    #[derive(Default)]
    struct FakeRunning {
        versions: Mutex<BTreeMap<ComponentId, Option<Version>>>,
    }

    #[async_trait]
    impl RunningIdentityProvider for FakeRunning {
        async fn running_version(
            &self,
            component: &ComponentPolicy,
        ) -> StudioResult<Option<Version>> {
            Ok(self
                .versions
                .lock()
                .unwrap()
                .get(&component.id)
                .cloned()
                .flatten())
        }
    }

    struct FakeProvider {
        id: ReleaseProviderId,
        latest: Mutex<BTreeMap<ComponentId, Result<Version, String>>>,
    }

    #[async_trait]
    impl ReleaseProvider for FakeProvider {
        fn provider_id(&self) -> ReleaseProviderId {
            self.id
        }
        async fn latest_release(
            &self,
            component: &ComponentPolicy,
        ) -> StudioResult<AvailableRelease> {
            match self.latest.lock().unwrap().get(&component.id).cloned() {
                Some(Ok(version)) => Ok(AvailableRelease {
                    component: component.id,
                    tag: format!("v{version}"),
                    version,
                    assets: Vec::<ReleaseAsset>::new(),
                    checksum_manifest_url: String::new(),
                }),
                Some(Err(error)) => Err(StudioError::ReleaseProviderUnreachable {
                    component: component.id.to_string(),
                    detail: error,
                }),
                None => Err(StudioError::NoStableRelease {
                    component: component.id.to_string(),
                }),
            }
        }
        async fn release(
            &self,
            _component: &ComponentPolicy,
            _version: &Version,
        ) -> StudioResult<AvailableRelease> {
            unreachable!()
        }
        fn select_asset<'a>(
            &self,
            _release: &'a AvailableRelease,
            _platform: Platform,
        ) -> StudioResult<&'a ReleaseAsset> {
            unreachable!()
        }
        async fn checksum_manifest(&self, _release: &AvailableRelease) -> StudioResult<Vec<u8>> {
            unreachable!()
        }
    }

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn make_service(
        source_root: PathBuf,
        installed: Arc<FakeInstalled>,
        running: Arc<FakeRunning>,
        project_latest: BTreeMap<ComponentId, Result<Version, String>>,
        tunnel_latest: BTreeMap<ComponentId, Result<Version, String>>,
        desired: BTreeMap<ComponentId, Version>,
    ) -> InventoryService {
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(
                PathBuf::from("/runtime/bin"),
                PathBuf::from("/runtime/state"),
            )
            .unwrap(),
        );
        let mut providers: BTreeMap<ReleaseProviderId, Arc<dyn ReleaseProvider>> = BTreeMap::new();
        providers.insert(
            ReleaseProviderId::ThirteenthXGitHub,
            Arc::new(FakeProvider {
                id: ReleaseProviderId::ThirteenthXGitHub,
                latest: Mutex::new(project_latest),
            }),
        );
        providers.insert(
            ReleaseProviderId::OpenAiGitHub,
            Arc::new(FakeProvider {
                id: ReleaseProviderId::OpenAiGitHub,
                latest: Mutex::new(tunnel_latest),
            }),
        );
        InventoryService::with_dependencies(
            catalog,
            source_root,
            platform(),
            desired,
            installed,
            running,
            providers,
        )
    }

    fn version(value: &str) -> Version {
        Version::parse(value).unwrap()
    }

    #[tokio::test]
    async fn full_current_inventory_and_update_available_are_deterministic() {
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("git")).unwrap();
        let installed = Arc::new(FakeInstalled::default());
        let running = Arc::new(FakeRunning::default());
        for id in ComponentId::ALL {
            installed
                .versions
                .lock()
                .unwrap()
                .insert(id, Ok(Some(version("1.0.0"))));
            running
                .versions
                .lock()
                .unwrap()
                .insert(id, Some(version("1.0.0")));
        }
        let project_latest = ComponentId::ALL
            .into_iter()
            .filter(|id| *id != ComponentId::Tunnel)
            .map(|id| {
                (
                    id,
                    Ok(version(if id == ComponentId::Git {
                        "1.1.0"
                    } else {
                        "1.0.0"
                    })),
                )
            })
            .collect();
        let tunnel_latest = BTreeMap::from([(ComponentId::Tunnel, Ok(version("1.0.0")))]);
        let service = make_service(
            source.path().to_owned(),
            installed,
            running,
            project_latest,
            tunnel_latest,
            BTreeMap::new(),
        );
        let views = service.check().await;
        assert_eq!(views.len(), 8);
        let git = views
            .iter()
            .find(|view| view.component == ComponentId::Git)
            .unwrap();
        assert_eq!(git.drift, DriftState::UpdateAvailable);
        assert!(git.update_available);
        assert!(git.source_present);
        assert_eq!(git.host_mode, HostMode::SourcePresent);
        let exec = views
            .iter()
            .find(|view| view.component == ComponentId::Exec)
            .unwrap();
        assert_eq!(exec.drift, DriftState::Current);
        assert_eq!(exec.host_mode, HostMode::RuntimeOnly);
    }

    #[tokio::test]
    async fn installed_running_and_desired_mismatches_map_to_expected_drift() {
        let installed = Arc::new(FakeInstalled::default());
        let running = Arc::new(FakeRunning::default());
        installed
            .versions
            .lock()
            .unwrap()
            .insert(ComponentId::Git, Ok(Some(version("1.1.0"))));
        running
            .versions
            .lock()
            .unwrap()
            .insert(ComponentId::Git, Some(version("1.0.0")));
        let service = make_service(
            PathBuf::from("/missing-source"),
            installed.clone(),
            running.clone(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let git = service.get(ComponentId::Git).await.unwrap();
        assert_eq!(git.drift, DriftState::InstalledRestartRequired);

        let desired = BTreeMap::from([(ComponentId::Git, version("2.0.0"))]);
        let service = make_service(
            PathBuf::from("/missing-source"),
            installed,
            running,
            BTreeMap::new(),
            BTreeMap::new(),
            desired,
        );
        let git = service.get(ComponentId::Git).await.unwrap();
        assert_eq!(git.drift, DriftState::Drifted);
    }

    #[tokio::test]
    async fn missing_or_malformed_installed_identity_is_broken() {
        let installed = Arc::new(FakeInstalled::default());
        let running = Arc::new(FakeRunning::default());
        installed
            .versions
            .lock()
            .unwrap()
            .insert(ComponentId::Git, Err("malformed --version".into()));
        let service = make_service(
            PathBuf::from("/no-source"),
            installed,
            running,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let git = service.get(ComponentId::Git).await.unwrap();
        assert_eq!(git.installation_health, InventoryHealth::Broken);
        assert_eq!(git.drift, DriftState::Broken);
    }

    #[tokio::test]
    async fn provider_failure_does_not_break_local_inventory() {
        let installed = Arc::new(FakeInstalled::default());
        let running = Arc::new(FakeRunning::default());
        installed
            .versions
            .lock()
            .unwrap()
            .insert(ComponentId::Git, Ok(Some(version("1.0.0"))));
        running
            .versions
            .lock()
            .unwrap()
            .insert(ComponentId::Git, Some(version("1.0.0")));
        let project_latest = BTreeMap::from([(ComponentId::Git, Err("offline".into()))]);
        let service = make_service(
            PathBuf::from("/no-source"),
            installed,
            running,
            project_latest,
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let views = service.check().await;
        let git = views
            .iter()
            .find(|view| view.component == ComponentId::Git)
            .unwrap();
        assert_eq!(git.installed_version.as_ref().unwrap().to_string(), "1.0.0");
        assert_eq!(git.last_check.status, CheckStatus::Error);
        assert_eq!(
            git.last_check.error.as_deref(),
            Some("provider_unreachable")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_binary_version_probe_is_strict() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("rust-mcp-git");
        fs::write(&binary, "#!/bin/sh\necho 'rust-mcp-git 1.2.3'\n").unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();

        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.path().to_owned(), root.path().join("runtime")).unwrap(),
        );
        let git = catalog.component(ComponentId::Git).unwrap();
        let provider = RuntimeInstalledIdentityProvider;
        let identity = provider
            .installed_identity(git, &binary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(identity.version.to_string(), "1.2.3");

        fs::write(&binary, "#!/bin/sh\necho 'not-semver'\n").unwrap();
        assert!(matches!(
            provider.installed_identity(git, &binary).await.unwrap_err(),
            StudioError::InstalledIdentity(_)
        ));
    }

    #[tokio::test]
    #[ignore = "explicit Aira workspace runtime inventory smoke"]
    async fn live_local_runtime_inventory_smoke() {
        let cwd = std::env::current_dir().unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(cwd.join("../bin"), cwd.join("../runtime")).unwrap(),
        );
        let service = InventoryService::new(
            catalog,
            cwd.join(".."),
            super::super::HostPlatform::detect().unwrap().platform(),
            BTreeMap::new(),
        )
        .unwrap();
        let views = service.list().await;
        assert_eq!(views.len(), 8);
        for id in [
            ComponentId::Filesystem,
            ComponentId::Git,
            ComponentId::Exec,
            ComponentId::Gateway,
            ComponentId::Blender,
            ComponentId::Studio,
            ComponentId::Tunnel,
        ] {
            let view = views.iter().find(|view| view.component == id).unwrap();
            assert!(
                view.installed_version.is_some(),
                "missing installed version for {id}"
            );
        }
        let fleet = views
            .iter()
            .find(|view| view.component == ComponentId::Fleet)
            .unwrap();
        assert_eq!(fleet.installation_health, InventoryHealth::Broken);
    }

    #[test]
    fn public_inventory_dto_omits_paths_and_repository_coordinates() {
        let entry = InventoryEntry {
            component: ComponentId::Git,
            display_name: "Git MCP",
            class: ComponentClass::McpBinary,
            provider: ReleaseProviderId::ThirteenthXGitHub,
            repository_owner: "13thx-mcp",
            repository_name: "git",
            install_path: PathBuf::from("/secret/runtime/bin/rust-mcp-git"),
            platform: platform(),
            source_present: false,
            installed_version: Some(version("1.0.0")),
            running_version: None,
            desired_version: Some(version("1.0.0")),
            latest_version: None,
            installation_health: InstallationHealth::Healthy,
            drift: DriftState::Unknown,
            last_check: ReleaseCheckState::default(),
        };
        let json = serde_json::to_value(InventoryView::from(&entry)).unwrap();
        let text = json.to_string();
        assert!(!text.contains("secret/runtime"));
        assert!(!text.contains("13thx-mcp"));
        assert!(!json.as_object().unwrap().contains_key("install_path"));
        assert!(!json.as_object().unwrap().contains_key("repository"));
    }
}
