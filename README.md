# MCP Studio

Local-first web control plane for monitoring and managing MCP servers and the configured secure tunnel runtime under `mcp-server/`.

## Current status

Milestone 3 — Secure Tunnel Management is implemented on `feature/secure-tunnel-management`.

Automated Rust and frontend release gates pass. The remaining Milestone 3 release gate is the manual real-runtime smoke test with `../tunnel-client/tunnel-client-runtime-cloudflared`; `v0.3.0` is not considered released until that smoke test succeeds.

Current capabilities include:

- Static MCP registry and MCP lifecycle supervision.
- MCP start / stop / restart with PID ownership safety.
- Secure tunnel start / stop / restart using one validated server-side tunnel configuration.
- MCP and tunnel PID, state, uptime, restart count, crash count, last exit, and last error reporting.
- Bounded recent logs for MCP and tunnel processes.
- Tunnel secret-value redaction before logs reach REST, WebSocket, or dashboard consumers.
- REST lifecycle APIs.
- Typed WebSocket runtime events and reconnect/resync behavior.
- React/TypeScript dashboard with separate Studio connection, MCP lifecycle, and tunnel lifecycle state.
- Same-origin browser protection for lifecycle mutations and WebSocket upgrades.
- Production frontend assets served by the Studio Axum service from `web/dist`.

Persistent registry/discovery, SQLite history, remote authentication, auto-restart/backoff, and MCP gateway request telemetry remain deferred to later milestones in `ROADMAP.md`.

## Requirements

- Rust 1.98.1
- Cargo
- Node.js and pnpm for dashboard development/build
- Unix platform (macOS/Linux) for signal-based graceful process control
- Existing `mcp-server/tunnel-client` runtime bundle for tunnel lifecycle management

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

## Tunnel runtime

The default Milestone 3 configuration manages:

```text
../tunnel-client/tunnel-client-runtime-cloudflared
```

with:

```text
../tunnel-client/config.yaml
```

Studio constructs the tunnel runtime invocation internally as:

```text
tunnel-client-runtime-cloudflared run --config <validated-config-file>
```

The browser cannot supply an executable path, shell command, PID, config path, or arbitrary argument array.

Only the tunnel PID spawned and currently tracked by Studio is eligible for lifecycle signals.

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

GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs

GET  /api/ws
```

`/api/ws` sends an initial snapshot containing both MCP and tunnel status, followed by typed process/tunnel status and log events. If either broadcast receiver lags, Studio requests resynchronization and sends a fresh combined snapshot.

## Dashboard

The dashboard provides:

- Studio realtime connection state.
- MCP managed/running/failed summary.
- Secure tunnel state summary.
- MCP list and per-server details.
- Separate tunnel detail panel.
- MCP and tunnel start / stop / restart actions.
- PID, uptime, restart count, crash count, last exit, and last error.
- Live WebSocket updates.
- Live MCP and tunnel logs.
- MCP stream filtering and auto-scroll.
- Automatic WebSocket reconnect with bounded exponential backoff.

Studio connection state, MCP lifecycle state, and tunnel lifecycle state are intentionally presented as separate concepts.

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

MCP relative `command` and `working_dir` paths are resolved against Studio's process working directory.

Tunnel configuration is server-side and typed:

```toml
[tunnel]
name = "Secure tunnel"
runtime = "../tunnel-client/tunnel-client-runtime-cloudflared"
working_dir = "../tunnel-client"
config_file = "../tunnel-client/config.yaml"
```

Tunnel runtime and config paths are canonicalized and must remain under the configured tunnel working directory.

Optional tunnel environment secrets use server-side references rather than browser-provided values:

```toml
[tunnel.env]
# TUNNEL_TOKEN = { from_env = "MCP_TUNNEL_TOKEN" }
# CLOUDFLARED_CREDENTIAL = { from_file = "../secrets/cloudflared-token" }
```

Resolved secret values are not included in tunnel status, REST responses, WebSocket status events, or frontend types.

Studio remains loopback-only. Remote access is intentionally unsupported until authentication/authorization and tunnel exposure are explicitly designed and reviewed.

## Lifecycle model

MCP and tunnel lifecycle domains both use the operational states:

```text
STOPPED
   │ start
   ▼
STARTING
   │ spawn
   ▼
RUNNING ───────────────┐
   │ stop              │ unexpected exit / wait error
   ▼                   ▼
STOPPING             FAILED
   │
   └──── exit ─────► STOPPED
```

An unexpected tunnel exit is considered a crash even when the child exits with status `0`; only an exit observed while the tunnel is explicitly stopping is treated as a normal stopped transition.

Stop behavior on Unix:

1. Send `SIGTERM` to the owned child PID.
2. Wait up to `stop_timeout_ms`.
3. Escalate to `SIGKILL` when the process does not exit.

Studio only signals PIDs that came from processes it spawned and currently tracks.

## Realtime model

Browser clients receive typed events:

```text
snapshot
process_status
log
tunnel_status
tunnel_log
resync_required
```

MCP and tunnel supervisors remain separate lifecycle domains. The WebSocket layer multiplexes their bounded realtime event streams into one browser protocol.

Each log stream carries monotonic sequence numbers so the dashboard can deduplicate REST history and WebSocket events safely.

Uptime is sampled from the backend and advanced locally in the browser between status events to avoid unnecessary one-second broadcast traffic.

## Security baseline

- Loopback-only HTTP control plane.
- No arbitrary shell-command API.
- Browser APIs do not accept executable paths or PIDs.
- Tunnel argv is constructed internally from typed server-side configuration.
- Tunnel runtime/config paths are canonicalized and confined to the configured tunnel working directory.
- Studio signals only tracked child PIDs.
- Browser lifecycle requests require same-origin `Origin`/`Host` alignment when an `Origin` header is present.
- WebSocket upgrades use the same same-origin policy.
- No wildcard CORS policy.
- Tunnel secret references and resolved values remain server-side.
- Tunnel logs redact resolved secret values and common secret-bearing fields before storage/streaming.
- No auto-execution of discovered projects.

MCP stdout/stderr remains a separate domain and is not covered by the Milestone 3 tunnel redactor; managed MCP servers must continue not to emit credentials.

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
- `docs/threat-model.md` — active security model.
- `docs/milestone-1-status.md` — completed process-supervisor milestone evidence.
- `docs/milestone-2-status.md` — completed dashboard milestone evidence.
- `docs/milestone-3-design.md` — secure tunnel management design boundary.
- `docs/milestone-3-status.md` — current M3 verification and closure status.
- `docs/adr/` — architecture decision records.
- `CONTRIBUTING.md` — SDLC and development workflow.
