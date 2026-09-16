# Milestone 3 Status — Secure Tunnel Management

## Status

**IMPLEMENTATION COMPLETE — automated release gates passed on 2026-09-16; real tunnel runtime smoke verification remains required before closure.**

Target version: `v0.3.0`

Current branch: `feature/secure-tunnel-management`

## Implemented

### Tunnel lifecycle domain

- Added a dedicated `TunnelSupervisor` separate from the MCP `Supervisor`.
- Added tunnel lifecycle states: stopped, starting, running, stopping, and failed.
- Added start, stop, and restart operations.
- Added Studio-owned PID tracking and signal ownership rules.
- Added tunnel uptime, restart count, crash count, last exit code, and last runtime error.
- Unexpected tunnel exits are treated as crashes even when the child exits with status `0`; only explicit-stop exits become `stopped`.
- Studio graceful shutdown stops the Studio-owned tunnel process.
- Added generation guarding so stale monitor tasks cannot overwrite newer tunnel runtime state.

### Constrained runtime configuration

- Milestone 3 manages one configured tunnel runtime.
- Default runtime: `../tunnel-client/tunnel-client-runtime-cloudflared`.
- Default config: `../tunnel-client/config.yaml`.
- Runtime invocation is constructed internally as `run --config <validated-config-file>`.
- Browser/API callers cannot provide an executable path, PID, shell command, config path, or arbitrary argument array.
- Tunnel working directory, runtime, and config file are canonicalized.
- Runtime and config paths must remain under the configured tunnel working directory.
- Runtime executable validation is performed before spawn.

### Secret handling and logs

- Added optional server-side environment/file secret references.
- Resolved secret values are used only for child environment construction and log redaction.
- Secret references and values are not serialized in `TunnelStatus`.
- Tunnel stdout/stderr/Studio lifecycle logs use a bounded in-memory ring buffer.
- ANSI control sequences are removed before tunnel log storage/streaming.
- Exact resolved secret values are redacted before log storage and publication.
- Common secret-bearing fields such as token, secret, password, credential, and authorization are redacted from tunnel log lines.

### REST API

Added:

```text
GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs
```

Tunnel lifecycle mutations use the same same-origin browser policy as MCP lifecycle mutations.

### Realtime model

Added typed WebSocket events:

```text
tunnel_status
tunnel_log
```

The initial/recovery `snapshot` now contains both MCP process status and tunnel status.

MCP and tunnel lifecycle publishers remain separate bounded event domains; the WebSocket layer multiplexes them into one typed browser protocol.

### Dashboard

- Added a secure tunnel section with state, runtime availability, PID, uptime, restart count, crash count, last exit, and last error.
- Added tunnel start / restart / stop controls.
- Added realtime tunnel status updates.
- Added recent tunnel log rendering.
- Reconnect/resync snapshots reconcile tunnel state as well as MCP state.
- Studio realtime connection state, MCP lifecycle state, and tunnel lifecycle state are visibly distinct.
- Frontend tunnel types intentionally exclude runtime path, config path, secret references, and secret values.

### Security review

- Updated `docs/threat-model.md` for tunnel process execution, accidental exposure, secret leakage, log streaming, config/path confinement, and process ownership.
- Updated `docs/architecture.md` for the separate tunnel lifecycle domain and realtime multiplexing.
- Added ADR 0003 for secure tunnel supervision and fixed invocation policy.
- The existing `mcp-server/tunnel-client` implementation was not modified.

## Automated Verification

The complete automated Milestone 3 release gates passed from the current feature-branch source state on 2026-09-16.

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

Tunnel lifecycle integration coverage includes:

- start / stop;
- restart with PID replacement;
- duplicate-start rejection;
- stop-while-stopped rejection;
- invalid runtime failure state;
- unexpected non-zero exit crash accounting;
- unexpected zero exit crash accounting;
- realtime tunnel status events;
- realtime tunnel log events;
- exact secret-value redaction;
- Studio shutdown cleanup.

The tunnel lifecycle suite was also rerun repeatedly under the default parallel test runner after fixing temp-fixture isolation; repeated runs passed.

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

Frontend verification includes tunnel lifecycle state rendering helpers, action enable/disable logic, tunnel log reconciliation, typed realtime tunnel updates, and reconnect snapshot modeling.

## Real Runtime Smoke Verification

**PENDING.**

The required smoke test must use:

```text
mcp-server/tunnel-client/tunnel-client-runtime-cloudflared
```

Required observations:

```text
start
→ running
→ PID present
→ uptime advances
→ logs visible

restart
→ PID changes
→ restart_count increments
→ crash_count remains correct

stop
→ stopped
→ PID cleared
```

Studio shutdown must also terminate a tunnel process that Studio owns.

### Important operational constraint

The current Aira Workspace connection itself may be using the same `tunnel-client` configuration and health listener (`127.0.0.1:18080`). Do not start a second Studio-managed instance against the same live config while that runtime is active, because the health listener or tunnel identity may conflict.

For closure, perform the smoke test in a controlled window where the existing tunnel runtime is stopped, or with an explicitly prepared smoke configuration whose local listener values do not conflict while preserving the same runtime binary and normal tunnel-client behavior.

## Exit Criteria

- Dedicated tunnel lifecycle domain implemented. **PASS**
- Constrained runtime invocation with no browser-supplied shell/argv. **PASS**
- Studio-owned PID signaling only. **PASS**
- Tunnel status/counters/logs exposed through REST and realtime events. **PASS**
- Tunnel dashboard controls and logs implemented. **PASS**
- Secret references remain server-side. **PASS**
- Tunnel log secret redaction implemented and tested. **PASS**
- Same-origin tunnel mutation protection tested. **PASS**
- Rust quality gates pass. **PASS**
- Frontend quality gates pass. **PASS**
- Dependency audit passes. **PASS**
- Locked release build succeeds. **PASS**
- Real tunnel runtime start/restart/stop smoke test. **PENDING**
- Studio shutdown cleanup with real tunnel runtime. **PENDING**

## Closure Decision

Milestone 3 is **not yet formally closed**. Implementation and automated verification are complete, but the roadmap Definition of Done requires the real runtime smoke test before release preparation, merge, and tag.

After the smoke test passes:

1. update this document to `COMPLETE`;
2. advance Rust/frontend package versions to `0.3.0`;
3. finalize the `0.3.0` changelog entry;
4. create `chore(release): prepare v0.3.0`;
5. pass dev-git-control readiness;
6. merge `feature/secure-tunnel-management` into `main` with `--no-ff` semantics;
7. create annotated release tag `v0.3.0`;
8. delete the merged feature branch.
