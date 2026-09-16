use std::{
    collections::BTreeMap,
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{StudioError, StudioResult},
    tunnel::TunnelConfig,
};

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
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
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
        for (key, reference) in &self.tunnel.env {
            if key.trim().is_empty() {
                return Err(StudioError::Config(
                    "tunnel environment variable name must not be empty".into(),
                ));
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
}
