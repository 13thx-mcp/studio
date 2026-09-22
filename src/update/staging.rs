use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use flate2::read::GzDecoder;
use reqwest::{Client, Url, header::ACCEPT, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder as TempBuilder;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};
use zip::ZipArchive;

use crate::error::{StudioError, StudioResult};

use super::{
    AvailableRelease, ComponentCatalog, ComponentClass, ComponentId, ComponentPolicy,
    InstallTarget, Platform, ReleaseAsset, ReleaseAssetKind, ReleaseProvider, ReleaseProviderId,
    Version,
    github_http::trusted_redirect_target,
    openai::{TUNNEL_FULL_ASSET_PREFIX, TUNNEL_RUNTIME_BINARY_NAME},
};

const STAGING_DIR_NAME: &str = ".mcp-studio-staging";
const STAGED_METADATA_NAME: &str = "staged.json";
const CHECKSUM_MANIFEST_NAME: &str = "SHA256SUMS.txt";
const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 4096;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedArtifact {
    pub component: ComponentId,
    pub version: Version,
    pub provider: ReleaseProviderId,
    pub release_tag: String,
    pub platform: Platform,
    pub asset_name: String,
    pub archive_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_asset_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_archive_sha256: Option<String>,
    pub staging_path: PathBuf,
    pub package_root: PathBuf,
    pub validated_executables: Vec<PathBuf>,
    pub validated_executable_sha256: Vec<String>,
    pub verified_at_unix_seconds: u64,
}

#[derive(Clone)]
pub struct ArtifactStager {
    catalog: ComponentCatalog,
    staging_root: PathBuf,
    fetcher: Arc<dyn ArtifactFetcher>,
    version_validator: Arc<dyn BinaryVersionValidator>,
}

impl ArtifactStager {
    pub fn new(catalog: ComponentCatalog) -> StudioResult<Self> {
        let staging_root = catalog.runtime_root().join(STAGING_DIR_NAME);
        prepare_staging_root(catalog.runtime_root(), &staging_root)?;
        cleanup_partial_directories(&staging_root)?;
        Ok(Self {
            catalog,
            staging_root,
            fetcher: Arc::new(ReqwestArtifactFetcher::new()?),
            version_validator: Arc::new(ProcessBinaryVersionValidator),
        })
    }

    #[cfg(test)]
    fn with_dependencies(
        catalog: ComponentCatalog,
        fetcher: Arc<dyn ArtifactFetcher>,
        version_validator: Arc<dyn BinaryVersionValidator>,
    ) -> StudioResult<Self> {
        let staging_root = catalog.runtime_root().join(STAGING_DIR_NAME);
        prepare_staging_root(catalog.runtime_root(), &staging_root)?;
        cleanup_partial_directories(&staging_root)?;
        Ok(Self {
            catalog,
            staging_root,
            fetcher,
            version_validator,
        })
    }

    pub fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    pub fn staged_id(&self, staged: &StagedArtifact) -> StudioResult<String> {
        let id = staged
            .staging_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| StudioError::StagingFailure("invalid staged transaction id".into()))?;
        if !safe_ready_id(id) {
            return Err(StudioError::StagingFailure(
                "invalid staged transaction id".into(),
            ));
        }
        Ok(id.to_owned())
    }

    pub fn load_ready(&self, transaction_id: &str) -> StudioResult<StagedArtifact> {
        if !safe_ready_id(transaction_id) {
            return Err(StudioError::StagingFailure(
                "invalid staged transaction id".into(),
            ));
        }
        let root = self.staging_root.join(transaction_id);
        let root_meta = fs::symlink_metadata(&root)?;
        if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
            return Err(StudioError::StagingFailure(
                "ready staging root is not a directory".into(),
            ));
        }
        let canonical_staging = fs::canonicalize(&self.staging_root)?;
        let canonical_root = fs::canonicalize(&root)?;
        if canonical_root.parent() != Some(canonical_staging.as_path()) {
            return Err(StudioError::StagingFailure(
                "ready staging path escaped staging root".into(),
            ));
        }
        let metadata_path = root.join(STAGED_METADATA_NAME);
        let metadata = fs::symlink_metadata(&metadata_path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StudioError::StagingFailure(
                "staged metadata is not a regular file".into(),
            ));
        }
        let staged: StagedArtifact = serde_json::from_slice(&fs::read(&metadata_path)?)?;
        if fs::canonicalize(&staged.staging_path)? != canonical_root {
            return Err(StudioError::StagingFailure(
                "staged metadata path mismatch".into(),
            ));
        }
        let policy = self.catalog.component(staged.component)?;
        if staged.provider != policy.provider
            || staged.release_tag != format!("v{}", staged.version)
        {
            return Err(StudioError::StagingFailure(
                "staged release identity mismatch".into(),
            ));
        }
        let expected_name = super::expected_asset_name(
            policy,
            &AvailableRelease {
                component: staged.component,
                version: staged.version.clone(),
                tag: staged.release_tag.clone(),
                assets: vec![],
                checksum_manifest_url: String::new(),
            },
            staged.platform,
        );
        if staged.asset_name != expected_name {
            return Err(StudioError::StagingFailure(
                "staged asset identity mismatch".into(),
            ));
        }
        let companion = match (
            staged.component == ComponentId::Tunnel,
            &staged.companion_asset_name,
            &staged.companion_archive_sha256,
        ) {
            (true, Some(name), Some(sha)) => Some((name, sha)),
            (true, _, _) => {
                return Err(StudioError::StagingFailure(
                    "tunnel full-client artifact identity is incomplete".into(),
                ));
            }
            (false, None, None) => None,
            (false, _, _) => {
                return Err(StudioError::StagingFailure(
                    "non-tunnel artifact has unexpected companion identity".into(),
                ));
            }
        };
        if let Some((name, _)) = companion {
            let expected = format!(
                "{TUNNEL_FULL_ASSET_PREFIX}-v{}-{}.zip",
                staged.version, staged.platform
            );
            if name != &expected {
                return Err(StudioError::StagingFailure(
                    "staged full-client asset identity mismatch".into(),
                ));
            }
        }
        let package_root = fs::canonicalize(&staged.package_root)?;
        if !package_root.starts_with(&canonical_root) {
            return Err(StudioError::StagingFailure(
                "staged package root escaped transaction".into(),
            ));
        }
        if staged.validated_executables.is_empty()
            || staged.validated_executables.len() != staged.validated_executable_sha256.len()
        {
            return Err(StudioError::StagingFailure(
                "staged executable identity is incomplete".into(),
            ));
        }
        for (executable, expected_sha) in staged
            .validated_executables
            .iter()
            .zip(&staged.validated_executable_sha256)
        {
            let metadata = fs::symlink_metadata(executable)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StudioError::StagingFailure(
                    "staged executable is not a regular file".into(),
                ));
            }
            if !fs::canonicalize(executable)?.starts_with(&canonical_root) {
                return Err(StudioError::StagingFailure(
                    "staged executable escaped transaction".into(),
                ));
            }
            if sha256_file(executable)? != *expected_sha {
                return Err(StudioError::StagingFailure(
                    "staged executable checksum changed after verification".into(),
                ));
            }
        }
        let archive = root.join("archive").join(&staged.asset_name);
        let archive_metadata = fs::symlink_metadata(&archive)?;
        if archive_metadata.file_type().is_symlink() || !archive_metadata.is_file() {
            return Err(StudioError::StagingFailure(
                "verified staged archive is missing".into(),
            ));
        }
        let manifest_path = root.join("archive").join(CHECKSUM_MANIFEST_NAME);
        let manifest_metadata = fs::symlink_metadata(&manifest_path)?;
        if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
            return Err(StudioError::StagingFailure(
                "retained checksum manifest is missing".into(),
            ));
        }
        let manifest_sha = checksum_for_asset(
            staged.component,
            &fs::read(&manifest_path)?,
            &staged.asset_name,
        )?;
        if manifest_sha != staged.archive_sha256 {
            return Err(StudioError::StagingFailure(
                "staged metadata digest no longer matches checksum manifest".into(),
            ));
        }
        let actual_sha = sha256_file(&archive)?;
        if actual_sha != manifest_sha {
            return Err(StudioError::ChecksumMismatch {
                component: staged.component.to_string(),
                asset: staged.asset_name.clone(),
                expected: manifest_sha,
                actual: actual_sha,
            });
        }
        if let Some((name, expected_sha)) = companion {
            let archive = root.join("archive").join(name);
            let metadata = fs::symlink_metadata(&archive)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StudioError::StagingFailure(
                    "verified full-client archive is missing".into(),
                ));
            }
            let manifest_sha =
                checksum_for_asset(staged.component, &fs::read(&manifest_path)?, name)?;
            if manifest_sha != *expected_sha {
                return Err(StudioError::StagingFailure(
                    "full-client metadata digest no longer matches checksum manifest".into(),
                ));
            }
            let actual_sha = sha256_file(&archive)?;
            if actual_sha != manifest_sha {
                return Err(StudioError::ChecksumMismatch {
                    component: staged.component.to_string(),
                    asset: name.clone(),
                    expected: manifest_sha,
                    actual: actual_sha,
                });
            }
        }
        Ok(staged)
    }

    pub async fn stage(
        &self,
        provider: &dyn ReleaseProvider,
        release: &AvailableRelease,
        platform: Platform,
    ) -> StudioResult<StagedArtifact> {
        let policy = self.catalog.component(release.component)?;
        if provider.provider_id() != policy.provider {
            return Err(StudioError::ReleaseProviderMismatch {
                component: policy.id.to_string(),
                expected: format!("{:?}", policy.provider),
                actual: format!("{:?}", provider.provider_id()),
            });
        }
        let selected = provider.select_asset(release, platform)?;
        validate_download_url(policy, &selected.download_url)?;
        let manifest = provider.checksum_manifest(release).await?;
        let expected_sha = checksum_for_asset(policy.id, &manifest, &selected.name)?;

        let temp = TempBuilder::new()
            .prefix(".partial-")
            .tempdir_in(&self.staging_root)?;
        let temp_path = temp.path().to_path_buf();
        let archive_dir = temp_path.join("archive");
        let extract_dir = temp_path.join("extracted");
        fs::create_dir_all(&archive_dir)?;
        fs::create_dir_all(&extract_dir)?;

        let archive_path = archive_dir.join(&selected.name);
        let downloaded = self
            .fetcher
            .download(&selected.download_url, &archive_path, MAX_ARTIFACT_BYTES)
            .await?;
        if downloaded.sha256 != expected_sha {
            return Err(StudioError::ChecksumMismatch {
                component: policy.id.to_string(),
                asset: selected.name.clone(),
                expected: expected_sha,
                actual: downloaded.sha256,
            });
        }
        fs::write(archive_dir.join(CHECKSUM_MANIFEST_NAME), &manifest)?;

        let extracted = match policy.asset.kind {
            ReleaseAssetKind::PlatformTarGz | ReleaseAssetKind::ArchitectureIndependentTarGz => {
                extract_tar_gz(policy.id, &archive_path, &extract_dir)?
            }
            ReleaseAssetKind::PlatformZip => extract_zip(policy.id, &archive_path, &extract_dir)?,
        };
        let mut validated = validate_package(
            policy,
            release,
            selected,
            platform,
            &extract_dir,
            &extracted,
        )?;
        for executable in &validated.executables {
            normalize_executable_permission(executable)?;
        }

        let mut companion_identity = None;
        if policy.id == ComponentId::Tunnel {
            let companion = provider
                .select_companion_asset(release, platform)?
                .ok_or_else(|| {
                    StudioError::StagingFailure("tunnel full-client asset is required".into())
                })?;
            validate_download_url(policy, &companion.download_url)?;
            let companion_expected_sha = checksum_for_asset(policy.id, &manifest, &companion.name)?;
            let companion_archive = archive_dir.join(&companion.name);
            let companion_downloaded = self
                .fetcher
                .download(
                    &companion.download_url,
                    &companion_archive,
                    MAX_ARTIFACT_BYTES,
                )
                .await?;
            if companion_downloaded.sha256 != companion_expected_sha {
                return Err(StudioError::ChecksumMismatch {
                    component: policy.id.to_string(),
                    asset: companion.name.clone(),
                    expected: companion_expected_sha,
                    actual: companion_downloaded.sha256,
                });
            }
            let companion_extract = temp_path.join("full-client-extracted");
            fs::create_dir_all(&companion_extract)?;
            let extracted = extract_zip(policy.id, &companion_archive, &companion_extract)?;
            let full_client =
                validate_tunnel_client_package(policy, companion, &companion_extract, &extracted)?;
            normalize_executable_permission(&full_client)?;
            self.version_validator
                .validate_pair(&validated.executables[0], &full_client, &release.version)
                .await?;
            let installed_full_client = validated.package_root.join(TUNNEL_FULL_ASSET_PREFIX);
            fs::copy(&full_client, &installed_full_client)?;
            normalize_executable_permission(&installed_full_client)?;
            validated.executables.push(installed_full_client);
            companion_identity = Some((companion.name.clone(), companion_downloaded.sha256));
        }

        let executable_sha256 = validated
            .executables
            .iter()
            .map(|path| sha256_file(path))
            .collect::<StudioResult<Vec<_>>>()?;

        let temp_name = temp_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| StudioError::StagingFailure("invalid temporary staging path".into()))?;
        let final_path = self
            .staging_root
            .join(temp_name.replacen(".partial-", "ready-", 1));
        let relative_package_root =
            validated
                .package_root
                .strip_prefix(&temp_path)
                .map_err(|_| {
                    StudioError::StagingFailure("package root escaped staging transaction".into())
                })?;
        let relative_executables = validated
            .executables
            .iter()
            .map(|path| {
                path.strip_prefix(&temp_path)
                    .map(Path::to_path_buf)
                    .map_err(|_| {
                        StudioError::StagingFailure("executable escaped staging transaction".into())
                    })
            })
            .collect::<StudioResult<Vec<_>>>()?;

        let staged = StagedArtifact {
            component: policy.id,
            version: release.version.clone(),
            provider: policy.provider,
            release_tag: release.tag.clone(),
            platform,
            asset_name: selected.name.clone(),
            archive_sha256: downloaded.sha256,
            companion_asset_name: companion_identity.as_ref().map(|(name, _)| name.clone()),
            companion_archive_sha256: companion_identity.map(|(_, sha)| sha),
            staging_path: final_path.clone(),
            package_root: final_path.join(relative_package_root),
            validated_executables: relative_executables
                .iter()
                .map(|path| final_path.join(path))
                .collect(),
            validated_executable_sha256: executable_sha256,
            verified_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| StudioError::StagingFailure(error.to_string()))?
                .as_secs(),
        };
        write_metadata_atomic(&temp_path, &staged)?;
        let kept = temp.keep();
        fs::rename(&kept, &final_path)?;
        Ok(staged)
    }
}

#[derive(Debug)]
struct DownloadResult {
    sha256: String,
}

#[async_trait]
trait ArtifactFetcher: Send + Sync {
    async fn download(
        &self,
        url: &str,
        destination: &Path,
        max_bytes: u64,
    ) -> StudioResult<DownloadResult>;
}

struct ReqwestArtifactFetcher {
    client: Client,
}

impl ReqwestArtifactFetcher {
    fn new() -> StudioResult<Self> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .https_only(true)
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("too many redirects");
                }
                if trusted_redirect_target(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .user_agent(format!(
                "mcp-studio/{} artifact-stager",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| StudioError::ArtifactDownloadFailed {
                component: "staging".into(),
                detail: error.to_string(),
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ArtifactFetcher for ReqwestArtifactFetcher {
    async fn download(
        &self,
        url: &str,
        destination: &Path,
        max_bytes: u64,
    ) -> StudioResult<DownloadResult> {
        let parsed = Url::parse(url).map_err(|_| StudioError::ArtifactDownloadFailed {
            component: "staging".into(),
            detail: "invalid artifact URL".into(),
        })?;
        if parsed.scheme() != "https" {
            return Err(StudioError::ArtifactDownloadFailed {
                component: "staging".into(),
                detail: "artifact URL must use HTTPS".into(),
            });
        }
        let mut response = self
            .client
            .get(url)
            .header(ACCEPT, "application/octet-stream")
            .send()
            .await
            .map_err(|error| StudioError::ArtifactDownloadFailed {
                component: "staging".into(),
                detail: if error.is_timeout() {
                    "request timed out".into()
                } else {
                    "request failed".into()
                },
            })?;
        if !response.status().is_success() {
            return Err(StudioError::ArtifactDownloadFailed {
                component: "staging".into(),
                detail: format!("HTTP status {}", response.status().as_u16()),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes)
        {
            return Err(StudioError::ArtifactTooLarge { limit: max_bytes });
        }
        let mut file = tokio::fs::File::create(destination).await?;
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|error| StudioError::ArtifactDownloadFailed {
                    component: "staging".into(),
                    detail: error.to_string(),
                })?
        {
            total = total.saturating_add(chunk.len() as u64);
            if total > max_bytes {
                return Err(StudioError::ArtifactTooLarge { limit: max_bytes });
            }
            digest.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(DownloadResult {
            sha256: hex_digest(digest.finalize().as_slice()),
        })
    }
}

#[async_trait]
trait BinaryVersionValidator: Send + Sync {
    async fn validate(&self, path: &Path, version: &Version) -> StudioResult<()>;

    async fn validate_pair(
        &self,
        runtime: &Path,
        full_client: &Path,
        version: &Version,
    ) -> StudioResult<()> {
        self.validate(runtime, version).await?;
        self.validate(full_client, version).await
    }
}

struct ProcessBinaryVersionValidator;

#[async_trait]
impl BinaryVersionValidator for ProcessBinaryVersionValidator {
    async fn validate(&self, path: &Path, version: &Version) -> StudioResult<()> {
        let mut command = Command::new(path);
        command.arg("--version").env_clear().kill_on_drop(true);
        let output = timeout(VERSION_TIMEOUT, command.output())
            .await
            .map_err(|_| StudioError::BinaryVersionValidationFailed {
                path: path.display().to_string(),
                detail: "--version timed out".into(),
            })??;
        if !output.status.success() {
            return Err(StudioError::BinaryVersionValidationFailed {
                path: path.display().to_string(),
                detail: format!("--version exited with {}", output.status),
            });
        }
        let stdout = String::from_utf8(output.stdout).map_err(|error| {
            StudioError::BinaryVersionValidationFailed {
                path: path.display().to_string(),
                detail: error.to_string(),
            }
        })?;
        if stdout.split_whitespace().next() != Some(version.to_string().as_str()) {
            return Err(StudioError::BinaryVersionValidationFailed {
                path: path.display().to_string(),
                detail: format!("expected version {version}, got {stdout:?}"),
            });
        }
        Ok(())
    }

    async fn validate_pair(
        &self,
        runtime: &Path,
        full_client: &Path,
        version: &Version,
    ) -> StudioResult<()> {
        let runtime = process_identity(runtime).await?;
        let full_client = process_identity(full_client).await?;
        if runtime.version != version.to_string() || full_client.version != version.to_string() {
            return Err(StudioError::BinaryVersionValidationFailed {
                path: full_client.path,
                detail: format!(
                    "expected version {version}, got runtime={} full-client={}",
                    runtime.version, full_client.version
                ),
            });
        }
        match (runtime.git_sha, full_client.git_sha) {
            (Some(runtime_sha), Some(full_client_sha)) if runtime_sha == full_client_sha => Ok(()),
            (runtime_sha, full_client_sha) => Err(StudioError::BinaryVersionValidationFailed {
                path: full_client.path,
                detail: format!(
                    "runtime/full-client release commit mismatch: runtime={runtime_sha:?} full-client={full_client_sha:?}"
                ),
            }),
        }
    }
}

struct BinaryIdentity {
    path: String,
    version: String,
    git_sha: Option<String>,
}

async fn process_identity(path: &Path) -> StudioResult<BinaryIdentity> {
    let mut command = Command::new(path);
    command.arg("--version").env_clear().kill_on_drop(true);
    let output = timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| StudioError::BinaryVersionValidationFailed {
            path: path.display().to_string(),
            detail: "--version timed out".into(),
        })??;
    if !output.status.success() {
        return Err(StudioError::BinaryVersionValidationFailed {
            path: path.display().to_string(),
            detail: format!("--version exited with {}", output.status),
        });
    }
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        StudioError::BinaryVersionValidationFailed {
            path: path.display().to_string(),
            detail: error.to_string(),
        }
    })?;
    parse_version_output(path, &stdout)
}

fn parse_version_output(path: &Path, stdout: &str) -> StudioResult<BinaryIdentity> {
    let tokens = stdout.split_whitespace().collect::<Vec<_>>();
    let Some(first) = tokens.first() else {
        return Err(StudioError::BinaryVersionValidationFailed {
            path: path.display().to_string(),
            detail: "--version returned no identity".into(),
        });
    };
    let (version, build_sha) = first.split_once('+').map_or_else(
        || ((*first).to_owned(), None),
        |(version, sha)| (version.to_owned(), Some(sha.to_owned())),
    );
    let labeled_sha = tokens
        .windows(2)
        .find_map(|pair| (pair[0] == "sha:").then(|| pair[1].to_owned()));
    let git_sha = labeled_sha.or(build_sha).filter(|sha| {
        (7..=64).contains(&sha.len()) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    Ok(BinaryIdentity {
        path: path.display().to_string(),
        version,
        git_sha,
    })
}

fn prepare_staging_root(runtime_root: &Path, staging_root: &Path) -> StudioResult<()> {
    fs::create_dir_all(runtime_root)?;
    fs::create_dir_all(staging_root)?;
    let runtime = fs::canonicalize(runtime_root)?;
    let staging = fs::canonicalize(staging_root)?;
    if !staging.starts_with(&runtime) || staging == runtime {
        return Err(StudioError::StagingFailure(
            "staging root is not confined below runtime_root".into(),
        ));
    }
    Ok(())
}

fn cleanup_partial_directories(staging_root: &Path) -> StudioResult<()> {
    for entry in fs::read_dir(staging_root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry.file_name().to_string_lossy().starts_with(".partial-")
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn validate_download_url(policy: &ComponentPolicy, value: &str) -> StudioResult<()> {
    let url = Url::parse(value).map_err(|_| StudioError::UntrustedReleaseAssetUrl {
        component: policy.id.to_string(),
        url: value.to_owned(),
    })?;
    let prefix = format!(
        "/{}/{}/releases/download/",
        policy.source.owner, policy.source.repository
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !url.path().starts_with(&prefix)
    {
        return Err(StudioError::UntrustedReleaseAssetUrl {
            component: policy.id.to_string(),
            url: value.to_owned(),
        });
    }
    Ok(())
}

fn checksum_for_asset(
    component: ComponentId,
    bytes: &[u8],
    filename: &str,
) -> StudioResult<String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| StudioError::InvalidChecksumManifest {
            component: component.to_string(),
            detail: error.to_string(),
        })?;
    let mut found = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 2
            || parts[0].len() != 64
            || !parts[0].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(StudioError::InvalidChecksumManifest {
                component: component.to_string(),
                detail: format!("invalid checksum line {line:?}"),
            });
        }
        if parts[1].trim_start_matches('*') == filename {
            found.push(parts[0].to_ascii_lowercase());
        }
    }
    match found.as_slice() {
        [checksum] => Ok(checksum.clone()),
        [] => Err(StudioError::ChecksumEntryMissing {
            component: component.to_string(),
            asset: filename.to_owned(),
        }),
        _ => Err(StudioError::ChecksumEntryAmbiguous {
            component: component.to_string(),
            asset: filename.to_owned(),
        }),
    }
}

#[derive(Debug)]
struct ExtractionResult {
    paths: Vec<PathBuf>,
}

fn extract_tar_gz(
    component: ComponentId,
    archive_path: &Path,
    destination: &Path,
) -> StudioResult<ExtractionResult> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    let mut extracted = 0_u64;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= MAX_ARCHIVE_ENTRIES {
            return Err(StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: "too many archive entries".into(),
            });
        }
        let mut entry = entry.map_err(|error| StudioError::UnsafeArchive {
            component: component.to_string(),
            detail: error.to_string(),
        })?;
        let relative = normalize_archive_path(
            component,
            &entry.path().map_err(|error| StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: error.to_string(),
            })?,
        )?;
        if !seen.insert(relative.clone()) {
            return Err(StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: format!("duplicate archive path {}", relative.display()),
            });
        }
        let kind = entry.header().entry_type();
        let target = destination.join(&relative);
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
        } else if kind.is_file() {
            extracted = extracted.saturating_add(entry.size());
            if extracted > MAX_EXTRACTED_BYTES {
                return Err(StudioError::UnsafeArchive {
                    component: component.to_string(),
                    detail: "extracted archive exceeds size limit".into(),
                });
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            io::copy(&mut entry, &mut output)?;
            output.flush()?;
        } else {
            return Err(StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: format!("unsupported tar entry type for {}", relative.display()),
            });
        }
        paths.push(relative);
    }
    Ok(ExtractionResult { paths })
}

fn extract_zip(
    component: ComponentId,
    archive_path: &Path,
    destination: &Path,
) -> StudioResult<ExtractionResult> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(|error| StudioError::UnsafeArchive {
        component: component.to_string(),
        detail: error.to_string(),
    })?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(StudioError::UnsafeArchive {
            component: component.to_string(),
            detail: "too many archive entries".into(),
        });
    }
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    let mut extracted = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: error.to_string(),
            })?;
        let relative = normalize_archive_path(component, Path::new(entry.name()))?;
        if !seen.insert(relative.clone()) {
            return Err(StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: format!("duplicate archive path {}", relative.display()),
            });
        }
        let mode = entry.unix_mode().unwrap_or(0);
        let file_type = mode & 0o170000;
        if file_type != 0 && file_type != 0o100000 && file_type != 0o040000 {
            return Err(StudioError::UnsafeArchive {
                component: component.to_string(),
                detail: format!("unsupported zip entry type for {}", relative.display()),
            });
        }
        let target = destination.join(&relative);
        if entry.is_dir() || file_type == 0o040000 {
            fs::create_dir_all(&target)?;
        } else {
            extracted = extracted.saturating_add(entry.size());
            if extracted > MAX_EXTRACTED_BYTES {
                return Err(StudioError::UnsafeArchive {
                    component: component.to_string(),
                    detail: "extracted archive exceeds size limit".into(),
                });
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            io::copy(&mut entry, &mut output)?;
            output.flush()?;
        }
        paths.push(relative);
    }
    Ok(ExtractionResult { paths })
}

fn normalize_archive_path(component: ComponentId, path: &Path) -> StudioResult<PathBuf> {
    let raw = path.to_string_lossy();
    if raw.contains('\\') {
        return Err(StudioError::UnsafeArchive {
            component: component.to_string(),
            detail: format!("ambiguous path separator in {raw:?}"),
        });
    }
    let mut normalized = PathBuf::new();
    for path_component in path.components() {
        match path_component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(StudioError::UnsafeArchive {
                    component: component.to_string(),
                    detail: format!("unsafe archive path {raw:?}"),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(StudioError::UnsafeArchive {
            component: component.to_string(),
            detail: "empty archive path".into(),
        });
    }
    Ok(normalized)
}

struct ValidatedPackage {
    package_root: PathBuf,
    executables: Vec<PathBuf>,
}

fn validate_package(
    policy: &ComponentPolicy,
    release: &AvailableRelease,
    asset: &ReleaseAsset,
    _platform: Platform,
    extract_dir: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<ValidatedPackage> {
    match policy.class {
        ComponentClass::McpBinary => validate_project_binary(policy, asset, extract_dir, extracted),
        ComponentClass::Service => validate_studio(policy, asset, extract_dir, extracted),
        ComponentClass::ControlBundle => {
            validate_fleet(policy, release, asset, extract_dir, extracted)
        }
        ComponentClass::UpstreamRuntime => validate_tunnel(policy, asset, extract_dir, extracted),
    }
}

fn tar_root_name(asset_name: &str) -> StudioResult<&str> {
    asset_name
        .strip_suffix(".tar.gz")
        .ok_or_else(|| StudioError::PackageValidationFailed {
            component: "package".into(),
            detail: format!("unexpected tar asset name {asset_name}"),
        })
}

fn ensure_all_under_root(
    component: ComponentId,
    root: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<()> {
    if extracted
        .paths
        .iter()
        .all(|path| path == root || path.starts_with(root))
    {
        Ok(())
    } else {
        Err(StudioError::PackageValidationFailed {
            component: component.to_string(),
            detail: "archive contains entries outside expected package root".into(),
        })
    }
}

fn validate_project_binary(
    policy: &ComponentPolicy,
    asset: &ReleaseAsset,
    extract_dir: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<ValidatedPackage> {
    let root_rel = PathBuf::from(tar_root_name(&asset.name)?);
    ensure_all_under_root(policy.id, &root_rel, extracted)?;
    let root = extract_dir.join(&root_rel);
    let InstallTarget::BinRootBinary { binary } = policy.install_target else {
        return Err(StudioError::PackageValidationFailed {
            component: policy.id.to_string(),
            detail: "MCP binary install policy mismatch".into(),
        });
    };
    let executable = root.join(binary);
    require_file(policy.id, &executable, binary)?;
    for path in &extracted.paths {
        if path == &root_rel {
            continue;
        }
        let relative = path.strip_prefix(&root_rel).unwrap();
        if relative == Path::new(binary)
            || relative == Path::new("README.md")
            || relative == Path::new("LICENSE")
        {
            continue;
        }
        return Err(StudioError::PackageValidationFailed {
            component: policy.id.to_string(),
            detail: format!("unexpected package entry {}", path.display()),
        });
    }
    Ok(ValidatedPackage {
        package_root: root,
        executables: vec![executable],
    })
}

fn validate_studio(
    policy: &ComponentPolicy,
    asset: &ReleaseAsset,
    extract_dir: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<ValidatedPackage> {
    let root_rel = PathBuf::from(tar_root_name(&asset.name)?);
    ensure_all_under_root(policy.id, &root_rel, extracted)?;
    let root = extract_dir.join(&root_rel);
    let executable = root.join("mcp-studio");
    require_file(policy.id, &executable, "mcp-studio")?;
    require_file(
        policy.id,
        &root.join("web/dist/index.html"),
        "web/dist/index.html",
    )?;
    for path in &extracted.paths {
        if path == &root_rel {
            continue;
        }
        let relative = path.strip_prefix(&root_rel).unwrap();
        if relative == Path::new("mcp-studio")
            || relative == Path::new("README.md")
            || relative == Path::new("LICENSE")
            || relative.starts_with("web")
        {
            continue;
        }
        return Err(StudioError::PackageValidationFailed {
            component: policy.id.to_string(),
            detail: format!("unexpected Studio package entry {}", path.display()),
        });
    }
    Ok(ValidatedPackage {
        package_root: root,
        executables: vec![executable],
    })
}

fn validate_fleet(
    policy: &ComponentPolicy,
    release: &AvailableRelease,
    asset: &ReleaseAsset,
    extract_dir: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<ValidatedPackage> {
    let root_rel = PathBuf::from(tar_root_name(&asset.name)?);
    ensure_all_under_root(policy.id, &root_rel, extracted)?;
    let root = extract_dir.join(&root_rel);
    for required in [
        "VERSION",
        "fleet.toml",
        "README.md",
        "scripts/fleetctl.py",
        "hosts/mirin.example.toml",
    ] {
        require_file(policy.id, &root.join(required), required)?;
    }
    let allowed = BTreeSet::from([
        PathBuf::from("VERSION"),
        PathBuf::from("fleet.toml"),
        PathBuf::from("README.md"),
        PathBuf::from("scripts"),
        PathBuf::from("scripts/fleetctl.py"),
        PathBuf::from("hosts"),
        PathBuf::from("hosts/mirin.example.toml"),
    ]);
    for path in &extracted.paths {
        if path == &root_rel {
            continue;
        }
        let relative = path.strip_prefix(&root_rel).unwrap();
        if !allowed.contains(relative) {
            return Err(StudioError::PackageValidationFailed {
                component: policy.id.to_string(),
                detail: format!("unexpected Fleet package entry {}", path.display()),
            });
        }
    }
    let version = fs::read_to_string(root.join("VERSION"))?;
    if version.trim() != release.version.to_string() {
        return Err(StudioError::PackageValidationFailed {
            component: policy.id.to_string(),
            detail: format!("Fleet VERSION mismatch: {:?}", version.trim()),
        });
    }
    let executable = root.join("scripts/fleetctl.py");
    Ok(ValidatedPackage {
        package_root: root,
        executables: vec![executable],
    })
}

fn validate_tunnel(
    policy: &ComponentPolicy,
    asset: &ReleaseAsset,
    extract_dir: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<ValidatedPackage> {
    validate_tunnel_package(
        policy,
        asset,
        extract_dir,
        extracted,
        TUNNEL_RUNTIME_BINARY_NAME,
    )
}

fn validate_tunnel_client_package(
    policy: &ComponentPolicy,
    asset: &ReleaseAsset,
    extract_dir: &Path,
    extracted: &ExtractionResult,
) -> StudioResult<PathBuf> {
    let validated = validate_tunnel_package(
        policy,
        asset,
        extract_dir,
        extracted,
        TUNNEL_FULL_ASSET_PREFIX,
    )?;
    Ok(validated.executables[0].clone())
}

fn validate_tunnel_package(
    policy: &ComponentPolicy,
    asset: &ReleaseAsset,
    extract_dir: &Path,
    extracted: &ExtractionResult,
    client_binary: &str,
) -> StudioResult<ValidatedPackage> {
    let stem =
        asset
            .name
            .strip_suffix(".zip")
            .ok_or_else(|| StudioError::PackageValidationFailed {
                component: policy.id.to_string(),
                detail: "tunnel asset is not a ZIP".into(),
            })?;
    let license = format!("{stem}-licenses.txt");
    let sbom = format!("{stem}.spdx.json");
    let expected = BTreeSet::from([
        PathBuf::from(client_binary),
        PathBuf::from("cloudflared"),
        PathBuf::from("cloudflared-manifest.json"),
        PathBuf::from("LICENSE"),
        PathBuf::from("NOTICE"),
        PathBuf::from(license),
        PathBuf::from(sbom),
    ]);
    let actual = extracted.paths.iter().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(StudioError::PackageValidationFailed {
            component: policy.id.to_string(),
            detail: format!("unexpected tunnel ZIP structure: {actual:?}"),
        });
    }
    let runtime = extract_dir.join(client_binary);
    let cloudflared = extract_dir.join("cloudflared");
    require_file(policy.id, &runtime, client_binary)?;
    require_file(policy.id, &cloudflared, "cloudflared")?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(extract_dir.join("cloudflared-manifest.json"))?)?;
    if manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(StudioError::PackageValidationFailed {
            component: policy.id.to_string(),
            detail: "cloudflared manifest is missing version".into(),
        });
    }
    Ok(ValidatedPackage {
        package_root: extract_dir.to_path_buf(),
        executables: vec![runtime, cloudflared],
    })
}

fn require_file(component: ComponentId, path: &Path, label: &str) -> StudioResult<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(StudioError::PackageValidationFailed {
            component: component.to_string(),
            detail: format!("missing expected file {label}"),
        })
    }
}

#[cfg(unix)]
fn normalize_executable_permission(path: &Path) -> StudioResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn normalize_executable_permission(_path: &Path) -> StudioResult<()> {
    Ok(())
}

fn write_metadata_atomic(transaction_root: &Path, staged: &StagedArtifact) -> StudioResult<()> {
    let temp = transaction_root.join(format!("{STAGED_METADATA_NAME}.tmp"));
    let final_path = transaction_root.join(STAGED_METADATA_NAME);
    let bytes = serde_json::to_vec_pretty(staged)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temp, final_path)?;
    Ok(())
}

fn safe_ready_id(value: &str) -> bool {
    value.starts_with("ready-")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn sha256_file(path: &Path) -> StudioResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex_digest(digest.finalize().as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Mutex};

    use flate2::{Compression, write::GzEncoder};
    use tempfile::TempDir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;
    use crate::update::{
        Architecture, ComponentCatalog, HostRuntimeRoots, OperatingSystem, ReleaseAsset,
        select_release_asset,
    };

    #[derive(Default)]
    struct FakeFetcher {
        bodies: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl FakeFetcher {
        fn insert(&self, url: &str, body: Vec<u8>) {
            self.bodies.lock().unwrap().insert(url.into(), body);
        }
    }

    #[async_trait]
    impl ArtifactFetcher for FakeFetcher {
        async fn download(
            &self,
            url: &str,
            destination: &Path,
            max_bytes: u64,
        ) -> StudioResult<DownloadResult> {
            let body = self
                .bodies
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| StudioError::ArtifactDownloadFailed {
                    component: "test".into(),
                    detail: "missing fake response".into(),
                })?;
            if body.len() as u64 > max_bytes {
                return Err(StudioError::ArtifactTooLarge { limit: max_bytes });
            }
            fs::write(destination, &body)?;
            Ok(DownloadResult {
                sha256: sha256_bytes(&body),
            })
        }
    }

    struct FakeVersionValidator;
    #[async_trait]
    impl BinaryVersionValidator for FakeVersionValidator {
        async fn validate(&self, _path: &Path, _version: &Version) -> StudioResult<()> {
            Ok(())
        }
    }

    struct TestProvider {
        id: ReleaseProviderId,
        manifest: Vec<u8>,
        policy: ComponentPolicy,
    }
    #[async_trait]
    impl ReleaseProvider for TestProvider {
        fn provider_id(&self) -> ReleaseProviderId {
            self.id
        }
        async fn latest_release(
            &self,
            _component: &ComponentPolicy,
        ) -> StudioResult<AvailableRelease> {
            unreachable!()
        }
        async fn release(
            &self,
            _component: &ComponentPolicy,
            _version: &Version,
        ) -> StudioResult<AvailableRelease> {
            unreachable!()
        }
        fn select_asset<'a>(
            &self,
            release: &'a AvailableRelease,
            platform: Platform,
        ) -> StudioResult<&'a ReleaseAsset> {
            select_release_asset(&self.policy, release, platform)
        }
        fn select_companion_asset<'a>(
            &self,
            release: &'a AvailableRelease,
            platform: Platform,
        ) -> StudioResult<Option<&'a ReleaseAsset>> {
            if self.policy.id != ComponentId::Tunnel {
                return Ok(None);
            }
            let expected = format!(
                "{TUNNEL_FULL_ASSET_PREFIX}-v{}-{platform}.zip",
                release.version
            );
            let matches = release
                .assets
                .iter()
                .filter(|asset| asset.name == expected)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [asset] => Ok(Some(*asset)),
                [] => Err(StudioError::ReleaseAssetNotFound {
                    component: self.policy.id.to_string(),
                    expected,
                }),
                _ => Err(StudioError::AmbiguousReleaseAsset {
                    component: self.policy.id.to_string(),
                    expected,
                    count: matches.len(),
                }),
            }
        }
        async fn checksum_manifest(&self, _release: &AvailableRelease) -> StudioResult<Vec<u8>> {
            Ok(self.manifest.clone())
        }
    }

    fn platform() -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch: Architecture::Arm64,
        }
    }

    fn setup(
        id: ComponentId,
        version: &str,
        archive: Vec<u8>,
    ) -> (
        TempDir,
        ArtifactStager,
        TestProvider,
        AvailableRelease,
        PathBuf,
    ) {
        let root = TempDir::new().unwrap();
        let runtime = root.path().join("runtime");
        let bin = root.path().join("bin");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let catalog =
            ComponentCatalog::new(HostRuntimeRoots::new(bin.clone(), runtime.clone()).unwrap());
        let policy = catalog.component(id).unwrap().clone();
        let version = Version::parse(version).unwrap();
        let asset_name = super::super::expected_asset_name(
            &policy,
            &AvailableRelease {
                component: id,
                version: version.clone(),
                tag: format!("v{version}"),
                assets: vec![],
                checksum_manifest_url: String::new(),
            },
            platform(),
        );
        let url = format!(
            "https://github.com/{}/{}/releases/download/v{version}/{asset_name}",
            policy.source.owner, policy.source.repository
        );
        let sha = sha256_bytes(&archive);
        let mut manifest = format!("{sha}  {asset_name}\n");
        let mut assets = vec![ReleaseAsset {
            name: asset_name.clone(),
            download_url: url.clone(),
        }];
        let mut companion = None;
        if id == ComponentId::Tunnel {
            let companion_name = format!(
                "{TUNNEL_FULL_ASSET_PREFIX}-v{version}-{platform}.zip",
                platform = platform()
            );
            let companion_url = format!(
                "https://github.com/{}/{}/releases/download/v{version}/{companion_name}",
                policy.source.owner, policy.source.repository
            );
            let companion_stem = companion_name.strip_suffix(".zip").unwrap();
            let companion_archive = zip_bytes(&[
                (TUNNEL_FULL_ASSET_PREFIX, b"full-client", 0o644),
                ("cloudflared", b"cloudflared", 0o644),
                (
                    "cloudflared-manifest.json",
                    br#"{"version":"2026.8.2"}"#,
                    0o644,
                ),
                ("LICENSE", b"license", 0o644),
                ("NOTICE", b"notice", 0o644),
                (
                    &format!("{companion_stem}-licenses.txt"),
                    b"licenses",
                    0o644,
                ),
                (&format!("{companion_stem}.spdx.json"), b"{}", 0o644),
            ]);
            let companion_sha = sha256_bytes(&companion_archive);
            manifest.push_str(&format!("{companion_sha}  {companion_name}\n"));
            assets.push(ReleaseAsset {
                name: companion_name,
                download_url: companion_url.clone(),
            });
            companion = Some((companion_url, companion_archive));
        }
        let manifest = manifest.into_bytes();
        let release = AvailableRelease {
            component: id,
            version,
            tag: format!("v{}", policy.id.as_str()).replace(
                &format!("v{}", policy.id.as_str()),
                &format!("v{}", policy.id.as_str()),
            ),
            assets,
            checksum_manifest_url: format!(
                "https://github.com/{}/{}/releases/download/check/SHA256SUMS.txt",
                policy.source.owner, policy.source.repository
            ),
        };
        let release = AvailableRelease {
            tag: format!("v{}", release.version),
            ..release
        };
        let fetcher = Arc::new(FakeFetcher::default());
        fetcher.insert(&url, archive);
        if let Some((url, archive)) = companion {
            fetcher.insert(&url, archive);
        }
        let stager =
            ArtifactStager::with_dependencies(catalog, fetcher, Arc::new(FakeVersionValidator))
                .unwrap();
        let provider = TestProvider {
            id: policy.provider,
            manifest,
            policy,
        };
        (root, stager, provider, release, runtime)
    }

    fn sha256_bytes(bytes: &[u8]) -> String {
        let mut digest = Sha256::new();
        digest.update(bytes);
        hex_digest(digest.finalize().as_slice())
    }

    fn tar_gz(files: &[(&str, &[u8])], links: &[(&str, &str)]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, *name, *bytes).unwrap();
        }
        for (name, target) in links {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_cksum();
            builder.append_link(&mut header, *name, *target).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn zip_bytes(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(cursor);
        for (name, bytes, mode) in files {
            zip.start_file(*name, SimpleFileOptions::default().unix_permissions(*mode))
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[tokio::test]
    async fn stages_project_tar_gz_and_records_identity() {
        let name = "rust-mcp-git-v1.0.0-darwin-arm64";
        let archive = tar_gz(
            &[
                (&format!("{name}/rust-mcp-git"), b"binary"),
                (&format!("{name}/README.md"), b"readme"),
            ],
            &[],
        );
        let (_root, stager, provider, release, _) = setup(ComponentId::Git, "1.0.0", archive);
        let staged = stager.stage(&provider, &release, platform()).await.unwrap();
        assert!(staged.package_root.join("rust-mcp-git").is_file());
        let metadata_path = staged.staging_path.join(STAGED_METADATA_NAME);
        assert!(metadata_path.is_file());
        let recorded: StagedArtifact =
            serde_json::from_slice(&fs::read(metadata_path).unwrap()).unwrap();
        assert_eq!(recorded, staged);
    }

    #[tokio::test]
    async fn stages_studio_and_fleet_tar_contracts() {
        let studio_root = "mcp-studio-v0.5.0-darwin-arm64";
        let studio = tar_gz(
            &[
                (&format!("{studio_root}/mcp-studio"), b"bin"),
                (&format!("{studio_root}/web/dist/index.html"), b"html"),
            ],
            &[],
        );
        let (_root, stager, provider, release, _) = setup(ComponentId::Studio, "0.5.0", studio);
        stager.stage(&provider, &release, platform()).await.unwrap();

        let fleet_root = "mcp-fleet-v0.2.0";
        let fleet = tar_gz(
            &[
                (&format!("{fleet_root}/VERSION"), b"0.2.0\n"),
                (&format!("{fleet_root}/fleet.toml"), b"x=1"),
                (&format!("{fleet_root}/README.md"), b"r"),
                (
                    &format!("{fleet_root}/scripts/fleetctl.py"),
                    b"#!/usr/bin/env python3",
                ),
                (&format!("{fleet_root}/hosts/mirin.example.toml"), b"x=1"),
            ],
            &[],
        );
        let (_root, stager, provider, release, _) = setup(ComponentId::Fleet, "0.2.0", fleet);
        stager.stage(&provider, &release, platform()).await.unwrap();
    }

    #[tokio::test]
    async fn stages_tunnel_zip_restores_permissions_and_validates_structure() {
        let stem = "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64";
        let manifest = br#"{"version":"2026.8.2"}"#;
        let zip = zip_bytes(&[
            ("tunnel-client-runtime-cloudflared", b"runtime", 0o644),
            ("cloudflared", b"cloudflared", 0o644),
            ("cloudflared-manifest.json", manifest, 0o644),
            ("LICENSE", b"l", 0o644),
            ("NOTICE", b"n", 0o644),
            (&format!("{stem}-licenses.txt"), b"licenses", 0o644),
            (&format!("{stem}.spdx.json"), b"{}", 0o644),
        ]);
        let (_root, stager, provider, release, _) = setup(ComponentId::Tunnel, "0.0.14", zip);
        let staged = stager.stage(&provider, &release, platform()).await.unwrap();
        assert_eq!(
            staged.companion_asset_name.as_deref(),
            Some("tunnel-client-v0.0.14-darwin-arm64.zip")
        );
        assert!(staged.package_root.join(TUNNEL_FULL_ASSET_PREFIX).is_file());
        assert_eq!(staged.validated_executables.len(), 3);
        assert_eq!(
            stager
                .load_ready(&stager.staged_id(&staged).unwrap())
                .unwrap(),
            staged
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&staged.validated_executables[0])
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0o111
            );
        }
    }

    #[tokio::test]
    async fn tunnel_stage_rejects_missing_or_unchecked_full_client() {
        let stem = "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64";
        let archive = zip_bytes(&[
            ("tunnel-client-runtime-cloudflared", b"runtime", 0o644),
            ("cloudflared", b"cloudflared", 0o644),
            (
                "cloudflared-manifest.json",
                br#"{"version":"2026.8.2"}"#,
                0o644,
            ),
            ("LICENSE", b"l", 0o644),
            ("NOTICE", b"n", 0o644),
            (&format!("{stem}-licenses.txt"), b"licenses", 0o644),
            (&format!("{stem}.spdx.json"), b"{}", 0o644),
        ]);
        let (_root, stager, mut provider, mut release, _) =
            setup(ComponentId::Tunnel, "0.0.14", archive);
        release
            .assets
            .retain(|asset| !asset.name.starts_with("tunnel-client-v"));
        assert!(matches!(
            stager.stage(&provider, &release, platform()).await.unwrap_err(),
            StudioError::ReleaseAssetNotFound { component, expected }
                if component == "tunnel" && expected == "tunnel-client-v0.0.14-darwin-arm64.zip"
        ));

        let full_name = "tunnel-client-v0.0.14-darwin-arm64.zip";
        release.assets.push(ReleaseAsset {
            name: full_name.into(),
            download_url: "https://github.com/openai/tunnel-client/releases/download/v0.0.14/tunnel-client-v0.0.14-darwin-arm64.zip".into(),
        });
        provider.manifest = provider
            .manifest
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.ends_with(full_name.as_bytes()))
            .flat_map(|line| [line, b"\n".as_slice()])
            .flatten()
            .copied()
            .collect();
        assert!(matches!(
            stager.stage(&provider, &release, platform()).await.unwrap_err(),
            StudioError::ChecksumEntryMissing { component, asset }
                if component == "tunnel" && asset == full_name
        ));
    }

    #[test]
    fn tunnel_version_identity_normalizes_both_official_output_forms() {
        let runtime = parse_version_output(
            Path::new("runtime"),
            "0.0.14 git sha: 0f870e50a973fa820d4c409000059e181e8d242b flavor=runtime-cloudflared",
        )
        .unwrap();
        let full = parse_version_output(
            Path::new("full"),
            "0.0.14+0f870e50a973fa820d4c409000059e181e8d242b",
        )
        .unwrap();
        assert_eq!(runtime.version, full.version);
        assert_eq!(runtime.git_sha, full.git_sha);
    }

    #[tokio::test]
    async fn checksum_mismatch_and_missing_entry_cleanup_partial_and_preserve_active_runtime() {
        let name = "rust-mcp-git-v1.0.0-darwin-arm64";
        let archive = tar_gz(&[(&format!("{name}/rust-mcp-git"), b"binary")], &[]);
        let (_root, stager, mut provider, release, runtime) =
            setup(ComponentId::Git, "1.0.0", archive);
        let active = runtime.join("git/current");
        fs::create_dir_all(active.parent().unwrap()).unwrap();
        fs::write(&active, b"active").unwrap();
        provider.manifest =
            format!("{}  {}\n", "0".repeat(64), release.assets[0].name).into_bytes();
        assert!(matches!(
            stager
                .stage(&provider, &release, platform())
                .await
                .unwrap_err(),
            StudioError::ChecksumMismatch { .. }
        ));
        assert_eq!(fs::read(&active).unwrap(), b"active");
        assert!(!fs::read_dir(stager.staging_root()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".partial-")
        }));
        provider.manifest = format!("{}  other.tar.gz\n", "0".repeat(64)).into_bytes();
        assert!(matches!(
            stager
                .stage(&provider, &release, platform())
                .await
                .unwrap_err(),
            StudioError::ChecksumEntryMissing { .. }
        ));
        assert_eq!(fs::read(&active).unwrap(), b"active");
    }

    #[tokio::test]
    async fn truncated_archive_missing_executable_and_wrong_studio_layout_fail_closed() {
        let (_root, stager, provider, release, _) =
            setup(ComponentId::Git, "1.0.0", b"not-a-tar".to_vec());
        assert!(stager.stage(&provider, &release, platform()).await.is_err());

        let root = "rust-mcp-git-v1.0.0-darwin-arm64";
        let archive = tar_gz(&[(&format!("{root}/README.md"), b"r")], &[]);
        let (_root, stager, provider, release, _) = setup(ComponentId::Git, "1.0.0", archive);
        assert!(matches!(
            stager
                .stage(&provider, &release, platform())
                .await
                .unwrap_err(),
            StudioError::PackageValidationFailed { .. }
        ));

        let root = "mcp-studio-v0.5.0-darwin-arm64";
        let archive = tar_gz(&[(&format!("{root}/mcp-studio"), b"bin")], &[]);
        let (_root, stager, provider, release, _) = setup(ComponentId::Studio, "0.5.0", archive);
        assert!(matches!(
            stager
                .stage(&provider, &release, platform())
                .await
                .unwrap_err(),
            StudioError::PackageValidationFailed { .. }
        ));
    }

    #[tokio::test]
    async fn rejects_traversal_absolute_duplicate_and_link_entries() {
        for name in ["../escape", "/absolute"] {
            let zip = zip_bytes(&[(name, b"evil", 0o644)]);
            let (_root, stager, provider, release, _) = setup(ComponentId::Tunnel, "0.0.14", zip);
            assert!(matches!(
                stager
                    .stage(&provider, &release, platform())
                    .await
                    .unwrap_err(),
                StudioError::UnsafeArchive { .. }
            ));
        }

        let duplicate_root = "rust-mcp-git-v1.0.0-darwin-arm64";
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for bytes in [b"one".as_slice(), b"two".as_slice()] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{duplicate_root}/rust-mcp-git"), bytes)
                .unwrap();
        }
        let duplicate = builder.into_inner().unwrap().finish().unwrap();
        let (_root, stager, provider, release, _) = setup(ComponentId::Git, "1.0.0", duplicate);
        assert!(matches!(
            stager
                .stage(&provider, &release, platform())
                .await
                .unwrap_err(),
            StudioError::UnsafeArchive { .. }
        ));

        let root = "rust-mcp-git-v1.0.0-darwin-arm64";
        for hard_link in [false, true] {
            let encoder = GzEncoder::new(Vec::new(), Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{root}/rust-mcp-git"), &b"bin"[..])
                .unwrap();
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(if hard_link {
                tar::EntryType::Link
            } else {
                tar::EntryType::Symlink
            });
            link.set_size(0);
            link.set_mode(0o777);
            link.set_cksum();
            builder
                .append_link(&mut link, format!("{root}/escape"), "../../outside")
                .unwrap();
            let archive = builder.into_inner().unwrap().finish().unwrap();
            let (_root, stager, provider, release, _) = setup(ComponentId::Git, "1.0.0", archive);
            assert!(matches!(
                stager
                    .stage(&provider, &release, platform())
                    .await
                    .unwrap_err(),
                StudioError::UnsafeArchive { .. }
            ));
        }
    }

    #[tokio::test]
    #[ignore = "explicit real-network smoke against the official OpenAI release"]
    async fn live_official_tunnel_staging_smoke() {
        use crate::update::{HostPlatform, OpenAiTunnelReleaseProvider};

        let root = TempDir::new().unwrap();
        let runtime = root.path().join("runtime");
        let bin = root.path().join("bin");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let catalog = ComponentCatalog::new(HostRuntimeRoots::new(bin, runtime).unwrap());
        let provider = OpenAiTunnelReleaseProvider::new(catalog.clone()).unwrap();
        let tunnel = catalog.component(ComponentId::Tunnel).unwrap().clone();
        let release = provider.latest_release(&tunnel).await.unwrap();
        let stager = ArtifactStager::new(catalog).unwrap();
        let staged = stager
            .stage(
                &provider,
                &release,
                HostPlatform::detect().unwrap().platform(),
            )
            .await
            .unwrap();
        assert_eq!(staged.component, ComponentId::Tunnel);
        assert!(staged.validated_executables[0].is_file());
    }

    #[test]
    fn stale_partial_directories_are_cleaned_on_stager_construction() {
        let root = TempDir::new().unwrap();
        let runtime = root.path().join("runtime");
        let bin = root.path().join("bin");
        fs::create_dir_all(runtime.join(STAGING_DIR_NAME).join(".partial-stale")).unwrap();
        let catalog = ComponentCatalog::new(HostRuntimeRoots::new(bin, runtime.clone()).unwrap());
        let _ = ArtifactStager::with_dependencies(
            catalog,
            Arc::new(FakeFetcher::default()),
            Arc::new(FakeVersionValidator),
        )
        .unwrap();
        assert!(
            !runtime
                .join(STAGING_DIR_NAME)
                .join(".partial-stale")
                .exists()
        );
    }
}
