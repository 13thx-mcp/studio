use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Url;
use serde::Deserialize;

use crate::error::{StudioError, StudioResult};

use super::{
    AvailableRelease, ComponentCatalog, ComponentPolicy, Platform, ReleaseAsset, ReleaseProvider,
    ReleaseProviderId, Version,
    github_http::{
        BINARY_ACCEPT, GITHUB_JSON_ACCEPT, GitHubHttp, MAX_CHECKSUM_MANIFEST_BYTES,
        MAX_RELEASE_METADATA_BYTES, build_github_http, map_transport_error,
    },
    select_release_asset,
};

const API_BASE: &str = "https://api.github.com";
const TRUSTED_OWNER: &str = "13thx-mcp";
const CHECKSUM_MANIFEST_NAME: &str = "SHA256SUMS.txt";

#[derive(Clone)]
pub struct ThirteenthXReleaseProvider {
    catalog: ComponentCatalog,
    http: Arc<dyn GitHubHttp>,
}

impl ThirteenthXReleaseProvider {
    pub fn new(catalog: ComponentCatalog) -> StudioResult<Self> {
        Ok(Self {
            catalog,
            http: build_github_http("github_13thx")?,
        })
    }

    #[cfg(test)]
    fn with_http(catalog: ComponentCatalog, http: Arc<dyn GitHubHttp>) -> Self {
        Self { catalog, http }
    }

    fn trusted_policy(&self, supplied: &ComponentPolicy) -> StudioResult<&ComponentPolicy> {
        let trusted = self.catalog.component(supplied.id)?;
        if trusted.provider != ReleaseProviderId::ThirteenthXGitHub
            || trusted.source.owner != TRUSTED_OWNER
        {
            return Err(StudioError::Unsupported(format!(
                "component {} is not managed by the 13thx GitHub release provider",
                supplied.id
            )));
        }
        Ok(trusted)
    }

    fn trusted_policy_by_release(
        &self,
        release: &AvailableRelease,
    ) -> StudioResult<&ComponentPolicy> {
        let trusted = self.catalog.component(release.component)?;
        if trusted.provider != ReleaseProviderId::ThirteenthXGitHub
            || trusted.source.owner != TRUSTED_OWNER
        {
            return Err(StudioError::Unsupported(format!(
                "component {} is not managed by the 13thx GitHub release provider",
                release.component
            )));
        }
        Ok(trusted)
    }

    fn latest_url(policy: &ComponentPolicy) -> String {
        format!(
            "{API_BASE}/repos/{}/{}/releases/latest",
            policy.source.owner, policy.source.repository
        )
    }

    fn release_url(policy: &ComponentPolicy, version: &Version) -> String {
        format!(
            "{API_BASE}/repos/{}/{}/releases/tags/v{version}",
            policy.source.owner, policy.source.repository
        )
    }

    async fn fetch_release(
        &self,
        policy: &ComponentPolicy,
        url: &str,
        lookup: ReleaseLookup<'_>,
    ) -> StudioResult<AvailableRelease> {
        let component = policy.id.to_string();
        let response = self
            .http
            .get(url, GITHUB_JSON_ACCEPT, MAX_RELEASE_METADATA_BYTES)
            .await
            .map_err(|error| map_transport_error(&component, error))?;

        if response.status == 404 {
            return match lookup {
                ReleaseLookup::Latest => Err(StudioError::NoStableRelease { component }),
                ReleaseLookup::Explicit(version) => Err(StudioError::ReleaseVersionNotFound {
                    component,
                    version: version.to_string(),
                }),
            };
        }
        if !(200..300).contains(&response.status) {
            return Err(StudioError::ReleaseProviderHttp {
                component,
                status: response.status,
            });
        }

        let raw: GitHubRelease = serde_json::from_slice(&response.body).map_err(|error| {
            StudioError::MalformedReleaseMetadata {
                component: policy.id.to_string(),
                detail: error.to_string(),
            }
        })?;

        self.convert_release(policy, raw, lookup)
    }

    fn convert_release(
        &self,
        policy: &ComponentPolicy,
        raw: GitHubRelease,
        lookup: ReleaseLookup<'_>,
    ) -> StudioResult<AvailableRelease> {
        let component = policy.id.to_string();
        if raw.id == 0 {
            return Err(StudioError::MalformedReleaseMetadata {
                component,
                detail: "release id must be non-zero".into(),
            });
        }
        if !raw.tag_name.starts_with('v') {
            return Err(StudioError::InvalidReleaseTag {
                component,
                tag: raw.tag_name,
                detail: "project release tags must use the vMAJOR.MINOR.PATCH convention".into(),
            });
        }

        let version =
            Version::parse_tag(&raw.tag_name).map_err(|error| StudioError::InvalidReleaseTag {
                component: policy.id.to_string(),
                tag: raw.tag_name.clone(),
                detail: error.to_string(),
            })?;

        let stable = !raw.draft && !raw.prerelease && version.as_semver().pre.is_empty();
        if !stable {
            return match lookup {
                ReleaseLookup::Latest => Err(StudioError::NoStableRelease {
                    component: policy.id.to_string(),
                }),
                ReleaseLookup::Explicit(_) => Err(StudioError::ReleaseNotEligible {
                    component: policy.id.to_string(),
                    tag: raw.tag_name,
                    reason: "draft and prerelease releases are excluded from the stable channel"
                        .into(),
                }),
            };
        }

        if let ReleaseLookup::Explicit(requested) = lookup
            && &version != requested
        {
            return Err(StudioError::MalformedReleaseMetadata {
                component: policy.id.to_string(),
                detail: format!(
                    "requested v{requested} but GitHub returned tag {}",
                    raw.tag_name
                ),
            });
        }

        let mut checksum_url: Option<String> = None;
        let mut assets = Vec::with_capacity(raw.assets.len());
        for asset in raw.assets {
            if asset.id == 0 || asset.name.is_empty() {
                return Err(StudioError::MalformedReleaseMetadata {
                    component: policy.id.to_string(),
                    detail: "release asset id must be non-zero and name must not be empty".into(),
                });
            }
            self.validate_asset_url(policy, &asset.browser_download_url)?;
            if asset.name == CHECKSUM_MANIFEST_NAME {
                if checksum_url.is_some() {
                    return Err(StudioError::DuplicateChecksumManifest {
                        component: policy.id.to_string(),
                    });
                }
                checksum_url = Some(asset.browser_download_url.clone());
            }
            assets.push(ReleaseAsset {
                name: asset.name,
                download_url: asset.browser_download_url,
            });
        }

        let checksum_manifest_url =
            checksum_url.ok_or_else(|| StudioError::MissingChecksumManifest {
                component: policy.id.to_string(),
            })?;

        Ok(AvailableRelease {
            component: policy.id,
            version,
            tag: raw.tag_name,
            assets,
            checksum_manifest_url,
        })
    }

    fn validate_asset_url(&self, policy: &ComponentPolicy, value: &str) -> StudioResult<()> {
        let untrusted = || StudioError::UntrustedReleaseAssetUrl {
            component: policy.id.to_string(),
            url: value.to_owned(),
        };
        let url = Url::parse(value).map_err(|_| untrusted())?;
        let trusted_prefix = format!(
            "/{}/{}/releases/download/",
            policy.source.owner, policy.source.repository
        );
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || !url.path().starts_with(&trusted_prefix)
        {
            return Err(untrusted());
        }
        Ok(())
    }
}

#[async_trait]
impl ReleaseProvider for ThirteenthXReleaseProvider {
    fn provider_id(&self) -> ReleaseProviderId {
        ReleaseProviderId::ThirteenthXGitHub
    }

    async fn latest_release(&self, component: &ComponentPolicy) -> StudioResult<AvailableRelease> {
        let trusted = self.trusted_policy(component)?;
        let url = Self::latest_url(trusted);
        self.fetch_release(trusted, &url, ReleaseLookup::Latest)
            .await
    }

    async fn release(
        &self,
        component: &ComponentPolicy,
        version: &Version,
    ) -> StudioResult<AvailableRelease> {
        let trusted = self.trusted_policy(component)?;
        if !version.as_semver().pre.is_empty() {
            return Err(StudioError::ReleaseNotEligible {
                component: trusted.id.to_string(),
                tag: format!("v{version}"),
                reason: "the stable channel does not resolve prerelease versions".into(),
            });
        }
        let url = Self::release_url(trusted, version);
        self.fetch_release(trusted, &url, ReleaseLookup::Explicit(version))
            .await
    }

    fn select_asset<'a>(
        &self,
        release: &'a AvailableRelease,
        platform: Platform,
    ) -> StudioResult<&'a ReleaseAsset> {
        let trusted = self.trusted_policy_by_release(release)?;
        select_release_asset(trusted, release, platform)
    }

    async fn checksum_manifest(&self, release: &AvailableRelease) -> StudioResult<Vec<u8>> {
        let trusted = self.trusted_policy_by_release(release)?;
        self.validate_asset_url(trusted, &release.checksum_manifest_url)?;
        let response = self
            .http
            .get(
                &release.checksum_manifest_url,
                BINARY_ACCEPT,
                MAX_CHECKSUM_MANIFEST_BYTES,
            )
            .await
            .map_err(|error| map_transport_error(&trusted.id.to_string(), error))?;
        if !(200..300).contains(&response.status) {
            return Err(StudioError::ReleaseProviderHttp {
                component: trusted.id.to_string(),
                status: response.status,
            });
        }
        Ok(response.body)
    }
}

#[derive(Clone, Copy)]
enum ReleaseLookup<'a> {
    Latest,
    Explicit(&'a Version),
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    id: u64,
    name: String,
    browser_download_url: String,
}

#[cfg(test)]
use super::github_http::{HttpResponse, HttpTransportError};

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        path::PathBuf,
        sync::Mutex,
    };

    use serde_json::json;

    use super::*;
    use crate::update::{ComponentId, HostRuntimeRoots, ReleaseSource};

    #[derive(Default)]
    struct FakeHttp {
        responses: Mutex<HashMap<String, VecDeque<Result<HttpResponse, HttpTransportError>>>>,
        requests: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        fn respond(&self, url: String, response: HttpResponse) {
            self.responses
                .lock()
                .unwrap()
                .entry(url)
                .or_default()
                .push_back(Ok(response));
        }

        fn fail(&self, url: String, error: HttpTransportError) {
            self.responses
                .lock()
                .unwrap()
                .entry(url)
                .or_default()
                .push_back(Err(error));
        }

        fn requested_urls(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl GitHubHttp for FakeHttp {
        async fn get(
            &self,
            url: &str,
            _accept: &'static str,
            _max_bytes: usize,
        ) -> Result<HttpResponse, HttpTransportError> {
            self.requests.lock().unwrap().push(url.to_owned());
            self.responses
                .lock()
                .unwrap()
                .get_mut(url)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| panic!("unexpected HTTP request: {url}"))
        }
    }

    fn catalog() -> ComponentCatalog {
        ComponentCatalog::new(
            HostRuntimeRoots::new(
                PathBuf::from("/trusted/bin"),
                PathBuf::from("/trusted/runtime"),
            )
            .unwrap(),
        )
    }

    fn provider() -> (ThirteenthXReleaseProvider, Arc<FakeHttp>) {
        let http = Arc::new(FakeHttp::default());
        (
            ThirteenthXReleaseProvider::with_http(catalog(), http.clone()),
            http,
        )
    }

    fn policy(catalog: &ComponentCatalog, id: ComponentId) -> ComponentPolicy {
        catalog.component(id).unwrap().clone()
    }

    fn asset_url(repository: &str, tag: &str, name: &str) -> String {
        format!("https://github.com/13thx-mcp/{repository}/releases/download/{tag}/{name}")
    }

    fn release_body(repository: &str, tag: &str, draft: bool, prerelease: bool) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "id": 42,
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "assets": [
                {
                    "id": 101,
                    "name": format!("rust-mcp-{repository}-{tag}-darwin-amd64.tar.gz"),
                    "browser_download_url": asset_url(repository, tag, &format!("rust-mcp-{repository}-{tag}-darwin-amd64.tar.gz"))
                },
                {
                    "id": 102,
                    "name": format!("rust-mcp-{repository}-{tag}-darwin-arm64.tar.gz"),
                    "browser_download_url": asset_url(repository, tag, &format!("rust-mcp-{repository}-{tag}-darwin-arm64.tar.gz"))
                },
                {
                    "id": 103,
                    "name": CHECKSUM_MANIFEST_NAME,
                    "browser_download_url": asset_url(repository, tag, CHECKSUM_MANIFEST_NAME)
                }
            ]
        }))
        .unwrap()
    }

    fn ok(body: Vec<u8>) -> HttpResponse {
        HttpResponse { status: 200, body }
    }

    #[tokio::test]
    async fn latest_stable_release_parses_and_preserves_assets() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Git);
        let url = ThirteenthXReleaseProvider::latest_url(&component);
        http.respond(url, ok(release_body("git", "v1.2.3", false, false)));

        let release = provider.latest_release(&component).await.unwrap();
        assert_eq!(release.component, ComponentId::Git);
        assert_eq!(release.version.to_string(), "1.2.3");
        assert_eq!(release.tag, "v1.2.3");
        assert_eq!(release.assets.len(), 3);
        assert_eq!(
            release.assets[0].name,
            "rust-mcp-git-v1.2.3-darwin-amd64.tar.gz"
        );
        assert_eq!(
            release.assets[1].name,
            "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz"
        );
        assert_eq!(release.assets[2].name, CHECKSUM_MANIFEST_NAME);
        assert_eq!(
            release.checksum_manifest_url,
            asset_url("git", "v1.2.3", CHECKSUM_MANIFEST_NAME)
        );
    }

    #[tokio::test]
    async fn explicit_version_lookup_uses_validated_v_tag() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Filesystem);
        let version = Version::parse("0.1.0").unwrap();
        let url = ThirteenthXReleaseProvider::release_url(&component, &version);
        http.respond(
            url.clone(),
            ok(release_body("filesystem", "v0.1.0", false, false)),
        );

        let release = provider.release(&component, &version).await.unwrap();
        assert_eq!(release.version, version);
        assert_eq!(http.requested_urls(), vec![url]);
    }

    #[tokio::test]
    async fn stable_channel_rejects_draft_and_prerelease_metadata() {
        for (tag, draft, prerelease) in [("v1.0.0", true, false), ("v1.1.0-rc.1", false, true)] {
            let (provider, http) = provider();
            let catalog = catalog();
            let component = policy(&catalog, ComponentId::Exec);
            let url = ThirteenthXReleaseProvider::latest_url(&component);
            http.respond(url, ok(release_body("exec", tag, draft, prerelease)));
            assert!(matches!(
                provider.latest_release(&component).await.unwrap_err(),
                StudioError::NoStableRelease { component } if component == "exec"
            ));
        }

        let (provider, _) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Exec);
        let prerelease = Version::parse("1.1.0-rc.1").unwrap();
        assert!(matches!(
            provider.release(&component, &prerelease).await.unwrap_err(),
            StudioError::ReleaseNotEligible { component, .. } if component == "exec"
        ));
    }

    #[tokio::test]
    async fn missing_and_duplicate_checksum_manifest_are_distinct() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Gateway);
        let url = ThirteenthXReleaseProvider::latest_url(&component);
        let missing = json!({
            "id": 1,
            "tag_name": "v1.0.0",
            "draft": false,
            "prerelease": false,
            "assets": [{
                "id": 2,
                "name": "rust-mcp-gateway-v1.0.0-darwin-arm64.tar.gz",
                "browser_download_url": asset_url("gateway", "v1.0.0", "rust-mcp-gateway-v1.0.0-darwin-arm64.tar.gz")
            }]
        });
        http.respond(url.clone(), ok(serde_json::to_vec(&missing).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::MissingChecksumManifest { component } if component == "gateway"
        ));

        let duplicate = json!({
            "id": 1,
            "tag_name": "v1.0.0",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"id": 2, "name": CHECKSUM_MANIFEST_NAME, "browser_download_url": asset_url("gateway", "v1.0.0", CHECKSUM_MANIFEST_NAME)},
                {"id": 3, "name": CHECKSUM_MANIFEST_NAME, "browser_download_url": asset_url("gateway", "v1.0.0", CHECKSUM_MANIFEST_NAME)}
            ]
        });
        http.respond(url, ok(serde_json::to_vec(&duplicate).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::DuplicateChecksumManifest { component } if component == "gateway"
        ));
    }

    #[tokio::test]
    async fn malformed_json_and_invalid_tag_are_distinct() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Studio);
        let url = ThirteenthXReleaseProvider::latest_url(&component);
        http.respond(url.clone(), ok(b"not-json".to_vec()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::MalformedReleaseMetadata { component, .. } if component == "studio"
        ));

        let invalid = release_body("studio", "release-1.0.0", false, false);
        http.respond(url, ok(invalid));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::InvalidReleaseTag { component, .. } if component == "studio"
        ));
    }

    #[tokio::test]
    async fn transport_timeout_http_status_and_not_found_are_mapped() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Fleet);
        let url = ThirteenthXReleaseProvider::latest_url(&component);
        http.fail(
            url.clone(),
            HttpTransportError::Unreachable("request timed out".into()),
        );
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::ReleaseProviderUnreachable { component, .. } if component == "fleet"
        ));

        http.respond(
            url.clone(),
            HttpResponse {
                status: 503,
                body: Vec::new(),
            },
        );
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::ReleaseProviderHttp { component, status: 503 } if component == "fleet"
        ));

        http.respond(
            url,
            HttpResponse {
                status: 404,
                body: Vec::new(),
            },
        );
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::NoStableRelease { component } if component == "fleet"
        ));
    }

    #[tokio::test]
    async fn explicit_version_not_found_is_distinct_from_no_stable_release() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Git);
        let version = Version::parse("9.9.9").unwrap();
        let url = ThirteenthXReleaseProvider::release_url(&component, &version);
        http.respond(
            url,
            HttpResponse {
                status: 404,
                body: Vec::new(),
            },
        );
        assert!(matches!(
            provider.release(&component, &version).await.unwrap_err(),
            StudioError::ReleaseVersionNotFound { component, version } if component == "git" && version == "9.9.9"
        ));
    }

    #[tokio::test]
    async fn provider_uses_catalog_repository_even_if_supplied_policy_is_forged() {
        let (provider, http) = provider();
        let catalog = catalog();
        let mut forged = policy(&catalog, ComponentId::Git);
        forged.source = ReleaseSource {
            owner: "attacker",
            repository: "payloads",
        };
        let trusted = policy(&catalog, ComponentId::Git);
        let url = ThirteenthXReleaseProvider::latest_url(&trusted);
        http.respond(url.clone(), ok(release_body("git", "v1.0.0", false, false)));

        provider.latest_release(&forged).await.unwrap();
        assert_eq!(http.requested_urls(), vec![url]);
    }

    #[tokio::test]
    async fn provider_rejects_non_13thx_catalog_component() {
        let (provider, _) = provider();
        let catalog = catalog();
        let tunnel = policy(&catalog, ComponentId::Tunnel);
        assert!(matches!(
            provider.latest_release(&tunnel).await.unwrap_err(),
            StudioError::Unsupported(message) if message.contains("tunnel")
        ));
    }

    #[tokio::test]
    async fn untrusted_asset_url_is_rejected_before_it_can_reach_staging() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Git);
        let url = ThirteenthXReleaseProvider::latest_url(&component);
        let body = json!({
            "id": 1,
            "tag_name": "v1.0.0",
            "draft": false,
            "prerelease": false,
            "assets": [{
                "id": 2,
                "name": CHECKSUM_MANIFEST_NAME,
                "browser_download_url": "https://example.invalid/SHA256SUMS.txt"
            }]
        });
        http.respond(url, ok(serde_json::to_vec(&body).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::UntrustedReleaseAssetUrl { component, .. } if component == "git"
        ));
    }

    #[tokio::test]
    async fn checksum_manifest_fetch_is_bounded_metadata_only_retrieval() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Git);
        let latest_url = ThirteenthXReleaseProvider::latest_url(&component);
        http.respond(latest_url, ok(release_body("git", "v1.0.0", false, false)));
        let release = provider.latest_release(&component).await.unwrap();
        let manifest = b"abc123  rust-mcp-git-v1.0.0-darwin-arm64.tar.gz\n".to_vec();
        http.respond(release.checksum_manifest_url.clone(), ok(manifest.clone()));

        assert_eq!(
            provider.checksum_manifest(&release).await.unwrap(),
            manifest
        );
    }

    #[tokio::test]
    async fn oversized_response_is_mapped_without_parsing() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = policy(&catalog, ComponentId::Git);
        let url = ThirteenthXReleaseProvider::latest_url(&component);
        http.fail(
            url,
            HttpTransportError::ResponseTooLarge {
                limit: MAX_RELEASE_METADATA_BYTES,
            },
        );
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::ReleaseProviderResponseTooLarge { component, limit }
                if component == "git" && limit == MAX_RELEASE_METADATA_BYTES
        ));
    }

    #[tokio::test]
    async fn latest_release_is_queryable_for_every_project_owned_catalog_component() {
        let (provider, http) = provider();
        let catalog = catalog();
        for id in [
            ComponentId::Filesystem,
            ComponentId::Git,
            ComponentId::Exec,
            ComponentId::Gateway,
            ComponentId::Blender,
            ComponentId::Studio,
            ComponentId::Fleet,
        ] {
            let component = policy(&catalog, id);
            let url = ThirteenthXReleaseProvider::latest_url(&component);
            http.respond(
                url,
                ok(release_body(
                    component.source.repository,
                    "v1.0.0",
                    false,
                    false,
                )),
            );
            let release = provider.latest_release(&component).await.unwrap();
            assert_eq!(release.component, id);
            assert_eq!(release.version.to_string(), "1.0.0");
        }
    }

    #[test]
    fn provider_select_asset_uses_shared_trusted_platform_resolver() {
        let (provider, _) = provider();
        let release = AvailableRelease {
            component: ComponentId::Git,
            version: Version::parse("1.2.3").unwrap(),
            tag: "v1.2.3".into(),
            assets: vec![
                ReleaseAsset {
                    name: "rust-mcp-git-v1.2.3-darwin-amd64.tar.gz".into(),
                    download_url: asset_url(
                        "git",
                        "v1.2.3",
                        "rust-mcp-git-v1.2.3-darwin-amd64.tar.gz",
                    ),
                },
                ReleaseAsset {
                    name: "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz".into(),
                    download_url: asset_url(
                        "git",
                        "v1.2.3",
                        "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz",
                    ),
                },
            ],
            checksum_manifest_url: asset_url("git", "v1.2.3", CHECKSUM_MANIFEST_NAME),
        };
        let selected = provider
            .select_asset(
                &release,
                Platform {
                    os: crate::update::OperatingSystem::Darwin,
                    arch: crate::update::Architecture::Arm64,
                },
            )
            .unwrap();
        assert_eq!(selected.name, "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz");
    }

    #[test]
    fn catalog_maps_all_project_owned_components_to_trusted_13thx_repositories() {
        let catalog = catalog();
        for id in [
            ComponentId::Filesystem,
            ComponentId::Git,
            ComponentId::Exec,
            ComponentId::Gateway,
            ComponentId::Blender,
            ComponentId::Studio,
            ComponentId::Fleet,
        ] {
            let component = catalog.component(id).unwrap();
            assert_eq!(component.provider, ReleaseProviderId::ThirteenthXGitHub);
            assert_eq!(component.source.owner, TRUSTED_OWNER);
            assert_eq!(component.source.repository, id.as_str());
        }
    }
}
