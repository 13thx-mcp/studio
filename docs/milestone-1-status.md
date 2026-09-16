# Milestone 1 Status — Core Process Supervisor

## Status

**COMPLETE — re-closed after stdio lifecycle regression fix and manual re-verification on 2026-09-16.**

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

## Regression Found and Resolved

A manual smoke test exposed a stdio lifecycle bug where the Filesystem MCP briefly entered `running` and then exited with:

```text
Error: connection closed: initialize request
```

Observed failing lifecycle:

```text
start -> running -> failed(exit code 1)
```

Root cause: the supervisor left the piped child stdin owned by `tokio::process::Child` while immediately awaiting `Child::wait()`. Tokio closes an owned stdin handle while waiting to avoid deadlocks, causing the rmcp stdio server to observe EOF before any MCP client could send `initialize`.

Fix: `src/supervisor/mod.rs` now explicitly takes `ChildStdin` and retains it in the monitor task for the entire process lifetime. stdin is dropped only when the child exits.

Correct lifecycle after the fix:

```text
start -> running
restart -> running with replacement PID
stop -> stopped
```

## Automated Verification

The quality gates were rerun successfully after the fix with Rust `1.98.1`:

```bash
cargo check
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --all-targets --all-features
```

Automated test baseline:

- Library/API tests: `4 passed; 0 failed`.
- Supervisor lifecycle integration tests: `7 passed; 0 failed`.
- Total observed tests: `11 passed; 0 failed`.

## Manual Smoke Verification

Filesystem start was verified to remain alive beyond the initial spawn:

```text
start response:
state=running
pid=75934
restart_count=0
crash_count=0

status after ~1 second:
state=running
pid=75934
uptime_ms≈1014
crash_count=0
```

Restart was verified to replace the process and preserve healthy state:

```text
restart response:
state=running
pid=76395
restart_count=1
crash_count=0

status after ~1 second:
state=running
pid=76395
uptime_ms≈1012
restart_count=1
crash_count=0
```

Stop was verified successfully:

```text
state=stopped
pid=null
uptime_ms=null
restart_count=1
crash_count=0
last_error=null
```

This confirms that:

- stdio remains open while the MCP process is supervised,
- restart creates a replacement PID,
- restart does not falsely increment crash count,
- stop completes cleanly,
- final lifecycle state is correct.

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

## Known Milestone 1 Constraints

- Process signaling is Unix-oriented (macOS/Linux) in this milestone.
- Registry is static configuration; persistence and discovery are Milestone 4.
- Logs are memory-only; persistence is Milestone 5.
- No auto-restart policy yet; hardening/backoff is Milestone 6.
- No MCP request-level telemetry; gateway telemetry is Milestone 7.
- No tunnel management; that begins in Milestone 3.
- No web dashboard; that begins in Milestone 2.

## Closure Decision

Milestone 1 is formally re-closed because:

- all automated format/lint/test/build/audit gates pass,
- locked reproducibility gates pass,
- the stdio lifecycle regression has been resolved,
- the Filesystem MCP remains running after startup,
- restart creates a healthy replacement process,
- stop reaches the correct stopped state,
- no false crash is recorded during normal restart/stop lifecycle,
- documentation reflects the final verified behavior.

## Next Milestone

Proceed to **Milestone 2 — MVP Web Dashboard**.
