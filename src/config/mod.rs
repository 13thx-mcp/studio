use std::{fs, net::IpAddr, path::Path};

use serde::{Deserialize, Serialize};

use crate::error::{StudioError, StudioResult};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StudioConfig {
    #[serde(default)]
    pub server: ServerConfig,
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

fn default_listen_addr() -> String {
    "127.0.0.1:18100".to_owned()
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
                "Milestone 0 permits loopback bind addresses only; remote access requires an explicit security design".into(),
            ));
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
        };
        assert!(config.validate().is_err());
    }
}
