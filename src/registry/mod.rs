use std::collections::BTreeMap;

use crate::config::McpServerConfig;

#[derive(Debug, Clone)]
pub struct Registry {
    servers: BTreeMap<String, McpServerConfig>,
}

impl Registry {
    pub fn new(servers: BTreeMap<String, McpServerConfig>) -> Self {
        Self { servers }
    }

    pub fn get(&self, id: &str) -> Option<&McpServerConfig> {
        self.servers.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.servers.keys()
    }
}
