use serde::{Deserialize, Serialize};

use crate::error::{StudioError, StudioResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    LocalOperator,
    SystemStartup,
    OwnerCompensation,
    ExternalFleet,
    SystemShutdown,
}

impl ActorKind {
    pub(crate) const fn as_db(self) -> &'static str {
        match self {
            Self::LocalOperator => "local_operator",
            Self::SystemStartup => "system_startup",
            Self::OwnerCompensation => "owner_compensation",
            Self::ExternalFleet => "external_fleet",
            Self::SystemShutdown => "system_shutdown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    Mcp,
    Tunnel,
    Component,
    Studio,
    Registry,
    Fleet,
    System,
}

impl SubjectKind {
    pub(crate) const fn as_db(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Tunnel => "tunnel",
            Self::Component => "component",
            Self::Studio => "studio",
            Self::Registry => "registry",
            Self::Fleet => "fleet",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    McpStart,
    McpStop,
    McpRestart,
    TunnelStart,
    TunnelStop,
    TunnelRestart,
    RegistryRegister,
    RegistryUpdate,
    RegistryEnable,
    RegistryDisable,
    RegistryUnregister,
    DiscoveryApprove,
    UpdateCheck,
    UpdatePrepare,
    UpdateApply,
    UpdateRollback,
    ReconciliationCheck,
    ReconciliationAdopt,
    ReconciliationApply,
    StudioSelfUpdateRequest,
    StudioSelfUpdateFinalize,
}

impl HistoryAction {
    pub const ALL: [Self; 21] = [
        Self::McpStart,
        Self::McpStop,
        Self::McpRestart,
        Self::TunnelStart,
        Self::TunnelStop,
        Self::TunnelRestart,
        Self::RegistryRegister,
        Self::RegistryUpdate,
        Self::RegistryEnable,
        Self::RegistryDisable,
        Self::RegistryUnregister,
        Self::DiscoveryApprove,
        Self::UpdateCheck,
        Self::UpdatePrepare,
        Self::UpdateApply,
        Self::UpdateRollback,
        Self::ReconciliationCheck,
        Self::ReconciliationAdopt,
        Self::ReconciliationApply,
        Self::StudioSelfUpdateRequest,
        Self::StudioSelfUpdateFinalize,
    ];

    pub(crate) const fn as_db(self) -> &'static str {
        match self {
            Self::McpStart => "mcp.start",
            Self::McpStop => "mcp.stop",
            Self::McpRestart => "mcp.restart",
            Self::TunnelStart => "tunnel.start",
            Self::TunnelStop => "tunnel.stop",
            Self::TunnelRestart => "tunnel.restart",
            Self::RegistryRegister => "registry.register",
            Self::RegistryUpdate => "registry.update",
            Self::RegistryEnable => "registry.enable",
            Self::RegistryDisable => "registry.disable",
            Self::RegistryUnregister => "registry.unregister",
            Self::DiscoveryApprove => "discovery.approve",
            Self::UpdateCheck => "update.check",
            Self::UpdatePrepare => "update.prepare",
            Self::UpdateApply => "update.apply",
            Self::UpdateRollback => "update.rollback",
            Self::ReconciliationCheck => "reconciliation.check",
            Self::ReconciliationAdopt => "reconciliation.adopt",
            Self::ReconciliationApply => "reconciliation.apply",
            Self::StudioSelfUpdateRequest => "studio.self_update.request",
            Self::StudioSelfUpdateFinalize => "studio.self_update.finalize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationOutcome {
    NotDispatched,
    Succeeded,
    Failed,
    RolledBack,
    RollbackFailed,
    Indeterminate,
}

impl OperationOutcome {
    pub(crate) const fn effect_status(self) -> &'static str {
        match self {
            Self::NotDispatched => "not_dispatched",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::RolledBack => "rolled_back",
            Self::RollbackFailed => "rollback_failed",
            Self::Indeterminate => "indeterminate",
        }
    }

    pub(crate) const fn audit_disposition(self) -> &'static str {
        match self {
            Self::NotDispatched | Self::Failed | Self::RollbackFailed => "failed",
            Self::Succeeded | Self::RolledBack => "succeeded",
            Self::Indeterminate => "indeterminate",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationEventPayload {
    pub action: HistoryAction,
    pub actor: ActorKind,
    pub effect_status: String,
    pub audit_status: String,
    pub error_code: Option<String>,
    pub safety_exception: bool,
}

impl OperationEventPayload {
    pub(crate) fn new(
        action: HistoryAction,
        actor: ActorKind,
        effect_status: impl Into<String>,
        audit_status: impl Into<String>,
        error_code: Option<String>,
        safety_exception: bool,
    ) -> StudioResult<Self> {
        if let Some(code) = error_code.as_deref() {
            validate_error_code(code)?;
        }
        Ok(Self {
            action,
            actor,
            effect_status: effect_status.into(),
            audit_status: audit_status.into(),
            error_code,
            safety_exception,
        })
    }
}

pub(crate) fn validate_error_code(code: &str) -> StudioResult<()> {
    if code.is_empty()
        || code.len() > 64
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(StudioError::History(
            "invalid history operation error code".into(),
        ));
    }
    Ok(())
}
