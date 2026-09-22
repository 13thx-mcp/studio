# Mission 7 — Gateway Coordination, Concurrency & Tool Safety

**State:** M7.0 FROZEN / M7.1–M7.8 IMPLEMENTED / M7.9 QUALIFIED / CLOSURE FIXES INTEGRATED ON MAIN / RELEASE PUBLICATION PENDING
**Studio planning baseline:** `main` at `48906d892fb37275f8b2d10b73b7cad1eec355c9` (clean, `origin/main` aligned).  
**Gateway audit baseline:** `main` at `02a46af100c0cee5e011b8bc74079d7a78dd3658` (clean).  
**Filesystem audit baseline:** `main` at `0f09a665bb94af8388e01051d87cb3f865d4310d` (clean).  
**Fleet observation:** current worktree is on `feature/sonarqube-main-gate`; do not mix M7 Fleet changes into that branch.  
**Studio target:** `v0.7.0-beta`.

## Purpose

M7 turns Gateway from a catalog/router into a bounded request-coordination boundary while preserving typed domain MCPs as the capability/security boundary.

The plan deliberately does **not** create a general-purpose worker, make chat/session identity an authorization boundary, or promote M6 SQLite to live request authority.

## Main implementation decisions to freeze before coding

1. Gateway owns an in-memory request registry and scheduler for live request state. M6 receives sanitized historical projections only.
2. Request class is server-owned policy metadata (`read | mutation | long-running | control`), never caller supplied.
3. Admission and queueing are bounded before dispatch. Once a mutation is dispatched, timeout/cancel/disconnect must never be reported as proof that the side effect did not occur.
4. Gateway must keep the child call alive independently of the upstream caller long enough to classify the terminal result when possible. It must never automatically replay a mutation with a pending/unknown outcome.
5. Drain is an explicit control contract used before Gateway update/restart/reload and before a child restart that could interrupt active work.
6. Child restart becomes per-child and bounded. A failed child must not force healthy siblings through an avoidable full snapshot restart.
7. Global Gateway policy should be separated from per-child launch definitions. Proposed layout:
   - `runtime/gateway/gateway.yaml` — global request limits, active profile, payload/artifact policy;
   - `runtime/gateway/servers.d/*.yaml` — child launch, tool exposure/classification, per-child/tool limits and restart policy.
   This layout must be frozen in M7.0 because Fleet and Studio reconciliation currently know only the child files.
8. Filesystem v2 uses content revisions (SHA-256 for bounded text) as the correctness mechanism for stale-write detection. In-process locks are optional coordination only.
9. Tool profiles are operator/server policy. Hidden tools remain implemented but non-routable through the active profile.
10. Tunnel integration must target the supported upstream contract, not assume the currently installed run-only `tunnel-client-runtime-cloudflared` exposes the full administrative/runtime CLI.

## Frozen prerequisite ADRs

- [ADR 0026 — Gateway request outcome / no replay](../../adr/0026-gateway-request-outcome-no-replay.md)
- [ADR 0027 — Private Gateway control socket / drain ownership](../../adr/0027-gateway-control-socket-drain.md)
- [ADR 0028 — Bounded Gateway scheduler / concurrency](../../adr/0028-gateway-bounded-scheduler-concurrency.md)
- [ADR 0029 — Per-child runtime generation / recovery](../../adr/0029-gateway-child-generation-recovery.md)
- [ADR 0030 — Filesystem v2 revision / CAS / patch](../../adr/0030-filesystem-v2-revision-cas-patch.md)
- [ADR 0031 — Gateway policy / Fleet schema migration](../../adr/0031-gateway-policy-fleet-schema.md)
- [ADR 0032 — Tunnel dual-artifact adapter](../../adr/0032-tunnel-dual-artifact-adapter.md)
- [ADR 0033 — Gateway payload and artifact bounds](../../adr/0033-gateway-payload-artifact-bounds.md)
- [ADR 0034 — Gateway profile/resource/progress boundary](../../adr/0034-gateway-profile-resource-progress-boundary.md)
- [ADR 0035 — Gateway history privacy events](../../adr/0035-gateway-history-privacy-events.md)
- [M7 Studio ↔ Gateway integration](M7-STUDIO-INTEGRATION.md)
- [M7 qualification and closure audit](M7-QUALIFICATION.md)

## Package map

| Package | Scope | Primary repos |
|---|---|---|
| M7.0 | architecture/protocol/config freeze, source prerequisites, quality-gate cleanup | studio, gateway, filesystem, fleet |
| M7.1 | request identity, lifecycle, scheduler, queue/backpressure, cancellation/outcome semantics | gateway |
| M7.2 | active registry, drain contract, safe reload/update/restart integration | gateway, studio, fleet |
| M7.3 | per-child restart backoff, crash-loop detection, circuit breaker, sibling isolation | gateway |
| M7.4 | Filesystem v2 range/search/metadata/revision/CAS/patch | filesystem |
| M7.5 | request/response budgets, oversized-result guards, ephemeral artifacts | gateway |
| M7.6 | profiles, explicit workspace context, resources and progress forwarding; aliases retired by ADR 0034 | gateway, fleet |
| M7.7 | Gateway request/resilience/payload telemetry into M6 history | gateway, studio |
| M7.8 | official tunnel-client native runtime adapter and release-layout decision | studio, fleet |
| M7.9 | source-less/native qualification, soak/fault injection, release closure | all affected repos |

M7.4 and M7.8 may develop in parallel after M7.0. M7.7 starts only after request state/outcome/payload schemas are stable. M7.9 joins every package.

## Dependency order

```text
M7.0 contract freeze
  -> M7.1 request coordination core
      -> M7.2 drain/lifecycle integration
          -> M7.3 child resilience
      -> M7.5 payload guards
      -> M7.6 profiles/resources/progress
  -> M7.4 filesystem v2
  -> M7.8 tunnel adapter

M7.1 + M7.2 + M7.3 + M7.4 + M7.5 + M7.6
  -> M7.7 historical telemetry
      -> M7.9 qualification/closure

M7.8 -------------------------------------> M7.9
```

## Non-goals

- no arbitrary browser/client command execution;
- no persistent queue/replay engine for tool mutations;
- no distributed transaction across Gateway, children and Studio history;
- no authorization based on request/session/workspace alias identity;
- no automatic retry of unknown-outcome mutations;
- no replacement of typed Git/Filesystem/Exec/hardware MCPs with one broad worker;
- no M8 unattended update policy, auto-recovery policy or remote-auth work;
- no raw tool arguments/results persisted into M6 by default.

## Implementation entry point

Read in this order:

1. [SOURCE-AUDIT.md](SOURCE-AUDIT.md)
2. [RMCP-PROTOCOL-SPIKE.md](RMCP-PROTOCOL-SPIKE.md)
3. [COMMON.md](COMMON.md)
4. [M7.0-ARCHITECTURE-PROTOCOL-FREEZE.md](M7.0-ARCHITECTURE-PROTOCOL-FREEZE.md)
5. [M7-TASKS.md](M7-TASKS.md)
6. [M7-VERIFICATION-MATRIX.md](M7-VERIFICATION-MATRIX.md)
7. [M7-STUDIO-INTEGRATION.md](M7-STUDIO-INTEGRATION.md)
8. [M7-QUALIFICATION.md](M7-QUALIFICATION.md)

Do not start production M7 code until every M7.0 blocking prerequisite is either resolved or explicitly accepted with a testable alternative.
