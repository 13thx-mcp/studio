# Changelog

All notable changes to MCP Studio will be documented here.

The project follows Semantic Versioning once public/pre-release artifacts begin.

## [Unreleased]

### Added

- M8.1 automation foundation with backward-compatible manual/disabled policy defaults, bounded UTC scheduling, persistent circuit/deferral state, policy/config fingerprinting, and a single cancellable AutomationController.
- Private schema-v1 automation state under `runtime/studio/data/automation/state.json` with 256 KiB bounds, symlink/root confinement, atomic fsync replacement, and fail-closed corrupt/future-schema handling.
- Shared `OperationService` audit admission/terminal boundary used by existing API mutations and injected into AutomationController.
- Runtime-operation read-only ownership snapshots and bounded coordinator-busy deferral/backoff without introducing a durable job/replay queue.

### Verified

- M8.1 exact-source qualification at `2718ab2` passes 308 Studio tests plus full M5/M6/M7 cross-repository/runtime-only regression gates.

## [0.7.1] - 2026-09-22

### Added

- Exact-source M7 final reconciliation runner with cross-repository provenance, pinned Rust toolchain preflight, Fleet pure-render validation, component gates, targeted regressions, runtime-only smoke, and M5/M6 regression execution.
- Permanent tunnel lifecycle regression proving an already-running tunnel can still be stopped if its runtime executable disappears.

### Fixed

- Runtime-only reconciliation fixture now satisfies the current Fleet tunnel identity contract.
- Fleet render-plan process failures retain bounded stderr diagnostics instead of collapsing to an opaque failure.
- Tunnel child startup rejects configured and removes inherited `MCP_COMMAND`, `MCP_SERVER_URL`, and `CONTROL_PLANE_POLL_CHANNELS` overrides after applying explicit tunnel environment, preserving validated config as the sole MCP target/poll-channel authority.
- M5/M6/M7 roadmap and release-state documentation are reconciled to the current source and remote tag state.

### Verified

- Clean-source M7 qualification covers Studio, Gateway, Filesystem, Exec, Git and Fleet plus the Studio web build, Gateway bounded scheduler soak, Filesystem CAS conflicts, Exec cancellation, Fleet pure render-plan, runtime-only reconciliation, and M5/M6 regressions.
- Remote publication refresh proves `v0.7.0-beta` and `v0.7.0` are published immutable Studio tags; the closure patch therefore advances to `0.7.1` rather than rewriting either tag.

## [0.7.0-beta] - 2026-09-22

### Added

- M7 Gateway coordination: bounded admission/drain/recovery, strict policy profiles, bounded artifacts/resources/progress, sanitized M6 telemetry, and dual-artifact tunnel runtime support.

### Verified

- Gateway, Filesystem, Studio, and Fleet M7 regression gates run locally; publication remains intentionally separate.

## [0.6.0-alpha] - 2026-09-21

### Added

- M6 private SQLite operational history, typed audit evidence, lifecycle/update/config/drift lineage, bounded historical metrics and retention, historical APIs/realtime, and dashboard history views.
- Native Apple Silicon package qualification and rollback compatibility evidence.

### Changed

- Runtime support is macOS Apple Silicon (`darwin-arm64`) only; Intel hosts fail closed.
- GitHub Actions workflows were removed. Release packaging and M6 qualification run manually.

### Verified

- Rust format, Clippy, 263 tests, frontend lint, 31 frontend tests, and production build pass locally.

## [0.5.0] - 2026-09-19

### Added

- M5.2 read-only `13thx-mcp` GitHub release provider behind the M5.1 `ReleaseProvider` contract.
- Typed stable-release and explicit-version discovery with deterministic provider/network/metadata error mapping.
- Metadata validation for semantic `vMAJOR.MINOR.PATCH` tags, draft/prerelease exclusion, trusted repository-bound asset URLs, and exactly one `SHA256SUMS.txt`.
- Bounded HTTPS GitHub client behavior with fixed API origin, connect/request timeouts, response-size caps, and a Studio-specific User-Agent.
- Hermetic provider tests covering latest/specific releases, prerelease policy, malformed metadata, checksum-manifest ambiguity, transport/HTTP failures, trusted repository mapping, and asset-list preservation.
- ADR 0007 documenting the GitHub provider trust boundary and M5.2/M5.4/M5.5 separation.
- M5.3 official `openai/tunnel-client` release provider behind the same M5.1 `ReleaseProvider` abstraction.
- Upstream runtime-cloudflared asset-family validation for `tunnel-client-runtime-cloudflared-v<version>-<os>-<arch>.zip`, while preserving evidence sidecars and other release assets as metadata.
- Shared hardened GitHub HTTP transport used by both `github_13thx` and `github_openai` providers.
- Stable tunnel runtime identity constants for later platform selection, staging, and extracted binary version validation.
- ADR 0008 documenting the official upstream trust boundary and runtime-cloudflared release-family policy.
- M5.4 strict host-platform normalization for Darwin amd64/arm64 plus deterministic exact release-asset selection across MCP, Studio, Fleet, and tunnel packages.
- Explicit trusted release asset contracts on each managed component, including architecture-independent Fleet packaging.
- ADR 0009 documenting platform normalization and exact archive-selection policy.
- M5.5 verified artifact staging with bounded HTTPS downloads, SHA-256 manifest validation, safe tar.gz/ZIP extraction, strict package structure checks, transient `staged.json` identity, and official tunnel staged `--version` validation.
- ADR 0010 documenting the staging trust boundary, partial/ready directory lifecycle, archive safety rules, and guarantees available to later activation logic.
- M5.6 runtime-artifact inventory and deterministic drift service with installed/running/desired/latest version dimensions, runtime-only/source-present host mode, process-local release-check state, and browser-safe update inventory APIs.
- Server-side `[updates]` root configuration plus optional manual desired-version pins, with conventional `mcp-server/{bin,runtime}` root discovery for backwards-compatible runtime configs.
- `GET /api/updates`, `GET /api/updates/{component}`, same-origin `POST /api/updates/check`, and typed `updates_changed` realtime invalidation.
- ADR 0011 documenting inventory truth sources, preserve-current desired policy, running-identity uncertainty, and API projection boundaries.
- M5.7 transactional updates for generic non-Gateway flat Rust MCP binaries with two-phase prepare/apply, per-component locking, Supervisor state preservation, atomic same-directory activation, mandatory version/health verification, and automatic rollback.
- Ready staging metadata now binds validated executable SHA-256 values; M5.7 revalidates archive/executable hashes and the preparation fingerprint immediately before activation.
- Browser-safe update transaction API/realtime progress plus ADR 0012 documenting flat-MCP activation, rollback, interruption recovery, and the M5.8 Gateway handoff.
- M5.8 Gateway-specific update orchestration that follows the deployed tunnel-owned process model, preserves generated `runtime/gateway/servers.d`, protocol-probes the replacement Gateway/child catalog, performs bounded reconnect verification, and rolls back on activation/catalog/reconnect failure.
- ADR 0013 documenting Gateway ownership, exact tunnel binding, standalone protocol/catalog verification, intentional reconnect phases, and rollback semantics.
- M5.9 Fleet control-bundle transactions with legacy missing-VERSION repair, host profile/state byte preservation, schema compatibility gates, staged/active render validation, directory rollback, and source-less runtime support.
- ADR 0014 documenting Fleet generic-versus-local ownership, schema/migration policy, render validation and rollback/recovery semantics.
- M5.9A runtime configuration reconciliation with Fleet pure-render desired state, persisted managed fingerprints/generation, managed-safe drift repair, explicit adoption, validate-only launcher conflict detection, transactional config rollback, and startup drift detection.
- Browser-safe reconciliation REST/realtime status plus exact local Gateway tool-name catalog fingerprinting that distinguishes catalog identity from aggregate counts without claiming remote-client refresh.
- ADR 0015 documenting Fleet-managed generated-config ownership, pure-render authority, managed-state adoption, split launch-path validation and reconciliation rollback semantics.
- M5.10 durable Studio self-update transactions with verified backend+web release candidates, versioned `runtime/studio/current -> releases/vX.Y.Z` activation, Fleet-owned external launcher handoff, reconnect-safe transaction recovery, startup finalization, and legacy/versioned rollback.
- ADR 0016 documenting Studio/Fleet activation ownership, versioned release layout, durable self-update journal, health proof, reboot/interruption recovery, and bootstrap compatibility.
- M5.11 Updates/Fleet dashboard with browser-safe runtime inventory, version/drift/check visibility, typed update transaction progress, Gateway/Studio reconnect recovery, runtime reconciliation/catalog status, and managed-safe reconciliation control.
- Pending update transaction references are retained per transaction/component across browser reconnect and refetched from the authoritative transaction API; unrelated concurrent component transactions are preserved independently.
- Release metadata checks now fan out concurrently across the closed component catalog while preserving trusted provider/error semantics and deterministic inventory output.
- M5.12 official Tunnel update transactions with verified OpenAI staging, versioned `releases/vX.Y.Z` activation, atomic `current` switch, stopped/running ownership preservation, host-local config/credential preservation, automatic rollback, same-version force reinstall, and startup recovery.
- M5.12 incident-derived runtime reconciliation closure drill, source-less/runtime-only operator guides, rollback guide, Studio self-update operations guide, and detailed Milestone 5 status evidence.
- M5 review remediation adds permanent F1–F8 regressions, catalog-scoped mutation coordination, durable Tunnel crash-recovery journals, post-rename compensation, process-bound Studio/Fleet activation protocol v2 with nonce/PID/config proof, strict validate-only launcher templates, persistent loaded-config restart truth, and independently derived exact Gateway tool catalogs.
- Fixed-tree remediation verification tooling now records Studio/Fleet provenance, validates that required filtered tests actually executed, enforces per-command process-group timeouts/cleanup, archives raw gate logs without build trees, and verifies the current native Darwin package using the same backend+web layout as the release workflow.

### Security

- Browser/caller data cannot select GitHub repository coordinates or API base URLs; provider lookups re-resolve component policy through the server-owned catalog.
- Release asset URLs are accepted only when they are HTTPS GitHub release-download URLs under the trusted catalog owner/repository.
- M5.2 performs no release archive download, extraction, activation, restart, or rollback.
- The tunnel provider accepts only the catalog-owned `openai/tunnel-client` source and rejects caller-forged repository identity, malformed/mismatched runtime ZIPs, and ambiguous platform targets.
- M5.3 performs no release archive download, checksum verification, extraction, activation, restart, or rollback.
- M5.4 rejects unsupported hosts, missing/duplicate exact archive names, release/component mismatches, wrong-version/architecture lookalikes, and architecture fallback; it still performs no archive download or runtime mutation.
- M5.5 rejects checksum mismatches, malformed manifests, oversized/corrupt archives, traversal/absolute paths, duplicate paths, symlink/hardlink/special entries, unexpected package layouts, and tunnel version mismatch before any active installation change.
- M5.6 inventory probes only catalog-derived runtime artifacts, never source Git/manifests; update check errors are sanitized before browser publication and inventory DTOs omit internal paths/repository/download authority.
- M5.7 rejects arbitrary update paths/URLs, stale/tampered staged artifacts, registry/catalog target mismatch, duplicate same-component transactions, and unsafe restart behavior; known-good rollback material is checksum protected and verified after restoration.
- M5.8 rejects untrusted/ambiguous tunnel-to-Gateway bindings, generated-config mutation, managed child commands outside catalog flat-bin targets, duplicate Gateway transactions, failed child catalogs, reconnect timeout, and unverifiable rollback.
- M5.9 rejects Fleet generic-file TOCTOU, example-profile overwrite, unknown schema, untrusted host roots, unexpected/unsafe local state, render drift, duplicate Fleet transactions, and unverifiable bundle rollback.
- M5.9A rejects untrusted render-plan roots/paths/hashes/effects, symlink or special-file managed targets, unknown local edits, stale/noncanonical launcher bindings, cross-origin reconciliation mutations, equal-count catalog identity substitution, and unverifiable config rollback.
- M5.10 rejects source/debug self-update callers, browser-provided activation authority, staged/candidate tamper, unsafe release trees/current symlinks, mismatched backend/web release identity, launcher unavailability, and unverifiable self-update rollback.
- M5.11 exposes no browser release/path/PID/command/checksum authority and only permits reconciliation apply from backend-approved managed-safe drift.
- M5.12 Tunnel activation accepts only official catalog-owned OpenAI releases, revalidates staged/candidate identity before stop, confines mutation to versioned release/current paths, fingerprints host-local config/credentials, rejects downgrade/tamper/unsafe pointers, and makes rollback failure explicit.
- M5 review remediation rejects stale fixed-release Tunnel launch bindings, unjournaled/ambiguous crash recovery, premature Studio self-update terminal authority, unrelated-listener readiness substitution, launcher shadowing, false-cleared Studio restart state, and equal-count/wrong-name Gateway catalogs.

## [0.4.0] - 2026-09-17

### Milestone

- Milestone 4 — Registry, Configuration & Auto-Discovery — completed and closed.

### Added

- Schema-versioned persistent MCP registry backed by `data/registry.toml` by default.
- Atomic registry persistence using sibling temporary files, file sync, rename, and fail-safe publication.
- Deterministic one-time bootstrap from legacy `[mcp.*]` configuration when no persistent registry exists.
- Configurable canonicalized MCP root and project-relative registered launch configuration.
- Metadata-only discovery for Rust (`Cargo.toml`), Node (`package.json`), and Python (`pyproject.toml`) projects.
- Explicit discovery preview and operator approval before registration.
- Dynamic registration without Studio source changes or restart.
- Registry edit, enable/disable, and unregister operations.
- Stop-first conflict semantics for edit, disable, and unregister while a process is active.
- Dynamic supervisor integration with lazy runtime state for newly registered MCPs.
- Registry REST API and discovery REST API.
- Typed `registry_changed` and `discovery_changed` realtime invalidation events.
- Registry and Discovery dashboard sections.
- Backend coverage for persistence, restart survival, duplicate project rejection, malformed manifests, traversal, symlink escape, disabled-start rejection, and source preservation.
- Frontend registry/discovery request tests.
- ADR 0004 for file-backed registry/path confinement policy.
- ADR 0005 for live registry/supervisor reconciliation and discovery approval model.
- Milestone 4 design and closure documentation.

### Changed

- MCP Supervisor now reads from a live persistent registry instead of an immutable startup-only static map.
- Browser-safe registry DTOs expose structured launch metadata but omit stored environment values.
- Registered executable and working-directory paths are validated on mutation and again immediately before spawn.
- Discovery ignores infrastructure/build/generated/plain directories that are not supported MCP projects.
- Rust/Python discovery uses document TOML decoding and malformed supported manifests are isolated as warnings.
- Supervisor lifecycle test fixtures now use project-local executable scripts with collision-safe temporary paths.
- README, architecture, threat model, example configuration, and package versions updated for M4.
- Rust and frontend package versions advanced to `0.4.0`.

### Verified

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
- `cargo audit`
- `cargo build --release --locked`
- 24/24 Rust library unit tests passed.
- 4/4 registry/discovery integration tests passed.
- 7/7 supervisor lifecycle integration tests passed.
- 7/7 tunnel lifecycle regression tests passed.
- Supervisor lifecycle integration test repeated 5 times without failure.
- Registry/discovery integration test repeated 5 times without failure.
- `pnpm install`
- `pnpm lint`
- `pnpm typecheck`
- `pnpm test` — 4 test files / 12 tests passed.
- `pnpm build`
- Real workspace discovery detected Blender, Filesystem, and Git while ignoring Studio, tunnel-client, and gateway.
- Real Filesystem discovery → approval → registration → stopped → start → stop workflow passed.
- Registry configuration survived Studio restart.
- Disabled state survived Studio restart and start was rejected with HTTP 409 while disabled.
- Enable → start → stop passed after disabled-state recovery.
- WebSocket initial and reconnect snapshots passed.
- Real tunnel start → restart → stop regression passed with PID/state/restart-count checks.
- Unregister returned 404 afterward and remained absent after Studio restart.
- Filesystem source hash was unchanged before/after unregister.

### Security

- Discovery is metadata-only and does not execute, import, install, build, or run discovered project code.
- Registration requires an explicit operator action and never starts the MCP automatically.
- Absolute, traversal, prefix/root, and symlink-component execution paths are rejected.
- Executable and working-directory resolution is confined below the registered project and configured MCP root.
- Launch arguments remain structured argv and are never shell-interpolated.
- Browser registry responses/events omit stored environment values.
- Registry/discovery mutations retain same-origin browser protection.
- Disabled MCPs cannot start/restart.
- Edit/disable/unregister cannot mutate active processes silently.
- Unregister removes Studio registration only and never deletes project source code.
- Tunnel lifecycle remains a separate constrained domain and was regression-tested unchanged.
- Studio remains loopback-only.

## [0.3.0] - 2026-09-16

### Milestone

- Milestone 3 — Secure Tunnel Management — completed and closed.

### Added

- Dedicated secure tunnel supervisor with start, stop, restart, PID, uptime, restart count, crash count, last exit, and last runtime error.
- Single configured tunnel runtime model for `tunnel-client-runtime-cloudflared`.
- Constrained tunnel startup using internally constructed `run --config <validated-config-file>` arguments.
- Tunnel runtime/config path canonicalization and confinement under the configured tunnel working directory.
- Server-side tunnel environment/file secret references.
- Bounded recent tunnel stdout/stderr/Studio log capture.
- Tunnel log ANSI cleanup and secret redaction before REST/WebSocket/dashboard publication.
- Tunnel REST endpoints for status, lifecycle control, and recent logs.
- Typed `tunnel_status` and `tunnel_log` realtime events.
- Combined reconnect/resync snapshot containing both MCP and tunnel status.
- Secure tunnel dashboard section with lifecycle controls, runtime availability, PID, uptime, counters, errors, and realtime logs.
- Tunnel lifecycle integration tests covering start/stop/restart, PID replacement, duplicate start, stopped-state rejection, invalid runtime, unexpected exits, realtime events, secret redaction, and shutdown cleanup.
- Frontend verification for tunnel state/actions, log reconciliation, realtime updates, and reconnect snapshot modeling.
- ADR 0003 documenting secure tunnel supervision and fixed runtime invocation policy.
- Milestone 3 design and closure documentation.

### Changed

- Studio graceful shutdown now cleans up the Studio-owned tunnel process as well as MCP processes.
- Realtime snapshots now include current tunnel status.
- Dashboard now visibly separates Studio realtime connection state, MCP lifecycle state, and tunnel lifecycle state.
- Unexpected tunnel exit is treated as a crash even when the child exits with status `0`; only an explicit-stop exit transitions normally to `stopped`.
- Tunnel startup validation/secret-resolution failures transition tunnel state to `failed` with `last_error` populated.
- README, architecture, and threat model now describe active tunnel management.
- Rust and frontend package versions advanced to `0.3.0`.

### Verified

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
- `cargo audit`
- `cargo build --release --locked`
- Repeated parallel `cargo test --test tunnel_lifecycle` runs after fixture-isolation hardening.
- `pnpm install`
- `pnpm lint`
- `pnpm typecheck`
- `pnpm test`
- `pnpm build`
- Real `tunnel-client-runtime-cloudflared` lifecycle smoke test: start, running PID/uptime, restart PID replacement, restart counter, crash counter stability, stop, and PID clearing.
- Real Studio shutdown cleanup: Studio-owned tunnel PID was no longer present after shutdown.

### Security

- Tunnel lifecycle APIs accept no caller-supplied executable path, PID, raw shell command, config path, or arbitrary argv.
- Tunnel runtime and config files are validated and confined to the configured tunnel working directory.
- Studio signals only the tunnel PID it spawned and currently tracks.
- Tunnel environment/file secret references remain server-side and are not serialized into tunnel status or frontend types.
- Tunnel logs redact resolved secret values and common secret-bearing fields before storage/streaming.
- Tunnel lifecycle browser mutations retain same-origin enforcement.
- Studio remains loopback-only.

## [0.2.0] - 2026-09-16

### Milestone

- Milestone 2 — MVP Web Dashboard — completed and closed.

### Added

- React + TypeScript + Vite dashboard under `web/`.
- Dashboard overview with managed/running/failed/restart summaries.
- MCP server detail view with PID, live uptime, restart/crash counters, last exit, and last error.
- Browser start/stop/restart controls derived from supervisor lifecycle state.
- WebSocket endpoint at `GET /api/ws`.
- Typed realtime event model with initial snapshots, process status events, log events, and resync notifications.
- Broadcast-backed runtime event hub.
- Automatic frontend WebSocket reconnect with bounded exponential backoff.
- Live stdout/stderr/Studio log viewer with stream filtering, auto-scroll, and browser-local clear-view.
- Monotonic per-MCP log sequence numbers for REST/WebSocket deduplication.
- Production SPA serving from Axum using `web/dist` with index fallback.
- Vite development proxy for REST and WebSocket traffic.
- Frontend lint/typecheck/test/build toolchain and Vitest coverage for state helpers and reconnect behavior.
- Project `Makefile` for common install, development, verification, release, and run workflows.
- Milestone 2 architecture, threat-model, and status documentation.

### Changed

- Browser lifecycle mutations and WebSocket upgrades now enforce a same-origin `Origin`/`Host` policy when an Origin header is present.
- Dashboard uptime advances locally between backend status updates.
- Child-process ANSI terminal control sequences are stripped before logs are stored and streamed.
- Dashboard rendering removes duplicated tracing timestamp/level prefixes from child log messages while retaining table timestamp/stream metadata.
- Frontend dependency versions used by the lint/typecheck toolchain are pinned to a mutually compatible set.
- Package version advanced to `0.2.0`.

### Verified

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
- `cargo audit`
- `cargo build --release --locked`
- `pnpm install`
- `pnpm lint`
- `pnpm typecheck`
- `pnpm test`
- `pnpm build`
- Manual browser smoke test with the real Filesystem MCP, including start, restart, stop, live state/log updates, refresh reconciliation, and WebSocket reconnect.

### Security

- Studio remains loopback-only.
- Lifecycle APIs still accept only configured MCP identifiers, never caller-supplied PIDs or executable commands.
- Browser mutation and WebSocket paths reject foreign origins.
- Wildcard CORS is not enabled.
- Process status and dashboard data do not expose MCP environment configuration values.

## [0.1.0] - 2026-09-16

### Milestone

- Milestone 1 — Core Process Supervisor — completed and closed.

### Added

- Static MCP registry for the initial Blender and Filesystem servers.
- MCP lifecycle states for stopped, starting, running, stopping, and failed.
- Start, stop, and restart operations.
- PID, uptime, exit-code, restart-count, crash-count, and last-error reporting.
- Unix graceful termination using `SIGTERM` with configurable timeout and `SIGKILL` fallback.
- Process-generation guard to prevent stale monitor tasks from overwriting newer runtime state.
- stdout/stderr capture into bounded per-MCP in-memory log buffers.
- Studio shutdown cleanup for Studio-owned MCP child processes.
- REST endpoints for MCP status, lifecycle control, and recent logs.
- Unix lifecycle integration tests covering start/stop, restart, crash detection, duplicate start, invalid executable, already-stopped behavior, and shutdown cleanup.
- ADR 0002 documenting the process ownership and supervision model.
- Milestone 1 status and verification checklist.

### Changed

- Package version advanced to `0.1.0`.
- Tokio features extended for child processes, asynchronous I/O, timers, and synchronization.
- Added `nix` signal support for Unix process termination.
- Studio example configuration now includes static Blender/Filesystem registry entries, log capacity, and stop timeout.
- Architecture and threat-model documents updated for active process supervision.
- Supervisor shutdown logic adjusted to satisfy the zero-warning clippy gate on Rust `1.98.1`.

### Verified

- `cargo check`
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
- `cargo audit`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test --locked --all-targets --all-features`
- `cargo build --locked --all-targets --all-features`
- Automated tests: `11 passed; 0 failed`.
- Manual MCP lifecycle API smoke test and Studio-owned child cleanup on shutdown.

### Security

- Lifecycle APIs operate on configured MCP IDs and never accept caller-supplied PIDs.
- Studio only signals processes it spawned and currently owns.
- MCP commands are structured executable + argument configuration; lifecycle API does not expose arbitrary shell execution.
- Log buffers are bounded to limit memory growth.

## [0.0.1] - 2026-09-16

### Milestone

- Milestone 0 — Foundation & SDLC Bootstrap — completed and closed.

### Added

- Milestone 0 SDLC and production-readiness roadmap.
- Rust/Axum foundation service.
- Localhost-only validated configuration baseline.
- Structured JSON logging.
- `/health` endpoint.
- Typed error boundary.
- Module boundaries for API, supervisor, tunnel, registry, discovery, storage, and metrics.
- Architecture document.
- Initial threat model.
- ADR process and foundation ADR.
- Contribution/development conventions.
- CI baseline for format, lint, tests, build, and dependency audit.
- Expanded `.gitignore` for Rust, runtime state, logs, frontend artifacts, secrets, local configuration, IDE/OS metadata, and generated artifacts.
- `rust-toolchain.toml` pinned to Rust `1.98.1` with `rustfmt` and `clippy`.
- Explicit MSRV policy of Rust `1.98.1`.

### Changed

- CI Rust toolchain pinned to `1.98.1` instead of the moving `stable` channel.
- CI clippy, test, and build gates use the committed Cargo lockfile.
- Logging initialization now converts subscriber initialization errors explicitly for compatibility with the selected toolchain.
- `StudioConfig` uses derived `Default` where appropriate to satisfy the zero-warning clippy gate.

### Verified

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
- `cargo audit`
- Runtime startup on `127.0.0.1:18100`
- Unit tests: `3 passed; 0 failed`

### Security

- Non-loopback listen addresses are rejected during the current foundation baseline.
- Privileged MCP process and tunnel actions are intentionally deferred to later milestones.
- Arbitrary shell execution is outside the allowed Studio control model.
