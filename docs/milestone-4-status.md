# Milestone 4 Status — Registry, Configuration & Auto-Discovery

- Target release: `v0.4.0`
- Branch: `feature/registry-auto-discovery`
- Status: Implementation complete; release verification pending
- Date: 2026-09-16

## Implemented

### Persistent registry

- Schema-versioned file-backed MCP registry.
- Default local store: `data/registry.toml`.
- One-time bootstrap from legacy `[mcp.*]` configuration when the persistent registry does not exist.
- Deterministic `BTreeMap` server ordering/serialization.
- Atomic sibling-temp write, file sync, rename, then in-memory publication.
- Fail-safe load behavior for malformed or unsupported registry data.
- Stable MCP ID validation and duplicate ID/project rejection.
- Browser-safe registry views omit stored environment values.

### Path/executable policy

- Configured MCP root canonicalization.
- Project-relative project, executable, and working-directory model.
- Rejection of absolute paths and traversal/root/prefix components.
- Rejection of symlink components.
- Project confinement below MCP root.
- Executable/working-directory confinement below registered project.
- Revalidation immediately before spawn.
- Structured argv only; no shell command representation or interpolation.

### Discovery

- Metadata-only direct-child scanning of configured MCP root.
- Rust detection through `Cargo.toml`.
- Node detection through `package.json`.
- Python detection through `pyproject.toml`.
- Explicit exclusions for Studio, tunnel-client, gateway infrastructure, hidden directories, build/generated directories, and the nested infrastructure container.
- Malformed/unreadable candidate isolation.
- Discovery preview separate from registration.
- Server-side rescan during approval.
- Initial registration accepts only discovery-produced executable candidates.
- Registration does not start the process.

### Dynamic runtime integration

- Supervisor resolves configuration from the live registry rather than an immutable startup snapshot.
- Lazy transient runtime state for newly registered MCPs.
- Newly registered MCPs are immediately manageable without Studio restart.
- Disabled MCP start/restart rejection.
- Stop-first edit/disable/unregister semantics.
- Inactive runtime state removal after unregister.
- Unregister never deletes project source code.
- Studio shutdown enumerates the current registry and stops Studio-owned active MCPs.

### REST/realtime

Added registry endpoints:

```text
GET    /api/registry
GET    /api/registry/{id}
PUT    /api/registry/{id}
DELETE /api/registry/{id}
POST   /api/registry/{id}/enable
POST   /api/registry/{id}/disable
```

Added discovery endpoints:

```text
GET  /api/discovery
POST /api/discovery/scan
POST /api/discovery/{candidate_id}/register
```

Registry/discovery mutations use the existing browser same-origin protection.

Added typed realtime invalidations:

```text
registry_changed
discovery_changed
```

Operational reconnect snapshots remain focused on MCP/tunnel runtime state; clients refetch registry/discovery after invalidation.

### Dashboard

- Persistent Registry panel.
- Discovery panel with explicit scan/review/register flow.
- Display of ID, name, runtime, project path, executable, working directory, enabled state, and current runtime state.
- Safe structured configuration editing.
- Enable/disable controls.
- Stop-first UI disabling for edit/disable/unregister while active.
- Unregister confirmation explicitly states that source files are not deleted.
- Disabled MCP lifecycle controls are unavailable.
- Existing MCP/tunnel lifecycle/log workflows retained.
- No environment/secret fields are rendered.

## Automated coverage added

### Registry/discovery backend

- deterministic persistence round-trip;
- unsupported schema rejection;
- browser-safe public view excludes environment values;
- traversal rejection;
- Rust/Node/Python metadata detection;
- malformed manifest isolation;
- known infrastructure ignores;
- discovery → approval → registration;
- edit/disable persistence across registry reopen;
- unregister persistence across reopen;
- source preservation after unregister;
- duplicate project rejection;
- executable traversal rejection;
- symlink executable escape rejection.

### Runtime/API

- MCP lifecycle fixtures now run project-confined local executable copies rather than arbitrary host executables;
- disabled MCP cannot start;
- existing start/stop/restart/crash/shutdown coverage retained;
- cross-origin registry mutation rejection added.

### Frontend

- structured registry edit request coverage;
- enable/disable and unregister endpoint coverage;
- scan and registration verified as separate operations;
- request bodies contain structured executable/working-dir/args fields rather than shell command fields.

## Architecture/security documentation

Created:

- `docs/milestone-4-design.md`
- `docs/adr/0004-file-backed-mcp-registry.md`
- `docs/adr/0005-dynamic-registry-reconciliation.md`

Updated:

- `docs/architecture.md`
- `docs/threat-model.md`
- `studio.example.toml`

The threat model now actively covers malicious manifests, traversal, symlink escape, executable/argument abuse, environment leakage, registry tampering, malformed persistence, unauthorized mutations, running-state config mutation, TOCTOU, duplicate/stale registry entries, and unregister/delete confusion.

## Verification state

The implementation has been written and committed, but the full M4 release gates have **not yet been executed in this session** because the connected workspace interface provides repository/file/Git actions but no command-execution action.

Therefore M4 must not yet be marked released, merged to `main`, or tagged `v0.4.0`.

### Required Rust gates

Run from `mcp-server/studio`:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo build --release --locked
```

Also repeat race/filesystem-sensitive coverage, for example:

```bash
for i in 1 2 3 4 5; do cargo test --test registry_discovery || exit 1; done
```

### Required frontend gates

```bash
cd web
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

## Required real-workspace smoke

Start Studio with an isolated M4 smoke registry path so normal operator state is not modified.

Verify discovery at minimum:

```text
Blender      detected as Rust
Filesystem   detected as Rust
Git          inspected/detected as Rust
Studio       ignored
tunnel-client ignored
gateway      ignored
```

Then use one real stopped candidate that is not already registered:

```text
discover
→ review
→ register
→ appears in registry
→ remains stopped
→ start
→ running
→ stop
→ stopped
→ restart Studio
→ registration and configuration remain present
```

Enable/disable:

```text
disable
→ start rejected/unavailable
→ restart Studio
→ still disabled
→ enable
→ start succeeds
```

Unregister:

```text
stop
→ unregister
→ removed from registry
→ source tree remains untouched
→ restart Studio
→ remains unregistered
```

Regression smoke:

- existing MCP lifecycle;
- tunnel start/stop/restart;
- WebSocket reconnect/resync;
- Studio graceful shutdown cleanup.

## Release closure checklist

Do not close M4 until all are true:

- [ ] Rust gates pass.
- [ ] Frontend gates pass.
- [ ] Repeated registry/discovery test passes.
- [ ] Real Blender/Filesystem discovery smoke passes.
- [ ] Real registration/start/stop/restart-persistence smoke passes.
- [ ] Enable/disable persistence smoke passes.
- [ ] Unregister/source-preservation smoke passes.
- [ ] Existing MCP lifecycle regression smoke passes.
- [ ] Existing tunnel lifecycle regression smoke passes.
- [ ] WebSocket reconnect/resync regression passes.
- [ ] Threat-model review has no unresolved Critical/High issue.
- [ ] Dev-git-control readiness passes.
- [ ] Release-prep version/changelog changes are committed exactly as `chore(release): prepare v0.4.0`.
- [ ] Feature branch is reconciled with current `main` and clean.
- [ ] `--no-ff` merge is performed through Git MCP.
- [ ] Annotated `v0.4.0` tag points to the M4 merge commit.
- [ ] Feature branch is deleted and final `main` is clean.

## Current known limitation

M4 deliberately does not provide generic browser editing of MCP environment variables or secrets. Legacy server-side environment values can be preserved through bootstrap but are omitted from browser-visible registry DTOs. A generalized secret-reference schema should be designed separately rather than exposing raw stored values.
