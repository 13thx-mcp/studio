mod history;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use tokio::{
    sync::{Notify, broadcast::error::RecvError},
    time::{Duration, sleep},
};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    discovery::{DiscoveredProject, DiscoveryService, RegisterDiscoveryRequest},
    error::StudioError,
    operation::OperationService,
    realtime::{EventHub, StudioEvent},
    registry::{RegistryEntryView, RegistryUpdate},
    storage::{
        ActorKind, HistoryAction, HistoryHandle, OperationContext, OperationOutcome, SubjectKind,
    },
    supervisor::{LogEntry, ProcessStatus, Supervisor},
    tunnel::{TunnelLogEntry, TunnelStatus, TunnelSupervisor},
    update::{
        ApplyMcpUpdateRequest, ComponentId, FleetUpdateManager, GatewayUpdateManager,
        InventoryService, InventoryView, McpUpdateManager, McpUpdateTransactionView,
        PrepareMcpUpdateRequest, ReconciliationView, RuntimeOperationCoordinator,
        RuntimeOperationGuard, RuntimeReconciler, SelfUpdateManager, TunnelUpdateManager,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub supervisor: Arc<Supervisor>,
    pub discovery: Arc<DiscoveryService>,
    pub tunnel: Arc<TunnelSupervisor>,
    pub inventory: Arc<InventoryService>,
    pub updates: Arc<McpUpdateManager>,
    pub gateway_updates: Arc<GatewayUpdateManager>,
    pub fleet_updates: Arc<FleetUpdateManager>,
    pub self_updates: Arc<SelfUpdateManager>,
    pub tunnel_updates: Arc<TunnelUpdateManager>,
    pub reconciliation: Arc<RuntimeReconciler>,
    pub runtime_operations: Arc<RuntimeOperationCoordinator>,
    pub operations: OperationService,
    pub history: HistoryHandle,
    pub shutdown: Arc<Notify>,
    pub registry_events: EventHub,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct ApiStatusResponse {
    status: &'static str,
    managed_mcp_count: usize,
    tunnel_state: crate::tunnel::TunnelState,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn router(state: AppState) -> Router {
    let web = ServeDir::new("web/dist").not_found_service(ServeFile::new("web/dist/index.html"));

    Router::new()
        .route("/health", get(health))
        .route("/api/status", get(api_status))
        .route("/api/mcp", get(list_mcp))
        .route("/api/mcp/{id}", get(get_mcp))
        .route("/api/mcp/{id}/start", post(start_mcp))
        .route("/api/mcp/{id}/stop", post(stop_mcp))
        .route("/api/mcp/{id}/restart", post(restart_mcp))
        .route("/api/mcp/{id}/logs", get(get_logs))
        .route("/api/registry", get(list_registry))
        .route(
            "/api/registry/{id}",
            get(get_registry)
                .put(update_registry)
                .delete(unregister_mcp),
        )
        .route("/api/registry/{id}/enable", post(enable_mcp))
        .route("/api/registry/{id}/disable", post(disable_mcp))
        .route("/api/discovery", get(list_discovery))
        .route("/api/discovery/scan", post(scan_discovery))
        .route(
            "/api/discovery/{candidate_id}/register",
            post(register_discovery),
        )
        .route("/api/tunnel", get(get_tunnel))
        .route("/api/tunnel/diagnostics", get(get_tunnel_diagnostics))
        .route("/api/tunnel/health", get(get_tunnel_health))
        .route("/api/tunnel/start", post(start_tunnel))
        .route("/api/tunnel/stop", post(stop_tunnel))
        .route("/api/tunnel/restart", post(restart_tunnel))
        .route("/api/tunnel/logs", get(get_tunnel_logs))
        .route("/api/updates", get(list_updates))
        .route("/api/updates/check", post(check_updates))
        .route("/api/updates/{component}", get(get_update))
        .route("/api/updates/{component}/prepare", post(prepare_mcp_update))
        .route("/api/updates/{component}/apply", post(apply_mcp_update))
        .route(
            "/api/update-transactions/{transaction_id}",
            get(get_update_transaction),
        )
        .route("/api/reconciliation", get(get_reconciliation))
        .route("/api/reconciliation/check", post(check_reconciliation))
        .route("/api/reconciliation/adopt", post(adopt_reconciliation))
        .route("/api/reconciliation/apply", post(apply_reconciliation))
        .nest("/api/history/v1", history::router())
        .route("/api/ws", get(websocket))
        .fallback_service(web)
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "mcp-studio",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn api_status(State(state): State<AppState>) -> Json<ApiStatusResponse> {
    let servers = state.supervisor.list().await;
    let tunnel = state.tunnel.status().await;
    Json(ApiStatusResponse {
        status: "ok",
        managed_mcp_count: servers.len(),
        tunnel_state: tunnel.state,
    })
}

async fn list_mcp(State(state): State<AppState>) -> Json<Vec<ProcessStatus>> {
    Json(state.supervisor.list().await)
}

async fn get_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProcessStatus>, ApiError> {
    Ok(Json(state.supervisor.status(&id).await?))
}

const OPERATION_ID_HEADER: &str = "x-studio-operation-id";
const AUDIT_STATUS_HEADER: &str = "x-studio-audit-status";
const EFFECT_STATUS_HEADER: &str = "x-studio-effect-status";

async fn admit_local_operation(
    state: &AppState,
    subject_kind: SubjectKind,
    action: HistoryAction,
) -> Result<OperationContext, ApiError> {
    state
        .operations
        .admit(subject_kind, action, ActorKind::LocalOperator)
        .await
        .map_err(Into::into)
}

async fn audited_result_response<T: Serialize + Send + 'static>(
    state: &AppState,
    context: &OperationContext,
    result: Result<T, StudioError>,
) -> Response {
    let (mut response, outcome, effect_status, error_code) = match result {
        Ok(value) => (
            Json(value).into_response(),
            OperationOutcome::Succeeded,
            "succeeded",
            None,
        ),
        Err(error) => (
            ApiError::from(error).into_response(),
            OperationOutcome::Failed,
            "failed",
            Some("domain_error"),
        ),
    };

    let operation_id = context.operation_id();
    let audit_status = match state.operations.finish(context, outcome, error_code).await {
        Ok(_) => "complete",
        Err(error) => {
            tracing::warn!(
                operation_id = %operation_id,
                history_error = %error,
                "domain operation completed with incomplete audit terminal evidence"
            );
            "incomplete"
        }
    };

    if let Ok(value) = HeaderValue::from_str(&operation_id.to_string()) {
        response.headers_mut().insert(OPERATION_ID_HEADER, value);
    }
    response
        .headers_mut()
        .insert(AUDIT_STATUS_HEADER, HeaderValue::from_static(audit_status));
    response.headers_mut().insert(
        EFFECT_STATUS_HEADER,
        HeaderValue::from_static(effect_status),
    );
    response
}

fn acquire_mcp_operation(
    state: &AppState,
    id: &str,
    owner: &'static str,
) -> Result<RuntimeOperationGuard, ApiError> {
    let coordinator = state.runtime_operations.clone();
    match id.parse::<ComponentId>() {
        Ok(component)
            if matches!(
                component,
                ComponentId::Filesystem
                    | ComponentId::Git
                    | ComponentId::Exec
                    | ComponentId::Blender
            ) =>
        {
            Ok(coordinator.acquire_component(component, owner)?)
        }
        _ => Ok(coordinator.acquire_control(owner)?),
    }
}

async fn start_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "mcp_start")?;
    let operation =
        admit_local_operation(&state, SubjectKind::Mcp, HistoryAction::McpStart).await?;
    let result = state.supervisor.start(&id).await;
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn stop_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ProcessStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "mcp_stop")?;
    Ok(Json(state.supervisor.stop(&id).await?))
}

async fn restart_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "mcp_restart")?;
    let operation =
        admit_local_operation(&state, SubjectKind::Mcp, HistoryAction::McpRestart).await?;
    let result = state.supervisor.restart(&id).await;
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn get_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<LogEntry>>, ApiError> {
    Ok(Json(state.supervisor.logs(&id).await?))
}

async fn list_registry(State(state): State<AppState>) -> Json<Vec<RegistryEntryView>> {
    Json(state.supervisor.registry().list())
}

async fn get_registry(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RegistryEntryView>, ApiError> {
    state
        .supervisor
        .registry()
        .get_view(&id)
        .map(Json)
        .ok_or_else(|| StudioError::NotFound(id).into())
}

async fn update_registry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(update): Json<RegistryUpdate>,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "registry_update")?;
    require_inactive(&state.supervisor, &id, "edit").await?;
    let operation =
        admit_local_operation(&state, SubjectKind::Registry, HistoryAction::RegistryUpdate).await?;
    let result = state.supervisor.registry().update(&id, update);
    if let Ok(view) = &result {
        state
            .history
            .observe_registry_revision(view, true, operation.operation_id());
        state.registry_events.publish(StudioEvent::RegistryChanged);
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn enable_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "registry_enable")?;
    let operation =
        admit_local_operation(&state, SubjectKind::Registry, HistoryAction::RegistryEnable).await?;
    let result = state.supervisor.registry().set_enabled(&id, true);
    if let Ok(view) = &result {
        state
            .history
            .observe_registry_revision(view, true, operation.operation_id());
        state.registry_events.publish(StudioEvent::RegistryChanged);
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn disable_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "registry_disable")?;
    require_inactive(&state.supervisor, &id, "disable").await?;
    let operation = admit_local_operation(
        &state,
        SubjectKind::Registry,
        HistoryAction::RegistryDisable,
    )
    .await?;
    let result = state.supervisor.registry().set_enabled(&id, false);
    if let Ok(view) = &result {
        state
            .history
            .observe_registry_revision(view, true, operation.operation_id());
        state.registry_events.publish(StudioEvent::RegistryChanged);
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn unregister_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = acquire_mcp_operation(&state, &id, "registry_unregister")?;
    require_inactive(&state.supervisor, &id, "unregister").await?;
    let operation = admit_local_operation(
        &state,
        SubjectKind::Registry,
        HistoryAction::RegistryUnregister,
    )
    .await?;

    let result = match state.supervisor.registry().unregister(&id) {
        Ok(view) => match state.supervisor.remove_inactive_runtime(&id).await {
            Ok(()) => Ok(view),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    if let Ok(view) = &result {
        state
            .history
            .observe_registry_revision(view, false, operation.operation_id());
        state.registry_events.publish(StudioEvent::RegistryChanged);
        state.registry_events.publish(StudioEvent::DiscoveryChanged);
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn list_discovery(
    State(state): State<AppState>,
) -> Result<Json<Vec<DiscoveredProject>>, ApiError> {
    Ok(Json(state.discovery.scan()?))
}

async fn scan_discovery(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DiscoveredProject>>, ApiError> {
    ensure_same_origin(&headers)?;
    let projects = state.discovery.scan()?;
    state.registry_events.publish(StudioEvent::DiscoveryChanged);
    Ok(Json(projects))
}

async fn register_discovery(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<RegisterDiscoveryRequest>,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = state
        .runtime_operations
        .acquire_control("registry_register")?;
    let operation = admit_local_operation(
        &state,
        SubjectKind::Registry,
        HistoryAction::DiscoveryApprove,
    )
    .await?;

    let id = request.id.clone();
    let result = state
        .discovery
        .approve(&candidate_id, request)
        .and_then(|server| state.supervisor.registry().register(id, server));
    if let Ok(view) = &result {
        state
            .history
            .observe_registry_revision(view, true, operation.operation_id());
        state.registry_events.publish(StudioEvent::RegistryChanged);
        state.registry_events.publish(StudioEvent::DiscoveryChanged);
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn require_inactive(supervisor: &Supervisor, id: &str, action: &str) -> Result<(), ApiError> {
    if supervisor.is_active(id).await? {
        return Err(StudioError::Conflict(format!("{id} must be stopped before {action}")).into());
    }
    Ok(())
}

async fn get_tunnel(State(state): State<AppState>) -> Json<TunnelStatus> {
    Json(state.tunnel.status().await)
}

async fn get_tunnel_diagnostics(
    State(state): State<AppState>,
) -> Json<crate::tunnel::TunnelDiagnosticStatus> {
    Json(state.tunnel.diagnostics().await)
}

async fn get_tunnel_health(
    State(state): State<AppState>,
) -> Json<crate::tunnel::TunnelHealthStatus> {
    Json(state.tunnel.health().await)
}

async fn start_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = state.runtime_operations.acquire_control("tunnel_start")?;
    let operation =
        admit_local_operation(&state, SubjectKind::Tunnel, HistoryAction::TunnelStart).await?;
    let result = state.tunnel.start().await;
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn stop_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TunnelStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = state.runtime_operations.acquire_control("tunnel_stop")?;
    Ok(Json(state.tunnel.stop().await?))
}

async fn restart_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let _runtime_guard = state.runtime_operations.acquire_control("tunnel_restart")?;
    let operation =
        admit_local_operation(&state, SubjectKind::Tunnel, HistoryAction::TunnelRestart).await?;
    let result = state.tunnel.restart().await;
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn get_tunnel_logs(State(state): State<AppState>) -> Json<Vec<TunnelLogEntry>> {
    Json(state.tunnel.logs().await)
}

async fn list_updates(State(state): State<AppState>) -> Json<Vec<InventoryView>> {
    Json(state.inventory.list().await)
}

async fn get_update(
    State(state): State<AppState>,
    Path(component): Path<String>,
) -> Result<Json<InventoryView>, ApiError> {
    let component: ComponentId = component.parse()?;
    Ok(Json(state.inventory.get(component).await?))
}

async fn check_updates(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let operation =
        admit_local_operation(&state, SubjectKind::Component, HistoryAction::UpdateCheck).await?;
    let inventory = state.inventory.check().await;
    for view in &inventory {
        state
            .history
            .observe_update_check(view, operation.operation_id());
    }
    state.registry_events.publish(StudioEvent::UpdatesChanged);
    Ok(audited_result_response(&state, &operation, Ok::<_, StudioError>(inventory)).await)
}

async fn prepared_artifact_identity(
    state: &AppState,
    component: ComponentId,
    transaction_id: &str,
) -> Option<crate::update::ArtifactHistoryIdentity> {
    match component {
        ComponentId::Gateway => {
            state
                .gateway_updates
                .artifact_history_identity(transaction_id)
                .await
        }
        ComponentId::Fleet => {
            state
                .fleet_updates
                .artifact_history_identity(transaction_id)
                .await
        }
        ComponentId::Studio => state
            .self_updates
            .artifact_history_identity(transaction_id)
            .ok()
            .flatten(),
        ComponentId::Tunnel => {
            state
                .tunnel_updates
                .artifact_history_identity(transaction_id)
                .await
        }
        _ => {
            state
                .updates
                .artifact_history_identity(component, transaction_id)
                .await
        }
    }
}

async fn prepare_mcp_update(
    State(state): State<AppState>,
    Path(component): Path<String>,
    headers: HeaderMap,
    Json(request): Json<PrepareMcpUpdateRequest>,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let component: ComponentId = component.parse()?;
    let operation =
        admit_local_operation(&state, SubjectKind::Component, HistoryAction::UpdatePrepare).await?;
    let result = match component {
        ComponentId::Gateway => state.gateway_updates.prepare(request.version).await,
        ComponentId::Fleet => state.fleet_updates.prepare(request.version).await,
        ComponentId::Studio => state.self_updates.prepare(request.version).await,
        ComponentId::Tunnel => state.tunnel_updates.prepare(request.version).await,
        _ => state.updates.prepare(component, request.version).await,
    };
    if let Ok(view) = &result {
        let artifact = prepared_artifact_identity(&state, component, &view.transaction_id).await;
        state.history.observe_update_transaction_with_artifact(
            view,
            Some(operation.operation_id()),
            false,
            artifact,
        );
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn apply_mcp_update(
    State(state): State<AppState>,
    Path(component): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ApplyMcpUpdateRequest>,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let component: ComponentId = component.parse()?;
    let operation =
        admit_local_operation(&state, SubjectKind::Component, HistoryAction::UpdateApply).await?;
    let result = match component {
        ComponentId::Gateway => state.gateway_updates.apply(&request.transaction_id).await,
        ComponentId::Fleet => state.fleet_updates.apply(&request.transaction_id).await,
        ComponentId::Studio => match state.self_updates.apply(&request.transaction_id).await {
            Ok(transaction) => {
                let shutdown = state.shutdown.clone();
                tokio::spawn(async move {
                    sleep(Duration::from_millis(250)).await;
                    shutdown.notify_waiters();
                });
                Ok(transaction)
            }
            Err(error) => Err(error),
        },
        ComponentId::Tunnel => state.tunnel_updates.apply(&request.transaction_id).await,
        _ => {
            state
                .updates
                .apply(component, &request.transaction_id)
                .await
        }
    };

    let observed = match &result {
        Ok(view) => Some(view.clone()),
        Err(_) => match component {
            ComponentId::Gateway => state
                .gateway_updates
                .transaction(&request.transaction_id)
                .await
                .ok(),
            ComponentId::Fleet => state
                .fleet_updates
                .transaction(&request.transaction_id)
                .await
                .ok(),
            ComponentId::Studio => state
                .self_updates
                .transaction(&request.transaction_id)
                .await
                .ok(),
            ComponentId::Tunnel => state
                .tunnel_updates
                .transaction(&request.transaction_id)
                .await
                .ok(),
            _ => state
                .updates
                .transaction(&request.transaction_id)
                .await
                .ok(),
        },
    };
    if let Some(view) = observed.as_ref() {
        state
            .history
            .observe_update_transaction(view, Some(operation.operation_id()), true);
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn get_update_transaction(
    State(state): State<AppState>,
    Path(transaction_id): Path<String>,
) -> Result<Json<McpUpdateTransactionView>, ApiError> {
    match state.updates.transaction(&transaction_id).await {
        Ok(transaction) => Ok(Json(transaction)),
        Err(StudioError::NotFound(_)) => {
            match state.gateway_updates.transaction(&transaction_id).await {
                Ok(transaction) => Ok(Json(transaction)),
                Err(StudioError::NotFound(_)) => {
                    match state.fleet_updates.transaction(&transaction_id).await {
                        Ok(transaction) => Ok(Json(transaction)),
                        Err(StudioError::NotFound(_)) => {
                            match state.self_updates.transaction(&transaction_id).await {
                                Ok(transaction) => Ok(Json(transaction)),
                                Err(StudioError::NotFound(_)) => Ok(Json(
                                    state.tunnel_updates.transaction(&transaction_id).await?,
                                )),
                                Err(error) => Err(error.into()),
                            }
                        }
                        Err(error) => Err(error.into()),
                    }
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

async fn get_reconciliation(State(state): State<AppState>) -> Json<ReconciliationView> {
    Json(state.reconciliation.status().await)
}

async fn check_reconciliation(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let operation = admit_local_operation(
        &state,
        SubjectKind::Fleet,
        HistoryAction::ReconciliationCheck,
    )
    .await?;
    let result = state.reconciliation.check().await;
    if let Ok(view) = &result {
        state
            .history
            .observe_drift(view, operation.operation_id(), "check");
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn adopt_reconciliation(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let operation = admit_local_operation(
        &state,
        SubjectKind::Fleet,
        HistoryAction::ReconciliationAdopt,
    )
    .await?;
    let result = state.reconciliation.adopt().await;
    if let Ok(view) = &result {
        state
            .history
            .observe_drift(view, operation.operation_id(), "adopt");
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn apply_reconciliation(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    let operation = admit_local_operation(
        &state,
        SubjectKind::Fleet,
        HistoryAction::ReconciliationApply,
    )
    .await?;
    let result = state.reconciliation.apply().await;
    if let Ok(view) = &result {
        state
            .history
            .observe_drift(view, operation.operation_id(), "apply");
    }
    Ok(audited_result_response(&state, &operation, result).await)
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(ws
        .on_upgrade(move |socket| websocket_session(socket, state))
        .into_response())
}

async fn websocket_session(mut socket: WebSocket, state: AppState) {
    let mut mcp_receiver = state.supervisor.subscribe_events();
    let mut tunnel_receiver = state.tunnel.subscribe_events();
    let mut registry_receiver = state.registry_events.subscribe();
    let mut history_receiver = state.history.subscribe_events();

    if send_snapshot(&mut socket, &state).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            result = mcp_receiver.recv() => {
                if handle_event_result(&mut socket, &state, result).await.is_err() { break; }
            }
            result = tunnel_receiver.recv() => {
                if handle_event_result(&mut socket, &state, result).await.is_err() { break; }
            }
            result = registry_receiver.recv() => {
                if handle_event_result(&mut socket, &state, result).await.is_err() { break; }
            }
            result = history_receiver.recv() => {
                if handle_event_result(&mut socket, &state, result).await.is_err() { break; }
            }
        }
    }
}

async fn handle_event_result(
    socket: &mut WebSocket,
    state: &AppState,
    result: Result<StudioEvent, RecvError>,
) -> Result<(), ()> {
    match result {
        Ok(event) => send_event(socket, &event).await,
        Err(RecvError::Lagged(_)) => {
            send_event(socket, &StudioEvent::ResyncRequired).await?;
            send_snapshot(socket, state).await
        }
        Err(RecvError::Closed) => Err(()),
    }
}

async fn send_snapshot(socket: &mut WebSocket, state: &AppState) -> Result<(), ()> {
    send_event(
        socket,
        &StudioEvent::Snapshot {
            servers: state.supervisor.list().await,
            tunnel: state.tunnel.status().await,
        },
    )
    .await?;
    send_event(
        socket,
        &StudioEvent::ReconciliationChanged {
            status: state.reconciliation.status().await,
        },
    )
    .await
}

async fn send_event(socket: &mut WebSocket, event: &StudioEvent) -> Result<(), ()> {
    let payload = serde_json::to_string(event).map_err(|_| ())?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|_| ())
}

fn ensure_same_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(origin) = headers.get(axum::http::header::ORIGIN) else {
        return Ok(());
    };
    let Some(host) = headers.get(axum::http::header::HOST) else {
        return Err(ApiError::forbidden(
            "missing Host header for browser request",
        ));
    };

    let origin = origin
        .to_str()
        .map_err(|_| ApiError::forbidden("invalid Origin header"))?;
    let host = host
        .to_str()
        .map_err(|_| ApiError::forbidden("invalid Host header"))?;

    let allowed_http = format!("http://{host}");
    let allowed_https = format!("https://{host}");
    if origin == allowed_http || origin == allowed_https {
        Ok(())
    } else {
        Err(ApiError::forbidden("cross-origin browser request rejected"))
    }
}

struct ApiError {
    error: StudioError,
    status_override: Option<StatusCode>,
}

impl ApiError {
    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            error: StudioError::Config(message.into()),
            status_override: Some(StatusCode::FORBIDDEN),
        }
    }
}

impl From<StudioError> for ApiError {
    fn from(value: StudioError) -> Self {
        Self {
            error: value,
            status_override: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_override.unwrap_or(match &self.error {
            StudioError::NotFound(_)
            | StudioError::UnknownComponent(_)
            | StudioError::NoStableRelease { .. }
            | StudioError::ReleaseVersionNotFound { .. } => StatusCode::NOT_FOUND,
            StudioError::Duplicate(_)
            | StudioError::Disabled(_)
            | StudioError::Conflict(_)
            | StudioError::AlreadyRunning(_)
            | StudioError::NotRunning(_) => StatusCode::CONFLICT,
            StudioError::Config(_)
            | StudioError::Unsupported(_)
            | StudioError::ReleaseNotEligible { .. }
            | StudioError::UnsupportedOperatingSystem(_)
            | StudioError::UnsupportedArchitecture(_) => StatusCode::BAD_REQUEST,
            StudioError::ReleaseProviderUnreachable { .. }
            | StudioError::ReleaseProviderHttp { .. }
            | StudioError::ReleaseProviderResponseTooLarge { .. }
            | StudioError::MalformedReleaseMetadata { .. }
            | StudioError::InvalidReleaseTag { .. }
            | StudioError::MissingChecksumManifest { .. }
            | StudioError::DuplicateChecksumManifest { .. }
            | StudioError::UntrustedReleaseAssetUrl { .. }
            | StudioError::MissingReleaseAssetFamily { .. }
            | StudioError::AmbiguousReleaseAssetFamily { .. }
            | StudioError::ReleaseComponentMismatch { .. }
            | StudioError::ReleaseAssetNotFound { .. }
            | StudioError::AmbiguousReleaseAsset { .. }
            | StudioError::ReleaseProviderMismatch { .. }
            | StudioError::ArtifactDownloadFailed { .. }
            | StudioError::ArtifactTooLarge { .. }
            | StudioError::InvalidChecksumManifest { .. }
            | StudioError::ChecksumEntryMissing { .. }
            | StudioError::ChecksumEntryAmbiguous { .. }
            | StudioError::ChecksumMismatch { .. }
            | StudioError::UnsafeArchive { .. }
            | StudioError::PackageValidationFailed { .. }
            | StudioError::BinaryVersionValidationFailed { .. } => StatusCode::BAD_GATEWAY,
            StudioError::History(_)
            | StudioError::Automation(_)
            | StudioError::Process(_)
            | StudioError::InstalledIdentity(_)
            | StudioError::StagingFailure(_)
            | StudioError::UpdateTransaction(_)
            | StudioError::UpdateActivationFailed { .. }
            | StudioError::UpdateVerificationFailed { .. }
            | StudioError::RollbackFailed { .. }
            | StudioError::Io(_)
            | StudioError::Toml(_)
            | StudioError::TomlEncode(_)
            | StudioError::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
        });
        let error = public_error_message(&self.error);
        (status, Json(ErrorResponse { error })).into_response()
    }
}

fn public_error_message(error: &StudioError) -> String {
    match error {
        StudioError::ReleaseProviderUnreachable { .. } => "release_provider_unreachable".into(),
        StudioError::ReleaseProviderHttp { status, .. } => {
            format!("release_provider_http_{status}")
        }
        StudioError::ReleaseProviderResponseTooLarge { .. } => {
            "release_provider_response_too_large".into()
        }
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
        StudioError::ReleaseComponentMismatch { .. } => "release_component_mismatch".into(),
        StudioError::ReleaseAssetNotFound { .. } => "release_asset_not_found".into(),
        StudioError::AmbiguousReleaseAsset { .. } => "ambiguous_release_asset".into(),
        StudioError::ReleaseProviderMismatch { .. } => "release_provider_mismatch".into(),
        StudioError::ArtifactDownloadFailed { .. } => "artifact_download_failed".into(),
        StudioError::ArtifactTooLarge { .. } => "artifact_too_large".into(),
        StudioError::InvalidChecksumManifest { .. } => "invalid_checksum_manifest".into(),
        StudioError::ChecksumEntryMissing { .. } => "checksum_entry_missing".into(),
        StudioError::ChecksumEntryAmbiguous { .. } => "checksum_entry_ambiguous".into(),
        StudioError::ChecksumMismatch { .. } => "checksum_mismatch".into(),
        StudioError::UnsafeArchive { .. } => "unsafe_archive".into(),
        StudioError::PackageValidationFailed { .. } => "package_validation_failed".into(),
        StudioError::BinaryVersionValidationFailed { .. } => {
            "binary_version_validation_failed".into()
        }
        StudioError::History(_) => "history_storage_failed".into(),
        StudioError::Automation(_) => "automation_failed".into(),
        StudioError::StagingFailure(_) => "staging_failed".into(),
        StudioError::InstalledIdentity(_) => "installed_identity_error".into(),
        StudioError::UpdateTransaction(_) => "update_transaction_failed".into(),
        StudioError::UpdateActivationFailed { .. } => "update_activation_failed".into(),
        StudioError::UpdateVerificationFailed { .. } => "update_verification_failed".into(),
        StudioError::RollbackFailed { .. } => "rollback_failed".into(),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use crate::{
        discovery::DiscoveryService,
        error::StudioError,
        realtime::EventHub,
        registry::Registry,
        supervisor::Supervisor,
        tunnel::{TunnelConfig, TunnelSupervisor},
        update::{
            ComponentCatalog, FleetUpdateManager, GatewayUpdateManager, HostPlatform,
            HostRuntimeRoots, InventoryService, McpUpdateManager, RuntimeReconciler,
            SelfUpdateManager, TunnelUpdateManager,
        },
    };

    fn test_router() -> axum::Router {
        let registry =
            Registry::in_memory(std::env::current_dir().unwrap(), BTreeMap::new()).unwrap();
        let supervisor = Arc::new(Supervisor::new(
            registry.clone(),
            32,
            Duration::from_millis(100),
        ));
        let discovery = DiscoveryService::new(registry);
        let tunnel = TunnelSupervisor::new(
            TunnelConfig {
                name: "Test tunnel".into(),
                runtime: PathBuf::from("missing-runtime"),
                working_dir: PathBuf::from("."),
                config_file: PathBuf::from("missing-config"),
                health_url_file: None,
                env: BTreeMap::new(),
            },
            32,
            Duration::from_millis(100),
            PathBuf::from("."),
            EventHub::default(),
        );
        let root = std::env::temp_dir().join(format!(
            "mcp-studio-api-inventory-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.join("bin"), root.join("runtime")).unwrap(),
        );
        let runtime_operations = catalog.runtime_operations();
        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.join("source"),
                HostPlatform::from_raw("Darwin", "arm64")
                    .unwrap()
                    .platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let events = EventHub::default();
        let updates = Arc::new(
            McpUpdateManager::new(
                catalog.clone(),
                supervisor.clone(),
                inventory.clone(),
                events.clone(),
            )
            .unwrap(),
        );
        let gateway_updates = Arc::new(
            GatewayUpdateManager::new(
                catalog.clone(),
                Arc::new(tunnel.clone()),
                inventory.clone(),
                events.clone(),
            )
            .unwrap(),
        );
        let fleet_updates = Arc::new(
            FleetUpdateManager::new(catalog.clone(), inventory.clone(), events.clone()).unwrap(),
        );
        let self_updates = Arc::new(
            SelfUpdateManager::new(catalog.clone(), inventory.clone(), events.clone()).unwrap(),
        );
        let tunnel_updates = Arc::new(
            TunnelUpdateManager::new(
                catalog.clone(),
                Arc::new(tunnel.clone()),
                inventory.clone(),
                events.clone(),
            )
            .unwrap(),
        );
        let reconciliation = Arc::new(
            RuntimeReconciler::new(
                catalog,
                Arc::new(tunnel.clone()),
                inventory.clone(),
                events.clone(),
            )
            .unwrap(),
        );
        let history = crate::storage::HistoryHandle::initialize(&root.join("runtime"));
        let operations = crate::operation::OperationService::new(history.clone());
        super::router(super::AppState {
            supervisor,
            discovery: Arc::new(discovery),
            tunnel: Arc::new(tunnel),
            inventory,
            updates,
            gateway_updates,
            fleet_updates,
            self_updates,
            tunnel_updates,
            reconciliation,
            runtime_operations,
            operations,
            history,
            shutdown: Arc::new(Notify::new()),
            registry_events: events,
        })
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn admitted_mutation_returns_effect_and_audit_headers() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mcp/missing/start")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
        let operation_id = response
            .headers()
            .get("x-studio-operation-id")
            .unwrap()
            .to_str()
            .unwrap();
        uuid::Uuid::parse_str(operation_id).unwrap();
        assert_eq!(
            response
                .headers()
                .get("x-studio-audit-status")
                .unwrap()
                .to_str()
                .unwrap(),
            "complete"
        );
        assert_eq!(
            response
                .headers()
                .get("x-studio-effect-status")
                .unwrap()
                .to_str()
                .unwrap(),
            "failed"
        );
    }

    #[tokio::test]
    async fn unknown_mcp_returns_not_found() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/mcp/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_cross_origin_registry_mutation() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/registry/missing/disable")
                    .header("host", "127.0.0.1:18100")
                    .header("origin", "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_cross_origin_lifecycle_request() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mcp/missing/start")
                    .header("host", "127.0.0.1:18100")
                    .header("origin", "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_cross_origin_tunnel_lifecycle_request() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tunnel/start")
                    .header("host", "127.0.0.1:18100")
                    .header("origin", "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn updates_api_is_browser_safe_and_unknown_component_is_not_found() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/updates")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let items = json.as_array().unwrap();
        assert_eq!(items.len(), 8);
        let serialized = serde_json::to_string(&json).unwrap();
        for forbidden in [
            "install_path",
            "repository_owner",
            "repository_name",
            "download_url",
            "checksum_manifest_url",
        ] {
            assert!(!serialized.contains(forbidden));
        }

        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/updates/unknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_cross_origin_update_check() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/updates/check")
                    .header("host", "127.0.0.1:18100")
                    .header("origin", "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn reconciliation_api_is_browser_safe_and_mutations_require_same_origin() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/reconciliation")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("runtime_root"));
        assert!(!text.contains("bin_root"));
        assert!(!text.contains("content_b64"));
        assert!(!text.contains("config.yaml"));

        for uri in [
            "/api/reconciliation/check",
            "/api/reconciliation/adopt",
            "/api/reconciliation/apply",
        ] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("host", "127.0.0.1:18100")
                        .header("origin", "https://example.com")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn reconciliation_check_side_effect_is_audited() {
        let app = test_router();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/reconciliation/check")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-studio-audit-status")
                .unwrap()
                .to_str()
                .unwrap(),
            "complete"
        );

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/history/v1/drift?limit=10")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let items = json["items"].as_array().unwrap();
            if let Some(item) = items.first() {
                assert_eq!(item["observation_kind"], "check");
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "reconciliation drift audit observation did not become visible"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn update_prepare_and_apply_require_same_origin_and_reject_authority_fields() {
        for (uri, body) in [
            ("/api/updates/git/prepare", r#"{"version":"1.1.0"}"#),
            (
                "/api/updates/git/apply",
                r#"{"transaction_id":"txn-git-example"}"#,
            ),
        ] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("host", "127.0.0.1:18100")
                        .header("origin", "https://example.com")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
        }

        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/updates/git/prepare")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"version":"1.1.0","install_path":"/tmp/evil"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn unknown_update_transaction_is_not_found() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/update-transactions/txn-missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn update_errors_are_sanitized_for_browser_responses() {
        assert_eq!(
            super::public_error_message(&StudioError::UpdateActivationFailed {
                component: "git".into(),
                detail: "/secret/internal/path".into(),
            }),
            "update_activation_failed"
        );
        assert_eq!(
            super::public_error_message(&StudioError::UntrustedReleaseAssetUrl {
                component: "git".into(),
                url: "https://attacker.invalid/payload".into(),
            }),
            "untrusted_release_asset_url"
        );
    }
}
