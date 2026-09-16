use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{StudioError, StudioResult},
    registry::{RegisteredMcp, Registry, RuntimeKind},
};

const IGNORED_NAMES: &[&str] = &[
    "studio",
    "tunnel-client",
    "gateway",
    "mcp-server",
    "target",
    "node_modules",
    "dist",
    "build",
    "tmp",
    "temp",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiscoveredProject {
    pub candidate_id: String,
    pub suggested_id: Option<String>,
    pub name: String,
    pub project_path: PathBuf,
    pub runtime: Option<RuntimeKind>,
    pub manifest: Option<String>,
    pub executable_candidates: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub already_registered: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterDiscoveryRequest {
    pub id: String,
    pub name: Option<String>,
    pub executable: PathBuf,
}

#[derive(Clone)]
pub struct DiscoveryService {
    registry: Registry,
}

impl DiscoveryService {
    pub fn new(registry: Registry) -> Self {
        Self { registry }
    }

    pub fn scan(&self) -> StudioResult<Vec<DiscoveredProject>> {
        let root = self.registry.canonical_root();
        let mut projects = Vec::new();

        for entry in fs::read_dir(root)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(%error, "failed to read discovery entry");
                    continue;
                }
            };

            let name = entry.file_name().to_string_lossy().into_owned();
            if should_ignore(&name) {
                continue;
            }

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    tracing::warn!(entry = %name, %error, "failed to inspect discovery entry");
                    continue;
                }
            };
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }

            let project_path = PathBuf::from(&name);
            let mut candidate = inspect_project(&entry.path(), project_path.clone());

            // Plain directories are not discovery candidates. A malformed supported
            // manifest is still returned so the operator can see the warning.
            if candidate.manifest.is_none() {
                continue;
            }

            candidate.already_registered = self.registry.contains_project(&project_path);
            projects.push(candidate);
        }

        projects.sort_by(|a, b| a.project_path.cmp(&b.project_path));
        Ok(projects)
    }

    pub fn candidate(&self, candidate_id: &str) -> StudioResult<DiscoveredProject> {
        self.scan()?
            .into_iter()
            .find(|candidate| candidate.candidate_id == candidate_id)
            .ok_or_else(|| StudioError::NotFound(candidate_id.to_owned()))
    }

    pub fn approve(
        &self,
        candidate_id: &str,
        request: RegisterDiscoveryRequest,
    ) -> StudioResult<RegisteredMcp> {
        let candidate = self.candidate(candidate_id)?;
        if candidate.already_registered {
            return Err(StudioError::Duplicate(format!(
                "project {} is already registered",
                candidate.project_path.display()
            )));
        }

        let runtime = candidate.runtime.ok_or_else(|| {
            StudioError::Unsupported(format!(
                "{} has no supported project manifest",
                candidate.project_path.display()
            ))
        })?;

        if !candidate
            .executable_candidates
            .contains(&request.executable)
        {
            return Err(StudioError::Config(format!(
                "executable {} was not produced by discovery",
                request.executable.display()
            )));
        }

        Ok(RegisteredMcp {
            name: request.name.unwrap_or(candidate.name),
            enabled: true,
            runtime,
            project_path: candidate.project_path,
            executable: request.executable,
            working_dir: PathBuf::from("."),
            args: vec![],
            env: BTreeMap::new(),
        })
    }
}

fn should_ignore(name: &str) -> bool {
    name.starts_with('.') || IGNORED_NAMES.contains(&name)
}

fn inspect_project(path: &Path, project_path: PathBuf) -> DiscoveredProject {
    let candidate_id = project_path.to_string_lossy().into_owned();
    let suggested_id = sanitize_id(&candidate_id);

    let cargo = path.join("Cargo.toml");
    if cargo.is_file() {
        return inspect_rust(&cargo, project_path, candidate_id, suggested_id);
    }

    let package = path.join("package.json");
    if package.is_file() {
        return inspect_node(&package, project_path, candidate_id, suggested_id);
    }

    let pyproject = path.join("pyproject.toml");
    if pyproject.is_file() {
        return inspect_python(&pyproject, project_path, candidate_id, suggested_id);
    }

    DiscoveredProject {
        candidate_id,
        suggested_id,
        name: project_path.to_string_lossy().into_owned(),
        project_path,
        runtime: None,
        manifest: None,
        executable_candidates: vec![],
        warnings: vec!["no supported manifest found".into()],
        already_registered: false,
    }
}

fn inspect_rust(
    manifest_path: &Path,
    project_path: PathBuf,
    candidate_id: String,
    suggested_id: Option<String>,
) -> DiscoveredProject {
    let value = match read_toml_document(manifest_path) {
        Ok(value) => value,
        Err(error) => {
            return malformed_candidate(
                project_path,
                candidate_id,
                suggested_id,
                "Cargo.toml",
                error,
            );
        }
    };

    let package = value.get("package").and_then(toml::Value::as_table);
    let package_name = package
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    let default_run = package
        .and_then(|table| table.get("default-run"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);

    let mut binaries = Vec::new();
    if let Some(default_run) = default_run {
        binaries.push(default_run);
    }
    if let Some(array) = value.get("bin").and_then(toml::Value::as_array) {
        for item in array {
            if let Some(name) = item
                .as_table()
                .and_then(|table| table.get("name"))
                .and_then(toml::Value::as_str)
                && !binaries.iter().any(|binary| binary == name)
            {
                binaries.push(name.to_owned());
            }
        }
    }
    if binaries.is_empty()
        && let Some(name) = &package_name
    {
        binaries.push(name.clone());
    }

    let warnings = if binaries.is_empty() {
        vec!["Cargo manifest does not identify a package/binary name".into()]
    } else {
        vec![]
    };

    let mut executable_candidates = Vec::new();
    for binary in binaries {
        executable_candidates.push(PathBuf::from("target/release").join(&binary));
        executable_candidates.push(PathBuf::from("target/debug").join(binary));
    }

    DiscoveredProject {
        candidate_id,
        suggested_id,
        name: package_name.unwrap_or_else(|| project_path.to_string_lossy().into_owned()),
        project_path,
        runtime: Some(RuntimeKind::Rust),
        manifest: Some("Cargo.toml".into()),
        executable_candidates,
        warnings,
        already_registered: false,
    }
}

fn inspect_node(
    manifest_path: &Path,
    project_path: PathBuf,
    candidate_id: String,
    suggested_id: Option<String>,
) -> DiscoveredProject {
    let value = match fs::read_to_string(manifest_path)
        .map_err(|error| error.to_string())
        .and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|error| error.to_string())
        }) {
        Ok(value) => value,
        Err(error) => {
            return malformed_candidate(
                project_path,
                candidate_id,
                suggested_id,
                "package.json",
                error,
            );
        }
    };

    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| project_path.to_str().unwrap_or("node-mcp"))
        .to_owned();

    let mut executable_candidates = Vec::new();
    if let Some(bin) = value.get("bin") {
        match bin {
            serde_json::Value::String(path) => {
                executable_candidates.push(PathBuf::from(path));
            }
            serde_json::Value::Object(entries) => {
                for value in entries.values() {
                    if let Some(path) = value.as_str() {
                        executable_candidates.push(PathBuf::from(path));
                    }
                }
            }
            _ => {}
        }
    }
    executable_candidates.retain(|path| is_safe_manifest_relative(path));

    let warnings = if executable_candidates.is_empty() {
        vec!["no safe project-local package.json bin entry found".into()]
    } else {
        vec![]
    };

    DiscoveredProject {
        candidate_id,
        suggested_id,
        name,
        project_path,
        runtime: Some(RuntimeKind::Node),
        manifest: Some("package.json".into()),
        executable_candidates,
        warnings,
        already_registered: false,
    }
}

fn inspect_python(
    manifest_path: &Path,
    project_path: PathBuf,
    candidate_id: String,
    suggested_id: Option<String>,
) -> DiscoveredProject {
    let value = match read_toml_document(manifest_path) {
        Ok(value) => value,
        Err(error) => {
            return malformed_candidate(
                project_path,
                candidate_id,
                suggested_id,
                "pyproject.toml",
                error,
            );
        }
    };

    let project = value.get("project").and_then(toml::Value::as_table);
    let name = project
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| project_path.to_str().unwrap_or("python-mcp"))
        .to_owned();

    let mut executable_candidates = Vec::new();
    if let Some(scripts) = project
        .and_then(|table| table.get("scripts"))
        .and_then(toml::Value::as_table)
    {
        let project_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        for script in scripts.keys() {
            let unix = PathBuf::from(".venv/bin").join(script);
            let windows = PathBuf::from(".venv/Scripts").join(format!("{script}.exe"));
            if project_dir.join(&unix).is_file() {
                executable_candidates.push(unix);
            }
            if project_dir.join(&windows).is_file() {
                executable_candidates.push(windows);
            }
        }
    }

    let warnings = if executable_candidates.is_empty() {
        vec![
            "Python project metadata found, but no installed project-local .venv script is available"
                .into(),
        ]
    } else {
        vec![]
    };

    DiscoveredProject {
        candidate_id,
        suggested_id,
        name,
        project_path,
        runtime: Some(RuntimeKind::Python),
        manifest: Some("pyproject.toml".into()),
        executable_candidates,
        warnings,
        already_registered: false,
    }
}

fn read_toml_document(path: &Path) -> Result<toml::Value, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    toml::from_str(&text).map_err(|error| error.to_string())
}

fn malformed_candidate(
    project_path: PathBuf,
    candidate_id: String,
    suggested_id: Option<String>,
    manifest: &str,
    error: String,
) -> DiscoveredProject {
    DiscoveredProject {
        candidate_id,
        suggested_id,
        name: project_path.to_string_lossy().into_owned(),
        project_path,
        runtime: None,
        manifest: Some(manifest.into()),
        executable_candidates: vec![],
        warnings: vec![format!("malformed {manifest}: {error}")],
        already_registered: false,
    }
}

fn sanitize_id(value: &str) -> Option<String> {
    let id = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();

    if id.is_empty() { None } else { Some(id) }
}

fn is_safe_manifest_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mcp-studio-discovery-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn detects_rust_metadata_without_execution() {
        let project = temp_project("rust");
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"sample-mcp\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let candidate = inspect_project(&project, PathBuf::from("sample"));
        assert_eq!(candidate.runtime, Some(RuntimeKind::Rust));
        assert!(
            candidate
                .executable_candidates
                .contains(&PathBuf::from("target/release/sample-mcp"))
        );
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn detects_node_bin_metadata() {
        let project = temp_project("node");
        fs::write(
            project.join("package.json"),
            r#"{"name":"sample-node","bin":{"sample":"bin/server.js"}}"#,
        )
        .unwrap();

        let candidate = inspect_project(&project, PathBuf::from("sample-node"));
        assert_eq!(candidate.runtime, Some(RuntimeKind::Node));
        assert_eq!(
            candidate.executable_candidates,
            vec![PathBuf::from("bin/server.js")]
        );
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn detects_python_project_without_importing_code() {
        let project = temp_project("python");
        fs::create_dir_all(project.join(".venv/bin")).unwrap();
        fs::write(project.join(".venv/bin/sample"), "fixture").unwrap();
        fs::write(
            project.join("pyproject.toml"),
            "[project]\nname = \"sample-python\"\n[project.scripts]\nsample = \"sample:main\"\n",
        )
        .unwrap();

        let candidate = inspect_project(&project, PathBuf::from("sample-python"));
        assert_eq!(candidate.runtime, Some(RuntimeKind::Python));
        assert!(
            candidate
                .executable_candidates
                .contains(&PathBuf::from(".venv/bin/sample"))
        );
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn malformed_manifest_is_isolated_as_warning() {
        let project = temp_project("malformed");
        fs::write(project.join("Cargo.toml"), "not = [valid").unwrap();
        let candidate = inspect_project(&project, PathBuf::from("bad"));
        assert!(candidate.runtime.is_none());
        assert_eq!(candidate.manifest.as_deref(), Some("Cargo.toml"));
        assert!(!candidate.warnings.is_empty());
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn ignores_known_infrastructure_names() {
        assert!(should_ignore("studio"));
        assert!(should_ignore("tunnel-client"));
        assert!(should_ignore("gateway"));
        assert!(should_ignore(".hidden"));
        assert!(!should_ignore("blender"));
    }

    #[test]
    fn plain_directory_is_not_a_supported_candidate() {
        let project = temp_project("plain");
        let candidate = inspect_project(&project, PathBuf::from("data"));
        assert!(candidate.manifest.is_none());
        assert!(candidate.runtime.is_none());
        let _ = fs::remove_dir_all(project);
    }
}
