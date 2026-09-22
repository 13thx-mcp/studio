# ADR 0034 — Gateway profiles, resources and progress boundary

- **Status:** Accepted for M7.6 implementation
- **Date:** 2026-09-22

## Decision

Profiles are operator-owned policy: `inspect`, `develop`, `release`, `ops`, and
`hardware`. Tool class and membership come only from trusted Gateway policy. Unknown
class, child, tool or profile reference fails closed. A profile may reduce but never
expand a child launch allowlist. Profile change increments `profile_generation`,
atomically rebuilds routes, emits `tools/list_changed`, and records sanitized audit
metadata. Normal coding profiles exclude control-plane mutations.

Gateway aggregates resources with child-qualified URIs. Same unqualified resource URI
from multiple children is a conflict, never a winner chosen by order. Progress is
forwarded with Gateway-generated token mapping and is informational only: it cannot
mark a request terminal or successful. Workspace aliases are optional convenience
metadata and must resolve to a declared root before the target MCP re-applies its own
confinement.
