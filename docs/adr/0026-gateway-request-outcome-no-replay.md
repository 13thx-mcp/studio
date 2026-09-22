# ADR 0026 — Gateway Request Outcome and No-Replay Semantics

- **Status:** Accepted for M7.0
- **Date:** 2026-09-21
- **Decision scope:** M7 Gateway request coordination
- **Evidence:** `docs/plans/m7-gateway-coordination/RMCP-PROTOCOL-SPIKE.md`

## Context

The pre-M7 Gateway dispatches a child tool call and wraps the returned future with an external `tokio::time::timeout`. Permanent tests against the locked `rmcp 3.4.0` dependency prove that expiration of this outer timeout stops waiting but does not send `notifications/cancelled` to the child. The child may continue and commit a side effect.

For mutating tools, returning an ordinary timeout/failure and encouraging retry could therefore duplicate a side effect.

rmcp exposes a cancellable `RequestHandle`, child request ID, progress token and cooperative cancellation notification. Its own request timeout can send cancellation, but cancellation is still not rollback evidence.

## Decision

Gateway owns every admitted child call through an explicit in-memory request record and a cancellable child request handle.

The minimum lifecycle is:

```text
RECEIVED
  -> ADMITTED
  -> QUEUED
  -> DISPATCHED
  -> COMPLETED
```

Pre-dispatch terminal states include:

```text
REJECTED_POLICY
REJECTED_CAPACITY
CANCELLED_BEFORE_DISPATCH
DEADLINE_EXPIRED_BEFORE_DISPATCH
```

Post-dispatch terminal/exceptional classifications include:

```text
COMPLETED
CHILD_ERROR
RESPONSE_REJECTED
OUTCOME_PENDING
OUTCOME_UNKNOWN
```

### Dispatch boundary

Gateway records `DISPATCHED` in its live request registry before handing the request to rmcp.

Before that boundary, cancellation/deadline expiry guarantees that no child tool call occurred.

After that boundary:

- cancellation is cooperative only;
- caller timeout/disconnect does not prove the child stopped;
- Gateway may request child cancellation but does not claim rollback;
- the child call remains owned by Gateway for bounded terminal observation;
- mutation replay is forbidden.

### Pending versus unknown

`OUTCOME_PENDING` means Gateway still owns enough live observation state to learn a terminal child result.

`OUTCOME_UNKNOWN` means terminal evidence has been lost after dispatch, for example because the child transport/process failed and no trustworthy operation result is available.

Neither state is equivalent to a normal child failure.

For a mutation with unknown outcome, retry guidance requires inspection/reconciliation of domain state before another mutation.

### Identity

Gateway generates opaque operational `request_id` and `correlation_id` values.

MCP transport request IDs and progress tokens may be linked for observation, but no ID is authorization, idempotency or replay authority.

### Protocol lifecycle

The initial M7 request coordinator preserves the current child connection lifecycle and its rmcp 3.4.0 default protocol behavior (2025-11-25).

Switching children to the 2026-07-28 Discover lifecycle is a separate compatibility migration requiring explicit qualification.

## Consequences

- The production child dispatch path cannot remain `timeout(peer.call_tool(...))`.
- M7.1 requires an owned request task/handle whose lifetime is independent from the upstream wait.
- Gateway must bound the number and lifetime of post-dispatch observation obligations.
- Drain counts dispatched pending/unknown work according to tool class and cannot silently kill a mutation to meet a deadline.
- M6 receives sanitized historical projections only; it is not the live request registry.
- Reads may receive different operator retry guidance later, but no mutation auto-replay exception is permitted by this ADR.

## Rejected alternatives

### Treat timeout as child failure

Rejected because permanent rmcp tests prove the child can continue to completion after the current outer timeout.

### Automatically retry mutations on timeout/disconnect

Rejected because duplicate side effects are possible and there is no distributed transaction or universal idempotency key across typed child MCPs.

### Persist the live request queue in SQLite

Rejected because M6 history is evidence, not request execution authority; restart replay would create unsafe side-effect semantics.

### Use caller/session identity as replay key

Rejected because session identity is convenience/correlation, not authorization or domain idempotency.
