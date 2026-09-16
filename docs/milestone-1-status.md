# Milestone 1 Status — Core Process Supervisor

## Status

**IMPLEMENTED — runtime verification pending.**

Target version: `v0.1.0`

## Implemented

- Static MCP registry for Blender and Filesystem.
- Start MCP process.
- Stop MCP process.
- Restart MCP process.
- PID tracking.
- Lifecycle state tracking: stopped / starting / running / stopping / failed.
- Uptime reporting.
- Last exit-code reporting.
- Restart counter.
- Crash counter.
- Last runtime error reporting.
- Graceful Unix stop using `SIGTERM`.
- Force-kill fallback using `SIGKILL` after configurable timeout.
- Child ownership boundary: Studio only signals PIDs returned by processes it spawned.
- stdout/stderr capture.
- Per-MCP in-memory recent log ring buffer.
- Generation guard preventing stale monitor tasks from overwriting newer process state.
- Studio graceful shutdown stopping managed MCP children.
- Basic REST control/status API.
- Structured API errors for not-found and lifecycle conflicts.
- Lifecycle integration tests using real child processes on Unix.

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
```

## Verification Gate

Before Milestone 1 is closed, run from `mcp-server/studio`:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
```

Because Milestone 1 adds `nix` and Tokio process/sync/time features, allow Cargo to update `Cargo.lock` once, then verify the reproducible locked gate:

```bash
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --all-targets --all-features
```

## Manual Smoke Test

Build the managed MCP servers first:

```bash
cd mcp-server/blender
cargo build
cd ../filesystem
cargo build
cd ../studio
cargo run
```

In another terminal:

```bash
curl http://127.0.0.1:18100/api/status
curl http://127.0.0.1:18100/api/mcp
```

Filesystem lifecycle:

```bash
curl -X POST http://127.0.0.1:18100/api/mcp/filesystem/start
curl http://127.0.0.1:18100/api/mcp/filesystem
curl http://127.0.0.1:18100/api/mcp/filesystem/logs
curl -X POST http://127.0.0.1:18100/api/mcp/filesystem/restart
curl -X POST http://127.0.0.1:18100/api/mcp/filesystem/stop
```

Blender lifecycle may be started without Blender bridge availability, but bridge-backed MCP tools will report the bridge as unavailable until the Blender add-on is running.

## Required Test Coverage

Automated lifecycle tests currently cover:

- Successful start and stop.
- Restart creates a replacement PID and increments restart count.
- Unexpected non-zero exit transitions to failed and increments crash count.
- Invalid executable path.
- Duplicate start.
- Stop when already stopped.
- Studio-style shutdown of owned processes.

## Known Milestone 1 Constraints

- Process signaling is currently Unix-oriented (macOS/Linux).
- Registry is static configuration; persistence and discovery are Milestone 4.
- Logs are memory-only; persistence is Milestone 5.
- No auto-restart policy yet; hardening/backoff is Milestone 6.
- No MCP request-level telemetry; gateway telemetry is Milestone 7.
- No tunnel management; that begins in Milestone 3.
- No web dashboard; that begins in Milestone 2.

## Closure Criteria

Milestone 1 can be marked COMPLETE after:

- format gate passes,
- clippy with `-D warnings` passes,
- all tests pass,
- build passes,
- dependency audit has no unresolved Critical/High finding,
- locked gates pass with committed `Cargo.lock`,
- manual API smoke test succeeds for the managed MCP lifecycle,
- Studio shutdown leaves no Studio-owned MCP process running.
