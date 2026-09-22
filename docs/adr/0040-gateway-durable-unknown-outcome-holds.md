# ADR 0040 — Gateway Durable Unknown-Outcome Safety Holds and Classed Telemetry

- **Status:** Accepted for M8.6 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-04, P-05, unknown side-effect outcome durability, no replay

## Context

M7 correctly returns a structured non-retryable `Unknown` when a dispatched child call may have produced a side effect but the terminal result cannot be proven.

The existing Gateway request registry and telemetry store are bounded in-memory state. Request telemetry also omits the server-owned ToolClass. After Gateway restart, Studio cannot safely infer whether a historical unknown was a read or mutation from current policy.

Unattended M8 mutation therefore needs a small restart-safe veto owned by Gateway.

## Decision

Gateway records ToolClass in sanitized request telemetry at the moment the request is admitted.

ToolClass remains server-owned:

```text
read
mutation
long-running
control
```

No client-supplied class is accepted.

## Unsafe unknown classes

A terminal `Unknown` creates a safety hold for:

- mutation;
- long-running;
- control.

A read-class unknown does not create an automation mutation hold.

## Durable store

Gateway owns:

```text
runtime/gateway/state/safety-holds.json
```

Schema v1 is capped at 256 KiB and 256 retained records.

Each record contains only:

```text
hold_id
gateway_instance_id
request_id
child
tool
class
observed_at_ms
catalog_generation
profile_generation
state = open | resolved
resolution?
resolved_at_ms?
```

No arguments, results, environment, credentials, artifact bytes or filesystem paths are representable.

Store durability uses confined non-symlink directories, sibling-temp write, file fsync, atomic rename and parent fsync.

## Ordering

For an unsafe request whose terminal outcome becomes Unknown:

1. determine class from the already-admitted policy snapshot;
2. construct the sanitized hold;
3. durably persist the open hold;
4. make the hold visible through Gateway safety status;
5. emit sanitized telemetry/history projection;
6. return the structured non-retryable Unknown response.

If hold persistence cannot be proven, Gateway still returns an Unknown/non-retryable result but advertises `automation_safety_available=false`. Studio automatic mutation fails closed until the store is repaired.

Gateway never retries/replays the child call to resolve persistence failure.

## Resolution

Resolution values:

```text
effect-observed
no-effect-observed
abandoned-no-retry
```

Resolution is an explicit operator assertion. It:

- updates the Gateway-owned hold;
- records sanitized telemetry/audit projection;
- removes the hold from the open-veto set;
- never invokes the original child/tool;
- never synthesizes the historical child result.

## Retention

Open holds are never TTL-expired automatically.

Resolved holds may be compacted after successful M6 projection under bounded retention. Compaction can never remove an open hold.

If the store cannot safely compact below limits, automation safety becomes unavailable rather than deleting open evidence.

## Gateway update/rollback

The hold store is outside Gateway release binaries/config surfaces. Gateway update and rollback must preserve and validate it before advertising M8 automation capability.

## Alternatives considered

### Use M6 SQLite rows as the live hold

Rejected. History is not live authority.

### Infer class from current tool policy after restart

Rejected because policy may have changed.

### Persist complete request payload for operator recovery

Rejected for privacy/capability reasons and because it would enable replay.

## Consequences

- M8 gains restart-safe no-replay veto semantics;
- Gateway becomes owner of a small durable safety state domain;
- automation can distinguish old Gateway/no safety state from qualified M8 safety;
- hold resolution remains an operator judgment, not reconstruction of lost truth.
