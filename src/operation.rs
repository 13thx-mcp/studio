use crate::{
    error::{StudioError, StudioResult},
    storage::{
        ActorKind, HistoryAction, HistoryHandle, OperationContext, OperationOutcome,
        OperationTerminalReceipt, SubjectKind,
    },
};

#[derive(Clone)]
pub struct OperationService {
    history: HistoryHandle,
}

impl OperationService {
    pub fn new(history: HistoryHandle) -> Self {
        Self { history }
    }

    pub async fn admit(
        &self,
        subject_kind: SubjectKind,
        action: HistoryAction,
        actor: ActorKind,
    ) -> StudioResult<OperationContext> {
        let history = self.history.clone();
        tokio::task::spawn_blocking(move || {
            history
                .admit_operation(subject_kind, action, actor)
                .map(|(context, _)| context)
        })
        .await
        .map_err(|error| StudioError::History(format!("history admission task failed: {error}")))?
    }

    pub async fn finish(
        &self,
        context: &OperationContext,
        outcome: OperationOutcome,
        error_code: Option<&str>,
    ) -> StudioResult<OperationTerminalReceipt> {
        let history = self.history.clone();
        let context = context.clone();
        let error_code = error_code.map(ToOwned::to_owned);
        tokio::task::spawn_blocking(move || {
            history.finish_operation(&context, outcome, error_code.as_deref())
        })
        .await
        .map_err(|error| {
            StudioError::History(format!("history terminal receipt task failed: {error}"))
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_service_fails_admission_closed_when_history_is_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let history = HistoryHandle::initialize(root.path());
        let service = OperationService::new(history.clone());
        history.shutdown();

        assert!(
            service
                .admit(
                    SubjectKind::System,
                    HistoryAction::UpdateCheck,
                    ActorKind::LocalOperator,
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn shared_service_preserves_admission_and_terminal_semantics() {
        let root = tempfile::tempdir().unwrap();
        let history = HistoryHandle::initialize(root.path());
        let run_id = uuid::Uuid::new_v4();
        history
            .start_run(run_id, "operation-service-test", None)
            .unwrap();
        history.mark_run_ready().unwrap();

        let service = OperationService::new(history.clone());
        let context = service
            .admit(
                SubjectKind::System,
                HistoryAction::UpdateCheck,
                ActorKind::LocalOperator,
            )
            .await
            .unwrap();
        let operation_id = context.operation_id();
        let receipt = service
            .finish(&context, OperationOutcome::Succeeded, None)
            .await
            .unwrap();
        assert_eq!(receipt.operation_id, operation_id);
        history.shutdown();
    }
}
