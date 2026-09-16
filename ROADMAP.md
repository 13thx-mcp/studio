# MCP Studio Roadmap

> Scope: Web-based control plane for monitoring and managing MCP servers and secure tunnels under `mcp-server/`.
>
> Initial managed MCP servers: `blender` and `filesystem`.
>
> Target maturity path: **Foundation → MVP → Alpha → Beta → Release Candidate → Production Grade → Scale & Extensibility**.

---

## 1. Product Goal

MCP Studio is a local-first web service that provides a single control plane for MCP server lifecycle, secure tunnel lifecycle, configuration, discovery, monitoring, usage statistics, logs, and operational safety.

The first production-grade release must support:

- Start / stop / restart MCP servers.
- Detect crashes and optionally auto-restart MCP processes.
- Track MCP process state, uptime, restarts, failures, and usage metrics.
- Start / stop / restart the secure tunnel runtime.
- Monitor tunnel state and reconnect events.
- View live and historical metrics.
- View structured logs and recent operational events.
- Edit validated MCP configuration from the web UI.
- Scan paths below `mcp-server/` and discover new MCP projects.
- Review and explicitly approve discovered MCP servers before registration.
- Persist registry, configuration, statistics, and audit history.
- Operate safely without leaking secrets to the browser or logs.

---

# 2. SDLC Model

MCP Studio will follow an iterative SDLC with an explicit quality gate at every milestone.

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

Each milestone must complete the following stages before it is considered done.

## 2.1 Planning

- Define scope and exclusions.
- Identify dependencies.
- Record risks and assumptions.
- Define acceptance criteria.
- Identify migration or rollback requirements.

## 2.2 Requirements

- Functional requirements documented.
- Non-functional requirements documented.
- API behavior defined where applicable.
- Error states and recovery behavior defined.
- Security boundaries identified.

## 2.3 Design

- Architecture changes reviewed before implementation.
- Data model changes documented.
- Public API changes documented.
- Process lifecycle/state-machine changes documented.
- Threat-model impact reviewed for privileged operations.

Architecture decisions with long-term impact should be captured as ADRs under:

```text
docs/adr/
```

## 2.4 Implementation

- Small modules with explicit responsibilities.
- No secrets embedded in source code or committed config.
- Structured errors instead of silent failure.
- Backward-compatible config changes where feasible.
- Feature flags for risky or incomplete operational features.

## 2.5 Verification

Minimum verification layers:

- Unit tests.
- Integration tests.
- API tests.
- Process lifecycle tests.
- Failure/recovery tests.
- UI smoke tests for user-facing workflows.

Production-sensitive features additionally require:

- Security tests.
- Restart/recovery tests.
- Upgrade/migration tests.
- Load/soak tests where applicable.

## 2.6 Security Review

Before each release candidate:

- Dependency audit.
- Secret leakage review.
- Path traversal review.
- Command execution boundary review.
- Authentication/authorization review when remote access is enabled.
- Log redaction validation.
- Tunnel exposure review.

## 2.7 Release

Every releasable milestone must provide:

- Version number.
- Changelog entry.
- Release notes.
- Migration notes if required.
- Rollback procedure.
- Known limitations.

## 2.8 Operations and Feedback

After each milestone release:

- Observe runtime behavior.
- Record defects and operational friction.
- Review metrics and failure modes.
- Feed findings into the next planning cycle.

---

# 3. Definition of Ready

A milestone is ready for implementation when:

- Scope is defined.
- Acceptance criteria are testable.
- Major architecture decisions are resolved.
- Dependencies are known.
- Security implications are identified.
- Out-of-scope items are explicit.

---

# 4. Definition of Done

A milestone is complete only when:

- All acceptance criteria pass.
- New behavior has tests.
- Existing tests still pass.
- No known Critical or High severity security issue remains unresolved.
- Documentation reflects the implemented behavior.
- Config/data migrations are tested where applicable.
- Manual smoke test succeeds from a clean checkout/build.
- Rollback path is known.

---

# 5. Target Architecture

```text
Browser
   │
   │ HTTP / WebSocket
   ▼
┌──────────────────────── MCP Studio ──────────────────────────┐
│                                                             │
│  Web UI                                                     │
│     │                                                       │
│  HTTP / WS API                                              │
│     │                                                       │
│  ┌──────────────┐   ┌────────────────┐   ┌───────────────┐  │
│  │ MCP Registry │   │ MCP Supervisor │   │ Tunnel Manager│  │
│  └──────┬───────┘   └───────┬────────┘   └───────┬───────┘  │
│         │                   │                    │           │
│  ┌──────▼───────┐    ┌──────▼───────┐     ┌──────▼──────┐   │
│  │ Discovery    │    │ Metrics/Logs │     │ cloudflared │   │
│  └──────────────┘    └──────┬───────┘     └─────────────┘   │
│                              │                               │
│                         ┌────▼────┐                          │
│                         │ SQLite  │                          │
│                         └─────────┘                          │
└─────────────────────────────────────────────────────────────┘
              │                         │
              ▼                         ▼
        Blender MCP                Filesystem MCP
           stdio                       stdio
```

Initial technology direction:

- Backend: Rust.
- Async runtime: Tokio.
- HTTP/WebSocket: Axum.
- Persistence: SQLite.
- Frontend: React + TypeScript + Vite.
- Local bind by default: `127.0.0.1`.
- MCP process transport initially: stdio.
- Secure tunnel runtime: existing `mcp-server/tunnel-client` bundle.

Technology choices may change only through an ADR once implementation begins.

---

# 6. Milestone Roadmap

---

## Milestone 0 — Foundation & SDLC Bootstrap

**Target:** `v0.0.x`

### Goal

Establish engineering standards, repository structure, architecture boundaries, test strategy, and release discipline before feature implementation.

### Deliverables

- `ROADMAP.md`.
- Initial architecture document.
- Initial threat model.
- ADR template.
- Contribution/development guide.
- Coding conventions.
- Logging conventions.
- Error handling conventions.
- Configuration conventions.
- Test directory/layout.
- CI workflow skeleton.
- Build commands documented.

### Proposed project structure

```text
studio/
├── Cargo.toml
├── README.md
├── ROADMAP.md
├── CHANGELOG.md
├── docs/
│   ├── architecture.md
│   ├── threat-model.md
│   └── adr/
├── src/
│   ├── api/
│   ├── config/
│   ├── discovery/
│   ├── metrics/
│   ├── registry/
│   ├── storage/
│   ├── supervisor/
│   └── tunnel/
├── tests/
└── web/
```

### Key decisions

- Studio owns lifecycle management only for processes it starts.
- Arbitrary shell execution is prohibited from the web interface.
- MCP auto-discovery never auto-executes a newly found project.
- Secrets must remain server-side.
- Studio binds to localhost by default.

### Verification

- Clean project builds.
- Unit test framework executes.
- CI runs formatting, linting, tests, and dependency checks.

### Exit Criteria

- Architecture and threat model reviewed.
- Repository conventions agreed.
- CI baseline green.

---

## Milestone 1 — MVP Core Process Supervisor

**Target:** `v0.1.0`

### Goal

Manage the two existing MCP servers reliably from a single backend service.

### Scope

Initial managed servers:

- `blender`
- `filesystem`

### Features

- MCP registry.
- Static configuration for known MCP servers.
- Start MCP process.
- Stop MCP process.
- Restart/reconnect MCP process.
- PID tracking.
- Process state tracking.
- Exit-code tracking.
- Uptime tracking.
- Restart count.
- Crash count.
- Graceful stop with forced-kill fallback.
- Capture stdout/stderr.
- In-memory recent log ring buffer.
- Basic REST API.
- Basic health endpoint.

### Initial lifecycle model

```text
STOPPED
   │ start
   ▼
STARTING
   │ success
   ▼
RUNNING ───────────────┐
   │ stop              │ unexpected exit
   ▼                   ▼
STOPPING             FAILED
   │                   │
   ▼                   │ restart policy
STOPPED ◄──────────────┘
```

### Minimum API

```text
GET  /api/status
GET  /api/mcp
GET  /api/mcp/:id
POST /api/mcp/:id/start
POST /api/mcp/:id/stop
POST /api/mcp/:id/restart
GET  /api/mcp/:id/logs
```

### Tests

- Start process successfully.
- Stop process successfully.
- Restart process successfully.
- Detect unexpected exit.
- Invalid executable path.
- Duplicate start request.
- Stop already-stopped process.
- Studio shutdown terminates managed child processes according to policy.

### Out of Scope

- Historical metrics.
- Full UI.
- Tunnel control.
- MCP gateway/proxy.
- Remote authentication.

### Exit Criteria

From one backend process it is possible to reliably start, stop, restart, and inspect both Blender and Filesystem MCP servers.

---

## Milestone 2 — MVP Web Dashboard

**Target:** `v0.2.0`

### Goal

Provide an operational web interface for core MCP management.

### Features

- Dashboard overview.
- MCP list.
- Per-server detail page.
- Start / stop / restart controls.
- Live process state.
- Live log viewer.
- Uptime and restart counters.
- Last exit/error display.
- WebSocket live updates.
- Responsive layout for desktop-first operation.

### UX Requirements

Potentially destructive operations must clearly show:

- Target MCP server.
- Current state.
- Requested action.

No UI component may render secret environment variable values.

### Tests

- API/UI smoke test.
- WebSocket reconnect test.
- UI refresh preserves correct backend state.
- Action button state reflects process state.

### Exit Criteria

A user can manage the two MCP servers without using the command line after starting Studio.

---

## Milestone 3 — Secure Tunnel Management

**Target:** `v0.3.0`

### Goal

Manage the existing secure tunnel runtime safely through Studio.

### Features

- Detect tunnel runtime binary.
- Start tunnel.
- Stop tunnel.
- Restart tunnel.
- Tunnel PID/state.
- Tunnel uptime.
- Exit/restart counters.
- Capture structured tunnel logs.
- Track reconnect events where detectable.
- Configure tunnel startup arguments using a constrained schema.
- Environment/file secret references.
- Tunnel page in the UI.

### Security Requirements

- Allow-list the tunnel executable.
- Do not accept raw shell command strings from the browser.
- Never return token values via API.
- Redact known secret patterns from logs.
- Validate config file references.

### Tests

- Start/stop/restart tunnel.
- Runtime missing.
- Invalid configuration.
- Tunnel crashes and recovery behavior.
- Secret values absent from API responses and logs.

### Exit Criteria

Tunnel lifecycle is manageable from Studio with operational visibility and no secret exposure in normal UI/API paths.

---

## Milestone 4 — Registry, Configuration & Auto-Discovery

**Target:** `v0.4.0`

### Goal

Make Studio extensible beyond the initial two MCP servers.

### Features

- Persistent MCP registry.
- Scan `mcp-server/*`.
- Ignore internal/non-MCP directories such as:
  - `studio`
  - `tunnel-client`
  - build output
  - hidden directories
- Runtime/project detection:
  - Rust via `Cargo.toml`
  - Node via `package.json`
  - Python via `pyproject.toml` or supported manifest
- Detect likely executable/build information.
- Discovery preview.
- Manual approval before registration.
- Config editor.
- Config validation.
- Enable/disable MCP entry.
- Remove registration without deleting source code.
- Configuration schema versioning.

### Recommended registry concepts

```yaml
servers:
  blender:
    enabled: true
    path: ../blender
    runtime: rust
    transport: stdio
    command: target/release/rust-mcp-blender
    args: []
    restart:
      policy: on-failure
      max_attempts: 5
```

### Security Requirements

- Canonicalize paths.
- Prevent traversal outside configured MCP root.
- Do not automatically execute discovered code.
- Do not permit arbitrary executable selection outside allowed policy.

### Tests

- Detect Blender project.
- Detect Filesystem project.
- Ignore Studio itself.
- Ignore malformed project.
- Reject traversal paths.
- Config schema validation.
- Registry persistence after restart.

### Exit Criteria

A new supported MCP project placed under the configured MCP root can be discovered, reviewed, registered, configured, and managed without source-code changes to Studio.

---

## Milestone 5 — Persistence, Metrics & Auditability

**Target:** `v0.5.0`

### Goal

Move from transient monitoring to useful operational observability.

### Persistence

Use SQLite for:

- Registered MCP servers.
- Runtime sessions.
- Process events.
- Tunnel events.
- Metrics buckets.
- Configuration revisions where appropriate.
- Audit events.

### Process Metrics

- Current state.
- PID.
- Current uptime.
- Total launches.
- Restart count.
- Crash count.
- Last exit code.
- Last start time.
- Last stop time.
- Average session duration.

### Initial Usage Metrics

Metrics available without an MCP gateway:

- Process activity.
- Log/error event counts.
- Session uptime.
- Restart/crash behavior.

Request-level MCP metrics should only be reported when they are directly observable rather than guessed.

### Audit Events

Record actions such as:

- Server started.
- Server stopped.
- Server restarted.
- Tunnel started/stopped/restarted.
- Config changed.
- MCP registered/unregistered.
- Discovery approved.

### UI

- Historical charts.
- Time range selector.
- Recent event timeline.
- Error/restart trends.

### Tests

- Database migrations.
- Restart persistence.
- Event recording accuracy.
- Metrics retention.
- Corrupt/locked DB error handling.

### Exit Criteria

Operational history survives Studio restart and accurately reflects supervised process events.

---

## Milestone 6 — Alpha Hardening & Developer Preview

**Target:** `v0.6.0-alpha`

### Goal

Stabilize the full local management workflow before broader usage.

### Features

- Auto-restart policy.
- Restart backoff.
- Crash-loop detection.
- Health-check abstraction.
- Config backup before mutation.
- Safe rollback to previous config.
- Improved structured logs.
- Diagnostics bundle without secrets.
- Better startup validation.
- Better error surfaces in UI.

### Reliability Requirements

- Prevent infinite tight restart loops.
- Supervisor state must converge after process failure.
- Studio restart must reconcile stale persisted process state.
- Partial failure of one MCP must not crash Studio.
- Tunnel failure must not crash the MCP supervisor.

### Verification

- Fault injection.
- Repeated crash/restart test.
- Studio forced restart test.
- Invalid config recovery.
- Long-running local soak test.

### Exit Criteria

No known Critical/High defects in core lifecycle management, configuration, persistence, or tunnel management.

---

## Milestone 7 — Beta: MCP Gateway & Accurate Usage Telemetry

**Target:** `v0.7.0-beta`

### Goal

Introduce an optional gateway layer so Studio can observe real MCP traffic and expose multiple MCP servers through a controlled endpoint.

### Proposed Flow

```text
MCP Client
    │
    ▼
Secure Tunnel
    │
    ▼
MCP Studio Gateway
    │
    ├── Blender MCP
    ├── Filesystem MCP
    └── Other registered MCPs
```

### Features

- Gateway routing by MCP identifier.
- Session tracking.
- Request count.
- Success/error count.
- Requests per minute.
- Per-tool usage where protocol-safe.
- Request latency.
- p50/p95 latency.
- Active sessions.
- Gateway health.
- Optional per-MCP access policy.

### Privacy/Security Requirements

- Do not persist request payloads by default.
- Do not persist MCP tool arguments by default.
- Metadata telemetry should be sufficient for normal monitoring.
- Sensitive payload logging requires explicit opt-in and clear warning.

### Tests

- Request routing.
- Concurrent sessions.
- MCP process restart during active usage.
- Gateway timeout behavior.
- Backpressure.
- Large request/response boundaries.
- Metrics correctness.

### Exit Criteria

Studio can report real MCP usage statistics based on traffic it actually observes.

---

## Milestone 8 — Release Candidate: Security, Reliability & Upgrade Safety

**Target:** `v0.9.0-rc`

### Goal

Prepare for production-grade deployment.

### Security

- Formal threat-model review.
- Authentication model for non-localhost deployments.
- Authorization/RBAC if multi-user access is supported.
- CSRF protection for browser-authenticated deployments.
- Strict CORS policy.
- Secure headers.
- Secret-at-rest strategy where Studio stores credentials.
- Rate limiting for privileged APIs.
- Audit trail integrity review.
- Dependency vulnerability scan.
- Supply-chain/build provenance review.

### Reliability

- Graceful Studio shutdown.
- Child process reconciliation.
- Crash-loop circuit breaker.
- Database backup/restore procedure.
- Database migration rollback strategy.
- Config rollback.
- Log rotation.
- Metrics retention policy.
- Disk usage protections.

### Performance

- API load test.
- WebSocket fan-out test.
- Multi-MCP concurrency test.
- Long-running soak test.
- Memory leak observation.
- Database growth test.

### Compatibility

- macOS target validated.
- Linux target validated if production scope includes Linux.
- Supported browser matrix documented.

### Release Engineering

- Reproducible release build.
- Versioned artifacts.
- Checksums.
- Release notes.
- Upgrade guide.
- Rollback guide.

### Exit Criteria

- Release candidate passes security review.
- Migration tests pass from supported previous versions.
- No unresolved Critical/High security finding.
- No unresolved blocker defect.
- Soak/load targets pass.

---

## Milestone 9 — Production Grade v1.0

**Target:** `v1.0.0`

### Goal

Provide a stable, supportable MCP control plane suitable for daily operational use.

### Required Capabilities

#### MCP Lifecycle

- Start.
- Stop.
- Restart/reconnect.
- Health state.
- Auto-restart.
- Crash-loop protection.
- Process ownership safety.

#### Tunnel Lifecycle

- Start.
- Stop.
- Restart.
- State monitoring.
- Reconnect tracking.
- Safe secret handling.

#### Registry & Configuration

- Persistent registry.
- Auto-discovery.
- Explicit registration approval.
- Config validation.
- Config versioning.
- Safe rollback.

#### Observability

- Live status.
- Logs.
- Historical process metrics.
- Audit events.
- Real MCP usage metrics when gateway mode is enabled.

#### Security

- Localhost-safe defaults.
- Strong authentication for remote mode.
- Authorization where multi-user operation exists.
- No raw arbitrary command execution from UI.
- Secret redaction.
- Path confinement.
- Auditability.

#### Operations

- Backup/restore documentation.
- Upgrade/migration documentation.
- Log/metric retention controls.
- Health/readiness endpoints.
- Diagnostics export.
- Defined support matrix.

### Production SLO Targets

Initial targets to validate during RC testing:

- Studio control-plane availability: `>= 99.9%` when host is healthy.
- No loss of registry/config data during clean restart.
- Recovery from managed MCP crash according to configured restart policy.
- Control API p95 latency below an agreed local-network threshold under expected load.
- No known Critical/High severity vulnerability at release.

Exact numeric performance targets should be finalized after baseline measurements rather than guessed before implementation.

### Production Release Gate

Production release is approved only if:

- Functional acceptance tests pass.
- Security review passes.
- Migration/rollback tests pass.
- Backup/restore test passes.
- Soak test passes.
- Operational documentation is complete.
- Known limitations are documented.
- Release artifact can be recreated from source.

---

# 7. Post-v1.0 Roadmap

Potential milestones after production readiness:

## v1.1 — Plugin/Adapter System

- Custom runtime adapters.
- MCP templates.
- Custom health checks.
- Event hooks.

## v1.2 — Multi-host Agents

- Remote Studio agents.
- Central control plane.
- Host inventory.
- Mutual authentication.

## v1.3 — Advanced Policy

- Per-MCP permissions.
- Per-tool allow/deny rules.
- Session policies.
- Resource quotas.

## v1.4 — Advanced Observability

- OpenTelemetry export.
- Prometheus metrics endpoint.
- External log sink integration.
- Distributed tracing for gateway traffic.

---

# 8. Testing Strategy by Layer

| Layer | Purpose |
|---|---|
| Unit | Validate state machines, parsing, validation, redaction and business logic |
| Integration | Validate supervisor, DB, registry, discovery, tunnel and OS process behavior |
| API | Validate HTTP/WebSocket contract and error behavior |
| UI | Validate main operational workflows |
| Security | Validate traversal, secret handling, command restrictions and auth boundaries |
| Failure Injection | Validate crash/restart/recovery behavior |
| Migration | Validate upgrades and rollbacks |
| Load | Validate API/gateway concurrency and resource usage |
| Soak | Detect leaks, stale state and long-running degradation |

---

# 9. Security Threat Areas to Track from Day One

The threat model must explicitly track at least:

1. Arbitrary process execution.
2. Command/argument injection.
3. Path traversal.
4. Symlink escape where relevant.
5. Secret leakage through API/logs/UI.
6. Unauthorized remote control of MCP processes.
7. Tunnel accidental public exposure.
8. Malicious auto-discovered MCP project.
9. Crash-loop resource exhaustion.
10. Log/disk exhaustion.
11. Database tampering/corruption.
12. Dependency/supply-chain compromise.
13. Cross-site request attacks when remote web access is enabled.
14. Privilege escalation through managed MCP configuration.

---

# 10. Configuration Principles

- Config must have an explicit schema version.
- Unknown critical fields should fail safely.
- Secrets should be referenced, not embedded where possible.
- Web UI must never receive stored secret values after initial submission unless explicitly required by a secure design.
- Config changes should be validated before activation.
- Production-grade config changes should support rollback.

Example secret reference:

```yaml
env:
  API_KEY:
    from_env: MCP_API_KEY
```

---

# 11. Logging Principles

Logs should be structured and include where applicable:

- timestamp
- component
- mcp_id
- process_id
- event_type
- severity
- request/session identifier when safe

Logs must not contain:

- API tokens
- tunnel credentials
- authorization headers
- private environment values
- full MCP payloads by default

---

# 12. Database Migration Policy

Starting when SQLite is introduced:

- Every schema change has a numbered migration.
- Migration is tested from the previous supported release.
- Backups are created before destructive migrations where practical.
- Failed migration must not silently continue.
- Production releases document whether downgrade is supported.

---

# 13. Release Versioning

Recommended progression:

```text
v0.0.x        Foundation
v0.1.0        Core Supervisor MVP
v0.2.0        Web Dashboard MVP
v0.3.0        Tunnel Management
v0.4.0        Registry + Discovery
v0.5.0        Persistence + Metrics
v0.6.0-alpha  Hardening / Developer Preview
v0.7.0-beta   Gateway + Real Usage Telemetry
v0.9.0-rc     Production Release Candidate
v1.0.0        Production Grade
```

Semantic Versioning should be used from the first public/pre-release artifact.

---

# 14. Current Immediate Next Step

The next implementation work should be **Milestone 0 only**.

Do not jump directly into the dashboard or gateway.

Recommended execution order:

```text
1. Bootstrap Rust Studio project
2. Add docs/architecture.md
3. Add docs/threat-model.md
4. Add ADR template
5. Establish config model
6. Establish error/logging conventions
7. Establish test structure
8. Add CI baseline
9. Review Milestone 0 exit criteria
10. Begin Milestone 1 only after Milestone 0 passes
```

This keeps the project aligned with SDLC from the beginning and avoids accumulating operational/security debt before the privileged process-management code is introduced.
