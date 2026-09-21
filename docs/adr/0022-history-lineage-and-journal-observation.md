# ADR 0022 — History Lineage and Journal Observation

## Status

Accepted 2026-09-20.

## Context

M5 update journals are the safety authority for crash recovery. M6 needs
historical lineage without retaining journals merely for reporting or allowing
history to resume volatile updates.

## Decision

History observes only validated journal revisions and owner-verified receipts.
Each source `(domain, transaction_id, revision)` is deduplicated with a payload
digest; duplicate bytes are idempotent and conflicting bytes are integrity
failures. Tunnel terminal evidence is captured before ordinary journal cleanup.
Fleet remains sole authority for self-update terminal status.

Current installed identity, staged artifacts, and rollback verification become
historical lineage observations. They do not authorize an apply, recovery, or
choice of predecessor. Missing receipt windows remain partial coverage.

## Alternatives considered

- Retain journals until SQLite acknowledges: rejected because history failure
  must not alter M5 journal lifecycle.
- Resume update work from SQLite: rejected because it creates volatile-update
  authority outside existing owners.
- Treat same version as same artifact: rejected because same-version bytes can
  differ.

## Consequences

- M6.4 integrates receipt hooks at existing verified terminal branches.
- Historical self-update observation is finite and read-only.
- Journal replay conflicts and revision gaps become visible coverage findings,
  not silent duplicate rows or fabricated phases.
