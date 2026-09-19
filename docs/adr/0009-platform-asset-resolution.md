# ADR 0009: Strict platform normalization and release asset selection

- Status: Accepted
- Date: 2026-09-17

## Context

M5.2 and M5.3 provide trusted, typed release metadata for project-owned components and the official OpenAI tunnel runtime. M5.4 must convert the current host identity into one supported runtime platform and select exactly one archive without letting browser input, fuzzy filename matching, or implicit architecture fallbacks influence the result.

Current supported runtime targets are only `darwin-amd64` and `darwin-arm64`. Project MCPs and Studio publish platform-specific tarballs, Fleet publishes one architecture-independent tarball, and tunnel-client publishes platform-specific runtime-cloudflared ZIP archives.

## Decision

Studio adds one transport-free platform and asset-selection layer under `src/update/platform.rs`.

- `HostPlatform::detect()` is the production host detector. It uses Rust target identity and delegates to the same normalization functions used by tests.
- `normalize_os` maps `Darwin`/`darwin` and Rust's `macos` identity to `OperatingSystem::Darwin`.
- `normalize_arch` maps `x86_64`/`amd64` to `Architecture::Amd64` and `arm64`/`aarch64` to `Architecture::Arm64`.
- Any other OS or architecture fails closed with a typed error. Linux is intentionally not claimed as supported by M5.4.
- `ComponentPolicy` owns a trusted `ReleaseAssetPolicy { stem, kind }` in addition to its repository/install policy.
- `ReleaseAssetKind` explicitly distinguishes `PlatformTarGz`, `ArchitectureIndependentTarGz`, and `PlatformZip` packaging.
- `expected_asset_name` derives one exact filename from trusted component policy, release semantic version, and typed platform.
- `select_release_asset` requires exactly one asset with that exact name. Zero or duplicate matches fail closed. Similar versions, architectures, sidecars, or alternate product archives are never accepted as a fallback.
- Both concrete `ReleaseProvider::select_asset()` implementations delegate to this shared resolver after re-resolving trusted component policy through `ComponentCatalog`.

Current contracts are:

| Component family | Asset policy |
| --- | --- |
| Filesystem/Git/Exec/Gateway/Blender | `<binary>-v<version>-<platform>.tar.gz` |
| Studio | `mcp-studio-v<version>-<platform>.tar.gz` |
| Fleet | `mcp-fleet-v<version>.tar.gz` (explicitly architecture-independent) |
| Tunnel | `tunnel-client-runtime-cloudflared-v<version>-<platform>.zip` |

## Alternatives considered

### Let each provider implement filename selection independently

Rejected. Provider-specific implementations would duplicate platform logic and make future orchestration depend on inconsistent rules. Providers still own release-source validation; archive selection is one shared domain concern.

### Match asset names by substring or suffix

Rejected. Fuzzy matching can select a wrong architecture, wrong version, evidence sidecar, or related product archive. M5.4 derives an exact expected filename and requires exactly one equality match.

### Infer architecture independence when an asset lacks a platform suffix

Rejected. Fleet explicitly declares `ArchitectureIndependentTarGz`; architecture independence is policy, not an accidental naming observation.

### Fall back between amd64 and arm64

Rejected. M5 does not emulate/cross-run binaries. Missing host-specific artifacts are explicit failures.

## Consequences

M5.5 can call `HostPlatform::detect()` once and then `ReleaseProvider::select_asset(release, platform)` (or `select_release_asset` when it already has trusted component policy) before downloading anything. Unsupported hosts, missing archives, duplicate exact assets, or release/component mismatches fail before network artifact retrieval or filesystem mutation.

Adding Linux later requires both explicit `OperatingSystem` support and release-contract/tests for the relevant components; it must not appear automatically from host strings alone.
