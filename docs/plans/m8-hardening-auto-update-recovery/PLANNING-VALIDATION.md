# M8 Planning Validation

**Review date:** 2026-09-22  
**State:** PLANNING PACKAGE REVIEWED / M8.0 READY / PRODUCTION IMPLEMENTATION BLOCKED ON CONTRACT FREEZE

## Package completeness

The planning package contains:

- source audit and prerequisites;
- common authority/safety invariants;
- target architecture;
- update/reconciliation/restart scheduling policy;
- recovery and unknown-outcome safety contract;
- ADR decision plan;
- dependency-ordered M8.0–M8.8 execution checklist;
- R01–R65 verification matrix and Q01–Q15 qualification scenarios;
- package-specific entry/exit criteria;
- adversarial review.

No production M8 behavior is claimed implemented by these documents.

## Baseline

- Studio entry line: v0.7.1 / M7 exact-main closure.
- M7 request coordination/no-replay/drain contracts remain mandatory.
- Fleet pure render-plan remains desired-config boundary.
- M6 history remains non-live evidence.
- Runtime-only darwin-arm64 remains the supported production target inherited from prior milestones until a later explicit support expansion.

## Review findings incorporated

### Finding A — timer-wrapper design would be unsafe

Calling existing manual endpoints from a periodic task would not provide persisted backoff/circuit, config freshness, unknown-outcome hold or shared audit ownership.

**Disposition:** one internal AutomationController calls domain owners directly through a frozen policy/SafetyGate.

### Finding B — Gateway unknown outcome is not restart-safe

M7 telemetry is bounded/in-memory and omitted request class.

**Disposition:** classed telemetry + Gateway-owned durable safety holds are M8 blockers before automatic mutation.

### Finding C — generic flat MCP activation has dual runtime owners

A binary may be used by Studio Supervisor and Gateway child. On-disk replacement does not update an already-running Gateway child.

**Disposition:** targeted Gateway child quiesce/restart/verify is required for auto activation.

### Finding D — staged bytes outlive in-memory transaction authorization

Generic/Gateway/Fleet transaction maps are process-local while ready staging persists.

**Disposition:** previous-process staging is never automatic activation authority; reprepare or safe cleanup.

### Finding E — Fleet owns Studio config

Automation config only in Studio would create split desired state.

**Disposition:** Fleet host schema owns automation policy on Fleet-managed hosts and renders deterministic Studio config.

### Finding F — old Studio can run stale policy after reconciliation

Managed Studio config may change and require restart.

**Disposition:** destructive automation blocks while Studio config activation is pending/unknown; prepared state is policy-fingerprint bound.

### Finding G — release target/tool mismatch

M8 roadmap target is `v0.8.0-beta`, while the current guarded Git release-tag operation accepts only plain `vMAJOR.MINOR.PATCH`.

**Disposition:** M8.0 must either accept guarded SemVer prerelease support in Git MCP or explicitly change the target. The proposed direction is guarded prerelease support; generic tagging must not be used to bypass release evidence.

## Scope review

The following remain outside M8 and are not required for closure:

- remote auth/RBAC;
- public/non-loopback Studio;
- Linux support;
- central multi-host orchestration;
- general dependency compatibility matrix/signing strategy;
- arbitrary job/workflow queue;
- Workspace Skill Runtime implementation.

M8 does require narrow compatibility/capability checks necessary for its own safe automatic actions.

## Implementation readiness

### Ready now

M8.0 documentation/ADR work and source spikes may begin.

### Blocked until M8.0 exit

Any production background staging/restart/reconcile code.

### Hard-blocked until specific prerequisites qualify

`auto-update-safe` is blocked by:

- Gateway capability status;
- durable classed safety holds;
- targeted child activation;
- shared audit admission;
- prepared staging restart semantics;
- Fleet policy authority;
- loaded policy freshness.

## Review conclusion

The package is internally suitable to begin M8.0 contract/ADR implementation. The dependency graph deliberately prevents the highest-risk feature, automatic activation, from being implemented first.
