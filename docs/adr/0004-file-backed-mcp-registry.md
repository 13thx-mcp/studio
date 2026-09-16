# ADR 0004 — File-Backed MCP Registry and Path Confinement

- Status: Accepted
- Date: 2026-09-16
- Milestone: 4 — Registry, Configuration & Auto-Discovery

## Context

Milestone 4 requires MCP registrations and editable launch configuration to survive Studio restart. The roadmap later assigns SQLite, runtime history, metrics, and audit persistence to Milestone 5, so using SQLite now would couple M4 to work explicitly outside its scope.

At the same time, editable launch configuration increases command-execution risk. Studio must preserve the existing rule that the browser cannot turn the control plane into a generic process launcher.

## Decision

### Versioned local TOML store

M4 persists registry state in a small versioned TOML file, defaulting to `data/registry.toml`.

The document contains:

- explicit `schema_version`;
- configured MCP-root identity;
- a deterministic map of registered MCPs;
- structured executable path, working directory, and argument list;
- server-side environment values only where inherited from existing configuration.

Schema version 1 has no migration transform. Unsupported versions fail startup explicitly rather than being guessed or rewritten.

### Atomic replacement

Registry mutations are copy-on-write:

1. clone current logical state;
2. apply and validate the requested mutation;
3. serialize the complete candidate deterministically;
4. write a temporary sibling file;
5. sync the temporary file;
6. atomically rename it over the registry file;
7. only then publish the candidate as in-memory state.

A malformed pre-existing registry is never overwritten automatically.

### One-time bootstrap

When no persistent registry exists, Studio converts the existing static `[mcp.*]` configuration into schema-v1 records, validates it against the configured MCP root, and persists it once.

An existing registry always wins over legacy bootstrap input. This prevents duplicate entries on every restart.

### Project-relative launch policy

Persistent records use a project path relative to the configured MCP root. Executable and working-directory fields are paths relative to that registered project.

The browser never submits a raw command string or shell program. Arguments are an explicit array.

Studio rejects absolute paths, parent traversal, platform prefix/root components, and paths escaping the registered project.

### Symlink policy

M4 rejects symlink components in registered project paths, working directories, and executable paths. This avoids using mutable symlinks as an authorization boundary and makes path policy deterministic.

Validation is repeated immediately before every spawn.

### Secret exposure

Registry API DTOs and realtime events omit stored environment values. M4 does not provide a generic environment-variable editor and does not round-trip secret values to the browser.

## Consequences

### Positive

- Persistence is small, inspectable, and scoped to M4.
- Registry writes cannot partially truncate the previous valid file.
- Browser configuration remains structured rather than shell-like.
- The executable authority is confined to the registered project under the configured MCP root.
- Existing static configuration can transition without manual re-entry.
- SQLite remains cleanly deferred to M5.

### Negative

- TOML is not appropriate for high-volume historical data or concurrent multi-writer access.
- A local actor with filesystem write authority can still tamper with the registry file or race ordinary files between validation and spawn; Studio treats local filesystem integrity as part of the host trust boundary and revalidates at activation time.
- Rejecting symlinks is stricter than some development layouts and may require using real project directories.
- Legacy raw environment values, if configured, remain server-side persisted values until a later secret-reference migration is designed.

## Rejected alternatives

### SQLite in M4

Rejected because persistence of history/metrics/audit data and database migration policy belong to Milestone 5.

### Rewrite the original `studio.toml` on every registry mutation

Rejected because operational configuration and user-managed static configuration have different ownership/lifecycle semantics, and rewriting the source config would make recovery and migration ambiguous.

### Accept arbitrary executable paths from the browser

Rejected because it would allow Studio to become a generic host process launcher.

### Allow symlinks when canonical targets remain in root

Rejected for M4 because mutable symlinks complicate authorization and TOCTOU analysis. A future explicit policy may relax this if required.

## Deferred decisions

- SQLite migration/import of schema-v1 registry data in M5.
- Configuration revision history and rollback.
- File-integrity/authentication mechanisms for local registry tampering.
- Descriptor-based executable/openat confinement if later threat review requires stronger TOCTOU guarantees.
- General secret-reference schema for MCP environment variables.
