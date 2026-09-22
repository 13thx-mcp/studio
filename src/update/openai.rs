use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use reqwest::Url;
use serde::Deserialize;

use crate::error::{StudioError, StudioResult};

use super::{
    AvailableRelease, ComponentCatalog, ComponentId, ComponentPolicy, Platform, ReleaseAsset,
    ReleaseProvider, ReleaseProviderId, Version,
    github_http::{
        BINARY_ACCEPT, GITHUB_JSON_ACCEPT, GitHubHttp, MAX_CHECKSUM_MANIFEST_BYTES,
        MAX_RELEASE_METADATA_BYTES, build_github_http, map_transport_error,
    },
    select_release_asset,
};

const API_BASE: &str = "https://api.github.com";
const TRUSTED_OWNER: &str = "openai";
const TRUSTED_REPOSITORY: &str = "tunnel-client";
const CHECKSUM_MANIFEST_NAME: &str = "SHA256SUMS.txt";

pub const TUNNEL_RUNTIME_ASSET_PREFIX: &str = "tunnel-client-runtime-cloudflared";
pub const TUNNEL_RUNTIME_BINARY_NAME: &str = "tunnel-client-runtime-cloudflared";
pub const TUNNEL_FULL_ASSET_PREFIX: &str = "tunnel-client";

#[derive(Clone)]
pub struct OpenAiTunnelReleaseProvider {
    catalog: ComponentCatalog,
    http: Arc<dyn GitHubHttp>,
}

impl OpenAiTunnelReleaseProvider {
    pub fn new(catalog: ComponentCatalog) -> StudioResult<Self> {
        Ok(Self {
            catalog,
            http: build_github_http("github_openai")?,
        })
    }

    #[cfg(test)]
    fn with_http(catalog: ComponentCatalog, http: Arc<dyn GitHubHttp>) -> Self {
        Self { catalog, http }
    }

    fn trusted_policy(&self, supplied: &ComponentPolicy) -> StudioResult<&ComponentPolicy> {
        let trusted = self.catalog.component(supplied.id)?;
        if trusted.id != ComponentId::Tunnel
            || trusted.provider != ReleaseProviderId::OpenAiGitHub
            || trusted.source.owner != TRUSTED_OWNER
            || trusted.source.repository != TRUSTED_REPOSITORY
        {
            return Err(StudioError::Unsupported(format!(
                "component {} is not managed by the official OpenAI tunnel release provider",
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
        if trusted.id != ComponentId::Tunnel
            || trusted.provider != ReleaseProviderId::OpenAiGitHub
            || trusted.source.owner != TRUSTED_OWNER
            || trusted.source.repository != TRUSTED_REPOSITORY
        {
            return Err(StudioError::Unsupported(format!(
                "component {} is not managed by the official OpenAI tunnel release provider",
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
                detail: "official tunnel-client tags must use the vMAJOR.MINOR.PATCH convention"
                    .into(),
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
        let runtime_targets = self.validate_platform_asset_family(
            policy,
            &version,
            &assets,
            TUNNEL_RUNTIME_ASSET_PREFIX,
        )?;
        let full_targets = self.validate_platform_asset_family(
            policy,
            &version,
            &assets,
            TUNNEL_FULL_ASSET_PREFIX,
        )?;
        if runtime_targets != full_targets {
            return Err(StudioError::MalformedReleaseMetadata {
                component: policy.id.to_string(),
                detail: "full-client and runtime-cloudflared platform assets differ".into(),
            });
        }

        Ok(AvailableRelease {
            component: policy.id,
            version,
            tag: raw.tag_name,
            assets,
            checksum_manifest_url,
        })
    }

    fn validate_platform_asset_family(
        &self,
        policy: &ComponentPolicy,
        version: &Version,
        assets: &[ReleaseAsset],
        prefix: &str,
    ) -> StudioResult<BTreeSet<String>> {
        let expected_prefix = format!("{prefix}-v{version}-");
        let generic_prefix = format!("{prefix}-v");
        let mut targets = BTreeSet::new();

        for asset in assets {
            if !asset.name.starts_with(&generic_prefix) || !asset.name.ends_with(".zip") {
                continue;
            }
            let Some(target) = asset
                .name
                .strip_prefix(&expected_prefix)
                .and_then(|value| value.strip_suffix(".zip"))
            else {
                return Err(StudioError::MalformedReleaseMetadata {
                    component: policy.id.to_string(),
                    detail: format!(
                        "{prefix} ZIP {} does not match release v{version}",
                        asset.name
                    ),
                });
            };
            let parts = target.split('-').collect::<Vec<_>>();
            if parts.len() != 2 || parts.iter().any(|part| !valid_target_token(part)) {
                return Err(StudioError::MalformedReleaseMetadata {
                    component: policy.id.to_string(),
                    detail: format!(
                        "{prefix} ZIP {} has malformed platform identity",
                        asset.name
                    ),
                });
            }
            if !targets.insert(target.to_owned()) {
                return Err(StudioError::AmbiguousReleaseAssetFamily {
                    component: policy.id.to_string(),
                    family: prefix.into(),
                    detail: format!("duplicate platform target {target}"),
                });
            }
        }

        if targets.is_empty() {
            return Err(StudioError::MissingReleaseAssetFamily {
                component: policy.id.to_string(),
                family: prefix.into(),
            });
        }
        Ok(targets)
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

fn valid_target_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

#[async_trait]
impl ReleaseProvider for OpenAiTunnelReleaseProvider {
    fn provider_id(&self) -> ReleaseProviderId {
        ReleaseProviderId::OpenAiGitHub
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

    fn select_companion_asset<'a>(
        &self,
        release: &'a AvailableRelease,
        platform: Platform,
    ) -> StudioResult<Option<&'a ReleaseAsset>> {
        let trusted = self.trusted_policy_by_release(release)?;
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
                component: trusted.id.to_string(),
                expected,
            }),
            _ => Err(StudioError::AmbiguousReleaseAsset {
                component: trusted.id.to_string(),
                expected,
                count: matches.len(),
            }),
        }
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
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        path::PathBuf,
        sync::Mutex,
    };

    use serde_json::json;

    use super::*;
    use crate::update::{
        HostRuntimeRoots, ReleaseSource,
        github_http::{HttpResponse, HttpTransportError},
    };

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

    fn provider() -> (OpenAiTunnelReleaseProvider, Arc<FakeHttp>) {
        let http = Arc::new(FakeHttp::default());
        (
            OpenAiTunnelReleaseProvider::with_http(catalog(), http.clone()),
            http,
        )
    }

    fn tunnel_policy(catalog: &ComponentCatalog) -> ComponentPolicy {
        catalog.component(ComponentId::Tunnel).unwrap().clone()
    }

    fn asset_url(tag: &str, name: &str) -> String {
        format!("https://github.com/openai/tunnel-client/releases/download/{tag}/{name}")
    }

    fn runtime_asset(tag: &str, platform: &str) -> String {
        format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-{platform}.zip")
    }

    fn release_body(tag: &str, draft: bool, prerelease: bool) -> Vec<u8> {
        let amd64 = runtime_asset(tag, "darwin-amd64");
        let arm64 = runtime_asset(tag, "darwin-arm64");
        serde_json::to_vec(&json!({
            "id": 140014,
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "assets": [
                {"id": 1, "name": "SHA256SUMS.txt", "browser_download_url": asset_url(tag, "SHA256SUMS.txt")},
                {"id": 2, "name": amd64, "browser_download_url": asset_url(tag, &runtime_asset(tag, "darwin-amd64"))},
                {"id": 3, "name": format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-amd64-licenses.txt"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-amd64-licenses.txt"))},
                {"id": 4, "name": format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-amd64.spdx.json"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-amd64.spdx.json"))},
                {"id": 5, "name": arm64, "browser_download_url": asset_url(tag, &runtime_asset(tag, "darwin-arm64"))},
                {"id": 6, "name": format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-arm64-licenses.txt"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-arm64-licenses.txt"))},
                {"id": 7, "name": format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-arm64.spdx.json"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-{tag}-darwin-arm64.spdx.json"))},
                {"id": 8, "name": format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-source-{tag}.tar.gz"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_RUNTIME_ASSET_PREFIX}-source-{tag}.tar.gz"))},
                {"id": 9, "name": format!("{TUNNEL_FULL_ASSET_PREFIX}-{tag}-darwin-amd64.zip"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_FULL_ASSET_PREFIX}-{tag}-darwin-amd64.zip"))},
                {"id": 10, "name": format!("{TUNNEL_FULL_ASSET_PREFIX}-{tag}-darwin-arm64.zip"), "browser_download_url": asset_url(tag, &format!("{TUNNEL_FULL_ASSET_PREFIX}-{tag}-darwin-arm64.zip"))}
            ]
        }))
        .unwrap()
    }

    fn ok(body: Vec<u8>) -> HttpResponse {
        HttpResponse { status: 200, body }
    }

    #[tokio::test]
    async fn latest_stable_official_release_matches_v0_0_14_asset_family() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        http.respond(url, ok(release_body("v0.0.14", false, false)));

        let release = provider.latest_release(&component).await.unwrap();
        assert_eq!(provider.provider_id(), ReleaseProviderId::OpenAiGitHub);
        assert_eq!(release.component, ComponentId::Tunnel);
        assert_eq!(release.version.to_string(), "0.0.14");
        assert_eq!(release.tag, "v0.0.14");
        assert_eq!(release.assets.len(), 10);
        assert!(
            release
                .assets
                .iter()
                .any(|asset| asset.name == runtime_asset("v0.0.14", "darwin-amd64"))
        );
        assert!(
            release
                .assets
                .iter()
                .any(|asset| asset.name == runtime_asset("v0.0.14", "darwin-arm64"))
        );
        assert!(release.assets.iter().any(|asset| {
            asset.name == format!("{TUNNEL_FULL_ASSET_PREFIX}-v0.0.14-darwin-arm64.zip")
        }));
        assert_eq!(
            release.checksum_manifest_url,
            asset_url("v0.0.14", CHECKSUM_MANIFEST_NAME)
        );
    }

    #[tokio::test]
    async fn explicit_version_lookup_uses_official_v_tag() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let version = Version::parse("0.0.14").unwrap();
        let url = OpenAiTunnelReleaseProvider::release_url(&component, &version);
        http.respond(url.clone(), ok(release_body("v0.0.14", false, false)));

        let release = provider.release(&component, &version).await.unwrap();
        assert_eq!(release.version, version);
        assert_eq!(http.requested_urls(), vec![url]);
    }

    #[tokio::test]
    async fn prereleases_and_drafts_are_excluded_from_stable_channel() {
        for (tag, draft, prerelease) in [("v0.0.15", true, false), ("v0.0.15-dev", false, true)] {
            let (provider, http) = provider();
            let catalog = catalog();
            let component = tunnel_policy(&catalog);
            let url = OpenAiTunnelReleaseProvider::latest_url(&component);
            http.respond(url, ok(release_body(tag, draft, prerelease)));
            assert!(matches!(
                provider.latest_release(&component).await.unwrap_err(),
                StudioError::NoStableRelease { component } if component == "tunnel"
            ));
        }

        let (provider, _) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let prerelease = Version::parse("0.0.15-dev").unwrap();
        assert!(matches!(
            provider.release(&component, &prerelease).await.unwrap_err(),
            StudioError::ReleaseNotEligible { component, .. } if component == "tunnel"
        ));
    }

    #[tokio::test]
    async fn missing_checksum_manifest_is_rejected() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        let runtime = runtime_asset("v0.0.14", "darwin-arm64");
        let body = json!({
            "id": 1,
            "tag_name": "v0.0.14",
            "draft": false,
            "prerelease": false,
            "assets": [{"id": 2, "name": runtime, "browser_download_url": asset_url("v0.0.14", &runtime_asset("v0.0.14", "darwin-arm64"))}]
        });
        http.respond(url, ok(serde_json::to_vec(&body).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::MissingChecksumManifest { component } if component == "tunnel"
        ));
    }

    #[tokio::test]
    async fn missing_runtime_cloudflared_family_is_rejected() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        let body = json!({
            "id": 1,
            "tag_name": "v0.0.14",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"id": 2, "name": CHECKSUM_MANIFEST_NAME, "browser_download_url": asset_url("v0.0.14", CHECKSUM_MANIFEST_NAME)},
                {"id": 3, "name": "tunnel-client-v0.0.14-darwin-arm64.zip", "browser_download_url": asset_url("v0.0.14", "tunnel-client-v0.0.14-darwin-arm64.zip")}
            ]
        });
        http.respond(url, ok(serde_json::to_vec(&body).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::MissingReleaseAssetFamily { component, family }
                if component == "tunnel" && family == TUNNEL_RUNTIME_ASSET_PREFIX
        ));
    }

    #[tokio::test]
    async fn mismatched_full_client_platform_family_is_rejected() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        let mut body: serde_json::Value =
            serde_json::from_slice(&release_body("v0.0.14", false, false)).unwrap();
        body["assets"].as_array_mut().unwrap().retain(|asset| {
            asset["name"] != format!("{TUNNEL_FULL_ASSET_PREFIX}-v0.0.14-darwin-amd64.zip")
        });
        http.respond(url, ok(serde_json::to_vec(&body).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::MalformedReleaseMetadata { component, .. } if component == "tunnel"
        ));
    }

    #[tokio::test]
    async fn mismatched_or_ambiguous_runtime_assets_are_rejected() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        let wrong = runtime_asset("v0.0.13", "darwin-arm64");
        let malformed = json!({
            "id": 1,
            "tag_name": "v0.0.14",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"id": 2, "name": CHECKSUM_MANIFEST_NAME, "browser_download_url": asset_url("v0.0.14", CHECKSUM_MANIFEST_NAME)},
                {"id": 3, "name": wrong, "browser_download_url": asset_url("v0.0.14", &runtime_asset("v0.0.13", "darwin-arm64"))}
            ]
        });
        http.respond(url.clone(), ok(serde_json::to_vec(&malformed).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::MalformedReleaseMetadata { component, .. } if component == "tunnel"
        ));

        let duplicate = runtime_asset("v0.0.14", "darwin-arm64");
        let ambiguous = json!({
            "id": 1,
            "tag_name": "v0.0.14",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"id": 2, "name": CHECKSUM_MANIFEST_NAME, "browser_download_url": asset_url("v0.0.14", CHECKSUM_MANIFEST_NAME)},
                {"id": 3, "name": duplicate, "browser_download_url": asset_url("v0.0.14", &runtime_asset("v0.0.14", "darwin-arm64"))},
                {"id": 4, "name": duplicate, "browser_download_url": asset_url("v0.0.14", &runtime_asset("v0.0.14", "darwin-arm64"))}
            ]
        });
        http.respond(url, ok(serde_json::to_vec(&ambiguous).unwrap()));
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::AmbiguousReleaseAssetFamily { component, family, .. }
                if component == "tunnel" && family == TUNNEL_RUNTIME_ASSET_PREFIX
        ));
    }

    #[tokio::test]
    async fn provider_repository_identity_cannot_be_replaced_by_supplied_policy() {
        let (provider, http) = provider();
        let catalog = catalog();
        let mut forged = tunnel_policy(&catalog);
        forged.source = ReleaseSource {
            owner: "attacker",
            repository: "placeholder",
        };
        let trusted = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&trusted);
        http.respond(url.clone(), ok(release_body("v0.0.14", false, false)));

        provider.latest_release(&forged).await.unwrap();
        assert_eq!(http.requested_urls(), vec![url]);
    }

    #[tokio::test]
    async fn provider_rejects_project_owned_component() {
        let (provider, _) = provider();
        let catalog = catalog();
        let git = catalog.component(ComponentId::Git).unwrap().clone();
        assert!(matches!(
            provider.latest_release(&git).await.unwrap_err(),
            StudioError::Unsupported(message) if message.contains("git")
        ));
    }

    #[tokio::test]
    async fn untrusted_asset_url_is_rejected() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        let body = json!({
            "id": 1,
            "tag_name": "v0.0.14",
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
            StudioError::UntrustedReleaseAssetUrl { component, .. } if component == "tunnel"
        ));
    }

    #[tokio::test]
    async fn checksum_manifest_fetch_is_bounded_metadata_only_retrieval() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let latest_url = OpenAiTunnelReleaseProvider::latest_url(&component);
        http.respond(latest_url, ok(release_body("v0.0.14", false, false)));
        let release = provider.latest_release(&component).await.unwrap();
        let manifest =
            b"deadbeef  tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip\n".to_vec();
        http.respond(release.checksum_manifest_url.clone(), ok(manifest.clone()));

        assert_eq!(
            provider.checksum_manifest(&release).await.unwrap(),
            manifest
        );
    }

    #[tokio::test]
    async fn transport_failure_is_mapped_without_leaking_request_metadata() {
        let (provider, http) = provider();
        let catalog = catalog();
        let component = tunnel_policy(&catalog);
        let url = OpenAiTunnelReleaseProvider::latest_url(&component);
        http.fail(
            url,
            HttpTransportError::Unreachable("request timed out".into()),
        );
        assert!(matches!(
            provider.latest_release(&component).await.unwrap_err(),
            StudioError::ReleaseProviderUnreachable { component, detail }
                if component == "tunnel" && detail == "request timed out"
        ));
    }

    #[test]
    fn provider_select_asset_uses_shared_trusted_platform_resolver() {
        let (provider, _) = provider();
        let release = AvailableRelease {
            component: ComponentId::Tunnel,
            version: Version::parse("0.0.14").unwrap(),
            tag: "v0.0.14".into(),
            assets: vec![ReleaseAsset {
                name: runtime_asset("v0.0.14", "darwin-arm64"),
                download_url: asset_url("v0.0.14", &runtime_asset("v0.0.14", "darwin-arm64")),
            }],
            checksum_manifest_url: asset_url("v0.0.14", CHECKSUM_MANIFEST_NAME),
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
        assert_eq!(
            selected.name,
            "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip"
        );
    }

    #[test]
    fn provider_selects_exact_full_client_companion() {
        let (provider, _) = provider();
        let release = AvailableRelease {
            component: ComponentId::Tunnel,
            version: Version::parse("0.0.14").unwrap(),
            tag: "v0.0.14".into(),
            assets: vec![ReleaseAsset {
                name: "tunnel-client-v0.0.14-darwin-arm64.zip".into(),
                download_url: asset_url("v0.0.14", "tunnel-client-v0.0.14-darwin-arm64.zip"),
            }],
            checksum_manifest_url: asset_url("v0.0.14", CHECKSUM_MANIFEST_NAME),
        };
        assert_eq!(
            provider
                .select_companion_asset(
                    &release,
                    Platform {
                        os: crate::update::OperatingSystem::Darwin,
                        arch: crate::update::Architecture::Arm64,
                    },
                )
                .unwrap()
                .unwrap()
                .name,
            "tunnel-client-v0.0.14-darwin-arm64.zip"
        );
    }

    #[test]
    fn runtime_identity_constants_match_existing_fleet_contract() {
        assert_eq!(
            TUNNEL_RUNTIME_ASSET_PREFIX,
            "tunnel-client-runtime-cloudflared"
        );
        assert_eq!(
            TUNNEL_RUNTIME_BINARY_NAME,
            "tunnel-client-runtime-cloudflared"
        );
    }
}
