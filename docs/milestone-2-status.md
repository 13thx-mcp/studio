# Milestone 2 Status — MVP Web Dashboard

## Status

**IN IMPLEMENTATION — code-complete baseline pending final browser smoke test and release gates.**

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
- Log entries now include a monotonic per-MCP sequence number for REST/WebSocket deduplication.

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
- PID, uptime, restart count, crash count, last exit, and last error display.
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

## Automated Verification Completed So Far

Rust baseline previously passed after the M2 realtime foundation changes:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```

Observed Rust tests at that point:

- API/config/realtime library tests: 6 passed.
- Supervisor lifecycle integration tests: 7 passed.
- Total observed: 13 passed; 0 failed.

Frontend verification after dependency/toolchain stabilization:

```text
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Observed frontend tests before the final UX additions:

- Vitest: 3 passed; 0 failed.

Additional reconnect/backoff coverage has since been added and must be included in the final gate rerun.

## Required Final Verification

Before closing Milestone 2, rerun the complete final gates from the current branch:

### Rust

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo build --release --locked
```

### Frontend

```bash
cd web
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

## Manual Browser Smoke Test

Use a real Filesystem MCP instance and verify through the dashboard:

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
- Rust quality gates pass from the final source state.
- Frontend quality gates pass from the final source state.
- `cargo audit` passes or any advisory is explicitly reviewed and accepted.
- Release build succeeds with `--locked`.
- Manual browser smoke test succeeds with a real managed MCP.
- Version and changelog are advanced to `0.2.0` only after verification.

## Out of Scope

Still deferred beyond Milestone 2:

- Tunnel lifecycle management.
- Persistent registry/discovery.
- SQLite metrics/history.
- Authentication/authorization and remote mode.
- Request-level MCP gateway telemetry.
- Generic secret-redaction engine.

## Closure State

Milestone 2 is **not yet formally closed**. Implementation is at the final verification stage.
