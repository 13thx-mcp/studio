use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use uuid::Uuid;

use crate::error::{StudioError, StudioResult};

use super::events::{ActorKind, HistoryAction, SubjectKind};

pub const MAX_ACTIVE_OPERATIONS: usize = 32;

#[derive(Debug, Clone)]
pub struct OperationContext {
    operation_id: Uuid,
    subject_id: Uuid,
    action: HistoryAction,
    actor: ActorKind,
    subject_kind: SubjectKind,
    source_stream: String,
    run_id: Option<Uuid>,
    completion_claimed: Arc<AtomicBool>,
}

impl OperationContext {
    pub(crate) fn new(
        operation_id: Uuid,
        subject_id: Uuid,
        action: HistoryAction,
        actor: ActorKind,
        subject_kind: SubjectKind,
        run_id: Option<Uuid>,
    ) -> Self {
        Self {
            operation_id,
            subject_id,
            action,
            actor,
            subject_kind,
            source_stream: format!("operation/{operation_id}"),
            run_id,
            completion_claimed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub fn subject_id(&self) -> Uuid {
        self.subject_id
    }

    pub fn action(&self) -> HistoryAction {
        self.action
    }

    pub fn actor(&self) -> ActorKind {
        self.actor
    }

    pub fn subject_kind(&self) -> SubjectKind {
        self.subject_kind
    }

    pub(crate) fn run_id(&self) -> Option<Uuid> {
        self.run_id
    }

    pub(crate) fn source_stream(&self) -> &str {
        &self.source_stream
    }

    pub(crate) fn claim_completion(&self) -> StudioResult<()> {
        self.completion_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| StudioError::History("history operation already completed".into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAdmissionReceipt {
    pub operation_id: Uuid,
    pub admitted_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationTerminalReceipt {
    pub operation_id: Uuid,
    pub terminal_seq: u64,
}
