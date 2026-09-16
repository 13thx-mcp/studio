use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;

use crate::{
    error::StudioError,
    supervisor::{LogEntry, ProcessStatus, Supervisor},
};

#[derive(Clone)]
pub struct AppState {
    pub supervisor: Arc<Supervisor>,
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
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/status", get(api_status))
        .route("/api/mcp", get(list_mcp))
        .route("/api/mcp/{id}", get(get_mcp))
        .route("/api/mcp/{id}/start", post(start_mcp))
        .route("/api/mcp/{id}/stop", post(stop_mcp))
        .route("/api/mcp/{id}/restart", post(restart_mcp))
        .route("/api/mcp/{id}/logs", get(get_logs))
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
    Json(ApiStatusResponse {
        status: "ok",
        managed_mcp_count: servers.len(),
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
) -> Result<Json<ProcessStatus>, ApiError> {
    Ok(Json(state.supervisor.start(&id).await?))
}

async fn stop_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProcessStatus>, ApiError> {
    Ok(Json(state.supervisor.stop(&id).await?))
}

async fn restart_mcp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProcessStatus>, ApiError> {
    Ok(Json(state.supervisor.restart(&id).await?))
}

async fn get_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<LogEntry>>, ApiError> {
    Ok(Json(state.supervisor.logs(&id).await?))
}

struct ApiError(StudioError);

impl From<StudioError> for ApiError {
    fn from(value: StudioError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            StudioError::NotFound(_) => StatusCode::NOT_FOUND,
            StudioError::AlreadyRunning(_) | StudioError::NotRunning(_) => StatusCode::CONFLICT,
            StudioError::Config(_) => StatusCode::BAD_REQUEST,
            StudioError::Process(_) | StudioError::Io(_) | StudioError::Toml(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
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

    use crate::{registry::Registry, supervisor::Supervisor};

    fn test_router() -> axum::Router {
        let supervisor = Supervisor::new(
            Registry::new(BTreeMap::new()),
            32,
            Duration::from_millis(100),
            PathBuf::from("."),
        );
        super::router(super::AppState {
            supervisor: Arc::new(supervisor),
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
}
