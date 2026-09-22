# ADR 0038 — M8 Scheduler, UTC Maintenance Windows, Retry and Circuit Semantics

- **Status:** Accepted for M8.1 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-14, scheduler cadence, missed ticks, clock anomalies, retry/circuit accounting

## Context

Unattended operation needs periodic checks without provider hammering, catch-up bursts, uncontrolled retry loops, or ambiguous daylight-saving behavior.

Tokio monotonic timers are appropriate while a process is alive, but restart scheduling requires bounded wall-clock metadata.

## Decision

M8 uses in-process Tokio timers and persists only conservative due/backoff/circuit timestamps in AutomationState.

Schedule classes are:

```text
health
update_check
reconciliation_check
cleanup
```

At most one evaluation of each class may run concurrently. Missed ticks collapse to one due evaluation. Restart after a long sleep never replays every missed interval.

## Defaults and bounds

Initial defaults:

```text
startup grace                 30 s
health interval               15 s
update check interval          6 h
reconciliation check interval  5 min
cleanup interval               1 h
retry initial                 30 s
retry multiplier               2
retry maximum                 30 min
circuit threshold              5 failures
circuit cooldown               6 h
```

Implementation must validate package-defined lower/upper bounds. No configured cadence may create sub-second polling or an unbounded duration.

## Failure versus deferral

Failure increments a failure/circuit counter only when the controller attempted a domain operation and that operation failed.

Examples:

- provider/staging failure;
- health verification failure;
- reconciliation apply that began and failed/rolled back;
- safety capability that was previously qualified and becomes malformed;
- automation-state persistence failure.

Deferral does not increment failure count:

- maintenance window closed;
- runtime coordinator busy;
- unsafe Gateway hold open;
- source dirty/conflicted;
- component transitional;
- required capability absent on an older compatible component;
- policy disabled.

Deferrals are rate-limited in logs/history.

## Circuit

Per domain/component where applicable:

```text
closed
  └─ failures >= threshold → open
open
  └─ cooldown expires → half-open
half-open
  ├─ one qualified success → closed
  └─ failure → open
```

Only one half-open probe is admitted. Operator reset closes policy state only; it never performs the blocked action.

## Maintenance windows

M8 initial windows are UTC-only:

- explicit weekday list;
- start `HH:MM` UTC;
- duration 1–1440 minutes;
- end exclusive.

A destructive action must be admitted inside the window. If physical mutation has already begun, it must complete or rollback even after window end.

Observation, safe stop, rollback, recovery, diagnostics and hold resolution are not blocked by maintenance windows.

## Clock anomaly rule

AutomationState records the last persisted wall-clock timestamp.

If current wall time is more than five minutes earlier than the persisted timestamp, destructive automation enters `clock_anomaly` hold.

Observation may continue. The hold clears only when:

- wall time reaches the previously persisted timestamp minus the tolerance, or
- an explicit audited operator reset acknowledges the clock correction.

No wall-clock jump can manufacture multiple missed jobs.

## Alternatives considered

### Local/IANA timezone windows in M8

Deferred. DST fold/gap rules add policy ambiguity not needed for the initial single-host safe automation release.

### Execute every missed scheduled run

Rejected because it causes bursts after restart/sleep.

### Treat every deferral as failure

Rejected because normal busy/window conditions would open circuits unnecessarily.

## Consequences

- scheduling is deterministic and fake-clock testable;
- UTC is less ergonomic but avoids DST ambiguity;
- circuits bound failure loops across Studio restart;
- M8.7 can expose precise deferral/circuit reasons without implying a failed runtime.
