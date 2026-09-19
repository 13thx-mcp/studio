# ADR 0011: Runtime inventory truth, desired-version policy, and drift API

- Status: Accepted
- Date: 2026-09-17

## Context

M5.6 needs trustworthy installed/running/desired/latest version state before update activation is introduced. Runtime hosts may not contain project source, so source manifests and Git HEAD cannot be authoritative for installed identity. At the same time, the browser-safe M5.1 boundary intentionally excludes internal install paths and repository/download coordinates.

## Decision

Studio adds an `InventoryService` with separate truth sources:

- installed identity comes only from catalog-derived runtime artifacts;
- running identity comes from an explicit `RunningIdentityProvider` and is `Unknown` when it cannot be proven;
- desired identity is an optional server-side `[updates.desired]` semantic-version pin; without a pin, manual M5 policy preserves the currently installed version;
- latest identity is cached from the trusted release provider only when `POST /api/updates/check` is requested.

Installed identity sources are component-specific:

- project MCP binaries: flat `bin_root/<binary> --version`;
- Studio: `runtime_root/studio/mcp-studio --version`;
- Fleet: `runtime_root/fleet/VERSION`;
- tunnel: `runtime_root/tunnel-client/current/tunnel-client-runtime-cloudflared --version`, with `current` required to resolve within the tunnel install root and a `v<version>` release directory required to agree with the binary when present.

Source presence is reported only as host-mode context. It never participates in installed-version derivation.

The default update roots continue to support both a source checkout and a source-less runtime layout by locating the nearest conventional ancestor containing both `bin/` and `runtime/`. Explicit `[updates]` paths override this detection.

For M5, running version is exposed only when it is actually observable. Studio can prove its own running package version. Other components remain `None`/unknown until lifecycle ownership captures a stronger running-artifact identity. Fleet is a non-running control bundle, so drift derivation treats its installed identity as the effective runtime identity without serializing a fake `running_version`.

Release checks are per-process transient cache entries. Provider failure does not erase local installed state or a previous last-known latest version. Public check errors are reduced to safe categories/statuses and never serialize provider URLs or response details.

The public API is:

- `GET /api/updates`
- `GET /api/updates/{component}`
- `POST /api/updates/check`

The POST is same-origin protected, performs release metadata checks only, and publishes `updates_changed`. Public DTOs expose component/class/provider IDs, platform, host mode, installed/running/desired/latest versions, update availability, installation health, drift, and last-check status. They do not expose install paths, repository owner/name, release/download/checksum URLs, headers, or secrets.

## Alternatives considered

### Infer installed versions from source manifests

Rejected. Runtime-only hosts intentionally do not require source repositories, and source state can differ from deployed artifacts.

### Assume running version equals installed version

Rejected. A binary may have been replaced after process start or a configured process may not be tied to the catalog install path. Unknown is safer until runtime identity is captured explicitly.

### Make latest-stable the implicit desired version

Rejected for M5. That would blur manual update visibility with update policy. M5 keeps desired equal to installed unless an explicit server-side pin exists; unattended latest-stable policy belongs to M7.

### Expose internal paths/repositories in the inventory API

Rejected. Internal inventory retains those identities for trusted orchestration, but the browser projection preserves the M5.1 authority boundary.

## Consequences

M5.7 can update installed/desired/running state only through its transaction/lifecycle contracts; it must not infer synchronization from source state. M5.11 can build an Updates/Fleet UI on the stable inventory DTOs and `updates_changed` invalidation event. Persisted update/check history remains M6 scope.