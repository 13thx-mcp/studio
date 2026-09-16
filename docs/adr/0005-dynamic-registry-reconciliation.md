# ADR 0005 — Dynamic Registry Reconciliation and Discovery Approval

- Status: Accepted
- Date: 2026-09-16
- Milestone: 4 — Registry, Configuration & Auto-Discovery

## Context

The Milestone 1 supervisor was constructed from an immutable registry snapshot at Studio startup. Milestone 4 must add and remove registrations at runtime without restarting Studio, while preserving the rule that Studio only signals processes it owns.

Discovery also introduces untrusted project metadata. A scan must not automatically grant execution authority to newly found projects.

## Decision

### Shared live registry

The supervisor and API share one registry object. Lifecycle operations resolve the current registration/configuration when the operation begins rather than relying on a copied startup-only inventory.

Runtime process state remains owned by the supervisor and is separate from registry configuration.

### Lazy runtime state

Runtime state for a registered MCP is created lazily when status/lifecycle operations first need it. A newly registered server therefore becomes manageable immediately without a supervisor rebuild.

Unregistering a stopped/failed MCP removes its inactive runtime state. Active runtime state is never silently orphaned.

### Mutation conflict rules

For `starting`, `running`, or `stopping` MCPs:

- edit is rejected;
- disable is rejected;
- unregister is rejected.

The operator must stop the MCP first. Enable is allowed only for an already registered stopped/failed MCP.

Disabled MCPs remain visible and configured, but start/restart activation is rejected.

### Metadata-only discovery

Discovery reads filesystem metadata and supported manifest formats only. It does not invoke shells, package managers, compilers, interpreters, install scripts, project binaries, or imports.

M4 scans direct children of the configured MCP root and explicitly excludes Studio, tunnel-client, gateway infrastructure, hidden/build/temp directories, and other known non-target containers.

A malformed/unreadable candidate is isolated; it cannot abort discovery of unrelated projects.

### Explicit approval boundary

Discovery output is a preview, not a registration.

Registration is a separate same-origin mutation. The server rescans and resolves the candidate itself; it does not trust a browser-supplied project path. Initial executable selection must be one of the metadata-derived project-local candidates returned by discovery.

Registration persists configuration and makes the server visible in stopped state. It never starts the process.

### Realtime invalidation

Registry/discovery changes publish typed invalidation events. Clients refetch those resources over REST. Operational WebSocket snapshots remain focused on runtime/tunnel state and do not broadcast the complete registry.

## Consequences

### Positive

- Dynamic registration does not require restarting or rebuilding the supervisor.
- Persistent configuration and transient runtime state stay conceptually separate.
- Running processes cannot be silently detached from their registration.
- Discovery never executes untrusted project code.
- The browser cannot use registration to submit arbitrary filesystem paths.
- Realtime messages stay small and avoid exposing configuration unnecessarily.

### Negative

- Registry/runtime coordination still requires explicit conflict checks in the API/service boundary.
- Runtime state for a removed stopped server is intentionally discarded rather than retained as history; history belongs to M5.
- Direct-child discovery is intentionally narrower than arbitrary recursive project discovery.

## Rejected alternatives

### Reconstruct the entire supervisor after every registry mutation

Rejected because replacing supervisor state risks losing ownership/runtime state and creates race-prone handoff behavior.

### Automatically stop then disable/unregister

Rejected for M4 because it combines lifecycle and configuration mutations implicitly. Explicit stop-first semantics are easier to reason about and audit.

### Auto-register or auto-start discovered projects

Rejected because discovery operates on untrusted local project metadata and must not grant execution authority without an explicit operator action.

### Put the complete registry in every WebSocket snapshot

Rejected because a typed invalidation/refetch model is smaller and creates less risk of accidentally broadcasting sensitive configuration.

## Deferred decisions

- Coordinated stop+disable/unregister operations.
- Recursive/custom discovery adapters.
- Persisted runtime state/history.
- Automatic restart/backoff and health checks.
- Remote multi-user authorization for registry mutations.
