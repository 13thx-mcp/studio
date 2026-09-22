# MCP Studio Architecture

## Purpose

MCP Studio is a local-first control plane for MCP servers and the existing secure tunnel runtime under `mcp-server/`.

## Current boundary — M7 closed / M8 contract freeze

Milestones 0–4 provide MCP supervision, the browser dashboard, secure-tunnel lifecycle, a persistent MCP registry, and metadata-only project discovery. M5 adds trusted runtime distribution, inventory/drift, transactional MCP/Gateway/Fleet/Tunnel updates, runtime reconciliation, external Studio self-update, rollback, and the Updates/Fleet operator surface. M5 publication qualification is complete for the supported darwin-arm64 runtime.

M6 adds private SQLite-backed operational history, typed audit admission/outcome evidence, Studio/MCP/Tunnel observation sessions, update/artifact/install lineage, configuration/drift history, replay-safe metrics, bounded retention, and historical REST/realtime/UI. Registry/configuration files and live runtime/update owners remain authoritative; SQLite is historical evidence only and never resumes recovery or rehydrates live authority.

M7 is closed. M8.0 contracts, M8.1 automation foundation, and M8.2 health/restart safety are implemented/qualified. Studio now has typed health/freshness evidence and bounded restart policy for processes it directly owns; Gateway exposes additive child-status capability while retaining Gateway ownership of child recovery. Provider checking/staging, durable unknown-outcome holds, targeted child restart, automatic activation and reconciliation mutation remain later M8 packages. Remote authentication/RBAC remains M9 scope.

A proposed post-M7 Workspace Skill Runtime is documented in ADR 0036 and `docs/plans/workspace-skill-runtime/`. It does not change the current M7 implementation boundary: Gateway remains the external MCP request/safety boundary, typed MCPs remain concrete capability boundaries, and Studio would provide operator inventory/policy/evidence surfaces rather than becoming a general-purpose remote worker.

## Components

- `api`: HTTP/WebSocket API, same-origin mutation protection, SPA/static-file boundary.
- `config`: typed bootstrap/runtime configuration and validation.
- `registry`: schema-versioned persistent MCP registry, atomic writes, ID/path validation, and browser-safe views.
- `discovery`: metadata-only direct-child scanning of supported Rust, Node, and Python project manifests.
- `supervisor`: MCP child-process lifecycle, transient runtime state, PID ownership, log capture, event publication, and shutdown cleanup.
- `tunnel`: independent tunnel configuration/lifecycle/secret-redaction domain.
- `update`: trusted component catalog plus typed release, installed-artifact, version, platform, drift, provider, installed/running identity, deterministic asset selection, verified staging, M5.7–M5.10 update transactions including durable Studio self-update handoff, and M5.9A runtime reconciliation with managed-state fingerprints and exact local Gateway catalog identity; M5.2/M5.3 include separate read-only GitHub providers for project-owned and official upstream releases.
- `realtime`: typed bounded event streams plus registry/discovery/update/reconciliation/history invalidation and health events.
- `web`: React + TypeScript + Vite operational dashboard with Registry/Discovery, Updates/Fleet controls, and M6 historical views.
- `metrics`: M6 historical aggregation definitions and replay-safe bounded metric buckets plus sanitized M7 Gateway request/catalog/recovery history projections.
- `storage`: M6 private SQLite history/audit store, migrations, bounded worker queues/readers, retention, backup and historical query projections.
- `automation`: M8.1 provides bounded policy/scheduling state and shared audit admission; M8.2 adds typed health/restart ownership; M8.3 adds audited release checks, notify-only and verified prepare-only handlers with process-scoped staging authority and provider circuits. No M8.3 handler can activate runtime bytes; physical mutation remains behind the later M8.4 SafetyGate.
- `logging`: structured logging initialization.
- `error`: shared typed error boundary.

## M8.0 accepted automation architecture

M8.0 freezes the design in ADR 0037–0045 without enabling unattended mutation.

The accepted ownership model is:

```text
AutomationController (Studio)
  ├─ schedule/policy/circuit only
  ├─ shared audit admission
  ├─ activation-time SafetyGate
  └─ bounded automation state
          │
          ├─ InventoryService
          ├─ RuntimeReconciler
          ├─ component update managers
          └─ GatewayControlClient
                         │
                         ▼
                 Gateway live authority
                 ├─ M7 request/drain state
                 ├─ M8 durable safety holds
                 └─ global drain + targeted child restart
```

The controller is never a durable job queue. M6 SQLite remains historical evidence only.

For Fleet-managed hosts, automation configuration is desired state in Fleet host schema v3 and is rendered into the existing managed `studio.config` surface. A changed managed Studio config blocks destructive automation until a restarted Studio process proves it loaded the new canonical bytes.

Gateway M8 uses additive control-protocol capabilities. The initial generic MCP activation contract reuses the existing global M7 drain as the mutation barrier, restarts only the target child under the matching drain generation, verifies its new generation/catalog/identity, and then resumes admission. A second per-child scheduling system is intentionally not introduced.

Unsafe post-dispatch unknown outcomes for server-owned `mutation`, `long-running`, or `control` classes become bounded durable Gateway safety holds. The hold store is a veto authority only; it contains no request arguments/results and cannot replay work.

Generic/Gateway/Fleet auto-prepared staging is process-scoped authorization. After Studio restart, old ready staging may be revalidated for cleanup/reprepare but cannot be auto-applied merely because bytes remain on disk.

The detailed source feasibility record is `docs/plans/m8-hardening-auto-update-recovery/M8.0-SOURCE-SPIKES.md`.

## Runtime model

```text
Browser
   |
   | same-origin HTTP / WebSocket
   v
+-----------------------------------------------------------------+
| MCP Studio / Axum                                               |
|                                                                 |
| Registry/Discovery REST ---> Registry <----> data/registry.toml |
|                                 |                               |
| MCP lifecycle REST ------------+----> MCP Supervisor ---> MCP   |
|                                                                 |
| Tunnel lifecycle REST ----------------> Tunnel Supervisor       |
|                                                                 |
| WebSocket <--- process/tunnel events + registry invalidations   |
| React/Vite dashboard                                            |
+-----------------------------------------------------------------+
                |
                +---- metadata-only scan ----> configured MCP root
```

`DiscoveredProject`, persistent `RegisteredMcp`, and transient `ProcessStatus` are separate models.

## Runtime-distribution domain

M5.1 introduces a closed, server-owned component catalog. A component is selected by `ComponentId`; repository ownership, provider selection, install target, channel, and restart policy are derived from catalog policy rather than request data.

Current component mapping is:

| Component | Class | Trusted release source | Install target |
| --- | --- | --- | --- |
| filesystem | `McpBinary` | `13thx-mcp/filesystem` | flat `bin_root/rust-mcp-filesystem` |
| git | `McpBinary` | `13thx-mcp/git` | flat `bin_root/rust-mcp-git` |
| exec | `McpBinary` | `13thx-mcp/exec` | flat `bin_root/rust-mcp-exec` |
| gateway | `McpBinary` | `13thx-mcp/gateway` | flat `bin_root/rust-mcp-gateway` |
| blender | `McpBinary` | `13thx-mcp/blender` | flat `bin_root/rust-mcp-blender` |
| studio | `Service` | `13thx-mcp/studio` | `runtime_root/studio` |
| fleet | `ControlBundle` | `13thx-mcp/fleet` | `runtime_root/fleet` |
| tunnel | `UpstreamRuntime` | `openai/tunnel-client` | `runtime_root/tunnel-client` |

`HostRuntimeRoots` is a trusted server-side input for `bin_root` and `runtime_root`; `ComponentCatalog` derives install paths from those roots. Browser-safe `ComponentView` deliberately omits repository ownership, release/download/checksum URLs, install paths, binary names, and secrets.

Release versions use the `Version` semantic-version wrapper rather than string comparison. Update freshness uses explicit SemVer precedence (`cmp_precedence` / `is_newer_than`), so build metadata does not make an otherwise equal release newer. `VersionDimensions` keeps installed, running, desired, and latest versions separate. Drift derivation is deterministic: broken installation health maps to `BROKEN`; unknown health or incomplete installed/running/desired identity maps to `UNKNOWN`; installed differing from desired maps to `DRIFTED`; running differing from an otherwise desired installed version maps to `INSTALLED_RESTART_REQUIRED`; a newer latest release maps to `UPDATE_AVAILABLE`; otherwise the state is `CURRENT`.

`UpdateState` covers the M5 success path plus explicit download/verify/stage/stop/activate/health/rollback failures. `UpdateTransaction` and `RollbackMetadata` are transient models only; M5.1 adds no ad-hoc persistence.

`ReleaseProvider` is an asynchronous read-only boundary for latest/specific release lookup, asset selection, and checksum-manifest retrieval. `InstalledIdentityProvider` is a separate read-only boundary for installed artifact/runtime inspection.

M5.2 implements `ThirteenthXReleaseProvider` for catalog components whose trusted provider is `github_13thx`. Production requests use a fixed `https://api.github.com` origin, bounded connect/request timeouts, a bounded response body, a Studio-specific User-Agent, and no browser- or caller-supplied repository/base URL. The provider re-resolves the supplied component ID through `ComponentCatalog` and ignores mutable repository coordinates on a supplied `ComponentPolicy`, so the catalog remains the authority boundary.

The `github_13thx` channel is stable-only. Latest lookup uses GitHub's normal latest-release endpoint and rejects any returned draft or prerelease metadata. Explicit lookup accepts validated stable semantic versions and uses `vMAJOR.MINOR.PATCH` tags; prerelease versions are rejected rather than silently promoted into the stable channel. Release metadata must contain exactly one uploaded `SHA256SUMS.txt` asset. Every uploaded asset URL must be HTTPS on `github.com` under the trusted `/<owner>/<repository>/releases/download/` path before it is returned to later stages. Checksum-manifest retrieval is capped at 256 KiB; release JSON is capped at 1 MiB. No release archive is downloaded in M5.2.

The release packaging contract uses Rust MCP archives named `<binary>-vX.Y.Z-darwin-arm64.tar.gz`, Studio archives named `mcp-studio-vX.Y.Z-darwin-arm64.tar.gz`, Fleet archives named `mcp-fleet-vX.Y.Z.tar.gz`, and a single `SHA256SUMS.txt`. M5.2 preserves the complete uploaded asset list but deliberately does not choose an OS/architecture asset; `select_asset` remains fail-closed until M5.4 owns platform resolution.

M5.3 implements `OpenAiTunnelReleaseProvider` only for the catalog-owned `openai/tunnel-client` source (`github_openai`). It shares the hardened bounded GitHub HTTP transport introduced by M5.2 but keeps repository/provider validation separate from `github_13thx`; a supplied `ComponentPolicy` cannot redirect tunnel lookup to another owner or repository. The stable channel excludes drafts and prereleases, including `-dev` tags.

The tunnel provider preserves the complete GitHub asset list and additionally validates that the release contains an unambiguous `tunnel-client-runtime-cloudflared-v<release>-<os>-<arch>.zip` family plus exactly one `SHA256SUMS.txt`. License/SPDX/source/full-client assets remain metadata only and are not mistaken for runnable runtime archives. `TUNNEL_RUNTIME_ASSET_PREFIX` and `TUNNEL_RUNTIME_BINARY_NAME` preserve the upstream runtime identity needed by later staging/version-validation work.

As verified against the official `v0.0.14` release, the required Darwin candidate is `tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip`; the release also publishes matching evidence sidecars and `SHA256SUMS.txt`.

M5.4 adds `HostPlatform`, `normalize_os`, and `normalize_arch`. Supported host identity is deliberately narrow: Darwin/macOS on Apple Silicon, with arm64 (`arm64`/`aarch64`). Intel (`x86_64`/`amd64`), every other architecture, and non-macOS hosts fail closed.

Each trusted `ComponentPolicy` now includes `ReleaseAssetPolicy { stem, kind }`. `ReleaseAssetKind` distinguishes platform tarballs, architecture-independent tarballs, and platform ZIPs. Project MCPs derive `<binary>-v<version>-<platform>.tar.gz`; Studio derives `mcp-studio-v<version>-<platform>.tar.gz`; Fleet explicitly derives the architecture-independent `mcp-fleet-v<version>.tar.gz`; tunnel derives `tunnel-client-runtime-cloudflared-v<version>-<platform>.zip`. `select_release_asset` requires exactly one exact filename match and rejects component mismatch, zero matches, duplicates, wrong versions/architectures, and lookalike sidecars. Both concrete `ReleaseProvider::select_asset()` implementations delegate to this same trusted resolver.

M5.5 adds `ArtifactStager` and `StagedArtifact`. Staging is confined below `<runtime_root>/.mcp-studio-staging`, never an active install path. Each transaction starts as `.partial-*`, streams a bounded HTTPS artifact while computing SHA-256, verifies the exact `SHA256SUMS.txt` entry, safely extracts with entry-count/expanded-size limits, validates the closed package layout, normalizes expected executable permissions, writes `staged.json`, then atomically renames the transaction to a unique `ready-*` directory. Failed partial transactions are removed automatically and stale partial directories are cleaned at stager construction.

Safe extraction rejects absolute/traversal paths, ambiguous path separators, duplicate paths, symlinks, hardlinks, and special/unsupported entry types. Project/Studio/Fleet tarballs require their current exact package roots and required files. The official tunnel runtime ZIP is root-level and must contain exactly the runtime binary, `cloudflared`, `cloudflared-manifest.json`, `LICENSE`, `NOTICE`, release-specific license report, and SPDX document. Tunnel staging additionally runs only the checksum-verified staged runtime binary with `--version`, a bounded timeout, and an empty environment, requiring the semantic version to match the release.

`StagedArtifact` records component/version/provider/release/platform identity, selected asset, exact archive SHA-256, final staging path, validated package root/executables, and verification timestamp. The verified archive and checksum manifest are retained inside the ready staging transaction for later activation/audit use. M5.5 exposes no public staging/update endpoint and does not stop, replace, restart, activate, or roll back any component.

M5.6 adds `InventoryService`. Installed identity comes only from deployed runtime artifacts: flat MCP binaries and Studio use strict `--version` probes, Fleet uses deployed `VERSION`, and tunnel uses the confined `current` release plus runtime binary version. Source-tree presence is reported only as `source_present` / host-mode context and never contributes to installed version truth. Default update roots locate the nearest conventional ancestor containing `bin/` and `runtime/`, while explicit `[updates]` roots override discovery for runtime-only deployments.

Desired version policy remains manual: optional `[updates.desired]` semantic-version pins override the default preserve-current behavior (`desired = installed`). Latest versions are refreshed only by `POST /api/updates/check` and cached for the current Studio process. A failed remote check preserves local inventory and a last-known latest version while recording a sanitized check error category. Running identity remains separate and unknown when it cannot be proven; Studio can report its own running package version, while Fleet drift treats installed identity as effective state because it is a non-running control bundle.

Read-only inventory routes are `GET /api/updates`, `GET /api/updates/{component}`, and same-origin `POST /api/updates/check`. Public `InventoryView` exposes only browser-safe component/provider IDs, platform/host mode, installed/running/desired/latest versions, update availability, installation health, drift, and last-check status. Internal install paths and repository coordinates remain available to trusted orchestration but are never serialized. Successful checks publish `updates_changed`; M5.6 performs no artifact download, staging, activation, restart, or rollback.

M5.7 adds `McpUpdateManager` for project-owned `McpBinary` targets except Gateway. `POST /api/updates/{component}/prepare` accepts only an approved semantic version and resolves/stages the trusted release. `POST /api/updates/{component}/apply` accepts only the server-generated transaction ID; `GET /api/update-transactions/{transaction_id}` exposes browser-safe process-lifetime state. Transaction phases are `preparing`, `staged`, `stopping`, `activating`, `starting`, `verifying`, `rolling_back`, `completed`, `failed`, and `rollback_failed`, and the same DTO is emitted as realtime `update_transaction` events.

Before stopping a process, apply re-loads a `ready-*` artifact from the server-owned staging root and revalidates component/provider/release/asset identity, archive SHA-256, extracted executable SHA-256, path confinement, and the prepared staging fingerprint. It also requires the deployed source version to remain unchanged since preparation and the registered supervisor executable to resolve to the catalog-derived flat-bin target. Per-component locks reject duplicate operations while permitting unrelated components to proceed independently.

Activation prepares checksum-protected rollback material first, preserves the target's executable permissions, copies the new binary to a sibling scratch path, and uses same-directory rename replacement so only the selected flat-bin file changes. A stopped MCP stays stopped; a previously running MCP is stopped through `Supervisor`, replaced, and restarted only if it was running and enabled. Mandatory post-activation verification checks the exact semantic `--version`; for a restarted MCP it additionally requires `Running` immediately and after a bounded health window. The current Supervisor does not expose an MCP JSON-RPC request channel, so M5.7 does not claim protocol-level `initialize` verification.

Activation/restart/verification failure triggers automatic rollback. Studio stops the new runtime when necessary, validates and atomically restores the known-good rollback binary, restores the previous stopped/running state, and re-verifies the source binary version plus running-health window. Rollback failure is explicit. Startup removes unfinished activation/rollback-partial scratch files but preserves finalized rollback material for interrupted-transaction recovery.

M5.8 adds `GatewayUpdateManager` rather than routing Gateway through the generic M5.7 child-MCP lifecycle. On the deployed runtime, tunnel-client owns Gateway through the tunnel YAML `mcp.commands` main-channel command. Studio therefore validates that the server-owned tunnel config binds exactly to the catalog-derived `bin/rust-mcp-gateway` and `runtime/gateway/servers.d`, snapshots every generated config file by SHA-256, and validates known managed child commands against their flat `bin_root` catalog targets before any control-path stop. Browser prepare/apply endpoints are unchanged and dispatch Gateway to the dedicated manager; no browser-supplied runtime/config path is accepted.

For a running tunnel, Gateway activation intentionally stops the tunnel owner, prepares rollback, atomically replaces only the Gateway binary, and then probes the new Gateway as a temporary standalone stdio MCP server against the preserved `servers.d`. The probe requires MCP initialize, `tools/list`, all built-in Gateway control tools, structured `gateway_list_servers` output, matching configured/enabled child counts, no failed enabled child, every enabled child running, and a non-empty exposed child-tool catalog when enabled children exist. `servers.d` hashes and child metadata must remain unchanged throughout. A tunnel that was stopped before update remains stopped.

After a successful standalone probe, a previously running tunnel is restarted through `TunnelSupervisor`. `Starting`/`Stopping` are treated as intentional reconnect states inside a bounded timeout rather than immediate permanent failure. Success requires the owner to reach `Running`, remain `Running` through a health window, and still point at the trusted Gateway/config paths. Activation/probe/reconnect failure restores the prior Gateway binary, re-runs the old Gateway protocol/catalog probe, restores the previous tunnel ownership state, and exposes explicit rollback failure when recovery cannot be proven. Gateway-specific phases extend the common transaction model with `gateway_stopping`, `gateway_restarting`, `gateway_reconnecting`, and `gateway_catalog_verifying`.


M5.9 adds FleetUpdateManager. Fleet remains a non-running control bundle; Studio owns the update transaction as data rather than invoking Fleet to update itself. The trusted release owns only VERSION, fleet.toml, README.md, scripts/fleetctl.py, and hosts/mirin.example.toml. The deployed active non-example hosts/<host>.toml profile and optional state/ tree are host-local mutable state and are copied byte-for-byte with permission bits preserved into the candidate bundle after release generic files. Unexpected runtime entries and unsafe links fail closed rather than being silently discarded.

Fleet schema compatibility is explicit: M5.9 accepts Fleet schema 2 and host schema 1. The host filename must match host_id, and its bin_root/runtime_root must resolve to the trusted Studio catalog roots. No profile migration is attempted; incompatible schemas fail before activation and require a future explicit migrator/backup contract. The current Aira legacy Fleet deployment lacks VERSION; M5.9 permits a trusted explicit update from that state while retaining the entire previous directory as rollback material, instead of inferring installed version from source Git.

Before activation, the staged Fleet tooling itself runs read-only render-gateway, render-studio, and render-tunnel --check commands against the preserved active profile with fixed arguments, a minimal environment, and bounded timeouts. Generated Gateway/Studio/tunnel configs are never rewritten by Fleet update. The candidate generic control files are SHA-256 bound at prepare/apply, including non-executable metadata such as fleet.toml and VERSION.

Activation builds a complete sibling candidate, renames active runtime/fleet to a transaction rollback directory, and renames the candidate into runtime/fleet. Success requires target VERSION, byte-identical local profile/state, schema checks, and the same render validation from the activated tooling. Verification failure quarantines the new bundle, restores the prior directory, re-verifies the prior optional version/local state/render compatibility, and reports explicit rollback failure when recovery cannot be proven. Startup removes stale candidate/failed directories and restores a single finalized Fleet rollback if the active directory is missing.

M5.9A treats generated runtime configuration as desired-state inventory. `RuntimeReconciler` obtains deterministic desired bytes from the deployed Fleet `render-plan --host <host> --json` pure-render contract rather than reimplementing renderer semantics in Rust. It validates the trusted runtime root, a closed five-surface set, ownership/effect metadata, base64 content and SHA-256 before comparing active files.

Studio persists `runtime/fleet/state/reconciliation.json` as a versioned, atomically replaced managed-state manifest containing host identity, generation, per-surface last-known-managed SHA-256 and the local Gateway catalog fingerprint. Runtime state is classified as synchronized, managed-safe drift, unmanaged conflict or broken. A legacy runtime is auto-adopted only when every managed active byte already equals the trusted desired render and canonical binding/catalog proof succeeds; otherwise explicit `POST /api/reconciliation/adopt` records the current bytes without rewriting them.

Safe repair snapshots only changed Fleet-owned generated files into permission-confined rollback material, atomically replaces those files, rerenders to prove desired state did not change mid-transaction, and verifies active bytes. Evaluation itself does not persist baseline changes. Reconciliation check/adopt/apply share the catalog-scoped runtime-operation lease with Fleet/Gateway/Tunnel/Studio update activation; distinct ordinary MCP component updates retain per-component concurrency. The durable manifest is the reconciliation commit boundary, and a manifest-write failure after file mutation enters compensation that restores both files and the prior baseline. Gateway-only changes rely on the Gateway's default config watcher and a bounded watcher window; tunnel configuration changes restart the tunnel only when it was previously running, while a stopped tunnel remains stopped. A post-write failure restores captured bytes and prior tunnel state and re-verifies the restored catalog before rollback is accepted.

The current `runtime/tunnel-client/run.sh` is validate-only because Fleet does not emit it. Validation compares its complete bytes with a small explicit set of server-derived canonical templates; substring matches, comments, shadow assignments, duplicate/overriding arguments, wrong roots, symlinks, and unrecognized shell forms are rejected. It may not mask stale canonical tunnel YAML and is never auto-rewritten. Studio also records the canonical path, digest of the exact config bytes parsed at startup, and process instance. Reconciliation persists `studio_restart_pending` until a different ready process proves it loaded the exact reconciled `studio.config` bytes from the canonical path; file convergence and runtime activation are therefore separate facts. Exact local Gateway catalog identity is now proved from two independent observations. Studio reads the trusted `servers.d` snapshot, directly starts each enabled deployed child MCP from the trusted flat `bin/` root, consumes every `tools/list` page through the MCP client, applies allowlist-before-prefix semantics itself, and constructs an exact expected tool-name set including Gateway built-ins. A separate temporary Gateway process is then protocol-probed and its complete tool-name set must exactly equal that independently derived set; equal counts, substitutions, missing/extra names, duplicate names, invalid allowlists, prefix collisions, untrusted child commands, and disabled-child leakage fail closed. Remote-client freshness remains `unknown` or `refresh_pending` without an observable acknowledgement.

The browser-safe reconciliation surface is `GET /api/reconciliation` plus same-origin `POST /api/reconciliation/{check,adopt,apply}` and typed `reconciliation_changed` realtime events. Public status exposes only host/state/surface names, lifecycle impact, generation, catalog fingerprint, client-freshness state and sanitized errors; it does not expose desired bytes, filesystem authority or raw config.

The update/reconciliation catalog does not replace lifecycle ownership: orchestration must coordinate with the existing MCP and tunnel supervisors rather than creating a second process-control path.

M5.10 adds `SelfUpdateManager` for Studio. The active release unit is versioned below `runtime/studio/releases/vX.Y.Z` and contains both `mcp-studio` and its matching `web/dist`; `runtime/studio/current` is a relative symlink into that directory. `studio.toml`, registry/data, reconciliation state, self-update journal and other host-local files remain outside release directories. The legacy flat `runtime/studio/mcp-studio` is retained only as first-migration/bootstrap rollback material.

Prepare reuses trusted release discovery and M5.5 staging, copies the verified Studio package into a server-named candidate, validates binary version plus `web/dist/index.html`, fingerprints the bounded release tree, and persists a schema-versioned transaction under `runtime/studio/data/self-update`. Apply revalidates staging/candidate/source identity and accepts self-update only from the deployed Studio executable, never a source/debug process.

Studio does not replace or terminate itself directly. Before handoff it requires the deployed Fleet `studio-contract` protocol v2 capability. It then starts `studio-activate` with only server-selected host identity, durable transaction ID and Studio's own PID, returns `activation_pending`, then requests graceful server shutdown. Fleet holds a cross-process activation lock, waits for that PID to exit, atomically promotes/switches `current`, and launches the replacement with `runtime/studio/studio.toml`. Each target/rollback spawn receives a fresh Fleet-generated activation nonce. Only after the spawned Studio successfully binds its listener does it write a server-derived ready proof containing that nonce, its PID, canonical config path and SHA-256 of the exact config bytes it parsed. Fleet requires that proof to match the `Popen` instance, the expected release fingerprint, and the stable `/health` result before any terminal success. An unrelated listener that merely returns the target version cannot complete the transaction even while another spawned child remains alive. Fleet is the sole terminal-state writer; Studio startup only observes durable nonterminal/terminal state. Health, proof, launcher, journal-revision, or post-switch failure triggers rollback to the persisted previous versioned or legacy-flat identity.

Self-update transaction state is durable rather than process-local: `GET /api/update-transactions/{id}` can recover state after reconnect, and repeated apply can re-arm an interrupted transaction after revalidating the prepared release. Schema v2 records a monotonic journal revision, Fleet launcher protocol, launcher owner and launched process/fingerprint evidence. Fleet rejects stale revisions and owns terminal transitions; Studio startup must not infer completion from local runtime identity. Aira currently has no Studio launchd label; M5.10 therefore uses the small Fleet launcher contract rather than installing a boot service. Automatic service installation/unattended restart policy remains outside M5.10.

M5.11 consumes these browser-safe contracts without introducing a second update authority layer. The Updates/Fleet panel renders all eight catalog components using `InventoryView`, maps backend transaction phases directly to progress text, and uses only the existing same-origin update/reconciliation mutation routes. The browser never sends release URLs, install paths, checksums, process IDs, launcher paths, config paths, or arbitrary commands.

Per-component UI shows installed/running/desired/latest versions, platform, host mode, installation health, drift, last release-check result, runtime state, and transaction state. Gateway and Studio receive explicit control-path reconnect warnings. M5.12 completes the roadmap Tunnel update contract through `TunnelUpdateManager`, so Tunnel now uses the same prepare/apply transaction API with official OpenAI release authority, versioned runtime activation, stopped/running state preservation, config/credential preservation, automatic rollback, and same-version verified force reinstall. Rollback is not exposed as a synthetic browser mutation: the UI reports backend automatic rollback state/result.

Reconnect recovery persists every non-terminal update transaction reference in browser local storage as `(transaction_id, component)` pairs. On initial load and each realtime reconnect, the UI refetches every pending durable transaction and current inventory/reconciliation state. Terminal events remove only their own persisted reference, so unrelated concurrent component transactions are not lost.

The reconciliation panel renders host identity/mode, affected managed surfaces, generation, local Gateway catalog fingerprint, `refresh_pending`, managed-safe drift, unmanaged conflict, and sanitized failures. Only `managed_safe_drift && safe_to_reconcile` enables reconciliation apply. Unknown local edits remain inspect-only; the UI does not auto-adopt them.

M5.11 also makes release-check fan-out concurrent across the closed eight-component catalog while preserving trusted provider mapping, last-known latest version on failure, sanitized check errors, and deterministic final inventory order. This keeps one slow provider from serially multiplying the operator check latency.

M5.12 adds `TunnelUpdateManager`. Prepare accepts only a semantic version resolved through the catalog-owned official `openai/tunnel-client` provider and M5.5 staging. Apply revalidates ready staging plus a full staged candidate-tree fingerprint before any lifecycle mutation. Active Tunnel content is confined to `runtime/tunnel-client/releases/vX.Y.Z`; `current` is an atomic relative symlink. Host-local siblings such as `config.yaml`, credentials, and other non-release state are fingerprinted before activation and must remain byte-identical through success or rollback.

A newer Tunnel version is promoted as a new release directory and `current` is atomically switched. A same-version target is intentionally allowed only for Tunnel as a verified force-reinstall/integrity-repair path: the active release is moved to transaction rollback material, the verified candidate replaces it, and the prior release is restored on failure. The launcher must remain bound lexically to `current/tunnel-client-runtime-cloudflared`, the canonical Tunnel working directory, and canonical config. Each owned spawn records process generation, resolved executable/config/working directory and runtime SHA-256; running activation requires a new generation whose evidence matches the activated bytes through the health window. A stopped Tunnel remains stopped. Every mutation is journaled under `runtime/studio/data/tunnel-update/` before destructive operations. Startup restores journaled uncommitted activations to the verified predecessor and prior owner state, while journal-less or ambiguous scratch is preserved and fails closed instead of being guessed or deleted.

M5.12 closure evidence proves the incident-derived managed-drift repair/rollback sequence, official OpenAI v0.0.14 staging and force-reinstall on an isolated runtime, update/tamper/recovery matrices, repeated cleanup/soak checks, and full backend/UI/Fleet/release-build gates. The 2026-09-19 public-release 404 condition was later resolved: on 2026-09-21 `scripts/verify-m5-publication.py` qualified the complete darwin-arm64 project release set, source-less bootstrap, and Fleet 0.2.0 → 0.2.1 published transition. See `docs/milestone-5-status.md`.

## Persistent registry

M4 uses a versioned TOML registry, defaulting to:

```text
data/registry.toml
```

The registry owns:

- schema version;
- configured MCP-root identity;
- stable server IDs;
- display name and enabled state;
- project/runtime metadata;
- structured project-relative executable and working-directory paths;
- structured argument arrays;
- inherited server-side environment values where legacy configuration requires them.

Browser DTOs omit environment values.

Writes are complete-document copy-on-write updates:

```text
validate candidate state
→ deterministic serialize
→ write sibling temp file
→ fsync temp file
→ atomic rename
→ publish candidate as live in-memory state
```

Malformed or unsupported existing registry files fail startup and are never silently overwritten.

When no registry exists, existing `[mcp.*]` configuration is converted once into schema-v1 records. Once the persistent registry exists it is authoritative.

## MCP-root and executable policy

The configured MCP root is canonicalized when the registry opens.

Persistent MCP records store:

- project path relative to MCP root;
- executable path relative to the registered project;
- working directory relative to the registered project;
- arguments as an explicit list.

Studio rejects absolute paths, `..`, root/prefix escapes, and symlink components in project/working-directory/executable paths. The project must remain under the configured MCP root; working directory and executable must remain under that project.

The executable policy is validated on mutation and validated again immediately before spawn. The executable must be a regular file at activation time.

There is no raw shell-command representation and no shell interpolation.

## Discovery

M4 discovery scans direct child directories of the configured MCP root. It never invokes project code, package managers, compilers, interpreters, install hooks, shell commands, or imports.

Supported metadata:

- Rust: `Cargo.toml` package/default-run/bin metadata.
- Node: `package.json` package name and project-local `bin` entries.
- Python: `pyproject.toml` project/script metadata, with only existing project-local `.venv` script paths offered as executable candidates.

Known non-targets such as `studio`, `tunnel-client`, `gateway`, the nested infrastructure container, hidden directories, build output, and common generated directories are ignored.

A malformed or unreadable project is isolated so it cannot abort the rest of a scan.

Discovery returns preview candidates only. Registration is a separate same-origin mutation; the server rescans the candidate and only accepts an executable candidate produced by discovery. Registration never starts a process.

## Dynamic registry/runtime reconciliation

The supervisor and API share one live registry object. The supervisor does not own an immutable startup-only configuration snapshot.

Transient runtime state is created lazily for registered IDs.

Rules:

- newly registered MCPs are immediately visible and begin stopped;
- disabled MCPs remain registered/visible but cannot start or restart;
- edit, disable, and unregister are rejected while a process is starting/running/stopping;
- operator must stop first before those mutations;
- unregister removes Studio registration and inactive runtime state only;
- unregister never deletes source code or arbitrary files;
- shutdown enumerates the current registry and stops Studio-owned active children.

This avoids a separate registry-reload phase that could diverge after persistence succeeds.

## Lifecycle domains and ownership

MCP and tunnel use the same operational state vocabulary but remain separate supervisors:

```text
STOPPED
   | start
   v
STARTING
   | spawn
   v
RUNNING --------------------+
   | stop                    | unexpected exit / wait failure
   v                         v
STOPPING                   FAILED
   |                         |
   +---------- exit -------->+
```

Studio may signal only PIDs obtained from children it spawned and currently tracks. Browser APIs never accept a PID.

Unix stop remains SIGTERM + bounded wait + SIGKILL fallback.

## Tunnel boundary

Tunnel lifecycle remains independent from the MCP registry. M4 does not generalize tunnel configuration into a plugin/registry framework.

Tunnel startup, path confinement, secret references, and log redaction remain as defined by ADR 0003.

## HTTP API

M5.1 adds no public update endpoints; M5.6 owns the inventory/drift API boundary.

```text
GET  /health
GET  /api/status

GET  /api/mcp
GET  /api/mcp/{id}
POST /api/mcp/{id}/start
POST /api/mcp/{id}/stop
POST /api/mcp/{id}/restart
GET  /api/mcp/{id}/logs

GET    /api/registry
GET    /api/registry/{id}
PUT    /api/registry/{id}
DELETE /api/registry/{id}
POST   /api/registry/{id}/enable
POST   /api/registry/{id}/disable

GET  /api/discovery
POST /api/discovery/scan
POST /api/discovery/{candidate_id}/register

GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs

GET  /api/ws
```

Expected not-found/conflict/validation classes are mapped separately so callers can distinguish invalid configuration from runtime/persistence failures.

## Realtime model

Operational snapshot events remain focused on MCP process status and tunnel status.

Typed incremental events are:

```text
snapshot
process_status
log
tunnel_status
tunnel_log
registry_changed
discovery_changed
resync_required
```

Registry/discovery changes are invalidations: the browser refetches current REST state instead of receiving full configuration in WebSocket payloads.

## Browser security boundary

Studio remains loopback-only.

All lifecycle/registry/discovery/tunnel mutations and WebSocket upgrades use the existing same-origin `Origin`/`Host` protection. Local non-browser clients without an `Origin` header remain supported by design.

Remote operation remains unsupported until authentication, authorization, CSRF/CORS, TLS/exposure, and session policy are explicitly designed.

## Frontend architecture

The dashboard separates:

- Studio realtime connection state;
- MCP runtime state;
- tunnel runtime state;
- persistent registry configuration;
- discovery preview/approval.

Registry UI exposes only safe metadata. It provides edit, enable/disable, and unregister actions. Discovery UI provides scan/review/register and does not auto-register or auto-start.

## Error handling

- Library boundaries return typed `StudioError` values.
- Expected lifecycle/registry conflicts do not panic.
- Browser same-origin failures return `403 Forbidden`.
- Registry/path validation failures are rejected before mutation or spawn.
- Persistence failure leaves the previous in-memory registry authoritative because the new state is published only after successful atomic replacement.
- Per-project discovery parse/read failures do not terminate the whole scan.

## Security boundary summary

1. Studio HTTP remains loopback-only.
2. Browser privileged operations are same-origin constrained.
3. Discovery is metadata-only and never grants execution automatically.
4. Browser APIs accept no raw shell commands or PIDs.
5. MCP executable authority is confined to a registered project under the configured MCP root.
6. Project/working/executable paths reject traversal and symlink components and are revalidated before spawn.
7. Studio signals only child PIDs it owns.
8. Registry browser views/events omit environment/secret values.
9. Unregister never deletes source code.
10. Tunnel remains a separate constrained lifecycle/secret domain.

## Evolution rule

Changes that materially affect process ownership, executable/root policy, symlink policy, registry persistence/migration, discovery approval, browser-origin policy, authentication, tunnel invocation/secret policy, gateway behavior, or external API contracts require architecture/threat review and may require a new ADR.
