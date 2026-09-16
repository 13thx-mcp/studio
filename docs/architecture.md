# MCP Studio Architecture

## Purpose

MCP Studio is a local-first control plane for MCP servers and the existing secure tunnel runtime under `mcp-server/`.

## Current boundary — Milestone 3

Milestone 3 extends the completed MCP process supervisor and Milestone 2 browser dashboard with one separately supervised secure tunnel runtime.

Current managed MCP servers remain statically configured. The managed tunnel is the existing `mcp-server/tunnel-client/tunnel-client-runtime-cloudflared` bundle using its existing YAML configuration. Persistent discovery/registry, historical metrics, SQLite persistence, remote authentication, auto-restart/backoff, and MCP gateway request telemetry remain outside this milestone.

## Components

- `api`: HTTP, WebSocket, browser-origin validation, SPA/static-file boundary.
- `config`: typed TOML configuration and validation.
- `registry`: static in-memory MCP definitions loaded from configuration.
- `supervisor`: MCP child-process lifecycle, runtime state, PID ownership, log capture, event publication, and shutdown cleanup.
- `tunnel`: tunnel configuration, lifecycle supervisor, PID ownership, secret resolution/redaction, log capture, and shutdown cleanup.
- `realtime`: typed runtime event model and bounded Tokio broadcast abstraction.
- `web`: React + TypeScript + Vite dashboard.
- `discovery`: reserved for Milestone 4.
- `metrics`: reserved for historical/request metrics in later milestones.
- `storage`: reserved for persistent state in Milestone 5.
- `logging`: structured logging initialization.
- `error`: shared typed error boundary.

## Runtime model

```text
Browser
   |
   | same-origin HTTP / WebSocket
   v
+-----------------------------------------------------------+
| MCP Studio / Axum                                         |
|                                                           |
|  REST API ------+--> MCP Supervisor ----> managed MCP     |
|                 |                                         |
|                 +--> Tunnel Supervisor --> tunnel runtime |
|                                                           |
|  WebSocket <----+---- typed MCP event stream              |
|                 +---- typed tunnel event stream           |
|                                                           |
|  Static React/Vite dashboard                              |
+-----------------------------------------------------------+
```

The WebSocket endpoint multiplexes the bounded MCP and tunnel event receivers into one existing Studio WebSocket contract. This keeps the lifecycle domains independent while preserving one browser realtime channel.

## Lifecycle domains

MCP and tunnel lifecycle state machines intentionally use the same state vocabulary but separate supervisor types and runtime state.

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
   |                         |
   | exit                    | explicit start/restart
   v                         |
STOPPED <--------------------+
```

Auto-restart, exponential backoff, and crash-loop circuit breaking remain deferred.

## Process ownership

Studio may signal only a PID obtained from a child process it spawned and currently tracks. Neither MCP nor tunnel HTTP APIs accept a PID.

Unix stop behavior:

1. transition tracked runtime state to `stopping`;
2. send `SIGTERM` to the tracked child PID;
3. wait for the monitor to observe termination;
4. if `stop_timeout_ms` expires, send `SIGKILL`;
5. wait again for a terminal state.

Studio graceful shutdown stops the owned tunnel and then all owned MCP processes.

## Tunnel startup contract

Milestone 3 intentionally manages exactly one configured tunnel runtime.

Studio configuration provides:

```toml
[tunnel]
name = "Secure tunnel"
runtime = "../tunnel-client/tunnel-client-runtime-cloudflared"
working_dir = "../tunnel-client"
config_file = "../tunnel-client/config.yaml"
```

The backend constructs startup argv internally as exactly:

```text
run --config <validated-config-file>
```

There is no browser/API field for runtime path, config path, shell command, arbitrary argument array, or PID.

At start time Studio canonicalizes `working_dir`, `runtime`, and `config_file`. Canonical runtime/config paths must remain inside canonical `working_dir`; runtime must be a regular executable file and config must be a regular file.

## Tunnel secret references

Optional tunnel environment values use server-side references rather than browser-visible values.

Example:

```toml
[tunnel.env]
TUNNEL_TOKEN = { from_env = "MCP_TUNNEL_TOKEN" }
CREDENTIAL = { from_file = "../secrets/tunnel-token" }
```

References are resolved at spawn time. Public `TunnelStatus`, REST responses, WebSocket events, and frontend types contain no environment map, secret reference, runtime path, config path, or secret value.

Resolved secret values are also registered with tunnel log redaction before log entries enter Studio's in-memory buffer or realtime stream.

## Realtime event model

Typed variants are:

```text
snapshot
process_status
log
tunnel_status
tunnel_log
resync_required
```

The initial `snapshot` contains both current MCP process statuses and current tunnel status.

The MCP and tunnel supervisors each publish to bounded broadcast streams. The WebSocket session listens to both. If either receiver lags, the server emits `resync_required` and follows it with a fresh combined snapshot.

No uptime event is emitted once per second. Backend status carries an uptime sample and the browser advances displayed uptime locally while a runtime remains running.

## Logs

MCP and tunnel logs use separate bounded in-memory ring buffers with monotonically increasing per-domain sequence numbers.

Tunnel log handling additionally:

- removes ANSI control sequences;
- replaces exact Studio-resolved secret values;
- defensively redacts common token/secret/password/credential/authorization key-value fields.

Persistent logs, rotation, byte-rate controls, and generalized MCP secret redaction remain deferred.

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

GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs

GET  /api/ws
```

Expected lifecycle conflicts return `409 Conflict`. Configuration/runtime validation failures return an error without spawning an unmanaged command.

## Browser security boundary

Studio remains loopback-only. Browser lifecycle mutations and WebSocket upgrades enforce the same origin policy:

1. requests without an `Origin` header remain available to local non-browser clients;
2. browser requests with `Origin` require `Host`;
3. origin must exactly match `http://{Host}` or `https://{Host}`;
4. foreign origins are rejected;
5. wildcard CORS is not enabled.

Authentication/authorization remains intentionally absent because non-loopback Studio operation is prohibited.

## Frontend architecture

The dashboard explicitly separates:

- Studio realtime connection state;
- MCP server lifecycle state;
- secure tunnel lifecycle state.

Tunnel UI shows only operational state: runtime availability, lifecycle state, PID, uptime, restart/crash counters, last exit/error, actions, and redacted logs.

The frontend type contract intentionally contains no secret/config/runtime-path fields.

## Configuration

Configuration remains TOML and loopback-only.

Relative paths are resolved against Studio's process working directory. MCP definitions continue to use structured fields rather than shell command strings. Tunnel configuration is more constrained: no arbitrary tunnel argv field exists in M3.

## Error handling

- Library boundaries return typed `StudioError` values.
- Top-level startup may use `anyhow` for context aggregation.
- Expected lifecycle errors do not panic.
- Spawn/wait failures are recorded in runtime state.
- Browser same-origin failures return `403 Forbidden`.
- Tunnel path/secret validation fails before process spawn.

## Security boundary summary

1. Studio HTTP remains loopback-only.
2. Browser lifecycle mutations and WebSocket upgrades are same-origin constrained.
3. Browser APIs accept no shell commands, executable paths, arbitrary argv, config paths, or PIDs.
4. Studio terminates only process instances it started and tracks.
5. Tunnel executable/config paths are canonicalized and confined to configured tunnel working directory.
6. Tunnel secrets remain server-side and are absent from public status/event/frontend types.
7. Tunnel logs are redacted before buffering/streaming.
8. MCP log redaction remains a known later-hardening requirement.
9. Auto-discovery never auto-executes code when introduced later.

## Evolution rule

Changes that materially affect process ownership, signal behavior, executable policy, tunnel invocation/secret policy, browser-origin policy, authentication, persistence, gateway behavior, external API contracts, or tunnel exposure require architecture/threat review and may require a new ADR.
