# MCP Studio Threat Model

## Scope

MCP Studio is a privileged local control plane. As of Milestone 4 it can supervise registered MCP child processes, manage one separately configured secure tunnel runtime, persist MCP registration/configuration locally, and scan the configured MCP root for supported projects using metadata only.

SQLite history/metrics/audit persistence, automatic restart/backoff, remote authentication/RBAC, and MCP gateway traffic telemetry remain later milestones.

## Assets

- Host process execution capability.
- Persistent MCP registry/configuration.
- Configured MCP-root authority boundary.
- Managed child identity/PIDs and transient runtime state.
- MCP source trees and executable artifacts.
- Tunnel runtime/configuration and secret references.
- Operational stdout/stderr logs.
- Browser lifecycle/registry/discovery-control capability.

## Trust boundaries

1. Browser/operator ↔ Studio localhost HTTP/WebSocket API.
2. Studio registry/discovery service ↔ configured MCP root and project manifests.
3. Persistent registry file ↔ live registry service.
4. Registry service ↔ MCP supervisor launch configuration.
5. Studio ↔ managed MCP child processes.
6. Studio ↔ managed tunnel child process and server-side secret references.
7. Tunnel runtime ↔ external tunnel provider/control plane.

## Active M4 security properties

- Studio remains loopback-only.
- Privileged browser mutations and WebSocket upgrades enforce same-origin `Origin`/`Host` alignment.
- No API accepts raw shell commands or PIDs.
- Discovery never executes project code.
- Discovery never auto-registers or auto-starts a project.
- Registration requires explicit operator action and server-side candidate rescan.
- Persistent registry uses an explicit schema version and fails safely on malformed/unsupported state.
- Registry updates are complete-document atomic replacements; failed writes do not publish the candidate in memory.
- MCP project, working-directory, and executable paths are project/root confined.
- Absolute paths, traversal/root/prefix components, and symlink components are rejected.
- Executable/working-directory policy is revalidated immediately before spawn.
- Executable and arguments are structured data passed directly to `tokio::process::Command`; no shell interpolation is used.
- Disabled MCPs cannot start/restart.
- Edit, disable, and unregister are rejected while the MCP is active.
- Unregister removes Studio registration only and accepts no caller-supplied delete path.
- Studio signals only PIDs returned by children it spawned and currently tracks.
- Registry API/realtime payloads omit stored environment values.
- Tunnel remains a separate constrained lifecycle/secret domain with M3 log redaction controls.
- Recent logs and realtime fan-out remain bounded.

## Primary threats and controls

### Malicious project manifest / discovery-triggered execution

**Risk:** a project under the MCP root uses manifest content to cause Studio to execute code during scan or approval.

**Controls:**

- Rust discovery parses `Cargo.toml` as data only.
- Node discovery parses `package.json` as JSON only and never executes scripts/install hooks.
- Python discovery parses `pyproject.toml` as data only and never imports modules.
- Discovery invokes no shell, package manager, compiler, interpreter, or project executable.
- Registration is separate from discovery and never starts the project.
- One malformed/unreadable project is isolated from unrelated scan results.

**Residual risk:** manifest parsing still consumes attacker-controlled local data; parser/library vulnerabilities and resource-exhaustion cases remain supply-chain/robustness concerns.

### Path traversal / MCP-root escape

**Risk:** editable project/executable/working-directory paths escape the configured authority root.

**Controls:**

- MCP root is canonicalized by Studio.
- Persisted project paths are relative to MCP root.
- Executable and working-directory paths are relative to the registered project.
- Absolute paths and `..`, root, or platform-prefix components are rejected.
- Canonical project must remain under MCP root.
- Canonical executable/working directory must remain under the project.
- The same policy is revalidated before every spawn.

### Symlink escape

**Risk:** a path that appears confined lexically resolves through a symlink to another host location.

**Controls:**

- M4 rejects symlink components in registered project, working-directory, and executable paths rather than treating a mutable symlink target as an authorization boundary.
- Canonical containment is checked as a second layer.

**Residual risk:** a local actor with write access to ordinary path components can race filesystem replacement between validation and spawn. Revalidation narrows the window; descriptor-based execution/openat confinement is a possible later hardening step.

### Executable-path abuse / arbitrary host execution

**Risk:** registry editing turns Studio into a general process launcher.

**Controls:**

- Browser configuration uses a project-relative executable path, not a shell command.
- Executable must remain inside the registered project.
- Discovery registration initially accepts only executable candidates produced by the server-side metadata scan.
- Arguments are a structured list; no shell interpolation is performed.
- Executable must be a regular file at activation time.

**Residual risk:** code inside an explicitly registered project is operator-approved execution authority. Registering or editing an executable should therefore be treated as a privileged action.

### Argument injection

**Risk:** structured arguments are interpreted as shell syntax or mutate process selection.

**Controls:**

- Arguments are passed directly as argv elements to `Command`.
- No shell is invoked by Studio.
- Executable selection is independent of argument text and remains path-confined.

**Residual risk:** a managed MCP may itself interpret dangerous arguments. M4 confines executable authority but does not understand every child-specific option grammar; operators remain responsible for reviewed argument values.

### Environment/secret leakage

**Risk:** inherited environment values become visible through registry REST, WebSocket events, or UI.

**Controls:**

- Public registry DTOs omit the environment map entirely.
- M4 adds no generic browser environment/secret editor.
- Registry/discovery realtime events are invalidations without full configuration payloads.
- Tunnel secrets retain M3 server-side references/redaction.

**Residual risk:** legacy static MCP environment values may still exist server-side and managed MCP stdout/stderr remains a separate secret-leakage domain without generalized redaction.

### Registry file tampering / malformed persisted config

**Risk:** local modification or interrupted writes corrupt the execution policy.

**Controls:**

- Explicit schema version.
- Full validation on startup.
- Unsupported/malformed registry fails startup instead of falling back or overwriting it.
- Mutation writes a complete temp sibling, syncs it, atomically renames it, then publishes the new in-memory state.
- Deterministic map serialization makes operator review/diffing practical.

**Residual risk:** filesystem permissions remain the primary protection against a local actor who can intentionally rewrite the registry. File authentication/signing is not part of M4.

### Unauthorized registration/config mutation

**Risk:** a malicious webpage reaches localhost Studio and registers or changes executable configuration.

**Controls:**

- Registry/discovery mutations use the same browser same-origin check as lifecycle/tunnel mutations.
- Studio remains loopback-only.
- Foreign browser origins receive `403 Forbidden`.
- Discovery registration resolves the candidate server-side rather than accepting an arbitrary browser path.

**Residual risk:** local non-browser processes may call the API without `Origin` by design. Host account/process isolation is trusted until remote/multi-user authentication is introduced.

### Running-process config mutation / orphaned ownership

**Risk:** configuration changes underneath an active child and causes runtime/registry state to diverge.

**Controls:**

- Edit, disable, and unregister are rejected while state is `starting`, `running`, or `stopping`.
- Operator must explicitly stop first.
- Unregister removes only inactive runtime state.
- Studio continues to signal only the tracked owned PID.

### Duplicate or stale registry entries

**Risk:** multiple IDs refer to the same project or a removed entry remains startable through stale supervisor state.

**Controls:**

- Stable ID uniqueness is enforced.
- Duplicate registered project paths are rejected.
- Supervisor resolves live registry state for lifecycle operations instead of keeping an immutable startup-only registry snapshot.
- Disabled/unregistered state therefore takes effect without supervisor reconstruction.

### Persistence/runtime consistency

**Risk:** persistent mutation succeeds but runtime reconciliation fails, or memory changes before disk durability.

**Controls:**

- The registry object is the shared authority used directly by supervisor lifecycle resolution.
- Candidate state is published in memory only after successful persistence.
- No separate asynchronous supervisor reload step exists.
- Active mutations that would require reconciliation are rejected stop-first.

### TOCTOU between validation and spawn

**Risk:** filesystem contents change after validation but before `exec`.

**Controls:**

- Path/executable checks occur immediately before `Command::spawn`.
- Symlink components are rejected.
- Authorization remains scoped to the registered project root.

**Residual risk:** ordinary files can still be replaced by another local writer in the remaining window. Stronger descriptor/handle-based execution is deferred unless threat review requires it.

### Unregister/delete confusion

**Risk:** an unregister API accidentally deletes source code or an attacker supplies a delete target.

**Controls:**

- Unregister accepts an MCP ID only.
- It removes a registry record and inactive runtime state only.
- No filesystem deletion target exists in the API/domain operation.

### Process ownership / PID misuse

**Risk:** Studio signals an unrelated host process.

**Controls:**

- PID originates only from `Child::id()` for a Studio-spawned child.
- API accepts no PID.
- Generation guards prevent stale monitor tasks from overwriting newer runtime state.
- Stop/restart act only on currently tracked runtime state.

### Cross-origin localhost control

**Risk:** a remote webpage causes privileged requests to localhost Studio.

**Controls:**

- Browser mutations validate `Origin` when present and require matching `Host`.
- WebSocket upgrades use the same policy.
- Wildcard CORS is not enabled.
- Studio rejects non-loopback bind addresses.

### Tunnel secret/exposure threats

Tunnel management remains governed by ADR 0003/M3 controls:

- fixed server-side invocation shape;
- tunnel runtime/config confinement;
- server-side secret references;
- exact-secret/common-field log redaction;
- explicit lifecycle start;
- loopback-only Studio exposure.

M4 does not merge tunnel configuration into the MCP registry.

### Denial of service / log pressure

Current controls:

- duplicate starts are rejected;
- incompatible lifecycle/config mutations are rejected;
- MCP/tunnel recent log buffers are bounded;
- realtime uses bounded broadcast channels and resync on lag;
- one bad discovery candidate does not abort a whole scan.

Deferred controls include rate limiting, restart backoff/circuit breaking, and more explicit scan/manifest resource limits.

### Supply-chain compromise

Controls:

- `Cargo.lock` committed.
- Rust toolchain pinned to 1.98.1.
- release gates require formatting, clippy with warnings denied, tests, builds, dependency audit, and locked release build.
- frontend lockfile is committed and lint/typecheck/test/build are release gates.

## M4 security release gate

Milestone 4 must not close with an unresolved known Critical/High finding in:

- discovery-triggered execution;
- MCP-root/path/symlink confinement;
- executable/argument handling;
- secret exposure;
- registry persistence/tampering behavior;
- same-origin privileged mutations;
- process ownership;
- running-state registry mutation;
- unregister/delete separation;
- existing tunnel security guarantees.

## Review triggers

Update this threat model whenever materially changing process spawn/kill behavior, project/executable roots, symlink policy, discovery parsers, registry schema/persistence, browser-origin/CORS/CSRF policy, secret handling, remote access/authentication, gateway/proxying, plugins/adapters, or multi-host control.
