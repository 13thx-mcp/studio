# Mission 8 — Hardening, Auto-Update Policy & Recovery

**State:** M8.0–M8.3 VERIFIED-CLOSED / M8.5 + M8.6 READY / M8.4 SAFETY-BLOCKED
**Target:** Studio `v0.8.0-beta`
**Entry baseline:** Studio `v0.7.1`, M7 exact-main closure at `ab9aca19c3fc6091ca3d9d169f5fc2a314b8d476`
**Primary repositories:** Studio, Gateway, Fleet, and Git MCP for guarded release-control changes; other generic MCP repositories participate in compatibility/activation verification but do not gain broad new authority.

## Purpose

M8 turns the qualified M7 local control plane into a bounded unattended operator for a single runtime host.

The milestone adds periodic observation, constrained preparation, optional safe activation, automatic managed-drift repair, restart policy, and recovery workflows while preserving the authority boundaries established by M5–M7.

M8 is **not** "run the existing manual API on a timer." Automation must have its own explicit policy, durable safety state, bounded retry/circuit behavior, maintenance-window semantics, and fail-closed admission.

## Entry invariants inherited from M7

- Gateway owns live request admission, drain, concurrency, outcome classification and child resilience.
- Studio/Fleet own runtime/update policy and verified release activation.
- M6 SQLite is historical/audit evidence, not live execution authority.
- M5 journals/rollback material remain component recovery authority.
- Unknown mutation outcomes are never automatically replayed.
- Fleet `render-plan --json` is the desired runtime-config contract.
- typed domain MCPs remain the capability boundary.
- published release tags are immutable.
- runtime-only hosts must not require source trees, Rust, Cargo, Node, pnpm or build outputs.

## M8 outcome

A host may be configured in one of the following update modes:

```text
manual
notify-only
auto-prepare
auto-update-safe
```

and one reconciliation mode:

```text
manual
notify-only
auto-reconcile-safe
```

The defaults remain `manual`.

No automatic activation may occur unless every component-specific and global safety precondition is proven at the moment of activation.

## Read order

1. [SOURCE-AUDIT.md](SOURCE-AUDIT.md) and [M8.0 source spikes](M8.0-SOURCE-SPIKES.md)
2. [COMMON.md](COMMON.md)
3. [ARCHITECTURE.md](ARCHITECTURE.md)
4. [POLICY-AND-SCHEDULING.md](POLICY-AND-SCHEDULING.md)
5. [RECOVERY-AND-SAFETY.md](RECOVERY-AND-SAFETY.md)
6. [ADR-PLAN.md](ADR-PLAN.md)
7. [M8-TASKS.md](M8-TASKS.md)
8. [M8-VERIFICATION-MATRIX.md](M8-VERIFICATION-MATRIX.md)
9. [M8.1 qualification](M8.1-QUALIFICATION.md), [M8.2 qualification](M8.2-QUALIFICATION.md), and [M8.3 qualification](M8.3-QUALIFICATION.md), then package plans M8.4 → M8.8 in dependency order.

## Package map

| Package | Deliverable | Primary repos |
|---|---|---|
| M8.0 | architecture, prerequisites, schemas, safety contracts and release-control contract frozen | studio, gateway, fleet, git |
| M8.1 | automation state store, scheduler, shared audit admission, config | studio |
| M8.2 | health abstraction, restart policy, live Gateway safety snapshot | studio, gateway |
| M8.3 | update checks, target selection, notify-only and auto-prepare | studio |
| M8.4 | safe activation gate and component activation topology | studio, gateway |
| M8.5 | startup/periodic Fleet reconciliation with bounded circuit | studio, fleet |
| M8.6 | unknown-outcome hazard holds, restart recovery, staging/lock cleanup | gateway, studio, fleet |
| M8.7 | operator APIs/UI, diagnostics and resolution workflows | studio |
| M8.8 | fault injection, runtime-only soak, regression and release closure | all affected |

## Dependency order

```text
M8.0 contract freeze
  ├─> M8.1 automation foundation
  │     ├─> M8.3 check / notify / auto-prepare
  │     └─> M8.5 periodic reconciliation
  ├─> M8.2 health + restart + Gateway safety status
  │     └─> M8.4 safe activation
  └─> M8.6 durable hazard / recovery contracts
          └─> M8.4 safe activation

M8.3 + M8.4 + M8.5 + M8.6
  └─> M8.7 operator surface
        └─> M8.8 qualification / closure
```

M8.4 must not start production auto-activation until M8.2 and the hazard portion of M8.6 are qualified.

## Explicit non-goals

- remote Studio exposure, authentication or RBAC — M9;
- Linux production support — later explicit support matrix work;
- central multi-host rollout/control — post-v1;
- automatic source Git merge/rebase/conflict resolution;
- automatic downgrade;
- automatic retry of an operation with unknown side-effect outcome;
- generalized job queue or distributed workflow engine;
- arbitrary shell automation;
- Workspace Skill Runtime implementation;
- changing M6 SQLite into scheduler/lock/recovery authority;
- replacing component-specific health checks with one weak generic ping.

## Definition of Done

M8 closes only when:

- `manual` remains behaviorally backward-compatible;
- unattended checks/reconciliation cannot create tight loops;
- auto-prepare cannot activate runtime bytes;
- auto-update-safe proves all frozen preconditions at activation time;
- Gateway unknown mutation/control/long-running outcomes survive restart as safety holds;
- operator resolution never replays the original request;
- Gateway-managed child activation is coordinated and post-activation version/generation is verified;
- prepared staging cannot be silently reused across an unqualified Studio restart;
- source-present dirty/conflicted repositories block automatic activation;
- reconciliation only repairs current managed-safe drift and stops on ambiguity/circuit-open;
- rollback/recovery remains component-owned and is verified after forced interruption;
- runtime-only operation passes with no source/build toolchain;
- long-running soak shows bounded timers, retries, staging, disk use and state;
- M5/M6/M7 regressions remain green;
- no known Critical/High security blocker remains;
- release/version/publication/deployment states remain explicit and separate.
