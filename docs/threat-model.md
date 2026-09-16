# MCP Studio Threat Model

## Scope

MCP Studio is a privileged local control plane. As of Milestone 3 it can start, stop, restart, and observe configured MCP child processes and one configured secure tunnel runtime through REST and a browser dashboard with WebSocket updates.

Persistent discovery/registry, historical metrics, SQLite persistence, remote authentication, and MCP gateway request telemetry remain later milestones.

## Assets

- Host process execution capability.
- MCP configuration.
- Tunnel runtime configuration.
- Managed child process identity/PIDs.
- Filesystem paths exposed by MCP servers.
- stdout/stderr operational logs.
- Browser lifecycle-control capability.
- Tunnel credentials, tokens, and private environment/file secret references.
- Future registry, metrics, and audit history.

## Trust boundaries

1. Browser/operator ↔ Studio localhost HTTP/WebSocket API.
2. Studio ↔ managed MCP child processes.
3. Studio ↔ managed tunnel child process.
4. Studio ↔ local configuration/filesystem/secret references.
5. Tunnel runtime ↔ external control plane/tunnel provider.
6. Future Studio ↔ auto-discovered source trees.

## Milestone 3 security properties

- Studio rejects non-loopback HTTP bind addresses.
- Browser lifecycle mutations enforce same-origin `Origin`/`Host` alignment when `Origin` is present.
- WebSocket upgrades use the same browser-origin check.
- Wildcard CORS is not enabled.
- API callers cannot submit arbitrary executable paths, shell command strings, argument arrays, config-file paths, or PIDs.
- Only statically configured MCP definitions and the statically configured tunnel runtime can be started.
- Tunnel startup argv is constructed internally as `run --config <validated-config-file>`.
- Commands are executed directly via `tokio::process::Command`, not shell interpolation.
- Studio signals only PIDs returned from processes it spawned and currently tracks.
- Tunnel runtime and tunnel config paths are canonicalized and constrained below configured `tunnel.working_dir`.
- Tunnel runtime must exist, be a regular file, and have an executable permission bit.
- Tunnel secret references are resolved server-side only.
- Tunnel status/events do not serialize executable paths, config paths, secret references, or secret values.
- Resolved tunnel secret values are redacted before tunnel logs enter the in-memory log buffer or WebSocket stream.
- Common secret-bearing tunnel log fields are redacted defensively.
- Recent log buffers are bounded.
- Process state uses generation guards so stale monitor tasks cannot overwrite newer process state.
- WebSocket fan-out is bounded and lagged subscribers are resynchronized instead of blocking supervisors.
- Studio shutdown attempts to stop every owned MCP process and the owned tunnel process.

## Primary threats and controls

### Arbitrary command execution

Risk: an API or browser caller causes Studio to execute attacker-controlled commands.

Controls:

- No raw shell-command API.
- No caller-supplied executable, config path, argument array, or PID in lifecycle HTTP requests.
- MCP executable/args remain local typed configuration.
- Tunnel executable/config paths remain local typed configuration.
- Tunnel argv is fixed by Studio rather than browser-configurable.
- Milestone 3 registry remains static and loaded locally.

Residual risk: anyone able to modify Studio's local configuration can alter configured executables or secret references. Configuration file permissions remain part of host security.

### Tunnel executable/config path escape

Risk: a tunnel configuration points outside the intended runtime bundle or uses a symlink to escape the configured directory.

Controls:

- `working_dir`, `runtime`, and `config_file` are canonicalized before spawn.
- The canonical runtime and config file must remain below canonical `tunnel.working_dir`.
- Runtime must be a regular executable file.
- Config must be a regular file.

Residual risk: the configured tunnel working directory itself is trusted local configuration. Milestone 3 does not implement a global allow-list of absolute host roots beyond that server-side boundary.

### Cross-origin localhost control

Risk: a malicious remote webpage causes a user's browser to send privileged lifecycle requests to Studio on `127.0.0.1`.

Controls:

- MCP and tunnel lifecycle POST requests validate `Origin` when present.
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
- No remote Studio mode is supported in Milestone 3.

Remote operation must not ship until authentication, authorization, CSRF/CORS behavior, TLS/tunnel exposure, and session policy are explicitly designed and reviewed.

### Process ownership confusion / PID misuse

Risk: Studio kills an unrelated host process.

Controls:

- PID comes only from `Child::id()` for a Studio-spawned child.
- API does not expose kill-by-PID.
- State tracks whether the corresponding child is running/stopping.
- Generation guards prevent older monitor tasks from mutating newer runtime state.
- Tunnel stop/restart signals only the currently tracked Studio-owned tunnel PID.

Residual risk: PID reuse between detecting a hung process and signal escalation is theoretically possible. Production hardening should prefer stronger OS process identity/handle semantics where available.

### Orphan child processes

Risk: Studio exits while managed processes remain alive.

Controls:

- Graceful Studio shutdown calls tunnel shutdown plus MCP `shutdown_all()`.
- Spawned children use `kill_on_drop(true)` as a local-process safety net.
- Lifecycle tests verify supervisor shutdown behavior for tunnel fixtures.

Further hardening may add process groups/session management where managed runtimes spawn descendants.

### Tunnel secret leakage through API/events/UI

Risk: credentials or private configuration are serialized to browser-visible surfaces.

Controls:

- `TunnelStatus` contains only operational state and counters.
- Runtime path, config path, environment map, and secret-reference model are not part of public status/event types.
- Frontend tunnel types contain no secret/config fields.
- Snapshot and incremental WebSocket tunnel events carry only public operational fields and redacted log entries.

Residual risk: the tunnel runtime may emit sensitive material in formats not recognized by generic redaction. Exact resolved secret values are always redacted when Studio supplied them, but secrets sourced internally by the tunnel runtime may require runtime-specific patterns.

### Tunnel secret leakage through logs

Risk: the tunnel process writes secrets to stdout/stderr and Studio exposes them through REST/WebSocket/dashboard.

Controls:

- Exact secret values resolved by Studio are replaced before buffering or publishing.
- ANSI sequences are removed before display.
- Common key/value names such as token, secret, password, credential, and authorization are defensively redacted.
- Secret values are not written by Studio lifecycle log messages.

Residual risk:

- Redaction is best-effort for secrets unknown to Studio.
- Encoded, transformed, fragmented, or unusually formatted secret material may evade generic pattern matching.
- Security review of actual tunnel runtime output remains part of the real-runtime smoke gate.

### Secret-reference misuse

Risk: a local tunnel configuration reads unintended secret files or environment variables.

Controls:

- Secret references are server-side configuration only and never browser supplied.
- Missing or empty secret references fail startup.
- Resolved values are held only in process environment/redaction memory and are not returned by status APIs.

Residual risk: a user with permission to edit Studio configuration can reference files readable by the Studio process. Host filesystem permissions remain authoritative.

### Tunnel accidental public exposure

Risk: starting the managed tunnel exposes services externally in an unintended way.

Controls:

- Tunnel is stopped by default when Studio starts.
- Startup remains an explicit lifecycle action.
- Studio does not synthesize tunnel destinations or browser-supplied routing arguments.
- The existing reviewed `tunnel-client/config.yaml` remains authoritative for routing/channel behavior.
- Studio itself remains bound to loopback and is not automatically exposed by tunnel management.

Residual risk: the external exposure semantics are determined by the configured tunnel runtime/configuration and upstream control plane. Operators must review that configuration before start. Remote Studio access is still unsupported.

### Denial of service through lifecycle calls

Risk: repeated start/restart operations exhaust resources.

Controls now:

- Duplicate starts are rejected.
- One tunnel runtime record exists for Milestone 3.
- UI disables conflicting lifecycle controls during transitions/request execution.

Deferred controls:

- Restart backoff.
- API rate limiting.
- Crash-loop circuit breaker.
- Operational quotas.

These remain later hardening work.

### WebSocket subscriber pressure

Risk: a slow browser client blocks runtime event production or consumes unbounded memory.

Controls:

- MCP and tunnel event sources use bounded Tokio broadcast channels.
- Slow subscribers receive lag errors rather than blocking publishers.
- Lagged clients receive `resync_required` and a fresh combined snapshot.
- Browser reconnect backoff is capped to avoid a tight retry loop.

Residual risk: many simultaneous local browser clients can still create CPU/network load. Multi-user fan-out limits are deferred until remote/multi-user mode exists.

### Log/memory exhaustion

Risk: noisy child output consumes unbounded memory.

Controls:

- MCP and tunnel recent logs use fixed-capacity ring buffers.
- Oldest entries are discarded when capacity is reached.

Residual risk: extremely high stdout/stderr rates can still consume CPU and WebSocket bandwidth. Rate/byte limits may be added during hardening.

### Realtime state inconsistency

Risk: a browser misses events and displays stale lifecycle state.

Controls:

- WebSocket sends a complete initial snapshot containing MCP and tunnel status.
- Broadcast lag emits `resync_required` followed by a new snapshot.
- Browser can refetch REST state.
- Log sequence numbers deduplicate overlap between REST history and WebSocket events.
- Browser refresh rehydrates state from backend sources rather than persistent client state.
- Backend lifecycle validation remains authoritative regardless of UI state.

### MCP log secret leakage

Risk: MCP child processes write secrets to stdout/stderr and Studio exposes them.

Current limitation:

- MCP logs remain captured verbatim in Milestone 3.
- Tunnel-specific redaction is not generalized to MCP logs yet.

Controls:

- MCP environment configuration is not included in `ProcessStatus` or dashboard status payloads.
- Managed MCP servers are operationally required not to print credentials or secret environment values.

Production requirement:

- General secret-aware redaction before persistent storage or broader remote/browser deployment.

### MCP path traversal / executable escape

Risk: future registry editing or discovery points commands outside the allowed MCP root.

Milestone 3:

- MCP configuration is local/static and no browser endpoint mutates executable paths.

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

### Supply-chain compromise

Controls:

- `Cargo.lock` committed.
- Rust toolchain pinned to 1.98.1.
- Rust quality gates include locked release builds and `cargo audit`.
- Frontend uses a committed `pnpm-lock.yaml`.
- TypeScript/ESLint compatibility-sensitive toolchain versions are pinned through the lockfile/release gates.

## Security release gate

Milestone 3 must not close with unresolved known Critical or High severity findings in privileged lifecycle, browser-origin handling, tunnel executable/config confinement, secret handling, process ownership, or tunnel exposure.

The real-runtime smoke test must additionally inspect tunnel logs for credential/token leakage before release.

## Review triggers

Update this threat model whenever adding or materially changing:

- process spawn/kill behavior;
- descendant/process-group handling;
- browser origin/CORS/CSRF handling;
- WebSocket/event fan-out behavior;
- tunnel executable/configuration policy;
- tunnel secret handling/redaction;
- discovery/registration;
- editable executable configuration;
- persistent secrets;
- remote access/authentication;
- MCP gateway/proxying;
- plugins/adapters;
- multi-host control.
