# MCP Studio Threat Model

## Scope

MCP Studio is a privileged local control plane. As of Milestone 1 it can start, stop, restart, and observe configured MCP child processes. Later milestones add tunnel control, discovery, persistence, and remote-facing capabilities.

## Assets

- Host process execution capability.
- MCP configuration.
- Managed child process identity/PIDs.
- Filesystem paths exposed by MCP servers.
- stdout/stderr operational logs.
- Future tunnel credentials/tokens.
- Future registry, metrics, and audit history.

## Trust boundaries

1. Browser/operator ↔ Studio localhost HTTP API.
2. Studio ↔ managed MCP child processes.
3. Studio ↔ local configuration/filesystem.
4. Future Studio ↔ tunnel runtime.
5. Future Studio ↔ auto-discovered source trees.

## Milestone 1 security properties

- Studio rejects non-loopback HTTP bind addresses.
- API callers cannot submit arbitrary executable paths, shell command strings, or PIDs.
- Only statically configured MCP definitions can be started.
- Commands are executed directly via `Command`, not through shell interpolation.
- Studio signals only a PID returned from a process it spawned and currently tracks.
- Recent log buffers are bounded.
- Process state uses generation guards so stale monitor tasks cannot overwrite a newer process instance.
- Studio shutdown attempts to stop every owned MCP process.

## Primary threats and controls

### Arbitrary command execution

Risk: an API caller causes Studio to execute attacker-controlled commands.

Controls:

- No raw shell-command API.
- No command/PID field in lifecycle HTTP requests.
- Structured executable and argument configuration only.
- Milestone 1 registry is static and loaded locally.
- Future editable registry requires path confinement and explicit policy review.

Residual risk: anyone able to modify Studio's local configuration can alter configured executables. Configuration file permissions remain part of host security in Milestone 1.

### Process ownership confusion / PID misuse

Risk: Studio kills an unrelated host process.

Controls:

- PID comes only from `Child::id()` for a Studio-spawned child.
- API does not expose a kill-by-PID operation.
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

### Denial of service through process lifecycle calls

Risk: repeated start/restart operations exhaust resources.

Controls now:

- Duplicate starts are rejected.
- One runtime record exists per configured MCP.

Deferred controls:

- Restart backoff.
- Rate limiting.
- Crash-loop circuit breaker.
- Operational quotas.

These are required before production-grade remote operation.

### Log/memory exhaustion

Risk: noisy child output consumes unbounded memory.

Controls:

- Per-MCP recent logs use a fixed-capacity ring buffer.
- Oldest entries are discarded when capacity is reached.

Residual risk: extremely high stdout/stderr rates can still consume CPU. Rate/byte limits may be added during hardening.

### Secret leakage through logs

Risk: MCP child processes write secrets to stdout/stderr and Studio exposes them through `/logs`.

Current limitation:

- Milestone 1 captures child output verbatim and does not yet implement a generic redaction engine.

Required operational rule:

- Managed MCP servers must not print credentials or secret environment values.

Production requirement:

- Secret-aware redaction before persistent storage or remote/browser streaming.

### Path traversal / executable escape

Risk: future registry editing or discovery points commands outside the allowed MCP root.

Milestone 1:

- Configuration is local/static and no browser endpoint mutates paths.

Required before editable registry/discovery:

- Canonicalize paths.
- Constrain allowed roots.
- Define symlink policy.
- Explicit approval before registration/execution.

### Unauthorized remote control

Risk: lifecycle endpoints become remotely reachable without authentication.

Controls:

- Loopback-only bind validation.
- Remote mode is not supported in Milestone 1.

Remote mode must not ship until authentication, authorization, CSRF/CORS behavior, and tunnel/TLS exposure are reviewed.

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

Tunnel lifecycle is not implemented in Milestone 1.

Before Milestone 3 release:

- tunnel executable policy;
- secret references/redaction;
- safe stopped default;
- public endpoint/authentication review.

### Supply-chain compromise

Controls:

- `Cargo.lock` committed for the application.
- Rust toolchain pinned to 1.98.1.
- CI uses locked dependency builds.
- `cargo audit` is a release gate.

## Security release gate

No release candidate may contain unresolved known Critical or High severity findings in privileged lifecycle, secret handling, path confinement, authentication, or tunnel exposure.

## Review triggers

Update this threat model whenever adding or materially changing:

- process spawn/kill behavior;
- descendant/process-group handling;
- tunnel management;
- discovery/registration;
- editable executable configuration;
- persistent secrets;
- remote access/authentication;
- MCP gateway/proxying;
- plugins/adapters;
- multi-host control.
