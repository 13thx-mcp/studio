use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{sync::Mutex, time::sleep};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
    storage::HistoryHandle,
    tunnel::{TunnelState, TunnelSupervisor},
};

use super::{
    ArtifactHistoryIdentity, ArtifactStager, ComponentCatalog, ComponentId, InventoryService,
    McpUpdatePhase, McpUpdateTransactionView, OpenAiTunnelReleaseProvider, ReleaseProvider,
    StagedArtifact, Version,
    transaction::{PreparedStagedIdentity, now_ms, sanitize_transaction_error},
};

const CANDIDATE_PREFIX: &str = ".mcp-studio-tunnel-candidate-";
const ROLLBACK_PREFIX: &str = ".mcp-studio-tunnel-rollback-";
const FAILED_PREFIX: &str = ".mcp-studio-tunnel-failed-";
const HEALTH_WINDOW: Duration = Duration::from_millis(800);
const MAX_PACKAGE_FILES: usize = 4096;
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_LOCAL_FILES: usize = 4096;
const MAX_LOCAL_BYTES: u64 = 256 * 1024 * 1024;
const TUNNEL_RECOVERY_SCHEMA_VERSION: u32 = 1;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TunnelRecoveryPhase {
    Prepared,
    ActivationInProgress,
    HealthVerifying,
    Committed,
    RollingBack,
    RolledBack,
    RecoveryFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TunnelRecoveryJournal {
    schema_version: u32,
    revision: u64,
    transaction_id: String,
    source_version: Version,
    target_version: Version,
    source_tree_sha256: String,
    source_runtime_sha256: String,
    local_fingerprint: String,
    target_tree_sha256: String,
    target_runtime_sha256: String,
    previous_current: String,
    candidate_name: String,
    rollback_name: Option<String>,
    failed_name: Option<String>,
    was_running: bool,
    same_version: bool,
    phase: TunnelRecoveryPhase,
    verification: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct TunnelTransactionRecord {
    view: McpUpdateTransactionView,
    staged_id: Option<String>,
    staged_identity: Option<PreparedStagedIdentity>,
    candidate_fingerprint: Option<String>,
}

trait TunnelFsOps: Send + Sync {
    fn sync_dir(&self, path: &Path) -> StudioResult<()>;
}

struct RealTunnelFsOps;

impl TunnelFsOps for RealTunnelFsOps {
    fn sync_dir(&self, path: &Path) -> StudioResult<()> {
        sync_dir(path)
    }
}

#[async_trait]
trait TunnelUpdateLifecycle: Send + Sync {
    async fn state(&self) -> StudioResult<TunnelState>;
    async fn stop(&self) -> StudioResult<()>;
    async fn start(&self) -> StudioResult<()>;
    fn validate_update_binding(&self, install_root: &Path) -> StudioResult<()>;
    async fn launch_generation(&self) -> StudioResult<Option<u64>>;
    async fn verify_launch(
        &self,
        install_root: &Path,
        expected_runtime_sha256: &str,
        previous_generation: Option<u64>,
    ) -> StudioResult<()>;
}

struct SupervisorTunnelLifecycle {
    supervisor: Arc<TunnelSupervisor>,
}

#[async_trait]
impl TunnelUpdateLifecycle for SupervisorTunnelLifecycle {
    async fn state(&self) -> StudioResult<TunnelState> {
        Ok(self.supervisor.status().await.state)
    }

    async fn stop(&self) -> StudioResult<()> {
        self.supervisor.stop().await?;
        Ok(())
    }

    async fn start(&self) -> StudioResult<()> {
        self.supervisor.start().await?;
        Ok(())
    }

    fn validate_update_binding(&self, install_root: &Path) -> StudioResult<()> {
        self.supervisor.validate_update_binding(install_root)
    }

    async fn launch_generation(&self) -> StudioResult<Option<u64>> {
        Ok(self
            .supervisor
            .launch_evidence()
            .await
            .map(|evidence| evidence.generation))
    }

    async fn verify_launch(
        &self,
        install_root: &Path,
        expected_runtime_sha256: &str,
        previous_generation: Option<u64>,
    ) -> StudioResult<()> {
        let evidence = self.supervisor.launch_evidence().await.ok_or_else(|| {
            StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "running tunnel has no launch evidence".into(),
            }
        })?;
        if previous_generation.is_some_and(|generation| generation == evidence.generation) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "tunnel restart did not produce a new owned process generation".into(),
            });
        }
        let expected_root = fs::canonicalize(install_root)?;
        let expected_runtime =
            fs::canonicalize(install_root.join("current/tunnel-client-runtime-cloudflared"))?;
        let expected_config = fs::canonicalize(install_root.join("config.yaml"))?;
        if evidence.working_dir != expected_root
            || evidence.runtime_path != expected_runtime
            || evidence.config_path != expected_config
            || evidence.runtime_sha256 != expected_runtime_sha256
        {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "running tunnel launch identity does not match the activated artifact"
                    .into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct TunnelUpdateManager {
    catalog: ComponentCatalog,
    stager: ArtifactStager,
    provider: Arc<dyn ReleaseProvider>,
    inventory: Arc<InventoryService>,
    lifecycle: Arc<dyn TunnelUpdateLifecycle>,
    events: EventHub,
    active: Arc<StdMutex<bool>>,
    transactions: Arc<Mutex<BTreeMap<String, TunnelTransactionRecord>>>,
    fs_ops: Arc<dyn TunnelFsOps>,
    history: Option<HistoryHandle>,
}

impl TunnelUpdateManager {
    pub fn new(
        catalog: ComponentCatalog,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> StudioResult<Self> {
        Self::new_internal(catalog, tunnel, inventory, events, None)
    }

    pub fn new_with_history(
        catalog: ComponentCatalog,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
        history: HistoryHandle,
    ) -> StudioResult<Self> {
        Self::new_internal(catalog, tunnel, inventory, events, Some(history))
    }

    fn new_internal(
        catalog: ComponentCatalog,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
        history: Option<HistoryHandle>,
    ) -> StudioResult<Self> {
        recover_interrupted_tunnel_files(&catalog)?;
        Ok(Self {
            stager: ArtifactStager::new(catalog.clone())?,
            provider: Arc::new(OpenAiTunnelReleaseProvider::new(catalog.clone())?),
            catalog,
            inventory,
            lifecycle: Arc::new(SupervisorTunnelLifecycle { supervisor: tunnel }),
            events,
            active: Arc::new(StdMutex::new(false)),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
            fs_ops: Arc::new(RealTunnelFsOps),
            history,
        })
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        stager: ArtifactStager,
        provider: Arc<dyn ReleaseProvider>,
        inventory: Arc<InventoryService>,
        lifecycle: Arc<dyn TunnelUpdateLifecycle>,
        events: EventHub,
    ) -> StudioResult<Self> {
        recover_interrupted_tunnel_files(&catalog)?;
        Ok(Self {
            catalog,
            stager,
            provider,
            inventory,
            lifecycle,
            events,
            active: Arc::new(StdMutex::new(false)),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
            fs_ops: Arc::new(RealTunnelFsOps),
            history: None,
        })
    }

    pub async fn prepare(&self, target: Version) -> StudioResult<McpUpdateTransactionView> {
        let _guard = self.acquire()?;
        let source = self.current_installed_version().await?;
        if target.cmp_precedence(&source).is_lt() {
            return Err(StudioError::Conflict(format!(
                "tunnel: target {target} is older than installed {source}"
            )));
        }

        let transaction_id = new_transaction_id();
        let preparing = McpUpdateTransactionView {
            transaction_id: transaction_id.clone(),
            component: ComponentId::Tunnel,
            source_version: Some(source),
            target_version: target.clone(),
            phase: McpUpdatePhase::Preparing,
            was_running: None,
            rollback_succeeded: None,
            error: None,
            updated_at_ms: now_ms(),
        };
        self.transactions.lock().await.insert(
            transaction_id.clone(),
            TunnelTransactionRecord {
                view: preparing.clone(),
                staged_id: None,
                staged_identity: None,
                candidate_fingerprint: None,
            },
        );
        self.emit(&preparing);

        let result = async {
            let policy = self.catalog.component(ComponentId::Tunnel)?.clone();
            let release = self.provider.release(&policy, &target).await?;
            let staged = self
                .stager
                .stage(self.provider.as_ref(), &release, self.inventory.platform())
                .await?;
            if staged.component != ComponentId::Tunnel || staged.version != target {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "staged tunnel identity does not match requested target".into(),
                });
            }
            let staged_id = self.stager.staged_id(&staged)?;
            let candidate = self.prepare_candidate(&transaction_id, &staged)?;
            let fingerprint = tree_fingerprint(&candidate, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES)?;
            Ok::<_, StudioError>((
                staged_id,
                PreparedStagedIdentity::from(&staged),
                fingerprint,
            ))
        }
        .await;

        match result {
            Ok((staged_id, staged_identity, candidate_fingerprint)) => {
                let mut transactions = self.transactions.lock().await;
                let record = transactions
                    .get_mut(&transaction_id)
                    .expect("Tunnel preparing transaction exists");
                record.staged_id = Some(staged_id);
                record.staged_identity = Some(staged_identity);
                record.candidate_fingerprint = Some(candidate_fingerprint);
                record.view.phase = McpUpdatePhase::Staged;
                record.view.updated_at_ms = now_ms();
                let view = record.view.clone();
                drop(transactions);
                self.emit(&view);
                Ok(view)
            }
            Err(error) => {
                let _ = self.remove_candidate(&transaction_id);
                self.update_failed(&transaction_id, None, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    pub async fn apply(&self, transaction_id: &str) -> StudioResult<McpUpdateTransactionView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("tunnel_update")?;
        let _guard = self.acquire()?;
        let (source, target, staged_id, staged_identity, candidate_fingerprint) =
            self.ensure_applicable(transaction_id).await?;

        let installed = self.current_installed_version().await?;
        if installed != source {
            return Err(StudioError::Conflict(
                "tunnel installed version changed after update preparation".into(),
            ));
        }

        let staged = self.stager.load_ready(&staged_id)?;
        if staged.component != ComponentId::Tunnel
            || staged.version != target
            || PreparedStagedIdentity::from(&staged) != staged_identity
        {
            return Err(StudioError::Conflict(
                "tunnel staged artifact changed after preparation".into(),
            ));
        }

        let candidate = candidate_path(
            self.catalog.install_path(ComponentId::Tunnel)?,
            transaction_id,
        )?;
        if tree_fingerprint(&candidate, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES)?
            != candidate_fingerprint
        {
            return Err(StudioError::Conflict(
                "tunnel candidate changed after preparation".into(),
            ));
        }

        let install_root = self.catalog.install_path(ComponentId::Tunnel)?;
        let source_release = validate_current_release(&install_root, &source)?;
        self.lifecycle.validate_update_binding(&install_root)?;
        let source_runtime_sha256 =
            sha256_file(&source_release.join("tunnel-client-runtime-cloudflared"))?;
        let local_before = host_local_fingerprint(&install_root)?;
        let state = self.lifecycle.state().await?;
        let was_running = match state {
            TunnelState::Running => true,
            TunnelState::Stopped => false,
            TunnelState::Starting | TunnelState::Stopping | TunnelState::Failed => {
                return Err(StudioError::Conflict(
                    "tunnel update requires stable running or stopped state".into(),
                ));
            }
        };
        let previous_generation = if was_running {
            self.lifecycle.launch_generation().await?
        } else {
            None
        };
        self.set_was_running(transaction_id, was_running).await?;

        let same_version = target.cmp_precedence(&source).is_eq();
        let source_tree_sha256 =
            tree_fingerprint(&source_release, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES)?;
        let target_runtime_sha256 =
            sha256_file(&candidate.join("tunnel-client-runtime-cloudflared"))?;
        let journal = TunnelRecoveryJournal {
            schema_version: TUNNEL_RECOVERY_SCHEMA_VERSION,
            revision: 1,
            transaction_id: transaction_id.to_owned(),
            source_version: source.clone(),
            target_version: target.clone(),
            source_tree_sha256,
            source_runtime_sha256: source_runtime_sha256.clone(),
            local_fingerprint: local_before.clone(),
            target_tree_sha256: candidate_fingerprint.clone(),
            target_runtime_sha256,
            previous_current: format!("releases/v{source}"),
            candidate_name: format!("{CANDIDATE_PREFIX}{transaction_id}"),
            rollback_name: same_version.then(|| format!("{ROLLBACK_PREFIX}{transaction_id}")),
            failed_name: same_version.then(|| format!("{FAILED_PREFIX}{transaction_id}")),
            was_running,
            same_version,
            phase: TunnelRecoveryPhase::Prepared,
            verification: None,
            error: None,
        };
        if journal_path(&self.catalog, transaction_id)?.exists() {
            return Err(StudioError::Conflict(
                "tunnel recovery journal already exists for transaction".into(),
            ));
        }
        if let Err(error) = persist_recovery_journal(&self.catalog, &journal) {
            self.update_failed(
                transaction_id,
                Some(false),
                format!("failed to persist tunnel recovery journal: {error}"),
            )
            .await?;
            return Err(error);
        }
        let activation = if same_version {
            self.activate_same_version(
                transaction_id,
                &source_release,
                &candidate,
                &source,
                was_running,
                &local_before,
                &source_runtime_sha256,
                previous_generation,
            )
            .await
        } else {
            self.activate_upgrade(
                transaction_id,
                &source_release,
                &candidate,
                &source,
                &target,
                was_running,
                &local_before,
                &source_runtime_sha256,
                previous_generation,
            )
            .await
        };

        match activation {
            Ok(view) => {
                self.inventory
                    .set_desired(ComponentId::Tunnel, target)
                    .await;
                Ok(view)
            }
            Err(error) => Err(error),
        }
    }

    async fn remove_recovery_journal_observed(&self, transaction_id: &str) -> StudioResult<()> {
        let path = journal_path(&self.catalog, transaction_id)?;
        if !path.exists() {
            return Ok(());
        }
        let journal = load_recovery_journal(&self.catalog, transaction_id)?;
        if let Some(history) = &self.history {
            let phase = match journal.phase {
                TunnelRecoveryPhase::Prepared => "prepared",
                TunnelRecoveryPhase::ActivationInProgress => "activation_in_progress",
                TunnelRecoveryPhase::HealthVerifying => "health_verifying",
                TunnelRecoveryPhase::Committed => "committed",
                TunnelRecoveryPhase::RollingBack => "rolling_back",
                TunnelRecoveryPhase::RolledBack => "rolled_back",
                TunnelRecoveryPhase::RecoveryFailed => "recovery_failed",
            };
            match serde_json::to_vec(&journal) {
                Ok(bytes) => {
                    let digest = format!("{:x}", Sha256::digest(&bytes));
                    let history = history.clone();
                    let history_transaction_id = journal.transaction_id.clone();
                    let revision = journal.revision;
                    match tokio::task::spawn_blocking(move || {
                        history.record_validated_journal(
                            "tunnel",
                            &history_transaction_id,
                            revision,
                            &digest,
                            phase,
                        )
                    })
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => tracing::warn!(
                            transaction_id,
                            history_error = %error,
                            "tunnel recovery journal cleanup continues with incomplete history"
                        ),
                        Err(error) => tracing::warn!(
                            transaction_id,
                            history_error = %error,
                            "tunnel history receipt task failed; recovery journal cleanup continues"
                        ),
                    }
                }
                Err(error) => tracing::warn!(
                    transaction_id,
                    history_error = %error,
                    "tunnel history projection serialization failed; recovery journal cleanup continues"
                ),
            }
        }
        remove_recovery_journal(&self.catalog, transaction_id)
    }

    pub async fn recover_startup(&self) -> StudioResult<()> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("tunnel_recovery")?;
        let _guard = self.acquire()?;
        recover_interrupted_tunnel_files(&self.catalog)?;

        for journal in list_recovery_journals(&self.catalog)? {
            let committed = journal.phase == TunnelRecoveryPhase::Committed;
            let (expected, runtime_sha256) = if committed {
                (
                    &journal.target_version,
                    journal.target_runtime_sha256.as_str(),
                )
            } else {
                (
                    &journal.source_version,
                    journal.source_runtime_sha256.as_str(),
                )
            };

            if let Err(error) = self
                .lifecycle
                .validate_update_binding(&self.catalog.install_path(ComponentId::Tunnel)?)
            {
                persist_recovery_failed(&self.catalog, &journal, &error.to_string());
                return Err(error);
            }

            let state = self.lifecycle.state().await?;
            if journal.was_running {
                match state {
                    TunnelState::Stopped | TunnelState::Failed => {
                        if let Err(error) = self.lifecycle.start().await {
                            persist_recovery_failed(&self.catalog, &journal, &error.to_string());
                            return Err(error);
                        }
                    }
                    TunnelState::Running => {}
                    TunnelState::Starting | TunnelState::Stopping => {
                        let error = StudioError::Conflict(
                            "tunnel recovery encountered transitional owner state".into(),
                        );
                        persist_recovery_failed(&self.catalog, &journal, &error.to_string());
                        return Err(error);
                    }
                }
            } else if state != TunnelState::Stopped {
                let error = StudioError::Conflict(
                    "tunnel was stopped before interrupted update but startup owner is not stopped"
                        .into(),
                );
                persist_recovery_failed(&self.catalog, &journal, &error.to_string());
                return Err(error);
            }

            if let Err(error) = self
                .verify_active(
                    expected,
                    journal.was_running,
                    &journal.local_fingerprint,
                    runtime_sha256,
                    None,
                )
                .await
            {
                persist_recovery_failed(&self.catalog, &journal, &error.to_string());
                return Err(error);
            }

            if !committed
                && let Err(error) = update_recovery_journal(
                    &self.catalog,
                    &journal.transaction_id,
                    TunnelRecoveryPhase::RolledBack,
                    Some("startup_predecessor_verified"),
                    journal.error.as_deref(),
                )
            {
                persist_recovery_failed(&self.catalog, &journal, &error.to_string());
                return Err(error);
            }

            match cleanup_recovery_material(&self.catalog, &journal, committed) {
                Ok(()) => {
                    if let Err(error) = self
                        .remove_recovery_journal_observed(&journal.transaction_id)
                        .await
                    {
                        tracing::warn!(
                            %error,
                            transaction_id = %journal.transaction_id,
                            "tunnel recovery complete; journal cleanup deferred"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        transaction_id = %journal.transaction_id,
                        "tunnel recovery verified; cleanup deferred"
                    );
                }
            }
        }
        Ok(())
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
            .map(|identity| identity.history_identity(ComponentId::Tunnel))
    }

    pub async fn transaction(
        &self,
        transaction_id: &str,
    ) -> StudioResult<McpUpdateTransactionView> {
        if !safe_transaction_id(transaction_id) {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        self.transactions
            .lock()
            .await
            .get(transaction_id)
            .map(|record| record.view.clone())
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn activate_upgrade(
        &self,
        transaction_id: &str,
        source_release: &Path,
        candidate: &Path,
        source: &Version,
        target: &Version,
        was_running: bool,
        local_before: &str,
        source_runtime_sha256: &str,
        previous_generation: Option<u64>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let install_root = self.catalog.install_path(ComponentId::Tunnel)?;
        let releases = releases_root(&install_root)?;
        let target_release = releases.join(format!("v{target}"));
        if target_release.exists() {
            return Err(StudioError::Conflict(
                "target tunnel release directory already exists".into(),
            ));
        }

        update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::ActivationInProgress,
            None,
            None,
        )?;
        if was_running {
            self.update_phase(transaction_id, McpUpdatePhase::Stopping, None, None)
                .await?;
            if let Err(error) = self.lifecycle.stop().await {
                self.update_failed(
                    transaction_id,
                    Some(false),
                    format!("tunnel stop failed: {error}"),
                )
                .await?;
                return Err(error);
            }
        }

        self.update_phase(transaction_id, McpUpdatePhase::Activating, None, None)
            .await?;
        if let Err(error) = promote_candidate(candidate, &target_release) {
            if was_running && let Err(restart_error) = self.lifecycle.start().await {
                return self
                    .rollback_failed(
                        transaction_id,
                        format!(
                            "tunnel candidate promotion failed: {error}; failed to restore prior running state: {restart_error}"
                        ),
                    )
                    .await;
            }
            self.update_failed(
                transaction_id,
                Some(false),
                format!("tunnel candidate promotion failed: {error}"),
            )
            .await?;
            return Err(error);
        }
        if let Err(error) = self.set_current(&install_root, target) {
            return self
                .rollback_upgrade(
                    transaction_id,
                    source_release,
                    &target_release,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("tunnel current switch failed after possible rename: {error}"),
                )
                .await;
        }

        let target_runtime_sha256 =
            sha256_file(&target_release.join("tunnel-client-runtime-cloudflared"))?;
        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::HealthVerifying,
            Some("target_activated"),
            None,
        ) {
            return self
                .rollback_upgrade(
                    transaction_id,
                    source_release,
                    &target_release,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("failed to persist tunnel health-verifying state: {error}"),
                )
                .await;
        }

        if was_running {
            self.update_phase(transaction_id, McpUpdatePhase::Starting, None, None)
                .await?;
            if let Err(error) = self.lifecycle.start().await {
                return self
                    .rollback_upgrade(
                        transaction_id,
                        source_release,
                        &target_release,
                        source,
                        was_running,
                        local_before,
                        source_runtime_sha256,
                        format!("tunnel restart failed: {error}"),
                    )
                    .await;
            }
        }

        self.update_phase(transaction_id, McpUpdatePhase::Verifying, None, None)
            .await?;
        if let Err(error) = self
            .verify_active(
                target,
                was_running,
                local_before,
                &target_runtime_sha256,
                previous_generation,
            )
            .await
        {
            return self
                .rollback_upgrade(
                    transaction_id,
                    source_release,
                    &target_release,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("tunnel verification failed: {error}"),
                )
                .await;
        }

        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::Committed,
            Some("target_verified"),
            None,
        ) {
            return self
                .rollback_upgrade(
                    transaction_id,
                    source_release,
                    &target_release,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("failed to persist tunnel committed state: {error}"),
                )
                .await;
        }
        if let Err(error) = self.remove_recovery_journal_observed(transaction_id).await {
            tracing::warn!(%error, transaction_id, "tunnel committed; recovery journal cleanup deferred");
        }
        self.update_phase(transaction_id, McpUpdatePhase::Completed, None, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn activate_same_version(
        &self,
        transaction_id: &str,
        source_release: &Path,
        candidate: &Path,
        source: &Version,
        was_running: bool,
        local_before: &str,
        source_runtime_sha256: &str,
        previous_generation: Option<u64>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let install_root = self.catalog.install_path(ComponentId::Tunnel)?;
        let releases = releases_root(&install_root)?;
        let rollback = releases.join(format!("{ROLLBACK_PREFIX}{transaction_id}"));
        let failed = releases.join(format!("{FAILED_PREFIX}{transaction_id}"));
        if rollback.exists() || failed.exists() {
            return Err(StudioError::Conflict(
                "tunnel force-reinstall scratch path already exists".into(),
            ));
        }

        update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::ActivationInProgress,
            None,
            None,
        )?;
        if was_running {
            self.update_phase(transaction_id, McpUpdatePhase::Stopping, None, None)
                .await?;
            if let Err(error) = self.lifecycle.stop().await {
                self.update_failed(
                    transaction_id,
                    Some(false),
                    format!("tunnel stop failed: {error}"),
                )
                .await?;
                return Err(error);
            }
        }

        self.update_phase(transaction_id, McpUpdatePhase::Activating, None, None)
            .await?;
        if let Err(error) = fs::rename(source_release, &rollback).map_err(StudioError::from) {
            if was_running && let Err(restart_error) = self.lifecycle.start().await {
                return self
                    .rollback_failed(
                        transaction_id,
                        format!(
                            "tunnel source backup rename failed: {error}; failed to restore prior running state: {restart_error}"
                        ),
                    )
                    .await;
            }
            self.update_failed(
                transaction_id,
                Some(false),
                format!("tunnel source backup rename failed: {error}"),
            )
            .await?;
            return Err(error);
        }
        if let Err(error) = sync_dir(&releases) {
            return self
                .rollback_same_version(
                    transaction_id,
                    source_release,
                    &rollback,
                    &failed,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("tunnel source backup durability failed: {error}"),
                )
                .await;
        }
        if let Err(error) = promote_candidate(candidate, source_release) {
            return self
                .rollback_same_version(
                    transaction_id,
                    source_release,
                    &rollback,
                    &failed,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("tunnel same-version candidate promotion failed: {error}"),
                )
                .await;
        }

        let target_runtime_sha256 =
            sha256_file(&source_release.join("tunnel-client-runtime-cloudflared"))?;
        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::HealthVerifying,
            Some("same_version_target_activated"),
            None,
        ) {
            return self
                .rollback_same_version(
                    transaction_id,
                    source_release,
                    &rollback,
                    &failed,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("failed to persist tunnel health-verifying state: {error}"),
                )
                .await;
        }

        if was_running {
            self.update_phase(transaction_id, McpUpdatePhase::Starting, None, None)
                .await?;
            if let Err(error) = self.lifecycle.start().await {
                return self
                    .rollback_same_version(
                        transaction_id,
                        source_release,
                        &rollback,
                        &failed,
                        source,
                        was_running,
                        local_before,
                        source_runtime_sha256,
                        format!("tunnel restart failed: {error}"),
                    )
                    .await;
            }
        }

        self.update_phase(transaction_id, McpUpdatePhase::Verifying, None, None)
            .await?;
        if let Err(error) = self
            .verify_active(
                source,
                was_running,
                local_before,
                &target_runtime_sha256,
                previous_generation,
            )
            .await
        {
            return self
                .rollback_same_version(
                    transaction_id,
                    source_release,
                    &rollback,
                    &failed,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("tunnel verification failed: {error}"),
                )
                .await;
        }

        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::Committed,
            Some("same_version_target_verified"),
            None,
        ) {
            return self
                .rollback_same_version(
                    transaction_id,
                    source_release,
                    &rollback,
                    &failed,
                    source,
                    was_running,
                    local_before,
                    source_runtime_sha256,
                    format!("failed to persist tunnel committed state: {error}"),
                )
                .await;
        }
        match remove_regular_tree(&rollback) {
            Ok(()) => {
                if let Err(error) = self.remove_recovery_journal_observed(transaction_id).await {
                    tracing::warn!(%error, transaction_id, "tunnel committed; recovery journal cleanup deferred");
                }
            }
            Err(error) => {
                tracing::warn!(%error, transaction_id, "tunnel committed; rollback cleanup deferred");
            }
        }
        self.update_phase(transaction_id, McpUpdatePhase::Completed, None, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn rollback_upgrade(
        &self,
        transaction_id: &str,
        source_release: &Path,
        target_release: &Path,
        source: &Version,
        was_running: bool,
        local_before: &str,
        source_runtime_sha256: &str,
        reason: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::RollingBack,
            None,
            Some(&reason),
        ) {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to persist rollback intent: {error}"),
                )
                .await;
        }
        self.update_phase(
            transaction_id,
            McpUpdatePhase::RollingBack,
            None,
            Some(reason.clone()),
        )
        .await?;

        if was_running
            && self.lifecycle.state().await? == TunnelState::Running
            && let Err(error) = self.lifecycle.stop().await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to stop new tunnel: {error}"),
                )
                .await;
        }

        let install_root = self.catalog.install_path(ComponentId::Tunnel)?;
        if let Err(error) = self.set_current(&install_root, source) {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to restore tunnel current release: {error}"),
                )
                .await;
        }
        if validate_current_release(&install_root, source).is_err() || !source_release.is_dir() {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; source tunnel release could not be proven restored"),
                )
                .await;
        }

        if was_running && let Err(error) = self.lifecycle.start().await {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to restart restored tunnel: {error}"),
                )
                .await;
        }

        if let Err(error) = self
            .verify_active(
                source,
                was_running,
                local_before,
                source_runtime_sha256,
                None,
            )
            .await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; restored tunnel verification failed: {error}"),
                )
                .await;
        }

        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::RolledBack,
            Some("predecessor_verified"),
            Some(&reason),
        ) {
            return self
                .rollback_failed(
                    transaction_id,
                    format!(
                        "{reason}; predecessor restored but rollback state was not durable: {error}"
                    ),
                )
                .await;
        }
        let cleanup_ok = remove_regular_tree(target_release).is_ok();
        if cleanup_ok
            && let Err(error) = self.remove_recovery_journal_observed(transaction_id).await
        {
            tracing::warn!(%error, transaction_id, "tunnel rollback complete; journal cleanup deferred");
        }
        self.update_phase(
            transaction_id,
            McpUpdatePhase::Failed,
            Some(true),
            Some(reason.clone()),
        )
        .await?;
        Err(StudioError::UpdateTransaction(reason))
    }

    #[allow(clippy::too_many_arguments)]
    async fn rollback_same_version(
        &self,
        transaction_id: &str,
        active_release: &Path,
        rollback: &Path,
        failed: &Path,
        source: &Version,
        was_running: bool,
        local_before: &str,
        source_runtime_sha256: &str,
        reason: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::RollingBack,
            None,
            Some(&reason),
        ) {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to persist rollback intent: {error}"),
                )
                .await;
        }
        self.update_phase(
            transaction_id,
            McpUpdatePhase::RollingBack,
            None,
            Some(reason.clone()),
        )
        .await?;

        if was_running
            && self.lifecycle.state().await? == TunnelState::Running
            && let Err(error) = self.lifecycle.stop().await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to stop new tunnel: {error}"),
                )
                .await;
        }

        let releases = active_release
            .parent()
            .ok_or_else(|| StudioError::UpdateTransaction("tunnel release has no parent".into()))?;
        let restore = (|| -> StudioResult<()> {
            if active_release.exists() {
                fs::rename(active_release, failed)?;
            }
            fs::rename(rollback, active_release)?;
            sync_dir(releases)?;
            Ok(())
        })();
        if let Err(error) = restore {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to restore same-version tunnel release: {error}"),
                )
                .await;
        }

        if was_running && let Err(error) = self.lifecycle.start().await {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; failed to restart restored tunnel: {error}"),
                )
                .await;
        }

        if let Err(error) = self
            .verify_active(
                source,
                was_running,
                local_before,
                source_runtime_sha256,
                None,
            )
            .await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    format!("{reason}; restored tunnel verification failed: {error}"),
                )
                .await;
        }

        if let Err(error) = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::RolledBack,
            Some("predecessor_verified"),
            Some(&reason),
        ) {
            return self
                .rollback_failed(
                    transaction_id,
                    format!(
                        "{reason}; predecessor restored but rollback state was not durable: {error}"
                    ),
                )
                .await;
        }
        let cleanup_ok = remove_regular_tree(failed).is_ok();
        if cleanup_ok
            && let Err(error) = self.remove_recovery_journal_observed(transaction_id).await
        {
            tracing::warn!(%error, transaction_id, "tunnel rollback complete; journal cleanup deferred");
        }
        self.update_phase(
            transaction_id,
            McpUpdatePhase::Failed,
            Some(true),
            Some(reason.clone()),
        )
        .await?;
        Err(StudioError::UpdateTransaction(reason))
    }

    fn set_current(&self, install_root: &Path, version: &Version) -> StudioResult<()> {
        #[cfg(unix)]
        {
            atomic_set_current_with(install_root, version, |path| self.fs_ops.sync_dir(path))
        }
        #[cfg(not(unix))]
        {
            atomic_set_current(install_root, version)
        }
    }

    async fn verify_active(
        &self,
        expected: &Version,
        was_running: bool,
        local_before: &str,
        expected_runtime_sha256: &str,
        previous_generation: Option<u64>,
    ) -> StudioResult<()> {
        let installed = self.current_installed_version().await?;
        if &installed != expected {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: format!("expected tunnel {expected}, got {installed}"),
            });
        }
        let local_after = host_local_fingerprint(&self.catalog.install_path(ComponentId::Tunnel)?)?;
        if local_after != local_before {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "host-local tunnel config/credentials changed during update".into(),
            });
        }
        let state = self.lifecycle.state().await?;
        if was_running {
            if state != TunnelState::Running {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "tunnel did not return to running state".into(),
                });
            }
            sleep(HEALTH_WINDOW).await;
            if self.lifecycle.state().await? != TunnelState::Running {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "tunnel did not remain running through health window".into(),
                });
            }
            self.lifecycle
                .verify_launch(
                    &self.catalog.install_path(ComponentId::Tunnel)?,
                    expected_runtime_sha256,
                    previous_generation,
                )
                .await?;
        } else if state != TunnelState::Stopped {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "stopped tunnel was unexpectedly started by update".into(),
            });
        }
        Ok(())
    }

    async fn ensure_applicable(
        &self,
        transaction_id: &str,
    ) -> StudioResult<(Version, Version, String, PreparedStagedIdentity, String)> {
        if !safe_transaction_id(transaction_id) {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        let transactions = self.transactions.lock().await;
        let record = transactions
            .get(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        if record.view.phase != McpUpdatePhase::Staged {
            return Err(StudioError::Conflict(format!(
                "tunnel transaction {transaction_id} is not staged"
            )));
        }
        Ok((
            record.view.source_version.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("missing tunnel source version".into())
            })?,
            record.view.target_version.clone(),
            record.staged_id.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("missing tunnel staging id".into())
            })?,
            record.staged_identity.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("missing tunnel staging fingerprint".into())
            })?,
            record.candidate_fingerprint.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("missing tunnel candidate fingerprint".into())
            })?,
        ))
    }

    async fn current_installed_version(&self) -> StudioResult<Version> {
        self.inventory
            .get(ComponentId::Tunnel)
            .await?
            .installed_version
            .ok_or_else(|| {
                StudioError::InstalledIdentity("tunnel installed version is unavailable".into())
            })
    }

    fn prepare_candidate(
        &self,
        transaction_id: &str,
        staged: &StagedArtifact,
    ) -> StudioResult<PathBuf> {
        let install_root = self.catalog.install_path(ComponentId::Tunnel)?;
        let releases = releases_root(&install_root)?;
        let candidate = candidate_path(install_root, transaction_id)?;
        if candidate.exists() {
            return Err(StudioError::Conflict(
                "tunnel candidate already exists".into(),
            ));
        }
        copy_tree(&staged.package_root, &candidate)?;
        if candidate.parent() != Some(releases.as_path()) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "tunnel candidate escaped releases root".into(),
            });
        }
        Ok(candidate)
    }

    fn remove_candidate(&self, transaction_id: &str) -> StudioResult<()> {
        let candidate = candidate_path(
            self.catalog.install_path(ComponentId::Tunnel)?,
            transaction_id,
        )?;
        remove_regular_tree(&candidate)
    }

    async fn set_was_running(&self, transaction_id: &str, was_running: bool) -> StudioResult<()> {
        let mut transactions = self.transactions.lock().await;
        let record = transactions
            .get_mut(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        record.view.was_running = Some(was_running);
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

    async fn update_failed(
        &self,
        transaction_id: &str,
        rollback_succeeded: Option<bool>,
        detail: String,
    ) -> StudioResult<()> {
        let _ = self
            .update_phase(
                transaction_id,
                McpUpdatePhase::Failed,
                rollback_succeeded,
                Some(detail),
            )
            .await?;
        Ok(())
    }

    async fn rollback_failed(
        &self,
        transaction_id: &str,
        detail: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        let _ = update_recovery_journal(
            &self.catalog,
            transaction_id,
            TunnelRecoveryPhase::RecoveryFailed,
            None,
            Some(&detail),
        );
        let _ = self
            .update_phase(
                transaction_id,
                McpUpdatePhase::RollbackFailed,
                Some(false),
                Some(detail.clone()),
            )
            .await?;
        Err(StudioError::RollbackFailed {
            component: ComponentId::Tunnel.to_string(),
            detail,
        })
    }

    fn acquire(&self) -> StudioResult<TunnelUpdateGuard> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| StudioError::UpdateTransaction("tunnel update lock poisoned".into()))?;
        if *active {
            return Err(StudioError::Conflict(
                "tunnel update already in progress".into(),
            ));
        }
        *active = true;
        Ok(TunnelUpdateGuard {
            active: self.active.clone(),
        })
    }

    fn emit(&self, view: &McpUpdateTransactionView) {
        self.events.publish(StudioEvent::UpdateTransaction {
            transaction: view.clone(),
        });
    }
}

#[derive(Debug)]
struct TunnelUpdateGuard {
    active: Arc<StdMutex<bool>>,
}

impl Drop for TunnelUpdateGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = false;
        }
    }
}

fn releases_root(install_root: &Path) -> StudioResult<PathBuf> {
    let releases = install_root.join("releases");
    if !releases.exists() {
        fs::create_dir_all(&releases)?;
    }
    let metadata = fs::symlink_metadata(&releases)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel releases root is unsafe".into(),
        });
    }
    Ok(releases)
}

fn candidate_path(install_root: PathBuf, transaction_id: &str) -> StudioResult<PathBuf> {
    if !safe_transaction_id(transaction_id) {
        return Err(StudioError::UpdateTransaction(
            "invalid tunnel transaction id".into(),
        ));
    }
    Ok(install_root
        .join("releases")
        .join(format!("{CANDIDATE_PREFIX}{transaction_id}")))
}

fn safe_transaction_id(value: &str) -> bool {
    value.starts_with("txn-tunnel-")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn new_transaction_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "txn-tunnel-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn validate_current_release(install_root: &Path, version: &Version) -> StudioResult<PathBuf> {
    let current = install_root.join("current");
    let metadata = fs::symlink_metadata(&current)?;
    if !metadata.file_type().is_symlink() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel current pointer is not a symlink".into(),
        });
    }
    let target = fs::read_link(&current)?;
    let expected = PathBuf::from(format!("releases/v{version}"));
    if target != expected {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel current pointer does not match installed version".into(),
        });
    }
    let releases = fs::canonicalize(releases_root(install_root)?)?;
    let resolved = fs::canonicalize(&current)?;
    if resolved.parent() != Some(releases.as_path()) || !resolved.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel current pointer escaped releases root".into(),
        });
    }
    Ok(resolved)
}

fn atomic_set_current_with(
    install_root: &Path,
    version: &Version,
    sync: impl FnOnce(&Path) -> StudioResult<()>,
) -> StudioResult<()> {
    use std::os::unix::fs::symlink;

    let releases = releases_root(install_root)?;
    let target = releases.join(format!("v{version}"));
    if !target.is_dir() {
        return Err(StudioError::UpdateActivationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "target tunnel release directory is unavailable".into(),
        });
    }
    let temp = install_root.join(format!(".current-{}.tmp", std::process::id()));
    if temp.exists() || temp.is_symlink() {
        fs::remove_file(&temp)?;
    }
    symlink(PathBuf::from(format!("releases/v{version}")), &temp)?;
    fs::rename(&temp, install_root.join("current"))?;
    sync(install_root)?;
    Ok(())
}

#[cfg(not(unix))]
fn atomic_set_current(_install_root: &Path, _version: &Version) -> StudioResult<()> {
    Err(StudioError::UpdateActivationFailed {
        component: ComponentId::Tunnel.to_string(),
        detail: "tunnel activation currently requires Unix symlink semantics".into(),
    })
}

fn promote_candidate(candidate: &Path, target: &Path) -> StudioResult<()> {
    if !candidate.is_dir() || target.exists() {
        return Err(StudioError::UpdateActivationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel candidate promotion preconditions failed".into(),
        });
    }
    fs::rename(candidate, target)?;
    if let Some(parent) = target.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

fn host_local_fingerprint(install_root: &Path) -> StudioResult<String> {
    let mut entries = Vec::<(String, String)>::new();
    let mut total = 0_u64;
    for entry in fs::read_dir(install_root)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "releases" || name == "current" {
            continue;
        }
        collect_snapshot_entries(
            install_root,
            &entry.path(),
            &mut entries,
            &mut total,
            MAX_LOCAL_FILES,
            MAX_LOCAL_BYTES,
        )?;
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, hash) in entries {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(hash.as_bytes());
        digest.update(b"\n");
    }
    Ok(hex_digest(digest.finalize()))
}

fn tree_fingerprint(root: &Path, max_files: usize, max_bytes: u64) -> StudioResult<String> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel package fingerprint root is unsafe".into(),
        });
    }
    let mut entries = Vec::<(String, String)>::new();
    let mut total = 0_u64;
    for entry in fs::read_dir(root)? {
        collect_snapshot_entries(
            root,
            &entry?.path(),
            &mut entries,
            &mut total,
            max_files,
            max_bytes,
        )?;
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, hash) in entries {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(hash.as_bytes());
        digest.update(b"\n");
    }
    Ok(hex_digest(digest.finalize()))
}

fn collect_snapshot_entries(
    root: &Path,
    path: &Path,
    entries: &mut Vec<(String, String)>,
    total: &mut u64,
    max_files: usize,
    max_bytes: u64,
) -> StudioResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "tunnel snapshot path escaped root".into(),
            })?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push((relative, format!("symlink:{}", target.to_string_lossy())));
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            collect_snapshot_entries(root, &entry?.path(), entries, total, max_files, max_bytes)?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel snapshot contains unsupported special file".into(),
        });
    }
    if entries.len() >= max_files {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel snapshot contains too many files".into(),
        });
    }
    *total = total.saturating_add(metadata.len());
    if *total > max_bytes {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel snapshot exceeds size limit".into(),
        });
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel snapshot path escaped root".into(),
        })?
        .to_string_lossy()
        .replace('\\', "/");
    entries.push((relative, sha256_file(path)?));
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> StudioResult<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "staged tunnel package root is unsafe".into(),
        });
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
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "staged tunnel package contains a symlink".into(),
            });
        }
        if metadata.is_dir() {
            fs::create_dir(&dst)?;
            copy_tree_inner(&src, &dst)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "staged tunnel package contains unsupported entry".into(),
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
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel transaction tree is unsafe".into(),
        });
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn sync_dir(path: &Path) -> StudioResult<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn sha256_file(path: &Path) -> StudioResult<String> {
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
    Ok(hex_digest(digest.finalize()))
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn recovery_root(catalog: &ComponentCatalog) -> PathBuf {
    catalog
        .runtime_root()
        .join("studio")
        .join("data")
        .join("tunnel-update")
}

fn ensure_recovery_root(catalog: &ComponentCatalog) -> StudioResult<PathBuf> {
    let runtime_root = catalog.runtime_root();
    let metadata = fs::symlink_metadata(runtime_root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "trusted runtime root is not a regular directory".into(),
        });
    }
    let mut parent = runtime_root.to_path_buf();
    for name in ["studio", "data", "tunnel-update"] {
        let next = parent.join(name);
        if next.exists() {
            let metadata = fs::symlink_metadata(&next)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "tunnel recovery root contains an unsafe path component".into(),
                });
            }
        } else {
            fs::create_dir(&next)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&next, fs::Permissions::from_mode(0o700))?;
            }
            sync_dir(&parent)?;
        }
        parent = next;
    }
    Ok(parent)
}

fn validate_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_recovery_journal(journal: &TunnelRecoveryJournal) -> StudioResult<()> {
    if journal.schema_version != TUNNEL_RECOVERY_SCHEMA_VERSION
        || !safe_transaction_id(&journal.transaction_id)
        || !validate_digest(&journal.source_tree_sha256)
        || !validate_digest(&journal.source_runtime_sha256)
        || !validate_digest(&journal.local_fingerprint)
        || !validate_digest(&journal.target_tree_sha256)
        || !validate_digest(&journal.target_runtime_sha256)
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel recovery journal identity/schema is invalid".into(),
        });
    }
    let expected_current = format!("releases/v{}", journal.source_version);
    let expected_candidate = format!("{CANDIDATE_PREFIX}{}", journal.transaction_id);
    let expected_rollback = format!("{ROLLBACK_PREFIX}{}", journal.transaction_id);
    let expected_failed = format!("{FAILED_PREFIX}{}", journal.transaction_id);
    if journal.previous_current != expected_current
        || journal.candidate_name != expected_candidate
        || journal.rollback_name.as_deref()
            != journal.same_version.then_some(expected_rollback.as_str())
        || journal.failed_name.as_deref()
            != journal.same_version.then_some(expected_failed.as_str())
        || journal.same_version
            != journal
                .target_version
                .cmp_precedence(&journal.source_version)
                .is_eq()
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel recovery journal contains inconsistent derived identities".into(),
        });
    }
    Ok(())
}

fn journal_path(catalog: &ComponentCatalog, transaction_id: &str) -> StudioResult<PathBuf> {
    if !safe_transaction_id(transaction_id) {
        return Err(StudioError::NotFound(transaction_id.to_owned()));
    }
    Ok(recovery_root(catalog).join(format!("{transaction_id}.json")))
}

fn persist_recovery_journal(
    catalog: &ComponentCatalog,
    journal: &TunnelRecoveryJournal,
) -> StudioResult<()> {
    validate_recovery_journal(journal)?;
    let root = ensure_recovery_root(catalog)?;
    let destination = root.join(format!("{}.json", journal.transaction_id));
    if destination.exists() {
        let metadata = fs::symlink_metadata(&destination)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "tunnel recovery journal destination is unsafe".into(),
            });
        }
    }
    let mut bytes = serde_json::to_vec_pretty(journal)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(StudioError::UpdateTransaction(
            "tunnel recovery journal exceeds size limit".into(),
        ));
    }
    let temp = root.join(format!(
        ".{}.{}.{}.tmp",
        journal.transaction_id,
        std::process::id(),
        journal.revision
    ));
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
        sync_dir(&root)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

fn load_recovery_journal(
    catalog: &ComponentCatalog,
    transaction_id: &str,
) -> StudioResult<TunnelRecoveryJournal> {
    let path = journal_path(catalog, transaction_id)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_JOURNAL_BYTES
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel recovery journal is unsafe or oversized".into(),
        });
    }
    let journal: TunnelRecoveryJournal = serde_json::from_slice(&fs::read(path)?)?;
    validate_recovery_journal(&journal)?;
    if journal.transaction_id != transaction_id {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel recovery journal filename/transaction mismatch".into(),
        });
    }
    Ok(journal)
}

fn list_recovery_journals(catalog: &ComponentCatalog) -> StudioResult<Vec<TunnelRecoveryJournal>> {
    let root = ensure_recovery_root(catalog)?;
    let mut entries = fs::read_dir(&root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut journals = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "tunnel recovery directory contains a non-UTF8 entry".into(),
            });
        };
        if name.starts_with('.') && name.ends_with(".tmp") {
            continue;
        }
        if !name.ends_with(".json") {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "tunnel recovery directory contains an unexpected entry".into(),
            });
        }
        let transaction_id = name.trim_end_matches(".json");
        journals.push(load_recovery_journal(catalog, transaction_id)?);
    }
    Ok(journals)
}

fn remove_recovery_journal(catalog: &ComponentCatalog, transaction_id: &str) -> StudioResult<()> {
    let path = journal_path(catalog, transaction_id)?;
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "tunnel recovery journal cleanup target is unsafe".into(),
        });
    }
    fs::remove_file(path)?;
    sync_dir(&ensure_recovery_root(catalog)?)?;
    Ok(())
}

fn update_recovery_journal(
    catalog: &ComponentCatalog,
    transaction_id: &str,
    phase: TunnelRecoveryPhase,
    verification: Option<&str>,
    error: Option<&str>,
) -> StudioResult<TunnelRecoveryJournal> {
    let mut journal = load_recovery_journal(catalog, transaction_id)?;
    journal.revision = journal.revision.saturating_add(1);
    journal.phase = phase;
    journal.verification = verification.map(ToOwned::to_owned);
    journal.error = error.map(sanitize_transaction_error);
    persist_recovery_journal(catalog, &journal)?;
    Ok(journal)
}

fn tree_matches(path: &Path, expected: &str) -> StudioResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(false);
    }
    Ok(tree_fingerprint(path, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES)? == expected)
}

#[cfg(unix)]
fn durable_set_current(install_root: &Path, version: &Version) -> StudioResult<()> {
    atomic_set_current_with(install_root, version, sync_dir)
}

#[cfg(not(unix))]
fn durable_set_current(install_root: &Path, version: &Version) -> StudioResult<()> {
    atomic_set_current(install_root, version)
}

fn persist_recovery_failed(
    catalog: &ComponentCatalog,
    journal: &TunnelRecoveryJournal,
    detail: &str,
) {
    let mut failed = journal.clone();
    failed.revision = failed.revision.saturating_add(1);
    failed.phase = TunnelRecoveryPhase::RecoveryFailed;
    failed.error = Some(sanitize_transaction_error(detail));
    let _ = persist_recovery_journal(catalog, &failed);
}

fn restore_predecessor_files(
    catalog: &ComponentCatalog,
    journal: &TunnelRecoveryJournal,
) -> StudioResult<()> {
    let install_root = catalog.install_path(ComponentId::Tunnel)?;
    let releases = releases_root(&install_root)?;
    let source_release = releases.join(format!("v{}", journal.source_version));
    if journal.same_version {
        if !tree_matches(&source_release, &journal.source_tree_sha256)? {
            let rollback = releases.join(journal.rollback_name.as_deref().ok_or_else(|| {
                StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "same-version recovery has no rollback identity".into(),
                }
            })?);
            if !tree_matches(&rollback, &journal.source_tree_sha256)? {
                return Err(StudioError::RollbackFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "same-version predecessor bytes are unavailable or untrusted".into(),
                });
            }
            if source_release.exists() {
                if !tree_matches(&source_release, &journal.target_tree_sha256)? {
                    return Err(StudioError::RollbackFailed {
                        component: ComponentId::Tunnel.to_string(),
                        detail: "active same-version replacement identity is ambiguous".into(),
                    });
                }
                let failed = releases.join(journal.failed_name.as_deref().ok_or_else(|| {
                    StudioError::RollbackFailed {
                        component: ComponentId::Tunnel.to_string(),
                        detail: "same-version recovery has no failed identity".into(),
                    }
                })?);
                if failed.exists() {
                    return Err(StudioError::RollbackFailed {
                        component: ComponentId::Tunnel.to_string(),
                        detail: "multiple same-version failed recovery candidates exist".into(),
                    });
                }
                fs::rename(&source_release, &failed)?;
                sync_dir(&releases)?;
            }
            fs::rename(&rollback, &source_release)?;
            sync_dir(&releases)?;
        }
    } else if !tree_matches(&source_release, &journal.source_tree_sha256)? {
        return Err(StudioError::RollbackFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "upgrade predecessor release is unavailable or changed".into(),
        });
    }

    let source_runtime = source_release.join("tunnel-client-runtime-cloudflared");
    if sha256_file(&source_runtime)? != journal.source_runtime_sha256 {
        return Err(StudioError::RollbackFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "restored predecessor runtime fingerprint mismatch".into(),
        });
    }
    durable_set_current(&install_root, &journal.source_version)?;
    validate_current_release(&install_root, &journal.source_version)?;
    update_recovery_journal(
        catalog,
        &journal.transaction_id,
        TunnelRecoveryPhase::RollingBack,
        Some("filesystem_predecessor_restored"),
        journal.error.as_deref(),
    )?;
    Ok(())
}

fn verify_committed_files(
    catalog: &ComponentCatalog,
    journal: &TunnelRecoveryJournal,
) -> StudioResult<()> {
    let install_root = catalog.install_path(ComponentId::Tunnel)?;
    let target_release = validate_current_release(&install_root, &journal.target_version)?;
    if !tree_matches(&target_release, &journal.target_tree_sha256)? {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "committed tunnel target tree fingerprint mismatch".into(),
        });
    }
    if sha256_file(&target_release.join("tunnel-client-runtime-cloudflared"))?
        != journal.target_runtime_sha256
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Tunnel.to_string(),
            detail: "committed tunnel target runtime fingerprint mismatch".into(),
        });
    }
    Ok(())
}

fn cleanup_recovery_material(
    catalog: &ComponentCatalog,
    journal: &TunnelRecoveryJournal,
    committed: bool,
) -> StudioResult<()> {
    let install_root = catalog.install_path(ComponentId::Tunnel)?;
    let releases = releases_root(&install_root)?;
    let candidate = releases.join(&journal.candidate_name);
    if candidate.exists() {
        if !tree_matches(&candidate, &journal.target_tree_sha256)? {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Tunnel.to_string(),
                detail: "candidate cleanup identity mismatch".into(),
            });
        }
        remove_regular_tree(&candidate)?;
    }

    if let Some(name) = &journal.rollback_name {
        let rollback = releases.join(name);
        if rollback.exists() {
            if !tree_matches(&rollback, &journal.source_tree_sha256)? {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "rollback cleanup identity mismatch".into(),
                });
            }
            remove_regular_tree(&rollback)?;
        }
    }
    if let Some(name) = &journal.failed_name {
        let failed = releases.join(name);
        if failed.exists() {
            if !tree_matches(&failed, &journal.target_tree_sha256)? {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "failed-candidate cleanup identity mismatch".into(),
                });
            }
            remove_regular_tree(&failed)?;
        }
    }

    if !committed && !journal.same_version {
        let target_release = releases.join(format!("v{}", journal.target_version));
        if target_release.exists() {
            if !tree_matches(&target_release, &journal.target_tree_sha256)? {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "rollback target cleanup identity mismatch".into(),
                });
            }
            remove_regular_tree(&target_release)?;
        }
    }
    sync_dir(&releases)?;
    Ok(())
}

fn recover_interrupted_tunnel_files(catalog: &ComponentCatalog) -> StudioResult<()> {
    let install_root = catalog.install_path(ComponentId::Tunnel)?;
    if !install_root.exists() {
        return Ok(());
    }
    let releases = releases_root(&install_root)?;
    let journals = list_recovery_journals(catalog)?;
    if journals.len() > 1 {
        return Err(StudioError::Conflict(
            "multiple tunnel recovery journals require operator repair".into(),
        ));
    }

    let mut referenced = std::collections::BTreeSet::new();
    for journal in &journals {
        referenced.insert(journal.candidate_name.clone());
        if let Some(name) = &journal.rollback_name {
            referenced.insert(name.clone());
        }
        if let Some(name) = &journal.failed_name {
            referenced.insert(name.clone());
        }
    }
    for entry in fs::read_dir(&releases)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if (name.starts_with(CANDIDATE_PREFIX)
            || name.starts_with(ROLLBACK_PREFIX)
            || name.starts_with(FAILED_PREFIX))
            && !referenced.contains(&name)
        {
            return Err(StudioError::Conflict(
                "legacy or ambiguous tunnel recovery material requires operator repair".into(),
            ));
        }
    }

    for journal in journals {
        match journal.phase {
            TunnelRecoveryPhase::Committed => {
                if let Err(error) = verify_committed_files(catalog, &journal) {
                    persist_recovery_failed(catalog, &journal, &error.to_string());
                    return Err(error);
                }
            }
            TunnelRecoveryPhase::RecoveryFailed => {
                return Err(StudioError::RollbackFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: journal
                        .error
                        .unwrap_or_else(|| "tunnel recovery previously failed".into()),
                });
            }
            TunnelRecoveryPhase::Prepared
            | TunnelRecoveryPhase::ActivationInProgress
            | TunnelRecoveryPhase::HealthVerifying
            | TunnelRecoveryPhase::RollingBack
            | TunnelRecoveryPhase::RolledBack => {
                if let Err(error) = restore_predecessor_files(catalog, &journal) {
                    persist_recovery_failed(catalog, &journal, &error.to_string());
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as TestMutex;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        storage::HistoryHandle,
        update::{
            Architecture, AvailableRelease, HostRuntimeRoots, OperatingSystem, Platform,
            ReleaseAsset, ReleaseProviderId,
        },
    };

    struct NoopProvider;

    #[async_trait]
    impl ReleaseProvider for NoopProvider {
        fn provider_id(&self) -> ReleaseProviderId {
            ReleaseProviderId::OpenAiGitHub
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

    struct FakeLifecycle {
        state: TestMutex<TunnelState>,
        stop_count: TestMutex<u32>,
        start_count: TestMutex<u32>,
        fail_start_once: TestMutex<bool>,
        fail_start_always: TestMutex<bool>,
        fail_binding: TestMutex<bool>,
        stale_generation: TestMutex<bool>,
        generation: TestMutex<u64>,
    }

    impl FakeLifecycle {
        fn new(state: TunnelState) -> Self {
            Self {
                state: TestMutex::new(state),
                stop_count: TestMutex::new(0),
                start_count: TestMutex::new(0),
                fail_start_once: TestMutex::new(false),
                fail_start_always: TestMutex::new(false),
                fail_binding: TestMutex::new(false),
                stale_generation: TestMutex::new(false),
                generation: TestMutex::new(if state == TunnelState::Running { 1 } else { 0 }),
            }
        }
    }

    #[async_trait]
    impl TunnelUpdateLifecycle for FakeLifecycle {
        async fn state(&self) -> StudioResult<TunnelState> {
            Ok(*self.state.lock().unwrap())
        }

        async fn stop(&self) -> StudioResult<()> {
            *self.stop_count.lock().unwrap() += 1;
            *self.state.lock().unwrap() = TunnelState::Stopped;
            Ok(())
        }

        async fn start(&self) -> StudioResult<()> {
            *self.start_count.lock().unwrap() += 1;
            if *self.fail_start_always.lock().unwrap() {
                return Err(StudioError::Process("forced tunnel start failure".into()));
            }
            let mut fail_once = self.fail_start_once.lock().unwrap();
            if *fail_once {
                *fail_once = false;
                return Err(StudioError::Process("forced tunnel start failure".into()));
            }
            *self.state.lock().unwrap() = TunnelState::Running;
            if !*self.stale_generation.lock().unwrap() {
                let mut generation = self.generation.lock().unwrap();
                *generation = generation.wrapping_add(1);
            }
            Ok(())
        }

        fn validate_update_binding(&self, _install_root: &Path) -> StudioResult<()> {
            if *self.fail_binding.lock().unwrap() {
                return Err(StudioError::Config(
                    "forced stale tunnel launch binding".into(),
                ));
            }
            Ok(())
        }

        async fn launch_generation(&self) -> StudioResult<Option<u64>> {
            Ok(if *self.state.lock().unwrap() == TunnelState::Running {
                Some(*self.generation.lock().unwrap())
            } else {
                None
            })
        }

        async fn verify_launch(
            &self,
            _install_root: &Path,
            _expected_runtime_sha256: &str,
            previous_generation: Option<u64>,
        ) -> StudioResult<()> {
            if *self.state.lock().unwrap() != TunnelState::Running {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "fake tunnel is not running".into(),
                });
            }
            let generation = *self.generation.lock().unwrap();
            if previous_generation.is_some_and(|previous| previous == generation) {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Tunnel.to_string(),
                    detail: "fake tunnel launch generation did not advance".into(),
                });
            }
            Ok(())
        }
    }

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn write_version_script(path: &Path, version: &str, marker: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"{version} runtime\"; exit 0; fi\necho {marker}\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn write_release(root: &Path, version: &str, marker: &str) {
        fs::create_dir_all(root).unwrap();
        write_version_script(
            &root.join("tunnel-client-runtime-cloudflared"),
            version,
            marker,
        );
        write_version_script(&root.join("cloudflared"), version, marker);
        fs::write(
            root.join("cloudflared-manifest.json"),
            serde_json::to_vec(&json!({"version":"test"})).unwrap(),
        )
        .unwrap();
        fs::write(root.join("LICENSE"), "license").unwrap();
        fs::write(root.join("NOTICE"), "notice").unwrap();
        fs::write(root.join("bundle-licenses.txt"), "licenses").unwrap();
        fs::write(root.join("bundle.spdx.json"), "{}").unwrap();
    }

    fn digest(path: &Path) -> String {
        sha256_file(path).unwrap()
    }

    struct Fixture {
        _temp: TempDir,
        manager: TunnelUpdateManager,
        lifecycle: Arc<FakeLifecycle>,
        install_root: PathBuf,
    }

    fn fixture(running: bool) -> Fixture {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let runtime = root.join("runtime");
        let bin = root.join("bin");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let catalog = ComponentCatalog::new(HostRuntimeRoots::new(bin, runtime).unwrap());
        let install_root = catalog.install_path(ComponentId::Tunnel).unwrap();
        let release = install_root.join("releases/v1.0.0");
        write_release(&release, "1.0.0", "old");
        fs::write(install_root.join("config.yaml"), "secret: preserve\n").unwrap();
        fs::create_dir_all(install_root.join("credentials")).unwrap();
        fs::write(install_root.join("credentials/token"), "do-not-touch").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("releases/v1.0.0", install_root.join("current")).unwrap();

        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.join("missing-source"),
                platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let lifecycle = Arc::new(FakeLifecycle::new(if running {
            TunnelState::Running
        } else {
            TunnelState::Stopped
        }));
        let manager = TunnelUpdateManager::with_dependencies(
            catalog,
            stager,
            Arc::new(NoopProvider),
            inventory,
            lifecycle.clone(),
            EventHub::default(),
        )
        .unwrap();
        Fixture {
            _temp: temp,
            manager,
            lifecycle,
            install_root,
        }
    }

    fn create_ready(
        manager: &TunnelUpdateManager,
        version: &str,
        ready_id: &str,
        marker: &str,
    ) -> StagedArtifact {
        let root = manager.stager.staging_root().join(ready_id);
        let package_root = root.join("extracted");
        write_release(&package_root, version, marker);
        let stem = format!("tunnel-client-runtime-cloudflared-v{version}-darwin-arm64");
        let license = package_root.join(format!("{stem}-licenses.txt"));
        let spdx = package_root.join(format!("{stem}.spdx.json"));
        fs::rename(package_root.join("bundle-licenses.txt"), license).unwrap();
        fs::rename(package_root.join("bundle.spdx.json"), spdx).unwrap();

        let archive_dir = root.join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        let asset_name = format!("{stem}.zip");
        let archive = archive_dir.join(&asset_name);
        fs::write(&archive, b"verified-tunnel-archive").unwrap();
        let archive_sha = digest(&archive);
        fs::write(
            archive_dir.join("SHA256SUMS.txt"),
            format!("{archive_sha}  {asset_name}\n"),
        )
        .unwrap();

        let runtime = package_root.join("tunnel-client-runtime-cloudflared");
        let cloudflared = package_root.join("cloudflared");
        let staged = StagedArtifact {
            component: ComponentId::Tunnel,
            version: Version::parse(version).unwrap(),
            provider: ReleaseProviderId::OpenAiGitHub,
            release_tag: format!("v{version}"),
            platform: platform(),
            asset_name,
            archive_sha256: archive_sha,
            staging_path: root.clone(),
            package_root,
            validated_executables: vec![runtime.clone(), cloudflared.clone()],
            validated_executable_sha256: vec![digest(&runtime), digest(&cloudflared)],
            verified_at_unix_seconds: 1,
        };
        fs::write(
            root.join("staged.json"),
            serde_json::to_vec_pretty(&staged).unwrap(),
        )
        .unwrap();
        staged
    }

    async fn register_ready(
        manager: &TunnelUpdateManager,
        staged: &StagedArtifact,
        transaction_id: &str,
        ready_id: &str,
    ) {
        let source = manager.current_installed_version().await.unwrap();
        let candidate = manager.prepare_candidate(transaction_id, staged).unwrap();
        manager.transactions.lock().await.insert(
            transaction_id.to_owned(),
            TunnelTransactionRecord {
                view: McpUpdateTransactionView {
                    transaction_id: transaction_id.to_owned(),
                    component: ComponentId::Tunnel,
                    source_version: Some(source),
                    target_version: staged.version.clone(),
                    phase: McpUpdatePhase::Staged,
                    was_running: None,
                    rollback_succeeded: None,
                    error: None,
                    updated_at_ms: now_ms(),
                },
                staged_id: Some(ready_id.to_owned()),
                staged_identity: Some(PreparedStagedIdentity::from(staged)),
                candidate_fingerprint: Some(
                    tree_fingerprint(&candidate, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES).unwrap(),
                ),
            },
        );
    }

    fn seed_recovery_journal(
        manager: &TunnelUpdateManager,
        transaction_id: &str,
        target: &Version,
        was_running: bool,
    ) -> TunnelRecoveryJournal {
        let install_root = manager.catalog.install_path(ComponentId::Tunnel).unwrap();
        let source = Version::parse("1.0.0").unwrap();
        let source_release = install_root.join("releases/v1.0.0");
        let candidate = candidate_path(install_root.clone(), transaction_id).unwrap();
        let same_version = target.cmp_precedence(&source).is_eq();
        let journal = TunnelRecoveryJournal {
            schema_version: TUNNEL_RECOVERY_SCHEMA_VERSION,
            revision: 1,
            transaction_id: transaction_id.to_owned(),
            source_version: source.clone(),
            target_version: target.clone(),
            source_tree_sha256: tree_fingerprint(
                &source_release,
                MAX_PACKAGE_FILES,
                MAX_PACKAGE_BYTES,
            )
            .unwrap(),
            source_runtime_sha256: digest(
                &source_release.join("tunnel-client-runtime-cloudflared"),
            ),
            local_fingerprint: host_local_fingerprint(&install_root).unwrap(),
            target_tree_sha256: tree_fingerprint(&candidate, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES)
                .unwrap(),
            target_runtime_sha256: digest(&candidate.join("tunnel-client-runtime-cloudflared")),
            previous_current: format!("releases/v{source}"),
            candidate_name: format!("{CANDIDATE_PREFIX}{transaction_id}"),
            rollback_name: same_version.then(|| format!("{ROLLBACK_PREFIX}{transaction_id}")),
            failed_name: same_version.then(|| format!("{FAILED_PREFIX}{transaction_id}")),
            was_running,
            same_version,
            phase: TunnelRecoveryPhase::Prepared,
            verification: None,
            error: None,
        };
        persist_recovery_journal(&manager.catalog, &journal).unwrap();
        journal
    }

    fn assert_local_preserved(root: &Path) {
        assert_eq!(
            fs::read_to_string(root.join("config.yaml")).unwrap(),
            "secret: preserve\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("credentials/token")).unwrap(),
            "do-not-touch"
        );
    }

    #[tokio::test]
    async fn tunnel_terminal_receipt_precedes_journal_cleanup() {
        let mut fixture = fixture(false);
        let tx = "txn-tunnel-history-order";
        let staged = create_ready(&fixture.manager, "1.1.0", "ready-history-order", "new");
        fixture.manager.prepare_candidate(tx, &staged).unwrap();
        seed_recovery_journal(&fixture.manager, tx, &staged.version, false);

        let runtime_root = fixture.manager.catalog.runtime_root().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        }

        let history = HistoryHandle::initialize(&runtime_root);
        fixture.manager.history = Some(history.clone());
        fixture
            .manager
            .remove_recovery_journal_observed(tx)
            .await
            .unwrap();
        assert!(!journal_path(&fixture.manager.catalog, tx).unwrap().exists());

        let database = history.root().join("studio.sqlite3");
        let connection = rusqlite::Connection::open(&database).unwrap();
        let watermark: i64 = connection
            .query_row(
                "SELECT highest_revision FROM journal_watermarks
                 WHERE domain='tunnel' AND transaction_id=?1",
                [tx],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(watermark, 1);
        drop(connection);
        history.shutdown();
    }

    #[tokio::test]
    async fn history_failure_does_not_retain_recovery_journal() {
        let mut fixture = fixture(false);
        let tx = "txn-tunnel-history-unavailable";
        let staged = create_ready(
            &fixture.manager,
            "1.1.0",
            "ready-history-unavailable",
            "new",
        );
        fixture.manager.prepare_candidate(tx, &staged).unwrap();
        seed_recovery_journal(&fixture.manager, tx, &staged.version, false);

        let runtime_root = fixture.manager.catalog.runtime_root().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let unavailable = HistoryHandle::initialize(&runtime_root);
        unavailable.shutdown();
        fixture.manager.history = Some(unavailable);

        fixture
            .manager
            .remove_recovery_journal_observed(tx)
            .await
            .unwrap();
        assert!(
            !journal_path(&fixture.manager.catalog, tx).unwrap().exists(),
            "audit failure must not retain a recovery-authority journal"
        );
    }

    #[tokio::test]
    async fn stopped_upgrade_switches_current_and_remains_stopped() {
        let fixture = fixture(false);
        let ready_id = "ready-tunnel-stopped";
        let tx = "txn-tunnel-stopped";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        let view = fixture.manager.apply(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Completed);
        assert_eq!(*fixture.lifecycle.stop_count.lock().unwrap(), 0);
        assert_eq!(*fixture.lifecycle.start_count.lock().unwrap(), 0);
        assert_eq!(
            fs::read_link(fixture.install_root.join("current")).unwrap(),
            PathBuf::from("releases/v1.1.0")
        );
        assert_local_preserved(&fixture.install_root);
    }

    #[tokio::test]
    async fn running_upgrade_stops_starts_and_health_verifies() {
        let fixture = fixture(true);
        let ready_id = "ready-tunnel-running";
        let tx = "txn-tunnel-running";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        let view = fixture.manager.apply(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Completed);
        assert_eq!(*fixture.lifecycle.stop_count.lock().unwrap(), 1);
        assert_eq!(*fixture.lifecycle.start_count.lock().unwrap(), 1);
        assert_eq!(
            fixture.lifecycle.state().await.unwrap(),
            TunnelState::Running
        );
        assert_local_preserved(&fixture.install_root);
    }

    #[tokio::test]
    async fn same_version_force_reinstall_replaces_release_and_preserves_config() {
        let fixture = fixture(true);
        let ready_id = "ready-tunnel-force";
        let tx = "txn-tunnel-force";
        let staged = create_ready(&fixture.manager, "1.0.0", ready_id, "reinstalled");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        let view = fixture.manager.apply(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Completed);
        let runtime = fs::read_to_string(
            fixture
                .install_root
                .join("releases/v1.0.0/tunnel-client-runtime-cloudflared"),
        )
        .unwrap();
        assert!(runtime.contains("reinstalled"));
        assert_eq!(
            fs::read_link(fixture.install_root.join("current")).unwrap(),
            PathBuf::from("releases/v1.0.0")
        );
        assert_local_preserved(&fixture.install_root);
    }

    #[tokio::test]
    async fn restart_failure_rolls_back_upgrade_and_restores_running_state() {
        let fixture = fixture(true);
        *fixture.lifecycle.fail_start_once.lock().unwrap() = true;
        let ready_id = "ready-tunnel-rollback";
        let tx = "txn-tunnel-rollback";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        assert!(fixture.manager.apply(tx).await.is_err());
        assert_eq!(
            fs::read_link(fixture.install_root.join("current")).unwrap(),
            PathBuf::from("releases/v1.0.0")
        );
        assert_eq!(
            fixture.lifecycle.state().await.unwrap(),
            TunnelState::Running
        );
        let view = fixture.manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
        assert_local_preserved(&fixture.install_root);
    }

    #[tokio::test]
    async fn candidate_tamper_and_unsafe_current_fail_before_stop() {
        let tamper_fixture = fixture(true);
        let ready_id = "ready-tunnel-tamper";
        let tx = "txn-tunnel-tamper";
        let staged = create_ready(&tamper_fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&tamper_fixture.manager, &staged, tx, ready_id).await;
        fs::write(
            candidate_path(tamper_fixture.install_root.clone(), tx)
                .unwrap()
                .join("NOTICE"),
            "tampered",
        )
        .unwrap();
        assert!(tamper_fixture.manager.apply(tx).await.is_err());
        assert_eq!(*tamper_fixture.lifecycle.stop_count.lock().unwrap(), 0);

        let fixture = fixture(true);
        let ready_id = "ready-tunnel-current";
        let tx = "txn-tunnel-current";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;
        fs::remove_file(fixture.install_root.join("current")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("../outside", fixture.install_root.join("current")).unwrap();
        assert!(fixture.manager.apply(tx).await.is_err());
        assert_eq!(*fixture.lifecycle.stop_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn downgrade_and_duplicate_lock_fail_closed() {
        let fixture = fixture(false);
        let source = fixture.manager.current_installed_version().await.unwrap();
        assert_eq!(source.to_string(), "1.0.0");
        assert!(matches!(
            fixture
                .manager
                .prepare(Version::parse("0.9.0").unwrap())
                .await,
            Err(StudioError::Conflict(_))
        ));
        let _guard = fixture.manager.acquire().unwrap();
        assert!(matches!(
            fixture.manager.acquire().unwrap_err(),
            StudioError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn stale_launch_generation_rolls_back_verified_target() {
        let fixture = fixture(true);
        *fixture.lifecycle.stale_generation.lock().unwrap() = true;
        let ready_id = "ready-tunnel-stale-generation";
        let tx = "txn-tunnel-stale-generation";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        assert!(fixture.manager.apply(tx).await.is_err());
        assert_eq!(
            fs::read_link(fixture.install_root.join("current")).unwrap(),
            PathBuf::from("releases/v1.0.0")
        );
        let view = fixture.manager.transaction(tx).await.unwrap();
        assert_eq!(view.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn stale_launch_binding_fails_before_stop() {
        let fixture = fixture(true);
        *fixture.lifecycle.fail_binding.lock().unwrap() = true;
        let ready_id = "ready-tunnel-stale-binding";
        let tx = "txn-tunnel-stale-binding";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        assert!(fixture.manager.apply(tx).await.is_err());
        assert_eq!(*fixture.lifecycle.stop_count.lock().unwrap(), 0);
        assert_eq!(
            fs::read_link(fixture.install_root.join("current")).unwrap(),
            PathBuf::from("releases/v1.0.0")
        );
    }

    #[tokio::test]
    async fn rollback_failure_is_explicit() {
        let fixture = fixture(true);
        *fixture.lifecycle.fail_start_always.lock().unwrap() = true;
        let ready_id = "ready-tunnel-rollback-fail";
        let tx = "txn-tunnel-rollback-fail";
        let staged = create_ready(&fixture.manager, "1.1.0", ready_id, "new");
        register_ready(&fixture.manager, &staged, tx, ready_id).await;

        assert!(matches!(
            fixture.manager.apply(tx).await.unwrap_err(),
            StudioError::RollbackFailed { .. }
        ));
        let view = fixture.manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::RollbackFailed);
        assert_eq!(view.rollback_succeeded, Some(false));
    }

    #[tokio::test]
    #[ignore = "explicit real-network force-reinstall smoke against official OpenAI tunnel release"]
    async fn live_official_same_version_force_reinstall_smoke() {
        use crate::update::{HostPlatform, OpenAiTunnelReleaseProvider};

        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let bin = temp.path().join("bin");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&bin).unwrap();

        let catalog = ComponentCatalog::new(HostRuntimeRoots::new(bin, runtime.clone()).unwrap());
        let provider = Arc::new(OpenAiTunnelReleaseProvider::new(catalog.clone()).unwrap());
        let policy = catalog.component(ComponentId::Tunnel).unwrap().clone();
        let release = provider.latest_release(&policy).await.unwrap();

        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let staged = stager
            .stage(
                provider.as_ref(),
                &release,
                HostPlatform::detect().unwrap().platform(),
            )
            .await
            .unwrap();

        let install_root = catalog.install_path(ComponentId::Tunnel).unwrap();
        fs::create_dir_all(install_root.join("releases")).unwrap();
        let active = install_root
            .join("releases")
            .join(format!("v{}", release.version));
        copy_tree(&staged.package_root, &active).unwrap();
        fs::write(
            install_root.join("config.yaml"),
            "credentials: preserve-this\n",
        )
        .unwrap();
        fs::create_dir_all(install_root.join("credentials")).unwrap();
        fs::write(install_root.join("credentials/token"), "preserve-token").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            PathBuf::from(format!("releases/v{}", release.version)),
            install_root.join("current"),
        )
        .unwrap();

        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                temp.path().join("missing-source"),
                HostPlatform::detect().unwrap().platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let lifecycle = Arc::new(FakeLifecycle::new(TunnelState::Stopped));
        let manager = TunnelUpdateManager::with_dependencies(
            catalog,
            stager,
            provider,
            inventory,
            lifecycle.clone(),
            EventHub::default(),
        )
        .unwrap();

        let ready_id = manager.stager.staged_id(&staged).unwrap();
        let tx = "txn-tunnel-live-force";
        register_ready(&manager, &staged, tx, &ready_id).await;
        let result = manager.apply(tx).await.unwrap();

        assert_eq!(result.phase, McpUpdatePhase::Completed);
        assert_eq!(result.source_version, Some(release.version.clone()));
        assert_eq!(result.target_version, release.version);
        assert_eq!(lifecycle.state().await.unwrap(), TunnelState::Stopped);
        assert_eq!(*lifecycle.stop_count.lock().unwrap(), 0);
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 0);
        assert_eq!(
            fs::read_to_string(install_root.join("config.yaml")).unwrap(),
            "credentials: preserve-this\n"
        );
        assert_eq!(
            fs::read_to_string(install_root.join("credentials/token")).unwrap(),
            "preserve-token"
        );
    }

    #[test]
    fn legacy_scratch_without_journal_is_preserved_and_blocked() {
        let fixture = fixture(false);
        let active = fixture.install_root.join("releases/v1.0.0");
        let rollback = fixture
            .install_root
            .join("releases/.mcp-studio-tunnel-rollback-txn-tunnel-crash");
        fs::rename(&active, &rollback).unwrap();
        assert!(matches!(
            recover_interrupted_tunnel_files(&fixture.manager.catalog),
            Err(StudioError::Conflict(_))
        ));
        assert!(!active.exists());
        assert!(rollback.is_dir());
    }

    #[tokio::test]
    async fn journaled_same_version_crash_recovers_after_manager_reconstruction() {
        let fixture = fixture(false);
        let tx = "txn-tunnel-reconstruct";
        let staged = create_ready(
            &fixture.manager,
            "1.0.0",
            "ready-tunnel-reconstruct",
            "replacement",
        );
        register_ready(&fixture.manager, &staged, tx, "ready-tunnel-reconstruct").await;
        seed_recovery_journal(&fixture.manager, tx, &staged.version, false);

        let source = fixture.install_root.join("releases/v1.0.0");
        let original = digest(&source.join("tunnel-client-runtime-cloudflared"));
        let rollback = fixture
            .install_root
            .join("releases")
            .join(format!("{ROLLBACK_PREFIX}{tx}"));
        fs::rename(&source, &rollback).unwrap();
        let candidate = candidate_path(fixture.install_root.clone(), tx).unwrap();
        fs::rename(candidate, &source).unwrap();
        update_recovery_journal(
            &fixture.manager.catalog,
            tx,
            TunnelRecoveryPhase::HealthVerifying,
            Some("same_version_target_activated"),
            None,
        )
        .unwrap();

        let catalog = fixture.manager.catalog.clone();
        let lifecycle = Arc::new(FakeLifecycle::new(TunnelState::Stopped));
        let recreated = TunnelUpdateManager::with_dependencies(
            catalog.clone(),
            ArtifactStager::new(catalog.clone()).unwrap(),
            Arc::new(NoopProvider),
            fixture.manager.inventory.clone(),
            lifecycle.clone(),
            EventHub::default(),
        )
        .unwrap();
        recreated.recover_startup().await.unwrap();

        assert_eq!(
            digest(&source.join("tunnel-client-runtime-cloudflared")),
            original
        );
        assert_eq!(lifecycle.state().await.unwrap(), TunnelState::Stopped);
        assert!(!journal_path(&catalog, tx).unwrap().exists());
        assert!(!rollback.exists());
    }

    #[test]
    fn corrupt_or_unsafe_recovery_journal_fails_closed() {
        let fixture = fixture(false);
        let root = ensure_recovery_root(&fixture.manager.catalog).unwrap();
        fs::write(root.join("txn-corrupt.json"), b"{not-json").unwrap();
        assert!(recover_interrupted_tunnel_files(&fixture.manager.catalog).is_err());
        fs::remove_file(root.join("txn-corrupt.json")).unwrap();

        fs::remove_dir(&root).unwrap();
        let outside = fixture._temp.path().join("outside-recovery");
        fs::create_dir(&outside).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &root).unwrap();
        assert!(recover_interrupted_tunnel_files(&fixture.manager.catalog).is_err());
    }

    #[tokio::test]
    async fn multiple_recovery_journals_fail_closed_without_deleting_material() {
        let fixture = fixture(false);
        let a = create_ready(&fixture.manager, "1.0.0", "ready-recovery-a", "a");
        let b = create_ready(&fixture.manager, "1.0.0", "ready-recovery-b", "b");
        register_ready(
            &fixture.manager,
            &a,
            "txn-tunnel-recovery-a",
            "ready-recovery-a",
        )
        .await;
        register_ready(
            &fixture.manager,
            &b,
            "txn-tunnel-recovery-b",
            "ready-recovery-b",
        )
        .await;
        seed_recovery_journal(&fixture.manager, "txn-tunnel-recovery-a", &a.version, false);
        seed_recovery_journal(&fixture.manager, "txn-tunnel-recovery-b", &b.version, false);
        assert!(matches!(
            recover_interrupted_tunnel_files(&fixture.manager.catalog),
            Err(StudioError::Conflict(_))
        ));
        assert!(
            candidate_path(fixture.install_root.clone(), "txn-tunnel-recovery-a")
                .unwrap()
                .exists()
        );
        assert!(
            candidate_path(fixture.install_root.clone(), "txn-tunnel-recovery-b")
                .unwrap()
                .exists()
        );
    }

    #[tokio::test]
    async fn startup_recovery_restores_previously_running_owner_idempotently() {
        let fixture = fixture(false);
        let tx = "txn-tunnel-recovery-running";
        let staged = create_ready(
            &fixture.manager,
            "1.0.0",
            "ready-recovery-running",
            "replacement",
        );
        register_ready(&fixture.manager, &staged, tx, "ready-recovery-running").await;
        seed_recovery_journal(&fixture.manager, tx, &staged.version, true);

        let source = fixture.install_root.join("releases/v1.0.0");
        let rollback = fixture
            .install_root
            .join("releases")
            .join(format!("{ROLLBACK_PREFIX}{tx}"));
        fs::rename(&source, &rollback).unwrap();
        let candidate = candidate_path(fixture.install_root.clone(), tx).unwrap();
        fs::rename(candidate, &source).unwrap();
        update_recovery_journal(
            &fixture.manager.catalog,
            tx,
            TunnelRecoveryPhase::HealthVerifying,
            Some("same_version_target_activated"),
            None,
        )
        .unwrap();

        let catalog = fixture.manager.catalog.clone();
        let lifecycle = Arc::new(FakeLifecycle::new(TunnelState::Stopped));
        let recreated = TunnelUpdateManager::with_dependencies(
            catalog.clone(),
            ArtifactStager::new(catalog.clone()).unwrap(),
            Arc::new(NoopProvider),
            fixture.manager.inventory.clone(),
            lifecycle.clone(),
            EventHub::default(),
        )
        .unwrap();
        recreated.recover_startup().await.unwrap();
        assert_eq!(lifecycle.state().await.unwrap(), TunnelState::Running);
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 1);
        recreated.recover_startup().await.unwrap();
        assert_eq!(*lifecycle.start_count.lock().unwrap(), 1);
    }

    struct FailCurrentSyncOnce {
        failed: std::sync::atomic::AtomicBool,
    }

    impl TunnelFsOps for FailCurrentSyncOnce {
        fn sync_dir(&self, path: &Path) -> StudioResult<()> {
            if path.file_name().and_then(|value| value.to_str()) == Some("tunnel-client")
                && !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(std::io::Error::other(
                    "audit injected fsync failure after current rename",
                )
                .into());
            }
            sync_dir(path)
        }
    }

    #[tokio::test]
    async fn audit_tunnel_running_binary_must_match_activated_target() {
        let mut fixture = fixture(false);
        let old = fixture
            .install_root
            .join("releases/v1.0.0/tunnel-client-runtime-cloudflared");
        fs::write(&old,"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '1.0.0 runtime'; exit 0; fi\necho audit-running-old-1.0.0\nexec /bin/sleep 60\n").unwrap();
        let config = crate::tunnel::TunnelConfig {
            runtime: old,
            working_dir: fixture.install_root.clone(),
            config_file: fixture.install_root.join("config.yaml"),
            ..Default::default()
        };
        let supervisor = Arc::new(TunnelSupervisor::new(
            config,
            128,
            Duration::from_secs(1),
            fixture._temp.path().to_path_buf(),
            EventHub::default(),
        ));
        supervisor.start().await.unwrap();
        fixture.manager.lifecycle = Arc::new(SupervisorTunnelLifecycle {
            supervisor: supervisor.clone(),
        });
        let staged = create_ready(&fixture.manager, "1.1.0", "ready-audit-binding", "new");
        register_ready(
            &fixture.manager,
            &staged,
            "txn-tunnel-audit-binding",
            "ready-audit-binding",
        )
        .await;
        let result = fixture.manager.apply("txn-tunnel-audit-binding").await;
        let logs = supervisor.logs().await;
        supervisor.shutdown().await;
        assert!(
            logs.iter()
                .any(|entry| entry.message.contains("audit-running-old-1.0.0"))
        );
        assert!(
            !matches!(result,Ok(ref view) if view.phase==McpUpdatePhase::Completed),
            "real TunnelSupervisor restarted configured 1.0.0, but updater completed target 1.1.0"
        );
    }

    #[tokio::test]
    async fn audit_tunnel_fsync_failure_must_restore_current() {
        let mut fixture = fixture(false);
        fixture.manager.fs_ops = Arc::new(FailCurrentSyncOnce {
            failed: std::sync::atomic::AtomicBool::new(false),
        });
        let staged = create_ready(&fixture.manager, "1.1.0", "ready-audit-fsync", "new");
        register_ready(
            &fixture.manager,
            &staged,
            "txn-tunnel-audit-fsync",
            "ready-audit-fsync",
        )
        .await;
        assert!(
            fixture
                .manager
                .apply("txn-tunnel-audit-fsync")
                .await
                .is_err()
        );
        let target = fs::read_link(fixture.install_root.join("current")).unwrap();
        assert!(
            fixture.install_root.join(&target).is_dir(),
            "post-rename fsync failure left dangling current {:?}; target was deleted",
            target
        );
    }

    #[tokio::test]
    async fn audit_tunnel_restart_must_recover_unverified_same_version_swap() {
        let fixture = fixture(false);
        let source = fixture.install_root.join("releases/v1.0.0");
        let old = digest(&source.join("tunnel-client-runtime-cloudflared"));
        let tx = "txn-tunnel-audit-crash";
        let staged = create_ready(
            &fixture.manager,
            "1.0.0",
            "ready-audit-crash",
            "unverified-replacement",
        );
        register_ready(&fixture.manager, &staged, tx, "ready-audit-crash").await;
        seed_recovery_journal(&fixture.manager, tx, &staged.version, false);
        let backup = fixture
            .install_root
            .join("releases")
            .join(format!("{ROLLBACK_PREFIX}{tx}"));
        fs::rename(&source, &backup).unwrap();
        let candidate = candidate_path(fixture.install_root.clone(), tx).unwrap();
        fs::rename(candidate, &source).unwrap();
        update_recovery_journal(
            &fixture.manager.catalog,
            tx,
            TunnelRecoveryPhase::HealthVerifying,
            Some("same_version_target_activated"),
            None,
        )
        .unwrap();
        fixture.manager.transactions.lock().await.clear();
        recover_interrupted_tunnel_files(&fixture.manager.catalog).unwrap();
        assert_eq!(
            digest(&source.join("tunnel-client-runtime-cloudflared")),
            old,
            "startup kept promoted replacement with no health proof and no recoverable transaction"
        );
    }
}
