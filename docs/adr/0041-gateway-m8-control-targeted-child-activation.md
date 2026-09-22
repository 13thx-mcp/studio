# ADR 0041 — Gateway M8 Control Capabilities and Targeted Child Activation

- **Status:** Accepted for M8.2/M8.4/M8.6 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-03, P-06, capability negotiation, Gateway-managed child activation

## Context

Gateway control protocol v1 already exposes global drain state, active/queued counts, catalog/profile generations, history, reload and resume. Studio currently ignores some of that status and generic MCP update has no Gateway child coordination.

Replacing a flat binary on disk does not replace an already-running Gateway child process. M8 auto-update-safe therefore needs a safe way to restart only the target child after binary activation without restarting healthy siblings or replaying requests.

## Decision

Gateway keeps control protocol version 1 and extends it additively.

New Studio must first call `status` and inspect advertised capabilities before sending an M8-only action. Old Gateway remains manually compatible; absence of a required capability blocks unattended mutation.

Minimum M8 capabilities:

```text
m8_safety_holds
m8_child_status
m8_targeted_child_restart
```

## Status response additions

The v1 response adds:

- `capabilities: string[]`;
- `automation_safety_available: bool`;
- `safety_hold_count: usize`;
- existing `active_requests` / `queued_requests`;
- `children` bounded summaries when requested/status-sized.

A child summary contains only:

```text
name
enabled
running
generation
recovery_state
consecutive_failures
retry_at_ms?
tool_count
```

No executable path, argv, environment or tool payload is exposed through the control socket response.

## New control actions

M8 adds:

```text
holds
resolve_hold
restart_child
```

Request fields are additive and validated with `deny_unknown_fields` semantics:

- `child`;
- `hold_id`;
- `resolution`;
- `drain_generation`.

Old Gateway rejects unknown actions/fields, but new Studio never sends them without capability proof.

## Targeted child restart contract

M8 initial child activation deliberately uses the existing **global Gateway drain** rather than introducing a second per-child scheduler.

Flow:

1. Studio obtains a global drain generation through the existing control action.
2. Drain closes admission and waits until active/queued obligations are zero under M7 semantics.
3. Studio performs the component binary transaction while Gateway remains drained.
4. Studio sends `restart_child(child, drain_generation)`.
5. Gateway verifies:
   - it is still in `DRAINED`;
   - generation matches;
   - child exists/enabled;
   - no new work has been admitted;
   - target child config/policy remains valid.
6. Gateway replaces only the target child process/runtime generation using the current trusted child config.
7. Candidate child must initialize and provide a valid tool catalog under existing policy/allowlist rules.
8. Gateway commits the new child runtime/generation and returns the new child summary.
9. Healthy sibling process generations remain unchanged.
10. Studio verifies target running identity/version through the component/Gateway integration contract.
11. Studio resumes the same drain generation.

If targeted restart fails, Gateway retains or restores the last known good child runtime when possible and leaves admission closed until Studio decides rollback/resume. It never restarts unrelated healthy siblings as a shortcut.

## Binary rollback interaction

If Studio restores the old binary after a failed activation, it requests another targeted restart under the still-active drain generation and verifies the restored child before resuming admission.

A stale drain generation, running Gateway state, missing child, capability mismatch or candidate catalog failure blocks the action.

## Safety holds

Open durable holds do not count as active requests, but Studio SafetyGate separately requires `safety_hold_count == 0` and `automation_safety_available=true` before unattended mutation.

## Alternatives considered

### Per-child drain queues

Rejected for M8 initial release. Global drain already has qualified M7 no-replay semantics and avoids duplicating scheduler complexity.

### Full Gateway reload after every generic MCP update

Rejected because it needlessly churns healthy sibling generations.

### Replace binary and wait for crash/restart policy

Rejected because running identity could remain old indefinitely and success would be ambiguous.

## Consequences

- M8 reuses the strongest existing M7 drain proof;
- generic child activation can prove a new Gateway child generation;
- healthy siblings remain isolated;
- old Gateway versions fail unattended mutation closed without blocking manual operation.
