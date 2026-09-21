# ADR 0023 — Observed Metrics and Retention

## Status

Accepted 2026-09-20.

## Context

Operational counters become misleading when crashes, degraded storage,
bootstrap, or retention leave unknown intervals. History growth also needs a
fixed resource envelope.

## Decision

Metrics derive exactly once from canonical persisted owner facts, keyed by
definition version and aggregation watermark. They expose coverage quality and
never fill missing intervals with zero. MCP and Tunnel crash definitions retain
their owner-specific semantics.

Retention is bounded and runs only after aggregation reaches its deletion
boundary. It preserves compact marked lineage/config anchors, advances a
retention epoch atomically, and reports expired evidence as a boundary. Audit
retention is not silently cascaded with operational event deletion.

## Alternatives considered

- Snapshot live counters as historical totals: rejected because reset and gap
  semantics are unknowable.
- Zero-fill coverage gaps: rejected because it invents successful observation.
- Unlimited lineage pinning: rejected because storage growth becomes unbounded.

## Consequences

- M6.6 owns aggregation, quota, and compaction workers.
- Queries return metric watermark and quality alongside values.
- Metric definition changes require a new version, golden fixtures, and do not
  rewrite prior aggregates.
