# M5 Common Context — Runtime Distribution, Update Manager & Fleet State

## Purpose

This file is shared context for every M5 implementation submission. Read this file before the active `M5.x.md` file. Do not load every submilestone file unless needed.

## Current baseline

- MCP Studio current released baseline: `v0.4.0`.
- M0–M4 are complete.
- Studio `main` contains the updated roadmap and flat-runtime-root registry support.
- Aira is the reference development host.
- Do not modify Mirin during M5 implementation unless a later submission explicitly requires source-less-host verification.

## Current filesystem/runtime architecture

```text
mcp-server/
├── filesystem/
├── git/
├── exec/
├── gateway/
├── studio/
├── fleet/
├── tunnel-client/      # official openai/tunnel-client source checkout
├── blender/
│
├── bin/                # Rust MCP executables only, flat
│   ├── rust-mcp-filesystem
│   ├── rust-mcp-git
│   ├── rust-mcp-exec
│   ├── rust-mcp-gateway
│   └── rust-mcp-blender
│
└── runtime/            # non-MCP runtime/config/state
    ├── gateway/servers.d/
    ├── studio/
    ├── tunnel-client/
    └── fleet/
```

The separation is intentional:

- `source_root` = `mcp-server/`
- `bin_root` = `mcp-server/bin/`
- `runtime_root` = `mcp-server/runtime/`

Do not reintroduce nested `bin/<component>/` paths or `mcp-server/src/*`.

## Runtime-host requirement

A source-less runtime host must be able to operate without:

- project source repositories;
- Rust/Cargo;
- Node.js/pnpm;
- `target/`;
- `node_modules/`.

Runtime updates must therefore consume verified published artifacts rather than compile source locally.

## Release sources

Project-owned components use GitHub Releases under `13thx-mcp`:

```text
13thx-mcp/filesystem   v0.1.0+
13thx-mcp/git          v0.1.0+
13thx-mcp/exec         v0.1.0+
13thx-mcp/gateway      v0.1.0+
13thx-mcp/studio       v0.4.0+
13thx-mcp/fleet        v0.1.0+
```

Current Rust release asset contract:

```text
<binary>-vX.Y.Z-darwin-amd64.tar.gz
<binary>-vX.Y.Z-darwin-arm64.tar.gz
SHA256SUMS.txt
```

Examples:

```text
rust-mcp-git-v0.1.0-darwin-amd64.tar.gz
rust-mcp-git-v0.1.0-darwin-arm64.tar.gz
SHA256SUMS.txt
```

Studio release assets additionally contain the matching `web/dist` runtime dashboard.

Fleet is architecture-independent:

```text
mcp-fleet-vX.Y.Z.tar.gz
SHA256SUMS.txt
```

Tunnel-client must use official OpenAI releases only:

```text
https://github.com/openai/tunnel-client
```

Current tested release: `v0.0.14`.

## Existing fleet behavior

`mcp-server/fleet/scripts/fleetctl.py` already provides useful reference behavior including:

- platform normalization;
- tunnel-client latest-release discovery;
- SHA-256 verification;
- safe ZIP extraction;
- extracted binary version verification;
- versioned tunnel install directories;
- atomic `current` symlink activation;
- generated Gateway/Studio/tunnel runtime config;
- source/bin/runtime root separation;
- fleet doctor/status.

M5 should avoid duplicating unsafe ad-hoc logic. Reuse, extract, or formalize proven behavior where appropriate.

### Incident-derived runtime reconciliation requirement

A 2026-09-18 runtime incident proved that a healthy process tree and a successful Fleet bundle render check at update time do not guarantee that active generated configuration stays synchronized later.

Observed failure class:

- `runtime/tunnel-client/config.yaml` retained an old source-tree Gateway/config path;
- a runtime helper/CLI override could mask that stale canonical YAML;
- `runtime/gateway/servers.d/exec.yaml` drifted from the active Fleet host profile;
- Gateway could locally report the expected aggregate child-tool count while a connected client session still exposed an older/incomplete callable catalog.

M5.9A therefore owns deterministic runtime-config reconciliation and exact Gateway catalog repair. Generated config must be treated as desired-state inventory, and split launch paths must not be allowed to mask canonical drift. Fleet must provide a pure-render desired-state contract; Studio must persist last-known-managed fingerprints to distinguish safe managed drift from unknown local edits; launcher surfaces not owned by Fleet (including the current `runtime/tunnel-client/run.sh`) are validate-only unless a separate ownership contract is established.

## Studio current capabilities

Studio already has:

- Rust/Axum backend;
- React/TypeScript/Vite frontend;
- persistent schema-versioned file registry;
- MCP lifecycle supervisor;
- tunnel lifecycle supervisor;
- REST API and WebSocket events;
- structured logging and secret redaction;
- path confinement and symlink protections;
- dynamic MCP registry;
- localhost-only bind policy;
- browser same-origin mutation protections;
- flat runtime-root registry compatibility.

## M5 target

M5 must deliver a safe manual update system and fleet/version visibility. Unattended automatic release update is not part of M5. M5.9A additionally provides runtime drift detection and a transactionally safe reconciliation primitive for Fleet-managed generated configuration; periodic unattended reconciliation policy remains M7.

Target update state model:

```text
IDLE
  ↓ check
CHECKING
  ↓ newer version
AVAILABLE
  ↓ explicit operator/policy approval
DOWNLOADING
  ↓
VERIFYING
  ↓
STAGING
  ↓
PREPARING_RUNTIME
  ↓
ACTIVATING
  ↓
HEALTH_VERIFYING
  ↓ success
CURRENT
```

Failure states should be explicit rather than collapsed into a generic error:

```text
DOWNLOAD_FAILED
VERIFY_FAILED
STAGE_FAILED
STOP_FAILED
ACTIVATE_FAILED
HEALTH_FAILED
ROLLBACK_REQUIRED
ROLLBACK_FAILED
```

## Version identity

Studio must distinguish at least:

```text
Desired version
Installed version
Running version
Latest available version
```

These values can temporarily differ during an update/restart.

Useful drift states:

```text
CURRENT
UPDATE_AVAILABLE
INSTALLED_RESTART_REQUIRED
DRIFTED
UNKNOWN
BROKEN
```

Never infer runtime synchronization from source Git HEAD alone.

## Safety invariants

1. Never activate an artifact before integrity and structure validation.
2. A failed checksum/version/structure check must leave the active installation unchanged.
3. Release repositories and component IDs come from server-side policy; the browser must not provide arbitrary URLs or executable paths.
4. Archive extraction must reject traversal, absolute paths, and unsafe links.
5. Local config, secrets, registry/database state, and host profiles must not be overwritten by release archives.
6. Only the affected MCP should restart during a normal MCP update.
7. An MCP that was stopped before update should remain stopped after a successful update.
8. Rollback material must be prepared before destructive activation when rollback is required.
9. Studio self-update requires an external supervisor/launcher contract; Studio must not blindly replace/kill itself in-process.
10. Automatic/unattended release-update policy belongs to M7, after M5 manual transactions and M6 persisted history are proven.
11. Fleet-generated runtime config must converge to the active host profile and trusted `bin_root` / `runtime_root`; a helper/CLI override must not mask stale canonical config.
12. Gateway catalog verification must compare expected tool identity/set where feasible, not only aggregate counts.
13. Local Gateway synchronization and remote connected-client catalog freshness are separate facts unless an acknowledgement is observable.
14. Unknown/unmanaged local config edits must not be silently overwritten by automatic reconciliation.

## Platform support for M5

Required initial runtime targets:

```text
darwin-amd64
darwin-arm64
```

Normalize:

```text
x86_64 / amd64 → amd64
arm64 / aarch64 → arm64
Darwin → darwin
```

Unsupported platform/architecture must fail closed.

## Git / implementation workflow

M5 uses one milestone integration branch: `feat/m5-runtime-distribution`. Create it once from the `main` commit immediately before M5 implementation begins, then keep M5.1 through M5.12 linear on that branch. Each submilestone submission is represented by its own focused Conventional Commit rather than a temporary submilestone branch plus merge commit.

For each submilestone:

1. Inspect the current code and verify the integration branch is clean before editing.
2. Continue on `feat/m5-runtime-distribution`; do not create or merge a separate `M5.x` implementation branch.
3. Keep the change scoped to the active submilestone.
4. Add/update tests with the implementation.
5. Run relevant quality gates and record evidence in the active `M5.x.md` file.
6. Do not commit secrets, runtime state, generated host-local config, release downloads, or temporary fixtures.
7. Commit the verified submilestone as a focused Conventional Commit on the M5 integration branch.
8. Do not merge the integration branch into `main` after an individual M5.x submission.
9. At final M5 closure, reconcile the integration branch with current `main`, run the complete milestone gates, validate merge readiness, and merge `feat/m5-runtime-distribution` into `main` once with `--no-ff`.
10. Push the M5 integration branch as needed for collaboration/recovery; push `main` only after the final M5 merge is complete and approved.
11. After a branch has been successfully merged and the destination ref is safely published when required, delete the merged branch immediately. At final M5 closure this means deleting `feat/m5-runtime-distribution` after the final merge/push. Do not retain stale implementation branches.

This branch-cleanup rule also applies to any temporary/support branch created outside the normal M5 integration flow: once merged and no longer needed, delete it rather than leaving stale local or remote refs.

Do not create a release tag unless the active submission explicitly calls for release preparation.

## Expected quality gates

Backend changes normally require:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Studio UI changes additionally require:

```bash
pnpm --dir web lint
pnpm --dir web typecheck
pnpm --dir web test
pnpm --dir web build
```

Fleet Python changes normally require:

```bash
python3 -m py_compile scripts/fleetctl.py
python3 -m unittest discover -s tests -v
```

Run targeted real-workspace smoke tests for privileged lifecycle/update behavior.

## Security review topics for M5

Track explicitly:

- malicious release metadata;
- compromised/tampered archive;
- checksum mismatch;
- archive path traversal;
- symlink/link extraction attacks;
- wrong architecture artifact;
- downgrade/replay behavior;
- untrusted arbitrary release URL;
- update-time secret leakage;
- TOCTOU between verification and activation;
- partial activation;
- interrupted update;
- rollback failure;
- update/restart loops;
- disk exhaustion due to retained releases;
- Studio self-update launcher privilege boundary.

## Documentation expectations

When an implementation decision changes a durable architectural contract, update:

- `ROADMAP.md` if milestone scope changes;
- `docs/architecture.md`;
- `docs/threat-model.md`;
- an ADR under `docs/adr/` when appropriate;
- `CHANGELOG.md` when behavior is user-visible/release-relevant.

## Completion handoff for each submilestone

At the end of each submission, report:

- branch/commits/merge state;
- files changed;
- implemented contracts;
- tests/gates run and results;
- smoke tests and observed results;
- known limitations;
- whether the next submilestone can start;
- any prerequisite that must be carried forward.
