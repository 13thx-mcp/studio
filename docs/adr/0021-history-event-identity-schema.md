# ADR 0021 — History Event Identity and Schema

## Status

Accepted 2026-09-20.

## Context

Historical rows must be deterministic under concurrent owner delivery,
restarts, delayed messages, and retention. Timestamps and raw serialized
runtime objects are neither stable identities nor safe public payloads.

## Decision

M6 uses opaque UUID identities and SQLite commit sequence for pagination.
Every event has a unique source stream and monotonic source ordinal; a
conflicting duplicate is an integrity failure. Time is observation metadata,
not ordering authority. Payloads are closed, versioned, typed DTOs with strict
size limits and a digest. Browser counters and sequences are decimal strings.

The complete v1 schema is immutable once applied. Event-child foreign keys are
physical; retained projection provenance keys are logical references so a
response can state a retention boundary instead of inventing detail.

## Alternatives considered

- Timestamp ordering: rejected because clocks can jump and concurrent sources
  have no global causal order.
- `INSERT OR IGNORE` duplicates: rejected because it hides conflicting source
  bytes.
- Persisting raw structs or metadata maps: rejected because paths, secrets,
  and future fields could leak into history.

## Consequences

- Reducers must reject stale transitions and preserve terminal precedence.
- Schema additions require a new checksummed migration and upgrade fixture.
- M6.2 validates typed payloads before enqueue; M6.7 exposes only sanitized
  DTOs and bounded deterministic cursors.
