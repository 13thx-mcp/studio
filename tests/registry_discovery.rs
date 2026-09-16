#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use mcp_studio::{
    config::RegistryConfig,
    discovery::{DiscoveryService, RegisterDiscoveryRequest},
    error::StudioError,
    registry::{Registry, RegistryUpdate},
};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mcp-studio-m4-{name}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn registry_config(root: &Path) -> RegistryConfig {
    RegistryConfig {
        path: root.join("data/registry.toml"),
        mcp_root: root.to_path_buf(),
    }
}

fn make_rust_project(root: &Path, name: &str, package: &str) {
    let project = root.join(name);
    fs::create_dir_all(project.join("target/release")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    let executable = project.join("target/release").join(package);
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(executable, permissions).unwrap();
}

#[test]
fn discovery_registration_and_mutations_survive_restart() {
    let root = TempRoot::new("persistence");
    make_rust_project(&root.0, "sample", "sample-mcp");

    let config = registry_config(&root.0);
    let registry = Registry::open(Path::new("."), &config, &BTreeMap::new()).unwrap();
    let discovery = DiscoveryService::new(registry.clone());
    let candidates = discovery.scan().unwrap();
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.project_path == PathBuf::from("sample"))
        .unwrap();
    assert_eq!(candidate.suggested_id.as_deref(), Some("sample"));
    assert!(!candidate.already_registered);

    let server = discovery
        .approve(
            &candidate.candidate_id,
            RegisterDiscoveryRequest {
                id: "sample".into(),
                name: Some("Sample MCP".into()),
                executable: PathBuf::from("target/release/sample-mcp"),
            },
        )
        .unwrap();
    registry.register("sample".into(), server).unwrap();

    registry
        .update(
            "sample",
            RegistryUpdate {
                name: "Sample MCP Edited".into(),
                executable: PathBuf::from("target/release/sample-mcp"),
                working_dir: PathBuf::from("."),
                args: vec!["--fixture".into()],
            },
        )
        .unwrap();
    registry.set_enabled("sample", false).unwrap();
    drop(discovery);
    drop(registry);

    let reloaded = Registry::open(Path::new("."), &config, &BTreeMap::new()).unwrap();
    let entry = reloaded.get_view("sample").unwrap();
    assert_eq!(entry.name, "Sample MCP Edited");
    assert!(!entry.enabled);
    assert_eq!(entry.args, vec!["--fixture"]);

    reloaded.unregister("sample").unwrap();
    drop(reloaded);
    let reloaded = Registry::open(Path::new("."), &config, &BTreeMap::new()).unwrap();
    assert!(reloaded.get("sample").is_none());
    assert!(root.0.join("sample/Cargo.toml").is_file());
}

#[test]
fn duplicate_project_registration_is_rejected() {
    let root = TempRoot::new("duplicate");
    make_rust_project(&root.0, "sample", "sample-mcp");
    let config = registry_config(&root.0);
    let registry = Registry::open(Path::new("."), &config, &BTreeMap::new()).unwrap();
    let discovery = DiscoveryService::new(registry.clone());
    let candidate = discovery.candidate("sample").unwrap();

    let make = |id: &str| {
        discovery.approve(
            &candidate.candidate_id,
            RegisterDiscoveryRequest {
                id: id.into(),
                name: None,
                executable: PathBuf::from("target/release/sample-mcp"),
            },
        )
    };
    registry.register("sample".into(), make("sample").unwrap()).unwrap();
    let error = make("other").unwrap_err();
    assert!(matches!(error, StudioError::Duplicate(_)));
}

#[test]
fn discovery_ignores_infrastructure_and_hidden_directories() {
    let root = TempRoot::new("ignore");
    make_rust_project(&root.0, "blender", "rust-mcp-blender");
    make_rust_project(&root.0, "studio", "mcp-studio");
    make_rust_project(&root.0, "gateway", "rust-mcp-gateway");
    make_rust_project(&root.0, ".hidden", "hidden");

    let registry = Registry::open(
        Path::new("."),
        &registry_config(&root.0),
        &BTreeMap::new(),
    )
    .unwrap();
    let paths = DiscoveryService::new(registry)
        .scan()
        .unwrap()
        .into_iter()
        .map(|candidate| candidate.project_path)
        .collect::<Vec<_>>();
    assert_eq!(paths, vec![PathBuf::from("blender")]);
}

#[test]
fn executable_traversal_and_symlink_escape_are_rejected() {
    let root = TempRoot::new("escape");
    make_rust_project(&root.0, "sample", "sample-mcp");
    let outside = root.0.join("outside-bin");
    fs::write(&outside, "fixture").unwrap();
    symlink(&outside, root.0.join("sample/linked-bin")).unwrap();

    let registry = Registry::open(
        Path::new("."),
        &registry_config(&root.0),
        &BTreeMap::new(),
    )
    .unwrap();
    let discovery = DiscoveryService::new(registry.clone());
    let candidate = discovery.candidate("sample").unwrap();
    let server = discovery
        .approve(
            &candidate.candidate_id,
            RegisterDiscoveryRequest {
                id: "sample".into(),
                name: None,
                executable: PathBuf::from("target/release/sample-mcp"),
            },
        )
        .unwrap();
    registry.register("sample".into(), server).unwrap();

    let traversal = registry.update(
        "sample",
        RegistryUpdate {
            name: "sample".into(),
            executable: PathBuf::from("../outside-bin"),
            working_dir: PathBuf::from("."),
            args: vec![],
        },
    );
    assert!(traversal.is_err());

    let symlinked = registry.update(
        "sample",
        RegistryUpdate {
            name: "sample".into(),
            executable: PathBuf::from("linked-bin"),
            working_dir: PathBuf::from("."),
            args: vec![],
        },
    );
    assert!(symlinked.is_err());
}
