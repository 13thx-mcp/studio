# Milestone 4 Design — Registry, Configuration & Auto-Discovery

- Status: Implementing
- Date: 2026-09-16
- Target: v0.4.0

## Objective

Milestone 4 replaces Studio's compile-time/static MCP inventory with a persistent, explicitly approved registry while preserving the process-ownership, loopback, same-origin, and no-shell guarantees established in Milestones 1–3.

The milestone intentionally does not introduce SQLite. Registry/configuration persistence is file-backed in M4; historical runtime sessions, metrics, event persistence, audit history, and configuration revision history remain M5+ work.

## Observed workspace

The configured MCP root is expected to resolve to the current `mcp-server/` directory. Direct children observed during planning include:

- `blender/` — Rust MCP server (`rust-mcp-blender`)
- `filesystem/` — Rust MCP server (`rust-mcp-filesystem`)
- `git/` — Rust MCP server (`rust-mcp-git`)
- `gateway/` — MCP infrastructure aggregator; excluded from M4 discovery
- `studio/` — Studio itself; excluded
- `tunnel-client/` — tunnel domain; excluded
- `mcp-server/` — infrastructure/container directory; excluded

Discovery is intentionally based on direct children of the configured MCP root in M4. Recursive repository crawling is deferred.

## Architecture

```text
Browser
  │
  ├── MCP lifecycle REST ────────────────┐
  ├── registry/discovery REST            │
  └── WebSocket invalidation/status      │
                                         ▼
                               ┌──────────────────┐
                               │     Registry     │
                               │ shared live state│
                               └──────┬─────┬─────┘
                                      │     │
                         atomic TOML  │     │ validated launch snapshot
                                      │     ▼
                               data/registry.toml
                                            │
                                            ▼
                                      MCP Supervisor

DiscoveryService ── metadata-only scan ── configured MCP root
       │
       └── explicit registration approval ──> Registry
```

The runtime supervisor reads the shared registry at operation time. It does not keep an immutable startup-only copy of MCP configuration.

## Persistent registry format

M4 uses a versioned TOML file, defaulting to:

```text
data/registry.toml
```

Schema version 1 conceptually contains:

```toml
schema_version = 1
mcp_root = ".."

[servers.filesystem]
name = "Filesystem"
enabled = true
runtime = "rust"
project_path = "filesystem"
executable = "target/debug/rust-mcp-filesystem"
working_dir = "."
args = ["--root", ".."]
```

Environment values inherited from legacy static configuration remain server-side and are never included in registry API responses or realtime events. M4 does not add a generic browser environment-variable editor.

### Atomic update algorithm

Every mutation creates a complete candidate registry state, validates it, serializes deterministically, then:

```text
serialize complete state
→ create temp file in the registry directory
→ write all bytes
→ fsync temp file
→ atomic rename over registry file
→ publish candidate as in-memory state
```

If serialization/write/sync/rename fails, the previous registry file and previous in-memory state remain authoritative. A malformed or unsupported existing registry is never silently replaced.

## Startup/bootstrap behavior

### No registry file

1. Resolve and validate configured `registry.mcp_root`.
2. Convert existing `StudioConfig.mcp` entries into schema-v1 records.
3. Validate every bootstrapped path against the MCP root/project boundary.
4. Persist once atomically.
5. Use the resulting registry.

This preserves the current Blender and Filesystem configuration without duplicating entries on future restarts.

### Existing valid registry

Load and validate schema version, configured MCP-root identity, IDs, project paths, launch paths, and duplicate project registrations.

### Existing malformed/unsupported registry

Fail Studio startup with a configuration/persistence error. Do not fall back to defaults and do not overwrite the file.

## Registry identity rules

Registered IDs are stable map keys and are not implicitly changed when display names change.

M4 accepts IDs matching the semantic rule:

```text
first:  a-z or 0-9
rest:   a-z, 0-9, _, or -
```

IDs are unique. Registration also rejects a second registration for the same canonical project path.

## Path confinement and symlink policy

The configured MCP root is canonicalized once when opening the persistent registry.

Persistent M4 records store project-relative paths. Browser-editable executable and working-directory values are project-relative paths; raw shell commands and arbitrary host executable paths are not accepted.

Validation rules:

- project, executable, and working-directory paths must be relative;
- `..`, root, and platform prefix components are rejected;
- canonical project path must remain below the canonical MCP root;
- working directory and executable must remain below the canonical project directory;
- symlink components are rejected for project, working-directory, and executable paths;
- executable must be a regular file at activation/start time;
- working directory must be a directory;
- the same policy is revalidated immediately before spawn.

Rejecting symlink components makes the policy deterministic and avoids treating a mutable symlink as an authorization boundary. A local user able to replace ordinary path components between final validation and OS spawn still represents a residual TOCTOU risk; M4 narrows this window by revalidating on every start. Descriptor-based execution is deferred unless later threat review requires it.

## Discovery model

Discovery scans direct child directories of the canonical MCP root and never runs project code.

Ignored names include:

- hidden directories;
- `studio`;
- `tunnel-client`;
- `gateway`;
- `mcp-server`;
- `target`;
- common generated/temp names.

A failure to read or parse one child does not abort the rest of the scan.

### Rust

Detect `Cargo.toml` and read metadata only. Extract package name, optional `default-run`, and explicit bin target names where present. Infer release/debug executable candidates inside the project. No `cargo` command is run.

### Node

Detect `package.json` and parse JSON only. Extract package name and `bin` entries. Script names may be reported as metadata but scripts are never run by discovery. Only project-local bin paths are launch candidates in M4.

### Python

Detect `pyproject.toml` and parse TOML only. Extract `[project].name` and `[project.scripts]` names. Discovery may report conventional project-local virtual-environment console-script paths when they exist. Python modules are never imported.

## Discovery approval

`DiscoveredProject`, `RegisteredMcp`, and `ProcessStatus` remain distinct models.

A scan returns preview candidates. Registration requires a separate mutating request and rescans before approval so the browser cannot register an arbitrary caller-supplied filesystem path.

Registration:

- chooses a candidate returned by current metadata scan;
- validates/sanitizes the requested stable ID and display name;
- accepts only a discovery-produced executable candidate for initial registration;
- persists the registry atomically;
- creates stopped runtime state lazily/on reconciliation;
- never starts the process.

## Runtime reconciliation

The supervisor owns runtime process state; the registry owns configuration/authorization.

Rules:

- a newly registered MCP becomes immediately addressable by lifecycle API without restarting Studio;
- runtime state is created lazily when first queried/started;
- disabled MCPs remain visible but `start`/`restart` activation is rejected;
- edit, disable, and unregister are rejected while state is `starting`, `running`, or `stopping`;
- unregister of a stopped/failed MCP removes the registration and its inactive runtime entry;
- unregister never deletes source code or arbitrary filesystem content;
- supervisor shutdown enumerates the current registry and stops all Studio-owned active children.

Because the supervisor reads a shared registry object for each lifecycle operation, there is no separate asynchronous "reload registry" phase that can diverge after persistence succeeds.

## API contract

M4 adds the following minimal endpoints:

```text
GET    /api/registry
GET    /api/registry/:id
PUT    /api/registry/:id
DELETE /api/registry/:id
POST   /api/registry/:id/enable
POST   /api/registry/:id/disable

GET    /api/discovery
POST   /api/discovery/scan
POST   /api/discovery/:candidate_id/register
```

Registry/discovery mutations use the same same-origin browser check as lifecycle/tunnel mutations.

No endpoint accepts a shell command, PID, source deletion target, MCP-root replacement, or raw secret values.

## Realtime contract

M4 adds typed invalidation events:

```text
registry_changed
discovery_changed
```

Registry/discovery payloads are fetched over REST after invalidation rather than added to the operational snapshot. This keeps reconnect snapshots bounded and avoids broadcasting configuration details unnecessarily.

## UI

The existing lifecycle/tunnel dashboard remains operational. M4 adds Registry and Discovery panels:

- registry list: ID, display name, enabled state, runtime, project path, executable, working directory, runtime state;
- safe editor: display name, project-relative executable, project-relative working directory, structured args;
- enable/disable;
- unregister with explicit confirmation;
- discovery scan/review/register workflow;
- no secret fields or environment values are rendered.

## Error semantics

The API distinguishes at least:

- not found → 404;
- duplicate/running-state conflict → 409;
- disabled activation → 409;
- validation/path escape/unsupported candidate → 400;
- persistence/process/I/O failure → 500;
- browser origin rejection → 403.

## Test strategy

### Registry

- bootstrap once from legacy static config;
- deterministic serialization;
- register/edit/enable/disable/unregister persistence;
- duplicate ID/project rejection;
- malformed/unsupported registry rejection;
- failed write leaves previous file authoritative;
- traversal and symlink rejection.

### Discovery

- Rust/Node/Python fixture parsing;
- real Blender/Filesystem/Git recognition in manual smoke;
- Studio/tunnel-client/gateway/build/hidden ignores;
- malformed manifest isolation;
- no process execution path exists in scanner;
- already-registered marker.

### Runtime/API

- newly registered stopped MCP can start;
- disabled cannot start;
- update/disable/unregister running conflict;
- unregister removes inactive runtime state;
- same-origin enforcement on all M4 mutations;
- no environment/secret values in public DTOs/events.

### Frontend

- registry/discovery rendering;
- register workflow;
- edit validation surface;
- enable/disable and unregister state;
- invalidation refresh;
- dynamic lifecycle integration.

## Scope exclusions

M4 does not add SQLite, history/metrics/audit persistence, automatic restart/backoff, health checks, gateway traffic telemetry, remote authentication/RBAC, public bind support, tunnel plugin generalization, or source deletion.
