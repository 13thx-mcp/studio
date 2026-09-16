use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    error::StudioError,
    realtime::StudioEvent,
    supervisor::{LogEntry, ProcessStatus, Supervisor},
    tunnel::{TunnelLogEntry, TunnelStatus, TunnelSupervisor},
};

#[derive(Clone)]
pub struct AppState {
    pub supervisor: Arc<Supervisor>,
    pub tunnel: Arc<TunnelSupervisor>,
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
        .route("/api/tunnel", get(get_tunnel))
        .route("/api/tunnel/start", post(start_tunnel))
        .route("/api/tunnel/stop", post(stop_tunnel))
        .route("/api/tunnel/restart", post(restart_tunnel))
        .route("/api/tunnel/logs", get(get_tunnel_logs))
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

async fn start_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ProcessStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(Json(state.supervisor.start(&id).await?))
}

async fn stop_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ProcessStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(Json(state.supervisor.stop(&id).await?))
}

async fn restart_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ProcessStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(Json(state.supervisor.restart(&id).await?))
}

async fn get_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<LogEntry>>, ApiError> {
    Ok(Json(state.supervisor.logs(&id).await?))
}

async fn get_tunnel(State(state): State<AppState>) -> Json<TunnelStatus> {
    Json(state.tunnel.status().await)
}

async fn start_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TunnelStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(Json(state.tunnel.start().await?))
}

async fn stop_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TunnelStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(Json(state.tunnel.stop().await?))
}

async fn restart_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TunnelStatus>, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(Json(state.tunnel.restart().await?))
}

async fn get_tunnel_logs(State(state): State<AppState>) -> Json<Vec<TunnelLogEntry>> {
    Json(state.tunnel.logs().await)
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    ensure_same_origin(&headers)?;
    Ok(ws.on_upgrade(move |socket| websocket_session(socket, state)).into_response())
}

async fn websocket_session(mut socket: WebSocket, state: AppState) {
    if send_snapshot(&mut socket, &state).await.is_err() {
        return;
    }

    let mut mcp_receiver = state.supervisor.subscribe_events();
    let mut tunnel_receiver = state.tunnel.subscribe_events();
    loop {
        tokio::select! {
            result = mcp_receiver.recv() => {
                if handle_event_result(&mut socket, &state, result).await.is_err() {
                    break;
                }
            }
            result = tunnel_receiver.recv() => {
                if handle_event_result(&mut socket, &state, result).await.is_err() {
                    break;
                }
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
    .await
}

async fn send_event(socket: &mut WebSocket, event: &StudioEvent) -> Result<(), ()> {
    let payload = serde_json::to_string(event).map_err(|_| ())?;
    socket.send(Message::Text(payload.into())).await.map_err(|_| ())
}

fn ensure_same_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(origin) = headers.get(axum::http::header::ORIGIN) else {
        return Ok(());
    };
    let Some(host) = headers.get(axum::http::header::HOST) else {
        return Err(ApiError::forbidden("missing Host header for browser request"));
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
            StudioError::NotFound(_) => StatusCode::NOT_FOUND,
            StudioError::AlreadyRunning(_) | StudioError::NotRunning(_) => StatusCode::CONFLICT,
            StudioError::Config(_) => StatusCode::BAD_REQUEST,
            StudioError::Process(_) | StudioError::Io(_) | StudioError::Toml(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        });
        (
            status,
            Json(ErrorResponse {
                error: self.error.to_string(),
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use crate::{
        realtime::EventHub,
        registry::Registry,
        supervisor::Supervisor,
        tunnel::{TunnelConfig, TunnelSupervisor},
    };

    fn test_router() -> axum::Router {
        let supervisor = Supervisor::new(
            Registry::new(BTreeMap::new()),
            32,
            Duration::from_millis(100),
            PathBuf::from("."),
        );
        let tunnel = TunnelSupervisor::new(
            TunnelConfig {
                name: "Test tunnel".into(),
                runtime: PathBuf::from("missing-runtime"),
                working_dir: PathBuf::from("."),
                config_file: PathBuf::from("missing-config"),
                env: BTreeMap::new(),
            },
            32,
            Duration::from_millis(100),
            PathBuf::from("."),
            EventHub::default(),
        );
        super::router(super::AppState {
            supervisor: Arc::new(supervisor),
            tunnel: Arc::new(tunnel),
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
}
