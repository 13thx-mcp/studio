# ADR 0008: Official OpenAI tunnel-client release provider

- Status: Accepted
- Date: 2026-09-17

## Context

M5.3 adds release discovery for the tunnel runtime after M5.2 established the project-owned `13thx-mcp` GitHub provider. Tunnel-client is an upstream dependency and must not be treated as a project-owned release source. The authoritative source is only `openai/tunnel-client`; the historical `13thx-mcp/tunnel-client` placeholder is not an acceptable update authority.

The official release contains multiple product flavors and evidence sidecars. Studio specifically manages the `tunnel-client-runtime-cloudflared` runtime bundle. Later M5 stages need enough metadata to choose the correct platform archive and validate the extracted runtime version, without downloading or activating anything during M5.3.

## Decision

Studio adds `OpenAiTunnelReleaseProvider` behind the unchanged M5.1 `ReleaseProvider` trait.

- The provider accepts only the catalog-owned `ComponentId::Tunnel` policy whose provider is `github_openai` and whose exact source is `openai/tunnel-client`.
- Every lookup re-resolves component policy through `ComponentCatalog`; mutable repository coordinates on a supplied `ComponentPolicy` are ignored.
- The provider and the M5.2 `ThirteenthXReleaseProvider` share one internal bounded GitHub HTTP transport for HTTPS enforcement, timeouts, redirect policy, response-size limits, User-Agent behavior, and sanitized transport errors.
- Stable discovery excludes GitHub drafts and prereleases. Explicit semantic prerelease versions are rejected before network lookup.
- Release assets are preserved in full, but metadata is accepted only when exactly one `SHA256SUMS.txt` exists and at least one unambiguous runtime archive matches `tunnel-client-runtime-cloudflared-v<release>-<os>-<arch>.zip`.
- Runtime-cloudflared ZIP names with a version different from the release tag, malformed target identity, or duplicate platform target are rejected as malformed/ambiguous provider metadata.
- License/SPDX/source/full-client assets are retained as release metadata but are not considered runtime-cloudflared archive candidates.
- `TUNNEL_RUNTIME_ASSET_PREFIX` and `TUNNEL_RUNTIME_BINARY_NAME` expose the stable upstream runtime identity for later platform/staging/version-validation work.
- M5.3 does not select a host platform, download an archive, verify checksum contents, extract files, validate an extracted binary, activate `current`, restart the tunnel, or perform rollback.

## Alternatives considered

### Reuse the archived `13thx-mcp/tunnel-client` source

Rejected. That would violate the upstream authority contract and allow project-owned publication to impersonate the official runtime source.

### Treat every upstream ZIP as an equivalent tunnel runtime

Rejected. Official releases contain full-client, runtime-only, runtime-cloudflared, evidence, and source artifacts. Studio specifically manages the runtime-cloudflared bundle and must not accidentally select another product flavor.

### Implement platform selection in the provider

Rejected. M5.4 owns host normalization and deterministic platform matching for all providers. M5.3 validates the runtime archive family and preserves candidate metadata only.

### Duplicate the M5.2 network client

Rejected. Sharing the hardened GitHub transport reduces policy drift in timeout, redirect, body-size, and error-sanitization behavior while keeping repository/provider authority separate at the provider layer.

## Consequences

M5.4 can rely on tunnel `AvailableRelease` values having a validated semantic release version, a complete trusted asset list, exactly one checksum-manifest URL, and an internally validated runtime-cloudflared ZIP family. M5.4 still owns deterministic Darwin amd64/arm64 selection. M5.5 must verify `SHA256SUMS.txt`, safely extract ZIP contents, and validate the staged `tunnel-client-runtime-cloudflared` binary version before activation is possible.
