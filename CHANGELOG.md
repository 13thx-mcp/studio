# Changelog

All notable changes to MCP Studio will be documented here.

The project follows Semantic Versioning once public/pre-release artifacts begin.

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
