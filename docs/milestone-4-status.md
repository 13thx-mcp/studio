# Milestone 4 Status — Registry, Configuration & Auto-Discovery

- Target release: `v0.4.0`
- Branch during implementation: `feature/registry-auto-discovery`
- Status: COMPLETE — release verification passed
- Date: 2026-09-17

## Outcome

Milestone 4 replaces Studio's compile-time/static MCP inventory with a safe, persistent, explicitly approved MCP registry and metadata-only discovery workflow.

A supported MCP project below the configured MCP root can now be discovered, reviewed, registered, configured, enabled/disabled, supervised, and unregistered without changing Studio source code. Registration never starts a process automatically and unregister never deletes project source code.

## Implemented

### Persistent registry

- Schema-versioned file-backed MCP registry (`schema_version = 1`).
- Default local store: `data/registry.toml`.
- One-time bootstrap from legacy `[mcp.*]` configuration when no registry exists.
- Deterministic `BTreeMap` ordering/serialization.
- Atomic sibling-temp write, file sync, rename, then in-memory publication.
- Safe rejection of malformed or unsupported registry data.
- Stable MCP ID validation and duplicate ID/project rejection.
- Browser-safe registry views omit stored environment values.

### Path and executable policy

- Configured MCP root is canonicalized.
- Registered project, executable, and working directory use structured relative paths.
- Absolute paths, traversal/root/prefix components, and symlink components are rejected.
- Registered project must remain below the configured MCP root.
- Executable and working directory must remain below the registered project.
- Configuration is validated on mutation and again immediately before spawn.
- Arguments remain a structured argv list; no shell command representation or interpolation is accepted.

### Discovery

- Metadata-only direct-child scanning of the configured MCP root.
- Rust detection through `Cargo.toml`.
- Node detection through `package.json`.
- Python detection through `pyproject.toml`.
- Studio, tunnel-client, gateway infrastructure, hidden directories, and build/generated directories are ignored.
- Plain directories without supported manifests are not candidates.
- Malformed supported manifests are isolated as warnings instead of aborting the full scan.
- Discovery preview is separate from registration.
- Registration approval performs a server-side rescan and accepts only discovery-produced executable candidates.
- Discovery does not build, import, install, run scripts, or execute project code.

### Dynamic runtime integration

- Supervisor resolves configuration from the live registry rather than an immutable startup snapshot.
- Newly registered MCPs are immediately manageable without restarting Studio.
- Transient runtime state is created lazily.
- Disabled MCPs cannot start or restart.
- Edit, disable, and unregister require the MCP to be inactive.
- Unregister removes only Studio registration and inactive runtime state.
- Studio shutdown still stops Studio-owned active MCP processes.

### REST and realtime

Registry API:

```text
GET    /api/registry
GET    /api/registry/{id}
PUT    /api/registry/{id}
DELETE /api/registry/{id}
POST   /api/registry/{id}/enable
POST   /api/registry/{id}/disable
```

Discovery API:

```text
GET  /api/discovery
POST /api/discovery/scan
POST /api/discovery/{candidate_id}/register
```

Registry/discovery mutations use the existing browser same-origin protection.

Typed realtime invalidations:

```text
registry_changed
discovery_changed
```

Operational reconnect snapshots remain focused on MCP/tunnel runtime state; clients refetch registry/discovery after invalidation.

### Dashboard

- Registry panel with safe structured configuration editing.
- Discovery scan/review/register workflow.
- ID, display name, runtime, project path, executable, working directory, enabled state, and runtime state display.
- Enable/disable and unregister controls.
- Active-process mutation controls are disabled until stopped.
- Unregister confirmation explicitly states that source files are not deleted.
- Disabled MCP lifecycle controls are unavailable.
- Existing MCP/tunnel lifecycle and log workflows remain available.
- No environment or secret fields are rendered.

## Automated verification

Rust release gates passed:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo build --release --locked
```

The final targeted rerun additionally passed:

- 24/24 library unit tests.
- 4/4 registry/discovery integration tests.
- 7/7 supervisor lifecycle integration tests.
- 7/7 tunnel lifecycle integration tests.
- supervisor lifecycle repeated 5 consecutive runs with no failure.
- registry/discovery repeated 5 consecutive runs with no failure.

Frontend release gates passed:

```text
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

Frontend result: 4 test files, 12 tests passed, production Vite build succeeded.

## Real-workspace smoke verification

The real smoke used an isolated Studio instance on port `18104` and an isolated registry under `issues/m4-smoke/`, leaving normal Studio registry state untouched.

Verified discovery:

```text
Blender     detected as Rust
Filesystem  detected as Rust
Git         detected as Rust
Studio      ignored
tunnel-client ignored
gateway     ignored
```

Verified real Filesystem MCP workflow:

```text
discover
→ explicit register
→ remains stopped
→ configure structured args
→ start
→ running
→ stop
→ stopped
→ restart Studio
→ registration/config persisted
```

Verified enable/disable persistence:

```text
disable
→ start rejected with HTTP 409
→ restart Studio
→ still disabled
→ enable
→ start succeeds
→ stop succeeds
```

Verified WebSocket behavior:

- initial snapshot received;
- reconnect snapshot received.

Verified real tunnel regression:

```text
stopped
→ start / running PID
→ restart / new running PID / restart_count = 1
→ stop / stopped / PID cleared
```

Verified unregister/source preservation:

```text
unregister
→ registry lookup returns 404
→ Filesystem Cargo.toml SHA-256 unchanged
→ restart Studio
→ registry lookup still returns 404
```

Final isolated registry after unregister contained schema v1 with no registered servers.

Result:

```text
M4 real-workspace smoke: PASS
```

## Security review

`docs/threat-model.md` covers active M4 threats and controls including:

- malicious project manifests;
- path traversal and symlink escape;
- executable-path and argument abuse;
- environment/secret leakage;
- registry file tampering and malformed persisted state;
- unauthorized registration/config mutations;
- discovery-triggered execution risk;
- unregister/delete confusion;
- running-process configuration mutation;
- validation/spawn TOCTOU;
- duplicate/stale registry entries.

No known Critical or High severity M4 finding remains unresolved at release closure.

## Architecture records

Created:

- `docs/milestone-4-design.md`
- `docs/adr/0004-file-backed-mcp-registry.md`
- `docs/adr/0005-dynamic-registry-reconciliation.md`

Updated:

- `docs/architecture.md`
- `docs/threat-model.md`
- `README.md`
- `CHANGELOG.md`
- `studio.example.toml`

## Scope exclusions preserved

M4 does not introduce:

- SQLite;
- historical runtime/tunnel sessions;
- persisted metrics or audit-event history;
- automatic restart/backoff or crash-loop circuit breaking;
- gateway request/latency telemetry;
- remote authentication/RBAC/public Studio exposure;
- generic tunnel plugin architecture.

## Known limitation

M4 deliberately does not expose generic browser editing of MCP environment variables or secrets. Legacy server-side environment values may be preserved by bootstrap but are omitted from browser-visible registry DTOs. A generalized secret-reference model requires a separate security design.

## Release closure

All functional, automated, security-review, and real-workspace smoke prerequisites for the M4 release have passed. Release closure proceeds with:

1. exact final branch commit: `chore(release): prepare v0.4.0`;
2. dev-git-control readiness check;
3. switch to `main`;
4. `--no-ff` merge through Git MCP;
5. annotated `v0.4.0` tag;
6. delete `feature/registry-auto-discovery`;
7. verify clean `main`, tag target, and merge history.
