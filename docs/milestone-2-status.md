# Milestone 2 Status — MVP Web Dashboard

## Status

**COMPLETE — automated release gates and manual browser smoke verification passed on 2026-09-16.**

Released version: `v0.2.0`

## Implemented

### Realtime backend

- Added typed Studio runtime event model.
- Added Tokio broadcast-based event hub.
- Added WebSocket endpoint at `GET /api/ws`.
- Initial WebSocket connection sends a complete process snapshot.
- Process lifecycle transitions publish `process_status` events.
- stdout/stderr/Studio log entries publish `log` events.
- Lagged WebSocket receivers receive `resync_required` followed by a fresh snapshot.
- Log entries include a monotonic per-MCP sequence number for REST/WebSocket deduplication.
- ANSI terminal formatting is stripped before child-process logs are stored and streamed.

### Browser security boundary

- Lifecycle browser mutations validate same-origin `Origin` and `Host` when an `Origin` header is present.
- WebSocket upgrades use the same same-origin policy.
- Wildcard CORS is not enabled.
- Studio remains loopback-only.
- Process status does not serialize MCP environment configuration.

### Dashboard frontend

- React + TypeScript + Vite application under `web/`.
- Managed/running/failed/restart summary.
- MCP server list and selected-server detail view.
- Start / stop / restart controls derived from process state.
- PID, live uptime, restart count, crash count, last exit, and last error display.
- WebSocket connection status.
- Automatic reconnect using bounded exponential backoff.
- Initial REST synchronization plus WebSocket runtime updates.
- Live stdout/stderr/Studio log display.
- Log deduplication by sequence.
- Stream filtering for stdout/stderr/Studio.
- Auto-scroll toggle.
- Browser-local clear-view action that does not mutate supervisor logs.
- Tracing timestamp/level prefixes are removed from rendered child-log messages to avoid duplicating table metadata.
- Responsive desktop-first layout.

### Production serving and developer workflow

- Vite builds production assets to `web/dist`.
- Axum serves `web/dist` from the Studio service.
- SPA fallback serves `web/dist/index.html`.
- Vite development proxy forwards API and WebSocket traffic to `127.0.0.1:18100`.
- Added a project `Makefile` for install, development, build, test, audit, release, and run workflows.

## Automated Verification

The complete final automated release gates passed from the Milestone 2 source state.

### Rust

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo build --release --locked
```

Result: **PASS**.

### Frontend

```bash
cd web
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Result: **PASS**.

Frontend verification includes state-helper and reconnect/backoff coverage.

## Manual Browser Smoke Verification

A real browser smoke test with the Filesystem MCP passed.

Verified behavior:

- Studio dashboard loads from the Axum-served production frontend.
- Filesystem and Blender appear in the managed MCP list.
- Filesystem can be started from the browser.
- Running state, PID, and live uptime are shown correctly.
- Recent logs update through WebSocket without page refresh.
- ANSI terminal formatting no longer leaks into the dashboard log display.
- Stream filtering, auto-scroll, and local clear-view operate correctly.
- Filesystem restart replaces the PID and increments restart count.
- Filesystem stop reaches the stopped state.
- Browser refresh reconciles to current backend state.
- WebSocket reconnect recovers live state after interruption.

## Exit Criteria

- Dashboard overview and MCP detail workflows operate from the browser. **PASS**
- Start / stop / restart work without CLI lifecycle commands. **PASS**
- Live process state and logs update through WebSocket. **PASS**
- Browser refresh and WebSocket reconnect reconcile to backend state. **PASS**
- UI does not expose MCP environment secret values. **PASS**
- Same-origin protection remains enforced for browser mutation and WebSocket paths. **PASS**
- Rust quality gates pass from the final source state. **PASS**
- Frontend quality gates pass from the final source state. **PASS**
- `cargo audit` passes. **PASS**
- Release build succeeds with `--locked`. **PASS**
- Manual browser smoke test succeeds with a real managed MCP. **PASS**

## Out of Scope

Still deferred beyond Milestone 2:

- Tunnel lifecycle management.
- Persistent registry/discovery.
- SQLite metrics/history.
- Authentication/authorization and remote mode.
- Request-level MCP gateway telemetry.
- Generic secret-redaction engine.

## Closure Decision

Milestone 2 is formally closed because:

- the browser dashboard can manage the configured MCP servers without CLI lifecycle commands;
- REST and WebSocket state synchronization operate correctly;
- live process state and logs are visible and usable;
- browser refresh and WebSocket reconnect reconcile to backend state;
- same-origin browser protections remain enforced;
- Rust and frontend automated release gates pass;
- dependency audit and locked release build pass;
- the real Filesystem MCP browser smoke test passes;
- documentation reflects the verified `v0.2.0` behavior.

## Next Milestone

Proceed to **Milestone 3 — Secure Tunnel Management**.
