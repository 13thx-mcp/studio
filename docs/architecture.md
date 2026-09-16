# MCP Studio Architecture

## Purpose

MCP Studio is a local-first control plane for MCP servers and secure tunnels under `mcp-server/`.

## Current boundary — Milestone 1

Milestone 1 introduces privileged MCP child-process supervision for the statically configured `blender` and `filesystem` servers. Tunnel management, persistent discovery/registry, historical metrics, and the browser dashboard remain outside this milestone.

## Components

- `api`: HTTP boundary for health, status, lifecycle actions, and recent logs.
- `config`: typed TOML configuration and validation.
- `registry`: static in-memory MCP definitions loaded from configuration.
- `supervisor`: MCP child-process lifecycle, runtime state, PID ownership, log capture, and shutdown cleanup.
- `tunnel`: reserved for Milestone 3.
- `discovery`: reserved for Milestone 4.
- `metrics`: reserved for historical/request metrics in later milestones.
- `storage`: reserved for persistent state in Milestone 5.
- `logging`: structured JSON logging initialization.
- `error`: shared typed error boundary.

## Runtime model

```text
Browser / operator
       |
       | localhost HTTP
       v
+-----------------------+
| MCP Studio            |
|                       |
| API                   |
|  |                    |
|  +--> Registry        |
|  |                    |
|  `--> Supervisor      |
|         |             |
|         +--> stdout   |
|         +--> stderr   |
|         +--> PID      |
|         `--> state    |
+---------+-------------+
          |
          | spawn / SIGTERM / SIGKILL
          v
   managed MCP child
```

## Process lifecycle

```text
STOPPED
   | start
   v
STARTING
   | spawn
   v
RUNNING --------------------+
   | stop                    | unexpected non-zero exit / wait failure
   v                         v
STOPPING                   FAILED
   |
   | exit
   v
STOPPED
```

A monotonically changing generation value is attached to each managed process instance. Monitor tasks update runtime state only when their generation still matches the active generation, preventing a stale task from overwriting a newer process instance after restart.

## Process ownership

Studio may signal only a PID obtained from a child process it spawned and currently tracks. The HTTP API never accepts a PID or arbitrary executable command from callers.

Milestone 1 stop behavior on Unix:

1. transition tracked runtime state to `stopping`;
2. send `SIGTERM` to the tracked child PID;
3. wait for process monitor to observe termination;
4. if `stop_timeout_ms` expires, send `SIGKILL`;
5. wait again for the monitor to reach a terminal state.

Studio graceful shutdown invokes the same supervisor stop path for each owned running process.

## stdio ownership

The monitor task owns the complete `tokio::process::Child` for the process lifetime. Child stdin remains open while the MCP is running so stdio MCP servers do not receive an unintended EOF. stdout and stderr handles are detached into bounded asynchronous readers.

Milestone 1 does not proxy MCP messages through Studio; request-level telemetry is therefore intentionally unavailable.

## Recent log buffer

Each registered MCP has a bounded in-memory `VecDeque` containing recent:

- stdout lines;
- stderr lines;
- Studio lifecycle messages.

The buffer is capped by `log_capacity`. Persistence, rotation, and retention are deferred to later milestones.

## Configuration

Configuration is TOML. Built-in Milestone 1 defaults register Blender and Filesystem and continue to bind Studio only to loopback.

```toml
[server]
listen_addr = "127.0.0.1:18100"

log_capacity = 500
stop_timeout_ms = 3000
```

MCP definitions use structured fields (`command`, `args`, `working_dir`, `env`) rather than a shell command string.

Relative command and working-directory paths are resolved against Studio's process working directory.

## API contract

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

Expected lifecycle conflicts return `409 Conflict`; unknown MCP identifiers return `404 Not Found`.

## Error handling

- Library boundaries return typed `StudioError` values.
- Top-level startup may use `anyhow` for context aggregation.
- Expected lifecycle errors do not panic.
- Process launch/wait failures are recorded in runtime status.
- Operational failures are logged with structured context.

## Security boundary

1. HTTP binds to loopback only in Milestone 1.
2. Browser APIs do not accept shell command strings or PIDs.
3. Static MCP configuration is loaded before supervisor creation.
4. Studio only terminates process instances it started and tracks.
5. Auto-discovery never auto-executes code when introduced later.
6. Secrets must stay server-side and must not be emitted by future UI/logging layers.

## Evolution rule

Changes that materially affect process ownership, signal behavior, executable policy, security, persistence, gateway behavior, authentication, or external API contracts require an ADR.
