# ADR 0037 — M8 Automation Controller Ownership and Durable Policy State

- **Status:** Accepted for M8.1 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-01, P-02, P-13, automation ownership, durable scheduler/circuit state, shutdown

## Context

M7 closes live request coordination and runtime mutation ownership but does not provide unattended policy. Existing manual APIs already call typed domain owners and use RuntimeOperationCoordinator plus M6 audit admission. Simply running those endpoints from timers would create hidden authority, duplicate audit logic, and no restart-safe backoff/circuit semantics.

M6 SQLite is historical evidence only and cannot become a scheduler, work queue, mutex, or recovery authority.

## Decision

Studio introduces exactly one in-process `AutomationController` per Studio process.

The controller owns only:

- due-time evaluation;
- policy selection;
- bounded retry/backoff/circuit state;
- deferral accounting;
- activation-time SafetyGate evaluation;
- references to current-process prepared transactions;
- sanitized operator-visible automation status.

It never owns:

- child MCP request execution;
- component binary replacement;
- release verification;
- Fleet rendering;
- component rollback;
- Tunnel/Studio recovery journals.

The controller calls internal domain services directly. It never issues HTTP to Studio itself and never accepts arbitrary commands.

## Durable state

Live automation policy state is stored at:

```text
runtime/studio/data/automation/state.json
```

Schema v1 is bounded to 256 KiB and contains only:

- schema version;
- last persisted wall-clock timestamp;
- per-domain next-due time;
- failure/backoff/circuit metadata;
- current-process prepared references;
- deferral/result codes;
- operator reset/resolution references;
- loaded policy/config fingerprint.

It contains no secrets, raw environment, paths outside approved component identifiers, release URLs, MCP arguments/results, or arbitrary log text.

The state file is:

- below the trusted runtime root;
- a regular non-symlink file;
- atomically replaced through sibling temp + file fsync + rename + parent-directory fsync;
- owner-readable/writable only where the platform supports permissions;
- rejected on unknown future schema or over-size content.

Corrupt/future state does **not** silently reset to defaults. Studio remains available for read/manual recovery, while destructive automation is blocked with an explicit state error.

## RuntimeOperationCoordinator

The existing coordinator remains the in-process mutation mutex.

M8 adds a read-only snapshot for policy/UI diagnostics, but does not turn the coordinator into a persistent lock service or waiting queue.

A busy coordinator produces a typed automation deferral. The controller uses bounded backoff and never busy-spins.

## Shared audit admission

The manual API's local operation admission/audit logic is extracted into an internal service used by both HTTP handlers and AutomationController.

A discretionary automatic mutation cannot begin when admission evidence is unavailable. After a physical side effect has begun, audit failure never prevents required completion, rollback, stop, or recovery.

## Startup and shutdown

Startup order:

1. load/validate config;
2. initialize history and component recovery;
3. load automation state;
4. construct runtime owners;
5. finalize existing Studio/Tunnel recovery;
6. perform startup reconciliation check;
7. establish Gateway safety/capability status;
8. bind API and mark Studio ready;
9. start AutomationController after configured startup grace.

Shutdown cancels the controller before component shutdown. No new mutation begins after cancellation. In-progress owner recovery remains the domain owner's responsibility.

## Alternatives considered

### Use SQLite as the job/scheduler database

Rejected. It violates the M6 authority boundary and risks historical corruption authorizing live mutation.

### Call REST endpoints from a timer

Rejected. It duplicates browser/API authority and does not provide internal ownership, restart-safe policy, or activation-time revalidation.

### One durable queue of intended updates/reconciliations

Rejected. M8 persists safety/policy state, not replayable mutation jobs.

## Consequences

- background work has one explicit owner;
- manual and automatic mutation share admission semantics;
- automation survives restart conservatively without replay;
- state corruption disables unattended mutation rather than erasing evidence;
- M8.1 must provide deterministic state-store and controller tests before higher packages enable automation.
