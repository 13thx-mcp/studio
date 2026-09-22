# M8 Dependency-Ordered Execution Checklist

**State:** PLANNING / NOT IMPLEMENTED  
**Target:** Studio v0.8.0-beta  
**Entry:** M7 v0.7.1 exact-main qualification green.

## M8.0 — Architecture, policy and recovery freeze

- [ ] Re-run source audit against current Studio/Gateway/Fleet heads.
- [ ] Resolve P-01 through P-19.
- [ ] Accept/amend ADR 0037–0044.
- [ ] Freeze additive automation config schema/defaults and Fleet host-profile ownership.
- [ ] Freeze AutomationState schema/path/atomicity/size bounds.
- [ ] Freeze scheduler missed-tick, UTC window, clock anomaly and retry semantics.
- [ ] Freeze failure-vs-deferral taxonomy.
- [ ] Freeze Gateway M8 capability advertisement.
- [ ] Freeze request-class telemetry addition.
- [ ] Freeze durable Gateway safety-hold schema and persistence ordering.
- [ ] Freeze hold resolution semantics; prove no replay.
- [ ] Freeze targeted Gateway child activation contract.
- [ ] Freeze component activation/ownership matrix.
- [ ] Freeze health snapshot/freshness model.
- [ ] Freeze process-scoped prepared-authorization rule.
- [ ] Freeze staging TTL/disk-budget/cleanup ownership.
- [ ] Freeze source-present Git hygiene rule.
- [ ] Freeze reconciliation retry/circuit contract.
- [ ] Freeze diagnostics allowlist/caps.
- [ ] Freeze component target versions and guarded prerelease release-tag contract.
- [ ] Update threat model/architecture before production coding.

**Exit:** no unresolved ambiguity can cause automatic replay, competing restart owners, unsafe child replacement, unbounded retry, or cleanup of recovery authority.

## M8.1 — Automation foundation and durable policy state

- [ ] Extend config with backwards-compatible `[automation]`.
- [ ] Default every automatic mode to manual/disabled.
- [ ] Implement validated UTC maintenance window type.
- [ ] Implement AutomationStateStore with schema v1, size cap, confinement, atomic fsync replacement.
- [ ] Fail automatic mutation closed on corrupt/future state.
- [ ] Implement ScheduleEngine with startup grace and skipped missed ticks.
- [ ] Implement bounded retry/backoff/circuit state.
- [ ] Detect persisted clock regression and block destructive automation.
- [ ] Add RuntimeOperationCoordinator read-only snapshot.
- [ ] Treat busy coordinator as defer/backoff, never busy loop.
- [ ] Extract shared internal audit/admission service from API path.
- [ ] Route manual API actions through the shared service unchanged.
- [ ] Add AutomationController cancellation/shutdown contract.
- [ ] Add unit/fake-time/restart tests.

**Exit:** a controller can safely schedule observations and persist policy state but cannot yet auto-activate runtime.

## M8.2 — Health, restart reliability and Gateway safety snapshot

- [ ] Define typed HealthSnapshot/freshness.
- [ ] Add component-specific health adapters.
- [ ] Extend Gateway control response consumption with active/queued/generation/capabilities.
- [ ] Add Gateway child summary/identity capability required by activation contract.
- [ ] Old Gateway compatibility: read/manual works; automatic mutation unavailable.
- [ ] Add Supervisor auto-restart disabled/on-failure policy.
- [ ] Add Tunnel auto-restart disabled/on-failure policy.
- [ ] Preserve explicit-stop suppression.
- [ ] Suppress restart while update/reconciliation owns component.
- [ ] Add bounded exponential backoff/stability window/circuit.
- [ ] Keep Gateway child restart exclusively Gateway-owned.
- [ ] Add crash-loop/fake-time/ownership tests.

**Exit:** automation can obtain fresh typed safety evidence and process owners cannot enter competing restart loops.

## M8.3 — Update checks, notify-only and auto-prepare

- [ ] Implement periodic/startup update check policy.
- [ ] One concurrent provider check batch maximum.
- [ ] Apply desired exact-version precedence.
- [ ] Default selection to trusted stable releases.
- [ ] Reject automatic downgrade/same-version repair.
- [ ] Implement notify-only event/history/UI state.
- [ ] Implement auto-prepare through existing component managers.
- [ ] Assert no active runtime mutation during auto-prepare.
- [ ] Track current-process prepared references only.
- [ ] On Studio restart, refuse automatic apply of previous-process Generic/Gateway/Fleet staging.
- [ ] Add bounded ready-staging inventory/TTL/aggregate budget.
- [ ] Add source hygiene observation for development hosts.
- [ ] Dirty/conflicted source blocks activation but not notify/prepare.
- [ ] Add provider failure circuit and recovery tests.

**Exit:** M8 can safely observe and stage releases without activation.

## M8.4 — Safe automatic activation

**Blocked until M8.2 and Gateway hazard/activation prerequisites from M8.6 are qualified.**

- [ ] Implement final SafetyGate evaluated immediately before physical mutation.
- [ ] Require maintenance window.
- [ ] Require audit admission.
- [ ] Require no open Gateway unsafe hold.
- [ ] Require Gateway capability/status freshness.
- [ ] Require rollback baseline/material.
- [ ] Revalidate installed source version and staged identity.
- [ ] Revalidate source Git hygiene.
- [ ] Revalidate loaded Studio policy/config fingerprint and require no pending Studio config restart.
- [ ] Revalidate component health/stable owner state.
- [ ] Implement Gateway targeted child quiesce/restart/verify.
- [ ] Coordinate generic flat MCP binary activation with Gateway child ownership.
- [ ] Prove healthy siblings preserve generation.
- [ ] Preserve existing Gateway full-update drain/reconnect guarantees and preserve open Gateway safety holds across update/rollback.
- [ ] Preserve Fleet host-local profile/render validation.
- [ ] Preserve Tunnel state/config/credential and health semantics.
- [ ] Preserve Studio external-launcher activation/recovery semantics.
- [ ] Deferred actions do not consume failure budget.
- [ ] Physical failures do consume component circuit budget.
- [ ] Add forced-interruption tests around every destructive phase.

**Exit:** `auto-update-safe` can activate only qualified component transitions and cannot silently leave an old Gateway child running the replaced binary version.

## M8.5 — Periodic runtime reconciliation

- [ ] Implement manual/notify-only/auto-reconcile-safe modes.
- [ ] Preserve current startup check.
- [ ] Periodic check uses one-shot RuntimeReconciler.
- [ ] Auto apply only ManagedSafeDrift + safe_to_reconcile.
- [ ] Require global safety gate/audit/Gateway capability.
- [ ] Unknown/unmanaged/broken/local conflict never auto-overwritten.
- [ ] Persist retry/circuit counters outside M6 history.
- [ ] Repeated repair failure opens circuit.
- [ ] RollbackFailed opens hard operator hold.
- [ ] Startup after prior failed reconciliation does not loop.
- [ ] Successful stable reconciliation closes/reset circuit.
- [ ] Extend Fleet host schema/rendering for automation policy.
- [ ] Add Fleet render-plan drift and source-less tests.

**Exit:** managed-safe config drift can be repaired periodically without creating a drift/restart loop.

## M8.6 — Durable hazard holds and recovery hardening

- [ ] Add ToolClass to sanitized Gateway request telemetry.
- [ ] Add Gateway durable safety-hold store.
- [ ] Persist hold before returning terminal unknown for unsafe class.
- [ ] Read unknown outcome does not create mutation hold.
- [ ] Hold store survives Gateway restart.
- [ ] Corrupt/future hold state disables automation safety capability.
- [ ] Add capability/status hold count/list summary.
- [ ] Add explicit hold resolution control action.
- [ ] Resolution never calls/replays original tool.
- [ ] Project hold/resolution into M6 history.
- [ ] Reconcile Studio startup against Gateway hold authority.
- [ ] Harden orphan ready-staging cleanup.
- [ ] Harden ambiguous cross-process stale-lock handling.
- [ ] Add disk-free-space/staging-budget preflight.
- [ ] Preserve all component recovery journals/scratch authority.
- [ ] Add recovery-after-Studio/Gateway-forced-restart matrix.

**Exit:** unknown side-effect risk and interrupted automation survive restart as safe veto/recovery state, not as replayable jobs.

## M8.7 — Operator APIs, UI and diagnostics

- [ ] Add read-only automation status API.
- [ ] Add policy/circuit/deferral reason DTOs with no secret fields.
- [ ] Add explicit operator circuit reset endpoint.
- [ ] Add safety-hold list/resolve endpoint with same-origin enforcement.
- [ ] Add manual "check now" without changing configured policy.
- [ ] Add manual safe "activate prepared" using same SafetyGate.
- [ ] UI shows mode, next due, last result, deferral, circuit, maintenance window.
- [ ] UI clearly separates prepared from active.
- [ ] UI surfaces source-dirty / old-Gateway / safety-hold blockers.
- [ ] UI hold resolution warns that resolution does not retry.
- [ ] Add bounded sanitized diagnostics exporter.
- [ ] Secret-scan diagnostics fixtures.
- [ ] M6 audit/history projections for automation action/deferral/reset/hold resolution.
- [ ] Web realtime/reconnect tests.

**Exit:** operator can understand and safely intervene without hidden background state or privileged raw commands.

## M8.8 — Qualification and closure

- [ ] Full Rust fmt/check/Clippy/test/build/audit for affected Rust repos.
- [ ] Full Fleet tests.
- [ ] Full Studio frontend lint/typecheck/test/build.
- [ ] M5/M6/M7 regression suite.
- [ ] Manual-default backward compatibility.
- [ ] Fake-time scheduler/no-catchup proof.
- [ ] maintenance-window edge/boundary proof.
- [ ] provider outage/backoff/circuit proof.
- [ ] dirty source activation block.
- [ ] auto-prepare restart/orphan staging proof.
- [ ] Gateway unsafe unknown → durable hold → restart → resolve proof.
- [ ] old Gateway capability fail-closed proof.
- [ ] generic MCP + Gateway child coordinated activation proof.
- [ ] update while Gateway busy / drain timeout proof.
- [ ] interrupted update at each durable boundary.
- [ ] reconciliation repeated-failure circuit proof.
- [ ] disk-full/staging-budget/cleanup proof.
- [ ] Studio self-update scheduler handoff/restart proof.
- [ ] runtime-only full policy operation.
- [ ] diagnostics secret scan.
- [ ] long-running soak with bounded tasks/state/staging/disk.
- [ ] clean-source provenance manifest.
- [ ] docs/version/changelog/release evidence.
- [ ] guarded `v0.8.0-beta` release-tag evidence path; no generic-tag bypass.
- [ ] independent adversarial review before merge.

**Exit:** runtime host can run constrained unattended observation/preparation/activation/reconciliation without uncontrolled loops, hidden replay, authority inversion or unbounded state.
