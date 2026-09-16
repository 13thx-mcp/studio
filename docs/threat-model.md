# MCP Studio Threat Model

## Scope

MCP Studio will become a privileged local control plane capable of starting/stopping MCP servers and secure tunnels. This document establishes the security baseline before privileged features are implemented.

## Assets

- Host process execution capability.
- MCP configuration.
- Tunnel credentials/tokens.
- Filesystem paths exposed by MCP servers.
- Operational logs and metrics.
- Registry and audit history.

## Trust boundaries

1. Browser ↔ Studio HTTP/WebSocket API.
2. Studio ↔ managed MCP child processes.
3. Studio ↔ tunnel runtime.
4. Studio ↔ filesystem/config/database.
5. Studio ↔ auto-discovered source trees.

## Primary threats and required controls

### Arbitrary command execution
Risk: a browser/API caller causes Studio to execute attacker-controlled shell commands.

Controls:
- No raw shell command API.
- Structured executable/argument model only.
- Executable allow-list/policy.
- No shell interpolation.

### Path traversal / escape
Risk: registry/discovery/config points outside the configured MCP root.

Controls:
- Canonicalize paths.
- Reject traversal outside configured root.
- Treat symlinks explicitly in future discovery design.

### Secret leakage
Risk: tunnel/API credentials appear in logs, API responses, diagnostics, or UI.

Controls:
- Secret references rather than plaintext where possible.
- Redaction layer before persistence/streaming.
- Never echo secret values back to browser after submission.
- Diagnostics must be secret-safe by construction.

### Malicious discovered project
Risk: scanning a directory causes untrusted code execution.

Controls:
- Discovery is metadata-only.
- Registration requires explicit approval.
- Registration does not imply execution.
- Start action remains explicit and policy-checked.

### Unauthorized remote control
Risk: Studio becomes remotely reachable without adequate authentication.

Controls:
- Localhost-only default.
- Milestone 0 rejects non-loopback bind addresses.
- Remote mode must not ship until authentication, authorization, CSRF/CORS and TLS/tunnel exposure are reviewed.

### Process ownership confusion
Risk: Studio kills unrelated processes.

Controls:
- Track process handles/PIDs started by Studio.
- Never kill arbitrary caller-supplied PID.
- Reconciliation/adoption, if added, requires an ADR and audited identity checks.

### Crash-loop resource exhaustion
Controls:
- Backoff and circuit breaker in hardening milestone.
- Restart limits and event audit.

### Log/disk exhaustion
Controls:
- Bounded in-memory buffers initially.
- Rotation/retention before production.

### Tunnel accidental exposure
Controls:
- Explicit tunnel state/config in UI.
- Safe default: tunnel stopped unless configured otherwise.
- Review public endpoint/authentication before remote production use.

### Supply-chain compromise
Controls:
- Locked dependencies.
- CI dependency audit.
- Review release provenance before v1.0.

## Security release gate

No release candidate may contain unresolved known Critical or High severity findings in privileged lifecycle, secret handling, path confinement, authentication, or tunnel exposure.

## Review triggers

Update this threat model whenever adding:
- process spawn/kill behavior;
- tunnel management;
- discovery/registration;
- persistent secrets;
- remote access/authentication;
- MCP gateway/proxying;
- plugins/adapters;
- multi-host control.
