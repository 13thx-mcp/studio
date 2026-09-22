use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{process::Command, sync::Mutex, time::timeout};

use crate::{
    config::{AutomationPolicy, UpdateAutomationConfig},
    error::{StudioError, StudioResult},
    operation::OperationService,
    realtime::{EventHub, StudioEvent},
    storage::{ActorKind, HistoryAction, HistoryHandle, OperationOutcome, SubjectKind},
    update::{
        ArtifactHistoryIdentity, CheckStatus, ComponentId, FleetUpdateManager,
        GatewayUpdateManager, InventoryService, InventoryView, McpUpdateManager, McpUpdatePhase,
        SelfUpdateManager, TunnelUpdateManager, Version,
    },
};

const SOURCE_STATUS_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_READY_ENTRIES: usize = 256;
const MAX_READY_WALK_ENTRIES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceHygiene {
    RuntimeOnly,
    Clean,
    Dirty,
    Conflict,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCandidate {
    pub component: ComponentId,
    pub target: Version,
    pub source_hygiene: SourceHygiene,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAutomation {
    pub component: ComponentId,
    pub transaction_id: String,
    pub staging_id: String,
    pub target: Version,
    pub staged_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePolicyAction {
    ObserveOnly,
    PrepareOnly,
}

fn policy_action(policy: AutomationPolicy) -> Option<UpdatePolicyAction> {
    match policy {
        AutomationPolicy::Manual => None,
        AutomationPolicy::NotifyOnly => Some(UpdatePolicyAction::ObserveOnly),
        AutomationPolicy::AutoPrepare | AutomationPolicy::AutoUpdateSafe => {
            Some(UpdatePolicyAction::PrepareOnly)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateAutomationRun {
    pub checked_components: usize,
    pub candidates: Vec<UpdateCandidate>,
    pub prepared: Vec<PreparedAutomation>,
    pub failures: BTreeMap<ComponentId, String>,
}

#[derive(Clone)]
pub struct UpdateAutomationService {
    config: UpdateAutomationConfig,
    inventory: Arc<InventoryService>,
    updates: Arc<McpUpdateManager>,
    gateway_updates: Arc<GatewayUpdateManager>,
    fleet_updates: Arc<FleetUpdateManager>,
    self_updates: Arc<SelfUpdateManager>,
    tunnel_updates: Arc<TunnelUpdateManager>,
    operations: OperationService,
    history: HistoryHandle,
    events: EventHub,
    runtime_operations: Arc<crate::update::RuntimeOperationCoordinator>,
    staging: ReadyStagingPolicy,
    check_lock: Arc<Mutex<()>>,
}

impl UpdateAutomationService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: UpdateAutomationConfig,
        runtime_root: PathBuf,
        inventory: Arc<InventoryService>,
        updates: Arc<McpUpdateManager>,
        gateway_updates: Arc<GatewayUpdateManager>,
        fleet_updates: Arc<FleetUpdateManager>,
        self_updates: Arc<SelfUpdateManager>,
        tunnel_updates: Arc<TunnelUpdateManager>,
        operations: OperationService,
        history: HistoryHandle,
        events: EventHub,
        runtime_operations: Arc<crate::update::RuntimeOperationCoordinator>,
    ) -> Self {
        Self {
            staging: ReadyStagingPolicy::new(
                runtime_root,
                config.prepared_ttl_seconds,
                config.max_staging_bytes,
            ),
            config,
            inventory,
            updates,
            gateway_updates,
            fleet_updates,
            self_updates,
            tunnel_updates,
            operations,
            history,
            events,
            runtime_operations,
            check_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn run(
        &self,
        current_prepared: &BTreeSet<String>,
    ) -> StudioResult<UpdateAutomationRun> {
        let Some(action) = policy_action(self.config.policy) else {
            return Ok(UpdateAutomationRun::default());
        };
        let _batch = self.check_lock.lock().await;
        let operation = self
            .operations
            .admit(
                SubjectKind::Component,
                HistoryAction::UpdateCheck,
                ActorKind::SystemAutomation,
            )
            .await?;
        let inventory = self.inventory.check().await;
        for view in &inventory {
            self.history
                .observe_update_check(view, operation.operation_id());
        }
        self.events.publish(StudioEvent::UpdatesChanged);
        self.operations
            .finish(&operation, OperationOutcome::Succeeded, None)
            .await?;

        let mut run = UpdateAutomationRun {
            checked_components: inventory.len(),
            ..UpdateAutomationRun::default()
        };
        for view in &inventory {
            if let Some(candidate) = self.candidate(view).await? {
                run.candidates.push(candidate);
            }
        }
        run.candidates.sort_by_key(|candidate| candidate.component);

        if action == UpdatePolicyAction::ObserveOnly {
            return Ok(run);
        }

        // M8.3 deliberately has no activation action. AutoPrepare and
        // AutoUpdateSafe both stop at verified manager.prepare()/Staged.
        self.staging.cleanup_expired(current_prepared)?;
        self.staging.ensure_within_budget()?;
        if !self.runtime_operations.snapshot()?.is_idle() {
            return Ok(run);
        }

        for candidate in run.candidates.clone() {
            match self.prepare_candidate(&candidate).await {
                Ok(prepared) => run.prepared.push(prepared),
                Err(error) => {
                    run.failures
                        .insert(candidate.component, safe_failure_code(&error));
                }
            }
        }
        Ok(run)
    }

    async fn candidate(&self, view: &InventoryView) -> StudioResult<Option<UpdateCandidate>> {
        let Some(target) = candidate_target(view) else {
            return Ok(None);
        };
        Ok(Some(UpdateCandidate {
            component: view.component,
            target,
            source_hygiene: self.source_hygiene(view).await,
        }))
    }

    async fn source_hygiene(&self, view: &InventoryView) -> SourceHygiene {
        if !view.source_present {
            return SourceHygiene::RuntimeOnly;
        }
        let Ok(source) = self.inventory.source_path_for(view.component) else {
            return SourceHygiene::Unavailable;
        };
        let result = timeout(
            SOURCE_STATUS_TIMEOUT,
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(["status", "--porcelain=v1", "--untracked-files=all"])
                .output(),
        )
        .await;
        let Ok(Ok(output)) = result else {
            return SourceHygiene::Unavailable;
        };
        if !output.status.success() {
            return SourceHygiene::Unavailable;
        }
        classify_git_status(&String::from_utf8_lossy(&output.stdout))
    }

    async fn prepare_candidate(
        &self,
        candidate: &UpdateCandidate,
    ) -> StudioResult<PreparedAutomation> {
        let operation = self
            .operations
            .admit(
                SubjectKind::Component,
                HistoryAction::UpdatePrepare,
                ActorKind::SystemAutomation,
            )
            .await?;
        let result = match candidate.component {
            ComponentId::Gateway => self.gateway_updates.prepare(candidate.target.clone()).await,
            ComponentId::Fleet => self.fleet_updates.prepare(candidate.target.clone()).await,
            ComponentId::Studio => self.self_updates.prepare(candidate.target.clone()).await,
            ComponentId::Tunnel => self.tunnel_updates.prepare(candidate.target.clone()).await,
            _ => {
                self.updates
                    .prepare(candidate.component, candidate.target.clone())
                    .await
            }
        };

        match result {
            Ok(view) => {
                if view.phase != McpUpdatePhase::Staged {
                    let _ = self
                        .operations
                        .finish(&operation, OperationOutcome::Failed, None)
                        .await;
                    return Err(StudioError::Automation(
                        "automatic prepare did not reach staged phase".into(),
                    ));
                }
                let artifact = self
                    .artifact_identity(candidate.component, &view.transaction_id)
                    .await
                    .ok_or_else(|| {
                        StudioError::Automation(
                            "automatic prepare is missing staged artifact identity".into(),
                        )
                    })?;
                let staging_id = self
                    .prepared_staging_id(candidate.component, &view.transaction_id)
                    .await?
                    .ok_or_else(|| {
                        StudioError::Automation(
                            "automatic prepare is missing staging ownership identity".into(),
                        )
                    })?;
                self.history.observe_update_transaction_with_artifact(
                    &view,
                    Some(operation.operation_id()),
                    false,
                    Some(artifact.clone()),
                );
                self.operations
                    .finish(&operation, OperationOutcome::Succeeded, None)
                    .await?;
                self.events.publish(StudioEvent::UpdatesChanged);
                Ok(PreparedAutomation {
                    component: candidate.component,
                    transaction_id: view.transaction_id,
                    staging_id,
                    target: candidate.target.clone(),
                    staged_fingerprint: artifact.archive_sha256,
                })
            }
            Err(error) => {
                let _ = self
                    .operations
                    .finish(&operation, OperationOutcome::Failed, None)
                    .await;
                Err(error)
            }
        }
    }

    async fn prepared_staging_id(
        &self,
        component: ComponentId,
        transaction_id: &str,
    ) -> StudioResult<Option<String>> {
        match component {
            ComponentId::Gateway => Ok(self
                .gateway_updates
                .prepared_staging_id(transaction_id)
                .await),
            ComponentId::Fleet => Ok(self.fleet_updates.prepared_staging_id(transaction_id).await),
            ComponentId::Studio => self.self_updates.prepared_staging_id(transaction_id),
            ComponentId::Tunnel => Ok(self
                .tunnel_updates
                .prepared_staging_id(transaction_id)
                .await),
            _ => Ok(self
                .updates
                .prepared_staging_id(component, transaction_id)
                .await),
        }
    }

    async fn artifact_identity(
        &self,
        component: ComponentId,
        transaction_id: &str,
    ) -> Option<ArtifactHistoryIdentity> {
        match component {
            ComponentId::Gateway => {
                self.gateway_updates
                    .artifact_history_identity(transaction_id)
                    .await
            }
            ComponentId::Fleet => {
                self.fleet_updates
                    .artifact_history_identity(transaction_id)
                    .await
            }
            ComponentId::Studio => self
                .self_updates
                .artifact_history_identity(transaction_id)
                .ok()
                .flatten(),
            ComponentId::Tunnel => {
                self.tunnel_updates
                    .artifact_history_identity(transaction_id)
                    .await
            }
            _ => {
                self.updates
                    .artifact_history_identity(component, transaction_id)
                    .await
            }
        }
    }
}

fn candidate_target(view: &InventoryView) -> Option<Version> {
    let installed = view.installed_version.as_ref()?;
    if view.desired_pinned {
        return view
            .desired_version
            .as_ref()
            .filter(|desired| desired.is_newer_than(installed))
            .cloned();
    }
    if view.last_check.status != CheckStatus::Ok {
        return None;
    }
    view.latest_version
        .as_ref()
        .filter(|latest| latest.is_newer_than(installed))
        .cloned()
}

fn classify_git_status(text: &str) -> SourceHygiene {
    if text.is_empty() {
        return SourceHygiene::Clean;
    }
    if text.lines().any(|line| {
        let status = line.as_bytes().get(..2).unwrap_or_default();
        status.contains(&b'U') || status == b"AA" || status == b"DD"
    }) {
        SourceHygiene::Conflict
    } else {
        SourceHygiene::Dirty
    }
}

fn safe_failure_code(error: &StudioError) -> String {
    match error {
        StudioError::Conflict(_) => "conflict",
        StudioError::ReleaseProviderUnreachable { .. } => "provider_unavailable",
        StudioError::ReleaseVersionNotFound { .. } => "version_not_found",
        StudioError::StagingFailure(_) => "staging_failed",
        StudioError::ArtifactDownloadFailed { .. } => "download_failed",
        _ => "prepare_failed",
    }
    .into()
}

#[derive(Debug, Clone)]
struct ReadyStagingPolicy {
    root: PathBuf,
    ttl_seconds: u64,
    max_bytes: u64,
}

impl ReadyStagingPolicy {
    fn new(runtime_root: PathBuf, ttl_seconds: u64, max_bytes: u64) -> Self {
        Self {
            root: runtime_root.join(".mcp-studio-staging"),
            ttl_seconds,
            max_bytes,
        }
    }

    fn cleanup_expired(&self, current_prepared: &BTreeSet<String>) -> StudioResult<()> {
        if !self.root.exists() {
            return Ok(());
        }
        let canonical_root = fs::canonicalize(&self.root)?;
        let now = SystemTime::now();
        let mut entries = ready_entries(&self.root)?;
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| StudioError::StagingFailure("invalid ready staging name".into()))?;
            if current_prepared.contains(name) {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
            let age = now.duration_since(modified).unwrap_or_default().as_secs();
            if age < self.ttl_seconds {
                continue;
            }
            let canonical = fs::canonicalize(&path)?;
            if canonical.parent() != Some(canonical_root.as_path()) {
                return Err(StudioError::StagingFailure(
                    "ready staging cleanup path escaped root".into(),
                ));
            }
            validate_ready_tree(&canonical)?;
            fs::remove_dir_all(&canonical)?;
        }
        Ok(())
    }

    fn ensure_within_budget(&self) -> StudioResult<u64> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut total = 0_u64;
        for path in ready_entries(&self.root)? {
            total = total.saturating_add(ready_tree_bytes(&path)?);
            if total > self.max_bytes {
                return Err(StudioError::StagingFailure(
                    "ready staging aggregate exceeds automation budget".into(),
                ));
            }
        }
        Ok(total)
    }
}

fn ready_entries(root: &Path) -> StudioResult<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::StagingFailure(
            "automation staging root is unsafe".into(),
        ));
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("ready-") {
            continue;
        }
        if entries.len() >= MAX_READY_ENTRIES {
            return Err(StudioError::StagingFailure(
                "too many retained ready staging entries".into(),
            ));
        }
        let metadata = fs::symlink_metadata(item.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StudioError::StagingFailure(
                "ready staging entry is unsafe".into(),
            ));
        }
        entries.push(item.path());
    }
    Ok(entries)
}

fn validate_ready_tree(root: &Path) -> StudioResult<()> {
    let _ = ready_tree_bytes(root)?;
    Ok(())
}

fn ready_tree_bytes(root: &Path) -> StudioResult<u64> {
    let mut stack = vec![root.to_owned()];
    let mut visited = 0_usize;
    let mut total = 0_u64;
    while let Some(path) = stack.pop() {
        visited = visited.saturating_add(1);
        if visited > MAX_READY_WALK_ENTRIES {
            return Err(StudioError::StagingFailure(
                "ready staging tree exceeds entry limit".into(),
            ));
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::StagingFailure(
                "ready staging tree contains symlink".into(),
            ));
        }
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            for item in fs::read_dir(&path)? {
                stack.push(item?.path());
            }
        } else {
            return Err(StudioError::StagingFailure(
                "ready staging tree contains special file".into(),
            ));
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{
        Architecture, ComponentClass, HostMode, InventoryHealth, OperatingSystem, Platform,
        ReleaseCheckView, ReleaseProviderId,
    };

    fn view(
        desired_pinned: bool,
        installed: &str,
        desired: Option<&str>,
        latest: Option<&str>,
        check_status: CheckStatus,
    ) -> InventoryView {
        InventoryView {
            component: ComponentId::Git,
            display_name: "Git",
            class: ComponentClass::McpBinary,
            provider: ReleaseProviderId::ThirteenthXGitHub,
            platform: Platform {
                os: OperatingSystem::Darwin,
                arch: Architecture::Arm64,
            },
            host_mode: HostMode::RuntimeOnly,
            source_present: false,
            installed_version: Some(Version::parse(installed).unwrap()),
            running_version: None,
            desired_version: desired.map(|value| Version::parse(value).unwrap()),
            desired_pinned,
            latest_version: latest.map(|value| Version::parse(value).unwrap()),
            update_available: latest.is_some_and(|value| {
                Version::parse(value)
                    .unwrap()
                    .is_newer_than(&Version::parse(installed).unwrap())
            }),
            installation_health: InventoryHealth::Healthy,
            drift: crate::update::DriftState::Current,
            last_check: ReleaseCheckView {
                checked_at_ms: Some(1),
                status: check_status,
                error: None,
            },
        }
    }

    #[test]
    fn update_policy_contract_never_exposes_activation_in_m8_3() {
        assert_eq!(policy_action(AutomationPolicy::Manual), None);
        assert_eq!(
            policy_action(AutomationPolicy::NotifyOnly),
            Some(UpdatePolicyAction::ObserveOnly)
        );
        assert_eq!(
            policy_action(AutomationPolicy::AutoPrepare),
            Some(UpdatePolicyAction::PrepareOnly)
        );
        assert_eq!(
            policy_action(AutomationPolicy::AutoUpdateSafe),
            Some(UpdatePolicyAction::PrepareOnly)
        );
    }

    #[test]
    fn desired_pin_precedes_latest_and_never_downgrades_or_reinstalls_same_version() {
        let pinned = view(true, "1.0.0", Some("1.1.0"), Some("2.0.0"), CheckStatus::Ok);
        assert_eq!(
            candidate_target(&pinned).unwrap(),
            Version::parse("1.1.0").unwrap()
        );

        let downgrade = view(true, "1.0.0", Some("0.9.0"), Some("2.0.0"), CheckStatus::Ok);
        assert!(candidate_target(&downgrade).is_none());

        let same = view(true, "1.0.0", Some("1.0.0"), Some("2.0.0"), CheckStatus::Ok);
        assert!(candidate_target(&same).is_none());

        let latest = view(
            false,
            "1.0.0",
            Some("1.0.0"),
            Some("2.0.0"),
            CheckStatus::Ok,
        );
        assert_eq!(
            candidate_target(&latest).unwrap(),
            Version::parse("2.0.0").unwrap()
        );
    }

    #[test]
    fn stale_cached_latest_is_not_a_fresh_automatic_target() {
        let stale = view(
            false,
            "1.0.0",
            Some("1.0.0"),
            Some("2.0.0"),
            CheckStatus::Error,
        );
        assert!(candidate_target(&stale).is_none());
    }

    #[test]
    fn source_hygiene_classifies_clean_dirty_conflict_and_runtime_only() {
        assert_eq!(classify_git_status(""), SourceHygiene::Clean);
        assert_eq!(
            classify_git_status(
                " M src/lib.rs
"
            ),
            SourceHygiene::Dirty
        );
        assert_eq!(
            classify_git_status(
                "?? new-file
"
            ),
            SourceHygiene::Dirty
        );
        assert_eq!(
            classify_git_status(
                "UU src/lib.rs
"
            ),
            SourceHygiene::Conflict
        );
        assert_eq!(
            classify_git_status(
                "AA src/lib.rs
"
            ),
            SourceHygiene::Conflict
        );
        assert_eq!(
            classify_git_status(
                "DD src/lib.rs
"
            ),
            SourceHygiene::Conflict
        );

        let runtime_only = view(
            false,
            "1.0.0",
            Some("1.0.0"),
            Some("2.0.0"),
            CheckStatus::Ok,
        );
        assert!(!runtime_only.source_present);
        assert_eq!(runtime_only.host_mode, HostMode::RuntimeOnly);
    }

    #[test]
    fn expired_unprotected_ready_staging_is_removed_but_current_prepared_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join(".mcp-studio-staging");
        fs::create_dir(&staging).unwrap();
        let expired = staging.join("ready-expired");
        let protected = staging.join("ready-protected");
        fs::create_dir(&expired).unwrap();
        fs::create_dir(&protected).unwrap();
        fs::write(expired.join("archive"), b"old").unwrap();
        fs::write(protected.join("archive"), b"keep").unwrap();

        let policy = ReadyStagingPolicy::new(root.path().to_owned(), 0, 1024);
        policy
            .cleanup_expired(&BTreeSet::from(["ready-protected".to_owned()]))
            .unwrap();

        assert!(!expired.exists());
        assert!(protected.exists());
    }

    #[test]
    fn staging_budget_fails_closed_before_new_prepare_when_retained_bytes_exceed_limit() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join(".mcp-studio-staging");
        fs::create_dir(&staging).unwrap();
        let ready = staging.join("ready-large");
        fs::create_dir(&ready).unwrap();
        fs::write(ready.join("archive"), vec![0_u8; 33]).unwrap();
        let policy = ReadyStagingPolicy::new(root.path().to_owned(), 60, 32);
        assert!(policy.ensure_within_budget().is_err());
    }

    #[test]
    fn staging_budget_counts_only_safe_ready_directories_and_rejects_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join(".mcp-studio-staging");
        fs::create_dir(&staging).unwrap();
        let ready = staging.join("ready-one");
        fs::create_dir(&ready).unwrap();
        fs::write(ready.join("archive"), vec![0_u8; 16]).unwrap();
        let policy = ReadyStagingPolicy::new(root.path().to_owned(), 60, 32);
        assert!(policy.ensure_within_budget().is_ok());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let unsafe_ready = staging.join("ready-unsafe");
            symlink(&ready, &unsafe_ready).unwrap();
            assert!(policy.ensure_within_budget().is_err());
        }
    }
}
