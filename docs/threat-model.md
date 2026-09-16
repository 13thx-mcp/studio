# MCP Studio Threat Model

## Scope

MCP Studio is a privileged local control plane. As of Milestone 2 it can start, stop, restart, and observe configured MCP child processes through both REST and a browser dashboard with WebSocket updates.

Later milestones add tunnel control, discovery, persistence, remote authentication, gateway telemetry, and broader operational exposure.

## Assets

- Host process execution capability.
- MCP configuration.
- Managed child process identity/PIDs.
- Filesystem paths exposed by MCP servers.
- stdout/stderr operational logs.
- Browser lifecycle-control capability.
- Future tunnel credentials/tokens.
- Future registry, metrics, and audit history.

## Trust boundaries

1. Browser/operator ↔ Studio localhost HTTP/WebSocket API.
2. Studio ↔ managed MCP child processes.
3. Studio ↔ local configuration/filesystem.
4. Future Studio ↔ tunnel runtime.
5. Future Studio ↔ auto-discovered source trees.

## Milestone 2 security properties

- Studio rejects non-loopback HTTP bind addresses.
- Browser lifecycle mutations enforce same-origin `Origin`/`Host` alignment when `Origin` is present.
- WebSocket upgrades use the same browser-origin check.
- Wildcard CORS is not enabled.
- API callers cannot submit arbitrary executable paths, shell command strings, or PIDs.
- Only statically configured MCP definitions can be started.
- Commands are executed directly via `Command`, not shell interpolation.
- Studio signals only a PID returned from a process it spawned and currently tracks.
- Recent log buffers are bounded.
- Process state uses generation guards so stale monitor tasks cannot overwrite newer process state.
- WebSocket broadcast is bounded and lagged subscribers are resynchronized instead of blocking the supervisor.
- Process status does not serialize configured MCP environment variables.
- Studio shutdown attempts to stop every owned MCP process.

## Primary threats and controls

### Arbitrary command execution

Risk: an API or browser caller causes Studio to execute attacker-controlled commands.

Controls:

- No raw shell-command API.
- No command/PID field in lifecycle HTTP requests.
- Structured executable and argument configuration only.
- Milestone 2 registry remains static and loaded locally.
- Future editable registry requires path confinement and explicit policy review.

Residual risk: anyone able to modify Studio's local configuration can alter configured executables. Configuration file permissions remain part of host security.

### Cross-origin localhost control

Risk: a malicious remote webpage causes a user's browser to send privileged lifecycle requests to Studio on `127.0.0.1`.

Controls:

- Lifecycle POST requests validate `Origin` when present.
- A browser request with `Origin` must provide `Host`.
- Accepted origin must exactly match `http://{Host}` or `https://{Host}`.
- Foreign origins receive `403 Forbidden`.
- WebSocket upgrades use the same policy.
- Wildcard CORS is not enabled.
- Studio remains loopback-only.

Residual risk: non-browser local processes can call the API without `Origin`. This is intentional for local CLI/operator workflows and assumes local account/process isolation.

### Unauthorized remote control

Risk: lifecycle endpoints become reachable remotely without authentication.

Controls:

- Config validation rejects non-loopback bind addresses.
- No remote mode is supported in Milestone 2.

Remote operation must not ship until authentication, authorization, CSRF/CORS behavior, TLS/tunnel exposure, and session policy are explicitly designed and reviewed.

### Process ownership confusion / PID misuse

Risk: Studio kills an unrelated host process.

Controls:

- PID comes only from `Child::id()` for a Studio-spawned child.
- API does not expose kill-by-PID.
- State tracks whether that child is running/stopping.
- Generation guards prevent older monitor tasks from mutating newer runtime state.

Residual risk: PID reuse between detecting a hung process and signal escalation is theoretically possible. Production hardening should prefer stronger OS process identity/handle semantics where available.

### Orphan child processes

Risk: Studio exits while managed MCP processes remain alive.

Controls:

- Graceful Studio shutdown calls `shutdown_all()`.
- Spawned children use `kill_on_drop(true)` as a final local-process safety net.
- Lifecycle tests verify supervisor shutdown stops owned fixtures.

Further hardening may add process groups/session management where MCP servers spawn descendants.

### Denial of service through lifecycle calls

Risk: repeated start/restart operations exhaust resources.

Controls now:

- Duplicate starts are rejected.
- One runtime record exists per configured MCP.
- UI disables conflicting lifecycle controls during transitions/request execution.

Deferred controls:

- Restart backoff.
- API rate limiting.
- Crash-loop circuit breaker.
- Operational quotas.

These are required before production-grade remote operation.

### WebSocket subscriber pressure

Risk: a slow browser client blocks runtime event production or consumes unbounded memory.

Controls:

- Realtime fan-out uses a bounded Tokio broadcast channel.
- Slow subscribers receive lag errors rather than blocking publishers.
- Lagged clients are told to resynchronize and receive a fresh snapshot.
- Browser reconnect backoff is capped to avoid a tight retry loop.

Residual risk: many simultaneous local browser clients can still create CPU/network load. Multi-user fan-out limits are deferred until remote/multi-user mode exists.

### Log/memory exhaustion

Risk: noisy child output consumes unbounded memory.

Controls:

- Per-MCP recent logs use a fixed-capacity ring buffer.
- Oldest entries are discarded when capacity is reached.
- Browser clear-view is client-local and cannot alter server retention behavior.

Residual risk: extremely high stdout/stderr rates can still consume CPU and WebSocket bandwidth. Rate/byte limits may be added during hardening.

### Secret leakage through logs

Risk: MCP child processes write secrets to stdout/stderr and Studio exposes them through REST/WebSocket/dashboard.

Current limitation:

- Child output is captured verbatim.
- Milestone 2 does not implement a generic secret-redaction engine.

Controls:

- MCP environment configuration is not included in `ProcessStatus` or dashboard status payloads.
- Managed MCP servers are operationally required not to print credentials or secret environment values.

Production requirement:

- Secret-aware redaction before persistent storage or remote/browser streaming in broader deployment modes.

### Realtime state inconsistency

Risk: a browser misses events and displays stale lifecycle state.

Controls:

- WebSocket sends a complete initial snapshot.
- Broadcast lag emits `resync_required` followed by a new snapshot.
- Browser can refetch REST state.
- Log sequence numbers deduplicate overlap between REST history and WebSocket events.
- Browser refresh rehydrates state from backend sources rather than persistent client state.

This is primarily a correctness risk, but stale lifecycle state can also lead to unsafe operator actions, so lifecycle actions remain validated on the backend regardless of UI state.

### Path traversal / executable escape

Risk: future registry editing or discovery points commands outside the allowed MCP root.

Milestone 2:

- Configuration is local/static and no browser endpoint mutates executable paths.

Required before editable registry/discovery:

- Canonicalize paths.
- Constrain allowed roots.
- Define symlink policy.
- Explicit approval before registration/execution.

### Malicious discovered project

Deferred to Milestone 4.

Required controls:

- Discovery is metadata-only.
- No auto-execution.
- Registration requires explicit approval.
- Start remains an explicit action.

### Crash-loop resource exhaustion

Deferred controls:

- restart backoff;
- circuit breaker;
- restart limits;
- event audit.

### Tunnel accidental exposure

Tunnel lifecycle is not implemented in Milestone 2.

Before Milestone 3 release:

- tunnel executable policy;
- secret references/redaction;
- safe stopped default;
- public endpoint/authentication review.

### Supply-chain compromise

Controls:

- `Cargo.lock` committed.
- Rust toolchain pinned to 1.98.1.
- Rust quality gates include locked release builds and `cargo audit`.
- Frontend uses a committed `pnpm-lock.yaml`.
- TypeScript/ESLint compatibility-sensitive toolchain versions are explicitly pinned instead of relying entirely on moving `latest` tags.

Residual risk: some frontend packages are still intentionally range/latest-selected where compatibility was verified. Release gates and the lockfile define the actual dependency graph used for the milestone.

## Security release gate

No release candidate may contain unresolved known Critical or High severity findings in privileged lifecycle, browser-origin handling, secret handling, path confinement, authentication, or tunnel exposure.

## Review triggers

Update this threat model whenever adding or materially changing:

- process spawn/kill behavior;
- descendant/process-group handling;
- browser origin/CORS/CSRF handling;
- WebSocket/event fan-out behavior;
- tunnel management;
- discovery/registration;
- editable executable configuration;
- persistent secrets;
- remote access/authentication;
- MCP gateway/proxying;
- plugins/adapters;
- multi-host control.
