# MCP Studio

Local-first web control plane for monitoring and managing MCP servers and secure tunnels under `mcp-server/`.

## Current status

Milestone 2 — MVP Web Dashboard is in active implementation on top of the completed Milestone 1 process supervisor.

Current capabilities include:

- Static Blender and Filesystem MCP registry.
- Start / stop / restart process supervision.
- PID, state, uptime, restart-count, crash-count, exit-code, and error reporting.
- Bounded stdout/stderr/Studio log capture.
- REST lifecycle API.
- WebSocket runtime events and reconnect/resync behavior.
- React/TypeScript dashboard for lifecycle control and live status/log viewing.
- Same-origin browser protection for lifecycle mutations and WebSocket upgrades.
- Production frontend assets served by the Studio Axum service from `web/dist`.

Tunnel control, persistent registry/discovery, historical metrics, and authentication/remote mode remain deferred to later milestones in `ROADMAP.md`.

## Requirements

- Rust 1.98.1
- Cargo
- Node.js and pnpm for dashboard development/build
- Unix platform (macOS/Linux) for signal-based graceful process control

The pinned Rust toolchain is declared in `rust-toolchain.toml`.

## Build managed MCP servers first

Current defaults expect debug binaries at:

```text
../blender/target/debug/rust-mcp-blender
../filesystem/target/debug/rust-mcp-filesystem
```

Build them before starting those MCPs through Studio:

```bash
cd mcp-server/blender
cargo build

cd ../filesystem
cargo build
```

## Build the dashboard

```bash
cd mcp-server/studio/web
pnpm install
pnpm build
```

The production bundle is written to `web/dist` and served by Studio.

For frontend development, Vite proxies `/api` and WebSocket traffic to Studio on `127.0.0.1:18100`:

```bash
pnpm dev
```

## Run Studio

```bash
cd mcp-server/studio
cargo run -- --config studio.example.toml
```

Default listen address:

```text
127.0.0.1:18100
```

After building the dashboard, open:

```text
http://127.0.0.1:18100/
```

## API

```text
GET  /health
GET  /api/status
GET  /api/mcp
GET  /api/mcp/{id}
POST /api/mcp/{id}/start
POST /api/mcp/{id}/stop
POST /api/mcp/{id}/restart
GET  /api/mcp/{id}/logs
GET  /api/ws
```

`/api/ws` sends an initial process snapshot followed by process-status and log events. If the broadcast receiver lags, Studio requests resynchronization and sends a fresh snapshot.

## Dashboard

The Milestone 2 dashboard provides:

- Studio connection status.
- Managed/running/failed/restart summary.
- MCP list and per-server details.
- Start / stop / restart actions.
- PID, uptime, restart count, crash count, last exit, and last error.
- Live WebSocket updates.
- Live stdout/stderr/Studio logs.
- Stream filtering.
- Auto-scroll control.
- Local-only log-view clearing.
- Automatic WebSocket reconnect with bounded exponential backoff.

Clearing the browser log view does not mutate the supervisor's in-memory log buffer.

## Configuration

See `studio.example.toml`.

Run with a config file:

```bash
cargo run -- --config studio.example.toml
```

or:

```bash
MCP_STUDIO_CONFIG=studio.example.toml cargo run
```

Relative `command` and `working_dir` paths are resolved against Studio's process working directory.

Milestone 2 remains loopback-only. Remote access is intentionally unsupported until authentication/authorization and tunnel exposure are designed and reviewed.

## Process lifecycle

```text
STOPPED
   │ start
   ▼
STARTING
   │ spawn
   ▼
RUNNING ───────────────┐
   │ stop              │ unexpected non-zero exit / wait error
   ▼                   ▼
STOPPING             FAILED
   │
   └──── exit ─────► STOPPED
```

Stop behavior on Unix:

1. Send `SIGTERM` to the owned child PID.
2. Wait up to `stop_timeout_ms`.
3. Escalate to `SIGKILL` when the process does not exit.

Studio only signals PIDs that came from processes it spawned and currently tracks.

## Realtime model

The supervisor publishes typed runtime events through an in-process Tokio broadcast channel.

Browser clients receive:

```text
snapshot
process_status
log
resync_required
```

Each log entry carries a monotonic per-MCP sequence number so the dashboard can deduplicate REST history and WebSocket events safely.

Uptime is sampled from the backend and advanced locally in the browser between process-status events to avoid unnecessary one-second broadcast traffic.

## Security baseline

- Loopback-only HTTP control plane.
- No arbitrary shell-command API.
- Static MCP registry for the current milestone.
- Studio signals only tracked child PIDs.
- Lifecycle browser requests require same-origin `Origin`/`Host` alignment when an `Origin` header is present.
- WebSocket upgrades use the same same-origin policy.
- No wildcard CORS policy.
- MCP environment configuration is not serialized through process-status responses.
- No auto-execution of discovered projects.

Child stdout/stderr is still exposed verbatim through the logs API/dashboard. Managed MCP servers must not emit credentials; generic secret redaction remains a later hardening requirement.

## Development checks

Rust:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo build --release --locked
```

Dashboard:

```bash
cd web
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

## Engineering documents

- `ROADMAP.md` — milestones from foundation to production grade.
- `docs/architecture.md` — component boundaries and design principles.
- `docs/threat-model.md` — security baseline.
- `docs/milestone-1-status.md` — completed process-supervisor milestone evidence.
- `docs/milestone-2-status.md` — current dashboard milestone verification status.
- `docs/adr/` — architecture decision records.
- `CONTRIBUTING.md` — SDLC and development workflow.
