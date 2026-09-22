use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{StudioError, StudioResult};

use super::{HistoryEventInsert, insert_history_event, now_ms};

const GATEWAY_SUBJECT_ID: &str = "gateway-observation";
const MAX_BATCH_EVENTS: usize = 16;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayHistoryBatch {
    pub first_available_sequence: u64,
    pub last_sequence: u64,
    pub events: Vec<GatewayHistoryEvent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayHistoryEvent {
    pub sequence: u64,
    pub observed_at_ms: u128,
    #[serde(flatten)]
    pub observation: GatewayObservation,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayObservation {
    Request {
        request_id: String,
        child: String,
        tool: String,
        outcome: GatewayRequestOutcome,
        queue_ms: u64,
        execution_ms: u64,
        response_bytes: usize,
        guard: GatewayGuardAction,
        catalog_generation: u64,
        profile_generation: u64,
    },
    Catalog {
        catalog_generation: u64,
        profile_generation: u64,
        catalog_fingerprint: String,
        profile_fingerprint: String,
    },
    ChildRecovery {
        child: String,
        state: GatewayRecoveryState,
        generation: u64,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRequestOutcome {
    Returned,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayGuardAction {
    NotApplied,
    Passed,
    Guarded,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRecoveryState {
    Restarting,
    CircuitOpen,
    HalfOpen,
}

pub(super) fn record_gateway_events(
    connection: &mut Connection,
    instance_id: &str,
    events: &[GatewayHistoryEvent],
) -> StudioResult<u64> {
    if !valid_instance_id(instance_id) || events.len() > MAX_BATCH_EVENTS {
        return Err(StudioError::History("invalid Gateway history batch".into()));
    }
    let mut seen = BTreeSet::new();
    for event in events {
        if event.sequence == 0 || !seen.insert(event.sequence) || !valid_event(event) {
            return Err(StudioError::History("invalid Gateway history event".into()));
        }
    }
    let source_stream = format!("gateway/{instance_id}");
    let transaction = connection
        .unchecked_transaction()
        .map_err(super::history_error)?;
    let observed_at_ms = now_ms()?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO subjects(subject_id, kind, component_code, incarnation_id, first_observed_ms)
             VALUES (?1, 'component', 'gateway', ?1, ?2)",
            (GATEWAY_SUBJECT_ID, observed_at_ms),
        )
        .map_err(super::history_error)?;

    let mut high_watermark = 0;
    for event in events {
        let ordinal = i64::try_from(event.sequence)
            .map_err(|_| StudioError::History("Gateway event sequence overflow".into()))?;
        let exists: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM events WHERE source_stream=?1 AND source_ordinal=?2",
                (&source_stream, ordinal),
                |row| row.get(0),
            )
            .optional()
            .map_err(super::history_error)?;
        if exists.is_none() {
            let payload_json = serde_json::to_string(event)?;
            insert_history_event(
                &transaction,
                HistoryEventInsert {
                    run_id: None,
                    subject_id: GATEWAY_SUBJECT_ID,
                    operation_id: None,
                    source_stream: &source_stream,
                    source_ordinal: ordinal,
                    name: event_name(&event.observation),
                    category: event_category(&event.observation),
                    observed_at_ms,
                    evidence_kind: "recovery_observation",
                    payload_json: &payload_json,
                    retention_class: "observation",
                },
            )?;
        }
        high_watermark = high_watermark.max(event.sequence);
    }
    transaction.commit().map_err(super::history_error)?;
    Ok(high_watermark)
}

fn event_name(observation: &GatewayObservation) -> &'static str {
    match observation {
        GatewayObservation::Request { .. } => "gateway.request.observed",
        GatewayObservation::Catalog { .. } => "gateway.catalog.observed",
        GatewayObservation::ChildRecovery { .. } => "gateway.child.recovery",
    }
}

fn event_category(observation: &GatewayObservation) -> &'static str {
    match observation {
        GatewayObservation::Request { .. } => "outcome",
        GatewayObservation::Catalog { .. } => "observation",
        GatewayObservation::ChildRecovery { .. } => "recovery",
    }
}

fn valid_instance_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_event(event: &GatewayHistoryEvent) -> bool {
    let timestamp_ok = i64::try_from(event.observed_at_ms).is_ok();
    timestamp_ok
        && match &event.observation {
            GatewayObservation::Request {
                request_id,
                child,
                tool,
                ..
            } => {
                request_id.len() <= 64
                    && valid_identifier(child, 128)
                    && valid_identifier(tool, 256)
            }
            GatewayObservation::Catalog {
                catalog_fingerprint,
                profile_fingerprint,
                ..
            } => valid_sha256(catalog_fingerprint) && valid_sha256(profile_fingerprint),
            GatewayObservation::ChildRecovery { child, .. } => valid_identifier(child, 128),
        }
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::super::{MAX_PAGE_COUNT, open_and_migrate};
    use super::*;

    #[test]
    fn rejects_payload_like_fields_by_schema() {
        let value = r#"{"sequence":1,"observed_at_ms":1,"kind":"request","request_id":"r","child":"filesystem","tool":"read","outcome":"returned","queue_ms":0,"execution_ms":1,"response_bytes":1,"guard":"passed","catalog_generation":1,"profile_generation":1,"arguments":{"secret":true}}"#;
        assert!(serde_json::from_str::<GatewayHistoryEvent>(value).is_err());
    }

    #[test]
    fn persists_idempotently_without_payloads() {
        let fixture = tempfile::tempdir().unwrap();
        let runtime = fixture.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let mut writer = open_and_migrate(&runtime, MAX_PAGE_COUNT).unwrap();
        let event = GatewayHistoryEvent {
            sequence: 1,
            observed_at_ms: 1,
            observation: GatewayObservation::Request {
                request_id: "request-1".into(),
                child: "filesystem".into(),
                tool: "read_text_file".into(),
                outcome: GatewayRequestOutcome::Returned,
                queue_ms: 3,
                execution_ms: 7,
                response_bytes: 9,
                guard: GatewayGuardAction::Passed,
                catalog_generation: 1,
                profile_generation: 1,
            },
        };
        assert_eq!(
            record_gateway_events(
                &mut writer.connection,
                "123-456",
                std::slice::from_ref(&event)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            record_gateway_events(&mut writer.connection, "123-456", &[event]).unwrap(),
            1
        );
        let (count, payload): (i64, String) = writer.connection.query_row(
            "SELECT COUNT(*), MAX(payload_json) FROM events WHERE source_stream='gateway/123-456'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(count, 1);
        assert!(!payload.contains("arguments"));
        assert!(!payload.contains("result"));
    }

    #[test]
    fn new_gateway_instance_can_restart_sequence_without_replaying_old_instance() {
        let fixture = tempfile::tempdir().unwrap();
        let runtime = fixture.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let mut writer = open_and_migrate(&runtime, MAX_PAGE_COUNT).unwrap();
        let event = GatewayHistoryEvent {
            sequence: 1,
            observed_at_ms: 1,
            observation: GatewayObservation::Catalog {
                catalog_generation: 1,
                profile_generation: 1,
                catalog_fingerprint: "a".repeat(64),
                profile_fingerprint: "b".repeat(64),
            },
        };
        record_gateway_events(
            &mut writer.connection,
            "old-1",
            std::slice::from_ref(&event),
        )
        .unwrap();
        record_gateway_events(&mut writer.connection, "new-2", &[event]).unwrap();
        let count: i64 = writer
            .connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE name='gateway.catalog.observed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }
}
