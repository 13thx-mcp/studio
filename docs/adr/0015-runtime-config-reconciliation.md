# ADR 0015 — Fleet-Managed Runtime Configuration Reconciliation

## Status

Accepted and implemented in M5.9A.

## Context

M5.9 deliberately validates Fleet render compatibility without rewriting generated runtime configuration. A runtime incident showed that this leaves an operational gap: the deployed Fleet bundle and host profile can be valid while `runtime/tunnel-client/config.yaml` or `runtime/gateway/servers.d/*.yaml` remains stale.

The observed failure class included a tunnel YAML that referenced a removed source-tree Gateway/config path while a helper launch script supplied a correct CLI override. This split launch path masked canonical drift. In the same incident, Gateway locally reported the expected child-tool aggregate while a connected client session temporarily retained an older/incomplete callable tool catalog.

Process health, child count, and aggregate tool count are therefore insufficient to prove desired-state convergence.

## Decision

Studio will introduce a dedicated runtime reconciliation domain after M5.9.

Fleet desired render plus the active server-selected host profile is the authority for Fleet-owned generated runtime configuration. Studio will compare that desired state with active generated files and live Gateway catalog state.

Fleet must expose a side-effect-free pure-render contract (for example `fleetctl render-plan --host <host> --json`) so Studio consumes Fleet-authored desired bytes/digests instead of reimplementing render logic. The result is deterministic and identifies only trusted relative runtime destinations.

Studio persists a versioned managed-state manifest containing last-known-managed SHA-256 values and a generation. Automatic repair is allowed only when active bytes equal the previous managed fingerprint and differ from the new desired bytes. Unknown edits are `UNMANAGED_CONFLICT`. Legacy adoption requires either an explicitly recognized trusted legacy-generated form or an explicit operator adoption action; it is never inferred merely from filenames.

Reconciliation is transactional:

- target paths are fixed by trusted runtime roots and Fleet policy;
- desired output is rendered/validated before mutation;
- active managed files are snapshotted and hashed;
- only files proven Fleet-owned/managed-safe may be rewritten automatically;
- unknown local edits become an explicit unmanaged conflict;
- changed Gateway configuration triggers a bounded Gateway reload/catalog verification;
- changed tunnel binding/config may restart the tunnel only when required and only if it was previously running;
- failure after mutation restores the captured configuration and re-verifies the restored runtime.

Gateway verification uses an independently derived expected tool-name set. Studio parses the trusted child definitions, directly starts every enabled deployed child from the flat trusted `bin/` root, obtains the complete paginated `tools/list`, validates original names and configured allowlists, applies allowlist matching before prefixing, rejects cross-child/builtin collisions, and adds the three Gateway built-ins. A separate standalone Gateway protocol probe must expose exactly that set. Aggregate count is diagnostic only and can never establish identity. Studio exposes a catalog fingerprint/generation for diagnostics.

All supported Gateway launch surfaces must resolve to the same canonical flat-bin/runtime-config binding. A correct CLI override is not allowed to hide a stale canonical tunnel configuration.

Launch-surface ownership is explicit. Fleet-generated tunnel YAML may be repaired when managed ownership is proven. Server-owned Studio/TunnelSupervisor configuration is validated against the same binding. The current `runtime/tunnel-client/run.sh` helper is not emitted by Fleet and is therefore validate-only unless a separate ownership/fingerprint contract is introduced; divergence is an unmanaged conflict, not an automatic rewrite.

Gateway `notifications/tools/list_changed` can request client refresh, but Studio will not claim that a remote client session refreshed unless an acknowledgement becomes observable. Local synchronization and remote-client freshness remain separate states.

M5.9A provides startup drift detection and the safe reconciliation primitive. Periodic unattended reconciliation, retry/backoff and loop circuit breaking are M7 policy.

## Implemented contract

Fleet implements the pure-render boundary as `fleetctl render-plan --host <host> --json`. The JSON is bounded and validated by Studio and contains only five approved runtime surfaces with root-relative destinations, exact base64 bytes, SHA-256, `fleet_managed` ownership and lifecycle effects.

Studio stores the managed baseline at `runtime/fleet/state/reconciliation.json`, atomically and with restrictive state-file permissions. The public reconciliation API exposes no raw bytes or internal paths. Evaluation is side-effect free. `check`, `adopt`, and `apply` acquire the same runtime-operation control lease before the reconciler-local lease; a conflicting operation receives a typed conflict. First-generation byte-for-byte legacy adoption is a guarded commit step after validation, not an evaluator side effect. `adopt` remains an explicit operator action that records current bytes without rewriting them; `apply` is allowed only from managed-safe drift.

The `ComponentCatalog` carries the shared in-process runtime-operation coordinator. Control-path operations (reconciliation, Fleet replacement, Gateway replacement, Tunnel replacement, and Studio self-update handoff) are mutually exclusive with component activation. Ordinary flat MCP updates retain per-component concurrency for distinct component identities. Acquisition order is fixed: shared runtime-operation lease first, manager-local transaction lease second; transactions never recursively reacquire their own lease.

For reconciliation, the commit boundary is the durable next managed-state manifest after file replacement, rerender validation, runtime verification, and catalog verification. Manifest persistence failure before that boundary enters compensation. Compensation restores both changed files and the previous manifest and re-verifies runtime/catalog identity. Cleanup failure after a durable commit is a warning, not a claim that rollback happened. Active managed hashes are rechecked immediately before mutation so an external edit discovered after evaluation fails closed.

Gateway `servers.d` changes rely on the Gateway's default 1500 ms configuration watcher; Studio waits beyond a watcher tick and then protocol-probes a standalone Gateway to prove exact local tool-name identity. Tunnel config changes restart the tunnel only if it was previously running. The current `run.sh` helper remains validate-only and is accepted only when its complete bytes match one of the explicitly supported canonical launcher templates; comments, shadow assignments, alternate roots, overriding arguments, shell substitutions outside the template, and symlink launchers fail closed. Client refresh beyond the local Gateway remains unproven and is represented as `unknown` or `refresh_pending`.

Studio configuration activation is tracked separately from file convergence. At startup the exact config bytes used for TOML parsing are hashed from that same read and paired with the canonical config path and a process-instance identity. Reconciliation persists a `studio_restart_pending` marker containing the reconciled `studio.config` digest and the process instance that requested restart. Repeated checks and browser reconnects retain this marker. It clears only when a different process instance proves it loaded the canonical `runtime/studio/studio.toml` path with the exact reconciled digest. Wrong paths, old bytes, or unknown loaded identity remain `restart_required`; unrelated Gateway-only reconciliation never creates a Studio restart requirement.

## Alternatives considered

### Keep M5.9 read-only render validation only

Rejected because it detects the incident but still requires manual shell/Fleet repair and can leave divergent launch paths in production.

### Rewrite all generated files on every Studio start

Rejected because ownership may be ambiguous and future config may contain operator-owned or secret-bearing fields. Unknown edits must fail closed.

### Treat process/Gateway tool count as synchronization proof

Rejected because a stale canonical config can be masked by another launch path, and equal counts do not prove equal tool identity.

### Restart tunnel/Gateway unconditionally

Rejected because it causes avoidable disruption and can create restart loops. Reconciliation must restart only affected owners and preserve prior running/stopped state.

## Consequences

- Generated runtime config becomes explicit desired-state inventory, not incidental files.
- Studio gains a reusable primitive for M7 safe automatic reconciliation.
- Fleet render output/ownership needs a deterministic fingerprint or equivalent managed-state contract.
- Runtime-only hosts can self-diagnose drift without source repositories.
- Client-catalog freshness remains partially unobservable until the tunnel/client protocol exposes acknowledgement/telemetry.
