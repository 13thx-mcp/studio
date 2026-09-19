# ADR 0006: Runtime update domain and trusted component catalog

- Status: Accepted
- Date: 2026-09-17

## Context

Milestone 5 introduces release-based runtime management for project-owned MCP binaries, Studio, Fleet, and the official OpenAI tunnel-client. The update system must work on source-less hosts and must not let browser input choose arbitrary repositories, download URLs, checksum locations, executable paths, or install roots.

The existing Studio architecture already has separate runtime ownership domains: the persistent MCP registry and supervisor own MCP launch configuration/lifecycle, while the tunnel supervisor owns tunnel lifecycle and secret handling. Fleet tooling also already demonstrates release/version concepts and the flat `bin/` plus `runtime/` filesystem layout.

M5.1 needs a stable model boundary before M5.2 adds network release-provider behavior.

## Decision

Studio adds an `update` domain module with these durable boundaries:

- `ComponentId` is the closed set of managed component identities accepted by update logic.
- `ComponentCatalog` is server-owned policy mapping each component to its class, trusted release provider/repository, install target, channel, and restart policy.
- `HostRuntimeRoots` supplies trusted `bin_root` and `runtime_root`; install paths are derived from those roots plus catalog policy, never from browser input.
- `Version` wraps the `semver` crate and exposes explicit SemVer precedence comparison; build metadata is excluded from update freshness decisions.
- `InstalledArtifactIdentity`, `AvailableRelease`, `Platform`, `VersionDimensions`, `DriftState`, `UpdateState`, `UpdateTransaction`, and `RollbackMetadata` separate installed/running/desired/latest identity and update transaction state.
- `ReleaseProvider` is an asynchronous, read-only metadata interface. Concrete HTTP providers are deferred to M5.2/M5.3.
- `InstalledIdentityProvider` is a read-only artifact/runtime identity interface. It receives only server-derived install paths.
- `ComponentView` is a browser-safe projection that deliberately omits repository ownership, release/download/checksum URLs, install paths, binary names, and secrets.

The catalog preserves the current runtime layout:

- Filesystem, Git, Exec, Gateway, and Blender are `McpBinary` components installed as flat files under `bin_root`.
- Studio is a `Service` installed below `runtime_root/studio` and requires an external-supervisor restart contract.
- Fleet is a `ControlBundle` installed below `runtime_root/fleet`.
- Tunnel is an `UpstreamRuntime` installed below `runtime_root/tunnel-client` and sourced only from `openai/tunnel-client`.

M5.1 performs no release HTTP requests, downloads, verification, extraction, activation, restart, or rollback execution.

## Alternatives considered

### Reuse the MCP registry as the update catalog

Rejected. The registry is intentionally operator-editable lifecycle configuration and may contain dynamically discovered MCPs. Release repositories and install authority must instead be a closed server-side policy boundary.

### Accept release URLs or install paths in future update requests

Rejected. This would turn Studio into a generic downloader/installer and violate the M5 trust boundary. Browser requests must select only a known component identity and, where later allowed, a validated semantic version or operation.

### Compare version strings directly

Rejected. Lexicographic ordering gives incorrect results such as `1.10.0 < 1.9.0`. The update domain uses validated SemVer precedence explicitly rather than the crate total order, so build metadata does not affect update freshness.

### Put provider-specific HTTP response types into higher layers

Rejected. Provider transport and GitHub metadata shape should stay behind `ReleaseProvider` so later update orchestration depends on domain identities rather than transport details.

## Consequences

M5.2 must preserve the catalog as the authority for project-owned repositories and implement `ReleaseProvider` without adding caller-selected URLs. M5.3 must do the same for the official OpenAI tunnel source. Later inventory/update endpoints may expose browser-safe projections but must not serialize the internal policy/identity types containing paths or URLs directly.

Platform normalization is intentionally not implemented here; M5.4 owns host OS/architecture resolution. Downloading, checksums, safe extraction, activation, and rollback execution remain later M5 submilestones.
