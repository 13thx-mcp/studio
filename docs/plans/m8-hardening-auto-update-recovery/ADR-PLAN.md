# M8 ADR Plan

ADR 0037–0045 are **Accepted for M8 implementation** as of 2026-09-22. Any implementation divergence requires an ADR amendment before the affected package can close.

| ADR | Decision | Primary proof |
|---|---|---|
| 0037 | AutomationController ownership, state store and no durable replay queue | restart/state corruption tests |
| 0038 | Scheduler, UTC maintenance windows, missed-tick/backoff/circuit semantics | fake-clock/timer tests |
| 0039 | Update policy modes, process-scoped prepare authorization and staging retention | restart after prepare / no activation |
| 0040 | Gateway durable unknown-outcome safety holds and classed telemetry | crash/restart + hold resolution |
| 0041 | Gateway M8 control capabilities and targeted child activation | active mutation + child restart isolation |
| 0042 | Component health abstraction and single-owner restart policy | crash-loop/stability/circuit tests |
| 0043 | Automatic Fleet reconciliation retry/circuit and drift boundaries | repeated drift/failure tests |
| 0044 | Recovery cleanup, stale-lock policy and diagnostics sanitization | disk-full/corrupt/secret scan |
| 0045 | Guarded SemVer prerelease release-tag/evidence contract | tag parser/evidence mismatch tests |

## Frozen decision checklist

### ADR 0037

- exact automation state path/schema;
- atomic durability contract;
- behavior if state is corrupt/future version;
- background task ownership/shutdown.

### ADR 0038

- UTC-only M8 windows;
- interval lower/upper bounds;
- restart overdue semantics;
- clock-regression tolerance;
- failure vs deferral accounting.

### ADR 0039

- auto-prepare does not survive restart as activation authority;
- orphan ready staging TTL/disk handling;
- desired-vs-latest target selection;
- dirty-source activation block.

### ADR 0040

- which ToolClass values create holds;
- exact hold schema/caps/retention;
- persistence ordering relative to returning Unknown;
- resolution enum and audit behavior;
- compatibility when hold store unavailable.

### ADR 0041

- additive Gateway control capability negotiation;
- targeted child quiesce/restart request/response;
- active-work rejection/drain semantics;
- child generation/version/health proof;
- sibling isolation.

### ADR 0042

- health states/freshness;
- Studio Supervisor/Tunnel restart defaults;
- explicit-stop/update suppression;
- no duplicate Gateway child restart ownership.

### ADR 0043

- reconciliation scheduler state;
- circuit persistence/reset;
- exact automatic drift classes;
- rollback-failed behavior.

### ADR 0044

- cleanup ownership hierarchy;
- staging budget;
- stale-lock proof;
- diagnostics allowlist and size caps.

### ADR 0045

- accepted prerelease grammar and normalization;
- exact package/changelog/release-evidence version matching;
- existing published tag immutability;
- no fallback to generic tagging when release gate fails.

## Rejected shortcuts

- using SQLite rows as a work queue;
- treating "process running" as universal health;
- inferring unknown historical request class from current policy;
- deleting all old staging on startup;
- retagging/pulling source to solve runtime update conflict;
- using one shell script as the automation engine;
- polling every few seconds until a busy lock clears;
- retrying a mutation after Gateway returns unknown outcome.
