# MCP Studio

Local-first web control plane for safely discovering, registering, configuring, supervising, and observing MCP servers plus the separately managed secure tunnel runtime under `mcp-server/`.

## Current status

Milestone 6 — Persistence, Metrics & Auditability is **VERIFIED / CLOSED** as `v0.6.0-alpha`. Its full native darwin-arm64 qualification consumed independently qualified M5 publication evidence. The `v0.6.0-alpha` tag exists locally; pushing that tag or publishing a release remains a separate release action.

Current capabilities include:

- schema-versioned persistent MCP registry;
- atomic file-backed registry writes at `data/registry.toml` by default;
- one-time bootstrap from legacy `[mcp.*]` configuration;
- configurable, canonicalized MCP discovery root;
- metadata-only Rust/Node/Python discovery;
- explicit review/approval before registration;
- dynamic registration without Studio source changes or restart;
- structured project-relative executable, working-directory, and argument configuration;
- path traversal/symlink/executable confinement;
- enable/disable and unregister semantics;
- live registry integration with the MCP supervisor;
- MCP start / stop / restart with Studio-owned PID safety;
- secure tunnel start / stop / restart in a separate lifecycle domain;
- bounded MCP/tunnel logs;
- tunnel secret-value redaction;
- REST lifecycle/registry/discovery APIs;
- typed WebSocket runtime events and registry/discovery invalidations;
- React/TypeScript dashboard with MCP, tunnel, Registry, and Discovery sections;
- trusted release inventory, staged updates, rollback, Fleet reconciliation, and Studio/Tunnel update transactions;
- SQLite-backed operational history, typed audit evidence, metrics aggregation, retention, and historical API/UI;
- same-origin browser protection for privileged mutations and WebSocket upgrades;
- loopback-only Studio bind.

Automatic restart/backoff, remote authentication/RBAC, public Studio exposure, and Gateway-observed MCP request/usage telemetry remain later milestones. Historical persistence/metrics/auditability are part of the closed M6 baseline.

## Requirements

- Rust 1.98.1
- Cargo
- Node.js and pnpm for dashboard development/build
- macOS on Apple Silicon (arm64) for supported runtime operation
- Existing `mcp-server/tunnel-client` runtime bundle for tunnel lifecycle management

The pinned Rust toolchain is declared in `rust-toolchain.toml`.

## Registry and bootstrap

M4 adds:

```toml
[registry]
path = "data/registry.toml"
mcp_root = ".."
```

`registry.path` is Studio-owned mutable state. `registry.mcp_root` is the only root used for M4 discovery/registration.

If the registry file does not exist, Studio converts legacy `[mcp.*]` entries into schema-v1 registry records and persists them once. Once the registry exists, it is authoritative; static entries are not reapplied on every restart.

Example legacy bootstrap entry:

```toml
[mcp.filesystem]
name = "Filesystem"
command = "../filesystem/target/debug/rust-mcp-filesystem"
working_dir = "../filesystem"
args = ["--root", ".."]
```

## Registry path/executable policy

Registered MCPs use structured paths rather than shell command strings:

```text
MCP root
└── registered project
    ├── project-relative executable
    └── project-relative working directory
```

Studio rejects:

- absolute project/executable/working-directory paths;
- `..` traversal or root/prefix escapes;
- symlink components in registered execution paths;
- executable or working-directory resolution outside the registered project;
- project resolution outside the configured MCP root.

The policy is validated on configuration mutation and again immediately before process spawn. Arguments remain a structured argv list and are never shell-interpolated.

Browser registry responses do not include stored environment values.

## Discovery

Discovery is metadata-only and scans direct child directories of the configured MCP root.

Supported initial metadata sources:

- Rust: `Cargo.toml` package/default-run/bin metadata;
- Node: `package.json` package name and safe project-local `bin` entries;
- Python: `pyproject.toml` project/script metadata and existing project-local `.venv` script paths.

Discovery never runs:

- Cargo/build commands;
- npm/pnpm/yarn scripts or installs;
- Python imports or modules;
- project executables;
- shell commands.

Known infrastructure/build locations such as Studio itself, tunnel-client, gateway, hidden directories, target/build output, and common generated directories are ignored. Plain directories without a supported manifest are not shown as candidates.

A discovery result is only a preview. Registration requires a separate explicit operator action, the server rescans the candidate, and registration leaves the MCP stopped.

## Enable, disable, edit, and unregister semantics

- `enabled = false` keeps the MCP registered and visible but prevents start/restart.
- Edit, disable, and unregister require the MCP to be inactive; stop it first.
- Unregister removes only Studio registration and inactive runtime state.
- Unregister never deletes project source or arbitrary filesystem content.
- A newly registered MCP becomes available to the live supervisor without restarting Studio.

## Build managed MCP servers

Discovery does not build projects. An executable must exist and satisfy the project confinement policy before activation.

For the default Rust projects, for example:

```bash
cd mcp-server/blender
cargo build

cd ../filesystem
cargo build

cd ../git
cargo build
```

## Tunnel runtime

Tunnel lifecycle remains a separate domain from the MCP registry.

The configured tunnel runtime is invoked internally as:

```text
tunnel-client-runtime-cloudflared run --config <validated-config-file>
```

The browser cannot supply a tunnel executable, config path, shell command, arbitrary argv, or PID. Tunnel secret references stay server-side and resolved values are redacted from tunnel logs before REST/WebSocket/dashboard publication.

Studio rejects and removes tunnel-runtime authority overrides from `MCP_COMMAND`, `MCP_SERVER_URL`, and `CONTROL_PLANE_POLL_CHANNELS` when it starts the tunnel runtime. These keys cannot be reintroduced through `[tunnel.env]`; the validated tunnel configuration remains the sole MCP target and poll-channel authority.

## Build the dashboard

```bash
cd mcp-server/studio/web
pnpm install
pnpm build
```

For frontend development:

```bash
pnpm dev
```

Vite proxies REST and WebSocket traffic to Studio on `127.0.0.1:18100`.

## Run Studio

```bash
cd mcp-server/studio
cargo run -- --config studio.example.toml
```

Default listen address:

```text
127.0.0.1:18100
```

After building the dashboard, open:

```text
http://127.0.0.1:18100/
```

## API

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

Expected not-found, conflict, validation/path-policy, and persistence/process failures are mapped into distinct API error classes.

## Realtime model

Browser clients receive typed events:

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

The reconnect snapshot contains operational MCP/tunnel state. Registry/discovery events are invalidations; the browser refetches those resources over REST instead of receiving full registry configuration over WebSocket.

## Dashboard

The dashboard provides:

- Studio realtime connection state;
- MCP registered/running/failed summary;
- MCP lifecycle and recent logs;
- separate secure tunnel lifecycle and redacted logs;
- persistent Registry list and safe edit/enable/disable/unregister actions;
- Discovery scan/review/register workflow;
- runtime-state-aware disabling of unsafe registry mutations;
- automatic WebSocket reconnect/resync.

No environment/secret fields are rendered by the registry/discovery UI.

## Security baseline

- Loopback-only HTTP control plane.
- Same-origin protection for browser privileged mutations and WebSocket upgrades.
- No raw browser-supplied shell commands or PIDs.
- Metadata-only discovery with no auto-register or auto-start.
- Project/executable/working-directory confinement below configured MCP root.
- Traversal and symlink-component rejection.
- Structured argv with no shell interpolation.
- Pre-spawn path/executable revalidation.
- Stop-first edit/disable/unregister semantics.
- Studio signals only tracked child PIDs it spawned.
- Registry browser DTOs/events omit environment values.
- Unregister never deletes source code.
- Tunnel remains a separate constrained lifecycle/secret boundary.

See `docs/threat-model.md` and ADRs 0004/0005 for the detailed trust model and residual risks.

## Development and release checks

Rust:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo build --release --locked
```

Filesystem/discovery-sensitive coverage should also be repeated before release:

```bash
for i in 1 2 3 4 5; do cargo test --test registry_discovery || exit 1; done
```

Dashboard:

```bash
cd web
pnpm install
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

The M5 local release gate passed Rust/frontend quality checks, update/reconciliation regressions, security checks, and native Darwin arm64 packaging. Public artifact qualification remains tracked in `docs/milestone-5-status.md`.

## Engineering documents

- `ROADMAP.md` — long-term milestone plan.
- `docs/milestone-4-design.md` — M4 design contract.
- `docs/milestone-4-status.md` — completed M4 verification and closure evidence.
- `docs/milestone-5-status.md` — M5 local closeout and public-artifact qualification evidence.
- `docs/architecture.md` — current component/runtime architecture.
- `docs/threat-model.md` — active security model.
- `docs/adr/0004-file-backed-mcp-registry.md` — persistence/path policy.
- `docs/adr/0005-dynamic-registry-reconciliation.md` — live registry/discovery approval model.
- `docs/adr/0003-secure-tunnel-supervision.md` — independent tunnel boundary.
- `CONTRIBUTING.md` — SDLC and development workflow.
