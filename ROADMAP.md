# MCP Studio Roadmap

> Scope: local-first control plane for MCP runtime lifecycle, configuration, secure tunnel lifecycle, release/update management, fleet state, observability, and operational safety.
>
> Current qualified code baseline: **v0.6.0-alpha — M6 Persistence, Metrics & Auditability VERIFIED / CLOSED**. M5 publication evidence is qualified; the local `v0.6.0-alpha` tag exists, while remote tag push/publication remain separate release actions.
>
> Current deployment model: project source repositories under `mcp-server/<project>`, flat Rust MCP executables under `mcp-server/bin`, and non-MCP runtime/config/state under `mcp-server/runtime`.
>
> Target maturity path: **Foundation → Local Control Plane → Runtime Distribution → Fleet-Safe Operations → Telemetry → Release Candidate → Production Grade → Multi-host Scale**.

---

# 1. Product Goal

MCP Studio is a local-first web control plane for operating an MCP stack safely and reproducibly on development hosts and source-less runtime hosts.

Studio must eventually provide one operational surface for:

- MCP lifecycle management;
- secure tunnel lifecycle management;
- persistent MCP registration and configuration;
- project discovery on development hosts;
- installed runtime inventory on source-less hosts;
- release/version visibility;
- safe component updates and rollback;
- fleet desired-state and drift visibility;
- live and historical metrics;
- logs and audit history;
- gateway traffic telemetry where directly observable;
- diagnostics, backup, migration, and recovery.

The production design must explicitly support two host classes.

## 1.1 Development host

A development host may contain both source and installed runtime artifacts:

```text
mcp-server/
├── filesystem/
├── git/
├── exec/
├── gateway/
├── studio/
├── fleet/
├── tunnel-client/        # official OpenAI source checkout, optional for runtime
├── blender/
│
├── bin/                  # flat Rust MCP executables only
│   ├── rust-mcp-filesystem
│   ├── rust-mcp-git
│   ├── rust-mcp-exec
│   ├── rust-mcp-gateway
│   └── rust-mcp-blender
│
└── runtime/              # non-MCP executable/config/state
    ├── gateway/servers.d/
    ├── studio/
    ├── tunnel-client/
    └── fleet/
```

Development hosts may build locally and install artifacts into the runtime layout.

## 1.2 Runtime-only host

A runtime-only host must be able to operate without Rust, Cargo, Node.js, pnpm, source repositories, `target/`, or `node_modules`.

Minimum runtime-only layout:

```text
mcp-server/
├── bin/
│   ├── rust-mcp-filesystem
│   ├── rust-mcp-git
│   ├── rust-mcp-exec
│   └── rust-mcp-gateway
│
└── runtime/
    ├── gateway/servers.d/
    ├── studio/
    │   ├── mcp-studio
    │   ├── studio.toml
    │   └── data/
    ├── tunnel-client/
    │   ├── config.yaml
    │   ├── current -> releases/vX.Y.Z
    │   └── releases/
    └── fleet/
```

A runtime-only host must update from verified release artifacts rather than by pulling and compiling source.

---

# 2. Historical Baseline — v0.4.0

Milestones 0–4 are complete and formed the v0.4.0 implementation baseline. The current qualified code baseline is v0.6.0-alpha as recorded at the top of this roadmap.

## Completed capabilities

### M0 — Foundation & SDLC

- Rust backend architecture and project conventions.
- Threat model and ADR process.
- Test, CI, logging, error-handling, and release conventions.
- Localhost-first security posture.

### M1 — Core Process Supervisor

- MCP start / stop / restart.
- Studio-owned PID safety.
- State, uptime, restart, crash, exit-code tracking.
- stdout/stderr capture and bounded logs.
- graceful stop with forced termination fallback.

### M2 — Web Dashboard

- React/TypeScript UI.
- REST + WebSocket operational state.
- live lifecycle controls and logs.
- reconnect/resync behavior.

### M3 — Secure Tunnel Management

- tunnel start / stop / restart.
- tunnel runtime state and logs.
- constrained executable/config model.
- secret references and log redaction.
- tunnel lifecycle isolated from MCP supervisor lifecycle.

### M4 — Registry, Configuration & Auto-Discovery

- schema-versioned persistent MCP registry.
- metadata-only Rust/Node/Python discovery.
- explicit review/approval before registration.
- structured argv; no shell interpolation.
- path traversal/symlink confinement.
- dynamic registry integration without Studio restart.
- enable/disable/edit/unregister behavior.
- registry/discovery REST API and UI.
- same-origin browser protections.
- flat runtime-root compatibility: multiple MCP registrations may safely share the same confined runtime directory while retaining distinct executable names.

Detailed closure evidence remains in `docs/milestone-4-status.md`.

---

# 3. Source of Truth and Release Model

Studio must not treat arbitrary local files as fleet desired state.

## 3.1 Project-owned components

Project-owned repositories are hosted under the `13thx-mcp` GitHub organization:

```text
13thx-mcp/filesystem
13thx-mcp/git
13thx-mcp/exec
13thx-mcp/gateway
13thx-mcp/studio
13thx-mcp/fleet
```

Each component has an independent semantic version and release lifecycle.

Current release pipeline contract:

```text
tag vX.Y.Z
   ↓
version validation
   ↓
format / lint / clippy / test
   ↓
macOS native builds
   └── darwin-arm64
   ↓
versioned tar.gz artifacts
   ↓
SHA256SUMS.txt
   ↓
GitHub Release
```

Studio releases additionally package the web dashboard required at runtime.

## 3.2 Official tunnel-client

`tunnel-client` is not forked as a product component.

Source and release authority:

```text
https://github.com/openai/tunnel-client
```

Studio/Fleet must consume official OpenAI releases and must support:

- OS/architecture detection;
- installed-version detection;
- latest-release discovery;
- matching release-asset selection;
- SHA-256 verification;
- extracted binary version verification;
- versioned install directories;
- atomic `current` activation;
- rollback to a previous verified release;
- preservation of host-local `config.yaml` and credentials.

## 3.3 Synchronization boundary

For development/source synchronization, the publication boundary is a Git commit pushed to the authoritative repository.

For runtime synchronization, the deployment boundary is a published, verified release artifact.

Uncommitted source changes and locally built unversioned artifacts are never fleet desired state.

---

# 4. SDLC Model

MCP Studio follows an iterative SDLC with an explicit quality gate at every milestone.

```text
Plan
  ↓
Requirements
  ↓
Architecture / Design
  ↓
Implementation
  ↓
Verification
  ↓
Security Review
  ↓
Release
  ↓
Operate / Observe
  ↓
Feedback → next milestone
```

## 4.1 Planning

- Define scope and exclusions.
- Identify dependencies.
- Record risks and assumptions.
- Define testable acceptance criteria.
- Identify migration and rollback requirements.
- Identify source-host versus runtime-only-host behavior.

## 4.2 Requirements

- Functional requirements documented.
- Non-functional requirements documented.
- API behavior defined where applicable.
- Error/recovery states defined.
- Security boundaries identified.
- release/update compatibility rules identified.

## 4.3 Design

- Architecture changes reviewed before implementation.
- Data model changes documented.
- Public API changes documented.
- Process/update state-machine changes documented.
- Threat-model impact reviewed for privileged operations.
- Long-term decisions recorded as ADRs under `docs/adr/`.

## 4.4 Implementation

- Small modules with explicit responsibilities.
- No secrets embedded in source or committed config.
- Structured errors instead of silent failure.
- Backward-compatible config changes where feasible.
- Feature flags for incomplete/risky operational features.
- No raw arbitrary command execution from the browser.

## 4.5 Verification

Minimum verification layers:

- unit tests;
- integration tests;
- API tests;
- process lifecycle tests;
- release/update tests;
- failure/recovery tests;
- UI smoke tests.

Production-sensitive changes additionally require:

- security tests;
- restart/recovery tests;
- upgrade/migration tests;
- rollback tests;
- checksum/tamper tests;
- architecture-selection tests;
- load/soak tests where applicable.

## 4.6 Release

Every releasable milestone must provide:

- semantic version;
- changelog entry;
- release notes;
- versioned artifacts;
- checksums;
- migration notes when required;
- rollback procedure;
- known limitations.

---

# 5. Definition of Ready

A milestone is ready for implementation when:

- scope is defined;
- acceptance criteria are testable;
- architecture decisions are resolved;
- dependencies are known;
- security implications are identified;
- runtime-only-host implications are identified;
- migration/rollback behavior is defined;
- out-of-scope items are explicit.

---

# 6. Definition of Done

A milestone is complete only when:

- all acceptance criteria pass;
- new behavior has tests;
- existing tests remain green;
- no known Critical or High severity security issue remains unresolved;
- documentation reflects the implemented behavior;
- config/data migrations are tested where applicable;
- release/update migration is tested where applicable;
- manual smoke succeeds on a development host;
- relevant runtime-only smoke succeeds without source dependencies;
- rollback path is tested or explicitly documented as unsupported;
- published artifact can be recreated from source.

---

# 7. Target Architecture

```text
                           GitHub
        ┌────────────────────┴────────────────────┐
        │                                         │
  13thx-mcp releases                    openai/tunnel-client releases
        │                                         │
        └────────────────────┬────────────────────┘
                             │ verified artifacts
                             ▼
┌──────────────────────── MCP Studio ──────────────────────────────┐
│                                                                 │
│ Web UI / REST / WebSocket                                       │
│                                                                 │
│ ┌──────────────┐ ┌───────────────┐ ┌─────────────────────────┐ │
│ │ MCP Registry │ │ MCP Supervisor│ │ Release / Update Manager│ │
│ └──────┬───────┘ └───────┬───────┘ └────────────┬────────────┘ │
│        │                 │                      │              │
│ ┌──────▼──────┐   ┌──────▼──────┐      ┌────────▼────────┐    │
│ │ Discovery   │   │ Logs/Metrics│      │ Fleet State/Drift│    │
│ └─────────────┘   └──────┬──────┘      └─────────────────┘    │
│                          │                                     │
│ ┌────────────────┐  ┌────▼────┐                                │
│ │ Tunnel Manager │  │ SQLite  │                                │
│ └───────┬────────┘  └─────────┘                                │
└─────────┼───────────────────────────────────────────────────────┘
          │
          ▼
  runtime/tunnel-client

          Runtime execution layout

mcp-server/bin/
├── rust-mcp-filesystem
├── rust-mcp-git
├── rust-mcp-exec
├── rust-mcp-gateway
└── rust-mcp-blender

mcp-server/runtime/
├── gateway/servers.d/
├── studio/
├── tunnel-client/
└── fleet/
```

Technology baseline:

- backend: Rust;
- async runtime: Tokio;
- HTTP/WebSocket: Axum;
- frontend: React + TypeScript + Vite;
- persistence target: SQLite;
- local bind default: `127.0.0.1`;
- MCP child transport: stdio;
- project-owned release source: `13thx-mcp/*` GitHub Releases;
- tunnel release source: official `openai/tunnel-client` GitHub Releases.

---

# 8. Milestone Roadmap

---

## Milestones 0–4 — COMPLETE

**Historical released baseline:** `v0.4.0`

See Section 2 and milestone status documents for closure evidence.

---

## Milestone 5 — Runtime Distribution, Update Manager & Fleet State

**Target:** `v0.5.0`

**Implementation status:** **COMPLETE / PUBLICATION QUALIFIED.** M5.1–M5.12 implementation and local/source-less fault verification passed on 2026-09-19; the independent darwin-arm64 publication qualification subsequently passed on 2026-09-21 with published project artifacts, source-less bootstrap, and a Fleet 0.2.0 → 0.2.1 transition. See `docs/milestone-5-status.md`.

### Goal

Make Studio capable of safely operating and updating development hosts and source-less runtime hosts from verified release artifacts.

This milestone is the immediate priority because runtime-only hosts must not require source checkout or local compilation.

### 5.1 Component Inventory

Studio must expose installed/current/available identity for:

- Filesystem MCP;
- Git MCP;
- Exec MCP;
- Gateway MCP;
- Blender MCP when managed;
- Studio itself;
- Fleet control bundle;
- official tunnel-client.

Inventory data should include where applicable:

```text
component id
component class: mcp | service | control-bundle | upstream-runtime
installed version
installed artifact checksum
installed architecture
installed path
release source
latest available version
selected update channel
update available
runtime state
last update result
last successful update timestamp
```

### 5.2 Release Providers

Implement provider abstractions for at least:

#### 13thx GitHub Release Provider

Supports project-owned components from `13thx-mcp/*`.

Required behavior:

- query latest/stable or configured version;
- select correct OS/architecture asset;
- fetch `SHA256SUMS.txt`;
- verify downloaded archive;
- reject missing/ambiguous/checksum-mismatched assets;
- expose release metadata to Studio without executing artifact contents.

#### OpenAI tunnel-client Provider

Supports official `openai/tunnel-client` runtime releases with the same integrity guarantees plus extracted-binary version validation.

### 5.3 Platform Resolution

Minimum initial runtime targets:

```text
darwin-arm64
```

Platform resolution must normalize common host values safely:

```text
arm64 / aarch64 → arm64
Darwin → darwin
```

Unsupported OS/architecture must fail closed rather than guessing.

### 5.4 Update State Machine

Updates must use an explicit state machine:

```text
IDLE
  ↓ check
CHECKING
  ↓ newer version
AVAILABLE
  ↓ operator/policy approval
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

Failure paths:

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

No failed verification may modify the active executable.

### 5.5 Safe MCP Update Transaction

For a managed MCP:

```text
check release
→ download
→ checksum verify
→ extract/stage
→ verify expected executable
→ stop target MCP if running
→ atomic binary replacement in flat bin root
→ start target if previously running
→ protocol/health verification
→ persist update result
```

Only the affected MCP should restart.

A failed post-activation verification must trigger rollback to the last-known-good binary when rollback data exists.

### 5.6 Gateway Update Transaction

Gateway update must account for the fact that changing Gateway can interrupt the control path used by connected clients.

Required design:

- stage/verify before stopping Gateway;
- preserve generated `runtime/gateway/servers.d`;
- activate only after artifact verification;
- restart/reconnect Gateway deterministically;
- verify child catalog restoration;
- confirm expected child count/tool catalog or configured health criteria;
- expose transient reconnect state distinctly from failure.

### 5.7 Studio Self-Update

Studio must not rely on killing and replacing itself from inside the same process without an external supervisor contract.

Required design:

- Studio downloads/verifies/stages its own release;
- records `restart-required` / pending activation state;
- an external launcher/service manager performs process replacement;
- startup finalizes and health-verifies the pending update;
- failed startup can revert to last-known-good Studio release.

Initial macOS integration should define a `launchd`-compatible service contract.

Studio release activation must preserve:

- `runtime/studio/studio.toml`;
- registry/database state;
- logs according to policy;
- runtime secrets;
- packaged `web/dist` matching the backend release.

### 5.8 Fleet Control Bundle Update

Fleet update must not overwrite host-local profiles blindly.

Required behavior:

- update versioned fleet tooling/manifest templates;
- preserve active host-local `hosts/<host>.toml`;
- support schema-version validation/migration;
- validate rendered Gateway/Studio/tunnel config before activation.

### 5.9 Tunnel Update

Promote the existing official tunnel update behavior into Studio:

- installed-version display;
- latest-version display;
- update available state;
- OS/arch asset selection;
- SHA-256 verification;
- binary version verification;
- versioned install directories;
- atomic `current` switch;
- previous-version rollback;
- preserve `config.yaml` and credentials;
- restart only when required/approved.

### 5.10 Runtime Configuration Reconciliation & Catalog Repair

Generated runtime configuration is a first-class desired-state surface, not incidental host state.

Studio must compare Fleet desired render with the active runtime and live Gateway catalog for at least:

- `runtime/gateway/servers.d/*.yaml`;
- `runtime/tunnel-client/config.yaml`;
- Fleet-generated Studio runtime config;
- Gateway child/tool catalog identity.

Required behavior:

- detect stale source-tree, `target/release`, nested-bin, or otherwise non-canonical runtime paths;
- detect split launch paths where a helper/CLI override masks stale canonical config;
- distinguish safe Fleet-managed drift from unknown/unmanaged local edits;
- plan repair before mutation and snapshot rollback bytes/hashes;
- rewrite only config proven to be Fleet-owned and safe to reconcile;
- reload Gateway only when Gateway config changed;
- restart tunnel only when its binding/config changed and preserve prior running/stopped state;
- verify the exact expected child/tool-name set after reload/reconnect, not only an aggregate tool count;
- expose a stable catalog fingerprint/generation for diagnostics;
- keep local Gateway synchronization distinct from connected-client catalog freshness when the remote client provides no refresh acknowledgement;
- roll back generated config and runtime ownership state if post-repair verification fails.

M5.9A defines the transaction and startup drift detection. It requires a Fleet-authored side-effect-free pure-render contract, a persisted last-known-managed fingerprint/generation to distinguish managed-safe drift from unmanaged edits, and explicit validate-only handling for launcher surfaces that Fleet does not own. Periodic unattended reconciliation, retry/backoff and drift-loop circuit breaking belong to M7.

> Implementation note: this roadmap subsection is implemented by the `M5.9A` submission in the M5 session index; roadmap subsection numbering and submission numbering are not 1:1.

### 5.11 Fleet State and Drift

Studio must distinguish:

```text
Desired version
Installed version
Running version
```

These may temporarily differ during update/restart operations.

Drift states should include:

```text
CURRENT
UPDATE_AVAILABLE
INSTALLED_RESTART_REQUIRED
DRIFTED
UNKNOWN
BROKEN
```

Studio must never claim synchronized fleet state based only on source Git HEAD.

### 5.12 UI

Add an Updates/Fleet page showing:

- host identity;
- platform/architecture;
- component current/available versions;
- release source;
- checksum/integrity status;
- source-present versus runtime-only host mode;
- update state/progress;
- last result;
- restart requirement;
- drift state;
- manual update action;
- rollback action where safe.

### 5.13 API

Minimum API direction:

```text
GET  /api/updates
GET  /api/updates/{component}
POST /api/updates/check
POST /api/updates/{component}/prepare
POST /api/updates/{component}/apply
POST /api/updates/{component}/rollback

GET  /api/fleet
GET  /api/fleet/drift
```

Exact endpoint naming may change during design review.

### 5.14 Security Requirements

- HTTPS only for remote release retrieval.
- SHA-256 verification before extraction/activation.
- safe archive extraction with traversal rejection.
- no executable activation before integrity and structure validation.
- no browser-supplied arbitrary release URL/path.
- release repositories/component IDs must come from server-side policy.
- downgrade requires explicit policy/operator intent.
- secrets/config remain outside release archives.
- runtime update logs must redact tokens/headers.
- preserve least-privilege filesystem ownership.

### 5.15 Verification

- correct arm64 selection and Intel-host rejection;
- no matching artifact;
- malformed release metadata;
- checksum mismatch;
- path traversal in archive;
- truncated archive;
- expected executable missing;
- wrong binary version;
- MCP stop/update/restart success;
- update of inactive MCP remains inactive;
- rollback after failed health verification;
- Gateway update/reconnect;
- Studio staged self-update contract;
- Fleet host-local config preservation;
- tunnel update preserving local config;
- stale Fleet-generated Gateway/tunnel config detection and transactional repair;
- split launch-path detection where a correct override masks stale canonical config;
- exact Gateway child/tool-set restoration and catalog fingerprint verification;
- rollback after failed config reconciliation;
- complete runtime-only-host smoke with no source tree/toolchain.

### Exit Criteria

A source-less macOS host can be installed, inspected, updated, reconciled, restarted, and rolled back using only verified published artifacts plus host-local runtime configuration. Fleet-managed generated runtime drift can be detected without source checkouts and safely repaired when ownership is proven.

---

## Milestone 6 — Persistence, Metrics & Auditability

**Target:** `v0.6.0-alpha`

**Implementation status (2026-09-21):** **VERIFIED / CLOSED.** M6.0–M6.8 passed full clean-source native darwin-arm64 qualification, including storage-fault, restart/recovery, source-less/runtime-only, retention, package/rollback and M5 regression gates. Independent M5 publication evidence was qualified and consumed by the closure runner. See `docs/milestone-6-status.md`.

### Goal

Persist operational history, update history, audit data, and useful metrics across Studio restarts.

### Persistence

Introduce SQLite for:

- historical registry revision metadata only; the file-backed registry remains authoritative and is not migrated to SQLite;
- runtime sessions;
- process events;
- tunnel events;
- update attempts;
- artifact/install records;
- active/previous release identity;
- fleet drift observations;
- configuration revisions;
- audit events;
- metrics buckets.

### Process Metrics

- current state;
- PID;
- uptime;
- total launches;
- restart count;
- crash count;
- last exit code;
- last start/stop;
- average session duration.

### Update Metrics

- update checks;
- updates available;
- attempted installs;
- successful installs;
- failed installs;
- rollback count;
- update duration;
- last-known-good version.

### Audit Events

Record at least:

- MCP start/stop/restart;
- tunnel start/stop/restart;
- registry change;
- discovery approval;
- config change;
- update check;
- update prepare/apply;
- rollback;
- fleet drift resolution;
- Studio self-update request/finalization.

### UI

- historical charts;
- event timeline;
- update history;
- error/restart trends;
- fleet drift history.

### Exit Criteria

Operational and update history survives restart and can explain how the currently running artifacts/configuration were reached.

---

## Milestone 7 — Gateway Coordination, Concurrency & Tool Safety

**Target:** `v0.7.0-beta`

### Goal

Turn Gateway from a routing/aggregation layer into a bounded, observable request-coordination boundary for all MCP traffic before unattended runtime automation is enabled.

M7 must preserve the existing typed MCP model. It must **not** replace Filesystem/Git/Exec or other domain MCPs with a broad general-purpose worker, and it must not make chat/session identity an authorization boundary.

Target flow:

```text
ChatGPT / Codex / local MCP client
              │
              ▼
      Secure MCP Tunnel
              │
              ▼
┌───────────────────────────────────────┐
│ rust-mcp-gateway                      │
│                                       │
│ Request Coordination Layer            │
│ ├── admission / policy                │
│ ├── request identity / deadline       │
│ ├── bounded queue / concurrency       │
│ ├── cancellation / outcome semantics  │
│ ├── graceful drain                    │
│ ├── restart backoff / circuit breaker │
│ ├── payload budget                    │
│ ├── tool profiles                     │
│ └── telemetry / correlation           │
└──────────┬──────────┬──────────┬──────┘
           │          │          │
      filesystem     git        exec       ...
           │          │          │
           └──── typed capability MCPs ────┘
```

Studio remains the operator/control-plane surface. M6 SQLite remains historical evidence and must never become live request authority.

### 7.1 Request Lifecycle and Outcome Semantics

Gateway must distinguish request state before and after dispatch.

Minimum state model:

```text
RECEIVED
  ↓
ADMITTED
  ↓
QUEUED
  ↓
DISPATCHED
  ↓
COMPLETED
```

Required terminal/error distinctions:

```text
CANCELLED_BEFORE_DISPATCH
DEADLINE_EXPIRED_BEFORE_DISPATCH
OUTCOME_PENDING
OUTCOME_UNKNOWN
CHILD_ERROR
RESPONSE_REJECTED
```

Semantics:

- cancellation/deadline before dispatch must guarantee the child tool was not called;
- cancellation/deadline after dispatch must not claim that a side effect was undone;
- a caller disconnect after dispatch must retain enough local state to classify the result as pending or unknown rather than encouraging blind retry;
- mutation failures with unknown outcome must return explicit retry guidance;
- Gateway must never automatically replay a failed, expired, disconnected, or unknown-outcome mutation;
- request/correlation IDs must be generated or normalized server-side and must not be treated as authorization input.

Where the MCP transport supports cancellation/progress, Gateway should propagate it while preserving the outcome distinction above.

### 7.2 Active Request Registry and Graceful Drain

Gateway must maintain an in-memory active-request registry with enough metadata to support safe drain, status, and telemetry without persisting tool arguments by default.

Minimum tracked fields:

```text
request_id
correlation_id
child
tool
class: read | mutation | long-running | control
received_at
queued_at
dispatched_at
completed_at
deadline
state
```

Introduce explicit Gateway drain behavior:

```text
RUNNING
  ↓
DRAINING
  ↓
DRAINED
  ↓
RESTARTING / UPDATING
  ↓
RUNNING
```

Drain requirements:

- reject or defer new mutations once draining begins;
- optionally allow explicitly classified safe reads while policy permits;
- wait for dispatched mutations to reach known terminal state before destructive restart/update;
- expose active count, queued count, oldest request age, and drain reason;
- time-bounded drain must fail closed rather than killing work silently;
- Gateway reload/update, child restart, and Studio/Fleet activation paths must use the drain contract where they could interrupt active work.

### 7.3 Bounded Concurrency and Backpressure

Gateway must enforce bounded resource use independently of the Secure MCP Tunnel transport.

Support:

- global active-call limit;
- global bounded queue;
- per-child concurrency limit;
- optional per-tool concurrency limit;
- queue wait timeout/deadline;
- queue-depth and wait-time telemetry;
- deterministic rejection when capacity policy is exceeded.

Initial policy should allow different limits by risk profile, for example:

```text
filesystem/read      high concurrency
git/inspection       moderate concurrency
git/mutation         low concurrency
exec                 low bounded concurrency
hardware/HIL         serialized or explicitly bounded
release/update       single-flight
```

Concurrency policy must be configuration-driven and must not silently broaden capability.

### 7.4 Child Runtime Resilience

Extend the current `restart.policy = on-failure` model with bounded restart control:

```yaml
restart:
  policy: on-failure
  max_attempts: 3
  backoff:
    initial_ms: 500
    max_ms: 30000
  circuit_breaker:
    threshold: 5
    cooldown_ms: 60000
```

Required behavior:

- exponential backoff with an upper bound;
- consecutive-failure tracking;
- crash-loop detection;
- circuit-open state with `retry_at`;
- no unbounded restart loop;
- active/dispatched work blocks unsafe manual restart;
- child failure remains isolated from unrelated children where possible;
- recovery does not fabricate success if initialization/tool-catalog refresh fails.

Expose at least:

```text
healthy
degraded
circuit_open
restart_count
consecutive_failures
last_failure
retry_at
```

### 7.5 Filesystem v2 — Efficient Reads, CAS and Patch Safety

Keep Filesystem MCP capability-confined and typed. Do not replace it with a general shell/file worker.

Add at least:

```text
read_text_file_range
search_text
file_metadata
patch_text_file
```

Introduce content revision identity, preferably SHA-256 for bounded text files:

```text
read_text_file(...)
→ content
→ revision: sha256:<digest>
```

Mutations should support optimistic concurrency:

```text
patch_text_file(
  path,
  expected_revision,
  ...
)
```

and/or:

```text
write_text_file(
  path,
  content,
  expected_revision
)
```

If the current file differs from the expected revision, fail with an explicit stale-write conflict rather than overwriting.

Design requirements:

- CAS/revision is the correctness mechanism;
- optional in-process file locks may coordinate concurrent MCP requests but are not a filesystem security boundary;
- canonical root confinement remains mandatory;
- external edits by users, IDEs, generators, or other processes must be detectable through revision mismatch;
- patch operations must be atomic where practical;
- range/search APIs must be bounded to reduce unnecessary full-file ingestion and token cost.

### 7.6 Payload Budget and Oversized Result Handling

Gateway must apply transport-independent request/response budgets before large MCP results reach the client context.

Support bounded limits for:

```text
request bytes
response bytes
structured content bytes
text preview bytes
binary/base64 content
```

For oversized results:

- do not forward the full payload by default;
- return bounded metadata and a useful preview;
- provide a narrower retry recommendation such as range/filter/pagination;
- optionally spill the full result to a local ephemeral artifact referenced by opaque artifact ID/content hash;
- artifacts must have TTL, total disk budget, bounded retrieval, and cleanup;
- raw sensitive payload archival must be opt-in, not the default;
- M6 history should record metadata such as original/forwarded size and guard action, not full payload content.

### 7.7 Tool Profiles and Capability Surface

Build on the existing Gateway child `tool_allowlist` and introduce named operator policy profiles.

Initial profile direction:

```text
inspect
develop
release
ops
hardware
```

Example intent:

- `inspect`: read/search/status/diff/log/diagnostic tools;
- `develop`: adds confined file mutation, staging/commit, and approved execution;
- `release`: adds guarded history normalization, merge/tag/publication checks;
- `ops`: adds Gateway/Studio/Fleet control-plane operations;
- `hardware`: exposes explicitly approved GPIO/reader/belt/HIL operations.

Requirements:

- profile selection must be server/operator policy, not arbitrary untrusted request authority;
- hidden tools remain implemented but non-routable through the active profile;
- control-plane mutation tools such as Gateway reload/enable-disable should not be part of the normal default coding surface;
- `workspace_execute` remains explicit high-trust development execution and should not be required for read-only inspection workflows;
- profile changes emit tool-list refresh notifications and are auditable.

### 7.8 Workspace Context and Alias Convenience

A workspace registry may provide names/aliases for repeated project selection:

```text
workspace_register
workspace_list
workspace_bind
workspace_current
```

Rules:

- workspace/session context is convenience, not authorization;
- every resolved path must still pass the underlying MCP root/path policy;
- explicit tool arguments remain authoritative for the actual operation;
- ambiguous aliases fail closed;
- session metadata may help retain convenience binding across reconnects but must not grant additional filesystem/process capability;
- implementation must not require one child process per chat for otherwise stateless typed MCPs.

### 7.9 Gateway Resources and Progress Forwarding

Expand Gateway beyond tools where protocol support is stable.

Support aggregation/forwarding for:

```text
resources/list
resources/read
notifications/progress
```

Potential local read-only resources include:

```text
gateway://status
workspace://current
workspace://repositories
studio://runtime-inventory
hardware://inventory
```

Requirements:

- resources remain bounded and read-oriented;
- resource aggregation must preserve child identity and collision handling;
- long-running Exec/HIL/tool operations should forward meaningful progress when the child provides it;
- progress is informational and must never be mistaken for committed operation state.

### 7.10 Gateway Telemetry into M6 History

Integrate real Gateway-observed request metadata into Studio/M6 history.

Record bounded metadata such as:

- request count;
- success/error count;
- queue depth and wait time;
- active calls;
- request latency;
- p50/p95 latency;
- timeout/cancellation counts;
- outcome-pending/outcome-unknown counts;
- child/tool identity where policy permits;
- response-size and payload-guard metrics;
- restart/circuit-breaker events;
- catalog generation / exposed-tool fingerprint.

Privacy requirements:

- do not persist tool arguments by default;
- do not persist complete tool results by default;
- do not persist arbitrary MCP payloads by default;
- correlation IDs must not encode secret/user content;
- sensitive payload debugging requires explicit opt-in with bounded retention.

### 7.11 Secure MCP Tunnel Native Runtime Adapter

Treat official `openai/tunnel-client` as the transport/runtime substrate rather than duplicating all of its process/runtime semantics inside Studio.

Add a constrained adapter over supported upstream interfaces such as:

```text
doctor
runtimes connect
runtimes status --json
runtimes stop
health/readiness/metrics
control-plane poll health
```

Boundary:

**Official tunnel-client owns**

- tunnel protocol transport;
- runtime/profile semantics;
- local health/readiness;
- control-plane polling;
- supported runtime process management.

**Studio/Fleet retain authority for**

- desired version;
- verified release discovery/staging;
- transactional update/rollback;
- installed/running identity;
- fleet drift;
- operator policy;
- historical audit.

Do not regress M5 transactional update/rollback guarantees when delegating runtime lifecycle details upstream.

Before freezing this adapter contract, synchronize and review the current official tunnel-client revision rather than designing against a stale checkout.

### 7.12 Verification

Permanent tests must cover at least:

- cancellation before dispatch proves no child call occurred;
- timeout after dispatch reports pending/unknown outcome without replay;
- caller disconnect during mutation;
- drain with active reads/mutations;
- restart/update rejected or deferred while unsafe work remains;
- global/per-child queue saturation;
- queue timeout and fairness;
- crash-loop backoff and circuit-open behavior;
- child recovery without affecting healthy siblings;
- stale external file edit rejected by CAS;
- concurrent patch conflict;
- bounded range/search behavior;
- oversized text/structured/binary response guarding;
- ephemeral artifact TTL/disk bound;
- profile tool-surface correctness and `tools/list_changed`;
- workspace alias ambiguity and confinement;
- resource aggregation/collision handling;
- progress forwarding;
- M6 telemetry persistence without payload persistence;
- Tunnel adapter health/readiness/poll-health distinction;
- long-running concurrency/soak and failure injection.

### Exit Criteria

Gateway is a bounded coordination boundary rather than only a router:

- mutating calls cannot be silently replayed after timeout/disconnect;
- updates/restarts can drain active work safely;
- queue/concurrency/restart behavior is bounded;
- file mutations detect stale external changes;
- oversized responses cannot flood client context unchecked;
- normal tool surfaces are least-privilege profiles;
- request metadata is observable through M6 without storing payloads by default;
- typed domain MCPs remain the capability/security boundary;
- official Secure MCP Tunnel remains the transport/runtime substrate.

---

## Milestone 8 — Hardening, Auto-Update Policy & Recovery

**Target:** `v0.8.0-beta`

### Goal

Use the M7 coordination/drain/health primitives to make local unattended operation safe enough for regular runtime-host use.

Automatic update/reconciliation must not be enabled before the M7 request-coordination and drain contracts are verified.

### Runtime Reliability

- configurable auto-restart;
- Gateway/child exponential restart backoff and circuit-breaker policy built on M7 primitives;
- health-check abstraction;
- partial component failure isolation;
- Studio restart reconciliation;
- startup and periodic Fleet-managed runtime-config reconciliation;
- bounded reconcile retry/backoff and drift-loop circuit breaker;
- unmanaged/local config conflicts never auto-overwritten;
- tunnel failure isolation;
- unattended actions respect Gateway drain/active-request state.

### Update Policies

Supported initial policies:

```text
manual
notify-only
auto-prepare
auto-update-safe
```

`auto-update-safe` must require all safety preconditions:

- trusted configured release provider;
- newer allowed semantic version;
- valid checksum;
- supported platform;
- no conflicting active update;
- rollback material available where required;
- component-specific restart policy permits activation;
- required Gateway/child drain has completed;
- no unknown-outcome mutation blocks safe activation.

Auto-update must never auto-resolve source Git conflicts.

### Update / Reconciliation Scheduling

Support:

- manual update check;
- startup update check;
- periodic update check;
- startup runtime-drift check;
- periodic runtime-drift check;
- maintenance window;
- deferred restart where component semantics allow it.

Safe automatic reconciliation may use the M5.9A primitive only for a proven managed-safe drift class. Unknown edits, secret-bearing ambiguity, repeated failed repair, active unsafe work, unknown request outcome, or rollback uncertainty must stop automatic mutation and require operator action.

### Recovery

- config backup before mutation;
- safe config rollback;
- binary last-known-good rollback;
- failed activation recovery;
- stale update-lock recovery;
- interrupted download/staging cleanup;
- diagnostics bundle without secrets;
- recovery after Studio/Gateway restart while request history contains pending/unknown outcomes;
- safe operator workflow to resolve an unknown mutation outcome before retry.

### Verification

- repeated crash/restart tests;
- interrupted update fault injection;
- update/restart while Gateway is busy;
- failed/expired drain;
- disk-full behavior;
- corrupted prior release;
- rollback failure handling;
- Studio forced restart during update state transitions;
- repeated reconciliation failure/circuit-open behavior;
- long-running runtime-host soak.

### Exit Criteria

A runtime host can operate with safe periodic update checking and optional constrained automatic updates without entering uncontrolled restart/update loops, interrupting active mutations silently, or replaying operations with unknown outcomes.

---


## Milestone 9 — Release Candidate: Security, Upgrade Safety & Operations

**Target:** `v0.9.0-rc`

### Goal

Prepare the complete control plane for production-grade deployment.

### Security

- formal threat-model review;
- release supply-chain review;
- dependency vulnerability scans;
- artifact checksum enforcement review;
- provenance/signing strategy decision;
- authentication for non-localhost deployments;
- RBAC if multi-user operation is supported;
- CSRF/CORS/secure-header review;
- secret-at-rest strategy;
- privileged API rate limiting;
- audit trail integrity review.

### Upgrade Safety

- supported-version upgrade matrix;
- rollback matrix;
- database migration backup/restore;
- config schema migration/rollback;
- component dependency compatibility checks;
- disk-space preflight;
- release retention policy;
- old-artifact cleanup policy.

### Operations

- graceful Studio shutdown;
- child process reconciliation;
- log rotation;
- metrics retention;
- disk usage protections;
- runtime backup/restore;
- diagnostics export;
- launchd installation/service documentation;
- host bootstrap/update/rollback guides.

### Performance

- API load test;
- WebSocket fan-out test;
- multi-MCP concurrency test;
- Gateway throughput/latency test;
- long-running soak;
- memory leak observation;
- database growth test.

### Compatibility

Initial required production matrix:

```text
macOS / Apple Silicon (arm64)
```

Linux support may be added only with explicit artifacts, CI coverage, and operational testing.

### Exit Criteria

- RC security review passes;
- migration/rollback tests pass;
- runtime-only upgrade tests pass across supported macOS architectures;
- no unresolved Critical/High security finding;
- no blocker defect;
- soak/load targets pass.

---

## Milestone 10 — Production Grade v1.0

**Target:** `v1.0.0`

### Goal

Provide a stable, supportable MCP control plane suitable for daily development-host and runtime-host operation.

### Required Capabilities

#### MCP Lifecycle

- start/stop/restart;
- health state;
- auto-restart/backoff;
- crash-loop protection;
- process ownership safety.

#### Tunnel Lifecycle

- start/stop/restart;
- state/reconnect monitoring;
- official release update/rollback;
- safe secret handling.

#### Registry & Configuration

- persistent registry;
- development-host discovery;
- explicit registration approval;
- runtime-only installed inventory;
- config validation/versioning;
- safe rollback.

#### Runtime Distribution

- verified project-owned release artifacts;
- verified official tunnel release artifacts;
- platform selection;
- installed/running/desired version tracking;
- manual safe update;
- optional constrained auto-update policy;
- component rollback;
- Studio self-update via external supervisor contract.

#### Fleet State

- host identity;
- runtime architecture;
- installed component versions;
- desired versions;
- drift state;
- last update results;
- diagnostics suitable for comparing Aira/Mirin-style hosts without requiring source checkout.

#### Observability

- live status/logs;
- historical process/update metrics;
- audit events;
- real Gateway traffic metrics when Gateway telemetry is enabled.

#### Security

- localhost-safe defaults;
- release integrity verification;
- strong authentication for remote mode;
- authorization where multi-user operation exists;
- no raw arbitrary browser command execution;
- secret redaction;
- path confinement;
- archive traversal protection;
- auditability.

#### Operations

- backup/restore;
- upgrade/rollback documentation;
- log/metric retention;
- health/readiness endpoints;
- diagnostics export;
- defined support matrix;
- runtime-only bootstrap procedure.

### Production SLO Targets

Initial targets to validate during RC testing:

- Studio control-plane availability `>= 99.9%` when host is healthy;
- no registry/config loss during clean restart;
- MCP crash recovery according to policy;
- no activation of artifacts failing integrity checks;
- failed component update recovers to known-good state when rollback is supported;
- control API p95 below an agreed local threshold under expected load;
- no known Critical/High severity vulnerability at release.

### Production Release Gate

- functional acceptance tests pass;
- security review passes;
- migration/rollback tests pass;
- runtime-only update test passes;
- backup/restore test passes;
- soak test passes;
- operational documentation complete;
- known limitations documented;
- release artifacts reproducible from source.

---

# 9. Post-v1.0 Roadmap

## v1.1 — Plugin / Runtime Adapter System

- custom runtime adapters;
- MCP templates;
- custom health checks;
- event hooks;
- alternate artifact providers subject to explicit trust policy.

## v1.2 — Multi-host Agents & Central Fleet View

Local per-host Studio remains authoritative for privileged local process/update actions.

Potential central features:

- authenticated remote Studio agents;
- host inventory;
- consolidated drift/update status;
- rollout groups;
- staged/canary rollout;
- remote update approval;
- mutual authentication;
- host health aggregation.

Central control must not require a shared writable filesystem and must avoid split-brain update ownership.

## v1.3 — Advanced Policy

- per-MCP permissions;
- per-tool allow/deny rules;
- release channels;
- component version constraints;
- maintenance-window policy;
- session policies;
- resource quotas.

## v1.4 — Advanced Observability

- OpenTelemetry export;
- Prometheus metrics;
- external log sinks;
- distributed tracing for Gateway traffic;
- cross-host fleet update/health dashboards.

---

# 10. Testing Strategy by Layer

| Layer | Purpose |
|---|---|
| Unit | State machines, parsing, validation, version/platform normalization, checksum logic, redaction |
| Integration | Supervisor, registry, update manager, tunnel, filesystem activation, persistence |
| API | HTTP/WebSocket contract, update operations, conflicts and errors |
| UI | Lifecycle, update, rollback, fleet/drift workflows |
| Release | Artifact naming, architecture matrix, checksum manifests, package contents |
| Security | Traversal, archive extraction, secret handling, command restrictions, release-source policy |
| Failure Injection | Crash/restart, interrupted update, failed activation, rollback |
| Migration | Config/DB/runtime layout upgrades and rollback |
| Runtime-only | Full operation/update without source tree or build toolchain |
| Load | API/Gateway concurrency and resource use |
| Soak | Leaks, stale state, repeated update checks/restarts, long-running degradation |

---

# 11. Security Threat Areas

The threat model must explicitly track at least:

1. arbitrary process execution;
2. command/argument injection;
3. path traversal;
4. symlink escape;
5. malicious archive traversal;
6. release artifact tampering;
7. malicious/incorrect release metadata;
8. wrong architecture artifact activation;
9. downgrade/replay attacks within supported release semantics;
10. secret leakage through API/logs/UI/update tooling;
11. unauthorized process/update control;
12. tunnel accidental public exposure;
13. malicious auto-discovered project manifests;
14. crash-loop resource exhaustion;
15. update/restart loop exhaustion;
16. disk exhaustion through releases/logs/metrics;
17. database tampering/corruption;
18. dependency/supply-chain compromise;
19. CSRF/cross-origin attacks in browser control paths;
20. privilege escalation through MCP/update configuration;
21. compromised external launcher/self-update path;
22. central multi-host split-brain or credential compromise when that capability is introduced.

---

# 12. Configuration Principles

- config has explicit schema version where persisted/shared;
- unknown critical fields fail safely;
- secrets are referenced, not embedded where possible;
- browser never receives stored secret values unless explicitly required by a reviewed design;
- config changes validate before activation;
- generated runtime config remains outside source repositories;
- production config changes support rollback;
- host-local config is preserved across component/fleet updates;
- source root, flat MCP bin root, and non-MCP runtime root remain distinct concepts.

---

# 13. Logging Principles

Logs should include where applicable:

- timestamp;
- host id;
- component;
- component version;
- MCP id;
- process id;
- event type;
- severity;
- update transaction id;
- request/session id when safe.

Logs must not contain:

- API tokens;
- tunnel credentials;
- authorization headers;
- private environment values;
- full MCP payloads by default;
- secret-bearing release request headers.

---

# 14. Database Migration Policy

When SQLite is introduced:

- every schema change has a numbered migration;
- migration tested from each supported previous release;
- backups created before destructive migrations where practical;
- failed migration cannot silently continue;
- Studio records schema and application version compatibility;
- production releases document downgrade support explicitly;
- update activation cannot irreversibly migrate state before rollback policy is satisfied.

---

# 15. Release Versioning

Current/target progression:

```text
v0.0.x        Foundation                           COMPLETE
v0.1.0        Core Supervisor MVP                  COMPLETE
v0.2.0        Web Dashboard MVP                    COMPLETE
v0.3.0        Tunnel Management                    COMPLETE
v0.4.0        Registry + Discovery                 COMPLETE
v0.5.0        Runtime Distribution + Update Mgr    COMPLETE / PUBLICATION QUALIFIED
v0.6.0-alpha  Persistence + Metrics + Audit          VERIFIED / CLOSED
v0.7.0-beta   Gateway Coordination + Tool Safety
v0.8.0-beta   Hardening + Auto-Update + Recovery
v0.9.0-rc     Security + Upgrade Safety + RC
v1.0.0        Production Grade
```

Semantic Versioning is required for Studio and all `13thx-mcp` component releases.

---

# 16. Current Immediate Next Step

**Milestone 7 — Gateway Coordination, Concurrency & Tool Safety** is the next implementation milestone.

M5 publication qualification and M6 closure are complete. Before M7 implementation begins, post-M6 hardening changes must keep the existing M5/M6 authority and recovery contracts green under the normal clean-tree quality/security qualification.

M7 must start from the closed M6 authority split: SQLite remains historical evidence only; live request authority stays in Gateway/runtime owners; unattended auto-update/reconciliation remains M8 scope until M7 drain, concurrency, cancellation and unknown-outcome semantics are verified.
