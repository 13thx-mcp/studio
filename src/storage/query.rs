use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    error::{StudioError, StudioResult},
    metrics::{
        DAY_MS, HOUR_MS, HistoricalMetricBucket, HistoricalMetricTotal, METRIC_DEFINITION_VERSION,
        MetricCode, MetricQuality,
    },
};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const MAX_CURSOR_BYTES: usize = 2048;
const MAX_RANGE_MS: i64 = 366 * 86_400_000;
const DEFAULT_RANGE_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HistorySnapshot {
    pub db_epoch: String,
    pub through_seq: String,
    pub retention_epoch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HistoryCoverage {
    pub state: String,
    pub reasons: Vec<String>,
    pub since_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub snapshot: HistorySnapshot,
    pub coverage: HistoryCoverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryStatus {
    pub state: &'static str,
    pub reason_code: Option<&'static str>,
    pub admission_available: bool,
    pub pending_obligations: String,
    pub db_epoch: Option<String>,
    pub latest_committed_seq: Option<String>,
    pub metrics_through_seq: Option<String>,
    pub coverage_state: &'static str,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalEvent {
    pub sequence: String,
    pub event_id: String,
    pub name: String,
    pub category: String,
    pub subject_id: String,
    pub operation_id: Option<String>,
    pub observed_at_ms: i64,
    pub source_at_ms: Option<i64>,
    pub evidence_kind: String,
    pub time_quality: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalSubject {
    pub subject_id: String,
    pub kind: String,
    pub component: Option<String>,
    pub retired_at_ms: Option<i64>,
    pub as_of_sequence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalSession {
    pub session_id: String,
    pub subject_id: String,
    pub owner_kind: String,
    pub generation: String,
    pub observed_pid: String,
    pub started_at_ms: i64,
    pub last_observed_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub end_kind: String,
    pub exit_code: Option<i32>,
    pub is_crash: Option<bool>,
    pub exact_duration_ms: Option<i64>,
    pub observed_duration_ms: String,
    pub sequence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalUpdate {
    pub history_id: String,
    pub transaction_id: String,
    pub component: Option<String>,
    pub source_version: Option<String>,
    pub target_version: String,
    pub last_phase: String,
    pub install_outcome: String,
    pub rollback_outcome: String,
    pub verification_scope: String,
    pub was_running: Option<bool>,
    pub completeness: String,
    pub first_observed_sequence: String,
    pub terminal_sequence: Option<String>,
    pub actionable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalOperation {
    pub operation_id: String,
    pub action: String,
    pub actor_kind: String,
    pub effect_status: String,
    pub audit_status: String,
    pub error_code: Option<String>,
    pub admitted_sequence: Option<String>,
    pub terminal_sequence: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalConfigRevision {
    pub revision_id: String,
    pub surface: String,
    pub provenance: String,
    pub change_categories: serde_json::Value,
    pub previous_revision_id: Option<String>,
    pub boundary_reason: Option<String>,
    pub observed_at_ms: i64,
    pub sequence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalDrift {
    pub observation_id: String,
    pub state: String,
    pub phase: String,
    pub studio_config_activation: String,
    pub client_freshness: String,
    pub rollback_outcome: String,
    pub observation_kind: String,
    pub surfaces: Vec<String>,
    pub sequence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalLineage {
    pub subject_id: String,
    pub latest_observation_id: Option<String>,
    pub previous_observation_id: Option<String>,
    pub last_verified_good_id: Option<String>,
    pub as_of_sequence: String,
    pub boundary_reason: Option<String>,
    pub current_match: &'static str,
    pub actionable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalMetrics {
    pub totals: Vec<HistoricalMetricTotal>,
    pub buckets: Vec<HistoricalMetricBucket>,
    pub through_seq: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryListRequest {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub subject: Option<String>,
    pub category: Option<String>,
    pub name: Option<String>,
    pub operation: Option<String>,
    pub component: Option<String>,
    pub state: Option<String>,
    pub surface: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsRequest {
    pub metric: Option<String>,
    pub resolution: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum HistoryReadRequest {
    Status,
    Events(HistoryListRequest),
    Subjects(HistoryListRequest),
    Sessions {
        subject_id: String,
        request: HistoryListRequest,
    },
    Updates(HistoryListRequest),
    UpdateDetail {
        history_id: String,
    },
    Operation {
        operation_id: String,
    },
    ConfigRevisions(HistoryListRequest),
    Drift(HistoryListRequest),
    Lineage {
        subject_id: String,
    },
    Metrics(MetricsRequest),
}

#[derive(Debug, Clone)]
pub enum HistoryReadResponse {
    Status(HistoryStatus),
    Events(HistoryPage<HistoricalEvent>),
    Subjects(HistoryPage<HistoricalSubject>),
    Sessions(HistoryPage<HistoricalSession>),
    Updates(HistoryPage<HistoricalUpdate>),
    UpdateDetail(HistoricalUpdate),
    Operation(HistoricalOperation),
    ConfigRevisions(HistoryPage<HistoricalConfigRevision>),
    Drift(HistoryPage<HistoricalDrift>),
    Lineage(HistoricalLineage),
    Metrics(HistoricalMetrics),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorV1 {
    version: u8,
    db_epoch: String,
    retention_epoch: u64,
    high_water: u64,
    last_seq: u64,
    from_ms: i64,
    to_ms: i64,
    filter_digest: String,
}

struct PageContext {
    snapshot: HistorySnapshot,
    high_water: i64,
    last_seq: i64,
    limit: usize,
    from_ms: i64,
    to_ms: i64,
    filter_digest: String,
}

type LineageHeadRow = (Option<String>, Option<String>, Option<String>, i64);

pub(crate) fn execute_query(
    connection: &Connection,
    request: HistoryReadRequest,
) -> StudioResult<HistoryReadResponse> {
    match request {
        HistoryReadRequest::Status => Ok(HistoryReadResponse::Status(query_status(connection)?)),
        HistoryReadRequest::Events(request) => Ok(HistoryReadResponse::Events(query_events(
            connection, request,
        )?)),
        HistoryReadRequest::Subjects(request) => Ok(HistoryReadResponse::Subjects(query_subjects(
            connection, request,
        )?)),
        HistoryReadRequest::Sessions {
            subject_id,
            request,
        } => Ok(HistoryReadResponse::Sessions(query_sessions(
            connection,
            &subject_id,
            request,
        )?)),
        HistoryReadRequest::Updates(request) => Ok(HistoryReadResponse::Updates(query_updates(
            connection, request,
        )?)),
        HistoryReadRequest::UpdateDetail { history_id } => Ok(HistoryReadResponse::UpdateDetail(
            query_update_detail(connection, &history_id)?,
        )),
        HistoryReadRequest::Operation { operation_id } => Ok(HistoryReadResponse::Operation(
            query_operation(connection, &operation_id)?,
        )),
        HistoryReadRequest::ConfigRevisions(request) => Ok(HistoryReadResponse::ConfigRevisions(
            query_config_revisions(connection, request)?,
        )),
        HistoryReadRequest::Drift(request) => Ok(HistoryReadResponse::Drift(query_drift(
            connection, request,
        )?)),
        HistoryReadRequest::Lineage { subject_id } => Ok(HistoryReadResponse::Lineage(
            query_lineage(connection, &subject_id)?,
        )),
        HistoryReadRequest::Metrics(request) => Ok(HistoryReadResponse::Metrics(query_metrics(
            connection, request,
        )?)),
    }
}

fn query_status(connection: &Connection) -> StudioResult<HistoryStatus> {
    let (db_epoch, _retention_epoch): (String, i64) = connection
        .query_row(
            "SELECT db_epoch, retention_epoch FROM history_meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(history_error)?;
    let latest: i64 = connection
        .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |row| {
            row.get(0)
        })
        .map_err(history_error)?;
    let metrics: Option<i64> = connection
        .query_row(
            "SELECT through_seq FROM projection_state WHERE projection_code='metrics.v1'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?;
    let gaps: i64 = connection
        .query_row("SELECT COUNT(*) FROM coverage_intervals", [], |row| {
            row.get(0)
        })
        .map_err(history_error)?;
    Ok(HistoryStatus {
        state: "healthy",
        reason_code: None,
        admission_available: true,
        pending_obligations: "0".into(),
        db_epoch: Some(db_epoch),
        latest_committed_seq: Some(latest.to_string()),
        metrics_through_seq: metrics.map(|value| value.to_string()),
        coverage_state: if gaps == 0 { "complete" } else { "partial" },
        observed_at_ms: now_ms()?,
    })
}

fn query_events(
    connection: &Connection,
    request: HistoryListRequest,
) -> StudioResult<HistoryPage<HistoricalEvent>> {
    validate_enum(
        request.category.as_deref(),
        &[
            "intent",
            "admission",
            "transition",
            "outcome",
            "recovery",
            "rollback",
            "observation",
        ],
    )?;
    let context = page_context(connection, &request, "events")?;
    let mut statement = connection
        .prepare(
            "SELECT seq,event_id,name,category,subject_id,operation_id,observed_at_ms,
                    source_at_ms,evidence_kind,time_quality,payload_json
             FROM events
             WHERE seq<=?1 AND seq<?2 AND observed_at_ms BETWEEN ?3 AND ?4
               AND (?5 IS NULL OR subject_id=?5)
               AND (?6 IS NULL OR category=?6)
               AND (?7 IS NULL OR name=?7)
               AND (?8 IS NULL OR operation_id=?8)
             ORDER BY seq DESC
             LIMIT ?9",
        )
        .map_err(history_error)?;
    let rows = statement
        .query_map(
            rusqlite::params![
                context.high_water,
                context.last_seq,
                context.from_ms,
                context.to_ms,
                request.subject.as_deref(),
                request.category.as_deref(),
                request.name.as_deref(),
                request.operation.as_deref(),
                i64::try_from(context.limit + 1).unwrap_or(201),
            ],
            |row| {
                let payload: String = row.get(10)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    HistoricalEvent {
                        sequence: row.get::<_, i64>(0)?.to_string(),
                        event_id: row.get(1)?,
                        name: row.get(2)?,
                        category: row.get(3)?,
                        subject_id: row.get(4)?,
                        operation_id: row.get(5)?,
                        observed_at_ms: row.get(6)?,
                        source_at_ms: row.get(7)?,
                        evidence_kind: row.get(8)?,
                        time_quality: row.get(9)?,
                        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
                    },
                ))
            },
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    page_from_rows(connection, context, rows)
}

fn query_subjects(
    connection: &Connection,
    request: HistoryListRequest,
) -> StudioResult<HistoryPage<HistoricalSubject>> {
    validate_enum(
        request.category.as_deref(),
        &[
            "mcp",
            "tunnel",
            "component",
            "studio",
            "registry",
            "fleet",
            "system",
        ],
    )?;
    let context = page_context(connection, &request, "subjects")?;
    let mut statement = connection
        .prepare(
            "SELECT s.subject_id,s.kind,s.component_code,s.retired_at_ms,
                    COALESCE((SELECT MAX(e.seq) FROM events e WHERE e.subject_id=s.subject_id),0) sort_seq
             FROM subjects s
             WHERE COALESCE((SELECT MAX(e.seq) FROM events e WHERE e.subject_id=s.subject_id),0)<=?1
               AND COALESCE((SELECT MAX(e.seq) FROM events e WHERE e.subject_id=s.subject_id),0)<?2
               AND (?3 IS NULL OR s.kind=?3)
             ORDER BY sort_seq DESC,s.subject_id
             LIMIT ?4",
        )
        .map_err(history_error)?;
    let rows = statement
        .query_map(
            rusqlite::params![
                context.high_water,
                context.last_seq,
                request.category.as_deref(),
                i64::try_from(context.limit + 1).unwrap_or(201)
            ],
            |row| {
                let seq: i64 = row.get(4)?;
                Ok((
                    seq,
                    HistoricalSubject {
                        subject_id: row.get(0)?,
                        kind: row.get(1)?,
                        component: row.get(2)?,
                        retired_at_ms: row.get(3)?,
                        as_of_sequence: seq.to_string(),
                    },
                ))
            },
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    page_from_rows(connection, context, rows)
}

fn query_sessions(
    connection: &Connection,
    subject_id: &str,
    mut request: HistoryListRequest,
) -> StudioResult<HistoryPage<HistoricalSession>> {
    validate_opaque_id(subject_id)?;
    request.subject = Some(subject_id.to_owned());
    let context = page_context(connection, &request, "sessions")?;
    let mut statement = connection
        .prepare(
            "SELECT session_id,subject_id,owner_kind,generation,pid,started_at_ms,last_observed_ms,
                    ended_at_ms,end_kind,exit_code,is_crash,exact_duration_ms,observed_duration_ms,started_seq
             FROM runtime_sessions
             WHERE subject_id=?1 AND started_seq<=?2 AND started_seq<?3
               AND started_at_ms BETWEEN ?4 AND ?5
             ORDER BY started_seq DESC LIMIT ?6",
        )
        .map_err(history_error)?;
    let rows = statement
        .query_map(
            (
                subject_id,
                context.high_water,
                context.last_seq,
                context.from_ms,
                context.to_ms,
                i64::try_from(context.limit + 1).unwrap_or(201),
            ),
            |row| {
                let seq: i64 = row.get(13)?;
                let crash: Option<i64> = row.get(10)?;
                Ok((
                    seq,
                    HistoricalSession {
                        session_id: row.get(0)?,
                        subject_id: row.get(1)?,
                        owner_kind: row.get(2)?,
                        generation: row.get::<_, i64>(3)?.to_string(),
                        observed_pid: row.get::<_, i64>(4)?.to_string(),
                        started_at_ms: row.get(5)?,
                        last_observed_ms: row.get(6)?,
                        ended_at_ms: row.get(7)?,
                        end_kind: row.get(8)?,
                        exit_code: row.get(9)?,
                        is_crash: crash.map(|value| value != 0),
                        exact_duration_ms: row.get(11)?,
                        observed_duration_ms: row.get::<_, i64>(12)?.to_string(),
                        sequence: seq.to_string(),
                    },
                ))
            },
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    page_from_rows(connection, context, rows)
}

fn query_updates(
    connection: &Connection,
    request: HistoryListRequest,
) -> StudioResult<HistoryPage<HistoricalUpdate>> {
    let context = page_context(connection, &request, "updates")?;
    let mut statement = connection
        .prepare(
            "SELECT u.attempt_id,u.transaction_id,s.component_code,u.source_version,u.target_version,
                    u.last_phase,u.install_outcome,u.rollback_outcome,u.verification_scope,u.was_running,
                    u.completeness,u.first_observed_seq,u.terminal_seq
             FROM update_attempts u
             JOIN subjects s ON s.subject_id=u.subject_id
             JOIN events e ON e.seq=u.first_observed_seq
             WHERE u.first_observed_seq<=?1 AND u.first_observed_seq<?2
               AND e.observed_at_ms BETWEEN ?3 AND ?4
               AND (?5 IS NULL OR s.component_code=?5)
               AND (?6 IS NULL OR u.install_outcome=?6)
             ORDER BY u.first_observed_seq DESC LIMIT ?7",
        )
        .map_err(history_error)?;
    let rows = statement
        .query_map(
            rusqlite::params![
                context.high_water,
                context.last_seq,
                context.from_ms,
                context.to_ms,
                request.component.as_deref(),
                request.state.as_deref(),
                i64::try_from(context.limit + 1).unwrap_or(201)
            ],
            update_row,
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    page_from_rows(connection, context, rows)
}

fn update_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, HistoricalUpdate)> {
    let first: i64 = row.get(11)?;
    Ok((
        first,
        HistoricalUpdate {
            history_id: row.get(0)?,
            transaction_id: row.get(1)?,
            component: row.get(2)?,
            source_version: row.get(3)?,
            target_version: row.get(4)?,
            last_phase: row.get(5)?,
            install_outcome: row.get(6)?,
            rollback_outcome: row.get(7)?,
            verification_scope: row.get(8)?,
            was_running: row.get::<_, Option<i64>>(9)?.map(|value| value != 0),
            completeness: row.get(10)?,
            first_observed_sequence: first.to_string(),
            terminal_sequence: row
                .get::<_, Option<i64>>(12)?
                .map(|value| value.to_string()),
            actionable: false,
        },
    ))
}

fn query_update_detail(
    connection: &Connection,
    history_id: &str,
) -> StudioResult<HistoricalUpdate> {
    validate_opaque_id(history_id)?;
    connection
        .query_row(
            "SELECT u.attempt_id,u.transaction_id,s.component_code,u.source_version,u.target_version,
                    u.last_phase,u.install_outcome,u.rollback_outcome,u.verification_scope,u.was_running,
                    u.completeness,u.first_observed_seq,u.terminal_seq
             FROM update_attempts u JOIN subjects s ON s.subject_id=u.subject_id
             WHERE u.attempt_id=?1",
            [history_id],
            |row| update_row(row).map(|(_, update)| update),
        )
        .optional()
        .map_err(history_error)?
        .ok_or_else(|| StudioError::NotFound(history_id.to_owned()))
}

fn query_operation(
    connection: &Connection,
    operation_id: &str,
) -> StudioResult<HistoricalOperation> {
    validate_opaque_id(operation_id)?;
    connection
        .query_row(
            "SELECT operation_id,action_code,actor_kind,effect_status,audit_status,error_code,
                    admitted_seq,terminal_seq
             FROM operations WHERE operation_id=?1",
            [operation_id],
            |row| {
                Ok(HistoricalOperation {
                    operation_id: row.get(0)?,
                    action: row.get(1)?,
                    actor_kind: row.get(2)?,
                    effect_status: row.get(3)?,
                    audit_status: row.get(4)?,
                    error_code: row.get(5)?,
                    admitted_sequence: row.get::<_, Option<i64>>(6)?.map(|v| v.to_string()),
                    terminal_sequence: row.get::<_, Option<i64>>(7)?.map(|v| v.to_string()),
                })
            },
        )
        .optional()
        .map_err(history_error)?
        .ok_or_else(|| StudioError::NotFound(operation_id.to_owned()))
}

fn query_config_revisions(
    connection: &Connection,
    request: HistoryListRequest,
) -> StudioResult<HistoryPage<HistoricalConfigRevision>> {
    let context = page_context(connection, &request, "config")?;
    let mut statement = connection
        .prepare(
            "SELECT revision_id,surface_code,provenance,change_categories_json,
                    previous_revision_id,boundary_reason,observed_at_ms,observed_seq
             FROM config_revisions
             WHERE observed_seq<=?1 AND observed_seq<?2
               AND observed_at_ms BETWEEN ?3 AND ?4
               AND (?5 IS NULL OR surface_code=?5)
             ORDER BY observed_seq DESC LIMIT ?6",
        )
        .map_err(history_error)?;
    let rows = statement
        .query_map(
            (
                context.high_water,
                context.last_seq,
                context.from_ms,
                context.to_ms,
                request.surface.as_deref(),
                i64::try_from(context.limit + 1).unwrap_or(201),
            ),
            |row| {
                let seq: i64 = row.get(7)?;
                let changes: String = row.get(3)?;
                Ok((
                    seq,
                    HistoricalConfigRevision {
                        revision_id: row.get(0)?,
                        surface: row.get(1)?,
                        provenance: row.get(2)?,
                        change_categories: serde_json::from_str(&changes)
                            .unwrap_or(serde_json::Value::Array(vec![])),
                        previous_revision_id: row.get(4)?,
                        boundary_reason: row.get(5)?,
                        observed_at_ms: row.get(6)?,
                        sequence: seq.to_string(),
                    },
                ))
            },
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    page_from_rows(connection, context, rows)
}

fn query_drift(
    connection: &Connection,
    request: HistoryListRequest,
) -> StudioResult<HistoryPage<HistoricalDrift>> {
    let context = page_context(connection, &request, "drift")?;
    let mut statement = connection
        .prepare(
            "SELECT observation_id,state,phase,studio_config_activation,client_freshness,
                    rollback_outcome,observation_kind,event_seq
             FROM drift_observations d
             JOIN events e ON e.seq=d.event_seq
             WHERE d.event_seq<=?1 AND d.event_seq<?2
               AND e.observed_at_ms BETWEEN ?3 AND ?4
               AND (?5 IS NULL OR d.state=?5)
             ORDER BY d.event_seq DESC LIMIT ?6",
        )
        .map_err(history_error)?;
    let base = statement
        .query_map(
            (
                context.high_water,
                context.last_seq,
                context.from_ms,
                context.to_ms,
                request.state.as_deref(),
                i64::try_from(context.limit + 1).unwrap_or(201),
            ),
            |row| {
                Ok((
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    let mut rows = Vec::with_capacity(base.len());
    for (seq, id, state, phase, activation, freshness, rollback, kind) in base {
        let mut surface_stmt = connection
            .prepare("SELECT surface_code FROM drift_surfaces WHERE observation_id=?1 ORDER BY surface_code")
            .map_err(history_error)?;
        let surfaces = surface_stmt
            .query_map([&id], |row| row.get(0))
            .map_err(history_error)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(history_error)?;
        rows.push((
            seq,
            HistoricalDrift {
                observation_id: id,
                state,
                phase,
                studio_config_activation: activation,
                client_freshness: freshness,
                rollback_outcome: rollback,
                observation_kind: kind,
                surfaces,
                sequence: seq.to_string(),
            },
        ));
    }
    page_from_rows(connection, context, rows)
}

fn query_lineage(connection: &Connection, subject_id: &str) -> StudioResult<HistoricalLineage> {
    validate_opaque_id(subject_id)?;
    let row: Option<LineageHeadRow> = connection
        .query_row(
            "SELECT latest_observation_id,previous_observation_id,last_verified_good_id,as_of_seq
             FROM observed_lineage_heads WHERE subject_id=?1",
            [subject_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(history_error)?;
    let Some((latest, previous, good, seq)) = row else {
        return Ok(HistoricalLineage {
            subject_id: subject_id.to_owned(),
            latest_observation_id: None,
            previous_observation_id: None,
            last_verified_good_id: None,
            as_of_sequence: "0".into(),
            boundary_reason: Some("unknown_predecessor".into()),
            current_match: "unknown",
            actionable: false,
        });
    };
    let boundary = if let Some(latest_id) = latest.as_deref() {
        connection
            .query_row(
                "SELECT boundary_reason FROM install_observations WHERE observation_id=?1",
                [latest_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(history_error)?
            .flatten()
    } else {
        Some("unknown_predecessor".into())
    };
    Ok(HistoricalLineage {
        subject_id: subject_id.to_owned(),
        latest_observation_id: latest,
        previous_observation_id: previous,
        last_verified_good_id: good,
        as_of_sequence: seq.to_string(),
        boundary_reason: boundary,
        current_match: "unknown",
        actionable: false,
    })
}

fn query_metrics(
    connection: &Connection,
    request: MetricsRequest,
) -> StudioResult<HistoricalMetrics> {
    let metric = request.metric.as_deref().map(parse_metric).transpose()?;
    let now = now_ms()?;
    let from_ms = request
        .from_ms
        .unwrap_or(now.saturating_sub(DEFAULT_RANGE_MS));
    let to_ms = request.to_ms.unwrap_or(now);
    validate_range(from_ms, to_ms)?;
    let resolution = match request.resolution.as_deref().unwrap_or("total") {
        "total" => None,
        "hour" => Some(HOUR_MS),
        "day" => Some(DAY_MS),
        _ => return Err(StudioError::History("unsupported metric resolution".into())),
    };
    let through: i64 = connection
        .query_row(
            "SELECT COALESCE(through_seq,0) FROM projection_state WHERE projection_code='metrics.v1'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(history_error)?
        .unwrap_or(0);

    let mut totals = Vec::new();
    if resolution.is_none() {
        let mut stmt = connection
            .prepare(
                "SELECT metric_code,count_value,sum_value,through_seq,coverage_start_ms,quality
                 FROM metrics_totals
                 WHERE subject_id='metrics-system'
                   AND definition_version=?1
                   AND (?2 IS NULL OR metric_code=?2)
                 ORDER BY metric_code",
            )
            .map_err(history_error)?;
        totals = stmt
            .query_map(
                (METRIC_DEFINITION_VERSION, metric.map(MetricCode::as_db)),
                |row| {
                    let code: String = row.get(0)?;
                    Ok(HistoricalMetricTotal {
                        subject_id: "metrics-system".into(),
                        metric: parse_metric_db(&code).unwrap_or(MetricCode::Launches),
                        count: row.get::<_, i64>(1)?.to_string(),
                        sum: row.get::<_, i64>(2)?.to_string(),
                        through_seq: row.get::<_, i64>(3)?.to_string(),
                        coverage_start_ms: row.get(4)?,
                        quality: MetricQuality::from_db(&row.get::<_, String>(5)?)
                            .unwrap_or(MetricQuality::Unknown),
                    })
                },
            )
            .map_err(history_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(history_error)?;
    }

    let mut buckets = Vec::new();
    if let Some(resolution) = resolution {
        let mut stmt = connection
            .prepare(
                "SELECT metric_code,resolution_ms,bucket_start_ms,count_value,sum_value,
                        min_value,max_value,through_seq,quality
                 FROM metrics_buckets
                 WHERE subject_id='metrics-system' AND definition_version=?1
                   AND resolution_ms=?2 AND bucket_start_ms BETWEEN ?3 AND ?4
                   AND (?5 IS NULL OR metric_code=?5)
                 ORDER BY bucket_start_ms,metric_code
                 LIMIT 1000",
            )
            .map_err(history_error)?;
        buckets = stmt
            .query_map(
                (
                    METRIC_DEFINITION_VERSION,
                    resolution,
                    from_ms,
                    to_ms,
                    metric.map(MetricCode::as_db),
                ),
                |row| {
                    let code: String = row.get(0)?;
                    Ok(HistoricalMetricBucket {
                        subject_id: "metrics-system".into(),
                        metric: parse_metric_db(&code).unwrap_or(MetricCode::Launches),
                        resolution_ms: row.get(1)?,
                        bucket_start_ms: row.get(2)?,
                        count: row.get::<_, i64>(3)?.to_string(),
                        sum: row.get::<_, i64>(4)?.to_string(),
                        min: row.get::<_, Option<i64>>(5)?.map(|v| v.to_string()),
                        max: row.get::<_, Option<i64>>(6)?.map(|v| v.to_string()),
                        through_seq: row.get::<_, i64>(7)?.to_string(),
                        quality: MetricQuality::from_db(&row.get::<_, String>(8)?)
                            .unwrap_or(MetricQuality::Unknown),
                    })
                },
            )
            .map_err(history_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(history_error)?;
    }

    Ok(HistoricalMetrics {
        totals,
        buckets,
        through_seq: through.to_string(),
    })
}

fn parse_metric(value: &str) -> StudioResult<MetricCode> {
    parse_metric_db(value).ok_or_else(|| StudioError::History("unsupported metric code".into()))
}

fn parse_metric_db(value: &str) -> Option<MetricCode> {
    MetricCode::ALL
        .into_iter()
        .find(|metric| metric.as_db() == value)
}

fn page_context(
    connection: &Connection,
    request: &HistoryListRequest,
    kind: &str,
) -> StudioResult<PageContext> {
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(StudioError::History(
            "history page limit is outside 1..=200".into(),
        ));
    }

    let decoded_cursor = request.cursor.as_deref().map(decode_cursor).transpose()?;
    let (from_ms, to_ms) = if let Some(cursor) = decoded_cursor.as_ref() {
        if request.from_ms.is_some_and(|value| value != cursor.from_ms)
            || request.to_ms.is_some_and(|value| value != cursor.to_ms)
        {
            return Err(StudioError::History(
                "history cursor filter mismatch".into(),
            ));
        }
        (cursor.from_ms, cursor.to_ms)
    } else {
        let now = now_ms()?;
        (
            request
                .from_ms
                .unwrap_or(now.saturating_sub(DEFAULT_RANGE_MS)),
            request.to_ms.unwrap_or(now),
        )
    };
    validate_range(from_ms, to_ms)?;

    let filter_json = serde_json::to_string(&(
        kind,
        request.subject.as_deref(),
        request.category.as_deref(),
        request.name.as_deref(),
        request.operation.as_deref(),
        request.component.as_deref(),
        request.state.as_deref(),
        request.surface.as_deref(),
        from_ms,
        to_ms,
    ))?;
    let filter_digest = format!("{:x}", Sha256::digest(filter_json.as_bytes()));
    let (db_epoch, retention_epoch): (String, i64) = connection
        .query_row(
            "SELECT db_epoch,retention_epoch FROM history_meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(history_error)?;
    let current_high: i64 = connection
        .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |row| {
            row.get(0)
        })
        .map_err(history_error)?;

    let (high_water, last_seq) = if let Some(cursor) = decoded_cursor {
        if cursor.db_epoch != db_epoch
            || cursor.retention_epoch != u64::try_from(retention_epoch).unwrap_or(u64::MAX)
        {
            return Err(StudioError::History("history_expired".into()));
        }
        if cursor.filter_digest != filter_digest {
            return Err(StudioError::History(
                "history cursor filter mismatch".into(),
            ));
        }
        (
            i64::try_from(cursor.high_water)
                .map_err(|_| StudioError::History("history cursor overflow".into()))?,
            i64::try_from(cursor.last_seq)
                .map_err(|_| StudioError::History("history cursor overflow".into()))?,
        )
    } else {
        (current_high, current_high.saturating_add(1))
    };

    Ok(PageContext {
        snapshot: HistorySnapshot {
            db_epoch,
            through_seq: high_water.to_string(),
            retention_epoch: retention_epoch.to_string(),
        },
        high_water,
        last_seq,
        limit,
        from_ms,
        to_ms,
        filter_digest,
    })
}

fn page_from_rows<T>(
    connection: &Connection,
    context: PageContext,
    mut rows: Vec<(i64, T)>,
) -> StudioResult<HistoryPage<T>> {
    let has_more = rows.len() > context.limit;
    if has_more {
        rows.pop();
    }
    let next_cursor = if has_more {
        rows.last()
            .map(|(seq, _)| encode_cursor(&context, *seq))
            .transpose()?
    } else {
        None
    };
    let items = rows.into_iter().map(|(_, item)| item).collect();
    Ok(HistoryPage {
        items,
        next_cursor,
        snapshot: context.snapshot,
        coverage: query_coverage(connection, context.from_ms, context.to_ms)?,
    })
}

fn query_coverage(
    connection: &Connection,
    from_ms: i64,
    to_ms: i64,
) -> StudioResult<HistoryCoverage> {
    let mut stmt = connection
        .prepare(
            "SELECT reason,from_ms FROM coverage_intervals
             WHERE (from_ms IS NULL OR from_ms<=?2) AND (to_ms IS NULL OR to_ms>=?1)
             ORDER BY COALESCE(from_ms,0) LIMIT 8",
        )
        .map_err(history_error)?;
    let rows = stmt
        .query_map((from_ms, to_ms), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
        })
        .map_err(history_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(history_error)?;
    let since = rows.iter().filter_map(|(_, value)| *value).min();
    Ok(HistoryCoverage {
        state: if rows.is_empty() {
            "complete"
        } else {
            "partial"
        }
        .into(),
        reasons: rows.into_iter().map(|(reason, _)| reason).collect(),
        since_ms: since,
    })
}

fn encode_cursor(context: &PageContext, last_seq: i64) -> StudioResult<String> {
    let retention_epoch = context
        .snapshot
        .retention_epoch
        .parse::<u64>()
        .map_err(|_| StudioError::History("invalid retention epoch".into()))?;
    let cursor = CursorV1 {
        version: 1,
        db_epoch: context.snapshot.db_epoch.clone(),
        retention_epoch,
        high_water: u64::try_from(context.high_water)
            .map_err(|_| StudioError::History("history cursor overflow".into()))?,
        last_seq: u64::try_from(last_seq)
            .map_err(|_| StudioError::History("history cursor overflow".into()))?,
        from_ms: context.from_ms,
        to_ms: context.to_ms,
        filter_digest: context.filter_digest.clone(),
    };
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor)?))
}

fn decode_cursor(value: &str) -> StudioResult<CursorV1> {
    if value.len() > MAX_CURSOR_BYTES {
        return Err(StudioError::History("history cursor is too large".into()));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| StudioError::History("malformed history cursor".into()))?;
    let cursor: CursorV1 = serde_json::from_slice(&bytes)
        .map_err(|_| StudioError::History("malformed history cursor".into()))?;
    if cursor.version != 1 || cursor.filter_digest.len() != 64 {
        return Err(StudioError::History("unsupported history cursor".into()));
    }
    Ok(cursor)
}

fn validate_range(from_ms: i64, to_ms: i64) -> StudioResult<()> {
    if from_ms < 0 || to_ms < 0 || from_ms > to_ms || to_ms.saturating_sub(from_ms) > MAX_RANGE_MS {
        return Err(StudioError::History(
            "history time range is invalid or too large".into(),
        ));
    }
    Ok(())
}

fn validate_opaque_id(value: &str) -> StudioResult<()> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(StudioError::History("invalid history identifier".into()));
    }
    Ok(())
}

fn validate_enum(value: Option<&str>, allowed: &[&str]) -> StudioResult<()> {
    if let Some(value) = value
        && !allowed.contains(&value)
    {
        return Err(StudioError::History("unsupported history filter".into()));
    }
    Ok(())
}

fn now_ms() -> StudioResult<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| StudioError::History("system clock is before UNIX epoch".into()))?
            .as_millis(),
    )
    .map_err(|_| StudioError::History("system clock overflow".into()))
}

fn history_error(error: rusqlite::Error) -> StudioError {
    StudioError::History(error.to_string())
}
