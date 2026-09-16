# MCP Studio Architecture

## Purpose

MCP Studio is a local-first control plane for MCP servers and the existing secure tunnel runtime under `mcp-server/`.

## Current boundary — Milestone 4

Milestone 4 extends the completed MCP supervisor, browser dashboard, and secure tunnel manager with a persistent MCP registry and metadata-only project discovery.

Registry/configuration persistence is intentionally file-backed in M4. SQLite, runtime/session history, persisted metrics/events/audit history, automatic restart/backoff, remote authentication, and MCP gateway traffic telemetry remain later milestones.

## Components

- `api`: HTTP/WebSocket API, same-origin mutation protection, SPA/static-file boundary.
- `config`: typed bootstrap/runtime configuration and validation.
- `registry`: schema-versioned persistent MCP registry, atomic writes, ID/path validation, and browser-safe views.
- `discovery`: metadata-only direct-child scanning of supported Rust, Node, and Python project manifests.
- `supervisor`: MCP child-process lifecycle, transient runtime state, PID ownership, log capture, event publication, and shutdown cleanup.
- `tunnel`: independent tunnel configuration/lifecycle/secret-redaction domain.
- `realtime`: typed bounded event streams and registry/discovery invalidation events.
- `web`: React + TypeScript + Vite operational dashboard plus Registry/Discovery management.
- `metrics`: reserved for later historical/request metrics.
- `storage`: reserved for SQLite-backed Milestone 5 persistence.
- `logging`: structured logging initialization.
- `error`: shared typed error boundary.

## Runtime model

```text
Browser
   |
   | same-origin HTTP / WebSocket
   v
+-----------------------------------------------------------------+
| MCP Studio / Axum                                               |
|                                                                 |
| Registry/Discovery REST ---> Registry <----> data/registry.toml |
|                                 |                               |
| MCP lifecycle REST ------------+----> MCP Supervisor ---> MCP   |
|                                                                 |
| Tunnel lifecycle REST ----------------> Tunnel Supervisor       |
|                                                                 |
| WebSocket <--- process/tunnel events + registry invalidations   |
| React/Vite dashboard                                            |
+-----------------------------------------------------------------+
                |
                +---- metadata-only scan ----> configured MCP root
```

`DiscoveredProject`, persistent `RegisteredMcp`, and transient `ProcessStatus` are separate models.

## Persistent registry

M4 uses a versioned TOML registry, defaulting to:

```text
data/registry.toml
```

The registry owns:

- schema version;
- configured MCP-root identity;
- stable server IDs;
- display name and enabled state;
- project/runtime metadata;
- structured project-relative executable and working-directory paths;
- structured argument arrays;
- inherited server-side environment values where legacy configuration requires them.

Browser DTOs omit environment values.

Writes are complete-document copy-on-write updates:

```text
validate candidate state
→ deterministic serialize
→ write sibling temp file
→ fsync temp file
→ atomic rename
→ publish candidate as live in-memory state
```

Malformed or unsupported existing registry files fail startup and are never silently overwritten.

When no registry exists, existing `[mcp.*]` configuration is converted once into schema-v1 records. Once the persistent registry exists it is authoritative.

## MCP-root and executable policy

The configured MCP root is canonicalized when the registry opens.

Persistent MCP records store:

- project path relative to MCP root;
- executable path relative to the registered project;
- working directory relative to the registered project;
- arguments as an explicit list.

Studio rejects absolute paths, `..`, root/prefix escapes, and symlink components in project/working-directory/executable paths. The project must remain under the configured MCP root; working directory and executable must remain under that project.

The executable policy is validated on mutation and validated again immediately before spawn. The executable must be a regular file at activation time.

There is no raw shell-command representation and no shell interpolation.

## Discovery

M4 discovery scans direct child directories of the configured MCP root. It never invokes project code, package managers, compilers, interpreters, install hooks, shell commands, or imports.

Supported metadata:

- Rust: `Cargo.toml` package/default-run/bin metadata.
- Node: `package.json` package name and project-local `bin` entries.
- Python: `pyproject.toml` project/script metadata, with only existing project-local `.venv` script paths offered as executable candidates.

Known non-targets such as `studio`, `tunnel-client`, `gateway`, the nested infrastructure container, hidden directories, build output, and common generated directories are ignored.

A malformed or unreadable project is isolated so it cannot abort the rest of a scan.

Discovery returns preview candidates only. Registration is a separate same-origin mutation; the server rescans the candidate and only accepts an executable candidate produced by discovery. Registration never starts a process.

## Dynamic registry/runtime reconciliation

The supervisor and API share one live registry object. The supervisor does not own an immutable startup-only configuration snapshot.

Transient runtime state is created lazily for registered IDs.

Rules:

- newly registered MCPs are immediately visible and begin stopped;
- disabled MCPs remain registered/visible but cannot start or restart;
- edit, disable, and unregister are rejected while a process is starting/running/stopping;
- operator must stop first before those mutations;
- unregister removes Studio registration and inactive runtime state only;
- unregister never deletes source code or arbitrary files;
- shutdown enumerates the current registry and stops Studio-owned active children.

This avoids a separate registry-reload phase that could diverge after persistence succeeds.

## Lifecycle domains and ownership

MCP and tunnel use the same operational state vocabulary but remain separate supervisors:

```text
STOPPED
   | start
   v
STARTING
   | spawn
   v
RUNNING --------------------+
   | stop                    | unexpected exit / wait failure
   v                         v
STOPPING                   FAILED
   |                         |
   +---------- exit -------->+
```

Studio may signal only PIDs obtained from children it spawned and currently tracks. Browser APIs never accept a PID.

Unix stop remains SIGTERM + bounded wait + SIGKILL fallback.

## Tunnel boundary

Tunnel lifecycle remains independent from the MCP registry. M4 does not generalize tunnel configuration into a plugin/registry framework.

Tunnel startup, path confinement, secret references, and log redaction remain as defined by ADR 0003.

## HTTP API

```text
GET  /health
GET  /api/status

GET  /api/mcp
GET  /api/mcp/{id}
POST /api/mcp/{id}/start
POST /api/mcp/{id}/stop
POST /api/mcp/{id}/restart
GET  /api/mcp/{id}/logs

GET    /api/registry
GET    /api/registry/{id}
PUT    /api/registry/{id}
DELETE /api/registry/{id}
POST   /api/registry/{id}/enable
POST   /api/registry/{id}/disable

GET  /api/discovery
POST /api/discovery/scan
POST /api/discovery/{candidate_id}/register

GET  /api/tunnel
POST /api/tunnel/start
POST /api/tunnel/stop
POST /api/tunnel/restart
GET  /api/tunnel/logs

GET  /api/ws
```

Expected not-found/conflict/validation classes are mapped separately so callers can distinguish invalid configuration from runtime/persistence failures.

## Realtime model

Operational snapshot events remain focused on MCP process status and tunnel status.

Typed incremental events are:

```text
snapshot
process_status
log
tunnel_status
tunnel_log
registry_changed
discovery_changed
resync_required
```

Registry/discovery changes are invalidations: the browser refetches current REST state instead of receiving full configuration in WebSocket payloads.

## Browser security boundary

Studio remains loopback-only.

All lifecycle/registry/discovery/tunnel mutations and WebSocket upgrades use the existing same-origin `Origin`/`Host` protection. Local non-browser clients without an `Origin` header remain supported by design.

Remote operation remains unsupported until authentication, authorization, CSRF/CORS, TLS/exposure, and session policy are explicitly designed.

## Frontend architecture

The dashboard separates:

- Studio realtime connection state;
- MCP runtime state;
- tunnel runtime state;
- persistent registry configuration;
- discovery preview/approval.

Registry UI exposes only safe metadata. It provides edit, enable/disable, and unregister actions. Discovery UI provides scan/review/register and does not auto-register or auto-start.

## Error handling

- Library boundaries return typed `StudioError` values.
- Expected lifecycle/registry conflicts do not panic.
- Browser same-origin failures return `403 Forbidden`.
- Registry/path validation failures are rejected before mutation or spawn.
- Persistence failure leaves the previous in-memory registry authoritative because the new state is published only after successful atomic replacement.
- Per-project discovery parse/read failures do not terminate the whole scan.

## Security boundary summary

1. Studio HTTP remains loopback-only.
2. Browser privileged operations are same-origin constrained.
3. Discovery is metadata-only and never grants execution automatically.
4. Browser APIs accept no raw shell commands or PIDs.
5. MCP executable authority is confined to a registered project under the configured MCP root.
6. Project/working/executable paths reject traversal and symlink components and are revalidated before spawn.
7. Studio signals only child PIDs it owns.
8. Registry browser views/events omit environment/secret values.
9. Unregister never deletes source code.
10. Tunnel remains a separate constrained lifecycle/secret domain.

## Evolution rule

Changes that materially affect process ownership, executable/root policy, symlink policy, registry persistence/migration, discovery approval, browser-origin policy, authentication, tunnel invocation/secret policy, gateway behavior, or external API contracts require architecture/threat review and may require a new ADR.
