# ADR 0028 — Bounded Gateway Scheduler and Concurrency Contract

- **Status:** Accepted for M7.1 implementation
- **Date:** 2026-09-21
- **Decision scope:** request admission, queueing, dispatch ordering, concurrency bounds and pre/post-dispatch races

## Context

M7 turns Gateway from a direct router into a bounded coordination boundary. The scheduler must prevent unbounded semaphore waiters, make cancellation semantics race-safe, avoid head-of-line blocking between unrelated children/tools, and keep mutation retry semantics consistent with ADR 0026.

The control/drain path from ADR 0027 must remain reachable even while normal request capacity is saturated.

## Decision

Gateway owns one in-memory coordination scheduler for normal MCP tool requests. Admission state, queue membership, concurrency reservations and the transition to `DISPATCHED` are changed only through that coordinator.

The first M7.1 implementation baseline is:

| Limit | Default | Meaning |
|---|---:|---|
| global active | 16 | maximum dispatched child calls with terminal-observation obligation |
| global queued | 64 | maximum requests in `QUEUED` |
| queue wait | 30,000 ms | maximum scheduler wait, further bounded by any earlier caller/request deadline |
| per-child active | 4 | maximum dispatched calls for one child |
| per-tool read | 4 | default active limit for one read tool |
| per-tool mutation | 1 | conservative single-flight default per mutation tool |
| per-tool long-running | 1 | default active limit for one long-running tool |
| per-tool control | 1 | default active limit for one normal MCP control-class tool |

Trusted configuration may lower or explicitly raise child/tool limits, but no effective limit may exceed the global active bound. Any change to these implementation defaults before M7 closure must be recorded as an explicit M7 contract change with qualification evidence.

The private lifecycle control socket from ADR 0027 is not a normal scheduled tool call and does not consume these normal request permits. Its own connection/request bounds remain separate and finite.

## Request lifecycle

Normal admitted requests follow:

```text
RECEIVED
  -> ADMITTED
  -> QUEUED
  -> DISPATCHED
  -> COMPLETED
```

A request that can dispatch immediately still passes through the logical `QUEUED` state; the residence time may be zero.

Pre-dispatch terminal states include:

```text
REJECTED_POLICY
REJECTED_CAPACITY
CANCELLED_BEFORE_DISPATCH
DEADLINE_EXPIRED_BEFORE_DISPATCH
```

Post-dispatch outcome semantics are governed by ADR 0026:

```text
DISPATCHED
  -> terminal child result
  -> OUTCOME_PENDING
  -> OUTCOME_UNKNOWN
```

Timeout, cancellation or disconnect after `DISPATCHED` never proves rollback and never authorizes automatic mutation replay.

## Admission and capacity

Admission performs policy/classification validation before queue insertion.

If the queue already contains 64 requests and the request cannot be dispatched without queueing, Gateway returns deterministic `REJECTED_CAPACITY`; it does not create an unbounded waiter.

The effective queue deadline is:

```text
min(admitted_at + queue_wait_ms, caller/request deadline when present)
```

Expiry before dispatch removes the request from the queue and guarantees zero child invocation.

## Dispatch reservation

For a queued request to be eligible, all of the following must be true at the same coordination decision:

1. global active capacity is available;
2. the target child has capacity;
3. the target tool has capacity;
4. current policy/profile/catalog generation still permits dispatch;
5. the request is not cancelled;
6. its queue deadline has not expired;
7. Gateway drain state permits that request class.

The scheduler atomically reserves global/child/tool capacity and records `DISPATCHED` before handing the call to rmcp.

This atomic boundary defines the cancellation race:

- cancellation/deadline observed before that transition => child call count must remain zero;
- cancellation/deadline/disconnect observed after that transition => post-dispatch pending/unknown semantics.

## Fairness

Each admitted request receives a monotonically increasing scheduler sequence number.

Whenever capacity changes or a new request arrives, the scheduler scans queued requests in sequence order and dispatches the **oldest eligible** request. It repeats while global capacity remains and at least one queued request is eligible.

A request blocked by a saturated child/tool does not head-of-line block unrelated work. Later requests may bypass it only while it is ineligible.

When an older request becomes eligible, no later eligible request may be selected ahead of it for the next available compatible dispatch opportunity.

This is work-conserving while preserving FIFO priority among requests that can actually run.

## Starvation handling

The scheduler does not use unbounded priority promotion or retry loops.

Starvation is bounded by two rules:

1. once a request is eligible, sequence ordering prevents later eligible requests from overtaking it;
2. if the required capacity never becomes available, the finite queue deadline terminates the wait with `DEADLINE_EXPIRED_BEFORE_DISPATCH`.

Metrics must record queue wait and deadline expiry so M7 qualification can detect pathological policy/limit choices.

## Tool-limit resolution

Tool class is trusted server configuration, never caller supplied.

Effective per-tool limit is resolved in this order:

1. explicit trusted `tool_policy.<tool>.concurrency`;
2. otherwise the class default from this ADR.

The final dispatch bound is the minimum of global, child and tool limits.

A configured value of zero is invalid configuration, not a way to create an indefinitely queued disabled tool.

## Owned child-call observation

A dispatched call occupies active capacity until Gateway has a trustworthy terminal result or has classified the outcome as no longer observable according to ADR 0026.

Dropping the upstream response future does not release scheduler capacity early.

Gateway must not create detached, unbounded terminal-observation tasks; the number of such obligations is bounded by `global_active`.

## Drain interaction

When drain starts, scheduler admission/dispatch applies ADR 0027 and the M7 drain policy:

- new mutations are rejected immediately;
- queued mutations that have not dispatched are cancelled-before-dispatch for destructive drain modes;
- safe reads may continue only when the configured drain policy explicitly allows them;
- dispatched mutation obligations remain active until trustworthy terminal classification;
- drain timeout fails closed and does not kill mutation work merely to reach `DRAINED`.

Control-socket requests remain serviceable independently of normal scheduler saturation.

## Verification requirements

M7.1 permanent tests must prove at minimum:

- global active never exceeds 16 under default configuration;
- queue depth never exceeds 64;
- queue expiry invokes the child zero times;
- per-child default limit 4 is enforced;
- mutation per-tool default is single-flight;
- a saturated child does not block eligible work for another child;
- oldest-eligible ordering prevents starvation once capacity is available;
- cancellation racing before dispatch invokes the child zero times;
- cancellation/timeout after dispatch preserves ADR 0026 pending/unknown semantics;
- upstream disconnect cannot release active capacity while child observation remains outstanding.

These map to R01-R10 and Q01-Q05 in the M7 verification matrix.

## Consequences

- M7.1 needs one explicit scheduler/registry ownership boundary rather than one spawned waiter per incoming call.
- Queue and active-state memory are bounded by configuration.
- Mutation throughput is conservative by default and can be raised only through trusted policy.
- Saturation of one child/tool does not freeze unrelated work.
- Control/drain coordination cannot deadlock merely because normal request permits are exhausted.

## Rejected alternatives

### FIFO head-only dispatch

Rejected because one saturated child/tool would block unrelated eligible requests.

### One semaphore per limit with spawned waiters

Rejected because waiter/task count can grow independently of active-call bounds and cancellation ordering becomes harder to prove.

### Caller-supplied priority

Rejected because it would allow clients to influence safety/fairness policy and could starve ordinary work.

### Automatic mutation retry after timeout/cancel

Rejected by ADR 0026 because a dispatched side effect may already have occurred.
