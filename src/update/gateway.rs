use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use async_trait::async_trait;
use rmcp::{
    ServiceError, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientRequest, Request, ServerResult},
    service::{Peer, PeerRequestOptions, RoleClient},
    transport::TokioChildProcess,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::Command,
    sync::Mutex,
    task::JoinSet,
    time::{sleep, timeout},
};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
    storage::GatewayHistoryBatch,
    tunnel::{TunnelState, TunnelSupervisor},
};

use super::{
    ArtifactHistoryIdentity, ArtifactStager, ComponentCatalog, ComponentId, InventoryService,
    McpUpdatePhase, McpUpdateTransactionView, ReleaseProvider, StagedArtifact,
    ThirteenthXReleaseProvider, Version,
    transaction::{
        PreparedStagedIdentity, activate_binary, now_ms, prepare_rollback,
        remove_rollback_material, restore_rollback, sanitize_transaction_error,
    },
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const RECONNECT_HEALTH_WINDOW: Duration = Duration::from_millis(800);
const MAX_CONFIG_FILES: usize = 256;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_EXPECTED_TOOLS: usize = 4096;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONTROL_RESPONSE_BYTES: usize = 16 * 1024;
const GATEWAY_LIST_SERVERS: &str = "gateway_list_servers";
const GATEWAY_RELOAD: &str = "gateway_reload";
const GATEWAY_SET_ENABLED: &str = "gateway_set_server_enabled";

#[derive(Debug, Clone)]
struct GatewayTransactionRecord {
    view: McpUpdateTransactionView,
    staged_id: Option<String>,
    staged_identity: Option<PreparedStagedIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayOwnerState {
    Stopped,
    Running,
    Transitional,
    Failed,
}

#[async_trait]
trait GatewayOwner: Send + Sync {
    async fn state(&self) -> StudioResult<GatewayOwnerState>;
    async fn stop(&self) -> StudioResult<()>;
    async fn start(&self) -> StudioResult<()>;
    fn validate_binding(&self, gateway_binary: &Path, servers_dir: &Path) -> StudioResult<()>;
}

struct TunnelGatewayOwner {
    tunnel: Arc<TunnelSupervisor>,
}

#[derive(Clone)]
pub struct GatewayControlClient {
    socket: PathBuf,
}

#[derive(Deserialize)]
struct GatewayControlResponse {
    ok: bool,
    #[serde(default)]
    instance_id: Option<String>,
    state: String,
    drain_generation: u64,
    #[serde(default)]
    history: Option<GatewayHistoryBatch>,
}

impl GatewayControlClient {
    pub fn from_catalog(catalog: &ComponentCatalog) -> Self {
        Self {
            socket: catalog.runtime_root().join("gateway/control/gateway.sock"),
        }
    }

    async fn exchange(&self, request: Value) -> StudioResult<GatewayControlResponse> {
        let stream = timeout(CONTROL_TIMEOUT, UnixStream::connect(&self.socket))
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket connection timed out".into(),
            })?
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket is unavailable".into(),
            })?;
        let (reader, mut writer) = stream.into_split();
        let mut encoded = serde_json::to_vec(&request).map_err(|error| {
            StudioError::UpdateTransaction(format!(
                "cannot encode Gateway control request: {error}"
            ))
        })?;
        encoded.push(b'\n');
        timeout(CONTROL_TIMEOUT, writer.write_all(&encoded))
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket write timed out".into(),
            })?
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket write failed".into(),
            })?;
        let mut line = Vec::new();
        let mut reader = BufReader::new(reader);
        let count = timeout(CONTROL_TIMEOUT, reader.read_until(b'\n', &mut line))
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket response timed out".into(),
            })?
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket response failed".into(),
            })?;
        if count == 0 || count > MAX_CONTROL_RESPONSE_BYTES || line.last() != Some(&b'\n') {
            return Err(StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket response is invalid".into(),
            });
        }
        serde_json::from_slice(&line[..line.len() - 1]).map_err(|_| {
            StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway control socket response schema is invalid".into(),
            }
        })
    }

    async fn drain_for_update(&self) -> StudioResult<u64> {
        self.drain("gateway_update").await
    }

    pub async fn history_after(
        &self,
        after_sequence: u64,
    ) -> StudioResult<(String, GatewayHistoryBatch)> {
        let response = self
            .exchange(json!({
                "version": 1,
                "action": "history",
                "after_sequence": after_sequence,
            }))
            .await?;
        let history = response.history.ok_or_else(|| {
            StudioError::History("Gateway did not provide a telemetry history batch".into())
        })?;
        if !response.ok || history.events.len() > 16 || history.last_sequence < after_sequence {
            return Err(StudioError::History(
                "Gateway telemetry response is invalid".into(),
            ));
        }
        let instance_id = response
            .instance_id
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or_else(|| {
                StudioError::History("Gateway telemetry instance identity is invalid".into())
            })?;
        Ok((instance_id, history))
    }

    pub(crate) async fn drain_for_reconciliation(&self) -> StudioResult<u64> {
        self.drain("reconciliation").await
    }

    async fn drain(&self, reason: &str) -> StudioResult<u64> {
        let response = self
            .exchange(json!({
                "version": 1,
                "action": "drain",
                "reason": reason
            }))
            .await?;
        if !response.ok || response.state != "DRAINED" || response.drain_generation == 0 {
            return Err(StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway did not confirm a drained control generation".into(),
            });
        }
        Ok(response.drain_generation)
    }

    pub(crate) async fn resume(&self, generation: u64) -> StudioResult<()> {
        let response = self
            .exchange(json!({
                "version": 1,
                "action": "resume",
                "drain_generation": generation
            }))
            .await?;
        if !response.ok || response.state != "RUNNING" || response.drain_generation != generation {
            return Err(StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway did not resume requested drain generation".into(),
            });
        }
        Ok(())
    }

    pub(crate) async fn reload(&self) -> StudioResult<()> {
        let response = self
            .exchange(json!({"version": 1, "action": "reload"}))
            .await?;
        if !response.ok || response.state != "RUNNING" {
            return Err(StudioError::UpdateVerificationFailed {
                component: "gateway".into(),
                detail: "Gateway did not confirm control reload completion".into(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl GatewayOwner for TunnelGatewayOwner {
    async fn state(&self) -> StudioResult<GatewayOwnerState> {
        Ok(match self.tunnel.status().await.state {
            TunnelState::Stopped => GatewayOwnerState::Stopped,
            TunnelState::Running => GatewayOwnerState::Running,
            TunnelState::Starting | TunnelState::Stopping => GatewayOwnerState::Transitional,
            TunnelState::Failed => GatewayOwnerState::Failed,
        })
    }

    async fn stop(&self) -> StudioResult<()> {
        self.tunnel.stop().await?;
        Ok(())
    }

    async fn start(&self) -> StudioResult<()> {
        self.tunnel.start().await?;
        Ok(())
    }

    fn validate_binding(&self, gateway_binary: &Path, servers_dir: &Path) -> StudioResult<()> {
        self.tunnel
            .validate_gateway_binding(gateway_binary, servers_dir)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayCatalogSnapshot {
    files: BTreeMap<String, String>,
    configured: BTreeMap<String, bool>,
    commands: BTreeMap<String, String>,
    children: BTreeMap<String, GatewayChildConfigProjection>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GatewayChildConfigProjection {
    name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    tool_prefix: Option<String>,
    #[serde(default)]
    tool_allowlist: Vec<String>,
    #[serde(default = "default_child_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    restart: GatewayRestartConfigProjection,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GatewayRestartConfigProjection {
    #[serde(default)]
    policy: GatewayRestartPolicy,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum GatewayRestartPolicy {
    Never,
    #[default]
    OnFailure,
}

fn default_child_timeout_ms() -> u64 {
    30_000
}

fn default_true() -> bool {
    true
}

impl GatewayCatalogSnapshot {
    fn capture(config_dir: &Path) -> StudioResult<Self> {
        let metadata = fs::symlink_metadata(config_dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StudioError::UpdateTransaction(
                "Gateway servers.d must be a regular directory".into(),
            ));
        }
        let canonical = fs::canonicalize(config_dir)?;
        let mut entries = fs::read_dir(&canonical)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        if entries.len() > MAX_CONFIG_FILES {
            return Err(StudioError::UpdateTransaction(
                "Gateway servers.d contains too many entries".into(),
            ));
        }

        let mut files = BTreeMap::new();
        let mut configured = BTreeMap::new();
        let mut commands = BTreeMap::new();
        let mut children = BTreeMap::new();
        for entry in entries {
            let path = entry.path();
            let entry_metadata = fs::symlink_metadata(&path)?;
            if entry_metadata.file_type().is_symlink() {
                return Err(StudioError::UpdateTransaction(
                    "Gateway servers.d must not contain symlinks".into(),
                ));
            }
            if !entry_metadata.is_file() {
                return Err(StudioError::UpdateTransaction(
                    "Gateway servers.d must contain files only".into(),
                ));
            }
            if entry_metadata.len() > MAX_CONFIG_BYTES {
                return Err(StudioError::UpdateTransaction(
                    "Gateway servers.d file exceeds size limit".into(),
                ));
            }
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| {
                    StudioError::UpdateTransaction("invalid Gateway config filename".into())
                })?
                .to_owned();
            let bytes = fs::read(&path)?;
            files.insert(name.clone(), sha256_bytes(&bytes));

            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !matches!(extension, "yaml" | "yml") {
                continue;
            }
            let config: GatewayChildConfigProjection =
                serde_yaml::from_slice(&bytes).map_err(|error| {
                    StudioError::UpdateTransaction(format!(
                        "invalid Gateway child config metadata: {error}"
                    ))
                })?;
            if configured
                .insert(config.name.clone(), config.enabled)
                .is_some()
                || commands
                    .insert(config.name.clone(), config.command.clone())
                    .is_some()
                || children.insert(config.name.clone(), config).is_some()
            {
                return Err(StudioError::UpdateTransaction(
                    "Gateway servers.d contains duplicate child names".into(),
                ));
            }
        }
        if configured.is_empty() {
            return Err(StudioError::UpdateTransaction(
                "Gateway servers.d has no configured child MCPs".into(),
            ));
        }
        Ok(Self {
            files,
            configured,
            commands,
            children,
        })
    }

    fn verify_unchanged(&self, config_dir: &Path) -> StudioResult<()> {
        let current = Self::capture(config_dir)?;
        if current.files != self.files
            || current.configured != self.configured
            || current.commands != self.commands
            || current.children != self.children
        {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway servers.d changed during update transaction".into(),
            });
        }
        Ok(())
    }

    fn validate_managed_paths(&self, catalog: &ComponentCatalog) -> StudioResult<()> {
        for (name, component) in [
            ("filesystem", ComponentId::Filesystem),
            ("git", ComponentId::Git),
            ("exec", ComponentId::Exec),
            ("blender", ComponentId::Blender),
        ] {
            let Some(command) = self.commands.get(name) else {
                continue;
            };
            let actual = fs::canonicalize(command).map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "Gateway child {name} command does not resolve: {error}"
                ))
            })?;
            let expected = fs::canonicalize(catalog.install_path(component)?).map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "catalog child {name} binary does not resolve: {error}"
                ))
            })?;
            if actual != expected {
                return Err(StudioError::UpdateTransaction(format!(
                    "Gateway child {name} command does not match flat bin_root target"
                )));
            }
        }
        self.validate_trusted_commands(catalog.bin_root())
    }

    fn validate_trusted_commands(&self, trusted_bin_root: &Path) -> StudioResult<()> {
        let trusted_bin_root = fs::canonicalize(trusted_bin_root).map_err(|error| {
            StudioError::UpdateTransaction(format!(
                "trusted Gateway bin root does not resolve: {error}"
            ))
        })?;
        for config in self.children.values() {
            let command_path = Path::new(&config.command);
            let metadata = fs::symlink_metadata(command_path).map_err(|error| {
                StudioError::UpdateTransaction(format!(
                    "Gateway child {} command does not resolve: {error}",
                    config.name
                ))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StudioError::UpdateTransaction(format!(
                    "Gateway child {} command must be a regular non-symlink file",
                    config.name
                )));
            }
            let actual = fs::canonicalize(command_path)?;
            if actual.parent() != Some(trusted_bin_root.as_path()) {
                return Err(StudioError::UpdateTransaction(format!(
                    "Gateway child {} command is outside trusted bin_root",
                    config.name
                )));
            }
        }
        Ok(())
    }

    fn configured_count(&self) -> usize {
        self.configured.len()
    }

    fn enabled_count(&self) -> usize {
        self.configured.values().filter(|enabled| **enabled).count()
    }
}

fn valid_gateway_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn validate_child_projection(config: &GatewayChildConfigProjection) -> StudioResult<()> {
    if config.name.is_empty()
        || config.name.len() > 128
        || !config
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        || config.timeout_ms == 0
        || config.timeout_ms > 600_000
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway child config has invalid identity or timeout".into(),
        });
    }
    if config.command.trim().is_empty() || config.command.as_bytes().contains(&0) {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway child command is invalid".into(),
        });
    }
    if let Some(prefix) = &config.tool_prefix
        && (prefix.is_empty()
            || !prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway child tool prefix is invalid".into(),
        });
    }
    if config.tool_allowlist.len() > 256 {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway child allowlist is too large".into(),
        });
    }
    let mut allow = BTreeSet::new();
    for name in &config.tool_allowlist {
        if !valid_gateway_tool_name(name) || !allow.insert(name) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway child allowlist is invalid or duplicated".into(),
            });
        }
    }
    Ok(())
}

fn apply_child_exposure_policy(
    config: &GatewayChildConfigProjection,
    original: impl IntoIterator<Item = String>,
) -> StudioResult<Vec<String>> {
    validate_child_projection(config)?;
    let original = original.into_iter().collect::<BTreeSet<_>>();
    for name in &original {
        if !valid_gateway_tool_name(name) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!("independent child has invalid tool name: {}", config.name),
            });
        }
    }
    for allowed in &config.tool_allowlist {
        if !original.contains(allowed) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!(
                    "Gateway allowlist references an unknown child tool: {}",
                    config.name
                ),
            });
        }
    }
    let mut exposed = Vec::new();
    for name in original {
        if !config.tool_allowlist.is_empty() && !config.tool_allowlist.contains(&name) {
            continue;
        }
        let exposed_name = match &config.tool_prefix {
            Some(prefix) => format!("{prefix}{name}"),
            None => name,
        };
        if !valid_gateway_tool_name(&exposed_name) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!(
                    "Gateway exposed child tool name is invalid: {}",
                    config.name
                ),
            });
        }
        exposed.push(exposed_name);
    }
    Ok(exposed)
}

async fn probe_expected_child_tools(
    config: GatewayChildConfigProjection,
    trusted_bin_root: PathBuf,
) -> StudioResult<Vec<String>> {
    validate_child_projection(&config)?;
    let command_path = PathBuf::from(&config.command);
    let metadata =
        fs::symlink_metadata(&command_path).map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway child executable is unavailable: {}", config.name),
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway child executable is unsafe: {}", config.name),
        });
    }
    let actual = fs::canonicalize(&command_path)?;
    let trusted = fs::canonicalize(&trusted_bin_root)?;
    if actual.parent() != Some(trusted.as_path()) {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!(
                "Gateway child executable is outside trusted bin root: {}",
                config.name
            ),
        });
    }
    let mut command = Command::new(&actual);
    command
        .args(&config.args)
        .env_clear()
        .envs(&config.env)
        .kill_on_drop(true);
    let transport =
        TokioChildProcess::new(command).map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("failed to create independent child probe: {}", config.name),
        })?;
    let child_timeout = Duration::from_millis(config.timeout_ms);
    let running = timeout(child_timeout, ().serve(transport))
        .await
        .map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("independent child initialize timed out: {}", config.name),
        })?
        .map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("independent child initialize failed: {}", config.name),
        })?;
    let result = async {
        let tools = timeout(child_timeout, running.list_all_tools())
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!("independent child tools/list timed out: {}", config.name),
            })?
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!("independent child tools/list failed: {}", config.name),
            })?;
        if tools.len() > MAX_EXPECTED_TOOLS {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!(
                    "independent child tool catalog is too large: {}",
                    config.name
                ),
            });
        }
        let mut original = BTreeSet::new();
        for tool in &tools {
            let name = tool.name.as_ref();
            if !original.insert(name.to_owned()) {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: format!(
                        "independent child has duplicate tool names: {}",
                        config.name
                    ),
                });
            }
        }
        apply_child_exposure_policy(&config, original)
    }
    .await;
    let _ = running.cancel().await;
    result
}

async fn independently_expected_gateway_tools(
    gateway_binary: &Path,
    snapshot: &GatewayCatalogSnapshot,
) -> StudioResult<Vec<String>> {
    let trusted_bin_root =
        gateway_binary
            .parent()
            .ok_or_else(|| StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway binary has no trusted bin parent".into(),
            })?;
    snapshot.validate_trusted_commands(trusted_bin_root)?;
    let mut tasks = JoinSet::new();
    for config in snapshot.children.values().filter(|config| config.enabled) {
        tasks.spawn(probe_expected_child_tools(
            config.clone(),
            trusted_bin_root.to_path_buf(),
        ));
    }
    let mut expected = BTreeSet::from([
        GATEWAY_LIST_SERVERS.to_owned(),
        GATEWAY_RELOAD.to_owned(),
        GATEWAY_SET_ENABLED.to_owned(),
    ]);
    while let Some(joined) = tasks.join_next().await {
        let names = joined.map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "independent child catalog task failed".into(),
        })??;
        for name in names {
            if !expected.insert(name.clone()) {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: format!("Gateway expected tool-name collision: {name}"),
                });
            }
        }
        if expected.len() > MAX_EXPECTED_TOOLS {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway expected catalog exceeds tool budget".into(),
            });
        }
    }
    Ok(expected.into_iter().collect())
}

fn compare_gateway_tool_sets(observed: &[String], expected: &[String]) -> StudioResult<()> {
    let observed_set = observed.iter().cloned().collect::<BTreeSet<_>>();
    let expected_set = expected.iter().cloned().collect::<BTreeSet<_>>();
    if observed.len() != observed_set.len() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway observed catalog contains duplicate tool names".into(),
        });
    }
    if observed_set != expected_set {
        let missing = expected_set
            .difference(&observed_set)
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = observed_set
            .difference(&expected_set)
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!(
                "Gateway tool identity mismatch; missing={missing:?}; unexpected={unexpected:?}"
            ),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GatewayProbeSummary {
    configured_servers: usize,
    enabled_servers: usize,
    running_servers: usize,
    exposed_child_tools: usize,
    tool_names: Vec<String>,
}

#[async_trait]
trait GatewayProbe: Send + Sync {
    async fn verify(
        &self,
        binary: &Path,
        config_dir: &Path,
        expected: &GatewayCatalogSnapshot,
        version: &Version,
    ) -> StudioResult<GatewayProbeSummary>;
}

struct ProcessGatewayProbe;

#[derive(Debug, Deserialize)]
struct GatewayListResult {
    servers: Vec<GatewayServerStatus>,
    exposed_child_tools: usize,
    gateway_tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GatewayServerStatus {
    name: String,
    enabled: bool,
    running: bool,
    restart_policy: GatewayRestartPolicy,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GatewayReloadResult {
    configured_servers: usize,
    enabled_servers: usize,
    running_servers: usize,
    exposed_child_tools: usize,
    failed_servers: Vec<String>,
}

async fn call_gateway_tool(
    peer: &Peer<RoleClient>,
    name: &str,
    request_timeout: Duration,
    side_effecting: bool,
) -> StudioResult<CallToolResult> {
    let request =
        ClientRequest::CallToolRequest(Request::new(CallToolRequestParams::new(name.to_owned())));
    let mut handle = peer
        .send_cancellable_request(request, PeerRequestOptions::no_options())
        .await
        .map_err(|error| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway {name} request could not be dispatched: {error}"),
        })?;
    let mut deadline = Box::pin(sleep(request_timeout));
    let response = tokio::select! {
        response = &mut handle.rx => response.map_err(|_| ServiceError::TransportClosed),
        _ = &mut deadline => {
            let request_id = handle.id.clone();
            let cancel_error = handle
                .cancel(Some(format!("Studio verification timeout after {} ms", request_timeout.as_millis())))
                .await
                .err();
            let outcome = if side_effecting {
                "outcome is unknown; do not retry"
            } else {
                "request was cancelled"
            };
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!(
                    "Gateway {name} timed out after dispatch (request_id={request_id:?}); {outcome}; cancel_error={cancel_error:?}"
                ),
            });
        }
    };
    let response = response.map_err(|error| StudioError::UpdateVerificationFailed {
        component: ComponentId::Gateway.to_string(),
        detail: format!("Gateway {name} request failed after dispatch: {error}"),
    })?;
    let response = response.map_err(|error| StudioError::UpdateVerificationFailed {
        component: ComponentId::Gateway.to_string(),
        detail: format!("Gateway {name} request failed after dispatch: {error}"),
    })?;
    let ServerResult::CallToolResult(result) = response else {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway {name} returned an unexpected response type"),
        });
    };
    if result.is_error == Some(true) {
        let outcome_unknown = result.structured_content.as_ref().is_some_and(|content| {
            content.get("code").and_then(serde_json::Value::as_str) == Some("child_outcome_unknown")
        });
        let outcome = if outcome_unknown {
            "outcome is unknown; do not retry"
        } else {
            "Gateway returned an error"
        };
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway {name} failed: {outcome}"),
        });
    }
    Ok(result)
}

#[async_trait]
impl GatewayProbe for ProcessGatewayProbe {
    async fn verify(
        &self,
        binary: &Path,
        config_dir: &Path,
        expected: &GatewayCatalogSnapshot,
        version: &Version,
    ) -> StudioResult<GatewayProbeSummary> {
        verify_binary_version(binary, version).await?;
        let expected_tool_names = independently_expected_gateway_tools(binary, expected).await?;

        let mut command = Command::new(binary);
        command
            .arg("--config-dir")
            .arg(config_dir)
            .arg("--no-watch")
            .env_clear()
            .kill_on_drop(true);
        let transport = TokioChildProcess::new(command).map_err(|error| {
            StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!("failed to create Gateway protocol probe: {error}"),
            }
        })?;
        let running = timeout(PROBE_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway MCP initialize probe timed out".into(),
            })?
            .map_err(|error| StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!("Gateway MCP initialize probe failed: {error}"),
            })?;

        let result = async {
            let tools = timeout(PROBE_TIMEOUT, running.list_all_tools())
                .await
                .map_err(|_| StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: "Gateway tools/list probe timed out".into(),
                })?
                .map_err(|error| StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: format!("Gateway tools/list probe failed: {error}"),
                })?;
            let names = tools
                .iter()
                .map(|tool| tool.name.as_ref())
                .collect::<BTreeSet<_>>();
            let mut tool_names = tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>();
            tool_names.sort();
            for builtin in [GATEWAY_LIST_SERVERS, GATEWAY_RELOAD, GATEWAY_SET_ENABLED] {
                if !names.contains(builtin) {
                    return Err(StudioError::UpdateVerificationFailed {
                        component: ComponentId::Gateway.to_string(),
                        detail: format!("Gateway builtin tool is missing: {builtin}"),
                    });
                }
            }

            let response =
                call_gateway_tool(running.peer(), GATEWAY_LIST_SERVERS, PROBE_TIMEOUT, false)
                    .await?;
            let structured = response.structured_content.ok_or_else(|| {
                StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: "Gateway child catalog probe returned no structured result".into(),
                }
            })?;
            let status: GatewayListResult =
                serde_json::from_value(structured).map_err(|error| {
                    StudioError::UpdateVerificationFailed {
                        component: ComponentId::Gateway.to_string(),
                        detail: format!("Gateway child catalog result is malformed: {error}"),
                    }
                })?;
            verify_gateway_catalog_shape(&status, expected, &tool_names, &expected_tool_names)?;

            let reload =
                call_gateway_tool(running.peer(), GATEWAY_RELOAD, PROBE_TIMEOUT, true).await?;
            let reload: GatewayReloadResult =
                serde_json::from_value(reload.structured_content.ok_or_else(|| {
                    StudioError::UpdateVerificationFailed {
                        component: ComponentId::Gateway.to_string(),
                        detail: "Gateway reload returned no structured result".into(),
                    }
                })?)
                .map_err(|error| StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: format!("Gateway reload result is malformed: {error}"),
                })?;
            verify_reload_summary(&reload, &status)?;

            let after_reload =
                call_gateway_tool(running.peer(), GATEWAY_LIST_SERVERS, PROBE_TIMEOUT, false)
                    .await?;
            let after_reload: GatewayListResult =
                serde_json::from_value(after_reload.structured_content.ok_or_else(|| {
                    StudioError::UpdateVerificationFailed {
                        component: ComponentId::Gateway.to_string(),
                        detail: "Gateway post-reload catalog returned no structured result".into(),
                    }
                })?)
                .map_err(|error| StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: format!("Gateway post-reload catalog is malformed: {error}"),
                })?;
            verify_gateway_catalog_shape(
                &after_reload,
                expected,
                &tool_names,
                &expected_tool_names,
            )?;
            expected.verify_unchanged(config_dir)?;
            Ok(GatewayProbeSummary {
                configured_servers: status.servers.len(),
                enabled_servers: status
                    .servers
                    .iter()
                    .filter(|server| server.enabled)
                    .count(),
                running_servers: status
                    .servers
                    .iter()
                    .filter(|server| server.running)
                    .count(),
                exposed_child_tools: status.exposed_child_tools,
                tool_names,
            })
        }
        .await;
        let _ = running.cancel().await;
        result
    }
}

pub(crate) async fn probe_gateway_tool_names(
    binary: &Path,
    config_dir: &Path,
    version: &Version,
) -> StudioResult<Vec<String>> {
    let expected = GatewayCatalogSnapshot::capture(config_dir)?;
    Ok(ProcessGatewayProbe
        .verify(binary, config_dir, &expected, version)
        .await?
        .tool_names)
}

fn verify_gateway_catalog_shape(
    status: &GatewayListResult,
    expected: &GatewayCatalogSnapshot,
    tool_names: &[String],
    expected_tool_names: &[String],
) -> StudioResult<()> {
    verify_probe_status(status, expected)?;
    let names = tool_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for builtin in [GATEWAY_LIST_SERVERS, GATEWAY_RELOAD, GATEWAY_SET_ENABLED] {
        if !names.contains(builtin) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!("Gateway builtin tool is missing: {builtin}"),
            });
        }
    }
    if tool_names.len() != status.exposed_child_tools.saturating_add(3) {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway tool catalog count does not match child catalog".into(),
        });
    }
    compare_gateway_tool_sets(tool_names, expected_tool_names)
}

fn verify_probe_status(
    status: &GatewayListResult,
    expected: &GatewayCatalogSnapshot,
) -> StudioResult<()> {
    if status.servers.len() != expected.configured_count() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway configured child count changed".into(),
        });
    }
    let gateway_tools = status
        .gateway_tools
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for builtin in [GATEWAY_LIST_SERVERS, GATEWAY_RELOAD, GATEWAY_SET_ENABLED] {
        if !gateway_tools.contains(builtin) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway reported an incomplete builtin tool catalog".into(),
            });
        }
    }
    let mut seen = BTreeSet::new();
    for server in &status.servers {
        let Some(expected_enabled) = expected.configured.get(&server.name) else {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway reported an unexpected child MCP".into(),
            });
        };
        if !seen.insert(server.name.clone()) || server.enabled != *expected_enabled {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway child enabled state differs from servers.d".into(),
            });
        }
        if server.restart_policy != expected.children[&server.name].restart.policy {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway child restart policy differs from servers.d".into(),
            });
        }
        if server.enabled && (!server.running || server.error.is_some()) {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!(
                    "enabled Gateway child failed health verification: {}",
                    server.name
                ),
            });
        }
        if !server.enabled && server.running {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: format!(
                    "disabled Gateway child unexpectedly running: {}",
                    server.name
                ),
            });
        }
    }
    if seen.len() != expected.configured_count() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway child catalog is incomplete".into(),
        });
    }
    if expected.enabled_count() > 0 && status.exposed_child_tools == 0 {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway exposed child tool catalog is empty".into(),
        });
    }
    Ok(())
}

fn verify_reload_summary(
    reload: &GatewayReloadResult,
    status: &GatewayListResult,
) -> StudioResult<()> {
    let enabled_servers = status
        .servers
        .iter()
        .filter(|server| server.enabled)
        .count();
    let running_servers = status
        .servers
        .iter()
        .filter(|server| server.running)
        .count();
    if reload.configured_servers != status.servers.len()
        || reload.enabled_servers != enabled_servers
        || reload.running_servers != running_servers
        || reload.exposed_child_tools != status.exposed_child_tools
        || !reload.failed_servers.is_empty()
    {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway selective reload summary does not match healthy catalog".into(),
        });
    }
    Ok(())
}

#[derive(Clone)]
pub struct GatewayUpdateManager {
    catalog: ComponentCatalog,
    stager: ArtifactStager,
    provider: Arc<dyn ReleaseProvider>,
    owner: Arc<dyn GatewayOwner>,
    control: Option<GatewayControlClient>,
    probe: Arc<dyn GatewayProbe>,
    inventory: Arc<InventoryService>,
    events: EventHub,
    active: Arc<StdMutex<bool>>,
    transactions: Arc<Mutex<BTreeMap<String, GatewayTransactionRecord>>>,
    reconnect_timeout: Duration,
    reconnect_health_window: Duration,
}

impl GatewayUpdateManager {
    pub fn new(
        catalog: ComponentCatalog,
        tunnel: Arc<TunnelSupervisor>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> StudioResult<Self> {
        let stager = ArtifactStager::new(catalog.clone())?;
        let provider = Arc::new(ThirteenthXReleaseProvider::new(catalog.clone())?);
        let control = GatewayControlClient::from_catalog(&catalog);
        Ok(Self {
            catalog,
            stager,
            provider,
            owner: Arc::new(TunnelGatewayOwner { tunnel }),
            control: Some(control),
            probe: Arc::new(ProcessGatewayProbe),
            inventory,
            events,
            active: Arc::new(StdMutex::new(false)),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
            reconnect_timeout: RECONNECT_TIMEOUT,
            reconnect_health_window: RECONNECT_HEALTH_WINDOW,
        })
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        stager: ArtifactStager,
        provider: Arc<dyn ReleaseProvider>,
        owner: Arc<dyn GatewayOwner>,
        probe: Arc<dyn GatewayProbe>,
        inventory: Arc<InventoryService>,
        events: EventHub,
    ) -> Self {
        Self {
            catalog,
            stager,
            provider,
            owner,
            control: None,
            probe,
            inventory,
            events,
            active: Arc::new(StdMutex::new(false)),
            transactions: Arc::new(Mutex::new(BTreeMap::new())),
            reconnect_timeout: Duration::from_millis(80),
            reconnect_health_window: Duration::from_millis(20),
        }
    }

    pub async fn prepare(&self, version: Version) -> StudioResult<McpUpdateTransactionView> {
        let _guard = self.acquire()?;
        let current = self.inventory.get(ComponentId::Gateway).await?;
        let source_version = current.installed_version.ok_or_else(|| {
            StudioError::UpdateTransaction(
                "gateway: installed version is unavailable; update requires rollback baseline"
                    .into(),
            )
        })?;
        if !version.is_newer_than(&source_version) {
            return Err(StudioError::Conflict(format!(
                "gateway: target {version} must be newer than installed {source_version}"
            )));
        }
        let transaction_id = new_gateway_transaction_id();
        let preparing = McpUpdateTransactionView {
            transaction_id: transaction_id.clone(),
            component: ComponentId::Gateway,
            source_version: Some(source_version),
            target_version: version.clone(),
            phase: McpUpdatePhase::Preparing,
            was_running: None,
            rollback_succeeded: None,
            error: None,
            updated_at_ms: now_ms(),
        };
        self.transactions.lock().await.insert(
            transaction_id.clone(),
            GatewayTransactionRecord {
                view: preparing.clone(),
                staged_id: None,
                staged_identity: None,
            },
        );
        self.emit(&preparing);
        let policy = self.catalog.component(ComponentId::Gateway)?.clone();
        let result = async {
            let release = self.provider.release(&policy, &version).await?;
            let staged = self
                .stager
                .stage(self.provider.as_ref(), &release, self.inventory.platform())
                .await?;
            let staged_id = self.stager.staged_id(&staged)?;
            Ok::<_, StudioError>((staged_id, PreparedStagedIdentity::from(&staged)))
        }
        .await;
        match result {
            Ok((staged_id, staged_identity)) => {
                let mut transactions = self.transactions.lock().await;
                let record = transactions
                    .get_mut(&transaction_id)
                    .expect("Gateway preparing transaction exists");
                record.staged_id = Some(staged_id);
                record.staged_identity = Some(staged_identity);
                record.view.phase = McpUpdatePhase::Staged;
                record.view.updated_at_ms = now_ms();
                let view = record.view.clone();
                drop(transactions);
                self.emit(&view);
                Ok(view)
            }
            Err(error) => {
                self.update_failed(&transaction_id, None, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    pub async fn apply(&self, transaction_id: &str) -> StudioResult<McpUpdateTransactionView> {
        let _runtime_guard = self
            .catalog
            .runtime_operations()
            .acquire_control("gateway_update")?;
        let _guard = self.acquire()?;
        let (staged_id, source_version, expected_staged) =
            self.ensure_transaction_applicable(transaction_id).await?;
        let current = self.inventory.get(ComponentId::Gateway).await?;
        if current.installed_version.as_ref() != Some(&source_version) {
            return Err(StudioError::Conflict(
                "gateway: installed version changed after update preparation".into(),
            ));
        }
        let staged = self.stager.load_ready(&staged_id)?;
        if staged.component != ComponentId::Gateway
            || PreparedStagedIdentity::from(&staged) != expected_staged
        {
            return Err(StudioError::Conflict(
                "gateway: staged artifact changed after preparation".into(),
            ));
        }
        self.apply_staged(transaction_id, source_version, staged)
            .await
    }

    pub(crate) async fn artifact_history_identity(
        &self,
        transaction_id: &str,
    ) -> Option<ArtifactHistoryIdentity> {
        self.transactions
            .lock()
            .await
            .get(transaction_id)
            .and_then(|record| record.staged_identity.as_ref())
            .map(|identity| identity.history_identity(ComponentId::Gateway))
    }

    pub async fn transaction(
        &self,
        transaction_id: &str,
    ) -> StudioResult<McpUpdateTransactionView> {
        self.transactions
            .lock()
            .await
            .get(transaction_id)
            .map(|record| record.view.clone())
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))
    }

    async fn ensure_transaction_applicable(
        &self,
        transaction_id: &str,
    ) -> StudioResult<(String, Version, PreparedStagedIdentity)> {
        if !safe_gateway_transaction_id(transaction_id) {
            return Err(StudioError::NotFound(transaction_id.to_owned()));
        }
        let transactions = self.transactions.lock().await;
        let record = transactions
            .get(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        if record.view.phase != McpUpdatePhase::Staged {
            return Err(StudioError::Conflict(format!(
                "Gateway transaction {transaction_id} is not staged"
            )));
        }
        Ok((
            record.staged_id.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("Gateway transaction has no staged artifact".into())
            })?,
            record.view.source_version.clone().ok_or_else(|| {
                StudioError::UpdateTransaction("Gateway transaction has no source version".into())
            })?,
            record.staged_identity.clone().ok_or_else(|| {
                StudioError::UpdateTransaction(
                    "Gateway transaction has no staging fingerprint".into(),
                )
            })?,
        ))
    }

    async fn apply_staged(
        &self,
        transaction_id: &str,
        source_version: Version,
        staged: StagedArtifact,
    ) -> StudioResult<McpUpdateTransactionView> {
        let target_version = staged.version.clone();
        let target = self.catalog.install_path(ComponentId::Gateway)?;
        self.ensure_target_confined(&target)?;
        let servers_dir = self.catalog.runtime_root().join("gateway/servers.d");
        let catalog_snapshot = GatewayCatalogSnapshot::capture(&servers_dir)?;
        catalog_snapshot.validate_managed_paths(&self.catalog)?;
        self.owner.validate_binding(&target, &servers_dir)?;
        let owner_state = self.owner.state().await?;
        if matches!(
            owner_state,
            GatewayOwnerState::Transitional | GatewayOwnerState::Failed
        ) {
            return Err(StudioError::Conflict(
                "Gateway owner must be stopped or running before update".into(),
            ));
        }
        let was_running = owner_state == GatewayOwnerState::Running;
        let staged_binary = staged.validated_executables.first().ok_or_else(|| {
            StudioError::UpdateTransaction("staged Gateway artifact has no executable".into())
        })?;
        if staged_binary.file_name().and_then(|value| value.to_str()) != Some("rust-mcp-gateway") {
            return Err(StudioError::UpdateTransaction(
                "staged Gateway executable name does not match catalog target".into(),
            ));
        }
        let rollback = prepare_rollback(&target, ComponentId::Gateway, transaction_id)?;

        if was_running {
            self.update_phase(
                transaction_id,
                McpUpdatePhase::GatewayStopping,
                Some(true),
                None,
                None,
            )
            .await?;
            let drain_generation = match self.control.as_ref() {
                Some(control) => match control.drain_for_update().await {
                    Ok(generation) => generation,
                    Err(error) => {
                        self.update_failed(
                            transaction_id,
                            Some(true),
                            format!("Gateway drain prerequisite failed: {error}"),
                        )
                        .await?;
                        return Err(error);
                    }
                },
                None => 0,
            };
            if let Err(error) = self.owner.stop().await {
                let resume_error = if drain_generation != 0 {
                    self.control
                        .as_ref()
                        .expect("Gateway control exists for a drain generation")
                        .resume(drain_generation)
                        .await
                        .err()
                } else {
                    None
                };
                let _ = remove_rollback_material(&rollback);
                self.update_failed(
                    transaction_id,
                    Some(true),
                    format!("stop failed: {error}; drain resume failed: {resume_error:?}"),
                )
                .await?;
                return Err(error);
            }
        }

        self.update_phase(
            transaction_id,
            McpUpdatePhase::Activating,
            Some(was_running),
            None,
            None,
        )
        .await?;
        if let Err(error) =
            activate_binary(staged_binary, &target, ComponentId::Gateway, transaction_id)
        {
            return self
                .rollback_after_failure(GatewayRollbackContext {
                    transaction_id,
                    source_version,
                    target_version,
                    target: &target,
                    servers_dir: &servers_dir,
                    catalog_snapshot: &catalog_snapshot,
                    rollback: &rollback,
                    was_running,
                    reason: format!("activation failed: {error}"),
                    new_owner_started: false,
                })
                .await;
        }

        self.update_phase(
            transaction_id,
            McpUpdatePhase::GatewayCatalogVerifying,
            Some(was_running),
            None,
            None,
        )
        .await?;
        if let Err(error) = self
            .probe
            .verify(&target, &servers_dir, &catalog_snapshot, &target_version)
            .await
        {
            return self
                .rollback_after_failure(GatewayRollbackContext {
                    transaction_id,
                    source_version,
                    target_version,
                    target: &target,
                    servers_dir: &servers_dir,
                    catalog_snapshot: &catalog_snapshot,
                    rollback: &rollback,
                    was_running,
                    reason: format!("Gateway catalog verification failed: {error}"),
                    new_owner_started: false,
                })
                .await;
        }
        if let Err(error) = catalog_snapshot.verify_unchanged(&servers_dir) {
            return self
                .rollback_after_failure(GatewayRollbackContext {
                    transaction_id,
                    source_version,
                    target_version,
                    target: &target,
                    servers_dir: &servers_dir,
                    catalog_snapshot: &catalog_snapshot,
                    rollback: &rollback,
                    was_running,
                    reason: format!("Gateway config preservation failed: {error}"),
                    new_owner_started: false,
                })
                .await;
        }

        if was_running {
            self.update_phase(
                transaction_id,
                McpUpdatePhase::GatewayRestarting,
                Some(true),
                None,
                None,
            )
            .await?;
            if let Err(error) = self.owner.start().await {
                return self
                    .rollback_after_failure(GatewayRollbackContext {
                        transaction_id,
                        source_version,
                        target_version,
                        target: &target,
                        servers_dir: &servers_dir,
                        catalog_snapshot: &catalog_snapshot,
                        rollback: &rollback,
                        was_running,
                        reason: format!("Gateway owner restart failed: {error}"),
                        new_owner_started: false,
                    })
                    .await;
            }
            self.update_phase(
                transaction_id,
                McpUpdatePhase::GatewayReconnecting,
                Some(true),
                None,
                None,
            )
            .await?;
            if let Err(error) = self.verify_owner_reconnected(&target, &servers_dir).await {
                return self
                    .rollback_after_failure(GatewayRollbackContext {
                        transaction_id,
                        source_version,
                        target_version,
                        target: &target,
                        servers_dir: &servers_dir,
                        catalog_snapshot: &catalog_snapshot,
                        rollback: &rollback,
                        was_running,
                        reason: format!("Gateway reconnect verification failed: {error}"),
                        new_owner_started: true,
                    })
                    .await;
            }
        }

        catalog_snapshot.verify_unchanged(&servers_dir)?;
        remove_rollback_material(&rollback)?;
        self.inventory
            .set_desired(ComponentId::Gateway, target_version)
            .await;
        self.update_phase(
            transaction_id,
            McpUpdatePhase::Completed,
            Some(was_running),
            None,
            None,
        )
        .await
    }

    async fn verify_owner_reconnected(
        &self,
        target: &Path,
        servers_dir: &Path,
    ) -> StudioResult<()> {
        self.owner.validate_binding(target, servers_dir)?;
        let deadline = tokio::time::Instant::now() + self.reconnect_timeout;
        loop {
            match self.owner.state().await? {
                GatewayOwnerState::Running => break,
                GatewayOwnerState::Transitional => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(StudioError::UpdateVerificationFailed {
                            component: ComponentId::Gateway.to_string(),
                            detail: "tunnel owner reconnect timed out".into(),
                        });
                    }
                    sleep(RECONNECT_POLL_INTERVAL.min(self.reconnect_timeout)).await;
                }
                GatewayOwnerState::Stopped | GatewayOwnerState::Failed => {
                    return Err(StudioError::UpdateVerificationFailed {
                        component: ComponentId::Gateway.to_string(),
                        detail: "tunnel owner entered a terminal state during reconnect".into(),
                    });
                }
            }
        }
        sleep(self.reconnect_health_window).await;
        if self.owner.state().await? != GatewayOwnerState::Running {
            return Err(StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "tunnel owner did not remain running during reconnect health window".into(),
            });
        }
        self.owner.validate_binding(target, servers_dir)
    }

    async fn rollback_after_failure(
        &self,
        context: GatewayRollbackContext<'_>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let GatewayRollbackContext {
            transaction_id,
            source_version,
            target_version,
            target,
            servers_dir,
            catalog_snapshot,
            rollback,
            was_running,
            reason,
            new_owner_started,
        } = context;
        self.update_phase(
            transaction_id,
            McpUpdatePhase::RollingBack,
            Some(was_running),
            None,
            Some(reason.clone()),
        )
        .await?;

        if new_owner_started
            && matches!(
                self.owner.state().await?,
                GatewayOwnerState::Running | GatewayOwnerState::Transitional
            )
            && let Err(error) = self.owner.stop().await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    target_version,
                    was_running,
                    format!("{reason}; failed to stop new Gateway owner: {error}"),
                )
                .await;
        }
        if let Err(error) = restore_rollback(target, rollback) {
            return self
                .rollback_failed(
                    transaction_id,
                    target_version,
                    was_running,
                    format!("{reason}; rollback activation failed: {error}"),
                )
                .await;
        }
        if let Err(error) = catalog_snapshot.verify_unchanged(servers_dir) {
            return self
                .rollback_failed(
                    transaction_id,
                    target_version,
                    was_running,
                    format!("{reason}; servers.d changed during rollback: {error}"),
                )
                .await;
        }
        if let Err(error) = self
            .probe
            .verify(target, servers_dir, catalog_snapshot, &source_version)
            .await
        {
            return self
                .rollback_failed(
                    transaction_id,
                    target_version,
                    was_running,
                    format!("{reason}; restored Gateway verification failed: {error}"),
                )
                .await;
        }
        if was_running {
            if let Err(error) = self.owner.start().await {
                return self
                    .rollback_failed(
                        transaction_id,
                        target_version,
                        true,
                        format!("{reason}; rollback reconnect start failed: {error}"),
                    )
                    .await;
            }
            if let Err(error) = self.verify_owner_reconnected(target, servers_dir).await {
                return self
                    .rollback_failed(
                        transaction_id,
                        target_version,
                        true,
                        format!("{reason}; rollback reconnect verification failed: {error}"),
                    )
                    .await;
            }
        }

        self.update_phase(
            transaction_id,
            McpUpdatePhase::Failed,
            Some(was_running),
            Some(true),
            Some(reason.clone()),
        )
        .await?;
        Err(StudioError::UpdateTransaction(reason))
    }

    async fn rollback_failed(
        &self,
        transaction_id: &str,
        target_version: Version,
        was_running: bool,
        detail: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        let view = McpUpdateTransactionView {
            transaction_id: transaction_id.to_owned(),
            component: ComponentId::Gateway,
            source_version: None,
            target_version,
            phase: McpUpdatePhase::RollbackFailed,
            was_running: Some(was_running),
            rollback_succeeded: Some(false),
            error: Some(sanitize_transaction_error(&detail)),
            updated_at_ms: now_ms(),
        };
        self.set_record(view).await;
        Err(StudioError::RollbackFailed {
            component: ComponentId::Gateway.to_string(),
            detail,
        })
    }

    async fn update_failed(
        &self,
        transaction_id: &str,
        was_running: Option<bool>,
        detail: String,
    ) -> StudioResult<McpUpdateTransactionView> {
        let mut transactions = self.transactions.lock().await;
        let record = transactions
            .get_mut(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        record.view.phase = McpUpdatePhase::Failed;
        record.view.was_running = was_running;
        record.view.error = Some(sanitize_transaction_error(&detail));
        record.view.updated_at_ms = now_ms();
        let view = record.view.clone();
        drop(transactions);
        self.emit(&view);
        Ok(view)
    }

    async fn update_phase(
        &self,
        transaction_id: &str,
        phase: McpUpdatePhase,
        was_running: Option<bool>,
        rollback_succeeded: Option<bool>,
        error: Option<String>,
    ) -> StudioResult<McpUpdateTransactionView> {
        let mut transactions = self.transactions.lock().await;
        let record = transactions
            .get_mut(transaction_id)
            .ok_or_else(|| StudioError::NotFound(transaction_id.to_owned()))?;
        record.view.phase = phase;
        if was_running.is_some() {
            record.view.was_running = was_running;
        }
        if rollback_succeeded.is_some() {
            record.view.rollback_succeeded = rollback_succeeded;
        }
        if let Some(error) = error {
            record.view.error = Some(sanitize_transaction_error(&error));
        }
        record.view.updated_at_ms = now_ms();
        let view = record.view.clone();
        drop(transactions);
        self.emit(&view);
        Ok(view)
    }

    async fn set_record(&self, mut view: McpUpdateTransactionView) {
        let mut transactions = self.transactions.lock().await;
        if let Some(record) = transactions.get_mut(&view.transaction_id) {
            if view.source_version.is_none() {
                view.source_version = record.view.source_version.clone();
            }
            record.view = view.clone();
        }
        drop(transactions);
        self.emit(&view);
    }

    fn emit(&self, view: &McpUpdateTransactionView) {
        self.events.publish(StudioEvent::UpdateTransaction {
            transaction: view.clone(),
        });
    }

    fn acquire(&self) -> StudioResult<GatewayUpdateGuard> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| StudioError::UpdateTransaction("Gateway update lock poisoned".into()))?;
        if *active {
            return Err(StudioError::Conflict(
                "Gateway update is already in progress".into(),
            ));
        }
        *active = true;
        Ok(GatewayUpdateGuard {
            active: self.active.clone(),
        })
    }

    fn ensure_target_confined(&self, target: &Path) -> StudioResult<()> {
        let metadata = fs::symlink_metadata(target).map_err(|error| {
            StudioError::UpdateTransaction(format!("Gateway target metadata unavailable: {error}"))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::UpdateTransaction(
                "Gateway target must be a regular non-symlink file".into(),
            ));
        }
        let parent = target
            .parent()
            .ok_or_else(|| StudioError::UpdateTransaction("Gateway target has no parent".into()))?;
        if fs::canonicalize(parent)? != fs::canonicalize(self.catalog.bin_root())? {
            return Err(StudioError::UpdateTransaction(
                "Gateway target is not directly inside bin_root".into(),
            ));
        }
        Ok(())
    }
}

struct GatewayRollbackContext<'a> {
    transaction_id: &'a str,
    source_version: Version,
    target_version: Version,
    target: &'a Path,
    servers_dir: &'a Path,
    catalog_snapshot: &'a GatewayCatalogSnapshot,
    rollback: &'a super::transaction::RollbackMaterial,
    was_running: bool,
    reason: String,
    new_owner_started: bool,
}

#[derive(Debug)]
struct GatewayUpdateGuard {
    active: Arc<StdMutex<bool>>,
}

impl Drop for GatewayUpdateGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = false;
        }
    }
}

async fn verify_binary_version(path: &Path, expected: &Version) -> StudioResult<()> {
    let output = timeout(
        Duration::from_secs(5),
        Command::new(path)
            .arg("--version")
            .env_clear()
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| StudioError::UpdateVerificationFailed {
        component: ComponentId::Gateway.to_string(),
        detail: "Gateway --version probe timed out".into(),
    })?
    .map_err(|error| StudioError::UpdateVerificationFailed {
        component: ComponentId::Gateway.to_string(),
        detail: format!("Gateway --version probe failed: {error}"),
    })?;
    if !output.status.success() {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway --version exited with {}", output.status),
        });
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: "Gateway --version output is not UTF-8".into(),
        })?;
    let token =
        stdout
            .split_whitespace()
            .last()
            .ok_or_else(|| StudioError::UpdateVerificationFailed {
                component: ComponentId::Gateway.to_string(),
                detail: "Gateway --version output is empty".into(),
            })?;
    let actual = Version::parse(token).map_err(|_| StudioError::UpdateVerificationFailed {
        component: ComponentId::Gateway.to_string(),
        detail: "Gateway reported invalid semantic version".into(),
    })?;
    if &actual != expected {
        return Err(StudioError::UpdateVerificationFailed {
            component: ComponentId::Gateway.to_string(),
            detail: format!("Gateway reports {actual}, expected {expected}"),
        });
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn safe_gateway_transaction_id(value: &str) -> bool {
    value.starts_with("txn-gateway-")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn new_gateway_transaction_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "txn-gateway-{}-{}-{}",
        std::process::id(),
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex as StdMutex};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::super::transaction::sha256_path;

    use super::*;
    use crate::update::{
        Architecture, AvailableRelease, HostRuntimeRoots, OperatingSystem, Platform, ReleaseAsset,
    };

    #[derive(Default)]
    struct FakeOwner {
        state: StdMutex<Option<GatewayOwnerState>>,
        stop_count: StdMutex<usize>,
        start_count: StdMutex<usize>,
        fail_stop: StdMutex<bool>,
        fail_start: StdMutex<bool>,
        fail_start_always: StdMutex<bool>,
        next_start_state: StdMutex<Option<GatewayOwnerState>>,
        fail_binding: StdMutex<bool>,
    }

    #[async_trait]
    impl GatewayOwner for FakeOwner {
        async fn state(&self) -> StudioResult<GatewayOwnerState> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .unwrap_or(GatewayOwnerState::Stopped))
        }

        async fn stop(&self) -> StudioResult<()> {
            if *self.fail_stop.lock().unwrap() {
                return Err(StudioError::Process(
                    "forced Gateway owner stop failure".into(),
                ));
            }
            *self.stop_count.lock().unwrap() += 1;
            *self.state.lock().unwrap() = Some(GatewayOwnerState::Stopped);
            Ok(())
        }

        async fn start(&self) -> StudioResult<()> {
            if *self.fail_start_always.lock().unwrap() {
                return Err(StudioError::Process(
                    "forced persistent Gateway owner start failure".into(),
                ));
            }
            {
                let mut fail = self.fail_start.lock().unwrap();
                if *fail {
                    *fail = false;
                    return Err(StudioError::Process(
                        "forced Gateway owner start failure".into(),
                    ));
                }
            }
            *self.start_count.lock().unwrap() += 1;
            let next = self
                .next_start_state
                .lock()
                .unwrap()
                .take()
                .unwrap_or(GatewayOwnerState::Running);
            *self.state.lock().unwrap() = Some(next);
            Ok(())
        }

        fn validate_binding(
            &self,
            _gateway_binary: &Path,
            _servers_dir: &Path,
        ) -> StudioResult<()> {
            if *self.fail_binding.lock().unwrap() {
                Err(StudioError::Config("forced ownership conflict".into()))
            } else {
                Ok(())
            }
        }
    }

    struct FakeProbe {
        fail_version: StdMutex<Option<Version>>,
    }

    impl Default for FakeProbe {
        fn default() -> Self {
            Self {
                fail_version: StdMutex::new(None),
            }
        }
    }

    #[async_trait]
    impl GatewayProbe for FakeProbe {
        async fn verify(
            &self,
            _binary: &Path,
            _config_dir: &Path,
            expected: &GatewayCatalogSnapshot,
            version: &Version,
        ) -> StudioResult<GatewayProbeSummary> {
            if self
                .fail_version
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|fail| fail == version)
            {
                return Err(StudioError::UpdateVerificationFailed {
                    component: ComponentId::Gateway.to_string(),
                    detail: "forced Gateway probe failure".into(),
                });
            }
            Ok(GatewayProbeSummary {
                configured_servers: expected.configured_count(),
                enabled_servers: expected.enabled_count(),
                running_servers: expected.enabled_count(),
                exposed_child_tools: expected.enabled_count(),
                tool_names: vec![
                    GATEWAY_LIST_SERVERS.into(),
                    GATEWAY_RELOAD.into(),
                    GATEWAY_SET_ENABLED.into(),
                ],
            })
        }
    }

    struct NoopProvider;

    #[async_trait]
    impl ReleaseProvider for NoopProvider {
        fn provider_id(&self) -> super::super::ReleaseProviderId {
            super::super::ReleaseProviderId::ThirteenthXGitHub
        }
        async fn latest_release(
            &self,
            _component: &super::super::ComponentPolicy,
        ) -> StudioResult<AvailableRelease> {
            unreachable!()
        }
        async fn release(
            &self,
            _component: &super::super::ComponentPolicy,
            _version: &Version,
        ) -> StudioResult<AvailableRelease> {
            unreachable!()
        }
        fn select_asset<'a>(
            &self,
            _release: &'a AvailableRelease,
            _platform: Platform,
        ) -> StudioResult<&'a ReleaseAsset> {
            unreachable!()
        }
        async fn checksum_manifest(&self, _release: &AvailableRelease) -> StudioResult<Vec<u8>> {
            unreachable!()
        }
    }

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn write_version_script(path: &Path, version: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!("#!/bin/sh\necho 'rust-mcp-gateway {version}'\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn write_servers_dir(root: &Path) -> PathBuf {
        let dir = root.join("runtime/gateway/servers.d");
        fs::create_dir_all(&dir).unwrap();
        let git = root.join("bin/rust-mcp-git");
        let disabled = root.join("bin/disabled-launcher");
        fs::create_dir_all(git.parent().unwrap()).unwrap();
        fs::write(&git, b"git").unwrap();
        fs::write(&disabled, b"disabled").unwrap();
        fs::write(
            dir.join("git.yaml"),
            format!(
                "name: git
enabled: true
command: {}
args: []
",
                git.display()
            ),
        )
        .unwrap();
        fs::write(
            dir.join("disabled.yaml"),
            format!(
                "name: disabled
enabled: false
command: {}
args: []
",
                disabled.display()
            ),
        )
        .unwrap();
        dir
    }

    fn create_ready(
        stager: &ArtifactStager,
        catalog: &ComponentCatalog,
        version: &str,
        ready_id: &str,
    ) -> StagedArtifact {
        let policy = catalog.component(ComponentId::Gateway).unwrap();
        let root = stager.staging_root().join(ready_id);
        let package_root = root
            .join("extracted")
            .join(format!("rust-mcp-gateway-v{version}-darwin-arm64"));
        let staged_binary = package_root.join("rust-mcp-gateway");
        write_version_script(&staged_binary, version);
        let asset_name = format!("rust-mcp-gateway-v{version}-darwin-arm64.tar.gz");
        let archive_dir = root.join("archive");
        fs::create_dir_all(&archive_dir).unwrap();
        let archive = archive_dir.join(&asset_name);
        fs::write(&archive, b"verified-gateway-test-archive").unwrap();
        let archive_sha = sha256_path(&archive).unwrap();
        fs::write(
            archive_dir.join("SHA256SUMS.txt"),
            format!("{archive_sha}  {asset_name}\n"),
        )
        .unwrap();
        let staged = StagedArtifact {
            component: ComponentId::Gateway,
            version: Version::parse(version).unwrap(),
            provider: policy.provider,
            release_tag: format!("v{version}"),
            platform: platform(),
            asset_name,
            archive_sha256: archive_sha,
            companion_asset_name: None,
            companion_archive_sha256: None,
            staging_path: root.clone(),
            package_root,
            validated_executables: vec![staged_binary.clone()],
            validated_executable_sha256: vec![sha256_path(&staged_binary).unwrap()],
            verified_at_unix_seconds: 1,
        };
        fs::write(
            root.join("staged.json"),
            serde_json::to_vec_pretty(&staged).unwrap(),
        )
        .unwrap();
        staged
    }

    fn fixture(
        running: bool,
    ) -> (
        tempfile::TempDir,
        GatewayUpdateManager,
        Arc<FakeOwner>,
        Arc<FakeProbe>,
        PathBuf,
        PathBuf,
    ) {
        let root = tempfile::TempDir::new().unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.path().join("bin"), root.path().join("runtime")).unwrap(),
        );
        fs::create_dir_all(catalog.bin_root()).unwrap();
        fs::create_dir_all(catalog.runtime_root()).unwrap();
        let target = catalog.install_path(ComponentId::Gateway).unwrap();
        write_version_script(&target, "1.0.0");
        let servers_dir = write_servers_dir(root.path());
        let stager = ArtifactStager::new(catalog.clone()).unwrap();
        let owner = Arc::new(FakeOwner::default());
        *owner.state.lock().unwrap() = Some(if running {
            GatewayOwnerState::Running
        } else {
            GatewayOwnerState::Stopped
        });
        let probe = Arc::new(FakeProbe::default());
        let inventory = Arc::new(
            InventoryService::new(
                catalog.clone(),
                root.path().join("source"),
                platform(),
                BTreeMap::new(),
            )
            .unwrap(),
        );
        let manager = GatewayUpdateManager::with_dependencies(
            catalog,
            stager,
            Arc::new(NoopProvider),
            owner.clone(),
            probe.clone(),
            inventory,
            EventHub::default(),
        );
        (root, manager, owner, probe, target, servers_dir)
    }

    async fn register_ready(
        manager: &GatewayUpdateManager,
        staged: &StagedArtifact,
        transaction_id: &str,
        ready_id: &str,
    ) {
        let source_version = manager
            .inventory
            .get(ComponentId::Gateway)
            .await
            .unwrap()
            .installed_version;
        manager.transactions.lock().await.insert(
            transaction_id.to_owned(),
            GatewayTransactionRecord {
                view: McpUpdateTransactionView {
                    transaction_id: transaction_id.to_owned(),
                    component: ComponentId::Gateway,
                    source_version,
                    target_version: staged.version.clone(),
                    phase: McpUpdatePhase::Staged,
                    was_running: None,
                    rollback_succeeded: None,
                    error: None,
                    updated_at_ms: now_ms(),
                },
                staged_id: Some(ready_id.to_owned()),
                staged_identity: Some(PreparedStagedIdentity::from(staged)),
            },
        );
    }

    #[tokio::test]
    async fn stopped_gateway_updates_without_starting_tunnel_and_preserves_config() {
        let (_root, manager, owner, _probe, target, servers_dir) = fixture(false);
        let before = GatewayCatalogSnapshot::capture(&servers_dir).unwrap();
        let ready_id = "ready-gateway-stopped";
        let tx = "txn-gateway-stopped";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;

        let result = manager.apply(tx).await.unwrap();
        assert_eq!(result.phase, McpUpdatePhase::Completed);
        assert_eq!(*owner.start_count.lock().unwrap(), 0);
        assert_eq!(*owner.stop_count.lock().unwrap(), 0);
        before.verify_unchanged(&servers_dir).unwrap();
        assert!(String::from_utf8_lossy(&fs::read(target).unwrap()).contains("1.1.0"));
    }

    #[tokio::test]
    async fn gateway_control_client_requires_matching_drained_generation() {
        let root = tempfile::TempDir::new().unwrap();
        let control_dir = root.path().join("runtime/gateway/control");
        fs::create_dir_all(&control_dir).unwrap();
        let socket = control_dir.join("gateway.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            for response in [
                r#"{"ok":true,"state":"DRAINED","drain_generation":9}"#,
                r#"{"ok":true,"state":"RUNNING","drain_generation":9}"#,
                r#"{"ok":true,"state":"RUNNING","drain_generation":10}"#,
            ] {
                let (stream, _) = listener.accept().await.unwrap();
                let (read, mut write) = stream.into_split();
                let mut line = Vec::new();
                BufReader::new(read)
                    .read_until(b'\n', &mut line)
                    .await
                    .unwrap();
                assert!(line.ends_with(b"\n"));
                write.write_all(response.as_bytes()).await.unwrap();
                write.write_all(b"\n").await.unwrap();
            }
        });
        let client = GatewayControlClient {
            socket: socket.clone(),
        };
        assert_eq!(client.drain_for_update().await.unwrap(), 9);
        client.resume(9).await.unwrap();
        client.reload().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn running_gateway_stops_owner_reconnects_and_preserves_catalog() {
        let (_root, manager, owner, _probe, _target, servers_dir) = fixture(true);
        let before = GatewayCatalogSnapshot::capture(&servers_dir).unwrap();
        let ready_id = "ready-gateway-running";
        let tx = "txn-gateway-running";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;

        let result = manager.apply(tx).await.unwrap();
        assert_eq!(result.phase, McpUpdatePhase::Completed);
        assert_eq!(*owner.stop_count.lock().unwrap(), 1);
        assert_eq!(*owner.start_count.lock().unwrap(), 1);
        assert_eq!(owner.state().await.unwrap(), GatewayOwnerState::Running);
        before.verify_unchanged(&servers_dir).unwrap();
    }

    #[tokio::test]
    async fn new_gateway_probe_failure_rolls_back_and_restores_owner() {
        let (_root, manager, owner, probe, target, _servers_dir) = fixture(true);
        *probe.fail_version.lock().unwrap() = Some(Version::parse("1.1.0").unwrap());
        let ready_id = "ready-gateway-probefail";
        let tx = "txn-gateway-probefail";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;

        assert!(manager.apply(tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(target).unwrap()).contains("1.0.0"));
        assert_eq!(owner.state().await.unwrap(), GatewayOwnerState::Running);
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn reconnect_failure_rolls_back_previous_gateway() {
        let (_root, manager, owner, _probe, target, _servers_dir) = fixture(true);
        *owner.fail_start.lock().unwrap() = true;
        let ready_id = "ready-gateway-reconnectfail";
        let tx = "txn-gateway-reconnectfail";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;

        assert!(manager.apply(tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(target).unwrap()).contains("1.0.0"));
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
        assert_eq!(owner.state().await.unwrap(), GatewayOwnerState::Running);
    }

    #[tokio::test]
    async fn reconnect_timeout_rolls_back_and_old_owner_recovers() {
        let (_root, manager, owner, _probe, target, _servers_dir) = fixture(true);
        *owner.next_start_state.lock().unwrap() = Some(GatewayOwnerState::Transitional);
        let ready_id = "ready-gateway-timeout";
        let tx = "txn-gateway-timeout";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;

        assert!(manager.apply(tx).await.is_err());
        assert!(String::from_utf8_lossy(&fs::read(target).unwrap()).contains("1.0.0"));
        assert_eq!(owner.state().await.unwrap(), GatewayOwnerState::Running);
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::Failed);
        assert_eq!(view.rollback_succeeded, Some(true));
    }

    #[tokio::test]
    async fn rollback_reconnect_failure_is_explicit() {
        let (_root, manager, owner, probe, _target, _servers_dir) = fixture(true);
        *probe.fail_version.lock().unwrap() = Some(Version::parse("1.1.0").unwrap());
        *owner.fail_start_always.lock().unwrap() = true;
        let ready_id = "ready-gateway-rollbackfail";
        let tx = "txn-gateway-rollbackfail";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;

        assert!(matches!(
            manager.apply(tx).await.unwrap_err(),
            StudioError::RollbackFailed { .. }
        ));
        let view = manager.transaction(tx).await.unwrap();
        assert_eq!(view.phase, McpUpdatePhase::RollbackFailed);
        assert_eq!(view.rollback_succeeded, Some(false));
    }

    #[tokio::test]
    async fn ownership_conflict_tamper_and_duplicate_apply_fail_before_control_path_stop() {
        let (_root, manager, owner, _probe, target, _servers_dir) = fixture(true);
        *owner.fail_binding.lock().unwrap() = true;
        let ready_id = "ready-gateway-ownerconflict";
        let tx = "txn-gateway-ownerconflict";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(*owner.stop_count.lock().unwrap(), 0);
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));

        let (_root, manager, owner, _probe, target, _servers_dir) = fixture(true);
        let ready_id = "ready-gateway-tamper";
        let tx = "txn-gateway-tamper";
        let staged = create_ready(&manager.stager, &manager.catalog, "1.1.0", ready_id);
        register_ready(&manager, &staged, tx, ready_id).await;
        fs::write(&staged.validated_executables[0], b"tampered").unwrap();
        assert!(manager.apply(tx).await.is_err());
        assert_eq!(*owner.stop_count.lock().unwrap(), 0);
        assert!(String::from_utf8_lossy(&fs::read(&target).unwrap()).contains("1.0.0"));

        let _guard = manager.acquire().unwrap();
        assert!(matches!(
            manager.acquire().unwrap_err(),
            StudioError::Conflict(_)
        ));
    }

    #[test]
    fn child_catalog_status_verification_detects_failed_or_missing_children() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = write_servers_dir(root.path());
        let expected = GatewayCatalogSnapshot::capture(&dir).unwrap();
        let valid = GatewayListResult {
            servers: vec![
                GatewayServerStatus {
                    name: "git".into(),
                    enabled: true,
                    running: true,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
                GatewayServerStatus {
                    name: "disabled".into(),
                    enabled: false,
                    running: false,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
            ],
            exposed_child_tools: 2,
            gateway_tools: vec![
                GATEWAY_LIST_SERVERS.into(),
                GATEWAY_RELOAD.into(),
                GATEWAY_SET_ENABLED.into(),
            ],
        };
        verify_probe_status(&valid, &expected).unwrap();

        let mut failed = valid;
        failed.servers[0].running = false;
        failed.servers[0].error = Some("child failed".into());
        assert!(verify_probe_status(&failed, &expected).is_err());
    }

    #[test]
    fn child_catalog_status_rejects_restart_policy_mismatch() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = write_servers_dir(root.path());
        let expected = GatewayCatalogSnapshot::capture(&dir).unwrap();
        let status = GatewayListResult {
            servers: vec![
                GatewayServerStatus {
                    name: "git".into(),
                    enabled: true,
                    running: true,
                    restart_policy: GatewayRestartPolicy::Never,
                    error: None,
                },
                GatewayServerStatus {
                    name: "disabled".into(),
                    enabled: false,
                    running: false,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
            ],
            exposed_child_tools: 2,
            gateway_tools: vec![
                GATEWAY_LIST_SERVERS.into(),
                GATEWAY_RELOAD.into(),
                GATEWAY_SET_ENABLED.into(),
            ],
        };
        assert!(verify_probe_status(&status, &expected).is_err());
    }

    #[test]
    fn selective_reload_summary_must_match_healthy_catalog() {
        let status = GatewayListResult {
            servers: vec![GatewayServerStatus {
                name: "git".into(),
                enabled: true,
                running: true,
                restart_policy: GatewayRestartPolicy::OnFailure,
                error: None,
            }],
            exposed_child_tools: 2,
            gateway_tools: vec![],
        };
        let summary = GatewayReloadResult {
            configured_servers: 1,
            enabled_servers: 1,
            running_servers: 1,
            exposed_child_tools: 2,
            failed_servers: vec![],
        };
        verify_reload_summary(&summary, &status).unwrap();

        let mut failed = summary;
        failed.failed_servers.push("git".into());
        assert!(verify_reload_summary(&failed, &status).is_err());
    }

    #[test]
    fn gateway_catalog_rejects_launcher_outside_trusted_bin_root() {
        let root = tempfile::TempDir::new().unwrap();
        let catalog = ComponentCatalog::new(
            HostRuntimeRoots::new(root.path().join("bin"), root.path().join("runtime")).unwrap(),
        );
        fs::create_dir_all(catalog.bin_root()).unwrap();
        fs::create_dir_all(catalog.runtime_root().join("gateway/servers.d")).unwrap();
        let launcher = catalog.bin_root().join("sonarqube-mcp");
        fs::write(&launcher, b"trusted launcher").unwrap();
        let config = catalog
            .runtime_root()
            .join("gateway/servers.d/sonarqube.yaml");
        fs::write(
            &config,
            format!(
                "name: sonarqube\nenabled: false\ncommand: {}\nargs: []\n",
                launcher.display()
            ),
        )
        .unwrap();
        let snapshot = GatewayCatalogSnapshot::capture(config.parent().unwrap()).unwrap();
        snapshot.validate_managed_paths(&catalog).unwrap();

        let untrusted = root.path().join("untrusted-launcher");
        fs::write(&untrusted, b"untrusted launcher").unwrap();
        fs::write(
            &config,
            format!(
                "name: sonarqube\nenabled: false\ncommand: {}\nargs: []\n",
                untrusted.display()
            ),
        )
        .unwrap();
        let snapshot = GatewayCatalogSnapshot::capture(config.parent().unwrap()).unwrap();
        assert!(snapshot.validate_managed_paths(&catalog).is_err());
    }

    #[test]
    fn independent_catalog_policy_applies_allowlist_before_prefix_and_rejects_unknown() {
        let config = GatewayChildConfigProjection {
            name: "child".into(),
            enabled: true,
            command: "/trusted/bin/child".into(),
            args: vec![],
            env: BTreeMap::new(),
            tool_prefix: Some("p_".into()),
            tool_allowlist: vec!["alpha".into()],
            timeout_ms: 30_000,
            restart: GatewayRestartConfigProjection::default(),
        };
        assert_eq!(
            apply_child_exposure_policy(&config, vec!["alpha".into(), "beta".into()]).unwrap(),
            vec!["p_alpha".to_owned()]
        );
        let mut invalid = config;
        invalid.tool_allowlist = vec!["missing".into()];
        assert!(
            apply_child_exposure_policy(&invalid, vec!["alpha".into(), "beta".into()]).is_err()
        );
    }

    #[test]
    fn exact_catalog_comparison_detects_missing_extra_duplicate_and_equal_count_substitution() {
        let expected = vec![
            GATEWAY_LIST_SERVERS.into(),
            GATEWAY_RELOAD.into(),
            GATEWAY_SET_ENABLED.into(),
            "child_a".into(),
            "child_b".into(),
        ];
        assert!(compare_gateway_tool_sets(&expected, &expected).is_ok());

        let mut missing = expected.clone();
        missing.pop();
        assert!(compare_gateway_tool_sets(&missing, &expected).is_err());

        let mut extra = expected.clone();
        extra.push("child_extra".into());
        assert!(compare_gateway_tool_sets(&extra, &expected).is_err());

        let mut duplicate = expected.clone();
        duplicate.push("child_a".into());
        assert!(compare_gateway_tool_sets(&duplicate, &expected).is_err());

        let mut substituted = expected.clone();
        *substituted.last_mut().unwrap() = "wrong_same_count".into();
        assert!(compare_gateway_tool_sets(&substituted, &expected).is_err());
    }

    #[test]
    fn expected_catalog_rejects_prefix_collisions_and_ignores_disabled_children_for_probe() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = write_servers_dir(root.path());
        let snapshot = GatewayCatalogSnapshot::capture(&dir).unwrap();
        assert_eq!(
            snapshot
                .children
                .values()
                .filter(|child| child.enabled)
                .count(),
            1
        );

        let one = GatewayChildConfigProjection {
            name: "one".into(),
            enabled: true,
            command: "/trusted/bin/one".into(),
            args: vec![],
            env: BTreeMap::new(),
            tool_prefix: Some("same_".into()),
            tool_allowlist: vec![],
            timeout_ms: 30_000,
            restart: GatewayRestartConfigProjection::default(),
        };
        let two = GatewayChildConfigProjection {
            name: "two".into(),
            command: "/trusted/bin/two".into(),
            ..one.clone()
        };
        let first = apply_child_exposure_policy(&one, vec!["tool".into()]).unwrap();
        let second = apply_child_exposure_policy(&two, vec!["tool".into()]).unwrap();
        let mut aggregate = BTreeSet::new();
        assert!(aggregate.insert(first[0].clone()));
        assert!(!aggregate.insert(second[0].clone()));
    }

    #[test]
    fn audit_gateway_equal_count_wrong_names_must_fail() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = write_servers_dir(root.path());
        let expected = GatewayCatalogSnapshot::capture(&dir).unwrap();
        let status = GatewayListResult {
            servers: vec![
                GatewayServerStatus {
                    name: "git".into(),
                    enabled: true,
                    running: true,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
                GatewayServerStatus {
                    name: "disabled".into(),
                    enabled: false,
                    running: false,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
            ],
            exposed_child_tools: 2,
            gateway_tools: vec![
                GATEWAY_LIST_SERVERS.into(),
                GATEWAY_RELOAD.into(),
                GATEWAY_SET_ENABLED.into(),
            ],
        };
        let wrong_same_count = vec![
            GATEWAY_LIST_SERVERS.into(),
            GATEWAY_RELOAD.into(),
            GATEWAY_SET_ENABLED.into(),
            "wrong_child_alpha".into(),
            "wrong_child_beta".into(),
        ];
        assert!(
            verify_gateway_catalog_shape(
                &status,
                &expected,
                &wrong_same_count,
                &[
                    GATEWAY_LIST_SERVERS.into(),
                    GATEWAY_RELOAD.into(),
                    GATEWAY_SET_ENABLED.into(),
                    "child_a".into(),
                    "child_b".into(),
                ],
            )
            .is_err(),
            "same-count wrong-name Gateway catalog was accepted"
        );
    }

    #[test]
    fn audit_gateway_valid_catalog_shape_still_passes() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = write_servers_dir(root.path());
        let expected = GatewayCatalogSnapshot::capture(&dir).unwrap();
        let status = GatewayListResult {
            servers: vec![
                GatewayServerStatus {
                    name: "git".into(),
                    enabled: true,
                    running: true,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
                GatewayServerStatus {
                    name: "disabled".into(),
                    enabled: false,
                    running: false,
                    restart_policy: GatewayRestartPolicy::OnFailure,
                    error: None,
                },
            ],
            exposed_child_tools: 2,
            gateway_tools: vec![
                GATEWAY_LIST_SERVERS.into(),
                GATEWAY_RELOAD.into(),
                GATEWAY_SET_ENABLED.into(),
            ],
        };
        let plausible = vec![
            GATEWAY_LIST_SERVERS.into(),
            GATEWAY_RELOAD.into(),
            GATEWAY_SET_ENABLED.into(),
            "child_a".into(),
            "child_b".into(),
        ];
        verify_gateway_catalog_shape(&status, &expected, &plausible, &plausible).unwrap();
    }

    #[tokio::test]
    #[ignore = "explicit Aira real Gateway protocol/catalog probe using temporary control path"]
    async fn live_safe_gateway_protocol_probe_smoke() {
        let source = PathBuf::from("../bin/rust-mcp-gateway");
        let source_config = PathBuf::from("../runtime/gateway/servers.d");
        assert!(source.is_file());
        assert!(source_config.is_dir());
        let version = Version::parse("0.1.0").unwrap();
        let expected = GatewayCatalogSnapshot::capture(&source_config).unwrap();
        let summary = ProcessGatewayProbe
            .verify(&source, &source_config, &expected, &version)
            .await
            .unwrap();
        assert_eq!(summary.configured_servers, expected.configured_count());
        assert_eq!(summary.enabled_servers, expected.enabled_count());
        assert_eq!(summary.running_servers, expected.enabled_count());
        assert!(summary.exposed_child_tools > 0);
    }
}
