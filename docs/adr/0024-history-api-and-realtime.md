# ADR 0024 — History API and Realtime

## Status

Accepted 2026-09-20.

## Context

History needs browser access without exposing private payloads, unbounded
queries, or allowing stale history to control live component actions.

## Decision

M6 adds read-only, additive history routes with closed sanitized DTOs, fixed
filters, deterministic cursor epochs, and finite page sizes. Commit sequence
is the cursor order. Live state and history remain separate responses.

The writer emits a compact invalidation only after commit. Clients subscribe
before taking their initial watermark; a missed notice causes bounded refetch,
not an assumed lost history row. Browser integers use decimal strings.

## Alternatives considered

- Client-side event history as authority: rejected because reconnect gaps and
  client state are not durable.
- Arbitrary JSON filtering: rejected because it permits unbounded query and
  private-field exposure.
- History controls in live action flows: rejected because historical identity
  cannot authorize current runtime action.

## Consequences

- M6.7 implements routes, reconnect behavior, UI coverage markers, and privacy
  fixtures.
- Every detail link can explicitly report a retention boundary.
- Existing live EventHub messages remain unchanged and are not audit input.
