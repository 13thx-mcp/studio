use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;

use crate::{
    error::StudioError,
    storage::{HistoryListRequest, HistoryReadRequest, HistoryReadResponse, MetricsRequest},
};

use super::{AppState, ensure_same_origin};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/subjects", get(subjects))
        .route("/subjects/{subject_id}/sessions", get(sessions))
        .route("/updates", get(updates))
        .route("/updates/{history_id}", get(update_detail))
        .route("/operations/{operation_id}", get(operation))
        .route("/config-revisions", get(config_revisions))
        .route("/drift", get(drift))
        .route("/subjects/{subject_id}/lineage", get(lineage))
        .route("/metrics", get(metrics))
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Status).await?;
    json_response(expect_status(response)?)
}

async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<HistoryListRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Events(request)).await?;
    match response {
        HistoryReadResponse::Events(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn subjects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<HistoryListRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Subjects(request)).await?;
    match response {
        HistoryReadResponse::Subjects(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject_id): Path<String>,
    Query(request): Query<HistoryListRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(
        &state,
        HistoryReadRequest::Sessions {
            subject_id,
            request,
        },
    )
    .await?;
    match response {
        HistoryReadResponse::Sessions(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn updates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<HistoryListRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Updates(request)).await?;
    match response {
        HistoryReadResponse::Updates(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn update_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(history_id): Path<String>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::UpdateDetail { history_id }).await?;
    match response {
        HistoryReadResponse::UpdateDetail(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn operation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(operation_id): Path<String>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Operation { operation_id }).await?;
    match response {
        HistoryReadResponse::Operation(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn config_revisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<HistoryListRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::ConfigRevisions(request)).await?;
    match response {
        HistoryReadResponse::ConfigRevisions(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn drift(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<HistoryListRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Drift(request)).await?;
    match response {
        HistoryReadResponse::Drift(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn lineage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject_id): Path<String>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Lineage { subject_id }).await?;
    match response {
        HistoryReadResponse::Lineage(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<MetricsRequest>,
) -> Result<Response, HistoryApiError> {
    boundary(&headers)?;
    let response = read(&state, HistoryReadRequest::Metrics(request)).await?;
    match response {
        HistoryReadResponse::Metrics(value) => json_response(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

async fn read(
    state: &AppState,
    request: HistoryReadRequest,
) -> Result<HistoryReadResponse, HistoryApiError> {
    let history = state.history.clone();
    tokio::task::spawn_blocking(move || history.history_read(request))
        .await
        .map_err(|_| HistoryApiError::internal("history read worker failed"))?
        .map_err(HistoryApiError::from)
}

fn expect_status(
    response: HistoryReadResponse,
) -> Result<crate::storage::HistoryStatus, HistoryApiError> {
    match response {
        HistoryReadResponse::Status(value) => Ok(value),
        _ => Err(HistoryApiError::internal("history response mismatch")),
    }
}

fn boundary(headers: &HeaderMap) -> Result<(), HistoryApiError> {
    ensure_same_origin(headers)
        .map_err(|_| HistoryApiError::forbidden("history_origin_rejected"))?;

    if let Some(site) = headers.get("sec-fetch-site") {
        let site = site
            .to_str()
            .map_err(|_| HistoryApiError::forbidden("history_fetch_site_rejected"))?;
        if site != "same-origin" && site != "none" {
            return Err(HistoryApiError::forbidden("history_fetch_site_rejected"));
        }
    }

    if let Some(host) = headers.get(header::HOST) {
        let host = host
            .to_str()
            .map_err(|_| HistoryApiError::forbidden("history_host_rejected"))?;
        let hostname = host
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']').map(|(host, _)| host))
            .unwrap_or_else(|| host.split(':').next().unwrap_or(host));
        if !matches!(hostname, "127.0.0.1" | "::1" | "localhost") {
            return Err(HistoryApiError::forbidden("history_host_rejected"));
        }
    }
    Ok(())
}

fn json_response<T: Serialize>(value: T) -> Result<Response, HistoryApiError> {
    let mut response = Json(value).into_response();
    apply_history_headers(&mut response);
    Ok(response)
}

fn apply_history_headers(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
}

struct HistoryApiError {
    status: StatusCode,
    code: &'static str,
}

impl HistoryApiError {
    fn forbidden(code: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
        }
    }

    fn internal(code: &'static str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
        }
    }
}

impl From<StudioError> for HistoryApiError {
    fn from(error: StudioError) -> Self {
        match error {
            StudioError::NotFound(_) => Self {
                status: StatusCode::NOT_FOUND,
                code: "history_not_found",
            },
            StudioError::History(message) if message == "history_expired" => Self {
                status: StatusCode::GONE,
                code: "history_expired",
            },
            StudioError::History(message)
                if message.contains("queue is full") || message.contains("timed out") =>
            {
                Self {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "history_busy",
                }
            }
            StudioError::History(message)
                if message.contains("cursor")
                    || message.contains("filter")
                    || message.contains("range")
                    || message.contains("identifier")
                    || message.contains("page limit")
                    || message.contains("unsupported") =>
            {
                Self {
                    status: StatusCode::BAD_REQUEST,
                    code: "history_bad_request",
                }
            }
            StudioError::History(_) => Self::internal("history_unavailable"),
            _ => Self::internal("history_unavailable"),
        }
    }
}

impl IntoResponse for HistoryApiError {
    fn into_response(self) -> Response {
        let mut response =
            (self.status, Json(serde_json::json!({ "error": self.code }))).into_response();
        apply_history_headers(&mut response);
        response
    }
}
