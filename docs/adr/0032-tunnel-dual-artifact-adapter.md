# ADR 0032 — Tunnel Dual-Artifact Release Boundary and Native Diagnostics Adapter

- **Status:** Accepted for M7.8 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-08 OpenAI tunnel-client artifact roles, health/readiness adapter and release verification

## Context

The deployed Aira runtime uses `tunnel-client-runtime-cloudflared` v0.0.14. Binary inspection proves that artifact exposes only the `run` command; `doctor` and `runtimes` are not available.

The official OpenAI tunnel-client distribution publishes separate full-client, runtime and runtime-cloudflared artifacts. The full client contains onboarding/diagnostic/native runtime commands that are useful to Studio/Fleet, but replacing the existing Studio-owned daemon with full-client native runtime ownership would change the proven M5 process/update/rollback authority model.

M7 therefore needs the additional capability without silently moving daemon ownership.

## Verified upstream target

Checked 2026-09-22:

- official repository: `openai/tunnel-client`;
- latest stable release: **v0.0.14**, release commit `0f870e50a973fa820d4c409000059e181e8d242b`;
- official v0.0.14 checksum manifest publishes:
  - `tunnel-client-v0.0.14-darwin-arm64.zip`
    - SHA-256 `b540493c5bdbcdbb755700c8e2e16597e28b1569e425007e0f73111047bd6a64`
  - `tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip`
    - SHA-256 `c52bd95f8018b7b2903bebb45a47a1695b2c6e146b0672e2b11c38461ce729c1`

Local deployed runtime binary:

```text
0.0.14 git sha: 0f870e50... flavor=runtime-cloudflared
Available Commands:
  run
doctor => unknown command
runtimes => unknown command
```

A disposable checksum-verified full-client v0.0.14 probe returned:

```text
0.0.14+0f870e50...
doctor --help    => supported
runtimes --help  => supported
```

The full archive contains `tunnel-client`, bundled `cloudflared`, manifest, license and SPDX evidence.

Upstream references:

- https://github.com/openai/tunnel-client/releases/tag/v0.0.14
- https://github.com/openai/tunnel-client/blob/master/docs/configuration.md
- https://github.com/openai/tunnel-client/blob/master/docs/end-user-guide.md

## Decision: dual-artifact release unit

M7 keeps **runtime-cloudflared** as the production daemon executable and adds the checksum-verified **full tunnel-client** as a co-versioned diagnostics/control artifact.

A versioned installation contains both roles:

```text
runtime/tunnel-client/
  current -> releases/v0.0.14
  releases/v0.0.14/
    tunnel-client
    tunnel-client-runtime-cloudflared
    cloudflared
    cloudflared-manifest.json
    SHA256SUMS-derived verification metadata
    SPDX/license evidence
  config.yaml
```

The two executable roles must come from the same upstream release tag/commit.

## Ownership boundary

Studio `TunnelSupervisor` remains authoritative for the production daemon process.

It continues to launch:

```text
tunnel-client-runtime-cloudflared run ...
```

M7 does **not** use `tunnel-client runtimes connect` to take ownership of that production daemon. Doing so would create a second process supervisor and invalidate M5 launch-generation/rollback assumptions.

The full client is used for bounded diagnostics/preflight and explicit operator workflows only.

## Adapter surfaces

M7 tunnel status separates:

```text
process_running
liveness
readiness
mcp_discovery_health
control_plane_poll_health
diagnostic_status
```

These are not aliases.

Primary runtime observations come from the running daemon's local health surfaces:

- `/healthz` => process/service liveness;
- `/readyz` => runtime readiness policy;
- `/health/mcp` => observed MCP discovery;
- `/health?details=true` => bounded component/control-plane details.

The full client may run:

- `--version` for release identity;
- `doctor --json/--explain` as a bounded preflight/diagnostic command when required credentials/config are available;
- read-only native-runtime inspection commands only for runtimes actually owned by that native runtime subsystem.

Studio must not infer production daemon ownership from `runtimes status` because the production daemon remains Studio-owned.

## Release provider changes

The existing official release provider currently selects only `tunnel-client-runtime-cloudflared`.

M7.8 changes the tunnel release contract to select and verify both exact platform assets from one release metadata snapshot:

```text
tunnel-client-v<version>-<os>-<arch>.zip
tunnel-client-runtime-cloudflared-v<version>-<os>-<arch>.zip
SHA256SUMS.txt
```

Both archives must:

- match the requested stable tag;
- have checksum entries in the same manifest;
- expose the expected executables;
- report the same semantic version and release commit;
- match the supported host platform;
- preserve existing config/credentials outside the immutable version directory.

No best-effort fallback to an unverified system `tunnel-client` is allowed.

## Activation and rollback

The tunnel update transaction remains versioned and journaled.

Activation is committed only after:

1. both assets are checksum verified;
2. archive structure is validated;
3. both executables report the target version/commit;
4. `current` switches atomically;
5. a previously running daemon restarts from the new runtime-cloudflared path;
6. liveness/readiness/control-plane health policy passes;
7. launch evidence proves the running binary is the activated runtime artifact.

Failure restores the previous versioned release and previous running/stopped ownership state using the existing M5 rollback model.

The full-client binary is never the rollback authority; it is part of the verified release payload.

## Security

- diagnostics must not log API keys/tokens/raw config secrets;
- Studio public DTOs expose state/reasons, not credential values or internal command argv;
- `doctor` output is sanitized before persistence/UI exposure;
- full-client mutating admin/runtimes commands are not exposed as general browser actions in M7;
- health endpoints remain loopback/Unix-socket scoped according to the trusted tunnel config.

## Verification requirements

M7.8 permanent tests must prove:

- provider selects both exact assets for the same release/platform;
- missing/duplicate checksum entry for either asset fails closed;
- mismatched full/runtime version or commit fails closed;
- runtime-cloudflared remains the launched production executable;
- full client `doctor` capability is available in the staged release;
- liveness/readiness/poll health remain distinct;
- config/credentials survive upgrade/rollback unchanged;
- running and stopped update matrices preserve prior owner state;
- source-less runtime package includes both executables and requires no source/toolchain.

These map to R42-R45 and Q12-Q13.

## Consequences

- M7 gains official diagnostic/native CLI capability without changing proven daemon ownership.
- Tunnel release storage grows because two official archives are retained per version.
- Provider/update tests must reason about a coordinated release unit instead of one asset.
- A future migration to native `runtimes connect` ownership requires a separate ADR and M5 authority migration; it is not implicit in M7.
