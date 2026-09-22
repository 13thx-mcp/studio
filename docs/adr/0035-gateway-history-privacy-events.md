# ADR 0035 — Gateway sanitized history events

- **Status:** Accepted for M7.7 implementation
- **Date:** 2026-09-22

## Decision

Gateway emits typed sanitized observations to Studio/M6: request ID, lifecycle/outcome,
queue and execution durations, child/tool policy identity, catalog/profile generation,
response byte count/guard action, and child restart/circuit events. Event payloads omit
tool arguments, tool results, artifact bytes, credentials, absolute workspace paths and
client-supplied identity claims.

M6 remains historical evidence only. Ingestion is best-effort after live state changes;
ingestion failure cannot authorize, retry, replay, roll back or otherwise alter a child
request outcome. Persistence uses bounded retention and restart-safe aggregates only.
