# Milestone 3 — Secure Tunnel Management Design

## Status

Implementation design for `v0.3.0`.

## Scope

Milestone 3 adds lifecycle management for one configured secure tunnel runtime:

```text
../tunnel-client/tunnel-client-runtime-cloudflared
```

The existing `tunnel-client` bundle remains the managed runtime. Studio does not redesign the tunnel client or add a generic command runner.

## Runtime invocation contract

Inspection of the current tunnel bundle shows the runtime is invoked as:

```text
tunnel-client-runtime-cloudflared run --config <config-file>
```

The current `run.sh` adds optional runtime-specific overrides, but the checked-in `config.yaml` already contains the required single-instance channel configuration. Milestone 3 therefore uses the smallest safe startup contract and does not expose arbitrary runtime arguments to the browser.

Studio configuration supplies only server-side typed fields:

```toml
[tunnel]
name = "Secure tunnel"
runtime = "../tunnel-client/tunnel-client-runtime-cloudflared"
working_dir = "../tunnel-client"
config_file = "../tunnel-client/config.yaml"
```

Startup argv is constructed internally as exactly:

```text
run --config <validated-config-file>
```

No browser endpoint accepts an executable path, PID, shell command, config path, or arbitrary argument array.

## Domain model

Tunnel lifecycle remains separate from MCP lifecycle:

```text
AppState
├── MCP Supervisor
└── Tunnel Supervisor
```

Both supervisors publish typed bounded runtime events. The existing WebSocket endpoint multiplexes both event receivers into one browser realtime protocol.

The tunnel domain exposes:

- `TunnelConfig`
- `TunnelState`
- `TunnelStatus`
- `TunnelSupervisor`
- `TunnelLogEntry`

The tunnel lifecycle states are:

```text
STOPPED
   │ start
   ▼
STARTING
   │ spawn succeeds
   ▼
RUNNING ────────────────┐
   │ stop               │ unexpected exit
   ▼                    ▼
STOPPING              FAILED
   │                    │
   ▼                    │ explicit start/restart
STOPPED ◄───────────────┘
```

Auto-restart, retry backoff, and crash-loop policy remain out of scope.

## Validation and ownership

At start time Studio resolves the configured working directory, runtime executable, and tunnel config file against the Studio working directory.

Validation requires:

1. the configured working directory exists and canonicalizes successfully;
2. the runtime is a regular executable file;
3. the config file is a regular file;
4. both runtime and config canonical paths remain inside the configured tunnel working directory;
5. only the PID returned by Studio's own `tokio::process::Command::spawn` is eligible for lifecycle signals.

The public API never accepts a PID.

## Server-side secret references

Tunnel environment secrets are optional server-side configuration only. Secret values are resolved at spawn time from environment or file references and are never included in `TunnelStatus`, REST responses, WebSocket events, or frontend types.

Any resolved secret values are also supplied to the tunnel log redactor so exact secret values are replaced before entering Studio's in-memory log buffer or realtime stream.

## Logs

Tunnel stdout, stderr, and Studio lifecycle messages are captured into a bounded in-memory ring buffer.

Each `TunnelLogEntry` contains:

- sequence number;
- timestamp;
- stream;
- redacted message.

Log handling removes ANSI control sequences and redacts:

- exact resolved secret values;
- common secret-bearing key/value fields such as token, secret, password, credential, and authorization fields.

Persistent logs and historical metrics remain deferred.

## Realtime contract

The typed event set becomes:

```text
snapshot
process_status
log
tunnel_status
tunnel_log
resync_required
```

A snapshot contains both current MCP process status and the current tunnel status. On lag from either bounded event receiver the WebSocket sends `resync_required` followed by a fresh combined snapshot.

## REST API

Milestone 3 adds:

```text
GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs
```

Tunnel lifecycle mutations use the same browser same-origin policy as MCP lifecycle mutations. Non-browser local clients without an `Origin` header remain supported.

## Frontend contract

The dashboard presents three separate operational concepts:

- Studio realtime connection state;
- MCP server lifecycle state;
- secure tunnel lifecycle state.

The tunnel view shows state, runtime availability, PID, uptime, restart count, crash count, last exit code, last runtime error, lifecycle actions, and recent redacted logs.

The browser receives no tunnel executable path, config path, secret reference, or secret value.

## Test plan

Backend coverage:

- start, stop, restart;
- duplicate start and stop-while-stopped conflicts;
- invalid/missing runtime;
- unexpected exit and crash counting;
- restart PID replacement;
- graceful shutdown cleanup;
- realtime tunnel status/log events;
- same-origin mutation rejection;
- serialization and log-redaction checks for secret absence.

Frontend coverage:

- tunnel lifecycle button-state logic;
- tunnel status/log realtime event types;
- combined snapshot/reconnect reconciliation;
- frontend types deliberately omit secret/config fields.

Manual smoke uses the real `tunnel-client-runtime-cloudflared` and verifies start, restart, stop, log visibility, PID replacement, counters, and Studio shutdown cleanup.

## Non-goals

Milestone 3 does not add:

- multiple tunnel instances;
- generic tunnel adapters;
- browser-editable tunnel configuration;
- arbitrary argument passthrough;
- persistence or SQLite;
- automatic restart/backoff;
- remote Studio authentication;
- MCP gateway request telemetry.
