use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    config::{McpServerConfig, RegistryConfig},
    error::{StudioError, StudioResult},
};

pub const REGISTRY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Rust,
    Node,
    Python,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredMcp {
    pub name: String,
    pub enabled: bool,
    pub runtime: RuntimeKind,
    pub project_path: PathBuf,
    pub executable: PathBuf,
    pub working_dir: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryDocument {
    pub schema_version: u32,
    pub mcp_root: PathBuf,
    #[serde(default)]
    pub servers: BTreeMap<String, RegisteredMcp>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryEntryView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub runtime: RuntimeKind,
    pub project_path: PathBuf,
    pub executable: PathBuf,
    pub working_dir: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryUpdate {
    pub name: String,
    pub executable: PathBuf,
    pub working_dir: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone)]
pub struct Registry {
    inner: Arc<RwLock<RegistryDocument>>,
    mutation: Arc<Mutex<()>>,
    path: Arc<PathBuf>,
    canonical_root: Arc<PathBuf>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("path", &self.path)
            .field("canonical_root", &self.canonical_root)
            .finish_non_exhaustive()
    }
}

impl Registry {
    pub fn open(
        base_dir: &Path,
        config: &RegistryConfig,
        bootstrap: &BTreeMap<String, McpServerConfig>,
    ) -> StudioResult<Self> {
        let root_path = resolve_path(base_dir, &config.mcp_root);
        let canonical_root = fs::canonicalize(&root_path).map_err(|error| {
            StudioError::Config(format!(
                "failed to resolve registry.mcp_root {}: {error}",
                root_path.display()
            ))
        })?;
        if !canonical_root.is_dir() {
            return Err(StudioError::Config(format!(
                "registry.mcp_root is not a directory: {}",
                canonical_root.display()
            )));
        }

        let registry_path = resolve_path(base_dir, &config.path);
        let document = if registry_path.exists() {
            let text = fs::read_to_string(&registry_path)?;
            let document: RegistryDocument = toml::from_str(&text)?;
            validate_document(&document, &canonical_root)?;
            document
        } else {
            let document = bootstrap_document(&canonical_root, base_dir, bootstrap)?;
            persist_document(&registry_path, &document)?;
            document
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(document)),
            mutation: Arc::new(Mutex::new(())),
            path: Arc::new(registry_path),
            canonical_root: Arc::new(canonical_root),
        })
    }

    #[cfg(test)]
    pub fn in_memory(
        root: PathBuf,
        servers: BTreeMap<String, RegisteredMcp>,
    ) -> StudioResult<Self> {
        let canonical_root = fs::canonicalize(root)?;
        let document = RegistryDocument {
            schema_version: REGISTRY_SCHEMA_VERSION,
            mcp_root: PathBuf::from("."),
            servers,
        };
        validate_document(&document, &canonical_root)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(document)),
            mutation: Arc::new(Mutex::new(())),
            path: Arc::new(PathBuf::new()),
            canonical_root: Arc::new(canonical_root),
        })
    }

    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub fn list(&self) -> Vec<RegistryEntryView> {
        let state = self.inner.read().expect("registry lock poisoned");
        state
            .servers
            .iter()
            .map(|(id, server)| public_view(id, server))
            .collect()
    }

    pub fn ids(&self) -> Vec<String> {
        let state = self.inner.read().expect("registry lock poisoned");
        state.servers.keys().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<RegisteredMcp> {
        let state = self.inner.read().expect("registry lock poisoned");
        state.servers.get(id).cloned()
    }

    pub fn get_view(&self, id: &str) -> Option<RegistryEntryView> {
        self.get(id).map(|server| public_view(id, &server))
    }

    pub fn contains_project(&self, project_path: &Path) -> bool {
        let state = self.inner.read().expect("registry lock poisoned");
        state
            .servers
            .values()
            .any(|server| server.project_path == project_path)
    }

    pub fn register(&self, id: String, server: RegisteredMcp) -> StudioResult<RegistryEntryView> {
        let _mutation = self
            .mutation
            .lock()
            .expect("registry mutation lock poisoned");
        validate_id(&id)?;
        let mut candidate = self.snapshot();
        if candidate.servers.contains_key(&id) {
            return Err(StudioError::Duplicate(id));
        }
        validate_server(&server, &self.canonical_root, false)?;
        candidate.servers.insert(id.clone(), server.clone());
        self.persist_and_replace(candidate)?;
        Ok(public_view(&id, &server))
    }

    pub fn update(&self, id: &str, update: RegistryUpdate) -> StudioResult<RegistryEntryView> {
        let _mutation = self
            .mutation
            .lock()
            .expect("registry mutation lock poisoned");
        let mut candidate = self.snapshot();
        let server = candidate
            .servers
            .get_mut(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        server.name = update.name;
        server.executable = update.executable;
        server.working_dir = update.working_dir;
        server.args = update.args;
        validate_server(server, &self.canonical_root, false)?;
        let view = public_view(id, server);
        self.persist_and_replace(candidate)?;
        Ok(view)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> StudioResult<RegistryEntryView> {
        let _mutation = self
            .mutation
            .lock()
            .expect("registry mutation lock poisoned");
        let mut candidate = self.snapshot();
        let server = candidate
            .servers
            .get_mut(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        server.enabled = enabled;
        let view = public_view(id, server);
        self.persist_and_replace(candidate)?;
        Ok(view)
    }

    pub fn unregister(&self, id: &str) -> StudioResult<RegistryEntryView> {
        let _mutation = self
            .mutation
            .lock()
            .expect("registry mutation lock poisoned");
        let mut candidate = self.snapshot();
        let server = candidate
            .servers
            .remove(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let view = public_view(id, &server);
        self.persist_and_replace(candidate)?;
        Ok(view)
    }

    pub fn validate_for_spawn(&self, id: &str) -> StudioResult<(RegisteredMcp, PathBuf, PathBuf)> {
        let server = self
            .get(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        if !server.enabled {
            return Err(StudioError::Disabled(id.to_owned()));
        }
        validate_server(&server, &self.canonical_root, true)?;
        let project = resolve_project(&self.canonical_root, &server.project_path)?;
        let working_dir = resolve_existing_child(&project, &server.working_dir, true)?;
        let executable = resolve_existing_child(&project, &server.executable, false)?;
        Ok((server, working_dir, executable))
    }

    fn snapshot(&self) -> RegistryDocument {
        self.inner.read().expect("registry lock poisoned").clone()
    }

    fn persist_and_replace(&self, candidate: RegistryDocument) -> StudioResult<()> {
        validate_document(&candidate, &self.canonical_root)?;
        if !self.path.as_os_str().is_empty() {
            persist_document(&self.path, &candidate)?;
        }
        *self.inner.write().expect("registry lock poisoned") = candidate;
        Ok(())
    }
}

fn public_view(id: &str, server: &RegisteredMcp) -> RegistryEntryView {
    RegistryEntryView {
        id: id.to_owned(),
        name: server.name.clone(),
        enabled: server.enabled,
        runtime: server.runtime.clone(),
        project_path: server.project_path.clone(),
        executable: server.executable.clone(),
        working_dir: server.working_dir.clone(),
        args: server.args.clone(),
    }
}

fn validate_document(document: &RegistryDocument, canonical_root: &Path) -> StudioResult<()> {
    if document.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(StudioError::Config(format!(
            "unsupported registry schema version {}; expected {}",
            document.schema_version, REGISTRY_SCHEMA_VERSION
        )));
    }
    for (id, server) in &document.servers {
        validate_id(id)?;
        validate_server(server, canonical_root, false)?;
    }
    Ok(())
}

fn validate_id(id: &str) -> StudioResult<()> {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return Err(StudioError::Config("MCP id must not be empty".into()));
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(StudioError::Config(format!("invalid MCP id: {id}")));
    }
    if !chars.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '-'
    }) {
        return Err(StudioError::Config(format!("invalid MCP id: {id}")));
    }
    Ok(())
}

fn validate_server(
    server: &RegisteredMcp,
    canonical_root: &Path,
    require_executable: bool,
) -> StudioResult<()> {
    if server.name.trim().is_empty() {
        return Err(StudioError::Config(
            "MCP display name must not be empty".into(),
        ));
    }
    validate_relative_path(&server.project_path, "project_path")?;
    validate_relative_path(&server.executable, "executable")?;
    validate_relative_path(&server.working_dir, "working_dir")?;

    let project = resolve_project(canonical_root, &server.project_path)?;
    let _ = resolve_existing_child(&project, &server.working_dir, true)?;

    let executable_lexical = project.join(&server.executable);
    ensure_no_symlink_components(&project, &server.executable)?;
    if require_executable || executable_lexical.exists() {
        let executable = fs::canonicalize(&executable_lexical).map_err(|error| {
            StudioError::Config(format!(
                "invalid executable {}: {error}",
                server.executable.display()
            ))
        })?;
        if !executable.starts_with(&project) || !executable.is_file() {
            return Err(StudioError::Config(format!(
                "executable must be a regular file within the registered project: {}",
                server.executable.display()
            )));
        }
    }
    Ok(())
}

fn resolve_project(root: &Path, relative: &Path) -> StudioResult<PathBuf> {
    validate_relative_path(relative, "project_path")?;
    ensure_no_symlink_components(root, relative)?;
    let canonical = fs::canonicalize(root.join(relative)).map_err(|error| {
        StudioError::Config(format!(
            "invalid project path {}: {error}",
            relative.display()
        ))
    })?;
    if !canonical.starts_with(root) || !canonical.is_dir() {
        return Err(StudioError::Config(format!(
            "project path escapes MCP root or is not a directory: {}",
            relative.display()
        )));
    }
    Ok(canonical)
}

fn resolve_existing_child(
    project: &Path,
    relative: &Path,
    directory: bool,
) -> StudioResult<PathBuf> {
    validate_relative_path(relative, "project-relative path")?;
    ensure_no_symlink_components(project, relative)?;
    let canonical = fs::canonicalize(project.join(relative)).map_err(|error| {
        StudioError::Config(format!("invalid path {}: {error}", relative.display()))
    })?;
    if !canonical.starts_with(project) {
        return Err(StudioError::Config(format!(
            "path escapes registered project: {}",
            relative.display()
        )));
    }
    if directory && !canonical.is_dir() {
        return Err(StudioError::Config(format!(
            "working directory is not a directory: {}",
            relative.display()
        )));
    }
    if !directory && !canonical.is_file() {
        return Err(StudioError::Config(format!(
            "executable is not a regular file: {}",
            relative.display()
        )));
    }
    Ok(canonical)
}

fn validate_relative_path(path: &Path, label: &str) -> StudioResult<()> {
    if path.as_os_str().is_empty() {
        return Err(StudioError::Config(format!("{label} must not be empty")));
    }
    if path.is_absolute() {
        return Err(StudioError::Config(format!("{label} must be relative")));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(StudioError::Config(format!(
                "{label} contains forbidden traversal/root component: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn ensure_no_symlink_components(base: &Path, relative: &Path) -> StudioResult<()> {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(segment) => current.push(segment),
            _ => continue,
        }
        if current.exists() {
            let metadata = fs::symlink_metadata(&current)?;
            if metadata.file_type().is_symlink() {
                return Err(StudioError::Config(format!(
                    "symlink components are not allowed: {}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

fn bootstrap_document(
    canonical_root: &Path,
    base_dir: &Path,
    bootstrap: &BTreeMap<String, McpServerConfig>,
) -> StudioResult<RegistryDocument> {
    let mut servers = BTreeMap::new();
    for (id, legacy) in bootstrap {
        validate_id(id)?;
        let working_absolute = resolve_path(base_dir, &legacy.working_dir);
        let working_canonical = fs::canonicalize(&working_absolute).map_err(|error| {
            StudioError::Config(format!(
                "MCP {id}: failed to resolve working directory {}: {error}",
                working_absolute.display()
            ))
        })?;
        if !working_canonical.starts_with(canonical_root) {
            return Err(StudioError::Config(format!(
                "MCP {id}: working directory escapes configured MCP root"
            )));
        }
        let project_path = working_canonical
            .strip_prefix(canonical_root)
            .map_err(|_| StudioError::Config(format!("MCP {id}: invalid project path")))?
            .to_path_buf();
        let project_path = if project_path.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            project_path
        };

        let command_absolute = resolve_path(base_dir, &legacy.command);
        let executable = if let Ok(canonical) = fs::canonicalize(&command_absolute) {
            canonical
                .strip_prefix(&working_canonical)
                .map_err(|_| {
                    StudioError::Config(format!(
                        "MCP {id}: legacy executable must be within its working directory"
                    ))
                })?
                .to_path_buf()
        } else {
            command_absolute
                .strip_prefix(&working_canonical)
                .map_err(|_| {
                    StudioError::Config(format!(
                        "MCP {id}: legacy executable must be within its working directory"
                    ))
                })?
                .to_path_buf()
        };

        let server = RegisteredMcp {
            name: legacy.name.clone(),
            enabled: true,
            runtime: RuntimeKind::Rust,
            project_path,
            executable,
            working_dir: PathBuf::from("."),
            args: legacy.args.clone(),
            env: legacy.env.clone(),
        };
        validate_server(&server, canonical_root, false)?;
        servers.insert(id.clone(), server);
    }
    Ok(RegistryDocument {
        schema_version: REGISTRY_SCHEMA_VERSION,
        mcp_root: PathBuf::from("."),
        servers,
    })
}

fn persist_document(path: &Path, document: &RegistryDocument) -> StudioResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let serialized = toml::to_string_pretty(document)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("registry.toml");
    let temp_path = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), stamp));

    let result = (|| -> StudioResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp_path, path)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn resolve_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mcp-studio-registry-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn server(project: &str) -> RegisteredMcp {
        RegisteredMcp {
            name: project.to_owned(),
            enabled: true,
            runtime: RuntimeKind::Rust,
            project_path: PathBuf::from(project),
            executable: PathBuf::from("target/debug/server"),
            working_dir: PathBuf::from("."),
            args: vec![],
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn rejects_traversal() {
        assert!(validate_relative_path(Path::new("../escape"), "test").is_err());
    }

    #[test]
    fn public_view_omits_environment() {
        let mut value = server("fixture");
        value.env.insert("SECRET".into(), "value".into());
        let json = serde_json::to_string(&public_view("fixture", &value)).unwrap();
        assert!(!json.contains("SECRET"));
        assert!(!json.contains("value"));
    }

    #[test]
    fn persistent_registry_round_trip_is_deterministic() {
        let root = temp_root("round-trip");
        let project = root.join("fixture");
        fs::create_dir_all(&project).unwrap();
        let path = root.join("registry.toml");
        let document = RegistryDocument {
            schema_version: REGISTRY_SCHEMA_VERSION,
            mcp_root: PathBuf::from("."),
            servers: BTreeMap::from([("fixture".into(), server("fixture"))]),
        };
        persist_document(&path, &document).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        persist_document(&path, &document).unwrap();
        let second = fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn allows_multiple_servers_in_same_runtime_directory() {
        let root = temp_root("shared-runtime");
        let project = root.join("fixture");
        fs::create_dir_all(&project).unwrap();
        let document = RegistryDocument {
            schema_version: REGISTRY_SCHEMA_VERSION,
            mcp_root: PathBuf::from("."),
            servers: BTreeMap::from([
                ("one".into(), server("fixture")),
                ("two".into(), server("fixture")),
            ]),
        };
        validate_document(&document, &fs::canonicalize(&root).unwrap()).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_concurrent_mutations_preserve_both_changes() {
        use std::sync::{Arc as StdArc, Barrier};
        use std::thread;

        let root = temp_root("concurrent-mutations");
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();

        let registry_path = root.join("registry.toml");
        let document = RegistryDocument {
            schema_version: REGISTRY_SCHEMA_VERSION,
            mcp_root: PathBuf::from("."),
            servers: BTreeMap::new(),
        };
        persist_document(&registry_path, &document).unwrap();

        let registry = Registry {
            inner: Arc::new(RwLock::new(document)),
            mutation: Arc::new(Mutex::new(())),
            path: Arc::new(registry_path.clone()),
            canonical_root: Arc::new(fs::canonicalize(&root).unwrap()),
        };
        let barrier = StdArc::new(Barrier::new(3));

        let one = {
            let registry = registry.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                registry.register("one".into(), server("one")).unwrap();
            })
        };
        let two = {
            let registry = registry.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                registry.register("two".into(), server("two")).unwrap();
            })
        };
        barrier.wait();
        one.join().unwrap();
        two.join().unwrap();

        let ids = registry.ids();
        assert_eq!(ids, vec!["one".to_owned(), "two".to_owned()]);

        let persisted: RegistryDocument =
            toml::from_str(&fs::read_to_string(&registry_path).unwrap()).unwrap();
        assert_eq!(
            persisted.servers.keys().cloned().collect::<Vec<_>>(),
            vec!["one".to_owned(), "two".to_owned()]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unsupported_schema_version() {
        let root = temp_root("schema");
        fs::create_dir_all(root.join("fixture")).unwrap();
        let document = RegistryDocument {
            schema_version: 99,
            mcp_root: PathBuf::from("."),
            servers: BTreeMap::new(),
        };
        assert!(validate_document(&document, &fs::canonicalize(&root).unwrap()).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
