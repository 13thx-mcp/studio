# Milestone 3 Status — Secure Tunnel Management

## Status

**COMPLETE — automated release gates and real tunnel runtime smoke verification passed on 2026-09-16.**

Released version: `v0.3.0`

## Implemented

### Tunnel lifecycle domain

- Dedicated `TunnelSupervisor` separate from the MCP `Supervisor`.
- Tunnel lifecycle states: stopped, starting, running, stopping, and failed.
- Start, stop, and restart operations.
- Studio-owned PID tracking and signal ownership rules.
- Tunnel uptime, restart count, crash count, last exit code, and last runtime error.
- Unexpected tunnel exits are treated as crashes even when the child exits with status `0`; only explicit-stop exits become `stopped`.
- Studio graceful shutdown stops the Studio-owned tunnel process.
- Generation guarding prevents stale monitor tasks from overwriting newer tunnel runtime state.

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

- Optional server-side environment/file secret references.
- Resolved secret values are used only for child environment construction and log redaction.
- Secret references and values are not serialized in `TunnelStatus`.
- Tunnel stdout/stderr/Studio lifecycle logs use a bounded in-memory ring buffer.
- ANSI control sequences are removed before tunnel log storage/streaming.
- Exact resolved secret values are redacted before log storage and publication.
- Common secret-bearing fields such as token, secret, password, credential, and authorization are redacted from tunnel log lines.

### REST API

```text
GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs
```

Tunnel lifecycle mutations use the same same-origin browser policy as MCP lifecycle mutations.

### Realtime model

Typed WebSocket events:

```text
tunnel_status
tunnel_log
```

The initial/recovery `snapshot` contains both MCP process status and tunnel status.

MCP and tunnel lifecycle publishers remain separate bounded event domains; the WebSocket layer multiplexes them into one typed browser protocol.

### Dashboard

- Secure tunnel section with state, runtime availability, PID, uptime, restart count, crash count, last exit, and last error.
- Tunnel start / restart / stop controls.
- Realtime tunnel status updates.
- Recent tunnel log rendering.
- Reconnect/resync snapshots reconcile tunnel state as well as MCP state.
- Studio realtime connection state, MCP lifecycle state, and tunnel lifecycle state are visibly distinct.
- Frontend tunnel types intentionally exclude runtime path, config path, secret references, and secret values.

### Security review

- Updated `docs/threat-model.md` for tunnel process execution, accidental exposure, secret leakage, log streaming, config/path confinement, and process ownership.
- Updated `docs/architecture.md` for the separate tunnel lifecycle domain and realtime multiplexing.
- Added ADR 0003 for secure tunnel supervision and fixed invocation policy.
- The existing `mcp-server/tunnel-client` implementation was not modified.

## Automated Verification

The complete Milestone 3 release gates passed on 2026-09-16.

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

The tunnel lifecycle suite was also rerun repeatedly under the default parallel test runner after fixture-isolation hardening; repeated runs passed.

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

The smoke test used the actual:

```text
mcp-server/tunnel-client/tunnel-client-runtime-cloudflared
```

A smoke-specific config used a non-conflicting local health listener while preserving the real runtime binary and normal tunnel-client behavior.

Observed lifecycle:

```text
initial
state=stopped
runtime_available=true
pid=null
restart_count=0
crash_count=0

start
state=running
pid=20095
uptime advanced from 0 ms to 13 ms
runtime log buffer contained Studio start entry

restart
old pid=20095
new pid=20099
restart_count=1
crash_count=0

stop
state=stopped
pid=null
restart_count=1
crash_count=0
```

Studio shutdown cleanup was then verified with a new Studio-owned tunnel PID `20277`. After Studio shutdown:

```text
kill -0 20277
→ no such process
```

Result: **PASS**.

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
- Real tunnel runtime start/restart/stop smoke test. **PASS**
- Studio shutdown cleanup with real tunnel runtime. **PASS**

## Closure Decision

Milestone 3 is formally closed because:

- the real tunnel runtime can be started, restarted, stopped, and observed through Studio;
- restart replaces the PID and increments the restart counter without falsely incrementing crash count;
- Studio shutdown cleans up the tunnel PID it owns;
- the browser/API surface does not expose arbitrary executable/argv/PID control;
- tunnel secret values remain server-side and are redacted from tunnel logs;
- same-origin and loopback-only control-plane protections remain enforced;
- Rust and frontend automated release gates pass;
- dependency audit and locked release build pass;
- architecture, threat-model, ADR, and operational documentation reflect the verified implementation.

## Next Milestone

Proceed to **Milestone 4 — Registry, Configuration & Auto-Discovery** only after the `v0.3.0` release merge/tag workflow is complete.
