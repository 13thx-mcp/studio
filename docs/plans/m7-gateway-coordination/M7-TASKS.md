# M7 Dependency-Ordered Execution Checklist

**State:** M7.0 FROZEN / P-01..P-09 CLOSED / M7.1–M7.8 IMPLEMENTED / M7.9 QUALIFIED / CLOSURE FIXES INTEGRATED / LOCAL RELEASE TAGS RECONCILED / PUBLICATION PENDING.

**P-01..P-09 closure checkpoint:**

- Studio planning/contracts: `653d2e6` (plus prior `7c93d1c`, `4f3f0d2`); implementation merge `1c5dac5`; runtime-only closure fix `543a6c4`; final closure merge on `main` `3423d8e`;
- Gateway protocol/control-socket evidence: `6397661` + `60a0c00`; implementation merge `5b886c9`; bounded-soak qualification `da432d7`; final qualified `main` `36aea90`;
- Filesystem M7.4 implementation: `ec68a04`, version 0.2.0;
- Fleet M7 policy implementation: `7bad5e2`, version 0.3.0; unrelated Sonar work remains isolated on its feature branch;
- local release tags are reconciled to the qualified release commits and verified unpublished; push, release publication and deployment are not implied by this checkpoint.

## M7.0 — Architecture, protocol and configuration freeze

- [x] Resolve P-01 through P-09 from SOURCE-AUDIT — P-01..P-09 now have frozen design/test dispositions.
- [x] Verify the exact locked rmcp API/protocol behavior with a minimal spike — rmcp 3.4.0, 7 permanent tests PASS.
- [x] Freeze request/outcome state machine and retry guidance — ADR 0026 / rmcp spike.
- [x] Freeze scheduler/backpressure/fairness algorithm and bounds — ADR 0028.
- [x] Freeze drain API/state and the Studio/Fleet caller contract — ADR 0027; P-06 viability 3/3 PASS.
- [x] Freeze per-child generation/restart/circuit state — ADR 0029.
- [x] Freeze Gateway global-policy and child-policy schemas plus Fleet migration — ADR 0031.
- [x] Freeze Filesystem v2 tool schemas/error model/CAS atomicity — ADR 0030.
- [x] Freeze payload/artifact bounds — ADR 0033.
- [x] Freeze profile/tool classification and workspace alias semantics — ADR 0034.
- [x] Freeze Gateway→M6 event DTOs and privacy boundary — ADR 0035.
- [x] Freeze tunnel full-client vs run-only artifact architecture — ADR 0032; v0.0.14 checksum-verified full-client probe PASS.
- [x] Resolve Gateway Node-marker quality false-fail — markers are local/excluded; use explicit Rust gates.
- [x] Record independent component version targets — Studio 0.7.0-beta, Gateway 0.2.0, Filesystem 0.2.0, Fleet 0.3.0; upstream tunnel target v0.0.14.
- [x] Add required ADR set before implementation.

**Exit:** no unresolved architectural ambiguity can cause side-effect replay, unsafe restart, authority inversion, or config migration uncertainty.

### M7.0 closure audit

The post-P-gate contracts are now frozen by ADR 0033–0035 and reconciled with ADR 0026–0032.

- [x] Payload/artifact bounds and privacy — ADR 0033.
- [x] Profiles/resources/progress boundary — ADR 0034.
- [x] Workspace aliases explicitly dropped from M7 — former alias verification rows R34/R35 retired by contract.
- [x] Gateway→M6 sanitized telemetry/privacy DTO — ADR 0035.
- [x] Final ADR/COMMON/verification reconciliation.
- [x] Version targets and downgrade/migration boundaries recorded.

M7.0 is frozen. Current implementation/qualification evidence is tracked in [M7-QUALIFICATION.md](M7-QUALIFICATION.md).

## M7.1 — Request coordination core

- [x] Refactor Gateway out of the monolithic request path into testable modules.
- [x] Add server-generated request/correlation IDs.
- [x] Add active request registry.
- [x] Add bounded queue and central scheduler.
- [x] Add global/per-child/per-tool active limits.
- [x] Add queue deadline/timeout.
- [x] Implement deterministic capacity/policy rejection.
- [x] Separate upstream wait from owned child-call observation.
- [x] Implement pre-dispatch cancellation/deadline guarantee.
- [x] Implement post-dispatch pending/unknown semantics.
- [x] Ensure mutation failures never auto-replay.
- [x] Add concurrency/fairness/cancel/disconnect integration tests.

**Exit:** request lifecycle semantics are independently testable and bounded.

## M7.2 — Drain and lifecycle integration

- [x] Add Gateway drain state machine and status surface.
- [x] Reject new mutations when drain begins.
- [x] Define safe-read behavior during drain.
- [x] Wait for dispatched mutations to known terminal state.
- [x] Fail drain timeout closed.
- [x] Make Gateway reload request-aware.
- [x] Make config watcher consume drain policy where needed.
- [x] Add Studio Gateway update drain prerequisite before stop/replace.
- [x] Add reconciliation-triggered Gateway reload drain prerequisite.
- [x] Define Fleet activation/update interaction with Gateway drain.
- [x] Preserve all M5 rollback/catalog proofs.

**Exit:** destructive Gateway lifecycle operations cannot silently interrupt active mutations.

## M7.3 — Child resilience

- [x] Introduce per-child runtime generation ownership.
- [x] Restart only the failed/changed child when possible.
- [x] Add bounded exponential backoff.
- [x] Add max attempts/consecutive failures.
- [x] Add circuit-open/cooldown/retry-at.
- [x] Block unsafe manual restart while active dispatched work exists.
- [x] Rebuild catalog atomically after one-child recovery.
- [x] Prove healthy sibling generations remain unchanged.
- [x] Add crash-loop and catalog-refresh failure tests.

**Exit:** no unbounded restart loop and no unnecessary sibling restart.

## M7.4 — Filesystem v2

- [x] Add `file_metadata`.
- [x] Add bounded `read_text_file_range`.
- [x] Add bounded `search_text`.
- [x] Add SHA-256 revision identity.
- [x] Extend reads to return revision metadata without breaking confinement.
- [x] Add `expected_revision` CAS to replacement writes.
- [x] Add bounded deterministic `patch_text_file`.
- [x] Add typed stale-revision conflict.
- [x] Preserve cap-std confinement and atomic replacement.
- [x] Add external-edit and concurrent-patch conflict tests.
- [x] Add range/search byte/line/match bound tests.

**Exit:** stale external changes cannot be silently overwritten.

## M7.5 — Payload budgets and artifacts

- [x] Add bounded request-size validation.
- [x] Add response structured/text/binary budgets.
- [x] Add response guard metadata and previews.
- [x] Add narrow-retry guidance.
- [x] Add optional ephemeral spill store.
- [x] Add TTL, per-artifact and total-disk bounds.
- [x] Add bounded artifact retrieval and cleanup.
- [x] Ensure sensitive payload persistence is opt-in only.
- [x] Add oversized text/JSON/base64 tests and disk-pressure cleanup tests.

**Exit:** one child response cannot flood client context or local disk unchecked.

## M7.6 — Profiles, workspace context, resources and progress

- [x] Implement server-owned tool classification.
- [x] Implement named profiles and active profile generation.
- [x] Preserve child base allowlist as a hard ceiling.
- [x] Emit `tools/list_changed` on profile/catalog changes.
- [x] Keep control-plane mutations out of normal coding profile.
- [x] Workspace alias registry/binding — **RETIRED by ADR 0034**; aliases are not part of the M7 surface.
- [x] Alias confinement requirement — **N/A after ADR 0034 retirement**; explicit target MCP confinement remains authoritative.
- [x] Aggregate `resources/list` and `resources/read` with collision policy.
- [x] Forward progress with token/correlation mapping.
- [x] Prove progress cannot mark operation success.

**Exit:** least-privilege routable surface is explicit and protocol extensions remain bounded.

## M7.7 — Gateway telemetry into M6

- [x] Define typed sanitized Gateway history events.
- [x] Ingest request outcome/latency/queue metrics.
- [x] Ingest response guard metrics.
- [x] Ingest child restart/circuit events.
- [x] Record catalog/profile fingerprints/generations.
- [x] Add p50/p95 aggregation from observed data.
- [x] Preserve M6 no-payload default.
- [x] Ensure history failure does not alter live request outcome.
- [x] Add retention/restart/history-unavailable tests.

**Exit:** Gateway request behavior is historically observable without turning SQLite into live authority.

## M7.8 — Secure MCP Tunnel native runtime adapter

- [x] Reconfirm latest official tunnel-client release and supported CLI at implementation start.
- [x] Choose full-client/runtime dual-artifact or full-client release-unit architecture — dual-artifact, ADR 0032.
- [x] Extend trusted release asset selection/checksum/version verification accordingly.
- [x] Add constrained adapter for the frozen supported operations.
- [x] Model liveness/readiness/control-plane poll health separately.
- [x] Preserve config and credential bytes.
- [x] Preserve M5 transactional activation/rollback guarantees.
- [x] Test running and stopped upgrade/rollback.
- [x] Test source-less operation with no source checkout.

**Exit:** Studio/Fleet use upstream runtime management without weakening release/update authority.

## M7.9 — Qualification and closure

- [x] Full Rust quality on Gateway/Filesystem/Studio and affected Fleet tests.
- [x] Frontend checks for any Studio UI additions.
- [x] Exact cancellation-before-dispatch proof.
- [x] Timeout/disconnect-after-dispatch pending/unknown proof.
- [x] Drain with mixed reads/mutations.
- [x] Queue saturation/fairness/timeout.
- [x] Crash-loop/circuit/healthy-sibling isolation.
- [x] Filesystem CAS/patch external-conflict proof.
- [x] Oversized response/artifact TTL/disk-bound proof.
- [x] Profile/list-changed and workspace confinement proof.
- [x] Resources/progress proof.
- [x] M6 telemetry-without-payload proof.
- [x] Tunnel adapter health/readiness/poll-health proof.
- [x] Long-running soak/fault injection — Gateway qualification `da432d7`: 4,000 bounded admissions plus permanent cancellation/crash/drain/artifact-pressure fault tests.
- [x] Runtime-only/source-less smoke — pre-fix merge exposed a real surface-count defect; Studio `543a6c4` fixes it and the explicit runtime-only smoke PASSes.
- [x] M5/M6 regression suites.
- [x] Clean-source provenance — detached exact-merge audit worktrees were clean; see M7-QUALIFICATION.
- [x] Documentation/changelog/version updates — version targets are present; qualification notes local tags are pre-closure/unpublished pending reconciliation.
- [x] Independent review before closure — Filesystem CAS/confinement/cleanup corrections plus this closure audit's runtime-only and soak findings are recorded; final release integration remains pending.

## Branch/repository discipline

- Use separate work branches per affected repository.
- Do not implement M7 Fleet changes on the current unrelated SonarQube feature branch.
- Keep Studio planning docs reviewable independently from Gateway/Filesystem production commits.
- Cross-repo integration tests must record exact dependency commit IDs.
- Do not publish/tag/deploy merely because implementation tests pass.
