# M8 Source Audit

**Audit baseline:** Studio `ab9aca19c3fc6091ca3d9d169f5fc2a314b8d476` / `0.7.1` plus the M7-qualified Gateway/Filesystem/Exec/Git/Fleet lines recorded in M7 qualification.

This audit records implementation prerequisites, not defects already claimed fixed.

## Current strengths M8 should reuse

1. `RuntimeOperationCoordinator` already prevents control-path mutation from overlapping component mutation in one Studio process.
2. Update managers already verify release source, platform, checksum, staging identity, rollback baseline and post-activation health/version according to component semantics.
3. Gateway already has bounded admission, drain, queue/concurrency limits, child backoff/circuit state and a private control socket.
4. Runtime reconciliation already distinguishes managed-safe drift from broken/unmanaged states and performs backup + rollback.
5. Tunnel and Studio self-update already have stronger durable recovery contracts than generic component updates.
6. Fleet already owns deterministic pure rendering and runtime-only deployment contracts.
7. M6 already stores sanitized audit/history projections and must remain historical only.

## M8.0 prerequisites — contract-resolved, implementation pending

### P-01 — no automation policy/scheduler configuration

`UpdatesConfig` currently contains only source/bin/runtime roots plus desired versions. There is no update mode, cadence, maintenance window, reconciliation cadence, restart policy, retry budget or circuit configuration.

**M8 disposition:** add schema-validated automation configuration with fail-safe defaults. Existing config without M8 fields must remain manual.

### P-02 — runtime-operation lease is conflict-only and process-local

`RuntimeOperationCoordinator` can reject overlap but exposes no safe status snapshot, owner metadata for UI/policy, waiting/fairness contract, or persisted state.

**M8 disposition:** keep the coordinator as the in-process mutation mutex. Automation never busy-spins waiting for it; a conflict becomes bounded defer/backoff. Add read-only snapshot/introspection rather than turning it into a durable lock service.

### P-03 — Studio drops Gateway live safety counters

Gateway control responses already contain `active_requests`, `queued_requests`, drain generation and generations. Studio currently deserializes only state/drain/history fields and has no public safety-status probe.

**M8 disposition:** add an additive capability-aware Gateway safety-status contract. Auto mutation must fail closed if the required Gateway capability is missing or stale.

### P-04 — sanitized Gateway telemetry lacks request class

Gateway policy owns `ToolClass = read | mutation | long-running | control`, but request telemetry currently persists child/tool/outcome without the class.

An `outcome=unknown` record therefore cannot be safely classified after restart without reinterpreting current policy.

**M8 disposition:** include server-owned request class in the sanitized telemetry DTO and history projection. Never infer historical class from current tool names/policy.

### P-05 — unknown side-effect outcomes have no durable live safety hold

Gateway returns structured non-retryable unknown outcomes, but its request/telemetry stores are bounded in-memory data. A Gateway restart can remove the live evidence needed to veto unattended mutation.

**M8 disposition:** introduce a small Gateway-owned durable safety-hold ledger containing only bounded sanitized identifiers/metadata. It is a veto authority, not a request queue and never stores arguments/results. Mutation/control/long-running unknown outcomes create holds before automation can proceed.

### P-06 — generic MCP update does not coordinate Gateway child ownership

`McpUpdateManager` has no Gateway drain/control dependency. A flat binary may be referenced by a Gateway child while Studio replaces its on-disk bytes. Renaming the binary does not change the already-running child process.

**M8 disposition:** freeze a runtime-ownership/activation matrix. For Gateway-managed children, automatic activation requires a narrow Gateway targeted child quiesce/restart/verify capability or an equally strong reviewed alternative. Do not claim installed version equals running version.

### P-07 — most prepared transaction maps are process-local

Generic MCP, Gateway and Fleet managers keep transaction records in memory while ready staging directories persist. A Studio restart can therefore leave valid-looking staged bytes without the transaction state that authorized them.

Studio self-update and Tunnel have stronger specialized durable state.

**M8 disposition:** process-scoped prepared state is the initial M8 rule. Generic/Gateway/Fleet staged artifacts from a previous Studio process are never auto-applied. They are revalidated for cleanup and, if policy still wants the version, re-prepared under a new transaction.

### P-08 — ready staging retention is not a bounded automation policy

The stager cleans partial directories at construction, but ready staging may outlive its in-memory transaction and has no M8 TTL/aggregate disk policy.

**M8 disposition:** add bounded ready-staging retention/cleanup with ownership checks, age and aggregate byte caps. Unknown/tampered entries are preserved for operator diagnosis or fail closed according to the frozen recovery contract; never guess-delete recovery authority.

### P-09 — reconciliation retry/circuit state is absent

`RuntimeReconciler` safely checks/applies/rolls back one action but has no periodic scheduling, retry budget or durable drift-loop circuit.

**M8 disposition:** scheduler owns bounded retry/circuit metadata; reconciler remains the one-shot authority. Only `ManagedSafeDrift && safe_to_reconcile` can auto-apply.

### P-10 — restart policy is fragmented

Gateway child restart backoff/circuit is already M7-owned. Studio Supervisor and Tunnel track crash/restart counters but do not implement the same automatic restart policy.

**M8 disposition:** add explicit restart policy only to owners that lack it. Do not create a second restart loop for Gateway children. One process has one restart authority.

### P-11 — no common health evidence abstraction

Component update code contains strong component-specific verification, but there is no common freshness/status model the automation policy can evaluate before/after unattended action.

**M8 disposition:** add typed `HealthSnapshot`/probe evidence that preserves component-specific semantics and freshness. A generic "process exists" result is insufficient for safe activation.

### P-12 — dirty source is a warning, not an automation admission rule

Fleet doctor already warns that a dirty source tree must block automatic update, but Studio activation does not own that policy.

**M8 disposition:** on source-present development hosts, automatic activation fails closed for a managed component when its authoritative source repo is dirty, conflicted, unresolved or cannot be inspected. Notify/prepare may continue because they do not mutate active runtime.

### P-13 — audit admission is API-shaped

Manual API actions use `admit_local_operation` / audited response handling. A background worker cannot safely bypass that evidence model.

**M8 disposition:** extract a shared internal operation-admission/audit service used by API and automation. Background mutations require the same or stricter admission semantics.

### P-14 — maintenance-window semantics are undefined

There is no timezone/DST contract.

**M8 disposition:** M8 initial windows are defined in UTC with explicit weekdays, start minute and duration. UI may render local equivalents. IANA/DST scheduling is deferred rather than guessed.

### P-15 — self-update intentionally exits the scheduler process

Studio self-update delegates activation to Fleet and requests graceful shutdown.

**M8 disposition:** auto-update-safe may prepare Studio automatically; activation requires maintenance window and durable external-launcher contract. The new Studio process must reconcile the prior automation intent without assuming the old scheduler completed.

### P-16 — no bounded diagnostics bundle

History, inventory, reconciliation, Gateway status and recovery files exist in separate domains.

**M8 disposition:** add a sanitized bounded diagnostics exporter. It may copy summaries and hashes, never credentials, raw env, raw MCP payloads or unrestricted paths.

### P-17 — automation config authority is not yet represented in Fleet host schema

Fleet owns deterministic rendering of `runtime/studio/studio.toml`. Adding automation policy only to Studio config would create two desired-state authorities and reconciliation churn.

**M8 disposition:** automation policy is represented in the Fleet host profile/schema and rendered deterministically into Studio config. Runtime-only hosts receive the same policy bytes. Direct local Studio config remains possible only outside Fleet-managed surfaces according to existing ownership rules.

### P-18 — loaded automation policy can become stale after Fleet reconciliation

Runtime reconciliation already tracks `studio_restart_required` / loaded config identity. If Fleet changes automation policy in `studio.toml`, the old Studio process must not continue destructive automation using the previous policy.

**M8 disposition:** automatic mutation is blocked while Studio config activation is RestartRequired/Unknown for a changed managed Studio config. The new process resumes only after loaded-config identity proves the current Fleet-rendered bytes. Prepared state records the policy/config fingerprint and is re-evaluated after restart.
### P-19 — guarded release tagging does not currently accept prerelease target syntax

M8 roadmap target is `v0.8.0-beta`, while the guarded Git release-tag operation currently enforces plain `vMAJOR.MINOR.PATCH`. Using generic tagging would bypass exact release-evidence enforcement.

**M8.0 disposition:** ADR 0045 accepts guarded SemVer prerelease support with exact release-prep/tag identity on the existing evidence-gated path. Git MCP target is 0.2.0; M8 remains `v0.8.0-beta`.
## M8.0 resolution matrix

| Prerequisite | Contract disposition |
|---|---|
| P-01 automation config/scheduler absent | Resolved by ADR 0037, ADR 0038 and ADR 0043 |
| P-02 runtime coordinator introspection | Resolved by ADR 0037 |
| P-03 Studio drops Gateway live safety fields | Resolved by ADR 0041 |
| P-04 telemetry lacks request class | Resolved by ADR 0040 |
| P-05 unknown outcome lacks durable hold | Resolved by ADR 0040 |
| P-06 generic update lacks Gateway child coordination | Resolved by ADR 0041 |
| P-07 process-local prepared transaction authorization | Resolved by ADR 0039 |
| P-08 staging retention/budget absent | Resolved by ADR 0039 and ADR 0044 |
| P-09 reconciliation retry/circuit absent | Resolved by ADR 0038 and ADR 0043 |
| P-10 restart ownership fragmented | Resolved by ADR 0042 |
| P-11 common health evidence absent | Resolved by ADR 0042 |
| P-12 dirty source not activation gate | Resolved by ADR 0039 |
| P-13 audit admission API-shaped | Resolved by ADR 0037 |
| P-14 maintenance-window semantics absent | Resolved by ADR 0038 |
| P-15 self-update exits controller process | Resolved by ADR 0037 and existing durable self-update authority |
| P-16 diagnostics bundle absent | Resolved by ADR 0044 |
| P-17 Fleet automation desired-state authority absent | Resolved by ADR 0043 |
| P-18 old process may hold stale policy | Resolved by ADR 0043 |
| P-19 guarded prerelease tag unsupported | Resolved by ADR 0045 |

Source feasibility for these dispositions is recorded in [M8.0-SOURCE-SPIKES.md](M8.0-SOURCE-SPIKES.md).

"Resolved" here means the architecture/test contract is frozen. It does not claim the production implementation exists.

## Cross-repository prerequisites that block auto-update-safe

The following are resolved at contract level but remain implementation/qualification blockers before M8.4 can enable automatic activation:

- P-03 live Gateway safety snapshot;
- P-04/P-05 durable classed unknown-outcome safety holds;
- P-06 Gateway child activation coordination;
- P-07/P-08 restart-safe prepared staging semantics;
- P-13 shared audit admission;
- P-17 Fleet ownership of automation config;
- P-18 loaded-policy/config-activation freshness.

## Source drift rule

Before implementing each package, re-run source audit searches against current main. If an upstream package changed one of these anchors, update the contract before coding rather than adapting silently.