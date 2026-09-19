use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use reqwest::{Client, Url, header::ACCEPT, redirect::Policy};

use crate::error::{StudioError, StudioResult};

pub(super) const GITHUB_JSON_ACCEPT: &str = "application/vnd.github+json";
pub(super) const BINARY_ACCEPT: &str = "application/octet-stream";
pub(super) const MAX_RELEASE_METADATA_BYTES: usize = 1024 * 1024;
pub(super) const MAX_CHECKSUM_MANIFEST_BYTES: usize = 256 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn build_github_http(provider: &str) -> StudioResult<Arc<dyn GitHubHttp>> {
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
            "mcp-studio/{} github-release-provider",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| StudioError::ReleaseProviderUnreachable {
            component: provider.to_owned(),
            detail: format!("HTTP client initialization failed: {error}"),
        })?;

    Ok(Arc::new(ReqwestGitHubHttp { client }))
}

#[derive(Debug, Clone)]
pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(super) enum HttpTransportError {
    Unreachable(String),
    ResponseTooLarge { limit: usize },
}

pub(super) fn map_transport_error(component: &str, error: HttpTransportError) -> StudioError {
    match error {
        HttpTransportError::Unreachable(detail) => StudioError::ReleaseProviderUnreachable {
            component: component.to_owned(),
            detail,
        },
        HttpTransportError::ResponseTooLarge { limit } => {
            StudioError::ReleaseProviderResponseTooLarge {
                component: component.to_owned(),
                limit,
            }
        }
    }
}

#[async_trait]
pub(super) trait GitHubHttp: Send + Sync {
    async fn get(
        &self,
        url: &str,
        accept: &'static str,
        max_bytes: usize,
    ) -> Result<HttpResponse, HttpTransportError>;
}

struct ReqwestGitHubHttp {
    client: Client,
}

#[async_trait]
impl GitHubHttp for ReqwestGitHubHttp {
    async fn get(
        &self,
        url: &str,
        accept: &'static str,
        max_bytes: usize,
    ) -> Result<HttpResponse, HttpTransportError> {
        let mut response = self
            .client
            .get(url)
            .header(ACCEPT, accept)
            .send()
            .await
            .map_err(sanitize_reqwest_error)?;
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(HttpTransportError::ResponseTooLarge { limit: max_bytes });
        }

        let status = response.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(sanitize_reqwest_error)? {
            if body.len().saturating_add(chunk.len()) > max_bytes {
                return Err(HttpTransportError::ResponseTooLarge { limit: max_bytes });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse { status, body })
    }
}

pub(super) fn trusted_redirect_target(url: &Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    matches!(url.host_str(), Some("github.com") | Some("api.github.com"))
        || url
            .host_str()
            .is_some_and(|host| host.ends_with(".githubusercontent.com"))
}

fn sanitize_reqwest_error(error: reqwest::Error) -> HttpTransportError {
    let detail = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else {
        "request failed"
    };
    HttpTransportError::Unreachable(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_policy_allows_only_https_github_controlled_hosts() {
        for allowed in [
            "https://api.github.com/repos/openai/tunnel-client/releases/latest",
            "https://github.com/openai/tunnel-client/releases/download/v0.0.14/SHA256SUMS.txt",
            "https://release-assets.githubusercontent.com/github-production-release-asset/file",
            "https://objects.githubusercontent.com/github-production-release-asset/file",
        ] {
            assert!(trusted_redirect_target(&Url::parse(allowed).unwrap()));
        }
        for blocked in [
            "http://github.com/openai/tunnel-client/releases/latest",
            "https://example.invalid/file",
            "https://github.com.attacker.invalid/file",
            "https://user:pass@github.com/file",
        ] {
            assert!(!trusted_redirect_target(&Url::parse(blocked).unwrap()));
        }
    }
}
