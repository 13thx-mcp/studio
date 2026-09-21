# ADR 0018 — History Authority Boundaries

## Status

Accepted 2026-09-20.

## Context

M6 needs durable operational history without creating a second authority for
runtime state, registry/configuration, releases, or existing recovery
journals. Using a history record to drive an action would make stale process,
version, or file observations unsafe control inputs.

## Decision

SQLite owns only immutable historical facts, audit delivery state, coverage,
and read projections. Live owners remain authoritative for current process
state. Registry TOML, Studio configuration, Fleet manifests, installed release
material, and M5 recovery journals retain their existing authority and format.

History producers submit typed observations at owner boundaries. The history
store never calls an owner, changes a source file, supplies a PID for control,
or resumes an update. Missing receipts are represented as incomplete coverage,
not inferred success.

## Alternatives considered

- Migrate registry/configuration to SQLite: rejected because it changes their
  authority and recovery contract.
- Reconstruct live state from stored events: rejected because it can act on
  stale identities after restart.
- Use EventHub/logs as the audit source: rejected because both are lossy and
  omit owner-specific completion evidence.

## Consequences

- Historical queries are additive and read-only.
- Cross-store actions retain explicit uncertainty windows.
- M6 cannot repair, roll back, restart, or otherwise control a component from
  a historical row.
- Requirements R01–R03 are enforced by storage boundaries and M5 regression
  coverage; later ADRs define store, event, and API details.
