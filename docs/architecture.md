# MCP Studio Architecture

## Purpose

MCP Studio is a local-first control plane for MCP servers and secure tunnels under `mcp-server/`.

## Current boundary — Milestone 2

Milestone 2 adds a browser dashboard and realtime WebSocket transport on top of the completed Milestone 1 MCP process supervisor.

Current managed servers remain statically configured `blender` and `filesystem` entries. Tunnel management, persistent discovery/registry, historical metrics, SQLite persistence, authentication/remote mode, and MCP gateway traffic are outside this milestone.

## Components

- `api`: HTTP, WebSocket, browser-origin validation, SPA/static-file boundary.
- `config`: typed TOML configuration and validation.
- `registry`: static in-memory MCP definitions loaded from configuration.
- `supervisor`: child-process lifecycle, runtime state, PID ownership, log capture, event publication, and shutdown cleanup.
- `realtime`: typed runtime event model and bounded Tokio broadcast channel.
- `web`: React + TypeScript + Vite dashboard.
- `tunnel`: reserved for Milestone 3.
- `discovery`: reserved for Milestone 4.
- `metrics`: reserved for historical/request metrics in later milestones.
- `storage`: reserved for persistent state in Milestone 5.
- `logging`: structured JSON logging initialization.
- `error`: shared typed error boundary.

## Runtime model

```text
Browser
   |
   | same-origin HTTP / WebSocket
   v
+------------------------------------------------+
| MCP Studio / Axum                              |
|                                                |
|  Static SPA --------+                          |
|                     |                          |
|  REST API ----------+--> Supervisor            |
|                     |       |                  |
|  WebSocket <--- Realtime Hub <-+               |
|                             |                  |
|                         status / logs          |
+-----------------------------+------------------+
                              |
                              | spawn / SIGTERM / SIGKILL
                              v
                       managed MCP child
```

Studio serves the production dashboard from `web/dist`. During development Vite runs separately and proxies `/api` plus WebSocket traffic to the Studio backend.

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

A monotonically changing generation value is attached to each managed process instance. Monitor tasks update runtime state only when their generation still matches the active generation, preventing stale tasks from overwriting a newer instance after restart.

## Process ownership

Studio may signal only a PID obtained from a child process it spawned and currently tracks. The HTTP API never accepts a PID or arbitrary executable command from callers.

Unix stop behavior:

1. transition tracked runtime state to `stopping`;
2. send `SIGTERM` to the tracked child PID;
3. wait for the monitor to observe termination;
4. if `stop_timeout_ms` expires, send `SIGKILL`;
5. wait again for a terminal state.

Studio graceful shutdown invokes the same supervisor stop path for each owned running process.

## stdio ownership

The monitor task owns the complete `tokio::process::Child` for the process lifetime. Child stdin remains open while the MCP is running so stdio MCP servers do not receive unintended EOF. stdout and stderr handles are detached into bounded asynchronous readers.

Studio does not proxy MCP protocol messages in Milestone 2; request-level telemetry remains intentionally unavailable.

## Realtime event model

The supervisor publishes typed events to an in-process bounded `tokio::sync::broadcast` channel.

Current event variants:

```text
snapshot
process_status
log
resync_required
```

A WebSocket connection receives an initial complete process snapshot before incremental runtime events.

If a subscriber lags behind the broadcast channel, the WebSocket layer sends `resync_required` followed by a fresh process snapshot. The client can also refetch REST state.

No process uptime event is emitted once per second. `ProcessStatus.uptime_ms` provides a backend sample and the dashboard advances the displayed value locally while the process remains running. Subsequent process-status events reset that baseline.

## Recent log buffer

Each registered MCP has a bounded in-memory `VecDeque` containing recent:

- stdout lines;
- stderr lines;
- Studio lifecycle messages.

Each entry includes a monotonic per-MCP sequence number. The browser uses the sequence to deduplicate initial REST history and WebSocket events.

The buffer is capped by `log_capacity`. Browser `Clear view` clears only client state; it does not mutate the supervisor buffer.

Persistence, rotation, and retention are deferred to later milestones.

## HTTP and WebSocket API

```text
GET  /health
GET  /api/status
GET  /api/mcp
GET  /api/mcp/{id}
POST /api/mcp/{id}/start
POST /api/mcp/{id}/stop
POST /api/mcp/{id}/restart
GET  /api/mcp/{id}/logs
GET  /api/ws
```

Expected lifecycle conflicts return `409 Conflict`; unknown MCP identifiers return `404 Not Found`.

## Browser security boundary

Milestone 2 introduces a browser as a privileged local client. A hostile remote webpage may still attempt to contact localhost services, so loopback binding alone is not treated as sufficient browser protection.

For lifecycle mutations and WebSocket upgrades:

1. requests without an `Origin` header remain available to non-browser local clients such as `curl`;
2. browser requests with an `Origin` header require a `Host` header;
3. the origin must exactly match `http://{Host}` or `https://{Host}`;
4. foreign origins are rejected;
5. wildcard CORS is not enabled.

Authentication/authorization is not implemented because non-loopback operation remains prohibited.

## Frontend architecture

The dashboard is intentionally small and dependency-light for the MVP:

- React for view composition and state.
- TypeScript with strict checking.
- Vite for development and production builds.
- Vitest for deterministic frontend logic tests.
- Native WebSocket with bounded exponential reconnect backoff.
- Typed REST wrappers for lifecycle commands and initial state/log retrieval.

The frontend derives lifecycle button availability from `ProcessState` and disables concurrent actions while a lifecycle request is in flight.

## Static asset serving

Production assets are generated into:

```text
web/dist
```

Axum serves that directory with an SPA fallback to `web/dist/index.html`.

The release process must build the frontend before running a production-style Studio instance. Embedding assets directly into the Rust executable is deferred; release packaging may revisit this later.

## Configuration

Configuration remains TOML. Milestone 2 continues to bind only to loopback.

```toml
[server]
listen_addr = "127.0.0.1:18100"

log_capacity = 500
stop_timeout_ms = 3000
```

MCP definitions use structured fields (`command`, `args`, `working_dir`, `env`) rather than shell command strings.

Relative command and working-directory paths are resolved against Studio's process working directory.

## Error handling

- Library boundaries return typed `StudioError` values.
- Top-level startup may use `anyhow` for context aggregation.
- Expected lifecycle errors do not panic.
- Process launch/wait failures are recorded in runtime status.
- Operational failures are logged with structured context.
- Browser same-origin failures return `403 Forbidden`.

## Security boundary

1. HTTP remains loopback-only.
2. Browser lifecycle mutations and WebSocket upgrades are same-origin constrained.
3. Browser APIs do not accept shell commands, executable paths, or PIDs.
4. Static MCP configuration is loaded before supervisor creation.
5. Studio only terminates process instances it started and tracks.
6. Process status does not serialize MCP environment configuration.
7. Child logs are still exposed verbatim and therefore must not contain secrets.
8. Auto-discovery never auto-executes code when introduced later.

## Evolution rule

Changes that materially affect process ownership, signal behavior, executable policy, browser-origin policy, authentication, persistence, gateway behavior, tunnel exposure, or external API contracts require architecture/threat review and may require a new ADR.
