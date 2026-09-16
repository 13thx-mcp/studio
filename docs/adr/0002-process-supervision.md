# ADR 0002 — MCP Process Supervision Model

- Status: Accepted
- Date: 2026-09-16
- Milestone: 1 — Core Process Supervisor

## Context

MCP Studio must start, stop, restart, inspect, and clean up stdio-based MCP server processes while preserving a strict process-ownership boundary. The initial managed servers are Blender and Filesystem MCP implementations written in Rust and served over stdio.

The design must also support crash detection, stdout/stderr capture, graceful shutdown, and future evolution toward richer health checks and restart policies without allowing callers to signal arbitrary host processes.

## Decision

### Process ownership

Studio may manage only child processes that it spawned itself. The public API identifies MCP servers by registry ID, never by caller-supplied PID.

A managed runtime stores the PID returned by the spawned child and associates it with a monotonically increasing process generation.

### Child ownership and stdio

The background monitor task owns the `tokio::process::Child` for the lifetime of the process. This is required for stdio MCP servers because dropping the child's stdin can produce EOF and cause the server to exit unexpectedly.

stdout and stderr are detached from the child and consumed by dedicated asynchronous readers into bounded in-memory log buffers.

### Lifecycle model

The initial lifecycle states are:

```text
STOPPED
   │ start
   ▼
STARTING
   │ spawn succeeds
   ▼
RUNNING ───────────────┐
   │ stop requested    │ unexpected exit
   ▼                   ▼
STOPPING             FAILED
   │                   │
   ▼                   │ explicit restart/start
STOPPED ◄──────────────┘
```

Auto-restart, exponential backoff, and crash-loop circuit breaking are intentionally deferred to the hardening milestone.

### Generation guard

Every successful spawn receives a new generation value. A monitor task may update terminal process state only if the generation it owns still matches the runtime's current generation.

This prevents a stale monitor task from an older process instance from overwriting the state of a newer process after a fast restart.

### Stop behavior

On Unix platforms, Studio requests graceful termination with `SIGTERM` and waits up to the configured stop timeout. If the process does not exit, Studio escalates to `SIGKILL`.

Signals are sent only to the PID recorded for the currently managed process generation.

### Crash accounting

An unexpected exit while the process is considered running is recorded as a crash. The terminal state becomes `FAILED` and the last exit code or runtime error is exposed through the status API.

An expected exit caused by an explicit stop transitions to `STOPPED` and does not increment the crash counter.

### Logging

stdout and stderr are captured into a bounded per-MCP ring buffer. Logs are volatile in Milestone 1; persistence and retention policy are deferred to the persistence milestone.

### Registry

Milestone 1 uses a static typed registry loaded from Studio configuration. Runtime discovery and persistent registry management are deferred to Milestone 4.

## API implications

The control API exposes operations by MCP registry ID:

```text
GET  /api/mcp
GET  /api/mcp/{id}
POST /api/mcp/{id}/start
POST /api/mcp/{id}/stop
POST /api/mcp/{id}/restart
GET  /api/mcp/{id}/logs
```

No endpoint accepts a raw shell command or arbitrary PID.

## Consequences

### Positive

- Strong ownership boundary around process termination.
- Correct lifetime for stdio MCP child processes.
- Race-resistant restart semantics through generation tracking.
- Clear extension point for future health checks and restart policies.
- Failure of one MCP process does not inherently terminate Studio.

### Negative

- Current signal implementation is Unix-oriented.
- Logs are memory-only.
- Studio cannot adopt externally started MCP processes.
- Process-group/subprocess-tree termination is not yet modeled.

## Deferred decisions

The following require later design review or ADRs:

- Windows process control.
- Process-group/session isolation and descendant cleanup.
- Auto-restart/backoff/circuit breaker policy.
- Adoption or reconciliation of externally started processes.
- Persistent logs and process history.
- Gateway ownership of MCP stdio request traffic.
