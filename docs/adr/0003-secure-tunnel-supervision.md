# ADR 0003 — Secure Tunnel Supervision Boundary

- Status: Accepted
- Date: 2026-09-16
- Milestone: 3 — Secure Tunnel Management

## Context

MCP Studio must manage the existing `tunnel-client-runtime-cloudflared` process without turning Studio into a generic process launcher or coupling tunnel behavior to the MCP supervisor.

The tunnel process has a different security profile from an MCP child: starting it may create externally reachable connectivity and it may consume credentials or other secret material. The browser must therefore receive operational state without receiving executable/configuration/secret data.

The existing tunnel bundle is already configured through `mcp-server/tunnel-client/config.yaml` and supports direct invocation as:

```text
tunnel-client-runtime-cloudflared run --config <config-file>
```

## Decision

### Separate lifecycle domain

Tunnel lifecycle is implemented by a dedicated `TunnelSupervisor` rather than adding tunnel-specific branches to the MCP `Supervisor`.

The two supervisors use similar lifecycle semantics and ownership rules but maintain independent runtime state, logs, counters, configuration, and event publication.

### Single configured tunnel in Milestone 3

Milestone 3 intentionally manages one configured tunnel runtime. The API is singular (`/api/tunnel`) and does not accidentally generalize to a registry of tunnel instances.

Multiple tunnel instances or runtime adapters require a later explicit design decision.

### Fixed startup argv

Studio configuration provides only server-side tunnel fields:

- display name;
- runtime path;
- working directory;
- config file;
- optional environment/file secret references.

Studio constructs argv internally as:

```text
run --config <validated-config-file>
```

No browser/API endpoint accepts a raw command, executable path, config path, arbitrary argument array, or PID.

### Path validation

Before spawn, Studio canonicalizes the configured working directory, runtime executable, and config file.

The runtime and config file must remain below the canonical tunnel working directory. The runtime must be a regular executable file and the config must be a regular file.

This validation occurs at lifecycle start so runtime availability can change without restarting Studio.

### Process ownership

Studio may signal only the PID returned by the tunnel child it spawned and currently tracks.

Tunnel stop uses the same Unix safety model as MCP supervision: `SIGTERM`, bounded wait, then `SIGKILL` fallback if required.

Studio graceful shutdown invokes tunnel shutdown before completing server shutdown.

### Secret handling

Optional tunnel environment values are configured as references to host environment variables or files. Values are resolved only at spawn time.

Secret references and resolved values are not part of public tunnel status, REST payloads, WebSocket events, or frontend types.

Resolved secret values are registered with the tunnel log redactor before log entries are buffered or published.

### Realtime integration

MCP and tunnel supervisors keep separate bounded event publishers. The existing WebSocket endpoint listens to both and emits one typed Studio event protocol:

```text
snapshot
process_status
log
tunnel_status
tunnel_log
resync_required
```

A snapshot contains both MCP status and tunnel status. Lag on either event receiver triggers a combined resynchronization snapshot.

## Consequences

### Positive

- Tunnel security policy remains explicit and independently reviewable.
- No generic command-execution surface is introduced.
- MCP supervision remains focused on MCP process concerns.
- Browser-visible tunnel state is decoupled from secret/configuration state.
- Existing WebSocket transport is reused without making events untyped.
- The single-runtime model matches the current tunnel-client deployment.

### Negative

- Some lifecycle implementation patterns are duplicated between MCP and tunnel supervisors.
- Two realtime publishers are multiplexed by the API layer rather than sharing one supervisor-owned publisher.
- Runtime adapters/multiple tunnels require future design work.
- Generic secret-pattern redaction cannot guarantee removal of secrets unknown to Studio.

## Rejected alternatives

### Reuse MCP `Supervisor` directly

Rejected because tunnel startup policy, secret handling, singular API semantics, and exposure risk are materially different from MCP server supervision.

### Expose arbitrary tunnel argv through the browser

Rejected because it would create a command/argument injection boundary and make the UI a generic privileged process launcher.

### Execute `run.sh`

Rejected for Milestone 3 because the wrapper accepts arbitrary trailing arguments and embeds additional runtime overrides. Studio instead uses the smallest direct runtime contract backed by the existing checked-in tunnel config.

### Modify tunnel-client first

Rejected because inspection found no Studio integration blocker. The existing runtime already supports the required direct invocation contract.

## Deferred decisions

- Multiple tunnel instances.
- Tunnel runtime adapter/plugin model.
- Auto-restart/backoff/circuit breaker.
- Process-group/descendant cleanup.
- Persistent tunnel logs/history.
- Remote Studio authentication/authorization.
- Runtime-specific structured log parsing beyond current redaction.
