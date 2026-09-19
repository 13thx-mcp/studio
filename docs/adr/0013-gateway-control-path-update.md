# ADR 0013 — Gateway Control-Path-Safe Update

## Status

Accepted for M5.8.

## Context

`rust-mcp-gateway` is not an ordinary Studio-managed child process on the reference runtime. The secure tunnel runtime owns the active Gateway process through the tunnel configuration's `mcp.commands` main-channel command, which points at the flat runtime binary and the generated host-local `runtime/gateway/servers.d` directory. Updating Gateway through the generic M5.7 Supervisor path would create competing process owners and could strand the control path used by the current session.

Gateway update success also requires more than a binary version probe. The replacement must initialize as an MCP server, reload the configured child catalog, expose its built-in control tools, preserve generated host configuration, and return the tunnel-owned control path to a healthy running state when it was running before the transaction.

## Decision

M5.8 uses a dedicated `GatewayUpdateManager` while reusing the trusted release/staging and atomic rollback primitives established by M5.5/M5.7.

Before lifecycle mutation, the manager:

- requires the trusted Gateway catalog target and a verified `ready-*` staged artifact;
- revalidates the prepared staging fingerprint and installed source version;
- snapshots every regular file in `runtime/gateway/servers.d` by SHA-256 and parses child enabled/command metadata;
- requires known managed child commands to canonicalize to their catalog-derived flat `bin_root` targets;
- parses the server-owned tunnel YAML and requires exactly one `main` MCP command bound to the catalog Gateway binary and trusted `servers.d` directory;
- rejects transitional/failed tunnel ownership state and duplicate Gateway update transactions.

If the tunnel owner was running, Studio stops the tunnel through the existing `TunnelSupervisor`. It then prepares checksum-protected rollback material and atomically replaces only `bin/rust-mcp-gateway` using the same-directory M5.7 activation primitive.

Before reconnecting the tunnel, Studio launches the new Gateway as a temporary standalone stdio MCP client probe against the existing `servers.d`. The probe requires:

- MCP initialize success;
- `tools/list` success;
- all three Gateway built-in tools (`gateway_list_servers`, `gateway_reload`, `gateway_set_server_enabled`);
- successful `gateway_list_servers` structured output;
- configured/enabled counts matching the pre-update snapshot;
- every enabled configured child visible, running, and without an error;
- no failed configured child;
- a non-empty exposed child-tool catalog whenever an enabled child is configured;
- unchanged `servers.d` file digests and child metadata.

If the tunnel owner was previously running, Studio then starts it through `TunnelSupervisor` and enters a bounded reconnect phase. `Starting`/`Stopping` are treated as intentional transitional states until a timeout. Success requires `Running`, remaining `Running` through a health window, and the tunnel-to-Gateway binding still matching the trusted paths.

Any activation, Gateway protocol/catalog, tunnel restart, reconnect-timeout, or reconnect-health failure triggers rollback. Rollback restores the previous binary, re-runs the standalone Gateway protocol/catalog probe against the same preserved `servers.d`, and restores/re-verifies the previous tunnel ownership state. Rollback failure is explicit.

The existing update API remains the browser surface. `POST /api/updates/gateway/prepare` and `/apply` are routed to `GatewayUpdateManager`; no config path, binary path, release URL, tunnel secret, or ownership parameter is accepted from the browser. Gateway-specific transaction phases are represented in the common browser-safe transaction DTO/realtime event stream.

## Consequences

- There is one process owner for the active Gateway: tunnel-client/TunnelSupervisor. Studio does not create a second Gateway supervisor.
- Generated `runtime/gateway/servers.d` is host-local state and is never overwritten by the release archive or activation transaction.
- A stopped tunnel remains stopped after a successful Gateway update; Studio does not start it merely to complete an update.
- A running tunnel experiences an intentional bounded disconnect/reconnect during activation.
- Gateway success has a stronger protocol/catalog proof than the generic M5.7 MCP path.
- Transaction records remain process-local; a Studio crash can still require recovery using preserved finalized rollback material.
- The standalone pre-reconnect probe starts configured child MCP processes briefly in a separate temporary Gateway instance. It does not mutate `servers.d`, but it does create a short-lived parallel child catalog while the production tunnel-owned Gateway is stopped.

## Alternatives rejected

### Treat Gateway as a normal M5.7 child MCP

Rejected because Studio's MCP Supervisor does not own the deployed Gateway process; tunnel-client does.

### Replace the Gateway binary while tunnel-client remains running

Rejected because the active process would continue using the old image and there would be no controlled proof that reconnect uses the new binary.

### Restart the tunnel unconditionally

Rejected because a tunnel that was stopped before update must remain stopped, and unnecessary reconnects increase disruption.

### Verify only `rust-mcp-gateway --version`

Rejected because that does not prove MCP initialization, generated-config loading, child startup, or tool-catalog restoration.
