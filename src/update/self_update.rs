use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{process::Command, time::timeout};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
};

use super::{
    ArtifactHistoryIdentity, ArtifactStager, ComponentCatalog, ComponentId, InventoryService,
    McpUpdatePhase, McpUpdateTransactionView, ReleaseProvider, ReleaseProviderId, StagedArtifact,
    ThirteenthXReleaseProvider, Version,
    transaction::{now_ms, sanitize_transaction_error},
};

const SELF_UPDATE_SCHEMA_VERSION: u32 = 2;
const SELF_UPDATE_LEGACY_SCHEMA_VERSION: u32 = 1;
const FLEET_ACTIVATION_PROTOCOL: u32 = 2;
const VERSION_VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
const LAUNCHER_VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RELEASE_FILES: usize = 8192;
const MAX_RELEASE_BYTES: u64 = 1024 * 1024 * 1024;
const TRANSACTION_PREFIX: &str = "txn-studio-";
const CANDIDATE_PREFIX: &str = ".candidate-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SelfUpdatePhase {
    Preparing,
    Staged,
    ActivationPending,
    ExternalActivating,
    ExternalActivated,
    RollingBack,
    RolledBack,
    Completed,
    ActivationFailed,
    RollbackFailed,
}

impl SelfUpdatePhase {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::RolledBack | Self::Completed | Self::ActivationFailed | Self::RollbackFailed
        )
    }

    fn public(self) -> McpUpdatePhase {
        match self {
            Self::Preparing => McpUpdatePhase::Preparing,
            Self::Staged => McpUpdatePhase::Staged,
            Self::ActivationPending => McpUpdatePhase::ActivationPending,
            Self::ExternalActivating => McpUpdatePhase::ExternalActivating,
            Self::ExternalActivated => McpUpdatePhase::HealthVerifying,
            Self::RollingBack => McpUpdatePhase::RollingBack,
            Self::RolledBack => McpUpdatePhase::RolledBack,
            Self::Completed => McpUpdatePhase::Completed,
            Self::ActivationFailed => McpUpdatePhase::Failed,
            Self::RollbackFailed => McpUpdatePhase::RollbackFailed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StagedFingerprint {
    version: Version,
    provider: ReleaseProviderId,
    asset_name: String,
    archive_sha256: String,
    executable_sha256: Vec<String>,
}

impl From<&StagedArtifact> for StagedFingerprint {
    fn from(staged: &StagedArtifact) -> Self {
        Self {
            version: staged.version.clone(),
            provider: staged.provider,
            asset_name: staged.asset_name.clone(),
            archive_sha256: staged.archive_sha256.clone(),
            executable_sha256: staged.validated_executable_sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SelfUpdateRecord {
    schema_version: u32,
    transaction_id: String,
    component: String,
    source_version: Version,
    target_version: Version,
    phase: SelfUpdatePhase,
    candidate_dir: String,
    target_release: String,
    candidate_fingerprint: Option<String>,
    staged_id: Option<String>,
    staged_fingerprint: Option<StagedFingerprint>,
    parent_pid: u32,
    previous_layout: Option<String>,
    previous_release: Option<String>,
    rollback_succeeded: Option<bool>,
    error: Option<String>,
    #[serde(default)]
    journal_revision: u64,
    #[serde(default)]
    launcher_protocol: Option<u32>,
    #[serde(default)]
    launcher_owner: Option<String>,
    #[serde(default)]
    launched_pid: Option<u32>,
    #[serde(default)]
    launched_release_fingerprint: Option<String>,
    updated_at_ms: u128,
}

impl SelfUpdateRecord {
    fn view(&self) -> McpUpdateTransactionView {
        McpUpdateTransactionView {
            transaction_id: self.transaction_id.clone(),
            component: ComponentId::Studio,
            source_version: Some(self.source_version.clone()),
            target_version: self.target_version.clone(),
            phase: self.phase.public(),
            was_running: Some(true),
            rollback_succeeded: self.rollback_succeeded,
            error: self.error.clone(),
            updated_at_ms: self.updated_at_ms,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FleetStudioContract {
    activation_protocol: u32,
    schema_versions: Vec<u32>,
    process_bound_readiness: bool,
    cross_process_lock: bool,
}

#[async_trait]
trait LauncherRequester: Send + Sync {
    async fn validate_contract(&self) -> StudioResult<()>;
    async fn request(
        &self,
        host_id: &str,
        transaction_id: &str,
        parent_pid: u32,
    ) -> StudioResult<()>;
}

struct FleetLauncherRequester {
    runtime_root: PathBuf,
}

impl FleetLauncherRequester {
    fn new(runtime_root: PathBuf) -> Self {
        Self { runtime_root }
    }

    fn script(&self) -> PathBuf {
        self.runtime_root.join("fleet/scripts/fleetctl.py")
    }

    async fn validate_available(&self) -> StudioResult<()> {
        let script = self.script();
        let metadata = fs::symlink_metadata(&script).map_err(|error| {
            StudioError::UpdateTransaction(format!("Fleet Studio launcher is unavailable: {error}"))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet Studio launcher is not a regular file".into(),
            ));
        }
        let fleet_root = self.runtime_root.join("fleet");
        let mut command = Command::new("python3");
        command
            .arg(&script)
            .arg("studio-contract")
            .arg("--json")
            .current_dir(&fleet_root)
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .kill_on_drop(true);
        let output = timeout(LAUNCHER_VERIFY_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                StudioError::UpdateTransaction("Fleet Studio launcher validation timed out".into())
            })?
            .map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "Fleet Studio launcher validation failed: {error}"
                ))
            })?;
        if !output.status.success() {
            return Err(StudioError::UpdateTransaction(
                "deployed Fleet bundle does not support Studio activation protocol discovery"
                    .into(),
            ));
        }
        let contract: FleetStudioContract =
            serde_json::from_slice(&output.stdout).map_err(|_| {
                StudioError::UpdateTransaction("Fleet Studio launcher contract is malformed".into())
            })?;
        if contract.activation_protocol != FLEET_ACTIVATION_PROTOCOL
            || !contract
                .schema_versions
                .contains(&SELF_UPDATE_SCHEMA_VERSION)
            || !contract.process_bound_readiness
            || !contract.cross_process_lock
        {
            return Err(StudioError::UpdateTransaction(
                "deployed Fleet bundle is incompatible with Studio self-update protocol v2".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl LauncherRequester for FleetLauncherRequester {
    async fn validate_contract(&self) -> StudioResult<()> {
        self.validate_available().await
    }

    async fn request(
        &self,
        host_id: &str,
        transaction_id: &str,
        parent_pid: u32,
    ) -> StudioResult<()> {
        let script = self.script();
        let fleet_root = self.runtime_root.join("fleet");
        Command::new("python3")
            .arg(&script)
            .arg("studio-activate")
            .arg("--host")
            .arg(host_id)
            .arg("--transaction")
            .arg(transaction_id)
            .arg("--parent-pid")
            .arg(parent_pid.to_string())
            .current_dir(fleet_root)
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "failed to start Fleet Studio launcher: {error}"
                ))
            })?;
        Ok(())
    }
}

trait CurrentProcessIdentity: Send + Sync {
    fn pid(&self) -> u32;
    fn executable(&self) -> StudioResult<PathBuf>;
    fn version(&self) -> StudioResult<Version>;
}

struct RuntimeProcessIdentity;

impl CurrentProcessIdentity for RuntimeProcessIdentity {
    fn pid(&self) -> u32 {
        std::process::id()
    }

    fn executable(&self) -> StudioResult<PathBuf> {
        std::env::current_exe().map_err(Into::into)
    }

    fn version(&self) -> StudioResult<Version> {
        Version::parse(env!("CARGO_PKG_VERSION"))
    }
}

#[derive(Clone)]
pub struct SelfUpdateManager {
    catalog: ComponentCatalog,
    stager: ArtifactStager,
    provider: Arc<dyn ReleaseProvider>,
    inventory: Arc<InventoryService>,
    launcher: Arc<dyn LauncherRequester>,
    process: Arc<dyn CurrentProcessIdentity>,
    events: EventHub,
    active: Arc<StdMutex<bool>>,
}

impl SelfUpdateManager {
    pub fn new(
        catalog: ComponentCatalog,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> StudioResult<Self> {
        let stager = ArtifactStager::new(catalog.clone())?;
        let provider = Arc::new(ThirteenthXReleaseProvider::new(catalog.clone())?);
        let manager = Self {
            catalog: catalog.clone(),
            stager,
            provider,
            inventory,
            launcher: Arc::new(FleetLauncherRequester::new(
                catalog.runtime_root().to_path_buf(),
            )),
            process: Arc::new(RuntimeProcessIdentity),
            events,
            active: Arc::new(StdMutex::new(false)),
        };
        manager.ensure_state_roots()?;
        Ok(manager)
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        stager: ArtifactStager,
        provider: Arc<dyn ReleaseProvider>,
        inventory: Arc<InventoryService>,
        launcher: Arc<dyn LauncherRequester>,
        process: Arc<dyn CurrentProcessIdentity>,
        events: EventHub,
    ) -> StudioResult<Self> {
        let manager = Self {
            catalog,
            stager,
            provider,
            inventory,
            launcher,
            process,
            events,
            active: Arc::new(StdMutex::new(false)),
        };
        manager.ensure_state_roots()?;
        Ok(manager)
    }

    pub async fn prepare(&self, target: Version) -> StudioResult<McpUpdateTransactionView> {
        let _guard = self.acquire()?;
        self.ensure_no_active_transaction()?;
        let source = self.current_installed_version().await?;
        if !target.is_newer_than(&source) {
            return Err(StudioError::Conflict(format!(
                "studio: target {target} must be newer than installed {source}"
            )));
        }

        let transaction_id = new_transaction_id();
        let mut record = SelfUpdateRecord {
            schema_version: SELF_UPDATE_SCHEMA_VERSION,
            transaction_id: transaction_id.clone(),
            component: ComponentId::Studio.to_string(),
            source_version: source,
            target_version: target.clone(),
            phase: SelfUpdatePhase::Preparing,
            candidate_dir: format!("{CANDIDATE_PREFIX}{transaction_id}"),
            target_release: format!("v{target}"),
            candidate_fingerprint: None,
            staged_id: None,
            staged_fingerprint: None,
            parent_pid: 0,
            previous_layout: None,
            previous_release: None,
            rollback_succeeded: None,
            error: None,
            journal_revision: 0,
            launcher_protocol: Some(FLEET_ACTIVATION_PROTOCOL),
            launcher_owner: None,
            launched_pid: None,
            launched_release_fingerprint: None,
            updated_at_ms: now_ms(),
        };
        self.write_record(&record)?;
        self.emit(&record.view());

        let result = async {
            let policy = self.catalog.component(ComponentId::Studio)?.clone();
            let release = self.provider.release(&policy, &target).await?;
            let staged = self
                .stager
                .stage(self.provider.as_ref(), &release, self.inventory.platform())
                .await?;
            if staged.component != ComponentId::Studio || staged.version != target {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Studio.to_string(),
                    detail: "staged Studio identity does not match requested target".into(),
                });
            }
            let staged_id = self.stager.staged_id(&staged)?;
            let candidate = self.prepare_candidate(&record, &staged).await?;
            let candidate_fingerprint = release_tree_fingerprint(&candidate)?;
            Ok::<_, StudioError>((
                staged_id,
                StagedFingerprint::from(&staged),
                candidate_fingerprint,
            ))
        }
        .await;

        match result {
            Ok((staged_id, staged_fingerprint, candidate_fingerprint)) => {
                record.staged_id = Some(staged_id);
                record.staged_fingerprint = Some(staged_fingerprint);
                record.candidate_fingerprint = Some(candidate_fingerprint);
                record.phase = SelfUpdatePhase::Staged;
                record.updated_at_ms = now_ms();
                self.write_record(&record)?;
                let view = record.view();
                self.emit(&view);
                Ok(view)
            }
            Err(error) => {
                let _ = self.remove_candidate(&record);
                record.phase = SelfUpdatePhase::ActivationFailed;
                record.error = Some(sanitize_transaction_error(&error.to_string()));
                record.updated_at_ms = now_ms();
                let _ = self.write_record(&record);
                self.emit(&record.view());
                Err(error)
            }
        }
    }

    pub async fn apply(&self, transaction_id: &str) -> StudioResult<McpUpdateTransactionView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("studio_self_update")?;
        let _guard = self.acquire()?;
        let mut record = self.read_record(transaction_id)?;
        let resumable = matches!(
            record.phase,
            SelfUpdatePhase::Staged
                | SelfUpdatePhase::ActivationPending
                | SelfUpdatePhase::ExternalActivating
                | SelfUpdatePhase::ExternalActivated
        );
        if !resumable {
            return Err(StudioError::Conflict(format!(
                "Studio self-update transaction {transaction_id} is not resumable"
            )));
        }

        if record.phase == SelfUpdatePhase::Staged {
            let installed = self.current_installed_version().await?;
            if installed != record.source_version {
                return Err(StudioError::Conflict(
                    "Studio installed version changed after self-update preparation".into(),
                ));
            }
        }

        self.validate_running_process(&record.source_version)?;
        self.revalidate_staged_and_candidate(&record).await?;
        let host_id = active_fleet_host_id(self.catalog.runtime_root())?;
        self.validate_launcher_contract(&host_id)?;
        self.launcher.validate_contract().await?;

        record.parent_pid = self.process.pid();
        record.launcher_protocol = Some(FLEET_ACTIVATION_PROTOCOL);
        record.phase = SelfUpdatePhase::ActivationPending;
        record.error = None;
        record.rollback_succeeded = None;
        record.updated_at_ms = now_ms();
        self.write_record(&record)?;
        self.emit(&record.view());

        if let Err(error) = self
            .launcher
            .request(&host_id, transaction_id, record.parent_pid)
            .await
        {
            record.phase = SelfUpdatePhase::ActivationFailed;
            record.error = Some("launcher_unavailable".into());
            record.updated_at_ms = now_ms();
            self.write_record(&record)?;
            self.emit(&record.view());
            return Err(error);
        }

        Ok(record.view())
    }

    pub(crate) fn artifact_history_identity(
        &self,
        transaction_id: &str,
    ) -> StudioResult<Option<ArtifactHistoryIdentity>> {
        let record = self.read_record(transaction_id)?;
        Ok(record
            .staged_fingerprint
            .as_ref()
            .map(|staged| ArtifactHistoryIdentity {
                component: ComponentId::Studio,
                version: staged.version.to_string(),
                provider_code: match staged.provider {
                    ReleaseProviderId::ThirteenthXGitHub => "github_13thx",
                    ReleaseProviderId::OpenAiGitHub => "github_openai",
                },
                platform_code: self.inventory.platform().to_string(),
                archive_sha256: staged.archive_sha256.clone(),
                member_sha256: staged.executable_sha256.clone(),
            }))
    }

    pub async fn transaction(
        &self,
        transaction_id: &str,
    ) -> StudioResult<McpUpdateTransactionView> {
        let record = self.read_record(transaction_id)?;
        if record.phase == SelfUpdatePhase::Completed {
            self.inventory
                .set_desired(ComponentId::Studio, record.target_version.clone())
                .await;
        }
        Ok(record.view())
    }

    pub async fn finalize_startup(&self) -> StudioResult<Vec<McpUpdateTransactionView>> {
        let records = self.list_records()?;
        let mut observed = Vec::new();
        for mut record in records {
            match record.phase {
                SelfUpdatePhase::Preparing => {
                    let _ = self.remove_candidate(&record);
                    record.phase = SelfUpdatePhase::ActivationFailed;
                    record.error = Some("interrupted_prepare".into());
                    record.updated_at_ms = now_ms();
                    self.write_record(&record)?;
                    let view = record.view();
                    self.emit(&view);
                    observed.push(view);
                }
                SelfUpdatePhase::Completed => {
                    self.inventory
                        .set_desired(ComponentId::Studio, record.target_version.clone())
                        .await;
                    let view = record.view();
                    self.emit(&view);
                    observed.push(view);
                }
                SelfUpdatePhase::RolledBack
                | SelfUpdatePhase::RollbackFailed
                | SelfUpdatePhase::ActivationFailed => {
                    let view = record.view();
                    self.emit(&view);
                    observed.push(view);
                }
                SelfUpdatePhase::ExternalActivating
                | SelfUpdatePhase::ExternalActivated
                | SelfUpdatePhase::RollingBack
                | SelfUpdatePhase::ActivationPending => {
                    // Fleet is the sole authority for terminal activation/rollback
                    // transitions. Local executable/pointer identity is diagnostic
                    // only and must not finalize the journal.
                    let view = record.view();
                    self.emit(&view);
                    observed.push(view);
                }
                SelfUpdatePhase::Staged => {}
            }
        }
        Ok(observed)
    }

    fn acquire(&self) -> StudioResult<SelfUpdateGuard> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| StudioError::UpdateTransaction("Studio update lock poisoned".into()))?;
        if *active {
            return Err(StudioError::Conflict(
                "Studio self-update already in progress".into(),
            ));
        }
        *active = true;
        Ok(SelfUpdateGuard {
            active: self.active.clone(),
        })
    }

    fn ensure_state_roots(&self) -> StudioResult<()> {
        let studio_root = self.studio_root()?;
        ensure_regular_dir(&studio_root)?;
        ensure_regular_dir(&studio_root.join("data"))?;
        ensure_regular_dir(&self.transaction_root()?)?;
        ensure_regular_dir(&self.releases_root()?)?;
        Ok(())
    }

    fn ensure_no_active_transaction(&self) -> StudioResult<()> {
        for record in self.list_records()? {
            if !record.phase.is_terminal() || record.phase == SelfUpdatePhase::RollbackFailed {
                return Err(StudioError::Conflict(format!(
                    "Studio self-update transaction {} is still active",
                    record.transaction_id
                )));
            }
        }
        Ok(())
    }

    async fn current_installed_version(&self) -> StudioResult<Version> {
        let view = self.inventory.get(ComponentId::Studio).await?;
        view.installed_version.ok_or_else(|| {
            StudioError::InstalledIdentity("Studio installed version is unavailable".into())
        })
    }

    fn studio_root(&self) -> StudioResult<PathBuf> {
        self.catalog.install_path(ComponentId::Studio)
    }

    fn transaction_root(&self) -> StudioResult<PathBuf> {
        Ok(self.studio_root()?.join("data/self-update"))
    }

    fn releases_root(&self) -> StudioResult<PathBuf> {
        Ok(self.studio_root()?.join("releases"))
    }

    fn record_path(&self, transaction_id: &str) -> StudioResult<PathBuf> {
        if !safe_transaction_id(transaction_id) {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        Ok(self
            .transaction_root()?
            .join(format!("{transaction_id}.json")))
    }

    fn candidate_path(&self, record: &SelfUpdateRecord) -> StudioResult<PathBuf> {
        if record.candidate_dir != format!("{CANDIDATE_PREFIX}{}", record.transaction_id) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio candidate identity mismatch".into(),
            });
        }
        let releases = self.releases_root()?;
        let candidate = releases.join(&record.candidate_dir);
        if candidate.parent() != Some(releases.as_path()) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio candidate escaped releases root".into(),
            });
        }
        Ok(candidate)
    }

    async fn prepare_candidate(
        &self,
        record: &SelfUpdateRecord,
        staged: &StagedArtifact,
    ) -> StudioResult<PathBuf> {
        let candidate = self.candidate_path(record)?;
        if candidate.exists() {
            return Err(StudioError::Conflict(
                "Studio self-update candidate already exists".into(),
            ));
        }
        copy_release_tree(&staged.package_root, &candidate)?;
        if let Err(error) = validate_release_candidate(&candidate, &record.target_version).await {
            let _ = remove_regular_tree(&candidate);
            return Err(error);
        }
        Ok(candidate)
    }

    fn remove_candidate(&self, record: &SelfUpdateRecord) -> StudioResult<()> {
        remove_regular_tree(&self.candidate_path(record)?)
    }

    async fn revalidate_staged_and_candidate(&self, record: &SelfUpdateRecord) -> StudioResult<()> {
        let staged_id =
            record
                .staged_id
                .as_deref()
                .ok_or_else(|| StudioError::UpdateVerificationFailed {
                    component: ComponentId::Studio.to_string(),
                    detail: "Studio transaction has no staged artifact identity".into(),
                })?;
        let expected_staged = record.staged_fingerprint.as_ref().ok_or_else(|| {
            StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio transaction has no staging fingerprint".into(),
            }
        })?;
        let staged = self.stager.load_ready(staged_id)?;
        if staged.component != ComponentId::Studio
            || staged.version != record.target_version
            || StagedFingerprint::from(&staged) != *expected_staged
        {
            return Err(StudioError::Conflict(
                "Studio staged artifact changed after preparation".into(),
            ));
        }

        let candidate = self.candidate_path(record)?;
        let prepared_release = if candidate.exists() {
            candidate
        } else if record.phase != SelfUpdatePhase::Staged {
            let target = self.releases_root()?.join(&record.target_release);
            if !target.exists() {
                return Err(StudioError::Conflict(
                    "Studio prepared release is unavailable for recovery".into(),
                ));
            }
            target
        } else {
            return Err(StudioError::Conflict(
                "Studio self-update candidate is unavailable".into(),
            ));
        };
        validate_release_candidate(&prepared_release, &record.target_version).await?;
        let expected_candidate = record.candidate_fingerprint.as_deref().ok_or_else(|| {
            StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio transaction has no candidate fingerprint".into(),
            }
        })?;
        if release_tree_fingerprint(&prepared_release)? != expected_candidate {
            return Err(StudioError::Conflict(
                "Studio self-update candidate changed after preparation".into(),
            ));
        }
        Ok(())
    }

    fn validate_running_process(&self, source_version: &Version) -> StudioResult<()> {
        if self.process.version()? != *source_version {
            return Err(StudioError::Conflict(
                "running Studio version does not match installed update source".into(),
            ));
        }
        let executable = fs::canonicalize(self.process.executable()?)?;
        let studio_root = self.studio_root()?;
        let legacy = studio_root.join("mcp-studio");
        let mut allowed = BTreeSet::new();
        if legacy.exists() {
            allowed.insert(fs::canonicalize(&legacy)?);
        }
        let current = studio_root.join("current");
        if current.exists() || current.is_symlink() {
            if !current.is_symlink() {
                return Err(StudioError::Conflict(
                    "Studio current activation pointer is not a symlink".into(),
                ));
            }
            allowed.insert(fs::canonicalize(current)?.join("mcp-studio"));
        }
        if !allowed.contains(&executable) {
            return Err(StudioError::Conflict(
                "Studio self-update is available only from the deployed Studio runtime".into(),
            ));
        }
        Ok(())
    }

    fn validate_launcher_contract(&self, host_id: &str) -> StudioResult<()> {
        let hosts = self.catalog.runtime_root().join("fleet/hosts");
        let host = hosts.join(format!("{host_id}.toml"));
        let metadata = fs::symlink_metadata(&host)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "active Fleet host profile is unavailable".into(),
            ));
        }
        let script = self
            .catalog
            .runtime_root()
            .join("fleet/scripts/fleetctl.py");
        let metadata = fs::symlink_metadata(&script)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet Studio launcher is unavailable".into(),
            ));
        }
        Ok(())
    }

    fn write_record(&self, record: &SelfUpdateRecord) -> StudioResult<()> {
        let root = self.transaction_root()?;
        ensure_regular_dir(&root)?;
        let path = self.record_path(&record.transaction_id)?;
        let temp = root.join(format!(
            ".{}.{}.tmp",
            record.transaction_id,
            std::process::id()
        ));
        if temp.exists() {
            fs::remove_file(&temp)?;
        }
        let next_revision = if path.exists() {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            value
                .get("journal_revision")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                .saturating_add(1)
        } else {
            1
        };
        let mut durable = record.clone();
        durable.journal_revision = next_revision;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut bytes = serde_json::to_vec_pretty(&durable)?;
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&temp, &path)?;
        File::open(&root)?.sync_all()?;
        Ok(())
    }

    fn read_record(&self, transaction_id: &str) -> StudioResult<SelfUpdateRecord> {
        let path = self.record_path(transaction_id)?;
        if !path.exists() {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio self-update metadata is not a regular file".into(),
            });
        }
        let record: SelfUpdateRecord = serde_json::from_slice(&fs::read(path)?)?;
        validate_record_identity(&record, transaction_id)?;
        Ok(record)
    }

    fn list_records(&self) -> StudioResult<Vec<SelfUpdateRecord>> {
        let root = self.transaction_root()?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let metadata = fs::symlink_metadata(&root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio self-update state root is unsafe".into(),
            });
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if safe_transaction_id(stem) {
                records.push(self.read_record(stem)?);
            }
        }
        records.sort_by(|left, right| left.transaction_id.cmp(&right.transaction_id));
        Ok(records)
    }

    fn emit(&self, view: &McpUpdateTransactionView) {
        self.events.publish(StudioEvent::UpdateTransaction {
            transaction: view.clone(),
        });
    }
}

struct SelfUpdateGuard {
    active: Arc<StdMutex<bool>>,
}

impl Drop for SelfUpdateGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = false;
        }
    }
}

fn validate_record_identity(record: &SelfUpdateRecord, transaction_id: &str) -> StudioResult<()> {
    if !matches!(
        record.schema_version,
        SELF_UPDATE_LEGACY_SCHEMA_VERSION | SELF_UPDATE_SCHEMA_VERSION
    ) || record.transaction_id != transaction_id
        || record.component != ComponentId::Studio.as_str()
        || record.candidate_dir != format!("{CANDIDATE_PREFIX}{transaction_id}")
        || record.target_release != format!("v{}", record.target_version)
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio self-update metadata identity/schema mismatch".into(),
        });
    }
    if record.schema_version == SELF_UPDATE_SCHEMA_VERSION
        && record.launcher_protocol != Some(FLEET_ACTIVATION_PROTOCOL)
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio self-update launcher protocol mismatch".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct StudioActivationReadyProof<'a> {
    schema_version: u32,
    transaction_id: &'a str,
    nonce: &'a str,
    pid: u32,
    config_path: String,
    config_sha256: &'a str,
}

pub fn write_activation_ready_proof_from_env(
    runtime_root: &Path,
    loaded_config: &crate::config::LoadedConfigIdentity,
) -> StudioResult<bool> {
    let transaction_id = std::env::var("MCP_STUDIO_ACTIVATION_TRANSACTION").ok();
    let nonce = std::env::var("MCP_STUDIO_ACTIVATION_NONCE").ok();
    match (transaction_id, nonce) {
        (None, None) => Ok(false),
        (Some(_), None) | (None, Some(_)) => Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "incomplete Studio activation proof environment".into(),
        }),
        (Some(transaction_id), Some(nonce)) => {
            if !safe_transaction_id(&transaction_id)
                || nonce.len() != 64
                || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Studio.to_string(),
                    detail: "invalid Studio activation proof identity".into(),
                });
            }
            let config_path = loaded_config.canonical_path.as_ref().ok_or_else(|| {
                StudioError::UpdateVerificationFailed {
                    component: ComponentId::Studio.to_string(),
                    detail: "activation proof requires an explicit loaded Studio config".into(),
                }
            })?;
            let config_sha256 = loaded_config.sha256.as_deref().ok_or_else(|| {
                StudioError::UpdateVerificationFailed {
                    component: ComponentId::Studio.to_string(),
                    detail: "activation proof requires loaded Studio config bytes".into(),
                }
            })?;
            let state_root = runtime_root.join("studio/data/self-update");
            let root_meta = fs::symlink_metadata(&state_root)?;
            if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Studio.to_string(),
                    detail: "Studio activation proof state root is unsafe".into(),
                });
            }
            let destination = state_root.join(format!("{transaction_id}.ready.json"));
            if destination.exists() || destination.is_symlink() {
                let metadata = fs::symlink_metadata(&destination)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(StudioError::UpdateVerificationFailed {
                        component: ComponentId::Studio.to_string(),
                        detail: "Studio activation proof destination is unsafe".into(),
                    });
                }
            }
            let proof = StudioActivationReadyProof {
                schema_version: 1,
                transaction_id: &transaction_id,
                nonce: &nonce,
                pid: std::process::id(),
                config_path: config_path
                    .to_str()
                    .ok_or_else(|| StudioError::UpdateVerificationFailed {
                        component: ComponentId::Studio.to_string(),
                        detail: "Studio activation config path is not UTF-8".into(),
                    })?
                    .to_owned(),
                config_sha256,
            };
            let mut bytes = serde_json::to_vec_pretty(&proof)?;
            bytes.push(b'\n');
            let temp = state_root.join(format!(
                ".{transaction_id}.ready.{}.tmp",
                std::process::id()
            ));
            if temp.exists() || temp.is_symlink() {
                let metadata = fs::symlink_metadata(&temp)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(StudioError::UpdateVerificationFailed {
                        component: ComponentId::Studio.to_string(),
                        detail: "Studio activation proof temporary path is unsafe".into(),
                    });
                }
                fs::remove_file(&temp)?;
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            let write_result = (|| -> StudioResult<()> {
                file.write_all(&bytes)?;
                file.flush()?;
                file.sync_all()?;
                fs::rename(&temp, &destination)?;
                File::open(&state_root)?.sync_all()?;
                Ok(())
            })();
            if write_result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            write_result?;
            Ok(true)
        }
    }
}

fn safe_transaction_id(value: &str) -> bool {
    value.starts_with(TRANSACTION_PREFIX)
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn new_transaction_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{TRANSACTION_PREFIX}{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn ensure_regular_dir(path: &Path) -> StudioResult<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(format!(
            "required Studio self-update directory is unsafe: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    if path.ends_with("self-update") {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn active_fleet_host_id(runtime_root: &Path) -> StudioResult<String> {
    #[derive(Deserialize)]
    struct HostIdentity {
        host_id: String,
        runtime_root: PathBuf,
    }

    let hosts = runtime_root.join("fleet/hosts");
    let metadata = fs::symlink_metadata(&hosts)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Fleet hosts directory is unavailable".into(),
        ));
    }
    let mut active = Vec::new();
    for entry in fs::read_dir(&hosts)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Fleet hosts directory contains an unsafe entry".into(),
            ));
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.ends_with(".toml") && !name.ends_with(".example.toml") {
            active.push((name, path));
        }
    }
    if active.len() != 1 {
        return Err(StudioError::UpdateTransaction(
            "Fleet runtime must contain exactly one active host profile".into(),
        ));
    }
    let (filename, path) = active.pop().expect("one active Fleet host");
    let host: HostIdentity = toml::from_str(&fs::read_to_string(path)?)?;
    if filename != format!("{}.toml", host.host_id) {
        return Err(StudioError::UpdateTransaction(
            "Fleet host profile filename does not match host_id".into(),
        ));
    }
    if fs::canonicalize(host.runtime_root)? != fs::canonicalize(runtime_root)? {
        return Err(StudioError::UpdateTransaction(
            "Fleet host runtime_root does not match trusted runtime root".into(),
        ));
    }
    Ok(host.host_id)
}

async fn validate_release_candidate(root: &Path, expected: &Version) -> StudioResult<()> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio release candidate root is unsafe".into(),
        });
    }
    let binary = root.join("mcp-studio");
    let binary_meta = fs::symlink_metadata(&binary)?;
    if binary_meta.file_type().is_symlink() || !binary_meta.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio release candidate binary is unavailable".into(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if binary_meta.permissions().mode() & 0o111 == 0 {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release candidate binary is not executable".into(),
            });
        }
    }
    let index = root.join("web/dist/index.html");
    let index_meta = fs::symlink_metadata(&index)?;
    if index_meta.file_type().is_symlink() || !index_meta.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio release candidate dashboard is unavailable".into(),
        });
    }
    // Validate the complete tree before executing downloaded candidate code.
    let _ = release_tree_fingerprint(root)?;
    let mut command = Command::new(&binary);
    command
        .arg("--version")
        .current_dir(root)
        .env_clear()
        .kill_on_drop(true);
    let output = timeout(VERSION_VERIFY_TIMEOUT, command.output())
        .await
        .map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio candidate --version timed out".into(),
        })??;
    if !output.status.success() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio candidate --version failed".into(),
        });
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio candidate --version output is not UTF-8".into(),
        })?;
    let expected_text = expected.to_string();
    if stdout.split_whitespace().last() != Some(expected_text.as_str()) {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio candidate version does not match target".into(),
        });
    }
    Ok(())
}

fn release_tree_fingerprint(root: &Path) -> StudioResult<String> {
    let canonical = fs::canonicalize(root)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "Studio release fingerprint root is unsafe".into(),
        });
    }
    let mut files = Vec::<(String, String)>::new();
    let mut total = 0_u64;
    collect_release_files(&canonical, &canonical, &mut files, &mut total)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, file_digest) in files {
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(file_digest.as_bytes());
        digest.update(b"\n");
    }
    Ok(hex_digest(digest.finalize()))
}

fn collect_release_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, String)>,
    total: &mut u64,
) -> StudioResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release contains a symlink".into(),
            });
        }
        if metadata.is_dir() {
            collect_release_files(root, &path, files, total)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release contains an unsupported entry".into(),
            });
        }
        if files.len() >= MAX_RELEASE_FILES {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release contains too many files".into(),
            });
        }
        *total = total.saturating_add(metadata.len());
        if *total > MAX_RELEASE_BYTES {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release exceeds size limit".into(),
            });
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release path escaped root".into(),
            })?
            .to_str()
            .ok_or_else(|| StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "Studio release path is not UTF-8".into(),
            })?
            .replace('\\', "/");
        let mut input = File::open(&path)?;
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        files.push((relative, hex_digest(hash.finalize())));
    }
    Ok(())
}

fn copy_release_tree(source: &Path, destination: &Path) -> StudioResult<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Studio.to_string(),
            detail: "staged Studio package root is unsafe".into(),
        });
    }
    fs::create_dir(destination)?;
    copy_release_dir(source, destination)?;
    File::open(destination)?.sync_all()?;
    Ok(())
}

fn copy_release_dir(source: &Path, destination: &Path) -> StudioResult<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src = entry.path();
        let dst = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&src)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "staged Studio package contains a symlink".into(),
            });
        }
        if metadata.is_dir() {
            fs::create_dir(&dst)?;
            copy_release_dir(&src, &dst)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Studio.to_string(),
                detail: "staged Studio package contains an unsupported entry".into(),
            });
        }
        let mut input = File::open(&src)?;
        let mut output = OpenOptions::new().write(true).create_new(true).open(&dst)?;
        std::io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        fs::set_permissions(&dst, metadata.permissions())?;
    }
    Ok(())
}

fn remove_regular_tree(path: &Path) -> StudioResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "Studio self-update candidate is unsafe".into(),
        ));
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::update::{
        Architecture, AvailableRelease, HostPlatform, HostRuntimeRoots, OperatingSystem, Platform,
        ReleaseAsset,
    };

    const FLEET_LAUNCHER_FIXTURE: &str = include_str!("../../tests/fixtures/fleetctl.py");
    const FLEET_LAUNCHER_FIXTURE_SHA256: &str =
        "9d35f9718c3b70e26478845ef35e04fed232dc18e1ec929b5f4768c18644b273";

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
    struct FakeLauncher {
        calls: Mutex<Vec<(String, String, u32)>>,
        fail: Mutex<bool>,
        contract_fail: Mutex<bool>,
    }

    #[async_trait]
    impl LauncherRequester for FakeLauncher {
        async fn validate_contract(&self) -> StudioResult<()> {
            if *self.contract_fail.lock().unwrap() {
                return Err(StudioError::UpdateTransaction(
                    "forced launcher capability failure".into(),
                ));
            }
            Ok(())
        }

        async fn request(
            &self,
            host_id: &str,
            transaction_id: &str,
            parent_pid: u32,
        ) -> StudioResult<()> {
            if *self.fail.lock().unwrap() {
                return Err(StudioError::UpdateTransaction(
                    "forced launcher failure".into(),
                ));
            }
            self.calls.lock().unwrap().push((
                host_id.to_owned(),
                transaction_id.to_owned(),
                parent_pid,
            ));
            Ok(())
        }
    }

    struct FakeProcess {
        pid: u32,
        executable: Mutex<PathBuf>,
        version: Mutex<Version>,
    }

    impl CurrentProcessIdentity for FakeProcess {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn executable(&self) -> StudioResult<PathBuf> {
            Ok(self.executable.lock().unwrap().clone())
        }

        fn version(&self) -> StudioResult<Version> {
            Ok(self.version.lock().unwrap().clone())
        }
    }

    fn version(value: &str) -> Version {
        Version::parse(value).unwrap()
    }

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn write_version_binary(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"mcp-studio {value}\"; exit 0; fi\nexit 0\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn sha256_bytes(bytes: &[u8]) -> String {
        let mut digest = Sha256::new();
        digest.update(bytes);
        hex_digest(digest.finalize())
    }

    fn sha256_path(path: &Path) -> String {
        let mut file = File::open(path).unwrap();
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = file.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        hex_digest(digest.finalize())
    }

    struct Fixture {
        _temp: TempDir,
        catalog: ComponentCatalog,
        manager: SelfUpdateManager,
        launcher: Arc<FakeLauncher>,
        process: Arc<FakeProcess>,
        stager: ArtifactStager,
        inventory: Arc<InventoryService>,
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let bin_root = root.join("bin");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&bin_root).unwrap();
        fs::create_dir_all(&runtime_root).unwrap();

        let studio_root = runtime_root.join("studio");
        fs::create_dir_all(&studio_root).unwrap();
        let legacy = studio_root.join("mcp-studio");
        write_version_binary(&legacy, "1.0.0");
        fs::write(
            studio_root.join("studio.toml"),
            "[server]\nlisten_addr = \"127.0.0.1:18100\"\n",
        )
        .unwrap();

        let fleet_root = runtime_root.join("fleet");
        fs::create_dir_all(fleet_root.join("hosts")).unwrap();
        fs::create_dir_all(fleet_root.join("scripts")).unwrap();
        fs::write(
            fleet_root.join("hosts/aira.toml"),
            format!(
                "host_id = \"aira\"\nruntime_root = \"{}\"\n",
                runtime_root.display()
            ),
        )
        .unwrap();
        fs::write(fleet_root.join("scripts/fleetctl.py"), "# launcher\n").unwrap();

        let catalog =
            ComponentCatalog::new(HostRuntimeRoots::new(bin_root, runtime_root.clone()).unwrap());
        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.join("missing-source"),
                HostPlatform::from_raw("Darwin", "arm64")
                    .unwrap()
                    .platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let launcher = Arc::new(FakeLauncher::default());
        let process = Arc::new(FakeProcess {
            pid: 4242,
            executable: Mutex::new(legacy),
            version: Mutex::new(version("1.0.0")),
        });
        let manager = SelfUpdateManager::with_dependencies(
            catalog.clone(),
            stager.clone(),
            Arc::new(NoopProvider),
            inventory.clone(),
            launcher.clone(),
            process.clone(),
            EventHub::default(),
        )
        .unwrap();

        Fixture {
            _temp: temp,
            catalog,
            manager,
            launcher,
            process,
            stager,
            inventory,
        }
    }

    fn seed_staged_transaction(fixture: &Fixture, tx: &str) -> SelfUpdateRecord {
        let target = version("1.1.0");
        let ready_id = format!("ready-{tx}");
        let ready = fixture.stager.staging_root().join(&ready_id);
        let package_root = ready
            .join("extracted")
            .join("mcp-studio-v1.1.0-darwin-arm64");
        fs::create_dir_all(package_root.join("web/dist")).unwrap();
        let executable = package_root.join("mcp-studio");
        write_version_binary(&executable, "1.1.0");
        fs::write(package_root.join("web/dist/index.html"), "web-1.1.0").unwrap();

        let archive_dir = ready.join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        let asset_name = "mcp-studio-v1.1.0-darwin-arm64.tar.gz".to_owned();
        let archive_bytes = b"verified-studio-archive";
        fs::write(archive_dir.join(&asset_name), archive_bytes).unwrap();
        let archive_sha = sha256_bytes(archive_bytes);
        fs::write(
            archive_dir.join("SHA256SUMS.txt"),
            format!("{archive_sha}  {asset_name}\n"),
        )
        .unwrap();

        let staged = StagedArtifact {
            component: ComponentId::Studio,
            version: target.clone(),
            provider: ReleaseProviderId::ThirteenthXGitHub,
            release_tag: "v1.1.0".into(),
            platform: platform(),
            asset_name,
            archive_sha256: archive_sha,
            companion_asset_name: None,
            companion_archive_sha256: None,
            staging_path: ready.clone(),
            package_root: package_root.clone(),
            validated_executables: vec![executable.clone()],
            validated_executable_sha256: vec![sha256_path(&executable)],
            verified_at_unix_seconds: 1,
        };
        fs::write(
            ready.join("staged.json"),
            serde_json::to_vec_pretty(&staged).unwrap(),
        )
        .unwrap();

        let candidate = fixture
            .catalog
            .runtime_root()
            .join("studio/releases")
            .join(format!("{CANDIDATE_PREFIX}{tx}"));
        copy_release_tree(&package_root, &candidate).unwrap();
        let candidate_fingerprint = release_tree_fingerprint(&candidate).unwrap();

        let record = SelfUpdateRecord {
            schema_version: SELF_UPDATE_SCHEMA_VERSION,
            transaction_id: tx.to_owned(),
            component: "studio".into(),
            source_version: version("1.0.0"),
            target_version: target,
            phase: SelfUpdatePhase::Staged,
            candidate_dir: format!("{CANDIDATE_PREFIX}{tx}"),
            target_release: "v1.1.0".into(),
            candidate_fingerprint: Some(candidate_fingerprint),
            staged_id: Some(ready_id),
            staged_fingerprint: Some(StagedFingerprint::from(&staged)),
            parent_pid: 0,
            previous_layout: None,
            previous_release: None,
            rollback_succeeded: None,
            error: None,
            journal_revision: 0,
            launcher_protocol: Some(FLEET_ACTIVATION_PROTOCOL),
            launcher_owner: None,
            launched_pid: None,
            launched_release_fingerprint: None,
            updated_at_ms: now_ms(),
        };
        fixture.manager.write_record(&record).unwrap();
        record
    }

    fn free_loopback_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn test_python3_executable() -> PathBuf {
        let output = std::process::Command::new("python3")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .expect("python3 is required for external launcher smoke tests");
        assert!(
            output.status.success(),
            "python3 interpreter probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let executable = String::from_utf8(output.stdout)
            .expect("python3 interpreter path must be UTF-8")
            .trim()
            .to_owned();
        let path = PathBuf::from(executable);
        assert!(
            path.is_absolute(),
            "python3 interpreter path must be absolute"
        );
        path
    }

    fn fake_studio_server(path: &Path, value: &str, serve: bool) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let run_body = if serve {
            r#"
config_index = sys.argv.index("--config") + 1
config_path = Path(sys.argv[config_index])
with config_path.open("rb") as handle:
    config = tomllib.load(handle)
host, port = config["server"]["listen_addr"].rsplit(":", 1)
data_dir = config_path.parent / "data"
data_dir.mkdir(parents=True, exist_ok=True)
(data_dir / "fake.pid").write_text(str(os.getpid()))
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/health":
            self.send_response(404); self.end_headers(); return
        body = json.dumps({"status":"ok","service":"mcp-studio","version":VERSION}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, format, *args):
        pass
server = HTTPServer((host, int(port)), Handler)
tx = os.environ.get("MCP_STUDIO_ACTIVATION_TRANSACTION")
nonce = os.environ.get("MCP_STUDIO_ACTIVATION_NONCE")
if tx and nonce:
    proof_dir = config_path.parent / "data" / "self-update"
    proof_dir.mkdir(parents=True, exist_ok=True)
    import hashlib
    proof = {
        "schema_version": 1,
        "transaction_id": tx,
        "nonce": nonce,
        "pid": os.getpid(),
        "config_path": str(config_path.resolve()),
        "config_sha256": hashlib.sha256(config_path.read_bytes()).hexdigest(),
    }
    (proof_dir / f"{tx}.ready.json").write_text(json.dumps(proof))
server.serve_forever()
"#
        } else {
            "raise SystemExit(1)\n"
        };
        let python = test_python3_executable();
        fs::write(
            path,
            format!(
                r#"#!{}
import json
import os
import sys
import tomllib
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
VERSION = "{value}"
if "--version" in sys.argv:
    print("mcp-studio " + VERSION)
    raise SystemExit(0)
{run_body}"#,
                python.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn external_launcher_fixture(
        root: &Path,
        tx: &str,
        target_serve: bool,
        health_timeout_seconds: Option<&str>,
    ) -> SelfUpdateRecord {
        let runtime_root = root.join("runtime");
        let studio_root = runtime_root.join("studio");
        let fleet_root = runtime_root.join("fleet");
        fs::create_dir_all(studio_root.join("data/self-update")).unwrap();
        fs::create_dir_all(studio_root.join("releases")).unwrap();
        fs::create_dir_all(fleet_root.join("hosts")).unwrap();
        fs::create_dir_all(fleet_root.join("scripts")).unwrap();

        let port = free_loopback_port();
        fs::write(
            studio_root.join("studio.toml"),
            format!("[server]\nlisten_addr = \"127.0.0.1:{port}\"\n# preserve-me\n"),
        )
        .unwrap();
        fs::write(studio_root.join("data/preserve.txt"), "preserved").unwrap();
        fake_studio_server(&studio_root.join("mcp-studio"), "1.0.0", true);

        fs::write(
            fleet_root.join("hosts/smoke.toml"),
            format!(
                "host_id = \"smoke\"\nruntime_root = \"{}\"\n",
                runtime_root.display()
            ),
        )
        .unwrap();
        let fixture_digest = format!("{:x}", Sha256::digest(FLEET_LAUNCHER_FIXTURE.as_bytes()));
        assert_eq!(
            fixture_digest, FLEET_LAUNCHER_FIXTURE_SHA256,
            "pinned Fleet launcher fixture changed without review"
        );
        let mut launcher_text = FLEET_LAUNCHER_FIXTURE.to_owned();
        if let Some(timeout_value) = health_timeout_seconds {
            launcher_text = launcher_text.replace(
                "SELF_UPDATE_HEALTH_TIMEOUT_SECONDS = 20.0",
                &format!("SELF_UPDATE_HEALTH_TIMEOUT_SECONDS = {timeout_value}"),
            );
        }
        fs::write(fleet_root.join("scripts/fleetctl.py"), launcher_text).unwrap();

        let candidate = studio_root
            .join("releases")
            .join(format!("{CANDIDATE_PREFIX}{tx}"));
        fs::create_dir_all(candidate.join("web/dist")).unwrap();
        fake_studio_server(&candidate.join("mcp-studio"), "9.9.9", target_serve);
        fs::write(candidate.join("web/dist/index.html"), "target-web").unwrap();
        let fingerprint = release_tree_fingerprint(&candidate).unwrap();

        let record = SelfUpdateRecord {
            schema_version: SELF_UPDATE_SCHEMA_VERSION,
            transaction_id: tx.to_owned(),
            component: "studio".into(),
            source_version: version("1.0.0"),
            target_version: version("9.9.9"),
            phase: SelfUpdatePhase::ActivationPending,
            candidate_dir: format!("{CANDIDATE_PREFIX}{tx}"),
            target_release: "v9.9.9".into(),
            candidate_fingerprint: Some(fingerprint),
            staged_id: None,
            staged_fingerprint: None,
            parent_pid: 999_999_999,
            previous_layout: None,
            previous_release: None,
            rollback_succeeded: None,
            error: None,
            journal_revision: 0,
            launcher_protocol: Some(FLEET_ACTIVATION_PROTOCOL),
            launcher_owner: None,
            launched_pid: None,
            launched_release_fingerprint: None,
            updated_at_ms: now_ms(),
        };
        fs::write(
            studio_root
                .join("data/self-update")
                .join(format!("{tx}.json")),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        record
    }

    fn stop_fake_studio(studio_root: &Path) {
        let pid_path = studio_root.join("data/fake.pid");
        if let Ok(pid) = fs::read_to_string(&pid_path) {
            let _ = std::process::Command::new("kill").arg(pid.trim()).status();
        }
    }
    #[test]
    #[ignore = "explicit isolated external launcher success smoke"]
    fn external_launcher_success_smoke_preserves_local_state_and_switches_release() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let tx = "txn-studio-external-success";
        external_launcher_fixture(root, tx, true, None);
        let runtime_root = root.join("runtime");
        let studio_root = runtime_root.join("studio");
        let config_before = fs::read(studio_root.join("studio.toml")).unwrap();
        let state_before = fs::read(studio_root.join("data/preserve.txt")).unwrap();

        let output = std::process::Command::new(test_python3_executable())
            .arg(runtime_root.join("fleet/scripts/fleetctl.py"))
            .arg("studio-activate")
            .arg("--host")
            .arg("smoke")
            .arg("--transaction")
            .arg(tx)
            .arg("--parent-pid")
            .arg("999999999")
            .current_dir(runtime_root.join("fleet"))
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Fleet launcher failed: status={:?} stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let current = fs::read_link(studio_root.join("current")).unwrap();
        assert_eq!(current, PathBuf::from("releases/v9.9.9"));
        assert_eq!(
            fs::read(studio_root.join("studio.toml")).unwrap(),
            config_before
        );
        assert_eq!(
            fs::read(studio_root.join("data/preserve.txt")).unwrap(),
            state_before
        );
        assert_eq!(
            fs::read_to_string(studio_root.join("releases/v9.9.9/web/dist/index.html")).unwrap(),
            "target-web"
        );
        let record: SelfUpdateRecord = serde_json::from_slice(
            &fs::read(
                studio_root
                    .join("data/self-update")
                    .join(format!("{tx}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(record.phase, SelfUpdatePhase::Completed);
        assert_eq!(record.previous_layout.as_deref(), Some("legacy_flat"));
        stop_fake_studio(&studio_root);
    }

    #[test]
    #[ignore = "explicit isolated external launcher rollback smoke"]
    fn external_launcher_health_failure_smoke_rolls_back_legacy_release() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let tx = "txn-studio-external-rollback";
        external_launcher_fixture(root, tx, false, Some("0.8"));
        let runtime_root = root.join("runtime");
        let studio_root = runtime_root.join("studio");
        let config_before = fs::read(studio_root.join("studio.toml")).unwrap();
        let state_before = fs::read(studio_root.join("data/preserve.txt")).unwrap();

        let output = std::process::Command::new(test_python3_executable())
            .arg(runtime_root.join("fleet/scripts/fleetctl.py"))
            .arg("studio-activate")
            .arg("--host")
            .arg("smoke")
            .arg("--transaction")
            .arg(tx)
            .arg("--parent-pid")
            .arg("999999999")
            .current_dir(runtime_root.join("fleet"))
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "Fleet rollback launcher returned unexpected status: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(!studio_root.join("current").exists());
        assert_eq!(
            fs::read(studio_root.join("studio.toml")).unwrap(),
            config_before
        );
        assert_eq!(
            fs::read(studio_root.join("data/preserve.txt")).unwrap(),
            state_before
        );
        let record: SelfUpdateRecord = serde_json::from_slice(
            &fs::read(
                studio_root
                    .join("data/self-update")
                    .join(format!("{tx}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(record.phase, SelfUpdatePhase::RolledBack);
        assert_eq!(record.rollback_succeeded, Some(true));
        assert_eq!(record.previous_layout.as_deref(), Some("legacy_flat"));
        stop_fake_studio(&studio_root);
    }

    #[test]
    fn transaction_id_validation_is_closed() {
        assert!(safe_transaction_id("txn-studio-1-2-3"));
        assert!(!safe_transaction_id("../txn-studio-x"));
        assert!(!safe_transaction_id("txn-git-1"));
    }

    #[tokio::test]
    async fn apply_persists_activation_pending_and_requests_external_launcher() {
        let fixture = fixture();
        let tx = "txn-studio-apply";
        seed_staged_transaction(&fixture, tx);

        let view = fixture.manager.apply(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::ActivationPending);
        assert_eq!(view.component, ComponentId::Studio);
        assert_eq!(
            fixture.launcher.calls.lock().unwrap().as_slice(),
            &[("aira".into(), tx.into(), 4242)]
        );

        let persisted = fixture.manager.transaction(tx).await.unwrap();
        assert_eq!(persisted.phase, McpUpdatePhase::ActivationPending);

        let recreated = SelfUpdateManager::with_dependencies(
            fixture.catalog.clone(),
            ArtifactStager::new(fixture.catalog.clone()).unwrap(),
            Arc::new(NoopProvider),
            fixture.inventory.clone(),
            fixture.launcher.clone(),
            fixture.process.clone(),
            EventHub::default(),
        )
        .unwrap();
        assert_eq!(
            recreated.transaction(tx).await.unwrap().phase,
            McpUpdatePhase::ActivationPending
        );
    }

    #[tokio::test]
    async fn startup_does_not_finalize_external_activation_without_fleet_terminal_state() {
        let fixture = fixture();
        let tx = "txn-studio-finalize";
        let mut record = seed_staged_transaction(&fixture, tx);
        let candidate = fixture.manager.candidate_path(&record).unwrap();
        let target = fixture
            .catalog
            .runtime_root()
            .join("studio/releases/v1.1.0");
        fs::rename(candidate, &target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "releases/v1.1.0",
            fixture.catalog.runtime_root().join("studio/current"),
        )
        .unwrap();

        record.phase = SelfUpdatePhase::ExternalActivated;
        record.parent_pid = 4242;
        fixture.manager.write_record(&record).unwrap();
        *fixture.process.executable.lock().unwrap() = target.join("mcp-studio");
        *fixture.process.version.lock().unwrap() = version("1.1.0");

        let desired_before = fixture
            .inventory
            .get(ComponentId::Studio)
            .await
            .unwrap()
            .desired_version;
        let changed = fixture.manager.finalize_startup().await.unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].phase, McpUpdatePhase::HealthVerifying);
        assert_eq!(
            fixture.manager.transaction(tx).await.unwrap().phase,
            McpUpdatePhase::HealthVerifying
        );
        assert_eq!(
            fixture
                .inventory
                .get(ComponentId::Studio)
                .await
                .unwrap()
                .desired_version,
            desired_before
        );
    }

    #[tokio::test]
    async fn apply_rejects_non_deployed_process_and_candidate_tamper_before_launcher() {
        let fixture = fixture();
        let tx = "txn-studio-reject";
        let record = seed_staged_transaction(&fixture, tx);

        let dev = fixture._temp.path().join("target/debug/mcp-studio");
        write_version_binary(&dev, "1.0.0");
        *fixture.process.executable.lock().unwrap() = dev;
        assert!(fixture.manager.apply(tx).await.is_err());
        assert!(fixture.launcher.calls.lock().unwrap().is_empty());

        *fixture.process.executable.lock().unwrap() =
            fixture.catalog.runtime_root().join("studio/mcp-studio");
        fs::write(
            fixture
                .manager
                .candidate_path(&record)
                .unwrap()
                .join("web/dist/index.html"),
            "tampered",
        )
        .unwrap();
        assert!(fixture.manager.apply(tx).await.is_err());
        assert!(fixture.launcher.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn startup_recovers_stale_preparing_transaction_without_blocking_future_update() {
        let fixture = fixture();
        let tx = "txn-studio-stale-prepare";
        let record = SelfUpdateRecord {
            schema_version: SELF_UPDATE_SCHEMA_VERSION,
            transaction_id: tx.into(),
            component: "studio".into(),
            source_version: version("1.0.0"),
            target_version: version("1.1.0"),
            phase: SelfUpdatePhase::Preparing,
            candidate_dir: format!("{CANDIDATE_PREFIX}{tx}"),
            target_release: "v1.1.0".into(),
            candidate_fingerprint: None,
            staged_id: None,
            staged_fingerprint: None,
            parent_pid: 0,
            previous_layout: None,
            previous_release: None,
            rollback_succeeded: None,
            error: None,
            journal_revision: 0,
            launcher_protocol: Some(FLEET_ACTIVATION_PROTOCOL),
            launcher_owner: None,
            launched_pid: None,
            launched_release_fingerprint: None,
            updated_at_ms: now_ms(),
        };
        let candidate = fixture.manager.candidate_path(&record).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        fs::write(candidate.join("partial"), "partial").unwrap();
        fixture.manager.write_record(&record).unwrap();

        let changed = fixture.manager.finalize_startup().await.unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].phase, McpUpdatePhase::Failed);
        assert_eq!(changed[0].error.as_deref(), Some("interrupted_prepare"));
        assert!(!candidate.exists());
        fixture.manager.ensure_no_active_transaction().unwrap();
    }

    #[tokio::test]
    async fn repeated_apply_recovers_activation_pending_after_candidate_promotion() {
        let fixture = fixture();
        let tx = "txn-studio-resume";
        let record = seed_staged_transaction(&fixture, tx);
        fixture.manager.apply(tx).await.unwrap();

        let candidate = fixture.manager.candidate_path(&record).unwrap();
        let target = fixture
            .catalog
            .runtime_root()
            .join("studio/releases/v1.1.0");
        fs::rename(candidate, &target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "releases/v1.1.0",
            fixture.catalog.runtime_root().join("studio/current"),
        )
        .unwrap();

        let resumed = fixture.manager.apply(tx).await.unwrap();
        assert_eq!(resumed.phase, McpUpdatePhase::ActivationPending);
        assert_eq!(fixture.launcher.calls.lock().unwrap().len(), 2);
        assert_eq!(
            fixture.manager.transaction(tx).await.unwrap().phase,
            McpUpdatePhase::ActivationPending
        );
    }

    #[tokio::test]
    async fn incompatible_fleet_contract_fails_before_activation_pending() {
        let fixture = fixture();
        let tx = "txn-studio-contract-fail";
        seed_staged_transaction(&fixture, tx);
        *fixture.launcher.contract_fail.lock().unwrap() = true;

        assert!(fixture.manager.apply(tx).await.is_err());
        let persisted = fixture.manager.transaction(tx).await.unwrap();
        assert_eq!(persisted.phase, McpUpdatePhase::Staged);
        assert!(fixture.launcher.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn launcher_request_failure_is_durable_and_does_not_shutdown_by_itself() {
        let fixture = fixture();
        let tx = "txn-studio-launcher-fail";
        seed_staged_transaction(&fixture, tx);
        *fixture.launcher.fail.lock().unwrap() = true;

        assert!(fixture.manager.apply(tx).await.is_err());
        let persisted = fixture.manager.transaction(tx).await.unwrap();
        assert_eq!(persisted.phase, McpUpdatePhase::Failed);
        assert_eq!(persisted.error.as_deref(), Some("launcher_unavailable"));
    }

    #[test]
    fn release_fingerprint_rejects_symlink_and_is_deterministic() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("release");
        fs::create_dir_all(root.join("web/dist")).unwrap();
        fs::write(root.join("mcp-studio"), "bin").unwrap();
        fs::write(root.join("web/dist/index.html"), "web").unwrap();
        assert_eq!(
            release_tree_fingerprint(&root).unwrap(),
            release_tree_fingerprint(&root).unwrap()
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("index.html", root.join("web/dist/link")).unwrap();
            assert!(release_tree_fingerprint(&root).is_err());
        }
    }

    #[tokio::test]
    async fn audit_self_update_requires_health_before_completed() {
        let fixture = fixture();
        let tx = "txn-studio-audit-health";
        let mut record = seed_staged_transaction(&fixture, tx);
        let candidate = fixture.manager.candidate_path(&record).unwrap();
        let target = fixture
            .catalog
            .runtime_root()
            .join("studio/releases/v1.1.0");
        fs::rename(candidate, &target).unwrap();
        std::os::unix::fs::symlink(
            "releases/v1.1.0",
            fixture.catalog.runtime_root().join("studio/current"),
        )
        .unwrap();
        record.phase = SelfUpdatePhase::ExternalActivating;
        fixture.manager.write_record(&record).unwrap();
        *fixture.process.executable.lock().unwrap() = target.join("mcp-studio");
        *fixture.process.version.lock().unwrap() = version("1.1.0");
        fixture.manager.finalize_startup().await.unwrap();
        assert_ne!(
            fixture.manager.transaction(tx).await.unwrap().phase,
            McpUpdatePhase::Completed,
            "no HTTP listener or external health acknowledgement exists, but startup published Completed"
        );
    }
}
