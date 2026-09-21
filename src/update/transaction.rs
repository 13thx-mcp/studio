use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    process::Command,
    sync::Mutex,
    time::{sleep, timeout},
};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
    supervisor::{ProcessState, Supervisor},
};

use super::{
    ArtifactStager, ComponentCatalog, ComponentClass, ComponentId, InventoryService,
    ReleaseProvider, ReleaseProviderId, StagedArtifact, ThirteenthXReleaseProvider, Version,
};

const VERSION_VERIFY_TIMEOUT: Duration = Duration::from_secs(5);
const RUNNING_HEALTH_WINDOW: Duration = Duration::from_millis(400);
const ACTIVATION_TEMP_PREFIX: &str = ".mcp-studio-activate-";
const ROLLBACK_PREFIX: &str = ".mcp-studio-rollback-";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareMcpUpdateRequest {
    pub version: Version,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyMcpUpdateRequest {
    pub transaction_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpUpdatePhase {
    Preparing,
    Staged,
    Stopping,
    Activating,
    Starting,
    Verifying,
    GatewayStopping,
    GatewayRestarting,
    GatewayReconnecting,
    GatewayCatalogVerifying,
    ActivationPending,
    ExternalActivating,
    HealthVerifying,
    RollingBack,
    RolledBack,
    Completed,
    Failed,
    RollbackFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpUpdateTransactionView {
    pub transaction_id: String,
    pub component: ComponentId,
    pub source_version: Option<Version>,
    pub target_version: Version,
    pub phase: McpUpdatePhase,
    pub was_running: Option<bool>,
    pub rollback_succeeded: Option<bool>,
    pub error: Option<String>,
    pub updated_at_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedStagedIdentity {
    version: Version,
    provider: ReleaseProviderId,
    platform: super::Platform,
    asset_name: String,
    archive_sha256: String,
    executable_sha256: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactHistoryIdentity {
    pub component: ComponentId,
    pub version: String,
    pub provider_code: &'static str,
    pub platform_code: String,
    pub archive_sha256: String,
    pub member_sha256: Vec<String>,
}

impl PreparedStagedIdentity {
    pub(crate) fn history_identity(&self, component: ComponentId) -> ArtifactHistoryIdentity {
        ArtifactHistoryIdentity {
            component,
            version: self.version.to_string(),
            provider_code: match self.provider {
                ReleaseProviderId::ThirteenthXGitHub => "github_13thx",
                ReleaseProviderId::OpenAiGitHub => "github_openai",
            },
            platform_code: self.platform.to_string(),
            archive_sha256: self.archive_sha256.clone(),
            member_sha256: self.executable_sha256.clone(),
        }
    }
}

impl From<&StagedArtifact> for PreparedStagedIdentity {
    fn from(staged: &StagedArtifact) -> Self {
        Self {
            version: staged.version.clone(),
            provider: staged.provider,
            platform: staged.platform,
            asset_name: staged.asset_name.clone(),
            archive_sha256: staged.archive_sha256.clone(),
            executable_sha256: staged.validated_executable_sha256.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct TransactionRecord {
    view: McpUpdateTransactionView,
    staged_id: Option<String>,
    staged_identity: Option<PreparedStagedIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Stopped,
    Running,
    Transitional,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LifecycleSnapshot {
    state: LifecycleState,
    enabled: bool,
}

#[async_trait]
trait McpLifecycle: Send + Sync {
    async fn snapshot(&self, component: ComponentId) -> StudioResult<LifecycleSnapshot>;
    async fn stop(&self, component: ComponentId) -> StudioResult<()>;
    async fn start(&self, component: ComponentId) -> StudioResult<()>;
    fn verify_registry_target(&self, component: ComponentId, expected: &Path) -> StudioResult<()>;
}

struct SupervisorMcpLifecycle {
    supervisor: Arc<Supervisor>,
}

impl SupervisorMcpLifecycle {
    fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

#[async_trait]
impl McpLifecycle for SupervisorMcpLifecycle {
    async fn snapshot(&self, component: ComponentId) -> StudioResult<LifecycleSnapshot> {
        let id = component.as_str();
        let registered = self.supervisor.registry().get(id).ok_or_else(|| {
            StudioError::UpdateTransaction(format!(
                "{component}: MCP is not registered with Studio"
            ))
        })?;
        let status = self.supervisor.status(id).await?;
        let state = match status.state {
            ProcessState::Stopped => LifecycleState::Stopped,
            ProcessState::Running => LifecycleState::Running,
            ProcessState::Starting | ProcessState::Stopping => LifecycleState::Transitional,
            ProcessState::Failed => LifecycleState::Failed,
        };
        Ok(LifecycleSnapshot {
            state,
            enabled: registered.enabled,
        })
    }

    async fn stop(&self, component: ComponentId) -> StudioResult<()> {
        self.supervisor.stop(component.as_str()).await?;
        Ok(())
    }

    async fn start(&self, component: ComponentId) -> StudioResult<()> {
        self.supervisor.start(component.as_str()).await?;
        Ok(())
    }

    fn verify_registry_target(&self, component: ComponentId, expected: &Path) -> StudioResult<()> {
        let id = component.as_str();
        let registry = self.supervisor.registry();
        let registered = registry.get(id).ok_or_else(|| {
            StudioError::UpdateTransaction(format!(
                "{component}: MCP is not registered with Studio"
            ))
        })?;
        let root = registry.canonical_root();
        let project = root
            .join(&registered.project_path)
            .canonicalize()
            .map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "{component}: registered project path does not resolve: {error}"
                ))
            })?;
        if !project.starts_with(root) {
            return Err(StudioError::UpdateTransaction(format!(
                "{component}: registered project path escapes MCP root"
            )));
        }
        let executable = project
            .join(&registered.executable)
            .canonicalize()
            .map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "{component}: registered executable does not resolve: {error}"
                ))
            })?;
        if !executable.starts_with(&project) {
            return Err(StudioError::UpdateTransaction(format!(
                "{component}: registered executable escapes project root"
            )));
        }
        let expected = fs::canonicalize(expected).map_err(|error| {
            StudioError::UpdateTransaction(format!(
                "{component}: target executable does not resolve: {error}"
            ))
        })?;
        if executable != expected {
            return Err(StudioError::UpdateTransaction(format!(
                "{component}: registry executable does not match catalog target"
            )));
        }
        Ok(())
    }
}

#[async_trait]
trait ActivatedBinaryVerifier: Send + Sync {
    async fn verify(&self, path: &Path, expected: &Version) -> StudioResult<()>;
}

struct ProcessActivatedBinaryVerifier;

#[async_trait]
impl ActivatedBinaryVerifier for ProcessActivatedBinaryVerifier {
    async fn verify(&self, path: &Path, expected: &Version) -> StudioResult<()> {
        let mut command = Command::new(path);
        command.arg("--version").env_clear().kill_on_drop(true);
        let output = timeout(VERSION_VERIFY_TIMEOUT, command.output())
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "mcp".into(),
                detail: "activated binary --version timed out".into(),
            })?
            .map_err(|error| StudioError::UpdateVerificationFailed {
                component: "mcp".into(),
                detail: format!("activated binary version probe failed: {error}"),
            })?;
        if !output.status.success() {
            return Err(StudioError::UpdateVerificationFailed {
                component: "mcp".into(),
                detail: format!("activated binary --version exited with {}", output.status),
            });
        }
        let stdout = String::from_utf8(output.stdout).map_err(|_| {
            StudioError::UpdateVerificationFailed {
                component: "mcp".into(),
                detail: "activated binary --version output is not UTF-8".into(),
            }
        })?;
        let token = stdout.split_whitespace().last().ok_or_else(|| {
            StudioError::UpdateVerificationFailed {
                component: "mcp".into(),
                detail: "activated binary --version output is empty".into(),
            }
        })?;
        let actual = Version::parse(token).map_err(|_| StudioError::UpdateVerificationFailed {
            component: "mcp".into(),
            detail: "activated binary reported an invalid semantic version".into(),
        })?;
        if &actual != expected {
            return Err(StudioError::UpdateVerificationFailed {
                component: "mcp".into(),
                detail: format!("activated binary reports {actual}, expected {expected}"),
            });
        }
        Ok(())
    }
}

struct RollbackContext<'a> {
    transaction_id: &'a str,
    component: ComponentId,
    source_version: Version,
    target_version: Version,
    was_running: bool,
    was_enabled: bool,
    target: &'a Path,
    rollback: &'a RollbackMaterial,
    reason: String,
    new_runtime_started: bool,
}

#[derive(Clone)]
pub struct McpUpdateManager {
    catalog: ComponentCatalog,
    stager: ArtifactStager,
    provider: Arc<dyn ReleaseProvider>,
    lifecycle: Arc<dyn McpLifecycle>,
    verifier: Arc<dyn ActivatedBinaryVerifier>,
    inventory: Arc<InventoryService>,
    events: EventHub,
    active_components: Arc<StdMutex<BTreeSet<ComponentId>>>,
    transactions: Arc<Mutex<BTreeMap<String, TransactionRecord>>>,
}

impl McpUpdateManager {
    pub fn new(
        catalog: ComponentCatalog,
        supervisor: Arc<Supervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> StudioResult<Self> {
        recover_interrupted_update_files(&catalog)?;
        let stager = ArtifactStager::new(catalog.clone())?;
        let provider = Arc::new(ThirteenthXReleaseProvider::new(catalog.clone())?);
        Ok(Self {
            catalog,
            stager,
            provider,
            lifecycle: Arc::new(SupervisorMcpLifecycle::new(supervisor)),
            verifier: Arc::new(ProcessActivatedBinaryVerifier),
            inventory,
            events,
            active_components: Arc::new(StdMutex::new(BTreeSet::new())),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        stager: ArtifactStager,
        provider: Arc<dyn ReleaseProvider>,
        lifecycle: Arc<dyn McpLifecycle>,
        verifier: Arc<dyn ActivatedBinaryVerifier>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> Self {
        Self {
            catalog,
            stager,
            provider,
            lifecycle,
            verifier,
            inventory,
            events,
            active_components: Arc::new(StdMutex::new(BTreeSet::new())),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub async fn prepare(
        &self,
        component: ComponentId,
        version: Version,
    ) -> StudioResult<McpUpdateTransactionView> {
        self.validate_target(component)?;
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_component(component, "mcp_update")?;
        let _guard = self.acquire(component).await?;
        let inventory = self.inventory.get(component).await?;
        let source_version = inventory.installed_version.ok_or_else(|| {
            StudioError::UpdateTransaction(format!(
                "{component}: installed version is unavailable; update requires rollback baseline"
            ))
        })?;
        if !version.is_newer_than(&source_version) {
            return Err(StudioError::Conflict(format!(
                "{component}: target {version} must be newer than installed {source_version}"
            )));
        }
        let transaction_id = new_transaction_id(component);
        let preparing = McpUpdateTransactionView {
            transaction_id: transaction_id.clone(),
            component,
            source_version: Some(source_version),
            target_version: version.clone(),
            phase: McpUpdatePhase::Preparing,
            was_running: None,
            rollback_succeeded: None,
            error: None,
            updated_at_ms: now_ms(),
        };
        self.transactions.lock().await.insert(
            transaction_id.clone(),
            TransactionRecord {
                view: preparing.clone(),
                staged_id: None,
                staged_identity: None,
            },
        );
        self.emit(&preparing);
        let policy = self.catalog.component(component)?.clone();
        let result = async {
            let release = self.provider.release(&policy, &version).await?;
            let staged = self
                .stager
                .stage(self.provider.as_ref(), &release, self.inventory.platform())
                .await?;
            let staged_id = self.stager.staged_id(&staged)?;
            Ok::<_, StudioError>((staged_id, PreparedStagedIdentity::from(&staged)))
        }
        .await;
        match result {
            Ok((staged_id, staged_identity)) => {
                let mut transactions = self.transactions.lock().await;
                let record = transactions
                    .get_mut(&transaction_id)
                    .expect("preparing transaction exists");
                record.staged_id = Some(staged_id);
                record.staged_identity = Some(staged_identity);
                record.view.phase = McpUpdatePhase::Staged;
                record.view.updated_at_ms = now_ms();
                let view = record.view.clone();
                drop(transactions);
                self.emit(&view);
                Ok(view)
            }
            Err(error) => {
                let mut transactions = self.transactions.lock().await;
                let record = transactions
                    .get_mut(&transaction_id)
                    .expect("preparing transaction exists");
                record.view.phase = McpUpdatePhase::Failed;
                record.view.error = Some(sanitize_transaction_error(&error.to_string()));
                record.view.updated_at_ms = now_ms();
                let view = record.view.clone();
                drop(transactions);
                self.emit(&view);
                Err(error)
            }
        }
    }

    pub(crate) async fn artifact_history_identity(
        &self,
        component: ComponentId,
        transaction_id: &str,
    ) -> Option<ArtifactHistoryIdentity> {
        self.transactions
            .lock()
            .await
            .get(transaction_id)
            .and_then(|record| record.staged_identity.as_ref())
            .map(|identity| identity.history_identity(component))
    }

    pub async fn apply(
        &self,
        component: ComponentId,
        transaction_id: &str,
    ) -> StudioResult<McpUpdateTransactionView> {
        self.validate_target(component)?;
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_component(component, "mcp_update")?;
        let _guard = self.acquire(component).await?;
        let (staged_id, source_version, expected_staged) = self
            .ensure_transaction_applicable(transaction_id, component)
            .await?;
        let current = self.inventory.get(component).await?;
        if current.installed_version.as_ref() != Some(&source_version) {
            return Err(StudioError::Conflict(format!(
                "{component}: installed version changed after update preparation"
            )));
        }
        let staged = self.stager.load_ready(&staged_id)?;
        if staged.component != component {
            return Err(StudioError::UpdateTransaction(format!(
                "transaction component mismatch: expected {component}, got {}",
                staged.component
            )));
        }
        if PreparedStagedIdentity::from(&staged) != expected_staged {
            return Err(StudioError::Conflict(format!(
                "{component}: staged artifact changed after preparation"
            )));
        }
        self.apply_staged(transaction_id, source_version, staged)
            .await
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

    fn validate_target(&self, component: ComponentId) -> StudioResult<()> {
        let policy = self.catalog.component(component)?;
        if policy.class != ComponentClass::McpBinary || component == ComponentId::Gateway {
            return Err(StudioError::Unsupported(format!(
                "component {component} is not a generic M5.7 MCP update target"
            )));
        }
        if policy.provider != ReleaseProviderId::ThirteenthXGitHub {
            return Err(StudioError::Unsupported(format!(
                "component {component} is not managed by the project release provider"
            )));
        }
        Ok(())
    }

    async fn ensure_transaction_applicable(
        &self,
        transaction_id: &str,
        component: ComponentId,
    ) -> StudioResult<(String, Version, PreparedStagedIdentity)> {
        if !safe_transaction_id(transaction_id) {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        let transactions = self.transactions.lock().await;
        let record = transactions
            .get(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        if record.view.component != component {
            return Err(StudioError::UpdateTransaction(
                "transaction component does not match request".into(),
            ));
        }
        if record.view.phase != McpUpdatePhase::Staged {
            return Err(StudioError::Conflict(format!(
                "transaction {transaction_id} is not staged"
            )));
        }
        let staged_id = record.staged_id.clone().ok_or_else(|| {
            StudioError::UpdateTransaction("transaction has no staged artifact".into())
        })?;
        let source_version = record.view.source_version.clone().ok_or_else(|| {
            StudioError::UpdateTransaction("transaction has no source version snapshot".into())
        })?;
        let staged_identity = record.staged_identity.clone().ok_or_else(|| {
            StudioError::UpdateTransaction("transaction has no prepared staging fingerprint".into())
        })?;
        Ok((staged_id, source_version, staged_identity))
    }

    async fn apply_staged(
        &self,
        transaction_id: &str,
        source_version: Version,
        staged: StagedArtifact,
    ) -> StudioResult<McpUpdateTransactionView> {
        let component = staged.component;
        let target_version = staged.version.clone();
        let target = self.catalog.install_path(component)?;
        if !target.exists() {
            return self
                .fail(
                    transaction_id,
                    component,
                    target_version,
                    None,
                    "current target binary is missing; no rollback baseline exists".into(),
                    false,
                )
                .await;
        }
        self.ensure_target_confined(component, &target)?;
        self.lifecycle.verify_registry_target(component, &target)?;
        let snapshot = self.lifecycle.snapshot(component).await?;
        if snapshot.state == LifecycleState::Transitional {
            return Err(StudioError::Conflict(format!(
                "{component} lifecycle transition is already in progress"
            )));
        }
        if snapshot.state == LifecycleState::Failed {
            return Err(StudioError::Conflict(format!(
                "{component} must be in stopped or running state before update"
            )));
        }
        let was_running = snapshot.state == LifecycleState::Running;
        if was_running && !snapshot.enabled {
            return Err(StudioError::Conflict(format!(
                "{component} is running while disabled"
            )));
        }

        let staged_binary = staged.validated_executables.first().ok_or_else(|| {
            StudioError::UpdateTransaction("staged MCP artifact has no validated executable".into())
        })?;
        self.verify_staged_target(component, staged_binary, &staged)?;
        let rollback = prepare_rollback(&target, component, transaction_id)?;

        if was_running {
            self.update_phase(
                transaction_id,
                McpUpdatePhase::Stopping,
                Some(true),
                None,
                None,
            )
            .await?;
            if let Err(error) = self.lifecycle.stop(component).await {
                let _ = remove_rollback_material(&rollback);
                return self
                    .fail(
                        transaction_id,
                        component,
                        target_version,
                        Some(true),
                        format!("stop failed: {error}"),
                        false,
                    )
                    .await;
            }
        }

        self.update_phase(
            transaction_id,
            McpUpdatePhase::Activating,
            Some(was_running),
            None,
            None,
        )
        .await?;
        if let Err(error) = activate_binary(staged_binary, &target, component, transaction_id) {
            return self
                .rollback_after_failure(RollbackContext {
                    transaction_id,
                    component,
                    source_version: source_version.clone(),
                    target_version,
                    was_running,
                    was_enabled: snapshot.enabled,
                    target: &target,
                    rollback: &rollback,
                    reason: format!("activation failed: {error}"),
                    new_runtime_started: false,
                })
                .await;
        }

        if was_running {
            self.update_phase(
                transaction_id,
                McpUpdatePhase::Starting,
                Some(true),
                None,
                None,
            )
            .await?;
            if let Err(error) = self.lifecycle.start(component).await {
                return self
                    .rollback_after_failure(RollbackContext {
                        transaction_id,
                        component,
                        source_version: source_version.clone(),
                        target_version,
                        was_running: true,
                        was_enabled: snapshot.enabled,
                        target: &target,
                        rollback: &rollback,
                        reason: format!("restart failed: {error}"),
                        new_runtime_started: false,
                    })
                    .await;
            }
        }

        self.update_phase(
            transaction_id,
            McpUpdatePhase::Verifying,
            Some(was_running),
            None,
            None,
        )
        .await?;
        let verification = self
            .verify_activated(component, &target, &target_version, was_running)
            .await;
        if let Err(error) = verification {
            return self
                .rollback_after_failure(RollbackContext {
                    transaction_id,
                    component,
                    source_version: source_version.clone(),
                    target_version,
                    was_running,
                    was_enabled: snapshot.enabled,
                    target: &target,
                    rollback: &rollback,
                    reason: format!("verification failed: {error}"),
                    new_runtime_started: was_running,
                })
                .await;
        }

        remove_rollback_material(&rollback)?;
        self.inventory
            .set_desired(component, target_version.clone())
            .await;
        self.update_phase(
            transaction_id,
            McpUpdatePhase::Completed,
            Some(was_running),
            None,
            None,
        )
        .await
    }

    async fn verify_activated(
        &self,
        component: ComponentId,
        target: &Path,
        expected: &Version,
        was_running: bool,
    ) -> StudioResult<()> {
        self.verifier.verify(target, expected).await?;
        if was_running {
            let first = self.lifecycle.snapshot(component).await?;
            if first.state != LifecycleState::Running {
                return Err(StudioError::UpdateVerificationFailed {
                    component: component.to_string(),
                    detail: "restarted MCP is not running".into(),
                });
            }
            sleep(RUNNING_HEALTH_WINDOW).await;
            let second = self.lifecycle.snapshot(component).await?;
            if second.state != LifecycleState::Running {
                return Err(StudioError::UpdateVerificationFailed {
                    component: component.to_string(),
                    detail: "restarted MCP did not remain running during health window".into(),
                });
            }
        }
        Ok(())
    }

    async fn rollback_after_failure(
        &self,
        context: RollbackContext<'_>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let RollbackContext {
            transaction_id,
            component,
            source_version,
            target_version,
            was_running,
            was_enabled,
            target,
            rollback,
            reason,
            new_runtime_started,
        } = context;
        self.update_phase(
            transaction_id,
            McpUpdatePhase::RollingBack,
            Some(was_running),
            None,
            Some(reason.clone()),
        )
        .await?;

        if new_runtime_started {
            let current = self.lifecycle.snapshot(component).await;
            if current.is_ok_and(|snapshot| snapshot.state == LifecycleState::Running)
                && let Err(error) = self.lifecycle.stop(component).await
            {
                return self
                    .rollback_failed(
                        transaction_id,
                        component,
                        target_version,
                        was_running,
                        format!("{reason}; failed to stop new runtime for rollback: {error}"),
                    )
                    .await;
            }
        }

        if let Err(error) = restore_rollback(target, rollback) {
            return self
                .rollback_failed(
                    transaction_id,
                    component,
                    target_version,
                    was_running,
                    format!("{reason}; rollback activation failed: {error}"),
                )
                .await;
        }

        if was_running
            && was_enabled
            && let Err(error) = self.lifecycle.start(component).await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    component,
                    target_version,
                    was_running,
                    format!("{reason}; rollback restart failed: {error}"),
                )
                .await;
        }
        if let Err(error) = self
            .verify_activated(
                component,
                target,
                &source_version,
                was_running && was_enabled,
            )
            .await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    component,
                    target_version,
                    was_running,
                    format!("{reason}; rollback verification failed: {error}"),
                )
                .await;
        }

        let failure = reason.clone();
        self.update_phase(
            transaction_id,
            McpUpdatePhase::Failed,
            Some(was_running),
            Some(true),
            Some(reason),
        )
        .await?;
        Err(StudioError::UpdateTransaction(failure))
    }

    async fn rollback_failed(
        &self,
        transaction_id: &str,
        component: ComponentId,
        target_version: Version,
        was_running: bool,
        detail: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        let view = McpUpdateTransactionView {
            transaction_id: transaction_id.to_owned(),
            component,
            source_version: None,
            target_version,
            phase: McpUpdatePhase::RollbackFailed,
            was_running: Some(was_running),
            rollback_succeeded: Some(false),
            error: Some(sanitize_transaction_error(&detail)),
            updated_at_ms: now_ms(),
        };
        self.set_record(view).await;
        Err(StudioError::RollbackFailed {
            component: component.to_string(),
            detail,
        })
    }

    async fn fail(
        &self,
        transaction_id: &str,
        component: ComponentId,
        target_version: Version,
        was_running: Option<bool>,
        detail: String,
        rollback_succeeded: bool,
    ) -> StudioResult<McpUpdateTransactionView> {
        let view = McpUpdateTransactionView {
            transaction_id: transaction_id.to_owned(),
            component,
            source_version: None,
            target_version,
            phase: McpUpdatePhase::Failed,
            was_running,
            rollback_succeeded: Some(rollback_succeeded),
            error: Some(sanitize_transaction_error(&detail)),
            updated_at_ms: now_ms(),
        };
        self.set_record(view).await;
        Err(StudioError::UpdateTransaction(detail))
    }

    async fn update_phase(
        &self,
        transaction_id: &str,
        phase: McpUpdatePhase,
        was_running: Option<bool>,
        rollback_succeeded: Option<bool>,
        error: Option<String>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let mut transactions = self.transactions.lock().await;
        let record = transactions
            .get_mut(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        record.view.phase = phase;
        if was_running.is_some() {
            record.view.was_running = was_running;
        }
        if rollback_succeeded.is_some() {
            record.view.rollback_succeeded = rollback_succeeded;
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
                TransactionRecord {
                    view: view.clone(),
                    staged_id: None,
                    staged_identity: None,
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

    async fn acquire(&self, component: ComponentId) -> StudioResult<ComponentUpdateGuard> {
        let mut active = self
            .active_components
            .lock()
            .map_err(|_| StudioError::UpdateTransaction("update lock poisoned".into()))?;
        if !active.insert(component) {
            return Err(StudioError::Conflict(format!(
                "update already in progress for {component}"
            )));
        }
        Ok(ComponentUpdateGuard {
            component,
            active: self.active_components.clone(),
        })
    }

    fn ensure_target_confined(&self, component: ComponentId, target: &Path) -> StudioResult<()> {
        let target_metadata = fs::symlink_metadata(target).map_err(|error| {
            StudioError::UpdateTransaction(format!(
                "{component}: target metadata is unavailable: {error}"
            ))
        })?;
        if target_metadata.file_type().is_symlink() || !target_metadata.is_file() {
            return Err(StudioError::UpdateTransaction(format!(
                "{component}: target must be a regular non-symlink file"
            )));
        }
        let target_parent = target.parent().ok_or_else(|| {
            StudioError::UpdateTransaction(format!("{component}: target has no parent directory"))
        })?;
        let canonical_bin = fs::canonicalize(self.catalog.bin_root())?;
        let canonical_parent = fs::canonicalize(target_parent)?;
        if canonical_parent != canonical_bin {
            return Err(StudioError::UpdateTransaction(format!(
                "{component}: target is not directly inside bin_root"
            )));
        }
        Ok(())
    }

    fn verify_staged_target(
        &self,
        component: ComponentId,
        staged_binary: &Path,
        staged: &StagedArtifact,
    ) -> StudioResult<()> {
        let policy = self.catalog.component(component)?;
        let expected_name = match policy.install_target {
            super::InstallTarget::BinRootBinary { binary } => binary,
            _ => {
                return Err(StudioError::UpdateTransaction(
                    "M5.7 target is not a flat binary".into(),
                ));
            }
        };
        if staged_binary.file_name().and_then(|value| value.to_str()) != Some(expected_name) {
            return Err(StudioError::UpdateTransaction(
                "staged executable name does not match catalog target".into(),
            ));
        }
        if staged.provider != policy.provider || staged.component != component {
            return Err(StudioError::UpdateTransaction(
                "staged provider/component identity mismatch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ComponentUpdateGuard {
    component: ComponentId,
    active: Arc<StdMutex<BTreeSet<ComponentId>>>,
}

impl Drop for ComponentUpdateGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.component);
        }
    }
}

pub(crate) struct RollbackMaterial {
    path: PathBuf,
    checksum_path: PathBuf,
    sha256: String,
}

pub(crate) fn prepare_rollback(
    target: &Path,
    component: ComponentId,
    transaction_id: &str,
) -> StudioResult<RollbackMaterial> {
    let rollback = sibling_path(target, ROLLBACK_PREFIX, component, transaction_id)?;
    let partial = append_suffix(&rollback, ".partial");
    let checksum_path = append_suffix(&rollback, ".sha256");
    if partial.exists() || checksum_path.exists() {
        return Err(StudioError::UpdateTransaction(
            "rollback scratch path already exists".into(),
        ));
    }
    if let Err(error) = copy_executable(target, &partial) {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    let sha256 = match sha256_path(&partial) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    };
    if let Err(error) = write_checksum_sidecar(&checksum_path, &sha256) {
        let _ = fs::remove_file(&partial);
        let _ = fs::remove_file(&checksum_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&partial, &rollback) {
        let _ = fs::remove_file(&partial);
        let _ = fs::remove_file(&checksum_path);
        return Err(StudioError::UpdateTransaction(format!(
            "failed to finalize rollback material: {error}"
        )));
    }
    sync_parent_dir(&rollback)?;
    Ok(RollbackMaterial {
        path: rollback,
        checksum_path,
        sha256,
    })
}

pub(crate) fn activate_binary(
    staged_binary: &Path,
    target: &Path,
    component: ComponentId,
    transaction_id: &str,
) -> StudioResult<()> {
    let temp = sibling_path(target, ACTIVATION_TEMP_PREFIX, component, transaction_id)?;
    if let Err(error) = copy_executable(staged_binary, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    fs::rename(&temp, target).map_err(|error| {
        let _ = fs::remove_file(&temp);
        StudioError::UpdateActivationFailed {
            component: component.to_string(),
            detail: error.to_string(),
        }
    })?;
    sync_parent_dir(target)?;
    Ok(())
}

pub(crate) fn restore_rollback(target: &Path, rollback: &RollbackMaterial) -> StudioResult<()> {
    let loaded = load_rollback_material(&rollback.path)?;
    if loaded.sha256 != rollback.sha256 {
        return Err(StudioError::RollbackFailed {
            component: "mcp".into(),
            detail: "rollback checksum sidecar changed".into(),
        });
    }
    fs::rename(&rollback.path, target).map_err(|error| StudioError::RollbackFailed {
        component: "mcp".into(),
        detail: error.to_string(),
    })?;
    let _ = fs::remove_file(&rollback.checksum_path);
    sync_parent_dir(target)?;
    Ok(())
}

fn load_rollback_material(path: &Path) -> StudioResult<RollbackMaterial> {
    let metadata = fs::symlink_metadata(path).map_err(|error| StudioError::RollbackFailed {
        component: "mcp".into(),
        detail: format!("rollback binary is unavailable: {error}"),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StudioError::RollbackFailed {
            component: "mcp".into(),
            detail: "rollback binary is not a regular file".into(),
        });
    }
    let checksum_path = append_suffix(path, ".sha256");
    let checksum_metadata =
        fs::symlink_metadata(&checksum_path).map_err(|error| StudioError::RollbackFailed {
            component: "mcp".into(),
            detail: format!("rollback checksum is unavailable: {error}"),
        })?;
    if checksum_metadata.file_type().is_symlink() || !checksum_metadata.is_file() {
        return Err(StudioError::RollbackFailed {
            component: "mcp".into(),
            detail: "rollback checksum is not a regular file".into(),
        });
    }
    let sha256 = fs::read_to_string(&checksum_path)?
        .trim()
        .to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StudioError::RollbackFailed {
            component: "mcp".into(),
            detail: "rollback checksum sidecar is invalid".into(),
        });
    }
    if sha256_path(path)? != sha256 {
        return Err(StudioError::RollbackFailed {
            component: "mcp".into(),
            detail: "rollback binary checksum changed".into(),
        });
    }
    Ok(RollbackMaterial {
        path: path.to_owned(),
        checksum_path,
        sha256,
    })
}

pub(crate) fn remove_rollback_material(rollback: &RollbackMaterial) -> StudioResult<()> {
    if rollback.path.exists() {
        fs::remove_file(&rollback.path)?;
    }
    if rollback.checksum_path.exists() {
        fs::remove_file(&rollback.checksum_path)?;
    }
    sync_parent_dir(&rollback.path)
}

fn recover_interrupted_update_files(catalog: &ComponentCatalog) -> StudioResult<()> {
    let bin_root = catalog.bin_root();
    if !bin_root.exists() {
        return Ok(());
    }
    if !bin_root.is_dir() {
        return Err(StudioError::UpdateTransaction(
            "configured bin_root is not a directory".into(),
        ));
    }
    for entry in fs::read_dir(bin_root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(ACTIVATION_TEMP_PREFIX)
            && !(name.starts_with(ROLLBACK_PREFIX) && name.ends_with(".partial"))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    sync_parent_dir(&bin_root.join(".recovery-sync"))
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn write_checksum_sidecar(path: &Path, sha256: &str) -> StudioResult<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    writeln!(file, "{sha256}")?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn sync_parent_dir(path: &Path) -> StudioResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| StudioError::UpdateTransaction("path has no parent directory".into()))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn sibling_path(
    target: &Path,
    prefix: &str,
    component: ComponentId,
    transaction_id: &str,
) -> StudioResult<PathBuf> {
    if !safe_transaction_id(transaction_id) {
        return Err(StudioError::UpdateTransaction(
            "invalid update transaction id".into(),
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| StudioError::UpdateTransaction("target has no parent".into()))?;
    Ok(parent.join(format!("{prefix}{component}-{transaction_id}")))
}

fn copy_executable(source: &Path, destination: &Path) -> StudioResult<()> {
    if destination.exists() {
        return Err(StudioError::UpdateTransaction(format!(
            "transaction scratch path already exists: {}",
            destination.display()
        )));
    }
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
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
    let permissions = fs::metadata(source)?.permissions();
    fs::set_permissions(destination, permissions)?;
    Ok(())
}

pub(crate) fn sha256_path(path: &Path) -> StudioResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
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

fn safe_transaction_id(value: &str) -> bool {
    value.starts_with("txn-")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub(crate) fn sanitize_transaction_error(value: &str) -> String {
    if value.contains("rollback") {
        "rollback_failed".into()
    } else if value.contains("verification") || value.contains("version") {
        "verification_failed".into()
    } else if value.contains("activation") {
        "activation_failed".into()
    } else if value.contains("stop") {
        "stop_failed".into()
    } else if value.contains("restart") || value.contains("start") {
        "start_failed".into()
    } else {
        "update_failed".into()
    }
}

fn new_transaction_id(component: ComponentId) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "txn-{component}-{}-{}-{}",
        std::process::id(),
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex as StdMutex};

    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::*;
    use crate::update::{
        Architecture, AvailableRelease, HostRuntimeRoots, OperatingSystem, Platform, ReleaseAsset,
    };

    #[derive(Default)]
    struct FakeLifecycle {
        states: StdMutex<BTreeMap<ComponentId, LifecycleSnapshot>>,
        targets: StdMutex<BTreeMap<ComponentId, PathBuf>>,
        fail_stop: StdMutex<bool>,
        fail_start: StdMutex<bool>,
        stop_count: StdMutex<usize>,
        start_count: StdMutex<usize>,
    }

    #[async_trait]
    impl McpLifecycle for FakeLifecycle {
        async fn snapshot(&self, component: ComponentId) -> StudioResult<LifecycleSnapshot> {
            self.states
                .lock()
                .unwrap()
                .get(&component)
                .copied()
                .ok_or_else(|| StudioError::NotFound(component.to_string()))
        }

        async fn stop(&self, component: ComponentId) -> StudioResult<()> {
            if *self.fail_stop.lock().unwrap() {
                return Err(StudioError::Process("forced stop failure".into()));
            }
            *self.stop_count.lock().unwrap() += 1;
            let mut states = self.states.lock().unwrap();
            let current = states.get(&component).copied().unwrap();
            states.insert(
                component,
                LifecycleSnapshot {
                    state: LifecycleState::Stopped,
                    enabled: current.enabled,
                },
            );
            Ok(())
        }

        async fn start(&self, component: ComponentId) -> StudioResult<()> {
            {
                let mut fail = self.fail_start.lock().unwrap();
                if *fail {
                    *fail = false;
                    return Err(StudioError::Process("forced start failure".into()));
                }
            }
            *self.start_count.lock().unwrap() += 1;
            let mut states = self.states.lock().unwrap();
            let current = states.get(&component).copied().unwrap();
            if !current.enabled {
                return Err(StudioError::Disabled(component.to_string()));
            }
            states.insert(
                component,
                LifecycleSnapshot {
                    state: LifecycleState::Running,
                    enabled: current.enabled,
                },
            );
            Ok(())
        }

        fn verify_registry_target(
            &self,
            component: ComponentId,
            expected: &Path,
        ) -> StudioResult<()> {
            if self
                .targets
                .lock()
                .unwrap()
                .get(&component)
                .is_some_and(|value| value == expected)
            {
                Ok(())
            } else {
                Err(StudioError::UpdateTransaction(
                    "registry target mismatch".into(),
                ))
            }
        }
    }

    struct FakeVerifier {
        fail_version: StdMutex<Option<Version>>,
        remove_before_fail: StdMutex<Option<PathBuf>>,
    }

    impl Default for FakeVerifier {
        fn default() -> Self {
            Self {
                fail_version: StdMutex::new(None),
                remove_before_fail: StdMutex::new(None),
            }
        }
    }

    #[async_trait]
    impl ActivatedBinaryVerifier for FakeVerifier {
        async fn verify(&self, _path: &Path, expected: &Version) -> StudioResult<()> {
            let should_fail = self
                .fail_version
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|version| version == expected);
            if should_fail {
                if let Some(path) = self.remove_before_fail.lock().unwrap().take() {
                    let _ = fs::remove_file(path);
                }
                return Err(StudioError::UpdateVerificationFailed {
                    component: "git".into(),
                    detail: "forced verification failure".into(),
                });
            }
            Ok(())
        }
    }

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

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn write_script(path: &Path, version: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("#!/bin/sh\necho 'rust-mcp-git {version}'\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn digest_file(path: &Path) -> String {
        let mut file = File::open(path).unwrap();
        let mut digest = Sha256::new();
        let mut buf = [0_u8; 8192];
        loop {
            let read = file.read(&mut buf).unwrap();
            if read == 0 {
                break;
            }
            digest.update(&buf[..read]);
        }
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn create_ready(
        stager: &ArtifactStager,
        catalog: &ComponentCatalog,
        component: ComponentId,
        version: &str,
        ready_id: &str,
    ) -> StagedArtifact {
        let policy = catalog.component(component).unwrap();
        let root = stager.staging_root().join(ready_id);
        let package_root = root
            .join("extracted")
            .join(format!("{}-v{}-darwin-arm64", policy.asset.stem, version));
        let binary_name = match policy.install_target {
            super::super::InstallTarget::BinRootBinary { binary } => binary,
            _ => panic!("test component must be flat binary"),
        };
        let staged_binary = package_root.join(binary_name);
        write_script(&staged_binary, version);
        let asset_name = format!("{}-v{}-darwin-arm64.tar.gz", policy.asset.stem, version);
        let archive_dir = root.join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        let archive = archive_dir.join(&asset_name);
        fs::write(&archive, b"verified-test-archive").unwrap();
        let sha = digest_file(&archive);
        fs::write(
            archive_dir.join("SHA256SUMS.txt"),
            format!("{sha}  {asset_name}\n"),
        )
        .unwrap();
        let staged = StagedArtifact {
            component,
            version: Version::parse(version).unwrap(),
            provider: policy.provider,
            release_tag: format!("v{version}"),
            platform: platform(),
            asset_name,
            archive_sha256: sha,
            staging_path: root.clone(),
            package_root,
            validated_executables: vec![staged_binary.clone()],
            validated_executable_sha256: vec![digest_file(&staged_binary)],
            verified_at_unix_seconds: 1,
        };
        fs::write(
            root.join("staged.json"),
            serde_json::to_vec_pretty(&staged).unwrap(),
        )
        .unwrap();
        staged
    }

    fn fixture(
        running: bool,
        enabled: bool,
    ) -> (
        TempDir,
        McpUpdateManager,
        Arc<FakeLifecycle>,
        Arc<FakeVerifier>,
        PathBuf,
    ) {
        let root = TempDir::new().unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.path().join("bin"), root.path().join("runtime")).unwrap(),
        );
        fs::create_dir_all(catalog.bin_root()).unwrap();
        fs::create_dir_all(catalog.runtime_root()).unwrap();
        let target = catalog.install_path(ComponentId::Git).unwrap();
        write_script(&target, "1.0.0");
        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let lifecycle = Arc::new(FakeLifecycle::default());
        lifecycle.states.lock().unwrap().insert(
            ComponentId::Git,
            LifecycleSnapshot {
                state: if running {
                    LifecycleState::Running
                } else {
                    LifecycleState::Stopped
                },
                enabled,
            },
        );
        lifecycle
            .targets
            .lock()
            .unwrap()
            .insert(ComponentId::Git, target.clone());
        let verifier = Arc::new(FakeVerifier::default());
        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.path().join("source"),
                platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let manager = McpUpdateManager::with_dependencies(
            catalog,
            stager,
            Arc::new(NoopProvider),
            lifecycle.clone(),
            verifier.clone(),
            inventory,
            EventHub::default(),
        );
        (root, manager, lifecycle, verifier, target)
    }

    async fn register_ready(
        manager: &McpUpdateManager,
        staged: &StagedArtifact,
        transaction_id: &str,
        ready_id: &str,
    ) {
        let source_version = manager
            .inventory
            .get(staged.component)
            .await
            .unwrap()
            .installed_version;
        manager.transactions.lock().await.insert(
            transaction_id.to_owned(),
            TransactionRecord {
                view: McpUpdateTransactionView {
                    transaction_id: transaction_id.to_owned(),
                    component: staged.component,
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
            },
        );
    }

    #[tokio::test]
    async fn mcp_apply_respects_control_lease() {
        let (_root, manager, _lifecycle, _verifier, _target) = fixture(false, true);
        let coordinator = manager.catalog.runtime_operations();
        let control = coordinator.acquire_control("reconciliation_test").unwrap();

        let error = manager
            .apply(ComponentId::Git, "txn-git-missing")
            .await
            .unwrap_err();
        assert!(matches!(error, StudioError::Conflict(_)));

        drop(control);
        let error = manager
            .apply(ComponentId::Git, "txn-git-missing")
            .await
            .unwrap_err();
        assert!(matches!(error, StudioError::NotFound(_)));
    }

    #[tokio::test]
    async fn mcp_apply_blocks_reconciliation_until_terminal() {
        let (_root, manager, _lifecycle, _verifier, _target) = fixture(false, true);
        let transactions_guard = manager.transactions.lock().await;
        let coordinator = manager.catalog.runtime_operations();
        let running_manager = manager.clone();

        let apply = tokio::spawn(async move {
            running_manager
                .apply(ComponentId::Git, "txn-git-blocked")
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(coordinator.acquire_control("reconciliation_test").is_err());
        drop(transactions_guard);

        let result = apply.await.unwrap();
        assert!(matches!(result, Err(StudioError::NotFound(_))));
        assert!(coordinator.acquire_control("reconciliation_test").is_ok());
    }

    #[tokio::test]
    async fn stopped_mcp_updates_and_remains_stopped_without_touching_other_bins() {
        let (_root, manager, lifecycle, _verifier, target) = fixture(false, true);
        let unrelated = target.parent().unwrap().join("rust-mcp-exec");
        fs::write(&unrelated, b"unchanged").unwrap();
        let ready_id = "ready-stopped";
        let tx = "txn-git-stopped";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;

        let result = manager.apply(ComponentId::Git, tx).await.unwrap();
        assert_eq!(result.phase, McpUpdatePhase::Completed);
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 0);
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 0);
        assert_eq!(fs::read(&unrelated).unwrap(), b"unchanged");
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.1.0"));
    }

    #[tokio::test]
    async fn running_mcp_is_stopped_restarted_and_health_verified() {
        let (_root, manager, lifecycle, _verifier, _target) = fixture(true, true);
        let ready_id = "ready-running";
        let tx = "txn-git-running";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;

        let result = manager.apply(ComponentId::Git, tx).await.unwrap();
        assert_eq!(result.phase, McpUpdatePhase::Completed);
        assert_eq!(result.was_running, Some(true));
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 1);
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn verification_failure_rolls_back_binary_and_running_state() {
        let (_root, manager, lifecycle, verifier, target) = fixture(true, true);
        *verifier.fail_version.lock().unwrap() = Some(Version::parse("1.1.0").unwrap());
        let ready_id = "ready-verifyfail";
        let tx = "txn-git-verifyfail";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;

        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 2);
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn stop_failure_and_activation_failure_leave_recoverable_state() {
        let (_root, manager, lifecycle, _verifier, target) = fixture(true, true);
        *lifecycle.fail_stop.lock().unwrap() = true;
        let ready_id = "ready-stopfail";
        let tx = "txn-git-stopfail";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));

        let (_root, manager, _lifecycle, _verifier, target) = fixture(false, true);
        let ready_id = "ready-activatefail";
        let tx = "txn-git-activatefail";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        let scratch = target
            .parent()
            .unwrap()
            .join(format!("{ACTIVATION_TEMP_PREFIX}git-{tx}"));
        fs::write(&scratch, b"collision").unwrap();
        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn rollback_failure_is_explicit() {
        let (_root, manager, _lifecycle, verifier, target) = fixture(false, true);
        *verifier.fail_version.lock().unwrap() = Some(Version::parse("1.1.0").unwrap());
        let ready_id = "ready-rollbackfail";
        let tx = "txn-git-rollbackfail";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        let rollback = target
            .parent()
            .unwrap()
            .join(format!("{ROLLBACK_PREFIX}git-{tx}"));
        *verifier.remove_before_fail.lock().unwrap() = Some(rollback);

        assert!(matches!(
            manager.apply(ComponentId::Git, tx).await.unwrap_err(),
            StudioError::RollbackFailed { .. }
        ));
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::RollbackFailed);
        assert_eq!(view.rollback_succeeded, Some(false));
    }

    #[tokio::test]
    async fn missing_target_invalid_staging_disabled_and_duplicate_apply_fail_closed() {
        let (_root, manager, _lifecycle, _verifier, target) = fixture(false, true);
        fs::remove_file(&target).unwrap();
        let ready_id = "ready-missing";
        let tx = "txn-git-missing";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(ComponentId::Git, tx).await.is_err());

        let (_root, manager, _lifecycle, _verifier, target) = fixture(false, true);
        let ready_id = "ready-tampered";
        let tx = "txn-git-tampered";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        fs::write(
            staged.staging_path.join("archive").join(&staged.asset_name),
            b"tampered",
        )
        .unwrap();
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));

        let (_root, manager, lifecycle, _verifier, _target) = fixture(false, false);
        let ready_id = "ready-disabled";
        let tx = "txn-git-disabled";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        manager.apply(ComponentId::Git, tx).await.unwrap();
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 0);

        let (_root, manager, _lifecycle, _verifier, _target) = fixture(false, true);
        let ready_id = "ready-once";
        let tx = "txn-git-once";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        manager.apply(ComponentId::Git, tx).await.unwrap();
        assert!(matches!(
            manager.apply(ComponentId::Git, tx).await.unwrap_err(),
            StudioError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn prepare_rejects_equal_or_older_target_before_provider_access() {
        let (_root, manager, _lifecycle, _verifier, _target) = fixture(false, true);
        assert!(matches!(
            manager
                .prepare(ComponentId::Git, Version::parse("1.0.0").unwrap())
                .await
                .unwrap_err(),
            StudioError::Conflict(_)
        ));
        assert!(matches!(
            manager
                .prepare(ComponentId::Git, Version::parse("0.9.0").unwrap())
                .await
                .unwrap_err(),
            StudioError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn stale_prepared_source_and_malformed_staged_metadata_are_rejected_before_stop() {
        let (_root, manager, lifecycle, _verifier, target) = fixture(true, true);
        let ready_id = "ready-stale";
        let tx = "txn-git-stale";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        write_script(&target, "1.0.1");
        assert!(matches!(
            manager.apply(ComponentId::Git, tx).await.unwrap_err(),
            StudioError::Conflict(_)
        ));
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 0);

        let (_root, manager, lifecycle, _verifier, _target) = fixture(true, true);
        let ready_id = "ready-badmeta";
        let tx = "txn-git-badmeta";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        let mut metadata = staged.clone();
        metadata.asset_name = "rust-mcp-git-v9.9.9-darwin-arm64.tar.gz".into();
        fs::write(
            staged.staging_path.join("staged.json"),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn staged_executable_tamper_is_rejected_before_stop() {
        let (_root, manager, lifecycle, _verifier, target) = fixture(true, true);
        let ready_id = "ready-executable-tamper";
        let tx = "txn-git-executable-tamper";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        fs::write(&staged.validated_executables[0], b"tampered executable").unwrap();

        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 0);
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));
    }

    #[test]
    fn startup_recovery_removes_partial_scratch_but_preserves_finalized_rollback() {
        let root = TempDir::new().unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.path().join("bin"), root.path().join("runtime")).unwrap(),
        );
        fs::create_dir_all(catalog.bin_root()).unwrap();
        let activation = catalog
            .bin_root()
            .join(".mcp-studio-activate-git-txn-git-interrupted");
        let rollback_partial = catalog
            .bin_root()
            .join(".mcp-studio-rollback-git-txn-git-interrupted.partial");
        let rollback_final = catalog
            .bin_root()
            .join(".mcp-studio-rollback-git-txn-git-recoverable");
        fs::write(&activation, b"new partial").unwrap();
        fs::write(&rollback_partial, b"old partial").unwrap();
        fs::write(&rollback_final, b"known good").unwrap();

        recover_interrupted_update_files(&catalog).unwrap();
        assert!(!activation.exists());
        assert!(!rollback_partial.exists());
        assert_eq!(fs::read(&rollback_final).unwrap(), b"known good");
    }

    #[tokio::test]
    async fn restart_failure_rolls_back_and_restores_running_state() {
        let (_root, manager, lifecycle, _verifier, target) = fixture(true, true);
        *lifecycle.fail_start.lock().unwrap() = true;
        let ready_id = "ready-restartfail";
        let tx = "txn-git-restartfail";
        let staged = create_ready(
            &manager.stager,
            &manager.catalog,
            ComponentId::Git,
            "1.1.0",
            ready_id,
        );
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(ComponentId::Git, tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 1);
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 1);
        assert_eq!(
            lifecycle
                .states
                .lock()
                .unwrap()
                .get(&ComponentId::Git)
                .unwrap()
                .state,
            LifecycleState::Running
        );
    }

    #[tokio::test]
    async fn per_component_lock_rejects_duplicate_but_allows_unrelated_components() {
        let (_root, manager, _lifecycle, _verifier, _target) = fixture(false, true);
        let _git_guard = manager.acquire(ComponentId::Git).await.unwrap();
        assert!(matches!(
            manager.acquire(ComponentId::Git).await.unwrap_err(),
            StudioError::Conflict(_)
        ));
        let _exec_guard = manager.acquire(ComponentId::Exec).await.unwrap();
    }

    #[test]
    fn gateway_and_non_mcp_targets_are_rejected_and_requests_deny_paths() {
        let (root, manager, _lifecycle, _verifier, _target) = fixture(false, true);
        assert!(manager.validate_target(ComponentId::Gateway).is_err());
        assert!(manager.validate_target(ComponentId::Studio).is_err());
        assert!(
            serde_json::from_str::<PrepareMcpUpdateRequest>(
                r#"{"version":"1.0.0","path":"/tmp/evil"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ApplyMcpUpdateRequest>(
                r#"{"transaction_id":"txn-git-x","download_url":"https://example.invalid"}"#
            )
            .is_err()
        );
        drop(root);
    }

    #[tokio::test]
    #[ignore = "explicit Aira safe real-binary activation smoke"]
    async fn live_safe_real_binary_activation_smoke() {
        let source = PathBuf::from("../bin/rust-mcp-git");
        assert!(source.is_file());
        let root = TempDir::new().unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.path().join("bin"), root.path().join("runtime")).unwrap(),
        );
        fs::create_dir_all(catalog.bin_root()).unwrap();
        fs::create_dir_all(catalog.runtime_root()).unwrap();
        let target = catalog.install_path(ComponentId::Git).unwrap();
        write_script(&target, "0.0.9");
        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let ready_id = "ready-live-real";
        let policy = catalog.component(ComponentId::Git).unwrap();
        let root_stage = stager.staging_root().join(ready_id);
        let package_root = root_stage.join("extracted/rust-mcp-git-v0.1.0-darwin-arm64");
        fs::create_dir_all(&package_root).unwrap();
        let staged_binary = package_root.join("rust-mcp-git");
        fs::copy(&source, &staged_binary).unwrap();
        let asset_name = "rust-mcp-git-v0.1.0-darwin-arm64.tar.gz".to_owned();
        let archive_dir = root_stage.join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        let archive = archive_dir.join(&asset_name);
        fs::write(&archive, b"safe-real-smoke").unwrap();
        let sha = digest_file(&archive);
        fs::write(
            archive_dir.join("SHA256SUMS.txt"),
            format!("{sha}  {asset_name}\n"),
        )
        .unwrap();
        let staged = StagedArtifact {
            component: ComponentId::Git,
            version: Version::parse("0.1.0").unwrap(),
            provider: policy.provider,
            release_tag: "v0.1.0".into(),
            platform: platform(),
            asset_name,
            archive_sha256: sha,
            staging_path: root_stage.clone(),
            package_root,
            validated_executables: vec![staged_binary.clone()],
            validated_executable_sha256: vec![digest_file(&staged_binary)],
            verified_at_unix_seconds: 1,
        };
        fs::write(
            root_stage.join("staged.json"),
            serde_json::to_vec_pretty(&staged).unwrap(),
        )
        .unwrap();
        let lifecycle = Arc::new(FakeLifecycle::default());
        lifecycle.states.lock().unwrap().insert(
            ComponentId::Git,
            LifecycleSnapshot {
                state: LifecycleState::Stopped,
                enabled: true,
            },
        );
        lifecycle
            .targets
            .lock()
            .unwrap()
            .insert(ComponentId::Git, target.clone());
        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.path().join("source"),
                platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let manager = McpUpdateManager::with_dependencies(
            catalog,
            stager,
            Arc::new(NoopProvider),
            lifecycle,
            Arc::new(ProcessActivatedBinaryVerifier),
            inventory,
            EventHub::default(),
        );
        let tx = "txn-git-live-real";
        register_ready(&manager, &staged, tx, ready_id).await;
        let result = manager.apply(ComponentId::Git, tx).await.unwrap();
        assert_eq!(result.phase, McpUpdatePhase::Completed);
        let output = Command::new(&target)
            .arg("--version")
            .output()
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("0.1.0"));
    }
}
