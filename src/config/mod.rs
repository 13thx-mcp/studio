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
