# ADR 0025 — History Bootstrap and Operations

## Status

Accepted 2026-09-20.

## Context

M5 has no historical schema. M6 needs a safe first boot, backup/upgrade
operations, and honest failure behavior without inventing prior events or
turning database health into runtime recovery authority.

## Decision

First M6 boot creates an empty store and records only current validated
bootstrap observations at observation time. It never backdates a past action
from a present file or process. History initialization occurs after config/root
resolution and before owner construction; failure supplies a degraded handle.

Backups use a fixed private destination and capture a consistent WAL state.
Upgrade requires a verified backup. Corrupt, unsafe, or newer schemas fail
closed. M6 does not auto-repair, downgrade, recreate, or substitute an
in-memory database. Existing readiness and M5 recovery ordering stay intact.

## Alternatives considered

- Import inferred pre-M6 lifecycle history: rejected because it fabricates
  evidence.
- Treat database open failure as process-start failure: rejected because safe
  runtime recovery must remain available.
- Auto-delete/recreate damaged databases: rejected because it destroys audit
  evidence and hides tampering.

## Consequences

- M6.1 owns path, migration, backup, source-less, and degraded-start tests.
- M6.8 requires real runtime/native qualification before closure.
- M5 publication qualification remains independent of M6 implementation.
