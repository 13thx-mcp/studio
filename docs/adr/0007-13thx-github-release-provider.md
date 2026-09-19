# ADR 0007: Trusted 13thx GitHub release discovery

- Status: Accepted
- Date: 2026-09-17

## Context

M5.1 established `ComponentCatalog` as the authority for component identity, release provider, repository ownership, install targets, and update channel. M5.2 needs read-only release discovery for project-owned components under `13thx-mcp` without weakening that boundary or introducing archive download/activation early.

GitHub release metadata is remote, mutable input. A provider must therefore distinguish transport failures from malformed or ineligible releases, avoid caller-selected repositories or base URLs, reject draft/prerelease content from the stable channel, and return only asset metadata that remains bound to the trusted repository.

## Decision

Studio implements `ThirteenthXReleaseProvider` behind the existing M5.1 `ReleaseProvider` trait.

- Production metadata requests use the fixed `https://api.github.com` origin.
- The provider re-resolves every supplied `ComponentPolicy` by `ComponentId` through its own `ComponentCatalog`; repository fields on the supplied instance are not trusted.
- Only catalog components mapped to `ReleaseProviderId::ThirteenthXGitHub` and owner `13thx-mcp` are accepted.
- Latest lookup uses GitHub's normal latest-release endpoint. Explicit lookup uses `releases/tags/v<semver>`.
- The provider supports the stable channel only. Draft releases, GitHub prereleases, semantic-version prereleases, and explicit prerelease requests are rejected.
- Release and asset immutable GitHub IDs must be non-zero. Tags must follow the project's `vMAJOR.MINOR.PATCH` convention and parse through the M5.1 `Version` type.
- Uploaded assets are preserved as `ReleaseAsset` metadata after validating that each browser download URL is HTTPS on `github.com` under the trusted owner/repository release-download path.
- Exactly one `SHA256SUMS.txt` uploaded asset is required.
- Release metadata responses are limited to 1 MiB and checksum manifests to 256 KiB. The HTTP client uses a 3-second connect timeout, 10-second request timeout, HTTPS-only requests, and a Studio-specific User-Agent.
- No browser token, repository URL, release URL, install path, or executable path is accepted by the provider.
- M5.2 may retrieve the small checksum manifest required by the M5.1 interface, but it does not download release archives, select platform archives, extract content, or activate anything. Platform selection remains fail-closed until M5.4.

## Alternatives considered

### Accept a repository or API base URL from the caller

Rejected. That would bypass the server-owned component catalog and turn Studio into a generic network fetcher. Testability is instead provided by an internal transport abstraction with hermetic fixtures.

### Use GitHub's release list and choose the highest semantic version locally

Rejected for M5.2 latest-release semantics. GitHub's normal latest-release endpoint is the repository's release-channel decision. Studio validates the returned tag and stable eligibility but does not silently redefine publisher ordering.

### Select Darwin architecture assets in the provider

Rejected. M5.4 owns platform normalization and deterministic asset selection. M5.2 only preserves validated asset metadata.

### Download release archives while discovering metadata

Rejected. M5.5 owns retrieval, checksum verification, and safe staging. Mixing those concerns would violate the milestone boundary and make a metadata check mutate disk/runtime state.

## Consequences

M5.3 can implement the OpenAI tunnel provider against the same `ReleaseProvider` boundary without inheriting project-repository authority. M5.4 can rely on a validated asset list and semantic release identity, while M5.5 can rely on a unique checksum-manifest URL that is already repository-bound but must still verify the manifest and archive contents cryptographically.

A compromised trusted repository can still publish malicious artifacts or a matching malicious checksum. M5.2 does not claim supply-chain provenance beyond GitHub/repository trust; later integrity/staging work must enforce checksum, structure, platform, and activation invariants.
