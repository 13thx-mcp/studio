# ADR 0019 — SQLite History Store

## Status

Accepted 2026-09-20.

## Context

History must survive Studio restarts while retaining bounded resource use and
not blocking Tokio or competing with another Studio process for a database.

## Decision

M6 uses pinned `rusqlite` with its bundled SQLite engine. One dedicated OS
thread owns the writer connection; commands and receipts use bounded channels.
The store is rooted only at `runtime/studio/data/history`, with a retained
advisory owner lock. SQLite is configured for WAL, FULL synchronous commits,
foreign keys, disabled trusted schema, finite busy timeout, and disabled mmap.

The complete version-one schema is an embedded checksummed migration. Unknown
application IDs, newer schemas, migration checksum mismatches, unsafe links,
or writer contention degrade history and never cause delete/recreate, repair,
or in-memory fallback.

## Alternatives considered

- A Tokio-owned connection or connection mutex: rejected because blocking SQL
  could stall runtime owners.
- A multi-writer pool: rejected because serial history ordering and migration
  ownership become ambiguous.
- System SQLite: rejected because deployed engine features and security fixes
  would vary by host.

## Consequences

- Database health is distinct from component health and cannot veto safety
  compensation.
- Opening the store has a bounded startup budget; failure yields an explicit
  degraded handle.
- M6.1 must still qualify the resolved bundled engine, backup path, fault
  behavior, and native targets before its package can close.
