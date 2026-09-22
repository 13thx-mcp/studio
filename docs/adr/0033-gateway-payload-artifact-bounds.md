# ADR 0033 — Gateway payload and ephemeral artifact bounds

- **Status:** Accepted for M7.5 implementation
- **Date:** 2026-09-22

## Decision

Gateway rejects request arguments above 1 MiB before admission. Child responses are
bounded to 2 MiB total: text preview 64 KiB, structured JSON 1 MiB and binary/base64
1 MiB. A response exceeding any bound is replaced with size metadata, bounded preview
where safe, and narrowing guidance; full content is never forwarded by default.

Spill artifacts are enabled only under `runtime/gateway/artifacts`. Each uses a random
opaque ID plus SHA-256, atomic create, 15 minute TTL, 8 MiB per item and 64 MiB total.
Readback is range-bounded. Startup and each write clean expired items before capacity
is checked. Artifact bytes and raw tool payloads never enter M6 by default.

## Consequences

Budget rejection happens before child dispatch. Guarding a response does not change a
child mutation outcome. Artifact-store failure returns bounded metadata and must not
retry, roll back or replay a child call.
