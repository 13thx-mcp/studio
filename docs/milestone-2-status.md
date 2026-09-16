# Milestone 2 Status — MVP Web Dashboard

## Status

**FINAL VERIFICATION — all automated release gates pass; manual browser smoke test remains before formal closure.**

Target version: `v0.2.0`

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
- Responsive desktop-first layout.

### Production serving

- Vite builds production assets to `web/dist`.
- Axum serves `web/dist` from the Studio service.
- SPA fallback serves `web/dist/index.html`.
- Vite development proxy forwards API and WebSocket traffic to `127.0.0.1:18100`.

## Automated Verification

The complete final automated release gates passed from the current Milestone 2 source state.

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

Frontend verification includes state-helper tests and reconnect/backoff coverage.

## Manual Browser Smoke Test

The remaining release gate is a real browser smoke test with the Filesystem MCP:

1. Build `web/dist`.
2. Start Studio from `mcp-server/studio` with a valid config.
3. Open `http://127.0.0.1:18100/`.
4. Confirm Filesystem and Blender appear.
5. Start Filesystem from the browser.
6. Confirm state becomes `running` and PID is populated.
7. Confirm uptime advances without page refresh.
8. Confirm recent Studio/stdout/stderr logs appear through WebSocket updates.
9. Exercise stream filters and auto-scroll.
10. Clear the browser log view and verify backend log history remains available after reselect/refresh.
11. Restart Filesystem and confirm PID changes and restart count increments.
12. Stop Filesystem and confirm state becomes `stopped`.
13. Refresh the browser and confirm state reconciles with the backend.
14. Interrupt/restart Studio or otherwise break the WebSocket connection and confirm the UI enters reconnecting state and recovers after the endpoint returns.

## Exit Criteria

Milestone 2 can be marked complete when:

- Dashboard overview and MCP detail workflows operate from the browser.
- Start / stop / restart work without CLI lifecycle commands.
- Live process state and logs update through WebSocket.
- Browser refresh and WebSocket reconnect reconcile to backend state.
- UI does not expose MCP environment secret values.
- Same-origin protection remains enforced for browser mutation and WebSocket paths.
- Rust quality gates pass from the final source state. **PASS**
- Frontend quality gates pass from the final source state. **PASS**
- `cargo audit` passes. **PASS**
- Release build succeeds with `--locked`. **PASS**
- Manual browser smoke test succeeds with a real managed MCP. **PENDING**
- Version and changelog are advanced to `0.2.0` only after manual verification.

## Out of Scope

Still deferred beyond Milestone 2:

- Tunnel lifecycle management.
- Persistent registry/discovery.
- SQLite metrics/history.
- Authentication/authorization and remote mode.
- Request-level MCP gateway telemetry.
- Generic secret-redaction engine.

## Closure State

Milestone 2 implementation and automated verification are complete. Formal milestone closure is pending only the manual browser smoke test and the resulting `0.2.0` release metadata update.