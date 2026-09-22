use std::{
    collections::BTreeMap,
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    error::{StudioError, StudioResult},
    tunnel::TunnelConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfigIdentity {
    pub canonical_path: Option<PathBuf>,
    pub sha256: Option<String>,
}

impl LoadedConfigIdentity {
    pub fn builtin() -> Self {
        Self {
            canonical_path: None,
            sha256: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default = "default_log_capacity")]
    pub log_capacity: usize,
    #[serde(default = "default_stop_timeout_ms")]
    pub stop_timeout_ms: u64,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub mcp: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    pub tunnel: TunnelConfig,
    #[serde(default)]
    pub updates: UpdatesConfig,
    #[serde(default)]
    pub automation: AutomationConfig,
}

impl Default for StudioConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            log_capacity: default_log_capacity(),
            stop_timeout_ms: default_stop_timeout_ms(),
            registry: RegistryConfig::default(),
            mcp: default_mcp_registry(),
            tunnel: TunnelConfig::default(),
            updates: UpdatesConfig::default(),
            automation: AutomationConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    #[serde(default = "default_registry_path")]
    pub path: PathBuf,
    #[serde(default = "default_mcp_root")]
    pub mcp_root: PathBuf,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            path: default_registry_path(),
            mcp_root: default_mcp_root(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesConfig {
    #[serde(default = "default_source_root")]
    pub source_root: PathBuf,
    #[serde(default = "default_bin_root")]
    pub bin_root: PathBuf,
    #[serde(default = "default_runtime_root")]
    pub runtime_root: PathBuf,
    #[serde(default)]
    pub desired: BTreeMap<String, String>,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            source_root: default_source_root(),
            bin_root: default_bin_root(),
            runtime_root: default_runtime_root(),
            desired: BTreeMap::new(),
        }
    }
}

impl UpdatesConfig {
    pub fn resolve_roots(&self, base_dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        if self.source_root == default_source_root()
            && self.bin_root == default_bin_root()
            && self.runtime_root == default_runtime_root()
        {
            for candidate in base_dir.ancestors().take(4) {
                if candidate.join("bin").is_dir() && candidate.join("runtime").is_dir() {
                    return (
                        candidate.to_owned(),
                        candidate.join("bin"),
                        candidate.join("runtime"),
                    );
                }
            }
        }
        (
            resolve_relative(base_dir, &self.source_root),
            resolve_relative(base_dir, &self.bin_root),
            resolve_relative(base_dir, &self.runtime_root),
        )
    }
}

fn resolve_relative(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base_dir.join(path)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutomationPolicy {
    #[default]
    Manual,
    NotifyOnly,
    AutoPrepare,
    AutoUpdateSafe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReconciliationAutomationPolicy {
    #[default]
    Manual,
    NotifyOnly,
    AutoReconcileSafe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartAutomationPolicy {
    #[default]
    Disabled,
    OnFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekdayUtc {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl WeekdayUtc {
    const fn monday_index(self) -> u64 {
        match self {
            Self::Mon => 0,
            Self::Tue => 1,
            Self::Wed => 2,
            Self::Thu => 3,
            Self::Fri => 4,
            Self::Sat => 5,
            Self::Sun => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAutomationConfig {
    #[serde(default)]
    pub policy: AutomationPolicy,
    #[serde(default)]
    pub check_on_startup: bool,
    #[serde(default = "default_update_check_interval_seconds")]
    pub check_interval_seconds: u64,
    #[serde(default = "default_automation_failure_threshold")]
    pub max_consecutive_failures: u32,
    #[serde(default = "default_automation_circuit_cooldown_seconds")]
    pub circuit_cooldown_seconds: u64,
    #[serde(default = "default_prepared_ttl_seconds")]
    pub prepared_ttl_seconds: u64,
    #[serde(default = "default_max_staging_bytes")]
    pub max_staging_bytes: u64,
}

impl Default for UpdateAutomationConfig {
    fn default() -> Self {
        Self {
            policy: AutomationPolicy::Manual,
            check_on_startup: false,
            check_interval_seconds: default_update_check_interval_seconds(),
            max_consecutive_failures: default_automation_failure_threshold(),
            circuit_cooldown_seconds: default_automation_circuit_cooldown_seconds(),
            prepared_ttl_seconds: default_prepared_ttl_seconds(),
            max_staging_bytes: default_max_staging_bytes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationAutomationConfig {
    #[serde(default)]
    pub policy: ReconciliationAutomationPolicy,
    #[serde(default = "default_true")]
    pub check_on_startup: bool,
    #[serde(default = "default_reconciliation_check_interval_seconds")]
    pub check_interval_seconds: u64,
    #[serde(default = "default_automation_failure_threshold")]
    pub max_consecutive_failures: u32,
    #[serde(default = "default_automation_circuit_cooldown_seconds")]
    pub circuit_cooldown_seconds: u64,
}

impl Default for ReconciliationAutomationConfig {
    fn default() -> Self {
        Self {
            policy: ReconciliationAutomationPolicy::Manual,
            check_on_startup: true,
            check_interval_seconds: default_reconciliation_check_interval_seconds(),
            max_consecutive_failures: default_automation_failure_threshold(),
            circuit_cooldown_seconds: default_automation_circuit_cooldown_seconds(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartAutomationConfig {
    #[serde(default)]
    pub mcp: RestartAutomationPolicy,
    #[serde(default)]
    pub tunnel: RestartAutomationPolicy,
    #[serde(default = "default_restart_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_restart_stability_window_seconds")]
    pub stability_window_seconds: u64,
    #[serde(default = "default_restart_initial_backoff_seconds")]
    pub initial_backoff_seconds: u64,
    #[serde(default = "default_restart_max_backoff_seconds")]
    pub max_backoff_seconds: u64,
    #[serde(default = "default_restart_circuit_cooldown_seconds")]
    pub circuit_cooldown_seconds: u64,
}

impl Default for RestartAutomationConfig {
    fn default() -> Self {
        Self {
            mcp: RestartAutomationPolicy::Disabled,
            tunnel: RestartAutomationPolicy::Disabled,
            max_attempts: default_restart_max_attempts(),
            stability_window_seconds: default_restart_stability_window_seconds(),
            initial_backoff_seconds: default_restart_initial_backoff_seconds(),
            max_backoff_seconds: default_restart_max_backoff_seconds(),
            circuit_cooldown_seconds: default_restart_circuit_cooldown_seconds(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceWindowConfig {
    #[serde(default = "default_maintenance_weekdays")]
    pub weekdays_utc: Vec<WeekdayUtc>,
    #[serde(default = "default_maintenance_start_utc")]
    pub start_utc: String,
    #[serde(default = "default_maintenance_duration_minutes")]
    pub duration_minutes: u16,
}

impl Default for MaintenanceWindowConfig {
    fn default() -> Self {
        Self {
            weekdays_utc: default_maintenance_weekdays(),
            start_utc: default_maintenance_start_utc(),
            duration_minutes: default_maintenance_duration_minutes(),
        }
    }
}

impl MaintenanceWindowConfig {
    fn start_minute(&self) -> StudioResult<u64> {
        let bytes = self.start_utc.as_bytes();
        if bytes.len() != 5 || bytes[2] != b':' {
            return Err(StudioError::Config(
                "automation.maintenance.start_utc must be HH:MM UTC".into(),
            ));
        }
        let hour = self.start_utc[..2]
            .parse::<u64>()
            .map_err(|_| StudioError::Config("invalid maintenance UTC hour".into()))?;
        let minute = self.start_utc[3..]
            .parse::<u64>()
            .map_err(|_| StudioError::Config("invalid maintenance UTC minute".into()))?;
        if hour > 23 || minute > 59 {
            return Err(StudioError::Config(
                "automation.maintenance.start_utc is outside 00:00..23:59".into(),
            ));
        }
        Ok(hour * 60 + minute)
    }

    pub fn contains_unix_ms(&self, unix_ms: u64) -> StudioResult<bool> {
        const MINUTES_PER_DAY: u64 = 24 * 60;
        const MINUTES_PER_WEEK: u64 = 7 * MINUTES_PER_DAY;
        let start_minute = self.start_minute()?;
        let unix_minutes = unix_ms / 60_000;
        let unix_days = unix_minutes / MINUTES_PER_DAY;
        let minute_of_day = unix_minutes % MINUTES_PER_DAY;
        // 1970-01-01 was Thursday; Monday = 0.
        let monday_day = (unix_days + 3) % 7;
        let minute_of_week = monday_day * MINUTES_PER_DAY + minute_of_day;
        let duration = u64::from(self.duration_minutes);

        for weekday in &self.weekdays_utc {
            let start = weekday.monday_index() * MINUTES_PER_DAY + start_minute;
            let end = start + duration;
            if end <= MINUTES_PER_WEEK {
                if minute_of_week >= start && minute_of_week < end {
                    return Ok(true);
                }
            } else {
                let wrapped_end = end - MINUTES_PER_WEEK;
                if minute_of_week >= start || minute_of_week < wrapped_end {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationConfig {
    #[serde(default = "default_startup_grace_seconds")]
    pub startup_grace_seconds: u64,
    #[serde(default = "default_health_interval_seconds")]
    pub health_interval_seconds: u64,
    #[serde(default = "default_cleanup_interval_seconds")]
    pub cleanup_interval_seconds: u64,
    #[serde(default)]
    pub updates: UpdateAutomationConfig,
    #[serde(default)]
    pub reconciliation: ReconciliationAutomationConfig,
    #[serde(default)]
    pub restart: RestartAutomationConfig,
    #[serde(default)]
    pub maintenance: MaintenanceWindowConfig,
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self {
            startup_grace_seconds: default_startup_grace_seconds(),
            health_interval_seconds: default_health_interval_seconds(),
            cleanup_interval_seconds: default_cleanup_interval_seconds(),
            updates: UpdateAutomationConfig::default(),
            reconciliation: ReconciliationAutomationConfig::default(),
            restart: RestartAutomationConfig::default(),
            maintenance: MaintenanceWindowConfig::default(),
        }
    }
}

impl AutomationConfig {
    pub fn fingerprint(&self) -> StudioResult<String> {
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn validate(&self) -> StudioResult<()> {
        bounded_u64(
            "automation.startup_grace_seconds",
            self.startup_grace_seconds,
            0,
            3600,
        )?;
        bounded_u64(
            "automation.health_interval_seconds",
            self.health_interval_seconds,
            5,
            3600,
        )?;
        bounded_u64(
            "automation.cleanup_interval_seconds",
            self.cleanup_interval_seconds,
            60,
            86_400,
        )?;

        bounded_u64(
            "automation.updates.check_interval_seconds",
            self.updates.check_interval_seconds,
            300,
            604_800,
        )?;
        bounded_u32(
            "automation.updates.max_consecutive_failures",
            self.updates.max_consecutive_failures,
            1,
            20,
        )?;
        bounded_u64(
            "automation.updates.circuit_cooldown_seconds",
            self.updates.circuit_cooldown_seconds,
            60,
            604_800,
        )?;
        bounded_u64(
            "automation.updates.prepared_ttl_seconds",
            self.updates.prepared_ttl_seconds,
            300,
            604_800,
        )?;
        bounded_u64(
            "automation.updates.max_staging_bytes",
            self.updates.max_staging_bytes,
            64 * 1024 * 1024,
            16 * 1024 * 1024 * 1024,
        )?;

        bounded_u64(
            "automation.reconciliation.check_interval_seconds",
            self.reconciliation.check_interval_seconds,
            30,
            86_400,
        )?;
        bounded_u32(
            "automation.reconciliation.max_consecutive_failures",
            self.reconciliation.max_consecutive_failures,
            1,
            20,
        )?;
        bounded_u64(
            "automation.reconciliation.circuit_cooldown_seconds",
            self.reconciliation.circuit_cooldown_seconds,
            60,
            604_800,
        )?;

        bounded_u32(
            "automation.restart.max_attempts",
            self.restart.max_attempts,
            1,
            20,
        )?;
        bounded_u64(
            "automation.restart.stability_window_seconds",
            self.restart.stability_window_seconds,
            5,
            3600,
        )?;
        bounded_u64(
            "automation.restart.initial_backoff_seconds",
            self.restart.initial_backoff_seconds,
            1,
            300,
        )?;
        bounded_u64(
            "automation.restart.max_backoff_seconds",
            self.restart.max_backoff_seconds,
            self.restart.initial_backoff_seconds,
            3600,
        )?;
        bounded_u64(
            "automation.restart.circuit_cooldown_seconds",
            self.restart.circuit_cooldown_seconds,
            10,
            86_400,
        )?;

        if self.maintenance.weekdays_utc.is_empty() {
            return Err(StudioError::Config(
                "automation.maintenance.weekdays_utc must not be empty".into(),
            ));
        }
        let unique = self
            .maintenance
            .weekdays_utc
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != self.maintenance.weekdays_utc.len() {
            return Err(StudioError::Config(
                "automation.maintenance.weekdays_utc contains duplicates".into(),
            ));
        }
        self.maintenance.start_minute()?;
        if !(1..=1440).contains(&self.maintenance.duration_minutes) {
            return Err(StudioError::Config(
                "automation.maintenance.duration_minutes must be between 1 and 1440".into(),
            ));
        }
        Ok(())
    }
}

fn bounded_u64(name: &str, value: u64, min: u64, max: u64) -> StudioResult<()> {
    if value < min || value > max {
        return Err(StudioError::Config(format!(
            "{name} must be between {min} and {max}"
        )));
    }
    Ok(())
}

fn bounded_u32(name: &str, value: u32, min: u32, max: u32) -> StudioResult<()> {
    if value < min || value > max {
        return Err(StudioError::Config(format!(
            "{name} must be between {min} and {max}"
        )));
    }
    Ok(())
}

fn default_true() -> bool {
    true
}
fn default_startup_grace_seconds() -> u64 {
    30
}
fn default_health_interval_seconds() -> u64 {
    15
}
fn default_cleanup_interval_seconds() -> u64 {
    3600
}
fn default_update_check_interval_seconds() -> u64 {
    21_600
}
fn default_reconciliation_check_interval_seconds() -> u64 {
    300
}
fn default_automation_failure_threshold() -> u32 {
    5
}
fn default_automation_circuit_cooldown_seconds() -> u64 {
    21_600
}
fn default_prepared_ttl_seconds() -> u64 {
    21_600
}
fn default_max_staging_bytes() -> u64 {
    1024 * 1024 * 1024
}
fn default_restart_max_attempts() -> u32 {
    3
}
fn default_restart_stability_window_seconds() -> u64 {
    30
}
fn default_restart_initial_backoff_seconds() -> u64 {
    1
}
fn default_restart_max_backoff_seconds() -> u64 {
    30
}
fn default_restart_circuit_cooldown_seconds() -> u64 {
    60
}
fn default_maintenance_weekdays() -> Vec<WeekdayUtc> {
    vec![WeekdayUtc::Sat, WeekdayUtc::Sun]
}
fn default_maintenance_start_utc() -> String {
    "02:00".into()
}
fn default_maintenance_duration_minutes() -> u16 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

fn default_listen_addr() -> String {
    "127.0.0.1:18100".to_owned()
}
fn default_log_capacity() -> usize {
    500
}
fn default_stop_timeout_ms() -> u64 {
    3_000
}
fn default_registry_path() -> PathBuf {
    PathBuf::from("data/registry.toml")
}
fn default_mcp_root() -> PathBuf {
    PathBuf::from("..")
}
fn default_source_root() -> PathBuf {
    PathBuf::from("..")
}
fn default_bin_root() -> PathBuf {
    PathBuf::from("../bin")
}
fn default_runtime_root() -> PathBuf {
    PathBuf::from("../runtime")
}

fn default_mcp_registry() -> BTreeMap<String, McpServerConfig> {
    BTreeMap::from([
        (
            "blender".to_owned(),
            McpServerConfig {
                name: "Blender".to_owned(),
                command: PathBuf::from("../blender/target/debug/rust-mcp-blender"),
                args: vec![],
                working_dir: PathBuf::from("../blender"),
                env: BTreeMap::new(),
            },
        ),
        (
            "filesystem".to_owned(),
            McpServerConfig {
                name: "Filesystem".to_owned(),
                command: PathBuf::from("../filesystem/target/debug/rust-mcp-filesystem"),
                args: vec!["--root".to_owned(), "..".to_owned()],
                working_dir: PathBuf::from("../filesystem"),
                env: BTreeMap::new(),
            },
        ),
    ])
}

impl StudioConfig {
    pub fn load(path: &Path) -> StudioResult<Self> {
        Ok(Self::load_with_identity(path)?.0)
    }

    pub fn load_with_identity(path: &Path) -> StudioResult<(Self, LoadedConfigIdentity)> {
        let bytes = fs::read(path)?;
        let canonical_path = fs::canonicalize(path)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| StudioError::Config("Studio config is not valid UTF-8".into()))?;
        let config = toml::from_str(text)?;
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok((
            config,
            LoadedConfigIdentity {
                canonical_path: Some(canonical_path),
                sha256: Some(sha256),
            },
        ))
    }

    pub fn validate(&self) -> StudioResult<()> {
        let addr: std::net::SocketAddr = self.server.listen_addr.parse().map_err(|_| {
            StudioError::Config(format!(
                "invalid server.listen_addr: {}",
                self.server.listen_addr
            ))
        })?;
        if !is_loopback(addr.ip()) {
            return Err(StudioError::Config(
                "Studio permits loopback bind addresses only; remote access requires an explicit security design".into(),
            ));
        }
        if self.log_capacity == 0 {
            return Err(StudioError::Config(
                "log_capacity must be greater than zero".into(),
            ));
        }
        if self.stop_timeout_ms == 0 {
            return Err(StudioError::Config(
                "stop_timeout_ms must be greater than zero".into(),
            ));
        }
        if self.registry.path.as_os_str().is_empty() {
            return Err(StudioError::Config(
                "registry.path must not be empty".into(),
            ));
        }
        if self.registry.mcp_root.as_os_str().is_empty() {
            return Err(StudioError::Config(
                "registry.mcp_root must not be empty".into(),
            ));
        }
        for (id, server) in &self.mcp {
            if id.trim().is_empty() {
                return Err(StudioError::Config("MCP id must not be empty".into()));
            }
            if server.name.trim().is_empty() {
                return Err(StudioError::Config(format!(
                    "MCP {id}: name must not be empty"
                )));
            }
            if server.command.as_os_str().is_empty() {
                return Err(StudioError::Config(format!(
                    "MCP {id}: command must not be empty"
                )));
            }
        }
        if self.tunnel.name.trim().is_empty() {
            return Err(StudioError::Config("tunnel.name must not be empty".into()));
        }
        if self.tunnel.runtime.as_os_str().is_empty()
            || self.tunnel.working_dir.as_os_str().is_empty()
            || self.tunnel.config_file.as_os_str().is_empty()
        {
            return Err(StudioError::Config(
                "tunnel runtime, working_dir, and config_file must not be empty".into(),
            ));
        }
        if self
            .tunnel
            .health_url_file
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(StudioError::Config(
                "tunnel.health_url_file must not be empty when configured".into(),
            ));
        }
        if self.updates.source_root.as_os_str().is_empty()
            || self.updates.bin_root.as_os_str().is_empty()
            || self.updates.runtime_root.as_os_str().is_empty()
        {
            return Err(StudioError::Config(
                "updates source_root, bin_root, and runtime_root must not be empty".into(),
            ));
        }
        for (component, version) in &self.updates.desired {
            component.parse::<crate::update::ComponentId>()?;
            crate::update::Version::parse(version)?;
        }
        self.automation.validate()?;

        for (key, reference) in &self.tunnel.env {
            if key.trim().is_empty() {
                return Err(StudioError::Config(
                    "tunnel environment variable name must not be empty".into(),
                ));
            }
            if crate::tunnel::RESERVED_TUNNEL_RUNTIME_ENV.contains(&key.as_str()) {
                return Err(StudioError::Config(format!(
                    "tunnel environment variable {key} is reserved for validated runtime authority"
                )));
            }
            match reference {
                crate::tunnel::SecretReference::FromEnv { from_env }
                    if from_env.trim().is_empty() =>
                {
                    return Err(StudioError::Config(format!(
                        "tunnel secret reference for {key} has an empty from_env"
                    )));
                }
                crate::tunnel::SecretReference::FromFile { from_file }
                    if from_file.as_os_str().is_empty() =>
                {
                    return Err(StudioError::Config(format!(
                        "tunnel secret reference for {key} has an empty from_file"
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_with_identity_hashes_exact_parsed_bytes_and_canonical_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("studio.toml");
        let bytes = b"[server]\nlisten_addr = \"127.0.0.1:18100\"\n";
        fs::write(&path, bytes).unwrap();
        let (config, identity) = StudioConfig::load_with_identity(&path).unwrap();
        assert_eq!(config.server.listen_addr, "127.0.0.1:18100");
        assert_eq!(
            identity.canonical_path,
            Some(fs::canonicalize(&path).unwrap())
        );
        let expected = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(identity.sha256.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn default_config_is_valid() {
        StudioConfig::default().validate().unwrap();
    }

    #[test]
    fn rejects_non_loopback_address() {
        let config = StudioConfig {
            server: ServerConfig {
                listen_addr: "0.0.0.0:18100".into(),
            },
            ..StudioConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_empty_registry_paths() {
        let mut config = StudioConfig::default();
        config.registry.path = PathBuf::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_reserved_tunnel_runtime_environment_authority() {
        for key in crate::tunnel::RESERVED_TUNNEL_RUNTIME_ENV {
            let mut config = StudioConfig::default();
            config.tunnel.env.insert(
                key.into(),
                crate::tunnel::SecretReference::FromEnv {
                    from_env: "SAFE_SECRET_SOURCE".into(),
                },
            );
            let error = config.validate().unwrap_err().to_string();
            assert!(error.contains("reserved for validated runtime authority"));
            assert!(error.contains(key));
        }
    }

    #[test]
    fn validates_update_roots_and_desired_versions() {
        let mut config = StudioConfig::default();
        config.updates.runtime_root = PathBuf::new();
        assert!(config.validate().is_err());

        let mut config = StudioConfig::default();
        config
            .updates
            .desired
            .insert("git".into(), "not-a-version".into());
        assert!(config.validate().is_err());

        let mut config = StudioConfig::default();
        config
            .updates
            .desired
            .insert("unknown".into(), "1.0.0".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn automation_defaults_are_manual_and_disabled() {
        let automation = AutomationConfig::default();
        assert_eq!(automation.updates.policy, AutomationPolicy::Manual);
        assert_eq!(
            automation.reconciliation.policy,
            ReconciliationAutomationPolicy::Manual
        );
        assert_eq!(automation.restart.mcp, RestartAutomationPolicy::Disabled);
        assert_eq!(automation.restart.tunnel, RestartAutomationPolicy::Disabled);
        automation.validate().unwrap();
    }

    #[test]
    fn automation_config_rejects_unsafe_bounds_and_invalid_window() {
        let mut automation = AutomationConfig::default();
        automation.updates.check_interval_seconds = 1;
        assert!(automation.validate().is_err());

        let mut automation = AutomationConfig::default();
        automation.restart.max_backoff_seconds = 0;
        assert!(automation.validate().is_err());

        let mut automation = AutomationConfig::default();
        automation.maintenance.start_utc = "25:00".into();
        assert!(automation.validate().is_err());

        let mut automation = AutomationConfig::default();
        automation.maintenance.weekdays_utc = vec![WeekdayUtc::Sat, WeekdayUtc::Sat];
        assert!(automation.validate().is_err());
    }

    #[test]
    fn utc_maintenance_window_is_end_exclusive_and_wraps_week() {
        let window = MaintenanceWindowConfig {
            weekdays_utc: vec![WeekdayUtc::Sun],
            start_utc: "23:30".into(),
            duration_minutes: 120,
        };
        // 1970-01-04 23:30 UTC was Sunday.
        let start = (3 * 24 * 60 + 23 * 60 + 30) * 60_000;
        assert!(window.contains_unix_ms(start).unwrap());
        assert!(window.contains_unix_ms(start + 119 * 60_000).unwrap());
        assert!(!window.contains_unix_ms(start + 120 * 60_000).unwrap());
    }

    #[test]
    fn automation_policy_fingerprint_is_deterministic() {
        let config = AutomationConfig::default();
        assert_eq!(config.fingerprint().unwrap(), config.fingerprint().unwrap());
        let mut changed = config.clone();
        changed.updates.policy = AutomationPolicy::NotifyOnly;
        assert_ne!(
            config.fingerprint().unwrap(),
            changed.fingerprint().unwrap()
        );
    }

    #[test]
    fn missing_automation_table_loads_manual_defaults() {
        let config: StudioConfig = toml::from_str(
            r#"[server]
listen_addr = "127.0.0.1:18100"
"#,
        )
        .unwrap();
        assert_eq!(config.automation, AutomationConfig::default());
        config.validate().unwrap();
    }

    #[test]
    fn default_update_roots_find_conventional_runtime_ancestor() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("bin")).unwrap();
        fs::create_dir_all(root.path().join("runtime/studio")).unwrap();
        let base = root.path().join("runtime/studio");
        let (source, bin, runtime) = UpdatesConfig::default().resolve_roots(&base);
        assert_eq!(source, root.path());
        assert_eq!(bin, root.path().join("bin"));
        assert_eq!(runtime, root.path().join("runtime"));
    }
}
