use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::{process::Command, sync::Mutex, time::timeout};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
};

use super::{
    ArtifactHistoryIdentity, ArtifactStager, ComponentCatalog, ComponentId, InventoryService,
    McpUpdatePhase, McpUpdateTransactionView, ReleaseProvider, StagedArtifact,
    ThirteenthXReleaseProvider, Version,
    transaction::{PreparedStagedIdentity, now_ms, sanitize_transaction_error},
};

const SUPPORTED_FLEET_SCHEMA: u64 = 2;
const SUPPORTED_HOST_SCHEMA: u64 = 1;
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_LOCAL_STATE_FILES: usize = 4096;
const MAX_LOCAL_STATE_BYTES: u64 = 64 * 1024 * 1024;
const CANDIDATE_PREFIX: &str = ".mcp-studio-fleet-candidate-";
const ROLLBACK_PREFIX: &str = ".mcp-studio-fleet-rollback-";
const FAILED_PREFIX: &str = ".mcp-studio-fleet-failed-";

#[derive(Debug, Clone)]
struct FleetTransactionRecord {
    view: McpUpdateTransactionView,
    staged_id: Option<String>,
    staged_identity: Option<PreparedStagedIdentity>,
    bundle_hashes: Option<BTreeMap<PathBuf, String>>,
}

#[derive(Debug, Deserialize)]
struct FleetSchema {
    schema_version: u64,
}

#[derive(Debug, Deserialize)]
struct HostProfileSchema {
    schema_version: u64,
    host_id: String,
    bin_root: PathBuf,
    runtime_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalFileIdentity {
    bytes: Vec<u8>,
    mode: u32,
}

#[derive(Debug, Clone)]
struct LocalFleetState {
    profile_name: String,
    host_id: String,
    profile: LocalFileIdentity,
    state_files: BTreeMap<PathBuf, LocalFileIdentity>,
}

impl LocalFleetState {
    fn capture(install: &Path, catalog: &ComponentCatalog) -> StudioResult<Self> {
        let metadata = fs::symlink_metadata(install).map_err(|error| {
            StudioError::UpdateTransaction(format!("Fleet runtime bundle is unavailable: {error}"))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StudioError::UpdateTransaction(
                "Fleet runtime bundle must be a regular directory".into(),
            ));
        }
        validate_runtime_top_level(install)?;

        let hosts = install.join("hosts");
        let hosts_meta = fs::symlink_metadata(&hosts)?;
        if hosts_meta.file_type().is_symlink() || !hosts_meta.is_dir() {
            return Err(StudioError::UpdateTransaction(
                "Fleet hosts directory must be a regular directory".into(),
            ));
        }

        let mut profiles = Vec::new();
        for entry in fs::read_dir(&hosts)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StudioError::UpdateTransaction(
                    "Fleet hosts directory may contain regular files only".into(),
                ));
            }
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| {
                    StudioError::UpdateTransaction("invalid Fleet host profile filename".into())
                })?
                .to_owned();
            if !name.ends_with(".toml") || name.ends_with(".example.toml") {
                continue;
            }
            profiles.push((
                name,
                LocalFileIdentity {
                    bytes: fs::read(&path)?,
                    mode: file_mode(&metadata),
                },
            ));
        }
        if profiles.len() != 1 {
            return Err(StudioError::UpdateTransaction(
                "Fleet runtime must contain exactly one active non-example host profile".into(),
            ));
        }
        let (profile_name, profile) = profiles.pop().expect("one profile");
        let parsed_profile: HostProfileSchema =
            toml::from_str(std::str::from_utf8(&profile.bytes).map_err(|_| {
                StudioError::UpdateTransaction("Fleet host profile is not UTF-8".into())
            })?)?;
        validate_host_profile(&profile_name, &parsed_profile, catalog)?;

        let state_files = capture_state_tree(&install.join("state"))?;
        Ok(Self {
            profile_name,
            host_id: parsed_profile.host_id,
            profile,
            state_files,
        })
    }

    fn install_into(&self, candidate: &Path) -> StudioResult<()> {
        let hosts = candidate.join("hosts");
        fs::create_dir_all(&hosts)?;
        let profile_path = hosts.join(&self.profile_name);
        write_new_file(&profile_path, &self.profile.bytes)?;
        set_file_mode(&profile_path, self.profile.mode)?;
        if !self.state_files.is_empty() {
            let state = candidate.join("state");
            fs::create_dir_all(&state)?;
            for (relative, identity) in &self.state_files {
                let destination = state.join(relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                write_new_file(&destination, &identity.bytes)?;
                set_file_mode(&destination, identity.mode)?;
            }
        }
        Ok(())
    }

    fn verify_preserved(&self, active: &Path) -> StudioResult<()> {
        let profile_path = active.join("hosts").join(&self.profile_name);
        let profile_metadata = fs::metadata(&profile_path)?;
        if fs::read(&profile_path)? != self.profile.bytes
            || file_mode(&profile_metadata) != self.profile.mode
        {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: "active Fleet host profile changed during update".into(),
            });
        }
        let state = capture_state_tree(&active.join("state"))?;
        if state != self.state_files {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: "Fleet local state changed during update".into(),
            });
        }
        Ok(())
    }
}

fn validate_runtime_top_level(install: &Path) -> StudioResult<()> {
    let allowed = BTreeSet::from([
        "VERSION",
        "fleet.toml",
        "README.md",
        "scripts",
        "hosts",
        "state",
    ]);
    for entry in fs::read_dir(install)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            StudioError::UpdateTransaction("invalid Fleet runtime filename".into())
        })?;
        if !allowed.contains(name) {
            return Err(StudioError::UpdateTransaction(format!(
                "unexpected Fleet runtime entry: {name}"
            )));
        }
    }
    Ok(())
}

fn validate_host_profile(
    profile_name: &str,
    profile: &HostProfileSchema,
    catalog: &ComponentCatalog,
) -> StudioResult<()> {
    if profile.schema_version != SUPPORTED_HOST_SCHEMA {
        return Err(StudioError::UpdateTransaction(format!(
            "unsupported Fleet host schema {}; explicit migration is required",
            profile.schema_version
        )));
    }
    let expected_name = format!("{}.toml", profile.host_id);
    if profile_name != expected_name {
        return Err(StudioError::UpdateTransaction(
            "Fleet host_id does not match active profile filename".into(),
        ));
    }
    let actual_bin = fs::canonicalize(&profile.bin_root).map_err(|error| {
        StudioError::UpdateTransaction(format!("Fleet host bin_root does not resolve: {error}"))
    })?;
    let expected_bin = fs::canonicalize(catalog.bin_root()).map_err(|error| {
        StudioError::UpdateTransaction(format!("catalog bin_root does not resolve: {error}"))
    })?;
    let actual_runtime = fs::canonicalize(&profile.runtime_root).map_err(|error| {
        StudioError::UpdateTransaction(format!("Fleet host runtime_root does not resolve: {error}"))
    })?;
    let expected_runtime = fs::canonicalize(catalog.runtime_root()).map_err(|error| {
        StudioError::UpdateTransaction(format!("catalog runtime_root does not resolve: {error}"))
    })?;
    if actual_bin != expected_bin || actual_runtime != expected_runtime {
        return Err(StudioError::UpdateTransaction(
            "Fleet host profile roots do not match trusted runtime roots".into(),
        ));
    }
    Ok(())
}

fn validate_candidate_schema(
    candidate: &Path,
    local: &LocalFleetState,
    catalog: &ComponentCatalog,
) -> StudioResult<()> {
    let fleet: FleetSchema = toml::from_str(&fs::read_to_string(candidate.join("fleet.toml"))?)?;
    if fleet.schema_version != SUPPORTED_FLEET_SCHEMA {
        return Err(StudioError::UpdateTransaction(format!(
            "unsupported Fleet schema {}; explicit migration is required",
            fleet.schema_version
        )));
    }
    let example: HostProfileSchema = toml::from_str(&fs::read_to_string(
        candidate.join("hosts/mirin.example.toml"),
    )?)?;
    if example.schema_version != SUPPORTED_HOST_SCHEMA {
        return Err(StudioError::UpdateTransaction(format!(
            "release host schema {} is incompatible with supported schema {}",
            example.schema_version, SUPPORTED_HOST_SCHEMA
        )));
    }
    let active: HostProfileSchema = toml::from_str(&fs::read_to_string(
        candidate.join("hosts").join(&local.profile_name),
    )?)?;
    validate_host_profile(&local.profile_name, &active, catalog)?;
    if active.host_id != local.host_id {
        return Err(StudioError::UpdateTransaction(
            "Fleet active host identity changed in candidate".into(),
        ));
    }
    Ok(())
}

fn capture_state_tree(root: &Path) -> StudioResult<BTreeMap<PathBuf, LocalFileIdentity>> {
    if !root.exists() {
        return Ok(BTreeMap::new());
    }
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet local state must be a regular directory".into(),
        ));
    }
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    capture_state_dir(root, root, &mut files, &mut total)?;
    Ok(files)
}

fn capture_state_dir(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<PathBuf, LocalFileIdentity>,
    total: &mut u64,
) -> StudioResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::UpdateTransaction(
                "Fleet local state must not contain symlinks".into(),
            ));
        }
        if metadata.is_dir() {
            capture_state_dir(root, &path, files, total)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet local state contains an unsupported entry".into(),
            ));
        }
        if files.len() >= MAX_LOCAL_STATE_FILES {
            return Err(StudioError::UpdateTransaction(
                "Fleet local state contains too many files".into(),
            ));
        }
        *total = total.saturating_add(metadata.len());
        if *total > MAX_LOCAL_STATE_BYTES {
            return Err(StudioError::UpdateTransaction(
                "Fleet local state exceeds the preservation size limit".into(),
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| StudioError::UpdateTransaction("Fleet state path escaped root".into()))?
            .to_owned();
        files.insert(
            relative,
            LocalFileIdentity {
                bytes: fs::read(&path)?,
                mode: file_mode(&metadata),
            },
        );
    }
    Ok(())
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> StudioResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_file_mode(path: &Path, mode: u32) -> StudioResult<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(mode != 0);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[async_trait]
trait FleetValidator: Send + Sync {
    async fn validate(&self, bundle: &Path, host_id: &str) -> StudioResult<()>;
}

struct ProcessFleetValidator;

#[async_trait]
impl FleetValidator for ProcessFleetValidator {
    async fn validate(&self, bundle: &Path, host_id: &str) -> StudioResult<()> {
        let script = bundle.join("scripts/fleetctl.py");
        let metadata = fs::symlink_metadata(&script)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: "Fleet validation script is unavailable".into(),
            });
        }
        let mut command = Command::new("python3");
        command
            .arg(&script)
            .arg("render-plan")
            .arg("--host")
            .arg(host_id)
            .arg("--json")
            .current_dir(bundle)
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .kill_on_drop(true);
        let output = timeout(VALIDATION_TIMEOUT, command.output())
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: "Fleet generated-config validation timed out".into(),
            })?
            .map_err(|error| StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: format!("Fleet generated-config validation failed to start: {error}"),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim().chars().take(512).collect::<String>();
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: if detail.is_empty() {
                    format!(
                        "Fleet generated-config validation exited with {}",
                        output.status
                    )
                } else {
                    format!("Fleet generated-config validation failed: {detail}")
                },
            });
        }
        serde_json::from_slice::<serde_json::Value>(&output.stdout).map_err(|error| {
            StudioError::UpdateVerificationFailed {
                component: ComponentId::Fleet.to_string(),
                detail: format!("Fleet generated-config validation returned invalid JSON: {error}"),
            }
        })?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct FleetUpdateManager {
    catalog: ComponentCatalog,
    stager: ArtifactStager,
    provider: Arc<dyn ReleaseProvider>,
    validator: Arc<dyn FleetValidator>,
    inventory: Arc<InventoryService>,
    events: EventHub,
    active: Arc<StdMutex<bool>>,
    transactions: Arc<Mutex<BTreeMap<String, FleetTransactionRecord>>>,
}

impl FleetUpdateManager {
    pub fn new(
        catalog: ComponentCatalog,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> StudioResult<Self> {
        recover_interrupted_fleet_swap(&catalog)?;
        let stager = ArtifactStager::new(catalog.clone())?;
        let provider = Arc::new(ThirteenthXReleaseProvider::new(catalog.clone())?);
        Ok(Self {
            catalog,
            stager,
            provider,
            validator: Arc::new(ProcessFleetValidator),
            inventory,
            events,
            active: Arc::new(StdMutex::new(false)),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        stager: ArtifactStager,
        provider: Arc<dyn ReleaseProvider>,
        validator: Arc<dyn FleetValidator>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> Self {
        Self {
            catalog,
            stager,
            provider,
            validator,
            inventory,
            events,
            active: Arc::new(StdMutex::new(false)),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub async fn prepare(&self, version: Version) -> StudioResult<McpUpdateTransactionView> {
        let _guard = self.acquire()?;
        let install = self.catalog.install_path(ComponentId::Fleet)?;
        let source_version = read_fleet_version_optional(&install)?;
        if let Some(source) = &source_version
            && !version.is_newer_than(source)
        {
            return Err(StudioError::Conflict(format!(
                "fleet: target {version} must be newer than installed {source}"
            )));
        }
        let transaction_id = new_fleet_transaction_id();
        let preparing = McpUpdateTransactionView {
            transaction_id: transaction_id.clone(),
            component: ComponentId::Fleet,
            source_version,
            target_version: version.clone(),
            phase: McpUpdatePhase::Preparing,
            was_running: None,
            rollback_succeeded: None,
            error: None,
            updated_at_ms: now_ms(),
        };
        self.transactions.lock().await.insert(
            transaction_id.clone(),
            FleetTransactionRecord {
                view: preparing.clone(),
                staged_id: None,
                staged_identity: None,
                bundle_hashes: None,
            },
        );
        self.emit(&preparing);

        let policy = self.catalog.component(ComponentId::Fleet)?.clone();
        let result = async {
            let release = self.provider.release(&policy, &version).await?;
            let staged = self
                .stager
                .stage(self.provider.as_ref(), &release, self.inventory.platform())
                .await?;
            let staged_id = self.stager.staged_id(&staged)?;
            let bundle_hashes = fleet_package_hashes(&staged.package_root)?;
            Ok::<_, StudioError>((
                staged_id,
                PreparedStagedIdentity::from(&staged),
                bundle_hashes,
            ))
        }
        .await;
        match result {
            Ok((staged_id, staged_identity, bundle_hashes)) => {
                let mut transactions = self.transactions.lock().await;
                let record = transactions
                    .get_mut(&transaction_id)
                    .expect("Fleet preparing transaction exists");
                record.staged_id = Some(staged_id);
                record.staged_identity = Some(staged_identity);
                record.bundle_hashes = Some(bundle_hashes);
                record.view.phase = McpUpdatePhase::Staged;
                record.view.updated_at_ms = now_ms();
                let view = record.view.clone();
                drop(transactions);
                self.emit(&view);
                Ok(view)
            }
            Err(error) => {
                self.update_failed(&transaction_id, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    pub async fn apply(&self, transaction_id: &str) -> StudioResult<McpUpdateTransactionView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("fleet_update")?;
        let _guard = self.acquire()?;
        let (staged_id, source_version, expected_staged, expected_bundle_hashes) =
            self.ensure_transaction_applicable(transaction_id).await?;
        let install = self.catalog.install_path(ComponentId::Fleet)?;
        let current_version = read_fleet_version_optional(&install)?;
        if current_version != source_version {
            return Err(StudioError::Conflict(
                "Fleet installed version changed after update preparation".into(),
            ));
        }
        let staged = self.stager.load_ready(&staged_id)?;
        if staged.component != ComponentId::Fleet
            || PreparedStagedIdentity::from(&staged) != expected_staged
            || fleet_package_hashes(&staged.package_root)? != expected_bundle_hashes
        {
            return Err(StudioError::Conflict(
                "Fleet staged artifact changed after preparation".into(),
            ));
        }
        self.apply_staged(transaction_id, source_version, staged)
            .await
    }

    pub(crate) async fn artifact_history_identity(
        &self,
        transaction_id: &str,
    ) -> Option<ArtifactHistoryIdentity> {
        self.transactions
            .lock()
            .await
            .get(transaction_id)
            .and_then(|record| record.staged_identity.as_ref())
            .map(|identity| identity.history_identity(ComponentId::Fleet))
    }

    pub async fn transaction(
        &self,
        transaction_id: &str,
    ) -> StudioResult<McpUpdateTransactionView> {
        self.transactions
            .lock()
            .await
            .get(transaction_id)
            .map(|record| record.view.clone())
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))
    }

    async fn ensure_transaction_applicable(
        &self,
        transaction_id: &str,
    ) -> StudioResult<(
        String,
        Option<Version>,
        PreparedStagedIdentity,
        BTreeMap<PathBuf, String>,
    )> {
        if !safe_fleet_transaction_id(transaction_id) {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        let transactions = self.transactions.lock().await;
        let record = transactions
            .get(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        if record.view.phase != McpUpdatePhase::Staged {
            return Err(StudioError::Conflict(format!(
                "Fleet transaction {transaction_id} is not staged"
            )));
        }
        Ok((
            record.staged_id.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("Fleet transaction has no staged artifact".into())
            })?,
            record.view.source_version.clone(),
            record.staged_identity.clone().ok_or_else(|| {
                StudioError::UpdateTransaction(
                    "Fleet transaction has no staging fingerprint".into(),
                )
            })?,
            record.bundle_hashes.clone().ok_or_else(|| {
                StudioError::UpdateTransaction(
                    "Fleet transaction has no bundle content fingerprint".into(),
                )
            })?,
        ))
    }

    async fn apply_staged(
        &self,
        transaction_id: &str,
        source_version: Option<Version>,
        staged: StagedArtifact,
    ) -> StudioResult<McpUpdateTransactionView> {
        let target_version = staged.version.clone();
        let install = self.catalog.install_path(ComponentId::Fleet)?;
        let local = LocalFleetState::capture(&install, &self.catalog)?;
        let candidate = candidate_path(
            self.catalog.runtime_root(),
            transaction_id,
            CANDIDATE_PREFIX,
        )?;
        let rollback =
            candidate_path(self.catalog.runtime_root(), transaction_id, ROLLBACK_PREFIX)?;
        let failed = candidate_path(self.catalog.runtime_root(), transaction_id, FAILED_PREFIX)?;
        for path in [&candidate, &rollback, &failed] {
            if path.exists() {
                return Err(StudioError::Conflict(
                    "Fleet transaction scratch path already exists".into(),
                ));
            }
        }

        let result = async {
            copy_tree(&staged.package_root, &candidate)?;
            local.install_into(&candidate)?;
            validate_candidate_schema(&candidate, &local, &self.catalog)?;
            let candidate_version = read_fleet_version_required(&candidate)?;
            if candidate_version != target_version {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Fleet.to_string(),
                    detail: "Fleet candidate VERSION changed after staging".into(),
                });
            }
            self.validator.validate(&candidate, &local.host_id).await?;
            local.verify_preserved(&candidate)?;
            Ok::<_, StudioError>(())
        }
        .await;
        if let Err(error) = result {
            let _ = remove_tree_if_exists(&candidate);
            self.update_failed(transaction_id, error.to_string())
                .await?;
            return Err(error);
        }

        self.update_phase(transaction_id, McpUpdatePhase::Activating, None, None)
            .await?;
        if let Err(error) = activate_bundle(&install, &candidate, &rollback) {
            let _ = recover_bundle_after_activation_error(&install, &candidate, &rollback);
            self.update_failed(transaction_id, format!("Fleet activation failed: {error}"))
                .await?;
            return Err(error);
        }

        self.update_phase(transaction_id, McpUpdatePhase::Verifying, None, None)
            .await?;
        let verification = async {
            if read_fleet_version_required(&install)? != target_version {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Fleet.to_string(),
                    detail: "active Fleet VERSION does not match target".into(),
                });
            }
            local.verify_preserved(&install)?;
            validate_candidate_schema(&install, &local, &self.catalog)?;
            self.validator.validate(&install, &local.host_id).await
        }
        .await;

        if let Err(error) = verification {
            return self
                .rollback_after_failure(
                    transaction_id,
                    source_version,
                    target_version,
                    &install,
                    &rollback,
                    &failed,
                    &local,
                    error.to_string(),
                )
                .await;
        }

        remove_tree_if_exists(&rollback)?;
        self.inventory
            .set_desired(ComponentId::Fleet, target_version.clone())
            .await;
        self.update_phase(transaction_id, McpUpdatePhase::Completed, None, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn rollback_after_failure(
        &self,
        transaction_id: &str,
        source_version: Option<Version>,
        target_version: Version,
        install: &Path,
        rollback: &Path,
        failed: &Path,
        local: &LocalFleetState,
        reason: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        self.update_phase(
            transaction_id,
            McpUpdatePhase::RollingBack,
            None,
            Some(reason.clone()),
        )
        .await?;

        let restore = (|| -> StudioResult<()> {
            if install.exists() {
                fs::rename(install, failed).map_err(|error| StudioError::RollbackFailed {
                    component: ComponentId::Fleet.to_string(),
                    detail: format!("failed to quarantine new Fleet bundle: {error}"),
                })?;
            }
            fs::rename(rollback, install).map_err(|error| StudioError::RollbackFailed {
                component: ComponentId::Fleet.to_string(),
                detail: format!("failed to restore prior Fleet bundle: {error}"),
            })?;
            sync_dir(self.catalog.runtime_root())?;
            Ok(())
        })();
        if let Err(error) = restore {
            return self
                .rollback_failed(transaction_id, target_version, reason, error.to_string())
                .await;
        }

        let verification = async {
            if read_fleet_version_optional(install)? != source_version {
                return Err(StudioError::RollbackFailed {
                    component: ComponentId::Fleet.to_string(),
                    detail: "restored Fleet VERSION does not match pre-update identity".into(),
                });
            }
            local.verify_preserved(install)?;
            self.validator.validate(install, &local.host_id).await?;
            Ok::<_, StudioError>(())
        }
        .await;
        if let Err(error) = verification {
            return self
                .rollback_failed(transaction_id, target_version, reason, error.to_string())
                .await;
        }
        let _ = remove_tree_if_exists(failed);
        let failure = reason.clone();
        self.update_phase(
            transaction_id,
            McpUpdatePhase::Failed,
            Some(true),
            Some(reason),
        )
        .await?;
        Err(StudioError::UpdateTransaction(failure))
    }

    async fn rollback_failed(
        &self,
        transaction_id: &str,
        target_version: Version,
        reason: String,
        rollback_error: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        let view = McpUpdateTransactionView {
            transaction_id: transaction_id.to_owned(),
            component: ComponentId::Fleet,
            source_version: None,
            target_version,
            phase: McpUpdatePhase::RollbackFailed,
            was_running: None,
            rollback_succeeded: Some(false),
            error: Some("rollback_failed".into()),
            updated_at_ms: now_ms(),
        };
        self.set_record(view).await;
        Err(StudioError::RollbackFailed {
            component: ComponentId::Fleet.to_string(),
            detail: format!("{reason}; {rollback_error}"),
        })
    }

    async fn update_failed(&self, transaction_id: &str, detail: String) -> StudioResult<()> {
        let mut transactions = self.transactions.lock().await;
        let record = transactions
            .get_mut(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        record.view.phase = McpUpdatePhase::Failed;
        record.view.error = Some(sanitize_transaction_error(&detail));
        record.view.updated_at_ms = now_ms();
        let view = record.view.clone();
        drop(transactions);
        self.emit(&view);
        Ok(())
    }

    async fn update_phase(
        &self,
        transaction_id: &str,
        phase: McpUpdatePhase,
        rollback_succeeded: Option<bool>,
        error: Option<String>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let mut transactions = self.transactions.lock().await;
        let record = transactions
            .get_mut(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        record.view.phase = phase;
        if let Some(value) = rollback_succeeded {
            record.view.rollback_succeeded = Some(value);
        }
        if let Some(error) = error {
            record.view.error = Some(sanitize_transaction_error(&error));
        }
        record.view.updated_at_ms = now_ms();
        let view = record.view.clone();
        drop(transactions);
        self.emit(&view);
        Ok(view)
    }

    async fn set_record(&self, mut view: McpUpdateTransactionView) {
        let mut transactions = self.transactions.lock().await;
        if let Some(record) = transactions.get_mut(&view.transaction_id) {
            if view.source_version.is_none() {
                view.source_version = record.view.source_version.clone();
            }
            record.view = view.clone();
        } else {
            transactions.insert(
                view.transaction_id.clone(),
                FleetTransactionRecord {
                    view: view.clone(),
                    staged_id: None,
                    staged_identity: None,
                    bundle_hashes: None,
                },
            );
        }
        drop(transactions);
        self.emit(&view);
    }

    fn emit(&self, view: &McpUpdateTransactionView) {
        self.events.publish(StudioEvent::UpdateTransaction {
            transaction: view.clone(),
        });
    }

    fn acquire(&self) -> StudioResult<FleetUpdateGuard> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| StudioError::UpdateTransaction("Fleet update lock poisoned".into()))?;
        if *active {
            return Err(StudioError::Conflict(
                "Fleet update already in progress".into(),
            ));
        }
        *active = true;
        Ok(FleetUpdateGuard {
            active: self.active.clone(),
        })
    }
}

#[derive(Debug)]
struct FleetUpdateGuard {
    active: Arc<StdMutex<bool>>,
}

impl Drop for FleetUpdateGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = false;
        }
    }
}

fn fleet_package_hashes(package_root: &Path) -> StudioResult<BTreeMap<PathBuf, String>> {
    let expected = BTreeSet::from([
        PathBuf::from("VERSION"),
        PathBuf::from("fleet.toml"),
        PathBuf::from("README.md"),
        PathBuf::from("scripts/fleetctl.py"),
        PathBuf::from("hosts/mirin.example.toml"),
    ]);
    let mut actual = BTreeSet::new();
    collect_regular_files(package_root, package_root, &mut actual)?;
    if actual != expected {
        return Err(StudioError::StagingFailure(
            "Fleet staged package file set changed after verification".into(),
        ));
    }
    let mut hashes = BTreeMap::new();
    for relative in expected {
        hashes.insert(
            relative.clone(),
            sha256_file(&package_root.join(&relative))?,
        );
    }
    Ok(hashes)
}

fn collect_regular_files(
    root: &Path,
    current: &Path,
    files: &mut BTreeSet<PathBuf>,
) -> StudioResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::StagingFailure(
                "Fleet staged package contains a symlink".into(),
            ));
        }
        if metadata.is_dir() {
            collect_regular_files(root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(StudioError::StagingFailure(
                "Fleet staged package contains an unsupported entry".into(),
            ));
        }
        files.insert(
            path.strip_prefix(root)
                .map_err(|_| StudioError::StagingFailure("Fleet package path escaped root".into()))?
                .to_owned(),
        );
    }
    Ok(())
}

fn sha256_file(path: &Path) -> StudioResult<String> {
    let mut input = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn read_fleet_version_optional(install: &Path) -> StudioResult<Option<Version>> {
    if !install.exists() {
        return Ok(None);
    }
    let path = install.join("VERSION");
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(StudioError::InstalledIdentity(
            "Fleet VERSION is not a regular file".into(),
        ));
    }
    let value = fs::read_to_string(path)?;
    Version::parse(value.trim())
        .map(Some)
        .map_err(|error| StudioError::InstalledIdentity(format!("invalid Fleet VERSION: {error}")))
}

fn read_fleet_version_required(install: &Path) -> StudioResult<Version> {
    read_fleet_version_optional(install)?.ok_or_else(|| StudioError::UpdateVerificationFailed {
        component: ComponentId::Fleet.to_string(),
        detail: "Fleet VERSION is missing".into(),
    })
}

fn candidate_path(root: &Path, transaction_id: &str, prefix: &str) -> StudioResult<PathBuf> {
    if !safe_fleet_transaction_id(transaction_id) {
        return Err(StudioError::UpdateTransaction(
            "invalid Fleet update transaction id".into(),
        ));
    }
    Ok(root.join(format!("{prefix}{transaction_id}")))
}

fn safe_fleet_transaction_id(value: &str) -> bool {
    value.starts_with("txn-fleet-")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn new_fleet_transaction_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "txn-fleet-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn copy_tree(source: &Path, destination: &Path) -> StudioResult<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet source bundle must be a regular directory".into(),
        ));
    }
    fs::create_dir(destination)?;
    copy_tree_inner(source, destination)?;
    sync_dir(destination)?;
    Ok(())
}

fn copy_tree_inner(source: &Path, destination: &Path) -> StudioResult<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src = entry.path();
        let dst = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&src)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::UpdateTransaction(
                "Fleet bundle copy rejected a symlink".into(),
            ));
        }
        if metadata.is_dir() {
            fs::create_dir(&dst)?;
            copy_tree_inner(&src, &dst)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet bundle copy found an unsupported entry".into(),
            ));
        }
        let mut input = File::open(&src)?;
        let mut output = File::create_new(&dst)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
        }
        output.flush()?;
        output.sync_all()?;
        fs::set_permissions(&dst, metadata.permissions())?;
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> StudioResult<()> {
    let mut file = File::create_new(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn activate_bundle(install: &Path, candidate: &Path, rollback: &Path) -> StudioResult<()> {
    if !install.is_dir() || !candidate.is_dir() || rollback.exists() {
        return Err(StudioError::UpdateActivationFailed {
            component: ComponentId::Fleet.to_string(),
            detail: "Fleet directory activation preconditions failed".into(),
        });
    }
    fs::rename(install, rollback).map_err(|error| StudioError::UpdateActivationFailed {
        component: ComponentId::Fleet.to_string(),
        detail: format!("failed to move prior Fleet bundle to rollback: {error}"),
    })?;
    let parent = install
        .parent()
        .ok_or_else(|| StudioError::UpdateTransaction("Fleet install path has no parent".into()))?;
    sync_dir(parent)?;
    if let Err(error) = fs::rename(candidate, install) {
        let _ = fs::rename(rollback, install);
        let _ = sync_dir(parent);
        return Err(StudioError::UpdateActivationFailed {
            component: ComponentId::Fleet.to_string(),
            detail: format!("failed to activate candidate Fleet bundle: {error}"),
        });
    }
    sync_dir(parent)?;
    Ok(())
}

fn recover_bundle_after_activation_error(
    install: &Path,
    candidate: &Path,
    rollback: &Path,
) -> StudioResult<()> {
    if !install.exists() && rollback.exists() {
        fs::rename(rollback, install)?;
    }
    let _ = remove_tree_if_exists(candidate);
    Ok(())
}

fn remove_tree_if_exists(path: &Path) -> StudioResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet transaction tree is not a regular directory".into(),
        ));
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn sync_dir(path: &Path) -> StudioResult<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn recover_interrupted_fleet_swap(catalog: &ComponentCatalog) -> StudioResult<()> {
    let runtime = catalog.runtime_root();
    if !runtime.exists() {
        return Ok(());
    }
    let install = catalog.install_path(ComponentId::Fleet)?;
    let mut rollbacks = Vec::new();
    for entry in fs::read_dir(runtime)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if name.starts_with(CANDIDATE_PREFIX) || name.starts_with(FAILED_PREFIX) {
            remove_tree_if_exists(&path)?;
        } else if name.starts_with(ROLLBACK_PREFIX) {
            rollbacks.push(path);
        }
    }
    if !install.exists() && rollbacks.len() == 1 {
        fs::rename(&rollbacks[0], &install)?;
        sync_dir(runtime)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as TestMutex;

    use tempfile::TempDir;

    use super::*;
    use crate::update::{
        Architecture, AvailableRelease, HostRuntimeRoots, OperatingSystem, Platform, ReleaseAsset,
        ReleaseProviderId,
    };

    struct NoopProvider;

    #[async_trait]
    impl ReleaseProvider for NoopProvider {
        fn provider_id(&self) -> ReleaseProviderId {
            ReleaseProviderId::ThirteenthXGitHub
        }

        async fn latest_release(
            &self,
            _component: &super::super::ComponentPolicy,
        ) -> StudioResult<AvailableRelease> {
            unreachable!()
        }

        async fn release(
            &self,
            _component: &super::super::ComponentPolicy,
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

    #[derive(Default)]
    struct FakeValidator {
        results: TestMutex<Vec<bool>>,
    }

    #[async_trait]
    impl FleetValidator for FakeValidator {
        async fn validate(&self, _bundle: &Path, _host_id: &str) -> StudioResult<()> {
            let mut results = self.results.lock().unwrap();
            let success = if results.is_empty() {
                true
            } else {
                results.remove(0)
            };
            if success {
                Ok(())
            } else {
                Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Fleet.to_string(),
                    detail: "forced Fleet render validation failure".into(),
                })
            }
        }
    }

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn host_profile(root: &Path, schema: u64) -> String {
        format!(
            "schema_version = {schema}\nhost_id = \"aira\"\nworkspace_root = \"{}\"\nsource_root = \"{}\"\nbin_root = \"{}\"\nruntime_root = \"{}\"\n\n[gateway]\nserver_dir = \"gateway/servers.d\"\n\n[servers.filesystem]\nenabled = true\n\n[servers.git]\nenabled = true\n\n[servers.exec]\nenabled = true\n",
            root.parent().unwrap().display(),
            root.join("missing-source").display(),
            root.join("bin").display(),
            root.join("runtime").display(),
        )
    }

    fn active_bundle(root: &Path, with_version: bool, host_schema: u64) -> PathBuf {
        let install = root.join("runtime/fleet");
        write_file(
            &install.join("fleet.toml"),
            b"schema_version = 2\nfleet_name = \"test\"\n",
        );
        write_file(&install.join("README.md"), b"old readme");
        write_file(
            &install.join("scripts/fleetctl.py"),
            b"#!/usr/bin/env python3\n",
        );
        write_file(
            &install.join("hosts/mirin.example.toml"),
            host_profile(root, 1)
                .replace("host_id = \"aira\"", "host_id = \"mirin\"")
                .as_bytes(),
        );
        write_file(
            &install.join("hosts/aira.toml"),
            host_profile(root, host_schema).as_bytes(),
        );
        write_file(
            &install.join("state/local.json"),
            b"{\"secret\":\"preserve\"}\n",
        );
        if with_version {
            write_file(&install.join("VERSION"), b"1.0.0\n");
        }
        install
    }

    fn create_ready(
        stager: &ArtifactStager,
        catalog: &ComponentCatalog,
        version: &str,
        ready_id: &str,
        fleet_schema: u64,
        host_schema: u64,
    ) -> StagedArtifact {
        let policy = catalog.component(ComponentId::Fleet).unwrap();
        let root = stager.staging_root().join(ready_id);
        let package_root = root.join("extracted").join(format!("mcp-fleet-v{version}"));
        write_file(
            &package_root.join("VERSION"),
            format!("{version}\n").as_bytes(),
        );
        write_file(
            &package_root.join("fleet.toml"),
            format!("schema_version = {fleet_schema}\nfleet_name = \"candidate\"\n").as_bytes(),
        );
        write_file(&package_root.join("README.md"), b"new readme");
        let script = package_root.join("scripts/fleetctl.py");
        write_file(&script, b"#!/usr/bin/env python3\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        write_file(
            &package_root.join("hosts/mirin.example.toml"),
            host_profile(catalog.runtime_root().parent().unwrap(), host_schema)
                .replace("host_id = \"aira\"", "host_id = \"mirin\"")
                .as_bytes(),
        );
        let archive_dir = root.join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        let asset_name = format!("mcp-fleet-v{version}.tar.gz");
        let archive = archive_dir.join(&asset_name);
        fs::write(&archive, b"verified-fleet-archive").unwrap();
        let archive_sha = digest(&archive);
        fs::write(
            archive_dir.join("SHA256SUMS.txt"),
            format!("{archive_sha}  {asset_name}\n"),
        )
        .unwrap();
        let script_sha = digest(&script);
        let staged = StagedArtifact {
            component: ComponentId::Fleet,
            version: Version::parse(version).unwrap(),
            provider: policy.provider,
            release_tag: format!("v{version}"),
            platform: platform(),
            asset_name,
            archive_sha256: archive_sha,
            companion_asset_name: None,
            companion_archive_sha256: None,
            staging_path: root.clone(),
            package_root,
            validated_executables: vec![script],
            validated_executable_sha256: vec![script_sha],
            verified_at_unix_seconds: 1,
        };
        fs::write(
            root.join("staged.json"),
            serde_json::to_vec_pretty(&staged).unwrap(),
        )
        .unwrap();
        staged
    }

    fn digest(path: &Path) -> String {
        let mut input = File::open(path).unwrap();
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = input.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn fixture(
        with_version: bool,
        host_schema: u64,
        validator_results: Vec<bool>,
    ) -> (TempDir, FleetUpdateManager, Arc<FakeValidator>, PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("runtime")).unwrap();
        for binary in [
            "rust-mcp-filesystem",
            "rust-mcp-git",
            "rust-mcp-exec",
            "rust-mcp-gateway",
            "rust-mcp-blender",
        ] {
            write_file(&root.join("bin").join(binary), b"bin");
        }
        let install = active_bundle(root, with_version, host_schema);
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.join("bin"), root.join("runtime")).unwrap(),
        );
        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let validator = Arc::new(FakeValidator {
            results: TestMutex::new(validator_results),
        });
        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.join("missing-source"),
                platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let manager = FleetUpdateManager::with_dependencies(
            catalog,
            stager,
            Arc::new(NoopProvider),
            validator.clone(),
            inventory,
            EventHub::default(),
        );
        (temp, manager, validator, install)
    }

    async fn register_ready(
        manager: &FleetUpdateManager,
        staged: &StagedArtifact,
        tx: &str,
        ready_id: &str,
    ) {
        let source_version =
            read_fleet_version_optional(&manager.catalog.install_path(ComponentId::Fleet).unwrap())
                .unwrap();
        manager.transactions.lock().await.insert(
            tx.to_owned(),
            FleetTransactionRecord {
                view: McpUpdateTransactionView {
                    transaction_id: tx.to_owned(),
                    component: ComponentId::Fleet,
                    source_version,
                    target_version: staged.version.clone(),
                    phase: McpUpdatePhase::Staged,
                    was_running: None,
                    rollback_succeeded: None,
                    error: None,
                    updated_at_ms: now_ms(),
                },
                staged_id: Some(ready_id.to_owned()),
                staged_identity: Some(PreparedStagedIdentity::from(staged)),
                bundle_hashes: Some(fleet_package_hashes(&staged.package_root).unwrap()),
            },
        );
    }

    #[tokio::test]
    async fn legacy_bundle_updates_and_preserves_profile_state_and_example_boundary() {
        let (_temp, manager, _validator, install) = fixture(false, 1, vec![true, true]);
        let profile_before = fs::read(install.join("hosts/aira.toml")).unwrap();
        let profile_mode_before =
            file_mode(&fs::metadata(install.join("hosts/aira.toml")).unwrap());
        let state_before = fs::read(install.join("state/local.json")).unwrap();
        let state_mode_before = file_mode(&fs::metadata(install.join("state/local.json")).unwrap());
        let ready_id = "ready-fleet-success";
        let tx = "txn-fleet-success";
        let staged = create_ready(&manager.stager, &manager.catalog, "0.2.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;

        let result = manager.apply(tx).await.unwrap();
        assert_eq!(result.phase, McpUpdatePhase::Completed);
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "0.2.0"
        );
        assert_eq!(
            fs::read(install.join("hosts/aira.toml")).unwrap(),
            profile_before
        );
        assert_eq!(
            fs::read(install.join("state/local.json")).unwrap(),
            state_before
        );
        assert_eq!(
            file_mode(&fs::metadata(install.join("hosts/aira.toml")).unwrap()),
            profile_mode_before
        );
        assert_eq!(
            file_mode(&fs::metadata(install.join("state/local.json")).unwrap()),
            state_mode_before
        );
        assert!(
            fs::read_to_string(install.join("hosts/mirin.example.toml"))
                .unwrap()
                .contains("host_id = \"mirin\"")
        );
    }

    #[tokio::test]
    async fn schema_incompatibility_and_render_failure_leave_runtime_unchanged() {
        let (_temp, manager, _validator, install) = fixture(true, 1, vec![]);
        let before = fs::read(install.join("README.md")).unwrap();
        let ready_id = "ready-fleet-schema";
        let tx = "txn-fleet-schema";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 3, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(fs::read(install.join("README.md")).unwrap(), before);
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );

        let (_temp, manager, _validator, install) = fixture(true, 1, vec![false]);
        let ready_id = "ready-fleet-render";
        let tx = "txn-fleet-render";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );
    }

    #[tokio::test]
    async fn incompatible_active_host_schema_fails_before_activation() {
        let (_temp, manager, _validator, install) = fixture(true, 2, vec![]);
        let ready_id = "ready-fleet-hostschema";
        let tx = "txn-fleet-hostschema";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );
    }

    #[tokio::test]
    async fn post_activation_validation_failure_rolls_back_generic_files_and_profile() {
        let (_temp, manager, _validator, install) = fixture(true, 1, vec![true, false, true]);
        let profile_before = fs::read(install.join("hosts/aira.toml")).unwrap();
        let readme_before = fs::read(install.join("README.md")).unwrap();
        let ready_id = "ready-fleet-rollback";
        let tx = "txn-fleet-rollback";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());

        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );
        assert_eq!(fs::read(install.join("README.md")).unwrap(), readme_before);
        assert_eq!(
            fs::read(install.join("hosts/aira.toml")).unwrap(),
            profile_before
        );
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn staged_tamper_duplicate_lock_and_unexpected_local_file_fail_closed() {
        let (_temp, manager, _validator, install) = fixture(true, 1, vec![]);
        let _guard = manager.acquire().unwrap();
        assert!(matches!(
            manager.acquire().unwrap_err(),
            StudioError::Conflict(_)
        ));
        drop(_guard);

        write_file(
            &install.join("secret.txt"),
            b"must not be silently discarded",
        );
        let ready_id = "ready-fleet-local";
        let tx = "txn-fleet-local";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());
        assert!(install.join("secret.txt").exists());

        fs::remove_file(install.join("secret.txt")).unwrap();
        let (_temp, manager, _validator, install) = fixture(true, 1, vec![]);
        let ready_id = "ready-fleet-tamper";
        let tx = "txn-fleet-tamper";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        fs::write(&staged.validated_executables[0], b"tampered").unwrap();
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );

        let (_temp, manager, _validator, install) = fixture(true, 1, vec![]);
        let ready_id = "ready-fleet-metadata-tamper";
        let tx = "txn-fleet-metadata-tamper";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        fs::write(
            staged.package_root.join("fleet.toml"),
            b"schema_version = 2\ntampered = true\n",
        )
        .unwrap();
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );

        let (_temp, manager, _validator, install) = fixture(true, 1, vec![]);
        let ready_id = "ready-fleet-extra-file";
        let tx = "txn-fleet-extra-file";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id, 2, 1);
        register_ready(&manager, &staged, tx, ready_id).await;
        write_file(&staged.package_root.join("scripts/extra.py"), b"bad");
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(
            read_fleet_version_required(&install).unwrap().to_string(),
            "1.0.0"
        );
    }

    #[test]
    fn interrupted_swap_recovers_missing_active_bundle_from_single_rollback() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("runtime")).unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.join("bin"), root.join("runtime")).unwrap(),
        );
        let rollback = root
            .join("runtime")
            .join(".mcp-studio-fleet-rollback-txn-fleet-crash");
        fs::create_dir_all(&rollback).unwrap();
        fs::write(rollback.join("README.md"), b"recover").unwrap();

        recover_interrupted_fleet_swap(&catalog).unwrap();
        assert_eq!(
            fs::read(root.join("runtime/fleet/README.md")).unwrap(),
            b"recover"
        );
    }

    #[tokio::test]
    async fn fleet_validation_uses_pure_render_plan() {
        let temp = TempDir::new().unwrap();
        let candidate = temp.path().join("fleet");
        fs::create_dir_all(candidate.join("scripts")).unwrap();
        fs::write(
            candidate.join("scripts/fleetctl.py"),
            r#"import json
import sys

expected = ["render-plan", "--host", "test-host", "--json"]
if sys.argv[1:] != expected:
    print(f"unexpected arguments: {sys.argv[1:]}", file=sys.stderr)
    raise SystemExit(2)
print(json.dumps({"outputs": []}))
"#,
        )
        .unwrap();

        ProcessFleetValidator
            .validate(&candidate, "test-host")
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "explicit Aira Fleet render validation smoke; requires adjacent Fleet checkout"]
    async fn live_aira_fleet_render_validation_smoke() {
        let temp = TempDir::new().unwrap();
        let candidate = temp.path().join("fleet");
        fs::create_dir_all(candidate.join("scripts")).unwrap();
        fs::create_dir_all(candidate.join("hosts")).unwrap();
        fs::create_dir_all(candidate.join("launchers")).unwrap();
        for relative in [
            "VERSION",
            "fleet.toml",
            "README.md",
            "scripts/fleetctl.py",
            "hosts/mirin.example.toml",
        ] {
            let source = PathBuf::from("../fleet").join(relative);
            let destination = candidate.join(relative);
            fs::copy(source, destination).unwrap();
        }
        fs::copy(
            "../runtime/fleet/hosts/aira.toml",
            candidate.join("hosts/aira.toml"),
        )
        .unwrap();
        fs::copy(
            "../fleet/launchers/sonarqube-mcp",
            candidate.join("launchers/sonarqube-mcp"),
        )
        .unwrap();
        ProcessFleetValidator
            .validate(&candidate, "aira")
            .await
            .unwrap();
    }
}
