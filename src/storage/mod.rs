//! Private, bounded SQLite history storage for M6.
//!
//! SQLite is authoritative only for committed historical evidence. Runtime owners,
//! registry/config files, and M5 recovery journals remain authoritative for live
//! control and recovery.

use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nix::fcntl::{Flock, FlockArg};
#[cfg(unix)]
use nix::unistd::Uid;
use rusqlite::{Connection, OpenFlags, OptionalExtension, backup::Backup};
use sha2::{Digest, Sha256};

use crate::{
    error::{StudioError, StudioResult},
    metrics::{DAY_MS, HOUR_MS, METRIC_DEFINITION_VERSION, MetricCode},
    realtime::{EventHub, StudioEvent},
};

mod config_history;
mod events;
mod gateway_history;
mod lifecycle;
mod operation;
mod query;
mod retention;
mod update_history;

pub use events::{ActorKind, HistoryAction, OperationOutcome, SubjectKind};
pub use gateway_history::{GatewayHistoryBatch, GatewayHistoryEvent};
pub use lifecycle::{LifecycleEndKind, LifecycleOwnerKind, LifecycleSessionContext};
pub use operation::{OperationAdmissionReceipt, OperationContext, OperationTerminalReceipt};
pub use query::{
    GatewayLatencyRequest, GatewayLatencySummary, HistoricalConfigRevision, HistoricalDrift,
    HistoricalEvent, HistoricalLineage, HistoricalMetrics, HistoricalOperation, HistoricalSession,
    HistoricalSubject, HistoricalUpdate, HistoryCoverage, HistoryListRequest, HistoryPage,
    HistoryReadRequest, HistoryReadResponse, HistorySnapshot, HistoryStatus, MetricsRequest,
};
pub use retention::HousekeepingReport;

use config_history::{ConfigRevisionObservation, DriftObservation, RegistryEntryProjection};
use events::OperationEventPayload;
use gateway_history::record_gateway_events;
use lifecycle::{LifecycleEventPayload, LifecycleTerminalFact};
use operation::MAX_ACTIVE_OPERATIONS;
use query::execute_query;
use retention::{
    AUDIT_RETENTION_MS, DAILY_RETENTION_MS, DETAIL_RETENTION_MS, HARD_EVENT_COUNT,
    HOURLY_RETENTION_MS, MAX_PAGE_COUNT, RETENTION_DELETE_LIMIT, TERMINAL_RESERVE_PAGES,
};
use update_history::{OperationKind, UpdateCheckObservation, UpdateTransactionObservation};

const APPLICATION_ID: i64 = 0x4d43_5348;
const SCHEMA_VERSION: i64 = 2;
const SCHEMA_V1_SHA256: &str = "1f24c81fcff6e7275bcdb939c14fc99abc4d946c817c4d617ef2d7593c8b3b70";
const SCHEMA_DOCUMENT: &str =
    include_str!("../../docs/plans/m6-persistence-metrics-auditability/SCHEMA.md");
const MIGRATION_V2_SQL: &str = include_str!("migrations/0002_system_automation_actor.sql");
const MIGRATION_V2_SHA256: &str =
    "b765c968dad0d359bc60f1e8849247e6e961e63d429559a1c6b790c3da864c0c";
const READ_WORKER_COUNT: usize = 2;
const MIN_SQLITE_VERSION: &str = "3.51.3";
const MAX_WRITER_CAPACITY: usize = 4096;
const MAX_TERMINAL_RESERVE: usize = 1024;
const MAX_READER_CAPACITY: usize = 2048;

#[derive(Debug, Clone)]
pub struct HistoryConfig {
    pub writer_capacity: usize,
    pub terminal_reserve: usize,
    pub reader_capacity: usize,
    pub max_page_count: i64,
    pub startup_timeout: Duration,
    pub receipt_timeout: Duration,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            writer_capacity: 512,
            terminal_reserve: 64,
            reader_capacity: 16,
            max_page_count: MAX_PAGE_COUNT,
            startup_timeout: Duration::from_secs(5),
            receipt_timeout: Duration::from_millis(750),
        }
    }
}

impl HistoryConfig {
    fn validate(&self) -> StudioResult<()> {
        if self.writer_capacity == 0
            || self.writer_capacity > MAX_WRITER_CAPACITY
            || self.terminal_reserve == 0
            || self.terminal_reserve > MAX_TERMINAL_RESERVE
        {
            return Err(StudioError::History(
                "history writer queue capacity is outside the supported bound".into(),
            ));
        }
        if self.reader_capacity == 0 || self.reader_capacity > MAX_READER_CAPACITY {
            return Err(StudioError::History(
                "history reader queue capacity is outside the supported bound".into(),
            ));
        }
        if self.max_page_count < 64 || self.max_page_count > MAX_PAGE_COUNT {
            return Err(StudioError::History(
                "history max page count is outside the supported bound".into(),
            ));
        }
        if self.startup_timeout.is_zero() || self.receipt_timeout.is_zero() {
            return Err(StudioError::History(
                "history timeouts must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryHealth {
    Healthy,
    Degraded { code: &'static str },
}

#[derive(Clone)]
pub struct HistoryHandle {
    inner: Arc<HistoryInner>,
}

struct HistoryInner {
    root: PathBuf,
    health: Mutex<HistoryHealth>,
    workers: Mutex<Option<WorkerSet>>,
    receipt_timeout: Duration,
    next_reader: AtomicUsize,
    active_operations: AtomicUsize,
    current_run: Mutex<Option<uuid::Uuid>>,
    events: EventHub,
}

struct WorkerSet {
    writer: mpsc::SyncSender<WriterCommand>,
    terminal: mpsc::SyncSender<TerminalCommand>,
    readers: Vec<mpsc::SyncSender<ReadCommand>>,
}

enum WriterCommand {
    Backup {
        destination: PathBuf,
        reply: mpsc::SyncSender<StudioResult<()>>,
    },
    RecordBootstrap {
        subject_id: uuid::Uuid,
        source_stream: String,
        source_ordinal: u64,
        reply: mpsc::SyncSender<StudioResult<u64>>,
    },
    AdmitOperation {
        context: OperationContext,
        reply: mpsc::SyncSender<StudioResult<OperationAdmissionReceipt>>,
    },
    StartRun {
        run_id: uuid::Uuid,
        process_identity: String,
        loaded_config_digest: Option<String>,
        reply: mpsc::SyncSender<StudioResult<()>>,
    },
    MarkRunReady {
        run_id: uuid::Uuid,
        reply: mpsc::SyncSender<StudioResult<()>>,
    },
    CloseRun {
        run_id: uuid::Uuid,
        end_state: &'static str,
        reply: mpsc::SyncSender<StudioResult<()>>,
    },
    ObserveLifecycleStarted {
        run_id: uuid::Uuid,
        context: LifecycleSessionContext,
    },
    ObserveLifecycleStartFailed {
        run_id: uuid::Uuid,
        owner_kind: LifecycleOwnerKind,
        generation: u64,
        subject_id: uuid::Uuid,
        source_stream: String,
    },
    ObserveLifecycleHeartbeat {
        run_id: uuid::Uuid,
        session_id: uuid::Uuid,
        observed_at_ms: i64,
    },
    ObserveUpdateTransaction {
        run_id: Option<uuid::Uuid>,
        observation: UpdateTransactionObservation,
    },
    ObserveUpdateCheck {
        run_id: Option<uuid::Uuid>,
        observation: UpdateCheckObservation,
    },
    ObserveConfigRevision {
        observation: ConfigRevisionObservation,
        registry: Option<RegistryEntryProjection>,
    },
    ObserveDrift {
        observation: DriftObservation,
    },
    ObserveGatewayEvents {
        instance_id: String,
        events: Vec<GatewayHistoryEvent>,
        reply: mpsc::SyncSender<StudioResult<u64>>,
    },
    RecordValidatedJournal {
        domain: String,
        transaction_id: String,
        revision: u64,
        payload_sha256: String,
        phase: String,
        reply: mpsc::SyncSender<StudioResult<()>>,
    },
    Housekeeping {
        reply: mpsc::SyncSender<StudioResult<HousekeepingReport>>,
    },
    Shutdown(mpsc::SyncSender<()>),
}

enum TerminalCommand {
    FinishOperation {
        context: OperationContext,
        outcome: OperationOutcome,
        error_code: Option<String>,
        reply: mpsc::SyncSender<StudioResult<OperationTerminalReceipt>>,
    },
    ObserveLifecycleTerminal {
        run_id: uuid::Uuid,
        fact: LifecycleTerminalFact,
    },
    ObserveUpdateTerminal {
        run_id: Option<uuid::Uuid>,
        observation: Box<UpdateTransactionObservation>,
    },
}

enum ReadCommand {
    Verify {
        full: bool,
        reply: mpsc::SyncSender<StudioResult<()>>,
    },
    Query {
        request: Box<HistoryReadRequest>,
        reply: mpsc::SyncSender<StudioResult<HistoryReadResponse>>,
    },
    Shutdown(mpsc::SyncSender<()>),
}

impl HistoryHandle {
    pub fn initialize(runtime_root: &Path) -> Self {
        Self::with_config(runtime_root, HistoryConfig::default())
    }

    pub fn with_config(runtime_root: &Path, config: HistoryConfig) -> Self {
        let events = EventHub::default();
        match start_workers(runtime_root, config.clone(), events.clone()) {
            Ok((root, workers)) => Self {
                inner: Arc::new(HistoryInner {
                    root,
                    health: Mutex::new(HistoryHealth::Healthy),
                    workers: Mutex::new(Some(workers)),
                    receipt_timeout: config.receipt_timeout,
                    next_reader: AtomicUsize::new(0),
                    active_operations: AtomicUsize::new(0),
                    current_run: Mutex::new(None),
                    events,
                }),
            },
            Err(error) => {
                tracing::error!(history_error = %error, "history store is degraded");
                events.publish(StudioEvent::HistoryHealth {
                    state: "degraded".into(),
                    reason_code: Some("history_open_failed".into()),
                    admission_available: false,
                });
                Self {
                    inner: Arc::new(HistoryInner {
                        root: history_root(runtime_root),
                        health: Mutex::new(HistoryHealth::Degraded {
                            code: "history_open_failed",
                        }),
                        workers: Mutex::new(None),
                        receipt_timeout: config.receipt_timeout,
                        next_reader: AtomicUsize::new(0),
                        active_operations: AtomicUsize::new(0),
                        current_run: Mutex::new(None),
                        events,
                    }),
                }
            }
        }
    }

    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<StudioEvent> {
        self.inner.events.subscribe()
    }

    pub fn health(&self) -> HistoryHealth {
        self.inner.health.lock().unwrap().clone()
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn shutdown(&self) {
        let Some(workers) = self.inner.workers.lock().unwrap().take() else {
            return;
        };

        for reader in workers.readers {
            let (done_sender, done_receiver) = mpsc::sync_channel(1);
            if reader.try_send(ReadCommand::Shutdown(done_sender)).is_ok() {
                let _ = done_receiver.recv_timeout(self.inner.receipt_timeout);
            }
        }

        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        if workers
            .writer
            .try_send(WriterCommand::Shutdown(done_sender))
            .is_ok()
        {
            let _ = done_receiver.recv_timeout(self.inner.receipt_timeout);
        }
    }

    pub fn start_run(
        &self,
        run_id: uuid::Uuid,
        process_identity: &str,
        loaded_config_digest: Option<&str>,
    ) -> StudioResult<()> {
        if process_identity.is_empty() || process_identity.len() > 512 {
            return Err(StudioError::History(
                "invalid Studio process identity".into(),
            ));
        }
        if let Some(digest) = loaded_config_digest
            && !valid_sha256(digest)
        {
            return Err(StudioError::History(
                "invalid loaded configuration digest".into(),
            ));
        }
        if self.inner.current_run.lock().unwrap().is_some() {
            return Err(StudioError::History(
                "history Studio run is already active".into(),
            ));
        }

        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::StartRun {
                run_id,
                process_identity: process_identity.to_owned(),
                loaded_config_digest: loaded_config_digest.map(ToOwned::to_owned),
                reply,
            })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        let result = receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history run start timed out".into()))?;
        match result {
            Ok(()) => {
                *self.inner.current_run.lock().unwrap() = Some(run_id);
                Ok(())
            }
            Err(error) => {
                self.mark_degraded("history_run_start_failed");
                Err(error)
            }
        }
    }

    pub fn mark_run_ready(&self) -> StudioResult<()> {
        let run_id = self
            .current_run_id()
            .ok_or_else(|| StudioError::History("no active history Studio run".into()))?;
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::MarkRunReady { run_id, reply })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history run ready receipt timed out".into()))?
    }

    pub fn close_run(&self, shutdown_complete: bool) -> StudioResult<()> {
        let run_id = self
            .current_run_id()
            .ok_or_else(|| StudioError::History("no active history Studio run".into()))?;
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::CloseRun {
                run_id,
                end_state: if shutdown_complete {
                    "clean"
                } else {
                    "shutdown_incomplete"
                },
                reply,
            })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        let result = receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history run close receipt timed out".into()))?;
        if result.is_ok() {
            *self.inner.current_run.lock().unwrap() = None;
        }
        result
    }

    pub fn observe_start_failed(&self, owner_kind: LifecycleOwnerKind, generation: u64) {
        if self.health() != HistoryHealth::Healthy {
            return;
        }
        let Some(run_id) = self.current_run_id() else {
            return;
        };
        let subject_id = uuid::Uuid::new_v4();
        let source_stream = format!("start-failed/{}", uuid::Uuid::new_v4());
        let Ok(sender) = self.writer_sender() else {
            return;
        };
        if sender
            .try_send(WriterCommand::ObserveLifecycleStartFailed {
                run_id,
                owner_kind,
                generation,
                subject_id,
                source_stream,
            })
            .is_err()
        {
            self.mark_degraded("history_lifecycle_queue_full");
        }
    }

    pub fn observe_session_started(
        &self,
        owner_kind: LifecycleOwnerKind,
        generation: u64,
        pid: u32,
    ) -> Option<LifecycleSessionContext> {
        if self.health() != HistoryHealth::Healthy {
            return None;
        }
        let run_id = self.current_run_id()?;
        let context = LifecycleSessionContext::new(owner_kind, generation, pid);
        let sender = self.writer_sender().ok()?;
        if sender
            .try_send(WriterCommand::ObserveLifecycleStarted {
                run_id,
                context: context.clone(),
            })
            .is_err()
        {
            self.mark_degraded("history_lifecycle_queue_full");
            return None;
        }
        Some(context)
    }

    pub fn observe_session_heartbeat(&self, context: &LifecycleSessionContext) {
        if self.health() != HistoryHealth::Healthy {
            return;
        }
        let Some(run_id) = self.current_run_id() else {
            return;
        };
        let Ok(observed_at_ms) = now_ms() else {
            self.mark_degraded("history_clock_failed");
            return;
        };
        let Ok(sender) = self.writer_sender() else {
            return;
        };
        if sender
            .try_send(WriterCommand::ObserveLifecycleHeartbeat {
                run_id,
                session_id: context.session_id(),
                observed_at_ms,
            })
            .is_err()
        {
            self.mark_degraded("history_lifecycle_queue_full");
        }
    }

    pub fn observe_session_terminal(
        &self,
        context: LifecycleSessionContext,
        end_kind: LifecycleEndKind,
        exit_code: Option<i32>,
        is_crash: bool,
        exact_duration: Option<Duration>,
    ) {
        let Some(run_id) = self.current_run_id() else {
            return;
        };
        let Ok(sender) = self.terminal_sender() else {
            return;
        };
        if sender
            .try_send(TerminalCommand::ObserveLifecycleTerminal {
                run_id,
                fact: LifecycleTerminalFact {
                    context,
                    end_kind,
                    exit_code,
                    is_crash,
                    exact_duration,
                },
            })
            .is_err()
        {
            self.mark_degraded("history_lifecycle_terminal_queue_full");
        }
    }

    pub fn observe_update_transaction(
        &self,
        view: &crate::update::McpUpdateTransactionView,
        operation_id: Option<uuid::Uuid>,
        is_apply: bool,
    ) {
        self.observe_update_transaction_with_artifact(view, operation_id, is_apply, None);
    }

    pub(crate) fn observe_update_transaction_with_artifact(
        &self,
        view: &crate::update::McpUpdateTransactionView,
        operation_id: Option<uuid::Uuid>,
        is_apply: bool,
        artifact: Option<crate::update::ArtifactHistoryIdentity>,
    ) {
        if self.health() != HistoryHealth::Healthy {
            return;
        }
        let observation = UpdateTransactionObservation::from_view_with_artifact(
            view,
            operation_id,
            if operation_id.is_none() {
                OperationKind::Observation
            } else if is_apply {
                OperationKind::Apply
            } else {
                OperationKind::Prepare
            },
            artifact,
        );
        let run_id = self.current_run_id();
        if observation.terminal {
            let Ok(sender) = self.terminal_sender() else {
                return;
            };
            if sender
                .try_send(TerminalCommand::ObserveUpdateTerminal {
                    run_id,
                    observation: Box::new(observation),
                })
                .is_err()
            {
                self.mark_degraded("history_update_terminal_queue_full");
            }
        } else {
            let Ok(sender) = self.writer_sender() else {
                return;
            };
            if sender
                .try_send(WriterCommand::ObserveUpdateTransaction {
                    run_id,
                    observation,
                })
                .is_err()
            {
                self.mark_degraded("history_update_queue_full");
            }
        }
    }

    pub fn observe_update_check(
        &self,
        view: &crate::update::InventoryView,
        operation_id: uuid::Uuid,
    ) {
        if self.health() != HistoryHealth::Healthy {
            return;
        }
        let observation = UpdateCheckObservation::from_view(view, operation_id);
        let run_id = self.current_run_id();
        let Ok(sender) = self.writer_sender() else {
            return;
        };
        if sender
            .try_send(WriterCommand::ObserveUpdateCheck {
                run_id,
                observation,
            })
            .is_err()
        {
            self.mark_degraded("history_update_queue_full");
        }
    }

    pub fn observe_registry_revision(
        &self,
        view: &crate::registry::RegistryEntryView,
        present: bool,
        operation_id: uuid::Uuid,
    ) {
        if self.health() != HistoryHealth::Healthy {
            return;
        }
        let (observation, registry) =
            ConfigRevisionObservation::registry(view, present, operation_id, self.current_run_id());
        let Ok(sender) = self.writer_sender() else {
            return;
        };
        if sender
            .try_send(WriterCommand::ObserveConfigRevision {
                observation,
                registry: Some(registry),
            })
            .is_err()
        {
            self.mark_degraded("history_config_queue_full");
        }
    }

    pub fn observe_drift(
        &self,
        view: &crate::update::ReconciliationView,
        operation_id: uuid::Uuid,
        observation_kind: &'static str,
    ) {
        if self.health() != HistoryHealth::Healthy {
            return;
        }
        let observation = DriftObservation::from_view(
            view,
            operation_id,
            observation_kind,
            self.current_run_id(),
        );
        let Ok(sender) = self.writer_sender() else {
            return;
        };
        if sender
            .try_send(WriterCommand::ObserveDrift { observation })
            .is_err()
        {
            self.mark_degraded("history_drift_queue_full");
        }
    }

    /// Records only Gateway's fixed, sanitized observation DTO. This is a
    /// best-effort historical sink; it is never on a live child-request path.
    pub fn observe_gateway_events(
        &self,
        instance_id: String,
        events: Vec<GatewayHistoryEvent>,
    ) -> StudioResult<u64> {
        if self.health() != HistoryHealth::Healthy {
            return Err(StudioError::History("history unavailable".into()));
        }
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::ObserveGatewayEvents {
                instance_id,
                events,
                reply,
            })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("Gateway history receipt timed out".into()))?
    }

    pub fn current_run_id(&self) -> Option<uuid::Uuid> {
        *self.inner.current_run.lock().unwrap()
    }

    pub fn record_validated_journal(
        &self,
        domain: &str,
        transaction_id: &str,
        revision: u64,
        payload_sha256: &str,
        phase: &str,
    ) -> StudioResult<()> {
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::RecordValidatedJournal {
                domain: domain.to_owned(),
                transaction_id: transaction_id.to_owned(),
                revision,
                payload_sha256: payload_sha256.to_owned(),
                phase: phase.to_owned(),
                reply,
            })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history journal receipt timed out".into()))?
    }

    pub fn housekeeping(&self) -> StudioResult<HousekeepingReport> {
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::Housekeeping { reply })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history housekeeping timed out".into()))?
    }

    pub fn backup(&self) -> StudioResult<PathBuf> {
        let destination = self.inner.root.join("backups/studio.sqlite3");
        ensure_private_directory(destination.parent().unwrap(), true)?;
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::Backup {
                destination: destination.clone(),
                reply,
            })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        let result = receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history writer receipt timed out".into()))?;
        if result.is_err() {
            self.mark_degraded("history_backup_failed");
        }
        result?;
        Ok(destination)
    }

    pub fn history_read(&self, request: HistoryReadRequest) -> StudioResult<HistoryReadResponse> {
        if matches!(request, HistoryReadRequest::Status) && self.health() != HistoryHealth::Healthy
        {
            return Ok(HistoryReadResponse::Status(HistoryStatus {
                state: "degraded",
                reason_code: Some("history_unavailable"),
                admission_available: false,
                pending_obligations: self.active_operation_count().to_string(),
                db_epoch: None,
                latest_committed_seq: None,
                metrics_through_seq: None,
                coverage_state: "unknown",
                observed_at_ms: i64::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                )
                .unwrap_or(i64::MAX),
            }));
        }
        let sender = self.reader_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(ReadCommand::Query {
                request: Box::new(request),
                reply,
            })
            .map_err(|_| StudioError::History("history reader queue is full".into()))?;
        receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history query timed out".into()))?
    }

    pub fn verify(&self) -> StudioResult<()> {
        let sender = self.reader_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(ReadCommand::Verify { full: true, reply })
            .map_err(|_| StudioError::History("history reader queue is full".into()))?;
        let result = receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history reader receipt timed out".into()))?;
        if result.is_err() {
            self.mark_degraded("history_verify_failed");
        }
        result
    }

    pub fn admit_operation(
        &self,
        subject_kind: SubjectKind,
        action: HistoryAction,
        actor: ActorKind,
    ) -> StudioResult<(OperationContext, OperationAdmissionReceipt)> {
        if self.health() != HistoryHealth::Healthy {
            return Err(StudioError::History(
                "history admission is unavailable while storage is degraded".into(),
            ));
        }
        self.reserve_operation()?;

        let context = OperationContext::new(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            action,
            actor,
            subject_kind,
            self.current_run_id(),
        );
        let sender = match self.writer_sender() {
            Ok(sender) => sender,
            Err(error) => {
                self.release_operation();
                return Err(error);
            }
        };
        let (reply, receipt) = mpsc::sync_channel(1);
        if sender
            .try_send(WriterCommand::AdmitOperation {
                context: context.clone(),
                reply,
            })
            .is_err()
        {
            self.release_operation();
            return Err(StudioError::History(
                "history admission queue is full".into(),
            ));
        }

        match receipt.recv_timeout(self.inner.receipt_timeout) {
            Ok(Ok(receipt)) => Ok((context, receipt)),
            Ok(Err(error)) => {
                self.release_operation();
                self.mark_degraded("history_admission_failed");
                Err(error)
            }
            Err(_) => {
                self.release_operation();
                self.mark_degraded("history_admission_timeout");
                Err(StudioError::History(
                    "history admission receipt timed out".into(),
                ))
            }
        }
    }

    pub fn finish_operation(
        &self,
        context: &OperationContext,
        outcome: OperationOutcome,
        error_code: Option<&str>,
    ) -> StudioResult<OperationTerminalReceipt> {
        if let Some(code) = error_code {
            events::validate_error_code(code)?;
        }
        context.claim_completion()?;

        let sender = match self.terminal_sender() {
            Ok(sender) => sender,
            Err(error) => {
                self.release_operation();
                self.mark_degraded("history_terminal_failed");
                return Err(error);
            }
        };
        let (reply, receipt) = mpsc::sync_channel(1);
        let command = TerminalCommand::FinishOperation {
            context: context.clone(),
            outcome,
            error_code: error_code.map(ToOwned::to_owned),
            reply,
        };
        if sender.try_send(command).is_err() {
            self.release_operation();
            self.mark_degraded("history_terminal_queue_full");
            return Err(StudioError::History(
                "history terminal queue is full".into(),
            ));
        }

        let result = receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history terminal receipt timed out".into()));
        self.release_operation();
        match result {
            Ok(Ok(receipt)) => Ok(receipt),
            Ok(Err(error)) => {
                self.mark_degraded("history_terminal_failed");
                Err(error)
            }
            Err(error) => {
                self.mark_degraded("history_terminal_timeout");
                Err(error)
            }
        }
    }

    pub fn active_operation_count(&self) -> usize {
        self.inner.active_operations.load(Ordering::Acquire)
    }

    fn reserve_operation(&self) -> StudioResult<()> {
        self.inner
            .active_operations
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_ACTIVE_OPERATIONS).then_some(current + 1)
            })
            .map(|_| ())
            .map_err(|_| StudioError::History("history operation limit reached".into()))
    }

    fn release_operation(&self) {
        let _ = self.inner.active_operations.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |current| current.checked_sub(1),
        );
    }
}

fn handle_terminal_command(connection: &mut Connection, command: TerminalCommand) {
    match command {
        TerminalCommand::FinishOperation {
            context,
            outcome,
            error_code,
            reply,
        } => {
            let _ = reply.send(record_operation_terminal(
                connection,
                &context,
                outcome,
                error_code.as_deref(),
            ));
        }
        TerminalCommand::ObserveLifecycleTerminal { run_id, fact } => {
            if let Err(error) = record_lifecycle_terminal(connection, run_id, &fact) {
                tracing::warn!(history_error = %error, "could not persist lifecycle terminal observation");
            }
        }
        TerminalCommand::ObserveUpdateTerminal {
            run_id,
            observation,
        } => {
            if let Err(error) = record_update_transaction(connection, run_id, &observation) {
                tracing::warn!(history_error = %error, "could not persist update terminal observation");
            }
        }
    }
}

type MetricBucketState = (i64, i64, Option<i64>, Option<i64>, String);

#[derive(Debug, Clone, Copy)]
struct MetricContribution {
    code: MetricCode,
    observed_at_ms: i64,
    count: i64,
    sum: i64,
    min: Option<i64>,
    max: Option<i64>,
}

fn ensure_admission_capacity(connection: &Connection) -> StudioResult<()> {
    let event_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .map_err(history_error)?;
    let page_count = pragma_i64(connection, "page_count")?;
    let max_page_count = pragma_i64(connection, "max_page_count")?;
    let effective_max = max_page_count.min(MAX_PAGE_COUNT);
    if event_count >= HARD_EVENT_COUNT
        || page_count >= effective_max.saturating_sub(TERMINAL_RESERVE_PAGES)
    {
        return Err(StudioError::History(
            "history capacity limit prevents new discretionary admission".into(),
        ));
    }
    Ok(())
}

fn run_housekeeping(connection: &mut Connection) -> StudioResult<HousekeepingReport> {
    run_housekeeping_with_fault(connection, false)
}

fn run_housekeeping_with_fault(
    connection: &mut Connection,
    fail_before_commit: bool,
) -> StudioResult<HousekeepingReport> {
    let now = now_ms()?;
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    let through_seq = aggregate_metrics_batch(&transaction)?;

    let hourly_cutoff = now.saturating_sub(HOURLY_RETENTION_MS);
    let daily_cutoff = now.saturating_sub(DAILY_RETENTION_MS);
    transaction
        .execute(
            "DELETE FROM metrics_buckets
             WHERE definition_version=?1
               AND ((resolution_ms=?2 AND bucket_start_ms<?3)
                 OR (resolution_ms=?4 AND bucket_start_ms<?5))",
            (
                METRIC_DEFINITION_VERSION,
                HOUR_MS,
                hourly_cutoff,
                DAY_MS,
                daily_cutoff,
            ),
        )
        .map_err(history_error)?;

    let audit_cutoff = now.saturating_sub(AUDIT_RETENTION_MS);
    let detail_cutoff = now.saturating_sub(DETAIL_RETENTION_MS);
    let mut statement = transaction
        .prepare(
            "SELECT e.seq
             FROM events e
             WHERE e.seq <= ?1
               AND (
                    (e.retention_class='audit' AND e.observed_at_ms < ?2)
                 OR (e.retention_class!='audit' AND e.observed_at_ms < ?3)
               )
               AND NOT EXISTS (
                    SELECT 1 FROM runtime_sessions s
                    WHERE s.end_kind='open' AND s.started_seq=e.seq
               )
               AND NOT EXISTS (
                    SELECT 1 FROM update_attempts u
                    WHERE u.terminal_seq IS NULL
                      AND (u.first_observed_seq=e.seq OR u.apply_started_seq=e.seq)
               )
               AND NOT EXISTS (
                    SELECT 1 FROM operations o
                    WHERE o.operation_id=e.operation_id AND o.terminal_seq IS NULL
               )
             ORDER BY e.seq
             LIMIT ?4",
        )
        .map_err(history_error)?;
    let candidates = statement
        .query_map(
            (
                i64::try_from(through_seq)
                    .map_err(|_| StudioError::History("metric watermark overflow".into()))?,
                audit_cutoff,
                detail_cutoff,
                RETENTION_DELETE_LIMIT,
            ),
            |row| row.get::<_, i64>(0),
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    drop(statement);

    let pruned_events = u64::try_from(candidates.len())
        .map_err(|_| StudioError::History("retention count overflow".into()))?;
    if let (Some(first), Some(last)) = (candidates.first(), candidates.last()) {
        transaction
            .execute(
                "INSERT INTO coverage_intervals(
                    coverage_id, reason, from_seq, to_seq, lost_count, completeness
                 ) VALUES (?1, 'retention', ?2, ?3, ?4, 'retained_boundary')",
                (
                    uuid::Uuid::new_v4().to_string(),
                    first,
                    last,
                    i64::try_from(candidates.len())
                        .map_err(|_| StudioError::History("retention count overflow".into()))?,
                ),
            )
            .map_err(history_error)?;
        for seq in &candidates {
            transaction
                .execute("DELETE FROM events WHERE seq=?1", [seq])
                .map_err(history_error)?;
        }
        transaction
            .execute(
                "UPDATE history_meta SET retention_epoch=retention_epoch+1 WHERE singleton=1",
                [],
            )
            .map_err(history_error)?;
    }

    transaction
        .execute(
            "DELETE FROM runtime_sessions
             WHERE session_id IN (
                 SELECT session_id FROM runtime_sessions
                 WHERE end_kind!='open'
                   AND COALESCE(ended_at_ms,last_observed_ms) < ?1
                 LIMIT ?2
             )",
            (detail_cutoff, RETENTION_DELETE_LIMIT),
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "DELETE FROM drift_observations
             WHERE observation_id IN (
                 SELECT d.observation_id FROM drift_observations d
                 WHERE NOT EXISTS (SELECT 1 FROM events e WHERE e.seq=d.event_seq)
                 LIMIT ?1
             )",
            [RETENTION_DELETE_LIMIT],
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "DELETE FROM update_attempts
             WHERE attempt_id IN (
                 SELECT u.attempt_id FROM update_attempts u
                 WHERE u.terminal_seq IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM events e WHERE e.seq=u.terminal_seq)
                 LIMIT ?1
             )",
            [RETENTION_DELETE_LIMIT],
        )
        .map_err(history_error)?;

    let retention_epoch: i64 = transaction
        .query_row(
            "SELECT retention_epoch FROM history_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(history_error)?;
    if fail_before_commit {
        return Err(StudioError::History(
            "injected housekeeping failure before commit".into(),
        ));
    }
    transaction.commit().map_err(history_error)?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
        .map_err(history_error)?;

    Ok(HousekeepingReport {
        through_seq,
        pruned_events,
        retention_epoch: u64::try_from(retention_epoch)
            .map_err(|_| StudioError::History("retention epoch overflow".into()))?,
    })
}

fn aggregate_metrics_batch(transaction: &rusqlite::Transaction<'_>) -> StudioResult<u64> {
    let through: i64 = transaction
        .query_row(
            "SELECT COALESCE(through_seq,0)
             FROM projection_state
             WHERE projection_code='metrics.v1'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?
        .unwrap_or(0);

    let mut statement = transaction
        .prepare(
            "SELECT seq, name, observed_at_ms
             FROM events
             WHERE seq>?1
             ORDER BY seq
             LIMIT 256",
        )
        .map_err(history_error)?;
    let rows = statement
        .query_map([through], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    drop(statement);

    if rows.is_empty() {
        return u64::try_from(through)
            .map_err(|_| StudioError::History("metric watermark overflow".into()));
    }

    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, incarnation_id, first_observed_ms
             ) VALUES ('metrics-system','system','metrics-system',?1)",
            [rows[0].2],
        )
        .map_err(history_error)?;

    let mut last_seq = through;
    for (seq, name, observed_at_ms) in rows {
        for contribution in metric_contributions_for_event(transaction, seq, &name, observed_at_ms)?
        {
            apply_metric_contribution(transaction, seq, contribution)?;
        }
        last_seq = seq;
    }

    transaction
        .execute(
            "INSERT INTO projection_state(
                projection_code, definition_version, through_seq, updated_at_ms
             ) VALUES ('metrics.v1', ?1, ?2, ?3)
             ON CONFLICT(projection_code) DO UPDATE SET
                definition_version=excluded.definition_version,
                through_seq=excluded.through_seq,
                updated_at_ms=excluded.updated_at_ms",
            (METRIC_DEFINITION_VERSION, last_seq, now_ms()?),
        )
        .map_err(history_error)?;

    u64::try_from(last_seq).map_err(|_| StudioError::History("metric watermark overflow".into()))
}

fn metric_contributions_for_event(
    transaction: &rusqlite::Transaction<'_>,
    seq: i64,
    name: &str,
    observed_at_ms: i64,
) -> StudioResult<Vec<MetricContribution>> {
    let mut contributions = Vec::new();

    if matches!(name, "mcp.session.started" | "tunnel.session.started") {
        contributions.push(counter_contribution(MetricCode::Launches, observed_at_ms));
    }
    if matches!(name, "mcp.session.ended" | "tunnel.session.ended") {
        let session: Option<(Option<i64>, Option<i64>)> = transaction
            .query_row(
                "SELECT exact_duration_ms, is_crash
                 FROM runtime_sessions WHERE closed_seq=?1",
                [seq],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(history_error)?;
        if let Some((duration, is_crash)) = session {
            if is_crash == Some(1) {
                contributions.push(counter_contribution(MetricCode::Crashes, observed_at_ms));
            }
            if let Some(duration) = duration {
                if duration < 0 {
                    return Err(StudioError::History(
                        "negative exact lifecycle duration".into(),
                    ));
                }
                contributions.push(MetricContribution {
                    code: MetricCode::ExactSessionDurationMs,
                    observed_at_ms,
                    count: 1,
                    sum: duration,
                    min: Some(duration),
                    max: Some(duration),
                });
            }
        }
    }

    if name == "operation.finished" || name == "operation.indeterminate" {
        let operation: Option<(String, String)> = transaction
            .query_row(
                "SELECT action_code, effect_status
                 FROM operations WHERE terminal_seq=?1",
                [seq],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(history_error)?;
        if let Some((action, effect)) = operation {
            if matches!(action.as_str(), "mcp.restart" | "tunnel.restart") {
                contributions.push(counter_contribution(
                    MetricCode::RestartAttempts,
                    observed_at_ms,
                ));
            }
            if action == "update.check" {
                contributions.push(counter_contribution(
                    MetricCode::UpdateChecks,
                    observed_at_ms,
                ));
            }
            if action == "update.prepare" {
                contributions.push(counter_contribution(
                    MetricCode::PrepareAttempts,
                    observed_at_ms,
                ));
                if effect == "failed" {
                    contributions.push(counter_contribution(
                        MetricCode::PrepareFailures,
                        observed_at_ms,
                    ));
                }
            }
        }
    }

    let available: Option<i64> = transaction
        .query_row(
            "SELECT update_available FROM update_checks WHERE event_seq=?1",
            [seq],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?
        .flatten();
    if available == Some(1) {
        contributions.push(counter_contribution(
            MetricCode::UpdatesAvailable,
            observed_at_ms,
        ));
    }

    let attempt: Option<(Option<i64>, Option<i64>, String, String)> = transaction
        .query_row(
            "SELECT apply_started_seq, terminal_seq, install_outcome, rollback_outcome
             FROM update_attempts
             WHERE apply_started_seq=?1 OR terminal_seq=?1
             LIMIT 1",
            [seq],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(history_error)?;
    if let Some((apply_started_seq, terminal_seq, install_outcome, rollback_outcome)) = attempt {
        if apply_started_seq == Some(seq) {
            contributions.push(counter_contribution(
                MetricCode::InstallAttempts,
                observed_at_ms,
            ));
        }
        if terminal_seq == Some(seq) {
            match install_outcome.as_str() {
                "succeeded" => contributions.push(counter_contribution(
                    MetricCode::InstallSuccesses,
                    observed_at_ms,
                )),
                "failed" => contributions.push(counter_contribution(
                    MetricCode::InstallFailures,
                    observed_at_ms,
                )),
                "interrupted" | "indeterminate" => contributions.push(counter_contribution(
                    MetricCode::InterruptedInstalls,
                    observed_at_ms,
                )),
                _ => {}
            }
            match rollback_outcome.as_str() {
                "succeeded" => contributions.push(counter_contribution(
                    MetricCode::VerifiedRollbacks,
                    observed_at_ms,
                )),
                "failed" => contributions.push(counter_contribution(
                    MetricCode::RollbackFailures,
                    observed_at_ms,
                )),
                _ => {}
            }
        }
    }

    if name == "fleet.drift.observed" {
        contributions.push(counter_contribution(
            MetricCode::DriftObservations,
            observed_at_ms,
        ));
    }

    Ok(contributions)
}

fn counter_contribution(code: MetricCode, observed_at_ms: i64) -> MetricContribution {
    MetricContribution {
        code,
        observed_at_ms,
        count: 1,
        sum: 1,
        min: None,
        max: None,
    }
}

fn apply_metric_contribution(
    transaction: &rusqlite::Transaction<'_>,
    through_seq: i64,
    contribution: MetricContribution,
) -> StudioResult<()> {
    let quality = metric_quality_at(transaction, contribution.observed_at_ms)?;
    let metric = contribution.code.as_db();
    let existing: Option<(i64, i64, i64, String)> = transaction
        .query_row(
            "SELECT count_value, sum_value, coverage_start_ms, quality
             FROM metrics_totals
             WHERE subject_id='metrics-system' AND metric_code=?1 AND definition_version=?2",
            (metric, METRIC_DEFINITION_VERSION),
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(history_error)?;
    let (count, sum, coverage_start, total_quality) =
        if let Some((old_count, old_sum, old_start, old_quality)) = existing {
            (
                old_count
                    .checked_add(contribution.count)
                    .ok_or_else(|| StudioError::History("metric count overflow".into()))?,
                old_sum
                    .checked_add(contribution.sum)
                    .ok_or_else(|| StudioError::History("metric sum overflow".into()))?,
                old_start.min(contribution.observed_at_ms),
                if old_quality == "partial" || quality == "partial" {
                    "partial"
                } else {
                    "complete"
                },
            )
        } else {
            (
                contribution.count,
                contribution.sum,
                contribution.observed_at_ms,
                quality,
            )
        };
    transaction
        .execute(
            "INSERT INTO metrics_totals(
                subject_id, metric_code, definition_version, count_value, sum_value,
                through_seq, coverage_start_ms, quality
             ) VALUES ('metrics-system',?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(subject_id,metric_code,definition_version) DO UPDATE SET
                count_value=excluded.count_value,
                sum_value=excluded.sum_value,
                through_seq=excluded.through_seq,
                coverage_start_ms=excluded.coverage_start_ms,
                quality=excluded.quality",
            (
                metric,
                METRIC_DEFINITION_VERSION,
                count,
                sum,
                through_seq,
                coverage_start,
                total_quality,
            ),
        )
        .map_err(history_error)?;

    for resolution in [HOUR_MS, DAY_MS] {
        let bucket_start = contribution.observed_at_ms.div_euclid(resolution) * resolution;
        let existing: Option<MetricBucketState> = transaction
            .query_row(
                "SELECT count_value, sum_value, min_value, max_value, quality
                 FROM metrics_buckets
                 WHERE subject_id='metrics-system' AND metric_code=?1
                   AND definition_version=?2 AND resolution_ms=?3 AND bucket_start_ms=?4",
                (metric, METRIC_DEFINITION_VERSION, resolution, bucket_start),
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(history_error)?;
        let (bucket_count, bucket_sum, bucket_min, bucket_max, bucket_quality) =
            if let Some((old_count, old_sum, old_min, old_max, old_quality)) = existing {
                (
                    old_count.checked_add(contribution.count).ok_or_else(|| {
                        StudioError::History("metric bucket count overflow".into())
                    })?,
                    old_sum
                        .checked_add(contribution.sum)
                        .ok_or_else(|| StudioError::History("metric bucket sum overflow".into()))?,
                    min_option(old_min, contribution.min),
                    max_option(old_max, contribution.max),
                    if old_quality == "partial" || quality == "partial" {
                        "partial"
                    } else {
                        "complete"
                    },
                )
            } else {
                (
                    contribution.count,
                    contribution.sum,
                    contribution.min,
                    contribution.max,
                    quality,
                )
            };
        transaction
            .execute(
                "INSERT INTO metrics_buckets(
                    subject_id, metric_code, definition_version, resolution_ms,
                    bucket_start_ms, count_value, sum_value, min_value, max_value,
                    through_seq, quality
                 ) VALUES ('metrics-system',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(
                    subject_id,metric_code,definition_version,resolution_ms,bucket_start_ms
                 ) DO UPDATE SET
                    count_value=excluded.count_value,
                    sum_value=excluded.sum_value,
                    min_value=excluded.min_value,
                    max_value=excluded.max_value,
                    through_seq=excluded.through_seq,
                    quality=excluded.quality",
                (
                    metric,
                    METRIC_DEFINITION_VERSION,
                    resolution,
                    bucket_start,
                    bucket_count,
                    bucket_sum,
                    bucket_min,
                    bucket_max,
                    through_seq,
                    bucket_quality,
                ),
            )
            .map_err(history_error)?;
    }
    Ok(())
}

fn metric_quality_at(
    transaction: &rusqlite::Transaction<'_>,
    observed_at_ms: i64,
) -> StudioResult<&'static str> {
    let has_gap: i64 = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM coverage_intervals
                WHERE (from_ms IS NULL OR from_ms<=?1)
                  AND (to_ms IS NULL OR to_ms>=?1)
            )",
            [observed_at_ms],
            |row| row.get(0),
        )
        .map_err(history_error)?;
    Ok(if has_gap != 0 { "partial" } else { "complete" })
}

fn min_option(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn max_option(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn record_operation_admission(
    connection: &mut Connection,
    context: &OperationContext,
) -> StudioResult<OperationAdmissionReceipt> {
    ensure_admission_capacity(connection)?;
    let now = now_ms()?;
    let operation_id = context.operation_id().to_string();
    let subject_id = context.subject_id().to_string();
    let transaction = connection.unchecked_transaction().map_err(history_error)?;

    transaction
        .execute(
            "INSERT INTO subjects(subject_id, kind, incarnation_id, first_observed_ms)
         VALUES (?1, ?2, ?1, ?3)",
            (&subject_id, context.subject_kind().as_db(), now),
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "INSERT INTO operations(
            operation_id, run_id, subject_id, action_code, actor_kind, admitted_at_ms,
            effect_status, audit_status
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'not_dispatched', 'complete')",
            (
                &operation_id,
                context.run_id().map(|value| value.to_string()),
                &subject_id,
                context.action().as_db(),
                context.actor().as_db(),
                now,
            ),
        )
        .map_err(history_error)?;

    let requested_payload = OperationEventPayload::new(
        context.action(),
        context.actor(),
        "not_dispatched",
        "complete",
        None,
        false,
    )?;
    let requested_seq = insert_operation_event(
        &transaction,
        context,
        &requested_payload,
        OperationEventSpec {
            source_ordinal: 0,
            name: "operation.requested",
            category: "intent",
            disposition: "requested",
            observed_at_ms: now,
        },
    )?;
    debug_assert!(requested_seq > 0);

    let admitted_payload = OperationEventPayload::new(
        context.action(),
        context.actor(),
        "not_dispatched",
        "complete",
        None,
        false,
    )?;
    let admitted_seq = insert_operation_event(
        &transaction,
        context,
        &admitted_payload,
        OperationEventSpec {
            source_ordinal: 1,
            name: "operation.admitted",
            category: "admission",
            disposition: "admitted",
            observed_at_ms: now,
        },
    )?;

    let admitted_seq_i64 = i64::try_from(admitted_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;
    transaction
        .execute(
            "UPDATE operations SET admitted_seq=?2 WHERE operation_id=?1",
            (&operation_id, admitted_seq_i64),
        )
        .map_err(history_error)?;

    transaction.commit().map_err(history_error)?;
    Ok(OperationAdmissionReceipt {
        operation_id: context.operation_id(),
        admitted_seq,
    })
}

fn record_operation_terminal(
    connection: &mut Connection,
    context: &OperationContext,
    outcome: OperationOutcome,
    error_code: Option<&str>,
) -> StudioResult<OperationTerminalReceipt> {
    if let Some(code) = error_code {
        events::validate_error_code(code)?;
    }

    let now = now_ms()?;
    let operation_id = context.operation_id().to_string();
    let transaction = connection.unchecked_transaction().map_err(history_error)?;

    let terminal_already_present: i64 = transaction
        .query_row(
            "SELECT CASE WHEN terminal_seq IS NULL THEN 0 ELSE 1 END
             FROM operations WHERE operation_id=?1",
            [&operation_id],
            |row| row.get(0),
        )
        .map_err(history_error)?;
    if terminal_already_present != 0 {
        return Err(StudioError::History(
            "history operation already has terminal evidence".into(),
        ));
    }

    let payload = OperationEventPayload::new(
        context.action(),
        context.actor(),
        outcome.effect_status(),
        "complete",
        error_code.map(ToOwned::to_owned),
        false,
    )?;
    let name = if outcome == OperationOutcome::Indeterminate {
        "operation.indeterminate"
    } else {
        "operation.finished"
    };
    let terminal_seq = insert_operation_event(
        &transaction,
        context,
        &payload,
        OperationEventSpec {
            source_ordinal: 2,
            name,
            category: "outcome",
            disposition: outcome.audit_disposition(),
            observed_at_ms: now,
        },
    )?;
    let terminal_seq_i64 = i64::try_from(terminal_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;

    let changed = transaction
        .execute(
            "UPDATE operations
         SET terminal_seq=?2, effect_status=?3, audit_status='complete', error_code=?4
         WHERE operation_id=?1 AND terminal_seq IS NULL",
            (
                &operation_id,
                terminal_seq_i64,
                outcome.effect_status(),
                error_code,
            ),
        )
        .map_err(history_error)?;
    if changed != 1 {
        return Err(StudioError::History(
            "history operation terminal update lost its admission".into(),
        ));
    }

    transaction.commit().map_err(history_error)?;
    Ok(OperationTerminalReceipt {
        operation_id: context.operation_id(),
        terminal_seq,
    })
}

struct HistoryEventInsert<'a> {
    run_id: Option<&'a str>,
    subject_id: &'a str,
    operation_id: Option<&'a str>,
    source_stream: &'a str,
    source_ordinal: i64,
    name: &'a str,
    category: &'a str,
    observed_at_ms: i64,
    evidence_kind: &'a str,
    payload_json: &'a str,
    retention_class: &'a str,
}

fn insert_history_event(
    transaction: &rusqlite::Transaction<'_>,
    spec: HistoryEventInsert<'_>,
) -> StudioResult<u64> {
    if spec.source_stream.is_empty()
        || spec.source_stream.len() > 256
        || spec.name.is_empty()
        || spec.name.len() > 96
        || spec.payload_json.len() > 4096
    {
        return Err(StudioError::History(
            "history event exceeds a bounded field".into(),
        ));
    }
    let event_id = uuid::Uuid::new_v4().to_string();
    let digest = format!("{:x}", Sha256::digest(spec.payload_json.as_bytes()));
    transaction
        .execute(
            "INSERT INTO events(
                event_id, run_id, subject_id, operation_id, source_stream, source_ordinal,
                name, category, observed_at_ms, time_quality, evidence_kind,
                payload_version, payload_json, payload_sha256, retention_class
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'local', ?10, 1, ?11, ?12, ?13)",
            (
                &event_id,
                spec.run_id,
                spec.subject_id,
                spec.operation_id,
                spec.source_stream,
                spec.source_ordinal,
                spec.name,
                spec.category,
                spec.observed_at_ms,
                spec.evidence_kind,
                spec.payload_json,
                &digest,
                spec.retention_class,
            ),
        )
        .map_err(history_error)?;
    u64::try_from(transaction.last_insert_rowid())
        .map_err(|_| StudioError::History("history sequence overflow".into()))
}

fn record_validated_journal(
    connection: &mut Connection,
    domain: &str,
    transaction_id: &str,
    revision: u64,
    payload_sha256: &str,
    phase: &str,
) -> StudioResult<()> {
    if !matches!(domain, "studio" | "tunnel")
        || transaction_id.is_empty()
        || transaction_id.len() > 160
        || phase.is_empty()
        || phase.len() > 64
        || !valid_sha256(payload_sha256)
    {
        return Err(StudioError::History(
            "invalid validated journal observation".into(),
        ));
    }
    let revision_i64 = i64::try_from(revision)
        .map_err(|_| StudioError::History("journal revision overflow".into()))?;
    let now = now_ms()?;
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    let current: Option<(i64, String)> = transaction
        .query_row(
            "SELECT highest_revision,payload_sha256
             FROM journal_watermarks WHERE domain=?1 AND transaction_id=?2",
            (domain, transaction_id),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(history_error)?;

    if let Some((highest, digest)) = current {
        if revision_i64 < highest {
            transaction.rollback().map_err(history_error)?;
            return Ok(());
        }
        if revision_i64 == highest {
            if digest == payload_sha256 {
                transaction.rollback().map_err(history_error)?;
                return Ok(());
            }
            return Err(StudioError::History(
                "validated journal revision digest conflict".into(),
            ));
        }
        if revision_i64 > highest.saturating_add(1) {
            transaction
                .execute(
                    "INSERT INTO coverage_intervals(
                        coverage_id, reason, lost_count, completeness
                     ) VALUES (?1,'revision_gap',?2,'partial')",
                    (
                        uuid::Uuid::new_v4().to_string(),
                        revision_i64.saturating_sub(highest).saturating_sub(1),
                    ),
                )
                .map_err(history_error)?;
        }
    }

    let subject_id = format!("{domain}-journal");
    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id,kind,component_code,incarnation_id,first_observed_ms
             ) VALUES (?1,'component',?2,?1,?3)",
            (&subject_id, domain, now),
        )
        .map_err(history_error)?;
    let payload_json = serde_json::json!({
        "domain": domain,
        "transaction_id": transaction_id,
        "revision": revision.to_string(),
        "phase": phase,
    })
    .to_string();
    let source_stream = format!("journal/{domain}/{transaction_id}");
    let seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: None,
            subject_id: &subject_id,
            operation_id: None,
            source_stream: &source_stream,
            source_ordinal: revision_i64,
            name: "update.journal.observed",
            category: "recovery",
            observed_at_ms: now,
            evidence_kind: "validated_journal",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    transaction
        .execute(
            "INSERT INTO journal_watermarks(
                domain,transaction_id,highest_revision,payload_sha256,observed_seq,last_seen_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(domain,transaction_id) DO UPDATE SET
                highest_revision=excluded.highest_revision,
                payload_sha256=excluded.payload_sha256,
                observed_seq=excluded.observed_seq,
                last_seen_at_ms=excluded.last_seen_at_ms",
            (
                domain,
                transaction_id,
                revision_i64,
                payload_sha256,
                i64::try_from(seq)
                    .map_err(|_| StudioError::History("history sequence overflow".into()))?,
                now,
            ),
        )
        .map_err(history_error)?;
    transaction.commit().map_err(history_error)
}

fn record_config_revision(
    connection: &mut Connection,
    observation: &ConfigRevisionObservation,
    registry: Option<&RegistryEntryProjection>,
) -> StudioResult<()> {
    if observation.surface_code.is_empty()
        || observation.surface_code.len() > 64
        || !valid_sha256(&observation.safe_projection_sha256)
        || observation
            .exact_bytes_sha256
            .as_deref()
            .is_some_and(|digest| !valid_sha256(digest))
        || observation.change_categories_json.len() > 1024
    {
        return Err(StudioError::History(
            "invalid bounded configuration revision".into(),
        ));
    }

    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    insert_config_revision(&transaction, observation, registry)?;
    transaction.commit().map_err(history_error)
}

fn insert_config_revision(
    transaction: &rusqlite::Transaction<'_>,
    observation: &ConfigRevisionObservation,
    registry: Option<&RegistryEntryProjection>,
) -> StudioResult<()> {
    let now = now_ms()?;
    let subject_id = registry
        .map(|value| value.subject_id.clone())
        .or_else(|| observation.run_id.map(|value| value.to_string()))
        .unwrap_or_else(|| format!("config-{}", uuid::Uuid::new_v4()));
    let subject_kind = if registry.is_some() {
        "registry"
    } else {
        "system"
    };
    let incarnation_id = registry
        .map(|value| value.registry_key_digest.clone())
        .unwrap_or_else(|| subject_id.clone());

    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, registry_key_digest, incarnation_id, first_observed_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                &subject_id,
                subject_kind,
                registry.map(|value| value.registry_key_digest.as_str()),
                &incarnation_id,
                now,
            ),
        )
        .map_err(history_error)?;

    let previous_revision: Option<String> = transaction
        .query_row(
            "SELECT revision_id FROM config_revisions
             WHERE surface_code=?1 ORDER BY observed_seq DESC LIMIT 1",
            [&observation.surface_code],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?;

    let payload_json = serde_json::json!({
        "surface": observation.surface_code,
        "provenance": observation.provenance.as_db(),
        "change_categories": serde_json::from_str::<serde_json::Value>(
            &observation.change_categories_json
        ).unwrap_or(serde_json::Value::Array(vec![])),
    })
    .to_string();
    let operation_id = observation.operation_id.map(|value| value.to_string());
    let run_id = observation.run_id.map(|value| value.to_string());
    let event_seq = insert_history_event(
        transaction,
        HistoryEventInsert {
            run_id: run_id.as_deref(),
            subject_id: &subject_id,
            operation_id: operation_id.as_deref(),
            source_stream: &format!(
                "config/{}/{}",
                observation.surface_code, observation.revision_id
            ),
            source_ordinal: 0,
            name: "config.revision.observed",
            category: "observation",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    let event_seq = i64::try_from(event_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;

    transaction
        .execute(
            "INSERT INTO config_revisions(
                revision_id, surface_code, exact_bytes_sha256, safe_projection_sha256,
                schema_code, observed_seq, observed_at_ms, provenance,
                change_categories_json, previous_revision_id, boundary_reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            (
                &observation.revision_id,
                &observation.surface_code,
                observation.exact_bytes_sha256.as_deref(),
                &observation.safe_projection_sha256,
                observation.schema_code.as_deref(),
                event_seq,
                now,
                observation.provenance.as_db(),
                &observation.change_categories_json,
                previous_revision.as_deref(),
            ),
        )
        .map_err(history_error)?;

    transaction
        .execute(
            "INSERT INTO config_links(
                link_id, event_seq, revision_id, run_id, attempt_id, relation
             ) VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            (
                uuid::Uuid::new_v4().to_string(),
                event_seq,
                &observation.revision_id,
                run_id.as_deref(),
                observation.relation,
            ),
        )
        .map_err(history_error)?;

    if let Some(registry) = registry {
        transaction
            .execute(
                "INSERT INTO registry_entry_revisions(
                    event_seq, subject_id, registry_revision_id, present, enabled, runtime_kind
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    event_seq,
                    &registry.subject_id,
                    &observation.revision_id,
                    i64::from(registry.present),
                    registry.enabled.map(i64::from),
                    registry.runtime_kind,
                ),
            )
            .map_err(history_error)?;
    }

    Ok(())
}

fn record_drift_observation(
    connection: &mut Connection,
    observation: &DriftObservation,
) -> StudioResult<()> {
    if observation.surfaces.len() > 32
        || observation
            .surfaces
            .iter()
            .any(|surface| surface.is_empty() || surface.len() > 64)
    {
        return Err(StudioError::History(
            "drift surface set exceeds bounded contract".into(),
        ));
    }
    let generation = i64::try_from(observation.generation)
        .map_err(|_| StudioError::History("drift generation overflow".into()))?;
    let now = now_ms()?;
    let subject_id = "fleet-runtime";
    let run_id = observation.run_id.map(|value| value.to_string());
    let operation_id = observation.operation_id.to_string();
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, incarnation_id, first_observed_ms
             ) VALUES (?1, 'fleet', ?1, ?2)",
            (subject_id, now),
        )
        .map_err(history_error)?;

    let payload_json = serde_json::json!({
        "state": observation.state,
        "phase": observation.phase,
        "generation": observation.generation.to_string(),
        "studio_config_activation": observation.studio_config_activation,
        "client_freshness": observation.client_freshness,
        "rollback_outcome": observation.rollback_outcome,
        "observation_kind": observation.observation_kind,
        "surfaces": observation.surfaces,
    })
    .to_string();
    let event_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: run_id.as_deref(),
            subject_id,
            operation_id: Some(&operation_id),
            source_stream: &observation.source_stream,
            source_ordinal: 0,
            name: "fleet.drift.observed",
            category: "observation",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    let event_seq = i64::try_from(event_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;

    transaction
        .execute(
            "INSERT INTO drift_observations(
                observation_id, event_seq, operation_id, host_key_digest, generation,
                manifest_digest, state, phase, studio_config_activation, client_freshness,
                rollback_outcome, observation_kind
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            (
                &observation.observation_id,
                event_seq,
                &operation_id,
                observation.host_key_digest.as_deref(),
                generation,
                observation.manifest_digest.as_deref(),
                observation.state,
                observation.phase,
                observation.studio_config_activation,
                observation.client_freshness,
                observation.rollback_outcome,
                observation.observation_kind,
            ),
        )
        .map_err(history_error)?;

    for surface in &observation.surfaces {
        transaction
            .execute(
                "INSERT INTO drift_surfaces(
                    observation_id, surface_code, desired_revision_id,
                    active_revision_id, changed
                 ) VALUES (?1, ?2, NULL, NULL, 1)",
                (&observation.observation_id, surface),
            )
            .map_err(history_error)?;
    }

    transaction.commit().map_err(history_error)
}

fn record_artifact_projection(
    transaction: &rusqlite::Transaction<'_>,
    observation: &UpdateTransactionObservation,
    event_seq: i64,
    observed_at_ms: i64,
) -> StudioResult<()> {
    let Some(artifact) = observation.artifact.as_ref() else {
        return Ok(());
    };
    if observation.phase != "staged"
        || artifact.component.to_string() != observation.component_code
        || !valid_sha256(&artifact.archive_sha256)
        || artifact.member_sha256.len() > 16
        || artifact
            .member_sha256
            .iter()
            .any(|digest| !valid_sha256(digest))
        || artifact.platform_code.is_empty()
        || artifact.platform_code.len() > 64
    {
        return Err(StudioError::History(
            "invalid staged artifact history identity".into(),
        ));
    }

    let artifact_key = format!(
        "{}:{}:{}",
        observation.component_code, artifact.archive_sha256, artifact.platform_code
    );
    let artifact_id = format!(
        "artifact-{}",
        &format!("{:x}", Sha256::digest(artifact_key.as_bytes()))[..32]
    );
    transaction
        .execute(
            "INSERT OR IGNORE INTO artifacts(
                artifact_id,component_code,version,provider_code,platform_code,
                archive_sha256,content_sha256,evidence_scope,first_observed_seq
             ) VALUES (?1,?2,?3,?4,?5,?6,?6,'verified_stage',?7)",
            (
                &artifact_id,
                &observation.component_code,
                &artifact.version,
                artifact.provider_code,
                &artifact.platform_code,
                &artifact.archive_sha256,
                event_seq,
            ),
        )
        .map_err(history_error)?;

    for (ordinal, digest) in artifact.member_sha256.iter().enumerate() {
        transaction
            .execute(
                "INSERT OR IGNORE INTO artifact_members(
                    artifact_id,member_role,ordinal,sha256
                 ) VALUES (?1,'executable',?2,?3)",
                (
                    &artifact_id,
                    i64::try_from(ordinal)
                        .map_err(|_| StudioError::History("artifact member overflow".into()))?,
                    digest,
                ),
            )
            .map_err(history_error)?;
    }

    transaction
        .execute(
            "UPDATE install_observations
             SET artifact_id=?1
             WHERE attempt_id=?2 AND state='verified_active' AND artifact_id IS NULL",
            (&artifact_id, &observation.attempt_id),
        )
        .map_err(history_error)?;

    let previous: Option<String> = transaction
        .query_row(
            "SELECT CASE
                WHEN i.attempt_id=?2 THEN i.previous_observation_id
                ELSE h.latest_observation_id
             END
             FROM observed_lineage_heads h
             LEFT JOIN install_observations i
               ON i.observation_id=h.latest_observation_id
             WHERE h.subject_id=?1",
            (&observation.subject_id, &observation.attempt_id),
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?
        .flatten();
    transaction
        .execute(
            "INSERT INTO install_observations(
                observation_id,event_seq,subject_id,attempt_id,artifact_id,
                layout_kind,state,proof_scope,previous_observation_id,
                boundary_reason,observed_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,'staged','artifact_probe',?7,NULL,?8)",
            (
                uuid::Uuid::new_v4().to_string(),
                event_seq,
                &observation.subject_id,
                &observation.attempt_id,
                &artifact_id,
                layout_kind_for_domain(observation.domain),
                previous.as_deref(),
                observed_at_ms,
            ),
        )
        .map_err(history_error)?;
    Ok(())
}

fn layout_kind_for_domain(domain: &str) -> &'static str {
    match domain {
        "mcp" => "flat_bin",
        "studio" | "tunnel" => "versioned_release",
        "gateway" | "fleet" => "control_bundle",
        _ => "legacy_flat",
    }
}

fn lineage_artifact_id(
    transaction: &rusqlite::Transaction<'_>,
    observation: &UpdateTransactionObservation,
    restored: bool,
) -> StudioResult<Option<String>> {
    if restored {
        return transaction
            .query_row(
                "SELECT i.artifact_id
                 FROM observed_lineage_heads h
                 JOIN install_observations i
                   ON i.observation_id=h.last_verified_good_id
                 WHERE h.subject_id=?1",
                [&observation.subject_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(history_error)
            .map(Option::flatten);
    }
    transaction
        .query_row(
            "SELECT artifact_id FROM install_observations
             WHERE attempt_id=?1 AND state='staged' AND artifact_id IS NOT NULL
             ORDER BY event_seq DESC LIMIT 1",
            [&observation.attempt_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)
        .map(Option::flatten)
}

fn update_phase_rank(phase: &str) -> u8 {
    match phase {
        "preparing" => 10,
        "staged" => 20,
        "stopping" | "gateway_stopping" => 30,
        "activating" | "gateway_restarting" | "activation_pending" => 40,
        "starting" | "gateway_reconnecting" | "external_activating" => 50,
        "verifying" | "gateway_catalog_verifying" | "health_verifying" => 60,
        "rolling_back" => 70,
        "completed" | "rolled_back" | "failed" | "rollback_failed" => 80,
        _ => 0,
    }
}

fn record_update_transaction(
    connection: &mut Connection,
    run_id: Option<uuid::Uuid>,
    observation: &UpdateTransactionObservation,
) -> StudioResult<()> {
    if observation.transaction_id.is_empty()
        || observation.transaction_id.len() > 160
        || observation.component_code.len() > 64
    {
        return Err(StudioError::History(
            "invalid bounded update history identity".into(),
        ));
    }

    let now = now_ms()?;
    let run_id = run_id.map(|value| value.to_string());
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, component_code, incarnation_id, first_observed_ms
             ) VALUES (?1, 'component', ?2, ?1, ?3)",
            (&observation.subject_id, &observation.component_code, now),
        )
        .map_err(history_error)?;

    let prior: Option<(String, String, i64, i64)> = transaction
        .query_row(
            "SELECT target_version, last_phase, last_source_ordinal,
                    COALESCE((
                        SELECT MAX(source_revision)
                        FROM update_phases
                        WHERE attempt_id=update_attempts.attempt_id
                    ), -1)
             FROM update_attempts WHERE attempt_id=?1",
            [&observation.attempt_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(history_error)?;

    if let Some((target_version, last_phase, _, _)) = &prior {
        if target_version != &observation.target_version {
            return Err(StudioError::History(
                "conflicting update attempt target identity".into(),
            ));
        }
        if last_phase == observation.phase {
            update_attempt_operation_link(&transaction, observation)?;
            transaction.commit().map_err(history_error)?;
            return Ok(());
        }
    }

    let source_ordinal = prior
        .as_ref()
        .map(|(_, _, ordinal, _)| ordinal.saturating_add(1))
        .unwrap_or(0);
    let source_revision = i64::try_from(observation.source_revision)
        .map_err(|_| StudioError::History("update source revision overflow".into()))?;
    let projection_is_newer = prior
        .as_ref()
        .map(|(_, last_phase, _, last_revision)| {
            source_revision > *last_revision
                || (source_revision == *last_revision
                    && update_phase_rank(observation.phase) >= update_phase_rank(last_phase))
        })
        .unwrap_or(true);
    let source_stream = format!(
        "update/{}/{}",
        observation.domain, observation.transaction_id
    );
    let payload_json = serde_json::json!({
        "component": observation.component_code,
        "domain": observation.domain,
        "transaction_id": observation.transaction_id,
        "source_version": observation.source_version,
        "target_version": observation.target_version,
        "phase": observation.phase,
        "was_running": observation.was_running,
        "install_outcome": observation.install_outcome,
        "rollback_outcome": observation.rollback_outcome,
        "verification_scope": observation.verification_scope,
        "error_code": observation.error_code,
    })
    .to_string();
    let operation_id = observation.operation_id.map(|id| id.to_string());
    let name = match observation.phase {
        "preparing" => "update.prepare.started",
        "staged" => "update.artifact.staged",
        "completed" => "update.install.verified",
        "rolled_back" => "update.rollback.verified",
        "rollback_failed" => "update.rollback.failed",
        "failed" => "update.install.failed",
        "rolling_back" => "update.rollback.started",
        _ => "update.phase.changed",
    };
    let event_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: run_id.as_deref(),
            subject_id: &observation.subject_id,
            operation_id: operation_id.as_deref(),
            source_stream: &source_stream,
            source_ordinal,
            name,
            category: if observation.terminal {
                "outcome"
            } else {
                "transition"
            },
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    let event_seq = i64::try_from(event_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;
    let was_running = observation.was_running.map(i64::from);

    if prior.is_none() {
        let (prepare_operation_id, apply_operation_id) = match observation.operation_kind {
            OperationKind::Prepare => (operation_id.as_deref(), None),
            OperationKind::Apply => (None, operation_id.as_deref()),
            OperationKind::Observation => (None, None),
        };
        transaction
            .execute(
                "INSERT INTO update_attempts(
                    attempt_id, subject_id, transaction_id, domain, originating_run_id,
                    prepare_operation_id, apply_operation_id, source_version, target_version,
                    first_observed_seq, apply_started_seq, terminal_seq, last_phase,
                    last_source_ordinal, install_outcome, rollback_outcome, verification_scope,
                    was_running, completeness
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, 'complete'
                 )",
                rusqlite::params![
                    &observation.attempt_id,
                    &observation.subject_id,
                    &observation.transaction_id,
                    observation.domain,
                    run_id.as_deref(),
                    prepare_operation_id,
                    apply_operation_id,
                    observation.source_version.as_deref(),
                    &observation.target_version,
                    event_seq,
                    (observation.operation_kind == OperationKind::Apply).then_some(event_seq),
                    observation.terminal.then_some(event_seq),
                    observation.phase,
                    source_ordinal,
                    observation.install_outcome,
                    observation.rollback_outcome,
                    observation.verification_scope,
                    was_running,
                ],
            )
            .map_err(history_error)?;
    } else {
        let prepare_operation_id = (observation.operation_kind == OperationKind::Prepare)
            .then(|| operation_id.clone())
            .flatten();
        let apply_operation_id = (observation.operation_kind == OperationKind::Apply)
            .then(|| operation_id.clone())
            .flatten();
        transaction
            .execute(
                "UPDATE update_attempts SET
                    prepare_operation_id=COALESCE(prepare_operation_id, ?2),
                    apply_operation_id=COALESCE(apply_operation_id, ?3),
                    source_version=COALESCE(source_version, ?4),
                    apply_started_seq=CASE
                        WHEN apply_started_seq IS NULL AND ?5=1 THEN ?6
                        ELSE apply_started_seq END,
                    terminal_seq=CASE
                        WHEN terminal_seq IS NULL AND ?7=1 THEN ?6
                        ELSE terminal_seq END,
                    last_phase=CASE WHEN ?14=1 THEN ?8 ELSE last_phase END,
                    last_source_ordinal=?9,
                    install_outcome=CASE WHEN ?14=1 THEN ?10 ELSE install_outcome END,
                    rollback_outcome=CASE WHEN ?14=1 THEN ?11 ELSE rollback_outcome END,
                    verification_scope=CASE WHEN ?14=1 THEN ?12 ELSE verification_scope END,
                    was_running=CASE
                        WHEN ?14=1 THEN COALESCE(?13, was_running)
                        ELSE was_running END
                 WHERE attempt_id=?1",
                (
                    &observation.attempt_id,
                    prepare_operation_id.as_deref(),
                    apply_operation_id.as_deref(),
                    observation.source_version.as_deref(),
                    i64::from(observation.operation_kind == OperationKind::Apply),
                    event_seq,
                    i64::from(observation.terminal),
                    observation.phase,
                    source_ordinal,
                    observation.install_outcome,
                    observation.rollback_outcome,
                    observation.verification_scope,
                    was_running,
                    i64::from(projection_is_newer),
                ),
            )
            .map_err(history_error)?;
    }

    record_artifact_projection(&transaction, observation, event_seq, now)?;
    transaction
        .execute(
            "INSERT INTO update_phases(
                event_seq, attempt_id, phase, source_revision, error_code, owner_verified
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                event_seq,
                &observation.attempt_id,
                observation.phase,
                source_revision,
                observation.error_code,
                i64::from(observation.owner_verified),
            ),
        )
        .map_err(history_error)?;
    record_lineage_projection(&transaction, observation, event_seq, now)?;
    transaction.commit().map_err(history_error)
}

fn record_lineage_projection(
    transaction: &rusqlite::Transaction<'_>,
    observation: &UpdateTransactionObservation,
    event_seq: i64,
    observed_at_ms: i64,
) -> StudioResult<()> {
    let restored = observation.rollback_outcome == "succeeded";
    let verified_active = observation.install_outcome == "succeeded";
    if !restored && !verified_active {
        return Ok(());
    }

    let previous: Option<String> = transaction
        .query_row(
            "SELECT latest_observation_id FROM observed_lineage_heads WHERE subject_id=?1",
            [&observation.subject_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?
        .flatten();

    let observation_id = uuid::Uuid::new_v4().to_string();
    let artifact_id = lineage_artifact_id(transaction, observation, restored)?;
    transaction
        .execute(
            "INSERT INTO install_observations(
                observation_id, event_seq, subject_id, attempt_id, artifact_id,
                layout_kind, state, proof_scope, previous_observation_id,
                boundary_reason, observed_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                &observation_id,
                event_seq,
                &observation.subject_id,
                &observation.attempt_id,
                artifact_id.as_deref(),
                layout_kind_for_domain(observation.domain),
                if restored {
                    "restored"
                } else {
                    "verified_active"
                },
                observation.verification_scope,
                previous.as_deref(),
                previous.is_none().then_some("unknown_predecessor"),
                observed_at_ms,
            ),
        )
        .map_err(history_error)?;

    transaction
        .execute(
            "INSERT INTO observed_lineage_heads(
                subject_id, latest_observation_id, previous_observation_id,
                last_verified_good_id, as_of_seq
             ) VALUES (?1, ?2, ?3, ?2, ?4)
             ON CONFLICT(subject_id) DO UPDATE SET
                previous_observation_id=observed_lineage_heads.latest_observation_id,
                latest_observation_id=excluded.latest_observation_id,
                last_verified_good_id=excluded.last_verified_good_id,
                as_of_seq=excluded.as_of_seq",
            (
                &observation.subject_id,
                &observation_id,
                previous.as_deref(),
                event_seq,
            ),
        )
        .map_err(history_error)?;
    Ok(())
}

fn update_attempt_operation_link(
    transaction: &rusqlite::Transaction<'_>,
    observation: &UpdateTransactionObservation,
) -> StudioResult<()> {
    let Some(operation_id) = observation.operation_id else {
        return Ok(());
    };
    let sql = match observation.operation_kind {
        OperationKind::Prepare => {
            "UPDATE update_attempts SET prepare_operation_id=COALESCE(prepare_operation_id, ?2)
             WHERE attempt_id=?1"
        }
        OperationKind::Apply => {
            "UPDATE update_attempts SET apply_operation_id=COALESCE(apply_operation_id, ?2)
             WHERE attempt_id=?1"
        }
        OperationKind::Observation => return Ok(()),
    };
    transaction
        .execute(sql, (&observation.attempt_id, operation_id.to_string()))
        .map_err(history_error)?;
    Ok(())
}

fn record_update_check(
    connection: &mut Connection,
    run_id: Option<uuid::Uuid>,
    observation: &UpdateCheckObservation,
) -> StudioResult<()> {
    let now = now_ms()?;
    let run_id = run_id.map(|value| value.to_string());
    let operation_id = observation.operation_id.to_string();
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, component_code, incarnation_id, first_observed_ms
             ) VALUES (?1, 'component', ?2, ?1, ?3)",
            (&observation.subject_id, &observation.component_code, now),
        )
        .map_err(history_error)?;
    let payload_json = serde_json::json!({
        "component": observation.component_code,
        "outcome": observation.check_outcome,
        "latest_version": observation.latest_version,
        "installed_version": observation.installed_version,
        "update_available": observation.update_available,
        "error_code": observation.error_code,
    })
    .to_string();
    let event_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: run_id.as_deref(),
            subject_id: &observation.subject_id,
            operation_id: Some(&operation_id),
            source_stream: &observation.source_stream,
            source_ordinal: 0,
            name: "update.check.component.finished",
            category: "observation",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    transaction
        .execute(
            "INSERT INTO update_checks(
                event_seq, operation_id, subject_id, check_outcome, latest_version,
                installed_version_observed, update_available, error_code
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                i64::try_from(event_seq)
                    .map_err(|_| StudioError::History("history sequence overflow".into()))?,
                &operation_id,
                &observation.subject_id,
                observation.check_outcome,
                observation.latest_version.as_deref(),
                observation.installed_version.as_deref(),
                observation.update_available.map(i64::from),
                observation.error_code,
            ),
        )
        .map_err(history_error)?;
    transaction.commit().map_err(history_error)
}

fn mark_interrupted_previous_runs(
    transaction: &rusqlite::Transaction<'_>,
    observed_at_ms: i64,
) -> StudioResult<()> {
    let mut run_stmt = transaction
        .prepare(
            "SELECT r.run_id,
                    COALESCE(MAX(e.seq), r.started_seq, 0),
                    COALESCE(MAX(e.observed_at_ms), r.last_seen_at_ms)
             FROM studio_runs r
             LEFT JOIN events e ON e.run_id=r.run_id
             WHERE r.end_state='open'
             GROUP BY r.run_id, r.started_seq, r.last_seen_at_ms",
        )
        .map_err(history_error)?;
    let old_runs = run_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    drop(run_stmt);

    for (old_run_id, last_seq, last_observed_ms) in old_runs {
        transaction
            .execute(
                "INSERT INTO coverage_intervals(
                    coverage_id,run_id,reason,from_seq,from_ms,to_ms,completeness
                 ) VALUES (?1,?2,'interrupted',?3,?4,?5,'unknown')",
                (
                    uuid::Uuid::new_v4().to_string(),
                    &old_run_id,
                    last_seq,
                    last_observed_ms,
                    observed_at_ms,
                ),
            )
            .map_err(history_error)?;
        let mut operation_stmt = transaction
            .prepare(
                "SELECT operation_id,subject_id,actor_kind,action_code
                 FROM operations
                 WHERE run_id=?1 AND terminal_seq IS NULL
                 ORDER BY operation_id",
            )
            .map_err(history_error)?;
        let operations = operation_stmt
            .query_map([&old_run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(history_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(history_error)?;
        drop(operation_stmt);

        for (operation_id, subject_id, actor_kind, action_code) in operations {
            let payload_json = serde_json::json!({
                "action": action_code,
                "actor": actor_kind,
                "effect_status": "indeterminate",
                "audit_status": "complete",
                "error_code": "observer_interrupted",
                "terminal": true,
            })
            .to_string();
            let source_stream = format!("operation/{operation_id}");
            let terminal_seq = insert_history_event(
                transaction,
                HistoryEventInsert {
                    run_id: Some(&old_run_id),
                    subject_id: &subject_id,
                    operation_id: Some(&operation_id),
                    source_stream: &source_stream,
                    source_ordinal: 2,
                    name: "operation.indeterminate",
                    category: "outcome",
                    observed_at_ms,
                    evidence_kind: "recovery_observation",
                    payload_json: &payload_json,
                    retention_class: "audit",
                },
            )?;
            let terminal_seq_i64 = i64::try_from(terminal_seq)
                .map_err(|_| StudioError::History("history sequence overflow".into()))?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO audit_events(
                        event_seq,operation_id,actor_kind,safety_exception,disposition
                     ) VALUES (?1,?2,?3,0,'indeterminate')",
                    (terminal_seq_i64, &operation_id, &actor_kind),
                )
                .map_err(history_error)?;
            transaction
                .execute(
                    "UPDATE operations
                     SET terminal_seq=?2,effect_status='indeterminate',
                         audit_status='complete',error_code='observer_interrupted'
                     WHERE operation_id=?1 AND terminal_seq IS NULL",
                    (&operation_id, terminal_seq_i64),
                )
                .map_err(history_error)?;
        }

        let mut attempt_stmt = transaction
            .prepare(
                "SELECT attempt_id,subject_id,domain,transaction_id,last_source_ordinal,
                        apply_operation_id
                 FROM update_attempts
                 WHERE originating_run_id=?1
                   AND terminal_seq IS NULL
                   AND apply_started_seq IS NOT NULL
                 ORDER BY attempt_id",
            )
            .map_err(history_error)?;
        let attempts = attempt_stmt
            .query_map([&old_run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(history_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(history_error)?;
        drop(attempt_stmt);

        for (attempt_id, subject_id, domain, transaction_id, last_ordinal, operation_id) in attempts
        {
            let source_stream = format!("update/{domain}/{transaction_id}");
            let source_ordinal = last_ordinal.saturating_add(1);
            let payload_json = serde_json::json!({
                "domain": domain,
                "transaction_id": transaction_id,
                "outcome": "indeterminate",
                "reason": "observer_interrupted",
            })
            .to_string();
            let terminal_seq = insert_history_event(
                transaction,
                HistoryEventInsert {
                    run_id: Some(&old_run_id),
                    subject_id: &subject_id,
                    operation_id: operation_id.as_deref(),
                    source_stream: &source_stream,
                    source_ordinal,
                    name: "update.install.indeterminate",
                    category: "recovery",
                    observed_at_ms,
                    evidence_kind: "recovery_observation",
                    payload_json: &payload_json,
                    retention_class: "operational",
                },
            )?;
            let terminal_seq_i64 = i64::try_from(terminal_seq)
                .map_err(|_| StudioError::History("history sequence overflow".into()))?;
            transaction
                .execute(
                    "UPDATE update_attempts
                     SET terminal_seq=?2,last_source_ordinal=?3,
                         install_outcome='indeterminate',
                         rollback_outcome=CASE
                             WHEN rollback_outcome='pending' THEN 'unknown'
                             ELSE rollback_outcome
                         END,
                         completeness='interrupted'
                     WHERE attempt_id=?1 AND terminal_seq IS NULL",
                    (&attempt_id, terminal_seq_i64, source_ordinal),
                )
                .map_err(history_error)?;
        }
    }

    Ok(())
}

fn record_run_start(
    connection: &mut Connection,
    run_id: uuid::Uuid,
    process_identity: &str,
    loaded_config_digest: Option<&str>,
) -> StudioResult<()> {
    let now = now_ms()?;
    let run_id = run_id.to_string();
    let process_identity_digest = format!("{:x}", Sha256::digest(process_identity.as_bytes()));
    let source_stream = format!("studio/{run_id}");
    let transaction = connection.unchecked_transaction().map_err(history_error)?;

    mark_interrupted_previous_runs(&transaction, now)?;
    transaction
        .execute(
            "UPDATE studio_runs SET end_state='interrupted' WHERE end_state='open'",
            [],
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "UPDATE runtime_sessions
             SET end_kind='interrupted', ended_at_ms=NULL, closed_seq=NULL,
                 exit_code=NULL, is_crash=NULL, exact_duration_ms=NULL
             WHERE end_kind='open'",
            [],
        )
        .map_err(history_error)?;

    transaction
        .execute(
            "INSERT INTO subjects(subject_id, kind, incarnation_id, first_observed_ms)
             VALUES (?1, 'studio', ?1, ?2)",
            (&run_id, now),
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "INSERT INTO studio_runs(
                run_id, started_at_ms, last_seen_at_ms, end_state, version,
                build_identity, process_identity_digest, loaded_config_digest
             ) VALUES (?1, ?2, ?2, 'open', ?3, ?4, ?5, ?6)",
            (
                &run_id,
                now,
                env!("CARGO_PKG_VERSION"),
                option_env!("GIT_COMMIT"),
                &process_identity_digest,
                loaded_config_digest,
            ),
        )
        .map_err(history_error)?;

    let started_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: Some(&run_id),
            subject_id: &run_id,
            operation_id: None,
            source_stream: &source_stream,
            source_ordinal: 0,
            name: "studio.run.started",
            category: "observation",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: r#"{"state":"started"}"#,
            retention_class: "operational",
        },
    )?;
    transaction
        .execute(
            "UPDATE studio_runs SET started_seq=?2 WHERE run_id=?1",
            (
                &run_id,
                i64::try_from(started_seq)
                    .map_err(|_| StudioError::History("history sequence overflow".into()))?,
            ),
        )
        .map_err(history_error)?;
    if let Some(digest) = loaded_config_digest {
        let observation = ConfigRevisionObservation::loaded_studio(
            digest,
            uuid::Uuid::parse_str(&run_id)
                .map_err(|_| StudioError::History("invalid Studio run identity".into()))?,
        );
        insert_config_revision(&transaction, &observation, None)?;
    }
    transaction.commit().map_err(history_error)
}

fn record_run_ready(connection: &mut Connection, run_id: uuid::Uuid) -> StudioResult<()> {
    let now = now_ms()?;
    let run_id = run_id.to_string();
    let source_stream = format!("studio/{run_id}");
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    let changed = transaction
        .execute(
            "UPDATE studio_runs SET ready_at_ms=?2, last_seen_at_ms=?2
             WHERE run_id=?1 AND end_state='open' AND ready_at_ms IS NULL",
            (&run_id, now),
        )
        .map_err(history_error)?;
    if changed != 1 {
        return Err(StudioError::History(
            "Studio run is not eligible for ready observation".into(),
        ));
    }
    insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: Some(&run_id),
            subject_id: &run_id,
            operation_id: None,
            source_stream: &source_stream,
            source_ordinal: 1,
            name: "studio.run.ready",
            category: "transition",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: r#"{"state":"ready"}"#,
            retention_class: "operational",
        },
    )?;
    transaction.commit().map_err(history_error)
}

fn record_run_close(
    connection: &mut Connection,
    run_id: uuid::Uuid,
    end_state: &'static str,
) -> StudioResult<()> {
    if !matches!(end_state, "clean" | "shutdown_incomplete") {
        return Err(StudioError::History("invalid Studio run end state".into()));
    }
    let now = now_ms()?;
    let run_id = run_id.to_string();
    let source_stream = format!("studio/{run_id}");
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    let payload = if end_state == "clean" {
        r#"{"state":"clean"}"#
    } else {
        r#"{"state":"shutdown_incomplete"}"#
    };
    let closed_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: Some(&run_id),
            subject_id: &run_id,
            operation_id: None,
            source_stream: &source_stream,
            source_ordinal: 2,
            name: "studio.run.closed",
            category: "outcome",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: payload,
            retention_class: "operational",
        },
    )?;
    let changed = transaction
        .execute(
            "UPDATE studio_runs
             SET ended_at_ms=?2, last_seen_at_ms=?2, end_state=?3, closed_seq=?4
             WHERE run_id=?1 AND end_state='open'",
            (
                &run_id,
                now,
                end_state,
                i64::try_from(closed_seq)
                    .map_err(|_| StudioError::History("history sequence overflow".into()))?,
            ),
        )
        .map_err(history_error)?;
    if changed != 1 {
        return Err(StudioError::History(
            "Studio run is not open during close".into(),
        ));
    }
    transaction.commit().map_err(history_error)
}

fn record_lifecycle_heartbeat(
    connection: &Connection,
    run_id: uuid::Uuid,
    session_id: uuid::Uuid,
    observed_at_ms: i64,
) -> StudioResult<()> {
    connection
        .execute(
            "UPDATE runtime_sessions
             SET last_observed_ms=CASE
                 WHEN ?3 > last_observed_ms THEN ?3 ELSE last_observed_ms END
             WHERE run_id=?1 AND session_id=?2 AND end_kind='open'",
            (run_id.to_string(), session_id.to_string(), observed_at_ms),
        )
        .map_err(history_error)?;
    Ok(())
}

fn record_lifecycle_start_failed(
    connection: &mut Connection,
    run_id: uuid::Uuid,
    owner_kind: LifecycleOwnerKind,
    generation: u64,
    subject_id: uuid::Uuid,
    source_stream: &str,
) -> StudioResult<()> {
    let now = now_ms()?;
    let run_id = run_id.to_string();
    let subject_id = subject_id.to_string();
    let generation = i64::try_from(generation)
        .map_err(|_| StudioError::History("lifecycle generation overflow".into()))?;
    let payload_json = serde_json::json!({
        "owner_kind": owner_kind,
        "generation": generation.to_string(),
        "reason": "start_failed"
    })
    .to_string();
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    transaction
        .execute(
            "INSERT INTO subjects(subject_id, kind, incarnation_id, first_observed_ms)
             VALUES (?1, ?2, ?1, ?3)",
            (&subject_id, owner_kind.subject_kind(), now),
        )
        .map_err(history_error)?;
    let seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: Some(&run_id),
            subject_id: &subject_id,
            operation_id: None,
            source_stream,
            source_ordinal: 0,
            name: match owner_kind {
                LifecycleOwnerKind::Mcp => "mcp.start.failed",
                LifecycleOwnerKind::Tunnel => "tunnel.start.failed",
            },
            category: "outcome",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    transaction
        .execute(
            "INSERT INTO lifecycle_events(
                event_seq, session_id, owner_kind, from_state, to_state, reason_code, is_crash
             ) VALUES (?1, NULL, ?2, 'starting', 'failed', 'start_failed', NULL)",
            (
                i64::try_from(seq)
                    .map_err(|_| StudioError::History("history sequence overflow".into()))?,
                owner_kind.as_db(),
            ),
        )
        .map_err(history_error)?;
    transaction.commit().map_err(history_error)
}

fn record_lifecycle_started(
    connection: &mut Connection,
    run_id: uuid::Uuid,
    context: &LifecycleSessionContext,
) -> StudioResult<()> {
    let now = now_ms()?;
    let run_id = run_id.to_string();
    let session_id = context.session_id.to_string();
    let subject_id = context.subject_id.to_string();
    let generation = i64::try_from(context.generation)
        .map_err(|_| StudioError::History("lifecycle generation overflow".into()))?;
    let pid = i64::from(context.pid);
    let payload_json = serde_json::to_string(&LifecycleEventPayload::started(context))?;
    let transaction = connection.unchecked_transaction().map_err(history_error)?;

    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, incarnation_id, first_observed_ms
             ) VALUES (?1, ?2, ?1, ?3)",
            (&subject_id, context.owner_kind.subject_kind(), now),
        )
        .map_err(history_error)?;

    let terminal: Option<(i64, i64, String, String, Option<i32>)> = transaction
        .query_row(
            "SELECT e.seq, e.observed_at_ms, e.payload_json, le.reason_code, le.is_crash
             FROM events e
             JOIN lifecycle_events le ON le.event_seq=e.seq
             WHERE e.source_stream=?1 AND e.source_ordinal=1",
            [&context.source_stream],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(history_error)?;

    let started_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: Some(&run_id),
            subject_id: &subject_id,
            operation_id: None,
            source_stream: &context.source_stream,
            source_ordinal: 0,
            name: context.owner_kind.start_event(),
            category: "transition",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    let started_seq_i64 = i64::try_from(started_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;

    if let Some((terminal_seq, terminal_at, terminal_payload, reason, is_crash)) = terminal {
        let payload: LifecycleEventPayload = serde_json::from_str(&terminal_payload)?;
        let exact_duration_ms = payload
            .exact_duration_ms
            .as_deref()
            .map(str::parse::<i64>)
            .transpose()
            .map_err(|_| StudioError::History("invalid lifecycle duration projection".into()))?;
        let observed_duration = terminal_at.saturating_sub(now).max(0);
        transaction
            .execute(
                "INSERT INTO runtime_sessions(
                    session_id, run_id, subject_id, owner_kind, generation, pid,
                    started_at_ms, started_seq, last_observed_ms, ended_at_ms, closed_seq,
                    observed_duration_ms, exact_duration_ms, end_kind, exit_code, is_crash,
                    last_source_ordinal
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 1)",
                (
                    &session_id,
                    &run_id,
                    &subject_id,
                    context.owner_kind.as_db(),
                    generation,
                    pid,
                    now,
                    started_seq_i64,
                    terminal_at,
                    terminal_seq,
                    observed_duration,
                    exact_duration_ms,
                    reason.as_str(),
                    payload.exit_code,
                    is_crash,
                ),
            )
            .map_err(history_error)?;
        transaction
            .execute(
                "UPDATE lifecycle_events SET session_id=?2 WHERE event_seq=?1",
                (terminal_seq, &session_id),
            )
            .map_err(history_error)?;
    } else {
        transaction
            .execute(
                "INSERT INTO runtime_sessions(
                    session_id, run_id, subject_id, owner_kind, generation, pid,
                    started_at_ms, started_seq, last_observed_ms, end_kind,
                    last_source_ordinal
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, 'open', 0)",
                (
                    &session_id,
                    &run_id,
                    &subject_id,
                    context.owner_kind.as_db(),
                    generation,
                    pid,
                    now,
                    started_seq_i64,
                ),
            )
            .map_err(history_error)?;
    }

    transaction
        .execute(
            "INSERT INTO lifecycle_events(
                event_seq, session_id, owner_kind, from_state, to_state, reason_code, is_crash
             ) VALUES (?1, ?2, ?3, 'starting', 'running', 'spawn_verified', NULL)",
            (started_seq_i64, &session_id, context.owner_kind.as_db()),
        )
        .map_err(history_error)?;
    transaction.commit().map_err(history_error)
}

fn record_lifecycle_terminal(
    connection: &mut Connection,
    run_id: uuid::Uuid,
    fact: &LifecycleTerminalFact,
) -> StudioResult<()> {
    let now = now_ms()?;
    let run_id = run_id.to_string();
    let session_id = fact.context.session_id.to_string();
    let subject_id = fact.context.subject_id.to_string();
    let payload_json = serde_json::to_string(&LifecycleEventPayload::ended(fact)?)?;
    let transaction = connection.unchecked_transaction().map_err(history_error)?;

    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(
                subject_id, kind, incarnation_id, first_observed_ms
             ) VALUES (?1, ?2, ?1, ?3)",
            (&subject_id, fact.context.owner_kind.subject_kind(), now),
        )
        .map_err(history_error)?;

    let terminal_seq = insert_history_event(
        &transaction,
        HistoryEventInsert {
            run_id: Some(&run_id),
            subject_id: &subject_id,
            operation_id: None,
            source_stream: &fact.context.source_stream,
            source_ordinal: 1,
            name: fact.context.owner_kind.end_event(),
            category: "outcome",
            observed_at_ms: now,
            evidence_kind: "owner",
            payload_json: &payload_json,
            retention_class: "operational",
        },
    )?;
    let terminal_seq_i64 = i64::try_from(terminal_seq)
        .map_err(|_| StudioError::History("history sequence overflow".into()))?;
    let exact_duration_ms = fact
        .exact_duration
        .map(|duration| {
            i64::try_from(duration.as_millis())
                .map_err(|_| StudioError::History("lifecycle duration overflow".into()))
        })
        .transpose()?;
    let is_crash = i64::from(fact.is_crash);
    let from_state = if fact.end_kind == LifecycleEndKind::RequestedStop {
        "stopping"
    } else {
        "running"
    };
    let to_state = if fact.is_crash { "failed" } else { "stopped" };

    let updated = transaction
        .execute(
            "UPDATE runtime_sessions
             SET ended_at_ms=?2, closed_seq=?3,
                 observed_duration_ms=CASE WHEN ?2 >= started_at_ms THEN ?2-started_at_ms ELSE 0 END,
                 exact_duration_ms=?4, end_kind=?5, exit_code=?6, is_crash=?7,
                 last_observed_ms=?2, last_source_ordinal=1
             WHERE session_id=?1 AND run_id=?8 AND end_kind='open'",
            (
                &session_id,
                now,
                terminal_seq_i64,
                exact_duration_ms,
                fact.end_kind.as_db(),
                fact.exit_code,
                is_crash,
                &run_id,
            ),
        )
        .map_err(history_error)?;

    transaction
        .execute(
            "INSERT INTO lifecycle_events(
                event_seq, session_id, owner_kind, from_state, to_state, reason_code, is_crash
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                terminal_seq_i64,
                if updated == 1 {
                    Some(session_id.as_str())
                } else {
                    None
                },
                fact.context.owner_kind.as_db(),
                from_state,
                to_state,
                fact.end_kind.reason_code(),
                is_crash,
            ),
        )
        .map_err(history_error)?;

    transaction.commit().map_err(history_error)
}

struct OperationEventSpec<'a> {
    source_ordinal: i64,
    name: &'a str,
    category: &'a str,
    disposition: &'a str,
    observed_at_ms: i64,
}

fn insert_operation_event(
    transaction: &rusqlite::Transaction<'_>,
    context: &OperationContext,
    payload: &OperationEventPayload,
    spec: OperationEventSpec<'_>,
) -> StudioResult<u64> {
    let OperationEventSpec {
        source_ordinal,
        name,
        category,
        disposition,
        observed_at_ms,
    } = spec;
    let payload_json = serde_json::to_string(payload)?;
    if payload_json.len() > 4096 {
        return Err(StudioError::History(
            "history event payload exceeds limit".into(),
        ));
    }
    let payload_sha256 = format!("{:x}", Sha256::digest(payload_json.as_bytes()));
    let event_id = uuid::Uuid::new_v4().to_string();
    let operation_id = context.operation_id().to_string();
    let subject_id = context.subject_id().to_string();

    let insert = transaction.execute(
        "INSERT INTO events(
        event_id, run_id, subject_id, operation_id, source_stream, source_ordinal,
        name, category, observed_at_ms, time_quality, evidence_kind,
        payload_version, payload_json, payload_sha256, retention_class
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'local', 'owner', 1, ?10, ?11, 'audit')",
        (
            &event_id,
            context.run_id().map(|value| value.to_string()),
            &subject_id,
            &operation_id,
            context.source_stream(),
            source_ordinal,
            name,
            category,
            observed_at_ms,
            &payload_json,
            &payload_sha256,
        ),
    );

    let seq = match insert {
        Ok(_) => u64::try_from(transaction.last_insert_rowid())
            .map_err(|_| StudioError::History("history sequence overflow".into()))?,
        Err(error) => {
            let existing: Option<(i64, String, String)> = transaction
                .query_row(
                    "SELECT seq, name, payload_sha256 FROM events
                 WHERE source_stream=?1 AND source_ordinal=?2",
                    (context.source_stream(), source_ordinal),
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(history_error)?;
            match existing {
                Some((seq, existing_name, existing_digest))
                    if existing_name == name && existing_digest == payload_sha256 =>
                {
                    u64::try_from(seq)
                        .map_err(|_| StudioError::History("history sequence overflow".into()))?
                }
                Some(_) => {
                    return Err(StudioError::History(
                        "conflicting history source ordinal".into(),
                    ));
                }
                None => return Err(history_error(error)),
            }
        }
    };

    let seq_i64 =
        i64::try_from(seq).map_err(|_| StudioError::History("history sequence overflow".into()))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO audit_events(
            event_seq, operation_id, actor_kind, safety_exception, disposition
         ) VALUES (?1, ?2, ?3, 0, ?4)",
            (seq_i64, &operation_id, context.actor().as_db(), disposition),
        )
        .map_err(history_error)?;
    Ok(seq)
}

impl HistoryHandle {
    pub fn record_bootstrap(
        &self,
        subject_id: uuid::Uuid,
        source_stream: &str,
        source_ordinal: u64,
    ) -> StudioResult<u64> {
        let result = self.record_bootstrap_inner(subject_id, source_stream, source_ordinal);
        if result.is_err() {
            self.mark_degraded("history_write_failed");
        }
        result
    }

    fn record_bootstrap_inner(
        &self,
        subject_id: uuid::Uuid,
        source_stream: &str,
        source_ordinal: u64,
    ) -> StudioResult<u64> {
        let sender = self.writer_sender()?;
        let (reply, receipt) = mpsc::sync_channel(1);
        sender
            .try_send(WriterCommand::RecordBootstrap {
                subject_id,
                source_stream: source_stream.into(),
                source_ordinal,
                reply,
            })
            .map_err(|_| StudioError::History("history writer queue is full".into()))?;
        receipt
            .recv_timeout(self.inner.receipt_timeout)
            .map_err(|_| StudioError::History("history writer receipt timed out".into()))?
    }

    fn writer_sender(&self) -> StudioResult<mpsc::SyncSender<WriterCommand>> {
        self.inner
            .workers
            .lock()
            .unwrap()
            .as_ref()
            .map(|workers| workers.writer.clone())
            .ok_or_else(|| StudioError::History("history store is unavailable".into()))
    }

    fn terminal_sender(&self) -> StudioResult<mpsc::SyncSender<TerminalCommand>> {
        self.inner
            .workers
            .lock()
            .unwrap()
            .as_ref()
            .map(|workers| workers.terminal.clone())
            .ok_or_else(|| StudioError::History("history store is unavailable".into()))
    }

    fn reader_sender(&self) -> StudioResult<mpsc::SyncSender<ReadCommand>> {
        let workers = self.inner.workers.lock().unwrap();
        let workers = workers
            .as_ref()
            .ok_or_else(|| StudioError::History("history store is unavailable".into()))?;
        if workers.readers.is_empty() {
            return Err(StudioError::History(
                "history read service is unavailable".into(),
            ));
        }
        let index = self.inner.next_reader.fetch_add(1, Ordering::Relaxed) % workers.readers.len();
        Ok(workers.readers[index].clone())
    }

    fn mark_degraded(&self, code: &'static str) {
        *self.inner.health.lock().unwrap() = HistoryHealth::Degraded { code };
    }
}

fn start_workers(
    runtime_root: &Path,
    config: HistoryConfig,
    history_events: EventHub,
) -> StudioResult<(PathBuf, WorkerSet)> {
    config.validate()?;
    let root = history_root(runtime_root);
    prepare_history_root(runtime_root, &root)?;

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let (writer_sender, writer_receiver) = mpsc::sync_channel(config.writer_capacity);
    let (terminal_sender, terminal_receiver) = mpsc::sync_channel(config.terminal_reserve);
    thread::Builder::new()
        .name("history-writer".into())
        .spawn({
            let root = root.clone();
            let max_page_count = config.max_page_count;
            move || match open_and_migrate(&root, max_page_count) {
                Ok(mut writer) => {
                    history_events.publish(StudioEvent::HistoryHealth {
                        state: "healthy".into(),
                        reason_code: None,
                        admission_available: true,
                    });
                    if ready_sender.send(Ok(())).is_err() {
                        return;
                    }
                    let mut last_notified_seq = -1_i64;
                    let mut last_notice_at = Instant::now()
                        .checked_sub(Duration::from_millis(250))
                        .unwrap_or_else(Instant::now);
                    loop {
                        while let Ok(command) = terminal_receiver.try_recv() {
                            handle_terminal_command(&mut writer.connection, command);
                        }
                        if last_notice_at.elapsed() >= Duration::from_millis(250)
                            && let Ok((db_epoch, latest_seq, retention_epoch)) =
                                history_commit_watermark(&writer.connection)
                            && latest_seq != last_notified_seq
                        {
                            history_events.publish(StudioEvent::HistoryCommitted {
                                db_epoch,
                                latest_seq: latest_seq.to_string(),
                                retention_epoch: retention_epoch.to_string(),
                            });
                            last_notified_seq = latest_seq;
                            last_notice_at = Instant::now();
                        }

                        match writer_receiver.recv_timeout(Duration::from_millis(10)) {
                            Ok(WriterCommand::Backup { destination, reply }) => {
                                let _ =
                                    reply.send(backup_database(&writer.connection, &destination));
                            }
                            Ok(WriterCommand::RecordBootstrap {
                                subject_id,
                                source_stream,
                                source_ordinal,
                                reply,
                            }) => {
                                let _ = reply.send(record_bootstrap(
                                    &mut writer.connection,
                                    subject_id,
                                    &source_stream,
                                    source_ordinal,
                                ));
                            }
                            Ok(WriterCommand::AdmitOperation { context, reply }) => {
                                let _ = reply.send(record_operation_admission(
                                    &mut writer.connection,
                                    &context,
                                ));
                            }
                            Ok(WriterCommand::StartRun {
                                run_id,
                                process_identity,
                                loaded_config_digest,
                                reply,
                            }) => {
                                let _ = reply.send(record_run_start(
                                    &mut writer.connection,
                                    run_id,
                                    &process_identity,
                                    loaded_config_digest.as_deref(),
                                ));
                            }
                            Ok(WriterCommand::MarkRunReady { run_id, reply }) => {
                                let _ = reply.send(record_run_ready(
                                    &mut writer.connection,
                                    run_id,
                                ));
                            }
                            Ok(WriterCommand::CloseRun {
                                run_id,
                                end_state,
                                reply,
                            }) => {
                                let _ = reply.send(record_run_close(
                                    &mut writer.connection,
                                    run_id,
                                    end_state,
                                ));
                            }
                            Ok(WriterCommand::ObserveLifecycleStarted { run_id, context }) => {
                                if let Err(error) = record_lifecycle_started(
                                    &mut writer.connection,
                                    run_id,
                                    &context,
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist lifecycle start observation");
                                }
                            }
                            Ok(WriterCommand::ObserveLifecycleStartFailed {
                                run_id,
                                owner_kind,
                                generation,
                                subject_id,
                                source_stream,
                            }) => {
                                if let Err(error) = record_lifecycle_start_failed(
                                    &mut writer.connection,
                                    run_id,
                                    owner_kind,
                                    generation,
                                    subject_id,
                                    &source_stream,
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist lifecycle start failure");
                                }
                            }
                            Ok(WriterCommand::ObserveLifecycleHeartbeat {
                                run_id,
                                session_id,
                                observed_at_ms,
                            }) => {
                                if let Err(error) = record_lifecycle_heartbeat(
                                    &writer.connection,
                                    run_id,
                                    session_id,
                                    observed_at_ms,
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist lifecycle heartbeat");
                                }
                            }
                            Ok(WriterCommand::ObserveUpdateTransaction {
                                run_id,
                                observation,
                            }) => {
                                if let Err(error) = record_update_transaction(
                                    &mut writer.connection,
                                    run_id,
                                    &observation,
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist update observation");
                                }
                            }
                            Ok(WriterCommand::ObserveUpdateCheck {
                                run_id,
                                observation,
                            }) => {
                                if let Err(error) = record_update_check(
                                    &mut writer.connection,
                                    run_id,
                                    &observation,
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist update check observation");
                                }
                            }
                            Ok(WriterCommand::ObserveConfigRevision {
                                observation,
                                registry,
                            }) => {
                                if let Err(error) = record_config_revision(
                                    &mut writer.connection,
                                    &observation,
                                    registry.as_ref(),
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist config revision");
                                }
                            }
                            Ok(WriterCommand::ObserveDrift { observation }) => {
                                if let Err(error) = record_drift_observation(
                                    &mut writer.connection,
                                    &observation,
                                ) {
                                    tracing::warn!(history_error = %error, "could not persist drift observation");
                                }
                            }
                            Ok(WriterCommand::ObserveGatewayEvents {
                                instance_id,
                                events,
                                reply,
                            }) => {
                                let _ = reply.send(record_gateway_events(
                                    &mut writer.connection,
                                    &instance_id,
                                    &events,
                                ));
                            }
                            Ok(WriterCommand::RecordValidatedJournal {
                                domain,
                                transaction_id,
                                revision,
                                payload_sha256,
                                phase,
                                reply,
                            }) => {
                                let _ = reply.send(record_validated_journal(
                                    &mut writer.connection,
                                    &domain,
                                    &transaction_id,
                                    revision,
                                    &payload_sha256,
                                    &phase,
                                ));
                            }
                            Ok(WriterCommand::Housekeeping { reply }) => {
                                let _ = reply.send(run_housekeeping(&mut writer.connection));
                            }
                            Ok(WriterCommand::Shutdown(done)) => {
                                while let Ok(command) = terminal_receiver.try_recv() {
                                    handle_terminal_command(&mut writer.connection, command);
                                }
                                drop(writer);
                                let _ = done.send(());
                                break;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                }
                Err(error) => {
                    let _ = ready_sender.send(Err(error));
                }
            }
        })
        .map_err(|error| StudioError::History(format!("cannot start writer: {error}")))?;

    ready_receiver
        .recv_timeout(config.startup_timeout)
        .map_err(|_| StudioError::History("history writer startup timed out".into()))??;

    let database = root.join("studio.sqlite3");
    let mut readers = Vec::with_capacity(READ_WORKER_COUNT);
    for index in 0..READ_WORKER_COUNT {
        let (reader_ready_sender, reader_ready_receiver) = mpsc::sync_channel(1);
        let (reader_sender, reader_receiver) = mpsc::sync_channel(config.reader_capacity);
        let reader_database = database.clone();
        thread::Builder::new()
            .name(format!("history-reader-{index}"))
            .spawn(move || match open_reader(&reader_database) {
                Ok(connection) => {
                    if reader_ready_sender.send(Ok(())).is_err() {
                        return;
                    }
                    while let Ok(command) = reader_receiver.recv() {
                        match command {
                            ReadCommand::Verify { full, reply } => {
                                let result = if full {
                                    verify_database(&connection)
                                } else {
                                    quick_verify_database(&connection)
                                };
                                let _ = reply.send(result);
                            }
                            ReadCommand::Query { request, reply } => {
                                let _ = reply.send(execute_query(&connection, *request));
                            }
                            ReadCommand::Shutdown(done) => {
                                drop(connection);
                                let _ = done.send(());
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = reader_ready_sender.send(Err(error));
                }
            })
            .map_err(|error| StudioError::History(format!("cannot start reader: {error}")))?;

        match reader_ready_receiver.recv_timeout(config.startup_timeout) {
            Ok(Ok(())) => readers.push(reader_sender),
            Ok(Err(error)) => {
                stop_partial_workers(&writer_sender, readers, config.receipt_timeout);
                return Err(error);
            }
            Err(_) => {
                stop_partial_workers(&writer_sender, readers, config.receipt_timeout);
                return Err(StudioError::History(
                    "history reader startup timed out".into(),
                ));
            }
        }
    }

    Ok((
        root,
        WorkerSet {
            writer: writer_sender,
            terminal: terminal_sender,
            readers,
        },
    ))
}

fn stop_partial_workers(
    writer: &mpsc::SyncSender<WriterCommand>,
    readers: Vec<mpsc::SyncSender<ReadCommand>>,
    timeout: Duration,
) {
    for reader in readers {
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        if reader.try_send(ReadCommand::Shutdown(done_sender)).is_ok() {
            let _ = done_receiver.recv_timeout(timeout);
        }
    }
    let (done_sender, done_receiver) = mpsc::sync_channel(1);
    if writer
        .try_send(WriterCommand::Shutdown(done_sender))
        .is_ok()
    {
        let _ = done_receiver.recv_timeout(timeout);
    }
}

fn history_root(runtime_root: &Path) -> PathBuf {
    runtime_root.join("studio/data/history")
}

struct HistoryWriter {
    connection: Connection,
    _owner_lock: Flock<File>,
}

fn prepare_history_root(runtime_root: &Path, root: &Path) -> StudioResult<()> {
    if !runtime_root.exists() {
        ensure_private_directory(runtime_root, false)?;
    } else {
        validate_directory(runtime_root, false)?;
    }

    let studio = runtime_root.join("studio");
    ensure_private_directory(&studio, false)?;
    let data = studio.join("data");
    ensure_private_directory(&data, false)?;
    ensure_private_directory(root, true)?;
    validate_store_paths(root)
}

fn open_and_migrate(root: &Path, max_page_count: i64) -> StudioResult<HistoryWriter> {
    validate_store_paths(root)?;
    let owner_lock = acquire_owner_lock(root)?;
    let database = root.join("studio.sqlite3");
    let is_new = create_database_if_missing(&database)?;
    validate_private_file(&database)?;
    validate_wal_sidecar_header(root)?;

    let connection = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(history_error)?;
    enforce_private_file_mode(&database)?;
    configure_writer(&connection, is_new, max_page_count)?;
    qualify_sqlite(&connection)?;
    migrate(&connection)?;
    quick_verify_database(&connection)?;
    harden_sidecars(root)?;

    Ok(HistoryWriter {
        connection,
        _owner_lock: owner_lock,
    })
}

fn open_reader(database: &Path) -> StudioResult<Connection> {
    validate_private_file(database)?;
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(history_error)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA busy_timeout=250;              PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY; PRAGMA query_only=ON;",
        )
        .map_err(history_error)?;
    qualify_sqlite(&connection)?;
    verify_connection_pragmas(&connection, true)?;
    verify_migration(&connection)?;
    Ok(connection)
}

fn validate_store_paths(root: &Path) -> StudioResult<()> {
    for name in [
        "owner.lock",
        "studio.sqlite3",
        "studio.sqlite3-wal",
        "studio.sqlite3-shm",
    ] {
        let path = root.join(name);
        if path.exists() || fs::symlink_metadata(&path).is_ok() {
            validate_private_file(&path)?;
        }
    }
    let backups = root.join("backups");
    if backups.exists() || fs::symlink_metadata(&backups).is_ok() {
        validate_directory(&backups, true)?;
        for name in [
            "studio.sqlite3",
            "studio.sqlite3.candidate",
            "studio.sqlite3.previous",
        ] {
            let path = backups.join(name);
            if path.exists() || fs::symlink_metadata(&path).is_ok() {
                validate_private_file(&path)?;
            }
        }
    }
    Ok(())
}

fn acquire_owner_lock(root: &Path) -> StudioResult<Flock<File>> {
    let path = root.join("owner.lock");
    let lock = if path.exists() {
        validate_private_file(&path)?;
        private_open_options().read(true).write(true).open(&path)?
    } else {
        private_open_options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?
    };
    enforce_private_file_mode(&path)?;
    Flock::lock(lock, FlockArg::LockExclusiveNonblock)
        .map_err(|_| StudioError::History("history store is already owned".into()))
}

fn create_database_if_missing(path: &Path) -> StudioResult<bool> {
    reject_symlink(path)?;
    if path.exists() {
        validate_private_file(path)?;
        return Ok(false);
    }
    private_open_options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    enforce_private_file_mode(path)?;
    Ok(true)
}

fn private_open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    options
}

fn ensure_private_directory(path: &Path, strict_private: bool) -> StudioResult<()> {
    reject_symlink(path)?;
    match fs::create_dir(path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    validate_directory(path, strict_private)
}

fn validate_directory(path: &Path, strict_private: bool) -> StudioResult<()> {
    reject_symlink(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(StudioError::History(format!(
            "history path is not a directory: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mode = metadata.permissions().mode() & 0o777;
        if metadata.uid() != Uid::effective().as_raw() {
            return Err(StudioError::History(
                "history path owner does not match Studio user".into(),
            ));
        }
        if mode & 0o022 != 0 || (strict_private && mode & 0o077 != 0) {
            return Err(StudioError::History(
                "history path permissions are not private".into(),
            ));
        }
    }
    Ok(())
}

fn validate_private_file(path: &Path) -> StudioResult<()> {
    reject_symlink(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(StudioError::History(
            "history file is not a regular file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != Uid::effective().as_raw() {
            return Err(StudioError::History(
                "history file owner does not match Studio user".into(),
            ));
        }
        if metadata.nlink() != 1 {
            return Err(StudioError::History(
                "hard-linked history files are not allowed".into(),
            ));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(StudioError::History(
                "history file permissions are not private".into(),
            ));
        }
    }
    Ok(())
}

fn enforce_private_file_mode(path: &Path) -> StudioResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    validate_private_file(path)
}

fn validate_wal_sidecar_header(root: &Path) -> StudioResult<()> {
    let wal = root.join("studio.sqlite3-wal");
    if !wal.exists() {
        return Ok(());
    }
    validate_private_file(&wal)?;
    let metadata = fs::metadata(&wal)?;
    if metadata.len() == 0 {
        return Ok(());
    }
    if metadata.len() < 32 {
        return Err(StudioError::History(
            "history WAL sidecar is truncated".into(),
        ));
    }
    let mut file = File::open(&wal)?;
    let mut magic = [0_u8; 4];
    use std::io::Read as _;
    file.read_exact(&mut magic)?;
    let magic = u32::from_be_bytes(magic);
    if !matches!(magic, 0x377f_0682 | 0x377f_0683) {
        return Err(StudioError::History(
            "history WAL sidecar has invalid header".into(),
        ));
    }
    Ok(())
}

fn harden_sidecars(root: &Path) -> StudioResult<()> {
    for name in ["studio.sqlite3", "studio.sqlite3-wal", "studio.sqlite3-shm"] {
        let path = root.join(name);
        if path.exists() {
            enforce_private_file_mode(&path)?;
        }
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> StudioResult<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(StudioError::History("unsafe history symlink".into()));
    }
    Ok(())
}

fn configure_writer(
    connection: &Connection,
    is_new: bool,
    max_page_count: i64,
) -> StudioResult<()> {
    if is_new {
        connection
            .execute_batch("PRAGMA page_size=4096; PRAGMA auto_vacuum=INCREMENTAL;")
            .map_err(history_error)?;
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA journal_mode=WAL;              PRAGMA synchronous=FULL; PRAGMA busy_timeout=250; PRAGMA wal_autocheckpoint=1000;              PRAGMA journal_size_limit=16777216; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY;",
        )
        .map_err(history_error)?;
    connection
        .pragma_update(None, "max_page_count", max_page_count)
        .map_err(history_error)?;
    #[cfg(target_os = "macos")]
    connection
        .execute_batch("PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")
        .map_err(history_error)?;
    verify_connection_pragmas(connection, false)
}

fn verify_connection_pragmas(connection: &Connection, reader: bool) -> StudioResult<()> {
    let page_size = pragma_i64(connection, "page_size")?;
    let auto_vacuum = pragma_i64(connection, "auto_vacuum")?;
    let foreign_keys = pragma_i64(connection, "foreign_keys")?;
    let trusted_schema = pragma_i64(connection, "trusted_schema")?;
    let busy_timeout = pragma_i64(connection, "busy_timeout")?;
    let mmap_size = pragma_i64(connection, "mmap_size")?;
    let temp_store = pragma_i64(connection, "temp_store")?;
    let query_only = pragma_i64(connection, "query_only")?;
    if page_size != 4096
        || auto_vacuum != 2
        || foreign_keys != 1
        || trusted_schema != 0
        || busy_timeout != 250
        || mmap_size != 0
        || temp_store != 2
        || query_only != i64::from(reader)
    {
        return Err(StudioError::History(
            "history SQLite pragma verification failed".into(),
        ));
    }
    if !reader {
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(history_error)?;
        let synchronous = pragma_i64(connection, "synchronous")?;
        let journal_size_limit = pragma_i64(connection, "journal_size_limit")?;
        if !journal_mode.eq_ignore_ascii_case("wal")
            || synchronous != 2
            || journal_size_limit != 16_777_216
        {
            return Err(StudioError::History(
                "history SQLite durability pragma verification failed".into(),
            ));
        }
        #[cfg(target_os = "macos")]
        {
            if pragma_i64(connection, "fullfsync")? != 1
                || pragma_i64(connection, "checkpoint_fullfsync")? != 1
            {
                return Err(StudioError::History(
                    "history SQLite Darwin fsync pragma verification failed".into(),
                ));
            }
        }
    }
    Ok(())
}

fn pragma_i64(connection: &Connection, name: &str) -> StudioResult<i64> {
    let sql = match name {
        "page_size" => "PRAGMA page_size",
        "auto_vacuum" => "PRAGMA auto_vacuum",
        "foreign_keys" => "PRAGMA foreign_keys",
        "trusted_schema" => "PRAGMA trusted_schema",
        "busy_timeout" => "PRAGMA busy_timeout",
        "mmap_size" => "PRAGMA mmap_size",
        "temp_store" => "PRAGMA temp_store",
        "query_only" => "PRAGMA query_only",
        "synchronous" => "PRAGMA synchronous",
        "journal_size_limit" => "PRAGMA journal_size_limit",
        "page_count" => "PRAGMA page_count",
        "max_page_count" => "PRAGMA max_page_count",
        "fullfsync" => "PRAGMA fullfsync",
        "checkpoint_fullfsync" => "PRAGMA checkpoint_fullfsync",
        _ => {
            return Err(StudioError::History(
                "unsupported internal history pragma".into(),
            ));
        }
    };
    connection
        .query_row(sql, [], |row| row.get(0))
        .map_err(history_error)
}

fn qualify_sqlite(connection: &Connection) -> StudioResult<(String, String)> {
    let (version, source_id): (String, String) = connection
        .query_row("SELECT sqlite_version(), sqlite_source_id()", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(history_error)?;
    let parsed = semver::Version::parse(&version)
        .map_err(|_| StudioError::History("unrecognized bundled SQLite version".into()))?;
    let minimum = semver::Version::parse(MIN_SQLITE_VERSION).unwrap();
    if parsed < minimum || source_id.len() < 64 {
        return Err(StudioError::History(
            "bundled SQLite engine does not satisfy the M6 qualification floor".into(),
        ));
    }
    Ok((version, source_id))
}

fn migrate(connection: &Connection) -> StudioResult<()> {
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(history_error)?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(StudioError::History(
            "database belongs to another application".into(),
        ));
    }

    let mut version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(history_error)?;
    if version > SCHEMA_VERSION {
        return Err(StudioError::History(
            "database schema is newer than this binary".into(),
        ));
    }
    if version == SCHEMA_VERSION {
        verify_migration(connection)?;
        return Ok(());
    }

    if version == 0 {
        let user_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(history_error)?;
        if application_id == 0 && user_tables != 0 {
            return Err(StudioError::History(
                "nonempty unowned database cannot be adopted as history".into(),
            ));
        }
        apply_v1_migration(connection)?;
        version = 1;
    }

    if version == 1 {
        apply_v2_migration(connection)?;
        version = 2;
    }

    if version != SCHEMA_VERSION {
        return Err(StudioError::History(
            "history migration chain is not contiguous".into(),
        ));
    }

    verify_migration(connection)
}

fn apply_v1_migration(connection: &Connection) -> StudioResult<()> {
    let sql = migration_v1_sql()?;
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    transaction.execute_batch(&sql).map_err(history_error)?;
    transaction
        .execute(
            "INSERT INTO schema_migrations(version, name, sha256, applied_at_ms) VALUES (?1, ?2, ?3, ?4)",
            (1_i64, "0001_history_v1", SCHEMA_V1_SHA256, now_ms()?),
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "INSERT INTO history_meta(singleton, db_epoch, created_at_ms) VALUES (1, ?1, ?2)",
            (uuid::Uuid::new_v4().to_string(), now_ms()?),
        )
        .map_err(history_error)?;
    transaction
        .execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=1;"
        ))
        .map_err(history_error)?;
    transaction.commit().map_err(history_error)
}

fn apply_v2_migration(connection: &Connection) -> StudioResult<()> {
    let digest = format!("{:x}", Sha256::digest(MIGRATION_V2_SQL.as_bytes()));
    if digest != MIGRATION_V2_SHA256 {
        return Err(StudioError::History(
            "M8 history migration checksum mismatch".into(),
        ));
    }

    connection
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(history_error)?;
    let migration_result = (|| -> StudioResult<()> {
        let transaction = connection.unchecked_transaction().map_err(history_error)?;
        transaction
            .execute_batch(MIGRATION_V2_SQL)
            .map_err(history_error)?;
        transaction
            .execute(
                "INSERT INTO schema_migrations(version, name, sha256, applied_at_ms) VALUES (2, '0002_system_automation_actor', ?1, ?2)",
                (MIGRATION_V2_SHA256, now_ms()?),
            )
            .map_err(history_error)?;
        transaction
            .execute_batch("PRAGMA user_version=2;")
            .map_err(history_error)?;
        transaction.commit().map_err(history_error)?;
        Ok(())
    })();

    let reenable_result = connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(history_error);
    migration_result?;
    reenable_result?;
    verify_foreign_keys(connection)
}

fn migration_v1_sql() -> StudioResult<String> {
    let sql = SCHEMA_DOCUMENT
        .split_once("\x60\x60\x60sql\n")
        .and_then(|(_, rest)| rest.split_once("\n\x60\x60\x60"))
        .map(|(sql, _)| sql)
        .ok_or_else(|| StudioError::History("embedded schema fixture is malformed".into()))?;
    let digest = format!("{:x}", Sha256::digest(sql.as_bytes()));
    if digest != SCHEMA_V1_SHA256 {
        return Err(StudioError::History(
            "embedded schema checksum mismatch".into(),
        ));
    }
    Ok(sql.to_owned())
}

fn verify_migration(connection: &Connection) -> StudioResult<()> {
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(history_error)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(history_error)?;
    if application_id != APPLICATION_ID || version != SCHEMA_VERSION {
        return Err(StudioError::History(
            "history schema identity mismatch".into(),
        ));
    }
    for (version, name, expected) in [
        (1_i64, "0001_history_v1", SCHEMA_V1_SHA256),
        (2_i64, "0002_system_automation_actor", MIGRATION_V2_SHA256),
    ] {
        let checksum: Option<String> = connection
            .query_row(
                "SELECT sha256 FROM schema_migrations WHERE version=?1 AND name=?2",
                (version, name),
                |row| row.get(0),
            )
            .optional()
            .map_err(history_error)?;
        if checksum.as_deref() != Some(expected) {
            return Err(StudioError::History(
                "schema migration checksum mismatch".into(),
            ));
        }
    }
    let operations_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='operations'",
            [],
            |row| row.get(0),
        )
        .map_err(history_error)?;
    if !operations_sql.contains("'system_automation'") {
        return Err(StudioError::History(
            "M8 automation actor schema is missing".into(),
        ));
    }
    verify_foreign_keys(connection)
}

fn now_ms() -> StudioResult<i64> {
    system_time_ms(SystemTime::now())
}

fn system_time_ms(value: SystemTime) -> StudioResult<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StudioError::History("system clock is before UNIX epoch".into()))
        .and_then(|duration| {
            i64::try_from(duration.as_millis())
                .map_err(|_| StudioError::History("system clock overflow".into()))
        })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn history_commit_watermark(connection: &Connection) -> StudioResult<(String, i64, i64)> {
    let (db_epoch, retention_epoch): (String, i64) = connection
        .query_row(
            "SELECT db_epoch,retention_epoch FROM history_meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(history_error)?;
    let latest_seq: i64 = connection
        .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |row| {
            row.get(0)
        })
        .map_err(history_error)?;
    Ok((db_epoch, latest_seq, retention_epoch))
}

fn history_error(error: rusqlite::Error) -> StudioError {
    StudioError::History(error.to_string())
}

pub fn backup_database(source: &Connection, destination: &Path) -> StudioResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| StudioError::History("invalid history backup path".into()))?;
    ensure_private_directory(parent, true)?;

    let candidate = parent.join("studio.sqlite3.candidate");
    let previous = parent.join("studio.sqlite3.previous");
    for path in [&candidate, &previous, destination] {
        if path.exists() || fs::symlink_metadata(path).is_ok() {
            validate_private_file(path)?;
        }
    }
    if candidate.exists() {
        return Err(StudioError::History(
            "stale history backup candidate requires operator attention".into(),
        ));
    }

    private_open_options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&candidate)?;
    enforce_private_file_mode(&candidate)?;
    let mut target = Connection::open_with_flags(
        &candidate,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(history_error)?;
    {
        let backup = Backup::new(source, &mut target).map_err(history_error)?;
        backup
            .run_to_completion(100, Duration::from_millis(1), None)
            .map_err(history_error)?;
    }
    verify_database(&target)?;
    drop(target);
    File::open(&candidate)?.sync_all()?;

    if previous.exists() {
        fs::remove_file(&previous)?;
    }
    if destination.exists() {
        fs::rename(destination, &previous)?;
    }
    if let Err(error) = fs::rename(&candidate, destination) {
        if previous.exists() && !destination.exists() {
            let _ = fs::rename(&previous, destination);
        }
        return Err(error.into());
    }
    enforce_private_file_mode(destination)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn quick_verify_database(connection: &Connection) -> StudioResult<()> {
    verify_migration(connection)?;
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(history_error)?;
    if check != "ok" {
        return Err(StudioError::History(
            "history integrity check failed".into(),
        ));
    }
    verify_foreign_keys(connection)
}

fn verify_database(connection: &Connection) -> StudioResult<()> {
    verify_migration(connection)?;
    let check: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(history_error)?;
    if check != "ok" {
        return Err(StudioError::History(
            "history full integrity check failed".into(),
        ));
    }
    verify_foreign_keys(connection)
}

fn verify_foreign_keys(connection: &Connection) -> StudioResult<()> {
    let foreign_key_problem: Option<i64> = connection
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()
        .map_err(history_error)?;
    if foreign_key_problem.is_some() {
        return Err(StudioError::History(
            "history foreign-key check failed".into(),
        ));
    }
    Ok(())
}

fn record_bootstrap(
    connection: &mut Connection,
    subject_id: uuid::Uuid,
    source_stream: &str,
    source_ordinal: u64,
) -> StudioResult<u64> {
    if source_stream.is_empty() || source_stream.len() > 256 {
        return Err(StudioError::History("invalid history source stream".into()));
    }
    let ordinal = i64::try_from(source_ordinal)
        .map_err(|_| StudioError::History("history source ordinal overflow".into()))?;
    let subject_id = subject_id.to_string();
    let event_id = uuid::Uuid::new_v4().to_string();
    let now = now_ms()?;
    let transaction = connection.unchecked_transaction().map_err(history_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(subject_id, kind, incarnation_id, first_observed_ms) VALUES (?1, 'system', ?1, ?2)",
            (&subject_id, now),
        )
        .map_err(history_error)?;
    transaction
        .execute(
            "INSERT INTO events(event_id, subject_id, source_stream, source_ordinal, name, category, observed_at_ms, time_quality, evidence_kind, payload_version, payload_json, payload_sha256, retention_class) VALUES (?1, ?2, ?3, ?4, 'history.bootstrap', 'observation', ?5, 'local', 'bootstrap', 1, '{}', ?6, 'observation')",
            (&event_id, &subject_id, source_stream, ordinal, now, format!("{:x}", Sha256::digest(b"{}"))),
        )
        .map_err(history_error)?;
    let sequence = transaction.last_insert_rowid();
    transaction.commit().map_err(history_error)?;
    u64::try_from(sequence).map_err(|_| StudioError::History("history sequence overflow".into()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn runtime_fixture() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn unavailable_history_rejects_gateway_observation_without_retry_side_effect() {
        let fixture = runtime_fixture();
        let blocked_root = fixture.path().join("not-a-directory");
        fs::write(&blocked_root, b"blocked").unwrap();
        let history = HistoryHandle::initialize(&blocked_root);
        assert!(matches!(history.health(), HistoryHealth::Degraded { .. }));
        let result = history.observe_gateway_events(
            "gateway-1".into(),
            vec![GatewayHistoryEvent {
                sequence: 1,
                observed_at_ms: 1,
                observation: gateway_history::GatewayObservation::Catalog {
                    catalog_generation: 1,
                    profile_generation: 1,
                    catalog_fingerprint: "a".repeat(64),
                    profile_fingerprint: "b".repeat(64),
                },
            }],
        );
        assert!(result.is_err());
    }

    fn create_runtime(root: &Path) -> PathBuf {
        let runtime = root.join("runtime");
        fs::create_dir(&runtime).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        }
        runtime
    }

    #[test]
    fn admission_failure_prevents_discretionary_effect() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);

        let subject = uuid::Uuid::new_v4();
        history
            .record_bootstrap(subject, "degrade/stream", 0)
            .unwrap();
        assert!(
            history
                .record_bootstrap(subject, "degrade/stream", 0)
                .is_err()
        );
        assert_ne!(history.health(), HistoryHealth::Healthy);

        let effect_started = std::sync::atomic::AtomicBool::new(false);
        let admitted = history.admit_operation(
            SubjectKind::Mcp,
            HistoryAction::McpStart,
            ActorKind::LocalOperator,
        );
        if admitted.is_ok() {
            effect_started.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        assert!(admitted.is_err());
        assert!(!effect_started.load(std::sync::atomic::Ordering::SeqCst));
        history.shutdown();
    }

    #[test]
    fn request_is_never_terminal_success() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let database = history.root().join("studio.sqlite3");

        let (context, admission) = history
            .admit_operation(
                SubjectKind::Mcp,
                HistoryAction::McpRestart,
                ActorKind::LocalOperator,
            )
            .unwrap();
        assert_eq!(context.operation_id(), admission.operation_id);
        assert_eq!(history.active_operation_count(), 1);

        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (effect, terminal): (String, Option<i64>) = connection
            .query_row(
                "SELECT effect_status, terminal_seq FROM operations WHERE operation_id=?1",
                [context.operation_id().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(effect, "not_dispatched");
        assert_eq!(terminal, None);
        let success_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation_id=?1 AND disposition='succeeded'",
                [context.operation_id().to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(success_count, 0);
        drop(connection);

        let terminal = history
            .finish_operation(&context, OperationOutcome::Succeeded, None)
            .unwrap();
        assert_eq!(terminal.operation_id, context.operation_id());
        assert!(terminal.terminal_seq > admission.admitted_seq);
        assert_eq!(history.active_operation_count(), 0);
        history.shutdown();
    }

    #[test]
    fn conflicting_source_ordinal_is_rejected() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let database = history.root().join("studio.sqlite3");

        let (context, _) = history
            .admit_operation(
                SubjectKind::Registry,
                HistoryAction::RegistryUpdate,
                ActorKind::LocalOperator,
            )
            .unwrap();
        history.shutdown();

        let connection = Connection::open(&database).unwrap();
        configure_writer(&connection, false, MAX_PAGE_COUNT).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        let payload = OperationEventPayload::new(
            context.action(),
            context.actor(),
            "failed",
            "complete",
            Some("different_payload".into()),
            false,
        )
        .unwrap();
        let error = insert_operation_event(
            &transaction,
            &context,
            &payload,
            OperationEventSpec {
                source_ordinal: 1,
                name: "operation.admitted",
                category: "admission",
                disposition: "admitted",
                observed_at_ms: now_ms().unwrap(),
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("conflicting history source ordinal")
        );
    }

    #[test]
    fn all_required_action_families_have_typed_receipts() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);

        for action in HistoryAction::ALL {
            let subject_kind = match action {
                HistoryAction::McpStart | HistoryAction::McpStop | HistoryAction::McpRestart => {
                    SubjectKind::Mcp
                }
                HistoryAction::TunnelStart
                | HistoryAction::TunnelStop
                | HistoryAction::TunnelRestart => SubjectKind::Tunnel,
                HistoryAction::RegistryRegister
                | HistoryAction::RegistryUpdate
                | HistoryAction::RegistryEnable
                | HistoryAction::RegistryDisable
                | HistoryAction::RegistryUnregister
                | HistoryAction::DiscoveryApprove => SubjectKind::Registry,
                HistoryAction::ReconciliationCheck
                | HistoryAction::ReconciliationAdopt
                | HistoryAction::ReconciliationApply => SubjectKind::Fleet,
                HistoryAction::StudioSelfUpdateRequest
                | HistoryAction::StudioSelfUpdateFinalize => SubjectKind::Studio,
                HistoryAction::UpdateCheck
                | HistoryAction::UpdatePrepare
                | HistoryAction::UpdateApply
                | HistoryAction::UpdateRollback => SubjectKind::Component,
            };
            let (context, admission) = history
                .admit_operation(subject_kind, action, ActorKind::LocalOperator)
                .unwrap();
            let terminal = history
                .finish_operation(&context, OperationOutcome::Succeeded, None)
                .unwrap();
            assert_eq!(admission.operation_id, terminal.operation_id);
            assert!(terminal.terminal_seq > admission.admitted_seq);
        }
        assert_eq!(history.active_operation_count(), 0);
        history.shutdown();
    }

    #[test]
    fn operation_admission_limit_is_bounded() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let mut contexts = Vec::new();

        for _ in 0..MAX_ACTIVE_OPERATIONS {
            let (context, _) = history
                .admit_operation(
                    SubjectKind::Component,
                    HistoryAction::UpdateApply,
                    ActorKind::LocalOperator,
                )
                .unwrap();
            contexts.push(context);
        }
        assert_eq!(history.active_operation_count(), MAX_ACTIVE_OPERATIONS);
        assert!(
            history
                .admit_operation(
                    SubjectKind::Component,
                    HistoryAction::UpdateApply,
                    ActorKind::LocalOperator,
                )
                .is_err()
        );

        for context in contexts {
            history
                .finish_operation(&context, OperationOutcome::Succeeded, None)
                .unwrap();
        }
        assert_eq!(history.active_operation_count(), 0);
        history.shutdown();
    }

    #[test]
    fn post_effect_history_failure_does_not_retry_action() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let (context, _) = history
            .admit_operation(
                SubjectKind::Mcp,
                HistoryAction::McpStop,
                ActorKind::LocalOperator,
            )
            .unwrap();

        let effects = std::sync::atomic::AtomicUsize::new(0);
        effects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        history.shutdown();

        assert!(
            history
                .finish_operation(&context, OperationOutcome::Succeeded, None)
                .is_err()
        );
        assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(history.active_operation_count(), 0);
    }

    #[test]
    fn sessions_survive_studio_restart() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let run_id = uuid::Uuid::new_v4();
        let history = HistoryHandle::initialize(&runtime_root);
        history.start_run(run_id, "process-one", None).unwrap();
        let session = history
            .observe_session_started(LifecycleOwnerKind::Mcp, 1, 4242)
            .unwrap();
        history.observe_session_terminal(
            session.clone(),
            LifecycleEndKind::CleanExit,
            Some(0),
            false,
            Some(Duration::from_millis(12)),
        );
        history.close_run(true).unwrap();
        history.shutdown();

        let reopened = HistoryHandle::initialize(&runtime_root);
        let database = reopened.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (end_kind, exit_code, exact_duration): (String, Option<i32>, Option<i64>) = connection
            .query_row(
                "SELECT end_kind, exit_code, exact_duration_ms
                 FROM runtime_sessions WHERE session_id=?1",
                [session.session_id().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(end_kind, "clean_exit");
        assert_eq!(exit_code, Some(0));
        assert_eq!(exact_duration, Some(12));
        drop(connection);
        reopened.shutdown();
    }

    #[test]
    fn interrupted_observer_does_not_invent_child_exit() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let first_run = uuid::Uuid::new_v4();
        let history = HistoryHandle::initialize(&runtime_root);
        history.start_run(first_run, "process-one", None).unwrap();
        let session = history
            .observe_session_started(LifecycleOwnerKind::Mcp, 1, 4242)
            .unwrap();
        history.shutdown();

        let second = HistoryHandle::initialize(&runtime_root);
        second
            .start_run(uuid::Uuid::new_v4(), "process-two", None)
            .unwrap();
        let database = second.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (run_state, session_state, ended_at, exit_code, is_crash): (
            String,
            String,
            Option<i64>,
            Option<i32>,
            Option<i64>,
        ) = connection
            .query_row(
                "SELECT r.end_state, s.end_kind, s.ended_at_ms, s.exit_code, s.is_crash
                 FROM studio_runs r
                 JOIN runtime_sessions s ON s.run_id=r.run_id
                 WHERE r.run_id=?1 AND s.session_id=?2",
                (first_run.to_string(), session.session_id().to_string()),
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(run_state, "interrupted");
        assert_eq!(session_state, "interrupted");
        assert_eq!(ended_at, None);
        assert_eq!(exit_code, None);
        assert_eq!(is_crash, None);
        drop(connection);
        second.shutdown();
    }

    #[test]
    fn terminal_before_start_cannot_reopen_session() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "process-one", None)
            .unwrap();
        let context = LifecycleSessionContext::new(LifecycleOwnerKind::Tunnel, 7, 7331);

        history.observe_session_terminal(
            context.clone(),
            LifecycleEndKind::UnexpectedExit,
            Some(0),
            true,
            Some(Duration::from_millis(5)),
        );
        let run_id = history.current_run_id().unwrap();
        history
            .writer_sender()
            .unwrap()
            .try_send(WriterCommand::ObserveLifecycleStarted {
                run_id,
                context: context.clone(),
            })
            .unwrap();
        history.close_run(true).unwrap();
        history.shutdown();

        let connection =
            Connection::open(runtime_root.join("studio/data/history/studio.sqlite3")).unwrap();
        let (end_kind, exit_code, is_crash, ordinal): (String, Option<i32>, Option<i64>, i64) =
            connection
                .query_row(
                    "SELECT end_kind, exit_code, is_crash, last_source_ordinal
                     FROM runtime_sessions WHERE session_id=?1",
                    [context.session_id().to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
        assert_eq!(end_kind, "unexpected_exit");
        assert_eq!(exit_code, Some(0));
        assert_eq!(is_crash, Some(1));
        assert_eq!(ordinal, 1);
    }

    #[test]
    fn run_ready_and_clean_close_are_distinct_events() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let run_id = uuid::Uuid::new_v4();
        let history = HistoryHandle::initialize(&runtime_root);
        history.start_run(run_id, "process-one", None).unwrap();
        history.mark_run_ready().unwrap();
        history.close_run(true).unwrap();
        history.shutdown();

        let connection =
            Connection::open(runtime_root.join("studio/data/history/studio.sqlite3")).unwrap();
        let names: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT name FROM events WHERE run_id=?1 ORDER BY source_ordinal")
                .unwrap();
            statement
                .query_map([run_id.to_string()], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(
            names,
            vec![
                "studio.run.started".to_owned(),
                "studio.run.ready".to_owned(),
                "studio.run.closed".to_owned(),
            ]
        );
    }

    #[test]
    fn linked_sqlite_version_and_source_id_are_qualified() {
        let connection = Connection::open_in_memory().unwrap();
        let (version, source_id) = qualify_sqlite(&connection).unwrap();
        assert!(semver::Version::parse(&version).unwrap() >= semver::Version::new(3, 51, 3));
        assert!(source_id.len() >= 64);
    }

    #[test]
    fn initializes_private_schema_and_reopens_it() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        assert_eq!(history.health(), HistoryHealth::Healthy);
        let database = history.root().join("studio.sqlite3");
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        drop(connection);

        let contended = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            contended.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );

        history.verify().unwrap();
        history.shutdown();

        let reopened = HistoryHandle::initialize(&runtime_root);
        assert_eq!(reopened.health(), HistoryHealth::Healthy);
        reopened.shutdown();
    }

    #[test]
    fn writer_serializes_concurrent_producers() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let mut threads = Vec::new();
        for index in 0..24_u64 {
            let history = history.clone();
            threads.push(thread::spawn(move || {
                history
                    .record_bootstrap(uuid::Uuid::new_v4(), &format!("test/concurrent/{index}"), 0)
                    .unwrap()
            }));
        }
        let mut sequences: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        sequences.sort_unstable();
        sequences.dedup();
        assert_eq!(sequences.len(), 24);
        history.shutdown();
    }

    #[test]
    fn unsafe_history_paths_fail_closed() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let root = runtime_root.join("studio/data/history");
        fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            std::os::unix::fs::symlink(fixture.path().join("target"), root.join("owner.lock"))
                .unwrap();
        }
        let history = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            history.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn hardlinked_history_database_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let root = runtime_root.join("studio/data/history");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let database = root.join("studio.sqlite3");
        private_open_options()
            .write(true)
            .create_new(true)
            .open(&database)
            .unwrap();
        fs::hard_link(&database, fixture.path().join("history-hardlink")).unwrap();

        assert_eq!(
            HistoryHandle::initialize(&runtime_root).health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[test]
    fn migration_checksum_and_atomic_upgrade() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let database = history.root().join("studio.sqlite3");
        history.shutdown();

        let connection = Connection::open(&database).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        transaction
            .execute_batch("CREATE TABLE fixture_v2(value INTEGER);")
            .unwrap();
        assert!(transaction.execute_batch("THIS IS NOT SQL").is_err());
        drop(transaction);
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='fixture_v2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        connection
            .execute(
                "UPDATE schema_migrations SET sha256='0000000000000000000000000000000000000000000000000000000000000000' WHERE version=1",
                [],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            HistoryHandle::initialize(&runtime_root).health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[test]
    fn m8_v1_to_v2_preserves_operation_foreign_keys_and_accepts_automation_actor() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        apply_v1_migration(&connection).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );

        connection
            .execute(
                "INSERT INTO subjects(subject_id,kind,incarnation_id,first_observed_ms)
                 VALUES('subject','system','incarnation',1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operations(
                    operation_id,subject_id,action_code,actor_kind,effect_status,audit_status
                 ) VALUES(
                    'operation','subject','update.check','local_operator','succeeded','complete'
                 )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO events(
                    event_id,subject_id,operation_id,source_stream,source_ordinal,name,category,
                    observed_at_ms,time_quality,evidence_kind,payload_version,payload_json,
                    payload_sha256,retention_class
                 ) VALUES(
                    'event','subject','operation','fixture',1,'operation.terminal','outcome',
                    1,'local','owner',1,'{}',
                    '0000000000000000000000000000000000000000000000000000000000000000',
                    'audit'
                 )",
                [],
            )
            .unwrap();

        apply_v2_migration(&connection).unwrap();
        verify_migration(&connection).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT operation_id FROM events WHERE event_id='event'",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap()
                .as_deref(),
            Some("operation")
        );
        connection
            .execute(
                "INSERT INTO operations(
                    operation_id,subject_id,action_code,actor_kind,effect_status,audit_status
                 ) VALUES(
                    'automation','subject','update.check','system_automation','succeeded','complete'
                 )",
                [],
            )
            .unwrap();
        verify_foreign_keys(&connection).unwrap();
    }

    #[test]
    fn rejects_future_schema_and_tampered_migration() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let database = history.root().join("studio.sqlite3");
        history.shutdown();

        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(&format!("PRAGMA user_version={};", SCHEMA_VERSION + 1))
            .unwrap();
        drop(connection);
        assert_eq!(
            HistoryHandle::initialize(&runtime_root).health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[test]
    fn corrupted_main_db_fails_closed_without_auto_repair() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let database = history.root().join("studio.sqlite3");
        history.shutdown();

        let corrupt = vec![0xA5_u8; 256];
        fs::write(&database, &corrupt).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let reopened = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            reopened.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
        assert_eq!(
            fs::read(&database).unwrap(),
            corrupt,
            "history startup must not auto-repair or replace a corrupted main DB"
        );
    }

    #[test]
    fn malformed_wal_fails_closed_without_auto_repair() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let root = history.root().to_path_buf();
        let database = root.join("studio.sqlite3");
        history.shutdown();

        let before_database = fs::read(&database).unwrap();
        let wal = root.join("studio.sqlite3-wal");
        let malformed = vec![0x5A_u8; 64];
        fs::write(&wal, &malformed).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&wal, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let reopened = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            reopened.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
        assert_eq!(
            fs::read(&wal).unwrap(),
            malformed,
            "history startup must not consume or rewrite an invalid WAL sidecar"
        );
        assert_eq!(
            fs::read(&database).unwrap(),
            before_database,
            "invalid WAL rejection must leave the main DB untouched"
        );
    }

    #[test]
    fn second_writer_is_rejected_while_owner_lock_is_held() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let first = HistoryHandle::initialize(&runtime_root);
        assert_eq!(first.health(), HistoryHealth::Healthy);

        let second = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            second.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );

        first
            .record_bootstrap(uuid::Uuid::new_v4(), "owner-lock/first", 0)
            .unwrap();
        first.shutdown();

        let reopened = HistoryHandle::initialize(&runtime_root);
        assert_eq!(reopened.health(), HistoryHealth::Healthy);
        reopened.shutdown();
    }

    #[test]
    fn rejects_database_owned_by_another_application() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let root = runtime_root.join("studio/data/history");
        fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let database = root.join("studio.sqlite3");
        private_open_options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&database)
            .unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA application_id=123")
            .unwrap();
        drop(connection);

        assert_eq!(
            HistoryHandle::initialize(&runtime_root).health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[test]
    fn rejects_nonempty_unowned_database() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let root = runtime_root.join("studio/data/history");
        fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let database = root.join("studio.sqlite3");
        private_open_options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&database)
            .unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("CREATE TABLE foreign_table(x)")
            .unwrap();
        drop(connection);

        assert_eq!(
            HistoryHandle::initialize(&runtime_root).health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[test]
    fn rejects_unbounded_or_overflowing_queue_configuration() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        assert_eq!(
            HistoryHandle::with_config(
                &runtime_root,
                HistoryConfig {
                    writer_capacity: 0,
                    ..HistoryConfig::default()
                }
            )
            .health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
        assert_eq!(
            HistoryHandle::with_config(
                &runtime_root,
                HistoryConfig {
                    writer_capacity: usize::MAX,
                    terminal_reserve: 1,
                    ..HistoryConfig::default()
                }
            )
            .health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
        assert_eq!(
            HistoryHandle::with_config(
                &runtime_root,
                HistoryConfig {
                    reader_capacity: 0,
                    ..HistoryConfig::default()
                }
            )
            .health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires disposable filesystem fixture via M6_ENOSPC_RUNTIME_ROOT"]
    fn live_filesystem_enospc_smoke() {
        let runtime_root = std::env::var_os("M6_ENOSPC_RUNTIME_ROOT")
            .map(PathBuf::from)
            .expect("M6_ENOSPC_RUNTIME_ROOT is required");
        fs::create_dir_all(&runtime_root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        }

        let history = HistoryHandle::initialize(&runtime_root);
        assert_eq!(history.health(), HistoryHealth::Healthy);
        let mut full_error = None;
        for index in 0..5_000_u64 {
            match history.record_bootstrap(
                uuid::Uuid::new_v4(),
                &format!("filesystem-enospc/{index:06}"),
                0,
            ) {
                Ok(_) => {}
                Err(error) => {
                    full_error = Some(error.to_string());
                    break;
                }
            }
        }
        let full_error = full_error.expect("filesystem fixture did not reach ENOSPC/SQLITE_FULL");
        assert!(
            full_error.contains("full")
                || full_error.contains("disk")
                || full_error.contains("space"),
            "unexpected filesystem-full error: {full_error}"
        );
        assert!(matches!(
            history.health(),
            HistoryHealth::Degraded {
                code: "history_write_failed"
            }
        ));
        history.shutdown();
    }

    #[test]
    fn sqlite_full_degrades_history_and_blocks_new_admission() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::with_config(
            &runtime_root,
            HistoryConfig {
                max_page_count: 128,
                ..HistoryConfig::default()
            },
        );
        assert_eq!(history.health(), HistoryHealth::Healthy);

        let mut full_error = None;
        for index in 0..20_000_u64 {
            match history.record_bootstrap(
                uuid::Uuid::new_v4(),
                &format!("sqlite-full/{index:05}"),
                0,
            ) {
                Ok(_) => {}
                Err(error) => {
                    full_error = Some(error.to_string());
                    break;
                }
            }
        }

        let full_error =
            full_error.expect("bounded SQLite file must eventually report SQLITE_FULL");
        assert!(
            full_error.contains("full") || full_error.contains("database or disk"),
            "unexpected SQLite full error: {full_error}"
        );
        assert_eq!(
            history.health(),
            HistoryHealth::Degraded {
                code: "history_write_failed"
            }
        );
        assert!(
            history
                .admit_operation(
                    SubjectKind::Mcp,
                    HistoryAction::McpStart,
                    ActorKind::LocalOperator,
                )
                .is_err(),
            "degraded history must reject new discretionary admissions"
        );
        history.shutdown();
    }

    #[test]
    fn store_faults_leave_live_safety_available() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history.shutdown();
        assert!(
            history
                .record_bootstrap(uuid::Uuid::new_v4(), "test/after-shutdown", 0)
                .is_err()
        );
        assert_eq!(
            history.health(),
            HistoryHealth::Degraded {
                code: "history_write_failed"
            }
        );
    }

    #[test]
    fn clock_jump_and_large_sequence_are_explicit() {
        assert!(system_time_ms(UNIX_EPOCH - Duration::from_secs(1)).is_err());
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        assert!(
            history
                .record_bootstrap(uuid::Uuid::new_v4(), "test/large-sequence", u64::MAX)
                .is_err()
        );
        history.shutdown();
    }

    #[test]
    fn wal_backup_restore_preserves_committed_history() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "test/backup", 0)
            .unwrap();
        let backup = history.backup().unwrap();
        let restored =
            Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            restored
                .query_row("SELECT COUNT(*) FROM events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            restored
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        history.shutdown();
    }

    #[test]
    fn runtime_only_history_path_is_release_independent() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        fs::create_dir_all(runtime_root.join("studio/releases/v0.6.0-alpha")).unwrap();
        let history = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            history.root(),
            runtime_root.join("studio/data/history").as_path()
        );
        assert!(
            !history
                .root()
                .starts_with(runtime_root.join("studio/releases"))
        );
        history.shutdown();
    }

    #[test]
    fn commits_typed_bootstrap_once() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let subject = uuid::Uuid::new_v4();
        assert_eq!(history.record_bootstrap(subject, "run/one", 0).unwrap(), 1);
        assert!(history.record_bootstrap(subject, "run/one", 0).is_err());
        assert_eq!(
            history.health(),
            HistoryHealth::Degraded {
                code: "history_write_failed"
            }
        );
        history.shutdown();
    }

    #[test]
    fn interrupted_apply_becomes_indeterminate_after_observer_restart() {
        use crate::update::{ComponentId, McpUpdatePhase, McpUpdateTransactionView, Version};

        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let first_run = uuid::Uuid::new_v4();
        history.start_run(first_run, "observer-one", None).unwrap();

        let (operation, _) = history
            .admit_operation(
                SubjectKind::Component,
                HistoryAction::UpdateApply,
                ActorKind::LocalOperator,
            )
            .unwrap();
        let view = McpUpdateTransactionView {
            transaction_id: "txn-interrupted-apply".into(),
            component: ComponentId::Git,
            source_version: Some(Version::parse("1.0.0").unwrap()),
            target_version: Version::parse("1.1.0").unwrap(),
            phase: McpUpdatePhase::Activating,
            was_running: Some(true),
            rollback_succeeded: None,
            error: None,
            updated_at_ms: 1,
        };
        history.observe_update_transaction(&view, Some(operation.operation_id()), true);

        // Simulate observer death: the next process has no in-memory current run,
        // while the old durable run is still open.
        *history.inner.current_run.lock().unwrap() = None;
        history
            .start_run(uuid::Uuid::new_v4(), "observer-two", None)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (effect, terminal): (String, Option<i64>) = connection
            .query_row(
                "SELECT effect_status,terminal_seq FROM operations WHERE operation_id=?1",
                [operation.operation_id().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(effect, "indeterminate");
        assert!(terminal.is_some());

        let (install, completeness, attempt_terminal): (String, String, Option<i64>) = connection
            .query_row(
                "SELECT install_outcome,completeness,terminal_seq
                 FROM update_attempts WHERE transaction_id='txn-interrupted-apply'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(install, "indeterminate");
        assert_eq!(completeness, "interrupted");
        assert!(attempt_terminal.is_some());
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn aggregation_retry_is_idempotent() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "metric-observer", None)
            .unwrap();

        let mcp = history
            .observe_session_started(LifecycleOwnerKind::Mcp, 1, 1001)
            .unwrap();
        history.observe_session_terminal(
            mcp,
            LifecycleEndKind::CleanExit,
            Some(0),
            false,
            Some(Duration::from_millis(10_000)),
        );
        let tunnel = history
            .observe_session_started(LifecycleOwnerKind::Tunnel, 2, 1002)
            .unwrap();
        history.observe_session_terminal(
            tunnel,
            LifecycleEndKind::UnexpectedExit,
            Some(0),
            true,
            Some(Duration::from_millis(6_000)),
        );
        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let durable: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM runtime_sessions
                     WHERE end_kind!='open' AND exact_duration_ms IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            drop(connection);
            if durable == 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "lifecycle facts did not become durable"
            );
            thread::sleep(Duration::from_millis(10));
        }

        history.housekeeping().unwrap();
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let before: Vec<(String, i64, i64)> = {
            let mut statement = connection
                .prepare(
                    "SELECT metric_code,count_value,sum_value
                     FROM metrics_totals ORDER BY metric_code",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        drop(connection);

        history.housekeeping().unwrap();
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let after: Vec<(String, i64, i64)> = {
            let mut statement = connection
                .prepare(
                    "SELECT metric_code,count_value,sum_value
                     FROM metrics_totals ORDER BY metric_code",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(before, after);
        assert!(after.contains(&("launches".into(), 2, 2)));
        assert!(after.contains(&("crashes".into(), 1, 1)));
        assert!(after.contains(&("exact_session_duration_ms".into(), 2, 16_000)));
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn housekeeping_crash_before_commit_is_all_or_nothing() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "crash-retention/detail", 0)
            .unwrap();
        let database = history.root().join("studio.sqlite3");
        history.shutdown();

        let old = now_ms().unwrap() - (100 * DAY_MS);
        let mut connection = Connection::open(&database).unwrap();
        connection
            .execute("UPDATE events SET observed_at_ms=?1", [old])
            .unwrap();

        let before_events: i64 = connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let before_epoch: i64 = connection
            .query_row(
                "SELECT retention_epoch FROM history_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(run_housekeeping_with_fault(&mut connection, true).is_err());

        let after_failed_events: i64 = connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let after_failed_epoch: i64 = connection
            .query_row(
                "SELECT retention_epoch FROM history_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let failed_watermark: Option<i64> = connection
            .query_row(
                "SELECT through_seq FROM projection_state
                 WHERE projection_code='metrics.v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();

        assert_eq!(after_failed_events, before_events);
        assert_eq!(after_failed_epoch, before_epoch);
        assert_eq!(failed_watermark, None);

        let report = run_housekeeping(&mut connection).unwrap();
        assert_eq!(report.pruned_events, 1);
        assert_eq!(report.retention_epoch, 1);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            connection
                .query_row(
                    "SELECT through_seq FROM projection_state
                     WHERE projection_code='metrics.v1'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                > 0
        );
    }

    #[test]
    fn retention_preserves_unexpired_audit_and_bounds_lineage() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let (operation, _) = history
            .admit_operation(
                SubjectKind::Mcp,
                HistoryAction::McpStart,
                ActorKind::LocalOperator,
            )
            .unwrap();
        history
            .finish_operation(&operation, OperationOutcome::Succeeded, None)
            .unwrap();
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "retention/detail", 0)
            .unwrap();
        let database = history.root().join("studio.sqlite3");
        history.shutdown();

        let old = now_ms().unwrap() - (100 * DAY_MS);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute("UPDATE events SET observed_at_ms=?1", [old])
            .unwrap();
        drop(connection);

        let history = HistoryHandle::initialize(&runtime_root);
        let report = history.housekeeping().unwrap();
        assert!(report.pruned_events >= 1);
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let audit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE retention_class='audit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let detail_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events
                 WHERE source_stream='retention/detail'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audit_count, 3);
        assert_eq!(detail_count, 0);
        assert!(report.retention_epoch > 0);
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn history_cursor_survives_insert_and_expires_on_prune() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "cursor/one", 0)
            .unwrap();
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "cursor/two", 0)
            .unwrap();

        let first = history
            .history_read(HistoryReadRequest::Events(HistoryListRequest {
                limit: Some(1),
                ..HistoryListRequest::default()
            }))
            .unwrap();
        let HistoryReadResponse::Events(first) = first else {
            panic!("expected event page");
        };
        let cursor = first.next_cursor.clone().unwrap();
        let snapshot_high = first.snapshot.through_seq.clone();

        history
            .record_bootstrap(uuid::Uuid::new_v4(), "cursor/new-head", 0)
            .unwrap();
        let second = history
            .history_read(HistoryReadRequest::Events(HistoryListRequest {
                cursor: Some(cursor.clone()),
                limit: Some(1),
                ..HistoryListRequest::default()
            }))
            .unwrap();
        let HistoryReadResponse::Events(second) = second else {
            panic!("expected event page");
        };
        assert_eq!(second.snapshot.through_seq, snapshot_high);
        assert!(
            second
                .items
                .iter()
                .all(|event| event.sequence <= snapshot_high)
        );

        let database = history.root().join("studio.sqlite3");
        history.shutdown();
        let old = now_ms().unwrap() - (100 * DAY_MS);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute(
                "UPDATE events SET observed_at_ms=?1
                 WHERE source_stream IN ('cursor/one','cursor/two')",
                [old],
            )
            .unwrap();
        drop(connection);

        let history = HistoryHandle::initialize(&runtime_root);
        history.housekeeping().unwrap();
        let expired = history.history_read(HistoryReadRequest::Events(HistoryListRequest {
            cursor: Some(cursor),
            limit: Some(1),
            ..HistoryListRequest::default()
        }));
        assert!(
            matches!(expired, Err(StudioError::History(message)) if message == "history_expired")
        );
        history.shutdown();
    }

    #[test]
    fn history_query_abuse_is_bounded() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);

        assert!(
            history
                .history_read(HistoryReadRequest::Events(HistoryListRequest {
                    limit: Some(201),
                    ..HistoryListRequest::default()
                }))
                .is_err()
        );
        assert!(
            history
                .history_read(HistoryReadRequest::Events(HistoryListRequest {
                    from_ms: Some(0),
                    to_ms: Some(MAX_PAGE_COUNT * DAY_MS),
                    ..HistoryListRequest::default()
                }))
                .is_err()
        );
        assert!(
            history
                .history_read(HistoryReadRequest::Events(HistoryListRequest {
                    cursor: Some("not-a-valid-cursor".into()),
                    ..HistoryListRequest::default()
                }))
                .is_err()
        );
        history.shutdown();
    }

    #[test]
    fn config_revision_return_to_old_content_is_new_observation() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let digest_a = "a".repeat(64);
        let digest_b = "b".repeat(64);

        history
            .start_run(uuid::Uuid::new_v4(), "config-a-1", Some(&digest_a))
            .unwrap();
        history.close_run(true).unwrap();
        history
            .start_run(uuid::Uuid::new_v4(), "config-b", Some(&digest_b))
            .unwrap();
        history.close_run(true).unwrap();
        history
            .start_run(uuid::Uuid::new_v4(), "config-a-2", Some(&digest_a))
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let rows: Vec<(String, String, Option<String>)> = {
            let mut statement = connection
                .prepare(
                    "SELECT revision_id,safe_projection_sha256,previous_revision_id
                     FROM config_revisions
                     WHERE surface_code='studio.config'
                     ORDER BY observed_seq",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].1, digest_a);
        assert_eq!(rows[1].1, digest_b);
        assert_eq!(rows[2].1, digest_a);
        assert_ne!(rows[0].0, rows[2].0);
        assert_eq!(rows[2].2.as_deref(), Some(rows[1].0.as_str()));
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn artifact_stage_and_verified_lineage_are_content_addressed() {
        use crate::update::{
            ArtifactHistoryIdentity, ComponentId, McpUpdatePhase, McpUpdateTransactionView, Version,
        };

        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "artifact-lineage-observer", None)
            .unwrap();

        let transaction_id = "txn-artifact-lineage";
        let staged = McpUpdateTransactionView {
            transaction_id: transaction_id.into(),
            component: ComponentId::Git,
            source_version: Some(Version::parse("1.0.0").unwrap()),
            target_version: Version::parse("1.1.0").unwrap(),
            phase: McpUpdatePhase::Staged,
            was_running: None,
            rollback_succeeded: None,
            error: None,
            updated_at_ms: 1,
        };
        let artifact = ArtifactHistoryIdentity {
            component: ComponentId::Git,
            version: "1.1.0".into(),
            provider_code: "github_13thx",
            platform_code: "darwin-arm64".into(),
            archive_sha256: "a".repeat(64),
            member_sha256: vec!["b".repeat(64), "c".repeat(64)],
        };
        let completed = McpUpdateTransactionView {
            phase: McpUpdatePhase::Completed,
            was_running: Some(true),
            updated_at_ms: 2,
            ..staged.clone()
        };

        // Reproduce the cross-queue race deterministically: terminal observations use the
        // reserved terminal queue and can become durable before an earlier staged observation.
        history.observe_update_transaction(&completed, None, true);
        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM install_observations
                     WHERE state='verified_active'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            drop(connection);
            if count == 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "terminal lineage observation did not become durable"
            );
            thread::sleep(Duration::from_millis(10));
        }

        history.observe_update_transaction_with_artifact(&staged, None, false, Some(artifact));
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let (staged_count, linked_active): (i64, i64) = connection
                .query_row(
                    "SELECT
                        SUM(CASE WHEN state='staged' THEN 1 ELSE 0 END),
                        SUM(CASE
                            WHEN state='verified_active' AND artifact_id IS NOT NULL THEN 1
                            ELSE 0 END)
                     FROM install_observations
                     WHERE state IN ('staged','verified_active')",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            drop(connection);
            if staged_count == 1 && linked_active == 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "late staged artifact did not backfill verified-active lineage"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM artifact_members", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let ids: Vec<Option<String>> = {
            let mut statement = connection
                .prepare(
                    "SELECT artifact_id FROM install_observations
                     WHERE state IN ('staged','verified_active') ORDER BY event_seq",
                )
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(Option::is_some));
        assert_eq!(ids[0], ids[1]);
        let (last_phase, install_outcome, rollback_outcome): (String, String, String) = connection
            .query_row(
                "SELECT last_phase, install_outcome, rollback_outcome
                 FROM update_attempts WHERE attempt_id=?1",
                [UpdateTransactionObservation::from_view(
                    &completed,
                    None,
                    OperationKind::Observation,
                )
                .attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(last_phase, "completed");
        assert_eq!(install_outcome, "succeeded");
        assert_eq!(rollback_outcome, "not_needed");
        let serialized: String = connection
            .query_row(
                "SELECT group_concat(payload_json,' ') FROM events",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!serialized.contains("staging_path"));
        assert!(!serialized.contains("asset_name"));
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn active_lineage_reports_unknown_predecessor() {
        use crate::update::{ComponentId, McpUpdatePhase, McpUpdateTransactionView, Version};

        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "lineage-observer", None)
            .unwrap();

        let view = McpUpdateTransactionView {
            transaction_id: "txn-lineage-first".into(),
            component: ComponentId::Git,
            source_version: Some(Version::parse("1.0.0").unwrap()),
            target_version: Version::parse("1.1.0").unwrap(),
            phase: McpUpdatePhase::Completed,
            was_running: Some(true),
            rollback_succeeded: None,
            error: None,
            updated_at_ms: 1,
        };
        history.observe_update_transaction(&view, None, false);

        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let subject_id = loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let subject: Option<String> = connection
                .query_row(
                    "SELECT subject_id FROM observed_lineage_heads LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            drop(connection);
            if let Some(subject) = subject {
                break subject;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "lineage observation did not become durable"
            );
            thread::sleep(Duration::from_millis(10));
        };

        let response = history
            .history_read(HistoryReadRequest::Lineage {
                subject_id: subject_id.clone(),
            })
            .unwrap();
        let HistoryReadResponse::Lineage(lineage) = response else {
            panic!("expected lineage response");
        };
        assert_eq!(lineage.subject_id, subject_id);
        assert!(lineage.latest_observation_id.is_some());
        assert_eq!(lineage.previous_observation_id, None);
        assert_eq!(
            lineage.boundary_reason.as_deref(),
            Some("unknown_predecessor")
        );
        assert_eq!(lineage.current_match, "unknown");
        assert!(!lineage.actionable);
        history.shutdown();
    }

    #[test]
    fn history_never_rehydrates_registry_authority() {
        use crate::registry::{RegisteredMcp, Registry, RuntimeKind};

        let fixture = runtime_fixture();
        let project_root = fixture.path().join("project");
        fs::create_dir_all(project_root.join("bin")).unwrap();
        let executable = project_root.join("bin/fixture");
        fs::write(
            &executable,
            "#!/bin/sh
exit 0
",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let registry = Registry::in_memory(fixture.path().to_path_buf(), BTreeMap::new()).unwrap();
        let view = registry
            .register(
                "fixture".into(),
                RegisteredMcp {
                    name: "Fixture".into(),
                    enabled: true,
                    runtime: RuntimeKind::Rust,
                    project_path: PathBuf::from("project"),
                    executable: PathBuf::from("bin/fixture"),
                    working_dir: PathBuf::from("."),
                    args: vec![],
                    env: BTreeMap::new(),
                },
            )
            .unwrap();

        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let (operation, _) = history
            .admit_operation(
                SubjectKind::Registry,
                HistoryAction::RegistryUpdate,
                ActorKind::LocalOperator,
            )
            .unwrap();
        history.observe_registry_revision(&view, false, operation.operation_id());
        history
            .finish_operation(&operation, OperationOutcome::Succeeded, None)
            .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let database = history.root().join("studio.sqlite3");
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM registry_entry_revisions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            drop(connection);
            if count == 1 {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            registry.get_view("fixture").is_some(),
            "historical projection must never mutate or rehydrate live registry authority"
        );
        assert_eq!(registry.list().len(), 1);
        history.shutdown();
    }

    #[test]
    fn written_config_is_not_loaded_config() {
        use crate::registry::{RegistryEntryView, RuntimeKind};

        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let loaded_digest = "a".repeat(64);
        history
            .start_run(
                uuid::Uuid::new_v4(),
                "config-separation-observer",
                Some(&loaded_digest),
            )
            .unwrap();

        let (operation, _) = history
            .admit_operation(
                SubjectKind::Registry,
                HistoryAction::RegistryUpdate,
                ActorKind::LocalOperator,
            )
            .unwrap();
        let view = RegistryEntryView {
            id: "safe-fixture".into(),
            name: "Safe Fixture".into(),
            enabled: true,
            runtime: RuntimeKind::Rust,
            project_path: PathBuf::from("/private/not-persisted/project"),
            executable: PathBuf::from("/private/not-persisted/bin"),
            working_dir: PathBuf::from("/private/not-persisted/work"),
            args: vec!["--secret-looking-but-not-persisted".into()],
        };
        history.observe_registry_revision(&view, true, operation.operation_id());
        history
            .finish_operation(&operation, OperationOutcome::Succeeded, None)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM config_revisions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            drop(connection);
            if count >= 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "config observations did not become durable"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let rows: Vec<(String, String, String)> = {
            let mut statement = connection
                .prepare(
                    "SELECT r.surface_code,r.provenance,l.relation
                     FROM config_revisions r
                     JOIN config_links l ON l.revision_id=r.revision_id
                     ORDER BY r.observed_seq",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(rows.contains(&("studio.config".into(), "loaded".into(), "loaded_by".into())));
        assert!(rows.contains(&(
            "registry.entry".into(),
            "committed".into(),
            "committed_by".into()
        )));

        let stored: String = connection
            .query_row(
                "SELECT group_concat(payload_json,' ') FROM events",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!stored.contains("/private/not-persisted"));
        assert!(!stored.contains("--secret-looking-but-not-persisted"));
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn history_housekeeping_never_invokes_runtime_policy() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "scope/bootstrap", 0)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let before: i64 = connection
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap();
        drop(connection);

        history.housekeeping().unwrap();

        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let after: i64 = connection
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap();
        let synthetic_updates: i64 = connection
            .query_row("SELECT COUNT(*) FROM update_attempts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 0);
        assert_eq!(after, 0);
        assert_eq!(synthetic_updates, 0);
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn binary_rollback_never_downgrades_history_schema() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let database = history.root().join("studio.sqlite3");
        history.shutdown();

        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(&format!("PRAGMA user_version={};", SCHEMA_VERSION + 1))
            .unwrap();
        drop(connection);

        let reopened = HistoryHandle::initialize(&runtime_root);
        assert_eq!(
            reopened.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION + 1
        );
    }

    #[test]
    fn historical_pid_never_authorizes_signal() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "pid-history-observer", None)
            .unwrap();
        let session = history
            .observe_session_started(LifecycleOwnerKind::Mcp, 1, 424_242)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM runtime_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            drop(connection);
            if count == 1 {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }

        let response = history
            .history_read(HistoryReadRequest::Sessions {
                subject_id: session.subject_id.to_string(),
                request: HistoryListRequest {
                    from_ms: Some(now_ms().unwrap().saturating_sub(DAY_MS)),
                    to_ms: Some(now_ms().unwrap()),
                    ..HistoryListRequest::default()
                },
            })
            .unwrap();
        let HistoryReadResponse::Sessions(page) = response else {
            panic!("expected session history");
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].observed_pid, "424242");
        let json = serde_json::to_string(&page.items[0]).unwrap();
        assert!(!json.contains("\"running\""));
        assert!(!json.contains("\"actionable\""));
        assert!(!json.contains("signal"));
        history.shutdown();
    }

    #[tokio::test]
    async fn history_reconnect_has_no_missed_commit() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let mut receiver = history.subscribe_events();

        history
            .record_bootstrap(uuid::Uuid::new_v4(), "realtime/commit", 0)
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let committed = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let event = tokio::time::timeout(remaining, receiver.recv())
                .await
                .expect("history commit notification timed out")
                .expect("history event channel closed");
            if let StudioEvent::HistoryCommitted {
                latest_seq,
                retention_epoch,
                ..
            } = event
            {
                break (latest_seq, retention_epoch);
            }
        };
        assert!(committed.0.parse::<u64>().unwrap() >= 1);
        assert_eq!(committed.1, "0");
        history.shutdown();
    }

    #[test]
    fn partial_history_is_not_zero_or_exact_duration() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "partial-history-one", None)
            .unwrap();
        let session = history
            .observe_session_started(LifecycleOwnerKind::Mcp, 1, 12345)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM runtime_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            drop(connection);
            if count == 1 {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }

        *history.inner.current_run.lock().unwrap() = None;
        history
            .start_run(uuid::Uuid::new_v4(), "partial-history-two", None)
            .unwrap();
        history.housekeeping().unwrap();

        let response = history
            .history_read(HistoryReadRequest::Sessions {
                subject_id: session.subject_id.to_string(),
                request: HistoryListRequest::default(),
            })
            .unwrap();
        let HistoryReadResponse::Sessions(page) = response else {
            panic!("expected sessions");
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].end_kind, "interrupted");
        assert_eq!(page.items[0].exact_duration_ms, None);

        let events = history
            .history_read(HistoryReadRequest::Events(HistoryListRequest::default()))
            .unwrap();
        let HistoryReadResponse::Events(events) = events else {
            panic!("expected events");
        };
        assert_eq!(events.coverage.state, "partial");
        assert!(
            events
                .coverage
                .reasons
                .iter()
                .any(|reason| reason == "interrupted")
        );

        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let exact_duration_metrics: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM metrics_totals
                 WHERE metric_code='exact_session_duration_ms'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exact_duration_metrics, 0);
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn long_reader_cannot_create_unbounded_history_work() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        for index in 0..260_u64 {
            history
                .record_bootstrap(uuid::Uuid::new_v4(), &format!("long-history/{index}"), 0)
                .unwrap();
        }

        let first = history
            .history_read(HistoryReadRequest::Events(HistoryListRequest {
                limit: Some(200),
                ..HistoryListRequest::default()
            }))
            .unwrap();
        let HistoryReadResponse::Events(first) = first else {
            panic!("expected events");
        };
        assert_eq!(first.items.len(), 200);
        assert!(first.next_cursor.is_some());

        let second = history
            .history_read(HistoryReadRequest::Events(HistoryListRequest {
                cursor: first.next_cursor,
                limit: Some(200),
                ..HistoryListRequest::default()
            }))
            .unwrap();
        let HistoryReadResponse::Events(second) = second else {
            panic!("expected events");
        };
        assert!(second.items.len() <= 60);
        assert_eq!(second.next_cursor, None);

        let oversized = history.history_read(HistoryReadRequest::Events(HistoryListRequest {
            limit: Some(201),
            ..HistoryListRequest::default()
        }));
        assert!(oversized.is_err());
        history.shutdown();
    }

    #[test]
    fn history_redaction_covers_db_api_and_backup() {
        use crate::registry::{RegistryEntryView, RuntimeKind};

        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .start_run(uuid::Uuid::new_v4(), "redaction-observer", None)
            .unwrap();
        let (operation, _) = history
            .admit_operation(
                SubjectKind::Registry,
                HistoryAction::RegistryUpdate,
                ActorKind::LocalOperator,
            )
            .unwrap();
        let secret = "M6-DO-NOT-PERSIST-SECRET";
        let view = RegistryEntryView {
            id: "redaction-fixture".into(),
            name: secret.into(),
            enabled: true,
            runtime: RuntimeKind::Rust,
            project_path: PathBuf::from(format!("/tmp/{secret}/project")),
            executable: PathBuf::from(format!("/tmp/{secret}/binary")),
            working_dir: PathBuf::from(format!("/tmp/{secret}/work")),
            args: vec![secret.into()],
        };
        history.observe_registry_revision(&view, true, operation.operation_id());
        history
            .finish_operation(&operation, OperationOutcome::Succeeded, None)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let connection =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM config_revisions
                     WHERE surface_code='registry.entry'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            drop(connection);
            if count == 1 {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }

        let response = history
            .history_read(HistoryReadRequest::ConfigRevisions(
                HistoryListRequest::default(),
            ))
            .unwrap();
        let HistoryReadResponse::ConfigRevisions(page) = response else {
            panic!("expected config revision page");
        };
        let api_json = serde_json::to_string(&page).unwrap();
        assert!(!api_json.contains(secret));
        assert!(!api_json.contains("/tmp/"));

        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let stored: String = connection
            .query_row(
                "SELECT group_concat(payload_json,' ') FROM events",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!stored.contains(secret));
        assert!(!stored.contains("/tmp/"));
        drop(connection);

        let backup = history.backup().unwrap();
        let backup_bytes = fs::read(backup).unwrap();
        let backup_text = String::from_utf8_lossy(&backup_bytes);
        assert!(!backup_text.contains(secret));
        assert!(!backup_text.contains("/tmp/"));
        history.shutdown();
    }

    #[test]
    fn m5_bootstrap_creates_observations_not_fake_events() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        history
            .record_bootstrap(uuid::Uuid::new_v4(), "bootstrap/m5", 0)
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (name, category, evidence): (String, String, String) = connection
            .query_row(
                "SELECT name,category,evidence_kind FROM events LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "history.bootstrap");
        assert_eq!(category, "observation");
        assert_eq!(evidence, "bootstrap");
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM operations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM update_attempts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM runtime_sessions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(connection);
        history.shutdown();
    }

    #[test]
    fn validated_journal_revision_conflicts_and_gaps_are_explicit() {
        let fixture = runtime_fixture();
        let runtime_root = create_runtime(fixture.path());
        let history = HistoryHandle::initialize(&runtime_root);
        let digest_a = "1".repeat(64);
        let digest_b = "2".repeat(64);

        history
            .record_validated_journal("tunnel", "txn-journal", 1, &digest_a, "prepared")
            .unwrap();
        history
            .record_validated_journal("tunnel", "txn-journal", 1, &digest_a, "prepared")
            .unwrap();
        assert!(
            history
                .record_validated_journal("tunnel", "txn-journal", 1, &digest_b, "prepared")
                .is_err()
        );
        history
            .record_validated_journal("tunnel", "txn-journal", 3, &digest_b, "committed")
            .unwrap();

        let database = history.root().join("studio.sqlite3");
        let connection =
            Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let watermark: i64 = connection
            .query_row(
                "SELECT highest_revision FROM journal_watermarks
                 WHERE domain='tunnel' AND transaction_id='txn-journal'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let gaps: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM coverage_intervals WHERE reason='revision_gap'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(watermark, 3);
        assert_eq!(gaps, 1);
        drop(connection);
        history.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_history_ancestor_before_creation() {
        let fixture = runtime_fixture();
        let runtime = create_runtime(fixture.path());
        let target = fixture.path().join("target");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, runtime.join("studio")).unwrap();

        let history = HistoryHandle::initialize(&runtime);
        assert_eq!(
            history.health(),
            HistoryHealth::Degraded {
                code: "history_open_failed"
            }
        );
        assert!(!target.join("data/history").exists());
    }
}
