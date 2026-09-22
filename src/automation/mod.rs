use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, RwLock};

use crate::{
    config::{AutomationConfig, AutomationPolicy},
    error::{StudioError, StudioResult},
    operation::OperationService,
    update::RuntimeOperationCoordinator,
};

const STATE_SCHEMA_VERSION: u32 = 1;
const MAX_STATE_BYTES: u64 = 256 * 1024;
const CLOCK_REGRESSION_TOLERANCE_MS: u64 = 5 * 60 * 1000;
const DEFAULT_DEFERRAL_INITIAL_MS: u64 = 30 * 1000;
const DEFAULT_DEFERRAL_MAX_MS: u64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleClass {
    Health,
    UpdateCheck,
    ReconciliationCheck,
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleState {
    pub next_due_ms: u64,
    pub interval_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitPhase {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircuitState {
    pub phase: CircuitPhase,
    pub consecutive_failures: u32,
    pub retry_not_before_ms: Option<u64>,
}

impl Default for CircuitState {
    fn default() -> Self {
        Self {
            phase: CircuitPhase::Closed,
            consecutive_failures: 0,
            retry_not_before_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DeferralState {
    pub consecutive_deferrals: u32,
    pub retry_not_before_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferralPolicy {
    pub initial_ms: u64,
    pub max_ms: u64,
}

impl DeferralState {
    pub fn record(&mut self, now_ms: u64, policy: DeferralPolicy) -> u64 {
        self.consecutive_deferrals = self.consecutive_deferrals.saturating_add(1);
        let exponent = self.consecutive_deferrals.saturating_sub(1).min(31);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        let delay = policy
            .initial_ms
            .saturating_mul(multiplier)
            .min(policy.max_ms);
        let retry = now_ms.saturating_add(delay);
        self.retry_not_before_ms = Some(retry);
        retry
    }

    pub fn ready(&self, now_ms: u64) -> bool {
        self.retry_not_before_ms.is_none_or(|retry| now_ms >= retry)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Success,
    Failure,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeCode {
    Completed,
    DomainFailure,
    CoordinatorBusy,
    MaintenanceClosed,
    SafetyHold,
    DirtySource,
    Transitional,
    PolicyDisabled,
    CapabilityUnavailable,
    ClockAnomaly,
    StateUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationOutcome {
    pub kind: OutcomeKind,
    pub code: OutcomeCode,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedReference {
    pub component: String,
    pub target_version: String,
    pub transaction_id: String,
    pub prepared_at_ms: u64,
    pub process_instance: String,
    pub policy_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationState {
    pub schema_version: u32,
    pub last_persisted_wall_ms: u64,
    pub policy_fingerprint: Option<String>,
    #[serde(default)]
    pub schedules: BTreeMap<ScheduleClass, ScheduleState>,
    #[serde(default)]
    pub circuits: BTreeMap<String, CircuitState>,
    #[serde(default)]
    pub deferrals: BTreeMap<String, DeferralState>,
    #[serde(default)]
    pub prepared: BTreeMap<String, PreparedReference>,
    pub last_outcome: Option<AutomationOutcome>,
}

impl AutomationState {
    fn new(now_ms: u64, policy_fingerprint: Option<String>) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            last_persisted_wall_ms: now_ms,
            policy_fingerprint,
            schedules: BTreeMap::new(),
            circuits: BTreeMap::new(),
            deferrals: BTreeMap::new(),
            prepared: BTreeMap::new(),
            last_outcome: None,
        }
    }

    fn validate(&self) -> StudioResult<()> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StudioError::Automation(format!(
                "unsupported automation state schema {}",
                self.schema_version
            )));
        }
        if self.prepared.len() > 32
            || self.circuits.len() > 64
            || self.deferrals.len() > 64
            || self.schedules.len() > 8
        {
            return Err(StudioError::Automation(
                "automation state exceeds bounded entry limits".into(),
            ));
        }
        Ok(())
    }

    fn clock_regressed(&self, now_ms: u64) -> bool {
        now_ms.saturating_add(CLOCK_REGRESSION_TOLERANCE_MS) < self.last_persisted_wall_ms
    }
}

#[derive(Debug, Clone)]
pub struct AutomationStateStore {
    runtime_root: PathBuf,
    state_path: PathBuf,
}

impl AutomationStateStore {
    pub fn new(runtime_root: impl Into<PathBuf>) -> Self {
        let runtime_root = runtime_root.into();
        let state_path = runtime_root.join("studio/data/automation/state.json");
        Self {
            runtime_root,
            state_path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.state_path
    }

    pub fn load(&self) -> StudioResult<Option<AutomationState>> {
        let parent = self
            .state_path
            .parent()
            .ok_or_else(|| StudioError::Automation("automation state path has no parent".into()))?;
        ensure_private_state_dir(&self.runtime_root, parent)?;
        let metadata = match fs::symlink_metadata(&self.state_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::Automation(
                "automation state path is not a regular file".into(),
            ));
        }
        if metadata.len() > MAX_STATE_BYTES {
            return Err(StudioError::Automation(
                "automation state exceeds 256 KiB limit".into(),
            ));
        }
        let bytes = fs::read(&self.state_path)?;
        let state: AutomationState = serde_json::from_slice(&bytes).map_err(|error| {
            StudioError::Automation(format!("invalid automation state JSON: {error}"))
        })?;
        state.validate()?;
        Ok(Some(state))
    }

    pub fn persist(&self, state: &AutomationState) -> StudioResult<()> {
        state.validate()?;
        let bytes = serde_json::to_vec_pretty(state)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_STATE_BYTES {
            return Err(StudioError::Automation(
                "automation state exceeds 256 KiB limit".into(),
            ));
        }

        let parent = self
            .state_path
            .parent()
            .ok_or_else(|| StudioError::Automation("automation state path has no parent".into()))?;
        ensure_private_state_dir(&self.runtime_root, parent)?;

        match fs::symlink_metadata(&self.state_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(StudioError::Automation(
                    "automation state target is not a regular file".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let temp = parent.join(format!(".state.{}.tmp", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        #[cfg(unix)]
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        if let Err(error) = (|| -> std::io::Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.state_path)?;
            sync_directory(parent)?;
            Ok(())
        })() {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(())
    }
}

fn ensure_private_state_dir(runtime_root: &Path, path: &Path) -> StudioResult<()> {
    let root_metadata = fs::symlink_metadata(runtime_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(StudioError::Automation(
            "automation runtime root is not a regular directory".into(),
        ));
    }
    let canonical_root = fs::canonicalize(runtime_root)?;
    let relative = path.strip_prefix(runtime_root).map_err(|_| {
        StudioError::Automation("automation state path escaped runtime root".into())
    })?;

    let mut cursor = runtime_root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(StudioError::Automation(
                "automation state directory contains unsafe path component".into(),
            ));
        };
        cursor.push(name);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(StudioError::Automation(
                    "automation state ancestor is not a regular directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&cursor)?;
                #[cfg(unix)]
                fs::set_permissions(&cursor, fs::Permissions::from_mode(0o700))?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let canonical_path = fs::canonicalize(path)?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(StudioError::Automation(
            "automation state directory escaped runtime root".into(),
        ));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

pub trait AutomationClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl AutomationClock for SystemClock {
    fn now_ms(&self) -> u64 {
        unix_ms()
    }
}

fn unix_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone)]
pub struct ScheduleEngine {
    intervals_ms: BTreeMap<ScheduleClass, u64>,
    startup_due: BTreeMap<ScheduleClass, bool>,
    startup_grace_ms: u64,
}

impl ScheduleEngine {
    pub fn from_config(config: &AutomationConfig) -> Self {
        let mut intervals_ms = BTreeMap::from([
            (
                ScheduleClass::Health,
                config.health_interval_seconds.saturating_mul(1000),
            ),
            (
                ScheduleClass::Cleanup,
                config.cleanup_interval_seconds.saturating_mul(1000),
            ),
        ]);
        let mut startup_due = BTreeMap::from([
            (ScheduleClass::Health, true),
            (ScheduleClass::Cleanup, false),
        ]);

        if config.updates.policy != AutomationPolicy::Manual {
            intervals_ms.insert(
                ScheduleClass::UpdateCheck,
                config.updates.check_interval_seconds.saturating_mul(1000),
            );
            startup_due.insert(ScheduleClass::UpdateCheck, config.updates.check_on_startup);
        }
        if config.reconciliation.policy != crate::config::ReconciliationAutomationPolicy::Manual {
            intervals_ms.insert(
                ScheduleClass::ReconciliationCheck,
                config
                    .reconciliation
                    .check_interval_seconds
                    .saturating_mul(1000),
            );
            startup_due.insert(
                ScheduleClass::ReconciliationCheck,
                config.reconciliation.check_on_startup,
            );
        }

        Self {
            intervals_ms,
            startup_due,
            startup_grace_ms: config.startup_grace_seconds.saturating_mul(1000),
        }
    }

    pub fn reconcile(&self, state: &mut AutomationState, now_ms: u64) {
        state
            .schedules
            .retain(|class, _| self.intervals_ms.contains_key(class));
        for (class, interval_ms) in &self.intervals_ms {
            let initial_delay = if self.startup_due.get(class).copied().unwrap_or(false) {
                self.startup_grace_ms
            } else {
                *interval_ms
            };
            state
                .schedules
                .entry(*class)
                .and_modify(|entry| entry.interval_ms = *interval_ms)
                .or_insert(ScheduleState {
                    next_due_ms: now_ms.saturating_add(initial_delay),
                    interval_ms: *interval_ms,
                });
        }
    }

    pub fn take_due(&self, state: &mut AutomationState, now_ms: u64) -> Vec<ScheduleClass> {
        let mut due = Vec::new();
        for (class, schedule) in &mut state.schedules {
            if now_ms >= schedule.next_due_ms {
                due.push(*class);
                schedule.next_due_ms = now_ms.saturating_add(schedule.interval_ms);
            }
        }
        due
    }

    pub fn next_due_ms(&self, state: &AutomationState) -> Option<u64> {
        state.schedules.values().map(|item| item.next_due_ms).min()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitPolicy {
    pub failure_threshold: u32,
    pub cooldown_ms: u64,
}

impl CircuitState {
    pub fn record_success(&mut self) {
        *self = Self::default();
    }

    pub fn record_failure(&mut self, now_ms: u64, policy: CircuitPolicy) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= policy.failure_threshold {
            self.phase = CircuitPhase::Open;
            self.retry_not_before_ms = Some(now_ms.saturating_add(policy.cooldown_ms));
        }
    }

    pub fn ready_for_probe(&mut self, now_ms: u64) -> bool {
        match self.phase {
            CircuitPhase::Closed => true,
            CircuitPhase::Open => {
                if self
                    .retry_not_before_ms
                    .is_some_and(|deadline| now_ms >= deadline)
                {
                    self.phase = CircuitPhase::HalfOpen;
                    true
                } else {
                    false
                }
            }
            CircuitPhase::HalfOpen => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationBlocker {
    StateUnavailable,
    ClockAnomaly,
}

#[derive(Clone)]
pub struct AutomationController {
    config: AutomationConfig,
    store: AutomationStateStore,
    clock: Arc<dyn AutomationClock>,
    state: Arc<Mutex<AutomationState>>,
    blocker: Arc<RwLock<Option<AutomationBlocker>>>,
    schedule: ScheduleEngine,
    runtime_operations: Arc<RuntimeOperationCoordinator>,
    operations: OperationService,
    persistence_available: Arc<RwLock<bool>>,
    evaluation: Arc<Mutex<()>>,
    run_active: Arc<AtomicBool>,
}

impl AutomationController {
    pub fn new(
        config: AutomationConfig,
        runtime_root: PathBuf,
        policy_fingerprint: Option<String>,
        runtime_operations: Arc<RuntimeOperationCoordinator>,
        operations: OperationService,
    ) -> Self {
        Self::with_clock(
            config,
            runtime_root,
            policy_fingerprint,
            runtime_operations,
            operations,
            Arc::new(SystemClock),
        )
    }

    fn with_clock(
        config: AutomationConfig,
        runtime_root: PathBuf,
        policy_fingerprint: Option<String>,
        runtime_operations: Arc<RuntimeOperationCoordinator>,
        operations: OperationService,
        clock: Arc<dyn AutomationClock>,
    ) -> Self {
        let store = AutomationStateStore::new(runtime_root);
        let now_ms = clock.now_ms();
        let schedule = ScheduleEngine::from_config(&config);
        let mut blocker = None;
        let mut persistence_available = true;
        let mut state = match store.load() {
            Ok(Some(state)) => state,
            Ok(None) => AutomationState::new(now_ms, policy_fingerprint.clone()),
            Err(_) => {
                blocker = Some(AutomationBlocker::StateUnavailable);
                persistence_available = false;
                AutomationState::new(now_ms, policy_fingerprint.clone())
            }
        };

        if state.clock_regressed(now_ms) {
            blocker = Some(AutomationBlocker::ClockAnomaly);
        }
        if state.policy_fingerprint != policy_fingerprint {
            state.prepared.clear();
            state.policy_fingerprint = policy_fingerprint;
        }
        schedule.reconcile(&mut state, now_ms);
        state.last_persisted_wall_ms = state.last_persisted_wall_ms.max(now_ms);

        if persistence_available && store.persist(&state).is_err() {
            blocker = Some(AutomationBlocker::StateUnavailable);
            persistence_available = false;
        }

        Self {
            config,
            store,
            clock,
            state: Arc::new(Mutex::new(state)),
            blocker: Arc::new(RwLock::new(blocker)),
            schedule,
            runtime_operations,
            operations,
            persistence_available: Arc::new(RwLock::new(persistence_available)),
            evaluation: Arc::new(Mutex::new(())),
            run_active: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn blocker(&self) -> Option<AutomationBlocker> {
        self.blocker.read().await.clone()
    }

    pub async fn state(&self) -> AutomationState {
        self.state.lock().await.clone()
    }

    pub fn config(&self) -> &AutomationConfig {
        &self.config
    }

    pub fn operation_service(&self) -> &OperationService {
        &self.operations
    }

    pub async fn run(self: Arc<Self>, shutdown: Arc<Notify>) {
        if self
            .run_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            tracing::warn!("duplicate AutomationController run loop rejected");
            return;
        }
        let _run_guard = AutomationRunGuard(self.run_active.clone());

        loop {
            let now_ms = self.clock.now_ms();
            if self.refresh_clock_blocker(now_ms).await.is_err() {
                *self.blocker.write().await = Some(AutomationBlocker::StateUnavailable);
                *self.persistence_available.write().await = false;
            }

            let sleep_ms = {
                let state = self.state.lock().await;
                self.schedule
                    .next_due_ms(&state)
                    .map(|deadline| deadline.saturating_sub(now_ms).max(1))
                    .unwrap_or(60_000)
                    .min(60_000)
            };

            tokio::select! {
                _ = shutdown.notified() => break,
                _ = tokio::time::sleep(Duration::from_millis(sleep_ms)) => {}
            }

            if *self.persistence_available.read().await && self.tick().await.is_err() {
                *self.blocker.write().await = Some(AutomationBlocker::StateUnavailable);
                *self.persistence_available.write().await = false;
            }
        }
    }

    async fn refresh_clock_blocker(&self, now_ms: u64) -> StudioResult<()> {
        let regressed = self.state.lock().await.clock_regressed(now_ms);
        let mut blocker = self.blocker.write().await;
        match (&*blocker, regressed) {
            (Some(AutomationBlocker::StateUnavailable), _) => {}
            (_, true) => *blocker = Some(AutomationBlocker::ClockAnomaly),
            (Some(AutomationBlocker::ClockAnomaly), false) => *blocker = None,
            _ => {}
        }
        Ok(())
    }

    async fn tick(&self) -> StudioResult<Vec<ScheduleClass>> {
        let _evaluation = self.evaluation.lock().await;
        let now_ms = self.clock.now_ms();
        if self.blocker.read().await.is_some() {
            return Ok(Vec::new());
        }

        // M8.1 intentionally performs no domain/network/runtime action.
        // It only advances bounded scheduler state; later packages attach typed handlers.
        let mut state = self.state.lock().await;
        let due = self.schedule.take_due(&mut state, now_ms);
        if !due.is_empty() {
            let operation_snapshot = self.runtime_operations.snapshot()?;
            let outcome = if operation_snapshot.is_idle() {
                state.deferrals.remove("runtime_operation");
                AutomationOutcome {
                    kind: OutcomeKind::Deferred,
                    code: OutcomeCode::PolicyDisabled,
                    observed_at_ms: now_ms,
                }
            } else {
                let retry = state
                    .deferrals
                    .entry("runtime_operation".into())
                    .or_default()
                    .record(
                        now_ms,
                        DeferralPolicy {
                            initial_ms: DEFAULT_DEFERRAL_INITIAL_MS,
                            max_ms: DEFAULT_DEFERRAL_MAX_MS,
                        },
                    );
                for class in &due {
                    if let Some(schedule) = state.schedules.get_mut(class) {
                        schedule.next_due_ms = schedule.next_due_ms.max(retry);
                    }
                }
                AutomationOutcome {
                    kind: OutcomeKind::Deferred,
                    code: OutcomeCode::CoordinatorBusy,
                    observed_at_ms: now_ms,
                }
            };
            state.last_outcome = Some(outcome);
        }
        state.last_persisted_wall_ms = now_ms.max(state.last_persisted_wall_ms);
        self.store.persist(&state)?;
        Ok(due)
    }
}

struct AutomationRunGuard(Arc<AtomicBool>);

impl Drop for AutomationRunGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{
        config::{AutomationConfig, AutomationPolicy},
        operation::OperationService,
        storage::{ActorKind, HistoryAction, HistoryHandle, OperationOutcome, SubjectKind},
        update::RuntimeOperationCoordinator,
    };

    #[derive(Default)]
    struct FakeClock(AtomicU64);

    impl FakeClock {
        fn new(now_ms: u64) -> Self {
            Self(AtomicU64::new(now_ms))
        }

        fn set(&self, now_ms: u64) {
            self.0.store(now_ms, Ordering::SeqCst);
        }
    }

    impl AutomationClock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn test_operation_service(runtime_root: &Path) -> OperationService {
        OperationService::new(HistoryHandle::initialize(runtime_root))
    }

    #[test]
    fn state_store_round_trips_and_rejects_future_or_corrupt_state() {
        let root = tempfile::tempdir().unwrap();
        let store = AutomationStateStore::new(root.path());
        let state = AutomationState::new(1_000, Some("policy".into()));
        store.persist(&state).unwrap();
        assert_eq!(store.load().unwrap(), Some(state.clone()));

        let mut future = state.clone();
        future.schema_version = 2;
        fs::write(store.path(), serde_json::to_vec(&future).unwrap()).unwrap();
        assert!(store.load().is_err());

        fs::write(store.path(), b"{not-json").unwrap();
        assert!(store.load().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn state_store_rejects_symlink_target() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let store = AutomationStateStore::new(root.path());
        let parent = store.path().parent().unwrap();
        fs::create_dir_all(parent).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, b"{}").unwrap();
        symlink(&outside, store.path()).unwrap();
        assert!(store.load().is_err());
        assert!(store.persist(&AutomationState::new(1, None)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn state_store_rejects_symlinked_ancestor() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("studio")).unwrap();
        let store = AutomationStateStore::new(root.path());
        assert!(store.load().is_err());
        assert!(store.persist(&AutomationState::new(1, None)).is_err());
    }

    #[test]
    fn manual_defaults_do_not_schedule_update_or_reconciliation_work() {
        let config = AutomationConfig::default();
        let engine = ScheduleEngine::from_config(&config);
        let mut state = AutomationState::new(1_000, None);
        engine.reconcile(&mut state, 1_000);

        assert!(state.schedules.contains_key(&ScheduleClass::Health));
        assert!(state.schedules.contains_key(&ScheduleClass::Cleanup));
        assert!(!state.schedules.contains_key(&ScheduleClass::UpdateCheck));
        assert!(
            !state
                .schedules
                .contains_key(&ScheduleClass::ReconciliationCheck)
        );
    }

    #[test]
    fn schedule_collapses_missed_intervals_to_one_due_event() {
        let mut config = AutomationConfig::default();
        config.updates.policy = AutomationPolicy::NotifyOnly;
        config.updates.check_on_startup = true;
        let engine = ScheduleEngine::from_config(&config);
        let mut state = AutomationState::new(1_000, None);
        engine.reconcile(&mut state, 1_000);

        let first_due = state.schedules[&ScheduleClass::UpdateCheck].next_due_ms;
        let now = first_due + 10 * config.updates.check_interval_seconds * 1_000;
        let due = engine.take_due(&mut state, now);
        assert_eq!(
            due.iter()
                .filter(|class| **class == ScheduleClass::UpdateCheck)
                .count(),
            1
        );
        assert_eq!(
            state.schedules[&ScheduleClass::UpdateCheck].next_due_ms,
            now + config.updates.check_interval_seconds * 1_000
        );
    }

    #[test]
    fn circuit_counts_failures_but_not_deferrals_and_half_open_is_single_probe() {
        let policy = CircuitPolicy {
            failure_threshold: 2,
            cooldown_ms: 100,
        };
        let mut circuit = CircuitState::default();
        let mut deferral = DeferralState::default();
        let retry = deferral.record(
            1_000,
            DeferralPolicy {
                initial_ms: 30,
                max_ms: 100,
            },
        );
        assert_eq!(retry, 1_030);
        assert_eq!(deferral.consecutive_deferrals, 1);
        assert_eq!(circuit.consecutive_failures, 0);

        circuit.record_failure(1_000, policy);
        assert_eq!(circuit.phase, CircuitPhase::Closed);
        circuit.record_failure(1_010, policy);
        assert_eq!(circuit.phase, CircuitPhase::Open);
        assert!(!circuit.ready_for_probe(1_050));
        assert!(circuit.ready_for_probe(1_110));
        assert_eq!(circuit.phase, CircuitPhase::HalfOpen);
        assert!(!circuit.ready_for_probe(1_111));
        circuit.record_success();
        assert_eq!(circuit.phase, CircuitPhase::Closed);
    }

    #[test]
    fn deferral_backoff_is_exponential_and_bounded() {
        let policy = DeferralPolicy {
            initial_ms: 10,
            max_ms: 40,
        };
        let mut state = DeferralState::default();
        assert_eq!(state.record(0, policy), 10);
        assert_eq!(state.record(10, policy), 30);
        assert_eq!(state.record(30, policy), 70);
        assert_eq!(state.record(70, policy), 110);
        assert_eq!(state.record(110, policy), 150);
        assert_eq!(state.retry_not_before_ms, Some(150));
        assert!(!state.ready(149));
        assert!(state.ready(150));
        state.clear();
        assert_eq!(state, DeferralState::default());
    }

    #[tokio::test]
    async fn controller_blocks_corrupt_state_without_preventing_manual_startup() {
        let root = tempfile::tempdir().unwrap();
        let store = AutomationStateStore::new(root.path());
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), b"{broken").unwrap();

        let clock = Arc::new(FakeClock::new(10_000));
        let controller = AutomationController::with_clock(
            AutomationConfig::default(),
            root.path().to_owned(),
            Some("policy".into()),
            Arc::new(RuntimeOperationCoordinator::default()),
            test_operation_service(root.path()),
            clock,
        );
        assert_eq!(
            controller.blocker().await,
            Some(AutomationBlocker::StateUnavailable)
        );
    }

    #[tokio::test]
    async fn controller_detects_backward_clock_and_recovers_when_time_catches_up() {
        let root = tempfile::tempdir().unwrap();
        let store = AutomationStateStore::new(root.path());
        store
            .persist(&AutomationState::new(1_000_000, Some("policy".into())))
            .unwrap();

        let clock = Arc::new(FakeClock::new(100_000));
        let controller = AutomationController::with_clock(
            AutomationConfig::default(),
            root.path().to_owned(),
            Some("policy".into()),
            Arc::new(RuntimeOperationCoordinator::default()),
            test_operation_service(root.path()),
            clock.clone(),
        );
        assert_eq!(
            controller.blocker().await,
            Some(AutomationBlocker::ClockAnomaly)
        );

        clock.set(1_000_000);
        controller.refresh_clock_blocker(1_000_000).await.unwrap();
        assert_eq!(controller.blocker().await, None);
    }

    #[tokio::test]
    async fn coordinator_busy_is_a_deferral_and_concurrent_ticks_consume_one_due_event() {
        let root = tempfile::tempdir().unwrap();
        let config = AutomationConfig {
            startup_grace_seconds: 0,
            ..AutomationConfig::default()
        };
        let clock = Arc::new(FakeClock::new(10_000));
        let coordinator = Arc::new(RuntimeOperationCoordinator::default());
        let _control = coordinator.acquire_control("busy_test").unwrap();
        let controller = Arc::new(AutomationController::with_clock(
            config,
            root.path().to_owned(),
            Some("policy".into()),
            coordinator,
            test_operation_service(root.path()),
            clock,
        ));

        let (left, right) = tokio::join!(controller.tick(), controller.tick());
        let total = left.unwrap().len() + right.unwrap().len();
        assert_eq!(total, 1);
        let state = controller.state().await;
        assert_eq!(
            state.last_outcome,
            Some(AutomationOutcome {
                kind: OutcomeKind::Deferred,
                code: OutcomeCode::CoordinatorBusy,
                observed_at_ms: 10_000,
            })
        );
        let deferral = &state.deferrals["runtime_operation"];
        assert_eq!(deferral.consecutive_deferrals, 1);
        assert_eq!(deferral.retry_not_before_ms, Some(40_000));
        assert!(
            state.schedules[&ScheduleClass::Health].next_due_ms >= 40_000,
            "coordinator deferral must push the due schedule to bounded retry"
        );
    }

    #[tokio::test]
    async fn restart_with_overdue_schedule_runs_once_without_catchup() {
        let root = tempfile::tempdir().unwrap();
        let mut config = AutomationConfig::default();
        config.updates.policy = AutomationPolicy::NotifyOnly;
        let engine = ScheduleEngine::from_config(&config);
        let mut persisted = AutomationState::new(1_000, Some("policy".into()));
        engine.reconcile(&mut persisted, 1_000);
        persisted
            .schedules
            .get_mut(&ScheduleClass::UpdateCheck)
            .unwrap()
            .next_due_ms = 2_000;
        AutomationStateStore::new(root.path())
            .persist(&persisted)
            .unwrap();

        let clock = Arc::new(FakeClock::new(1_000_000));
        let controller = AutomationController::with_clock(
            config.clone(),
            root.path().to_owned(),
            Some("policy".into()),
            Arc::new(RuntimeOperationCoordinator::default()),
            test_operation_service(root.path()),
            clock,
        );
        let first = controller.tick().await.unwrap();
        let second = controller.tick().await.unwrap();
        assert!(first.contains(&ScheduleClass::UpdateCheck));
        assert!(!second.contains(&ScheduleClass::UpdateCheck));
        assert_eq!(
            controller.state().await.schedules[&ScheduleClass::UpdateCheck].next_due_ms,
            1_000_000 + config.updates.check_interval_seconds * 1_000
        );
    }

    #[tokio::test]
    async fn controller_and_api_share_the_same_audit_service_contract() {
        let root = tempfile::tempdir().unwrap();
        let history = HistoryHandle::initialize(root.path());
        let run_id = uuid::Uuid::new_v4();
        history
            .start_run(run_id, "automation-audit-test", None)
            .unwrap();
        history.mark_run_ready().unwrap();
        let operations = OperationService::new(history.clone());

        let controller = AutomationController::new(
            AutomationConfig::default(),
            root.path().to_owned(),
            Some("policy".into()),
            Arc::new(RuntimeOperationCoordinator::default()),
            operations.clone(),
        );
        let context = controller
            .operation_service()
            .admit(
                SubjectKind::System,
                HistoryAction::UpdateCheck,
                ActorKind::LocalOperator,
            )
            .await
            .unwrap();
        let operation_id = context.operation_id();
        let receipt = controller
            .operation_service()
            .finish(&context, OperationOutcome::Succeeded, None)
            .await
            .unwrap();
        assert_eq!(receipt.operation_id, operation_id);
        history.shutdown();
    }

    #[tokio::test]
    async fn controller_shutdown_is_prompt_and_never_runs_mutation() {
        let root = tempfile::tempdir().unwrap();
        let controller = Arc::new(AutomationController::new(
            AutomationConfig::default(),
            root.path().to_owned(),
            Some("policy".into()),
            Arc::new(RuntimeOperationCoordinator::default()),
            test_operation_service(root.path()),
        ));
        let shutdown = Arc::new(Notify::new());
        let task = tokio::spawn(controller.run(shutdown.clone()));
        tokio::task::yield_now().await;
        shutdown.notify_waiters();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("controller shutdown timed out")
            .unwrap();
    }
}
