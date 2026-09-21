use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{StudioError, StudioResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOwnerKind {
    Mcp,
    Tunnel,
}

impl LifecycleOwnerKind {
    pub(crate) const fn as_db(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Tunnel => "tunnel",
        }
    }

    pub(crate) const fn subject_kind(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Tunnel => "tunnel",
        }
    }

    pub(crate) const fn start_event(self) -> &'static str {
        match self {
            Self::Mcp => "mcp.session.started",
            Self::Tunnel => "tunnel.session.started",
        }
    }

    pub(crate) const fn end_event(self) -> &'static str {
        match self {
            Self::Mcp => "mcp.session.ended",
            Self::Tunnel => "tunnel.session.ended",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEndKind {
    RequestedStop,
    CleanExit,
    UnexpectedExit,
    WaitError,
}

impl LifecycleEndKind {
    pub(crate) const fn as_db(self) -> &'static str {
        match self {
            Self::RequestedStop => "requested_stop",
            Self::CleanExit => "clean_exit",
            Self::UnexpectedExit => "unexpected_exit",
            Self::WaitError => "wait_error",
        }
    }

    pub(crate) const fn reason_code(self) -> &'static str {
        match self {
            Self::RequestedStop => "requested_stop",
            Self::CleanExit => "clean_exit",
            Self::UnexpectedExit => "unexpected_exit",
            Self::WaitError => "wait_error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LifecycleSessionContext {
    pub(crate) session_id: Uuid,
    pub(crate) subject_id: Uuid,
    pub(crate) owner_kind: LifecycleOwnerKind,
    pub(crate) generation: u64,
    pub(crate) pid: u32,
    pub(crate) source_stream: String,
}

impl LifecycleSessionContext {
    pub(crate) fn new(owner_kind: LifecycleOwnerKind, generation: u64, pid: u32) -> Self {
        let session_id = Uuid::new_v4();
        let subject_id = Uuid::new_v4();
        Self {
            session_id,
            subject_id,
            owner_kind,
            generation,
            pid,
            source_stream: format!("session/{session_id}"),
        }
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn observed_pid(&self) -> u32 {
        self.pid
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleTerminalFact {
    pub context: LifecycleSessionContext,
    pub end_kind: LifecycleEndKind,
    pub exit_code: Option<i32>,
    pub is_crash: bool,
    pub exact_duration: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifecycleEventPayload {
    pub owner_kind: LifecycleOwnerKind,
    pub generation: String,
    pub observed_pid: String,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub exact_duration_ms: Option<String>,
    pub is_crash: Option<bool>,
}

impl LifecycleEventPayload {
    pub(crate) fn started(context: &LifecycleSessionContext) -> Self {
        Self {
            owner_kind: context.owner_kind,
            generation: context.generation.to_string(),
            observed_pid: context.pid.to_string(),
            reason: "spawn_verified".into(),
            exit_code: None,
            exact_duration_ms: None,
            is_crash: None,
        }
    }

    pub(crate) fn ended(fact: &LifecycleTerminalFact) -> StudioResult<Self> {
        let exact_duration_ms = fact
            .exact_duration
            .map(|duration| {
                u64::try_from(duration.as_millis())
                    .map(|value| value.to_string())
                    .map_err(|_| StudioError::History("lifecycle duration overflow".into()))
            })
            .transpose()?;
        Ok(Self {
            owner_kind: fact.context.owner_kind,
            generation: fact.context.generation.to_string(),
            observed_pid: fact.context.pid.to_string(),
            reason: fact.end_kind.reason_code().to_owned(),
            exit_code: fact.exit_code,
            exact_duration_ms,
            is_crash: Some(fact.is_crash),
        })
    }
}
