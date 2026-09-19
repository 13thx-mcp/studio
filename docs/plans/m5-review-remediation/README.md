# MCP Studio — Mission 5 review remediation plan

**Status:** CLOSED — IMPLEMENTATION VERIFIED; RELEASE QUALIFICATION REMAINS SEPARATE
**Date:** 2026-09-19 (Asia/Bangkok)
**Scope:** Close review findings F1–F8, prove the fixes, and correct milestone readiness evidence.
**Studio baseline:** `8a704018d091a46b9db7adadd30170040f5fd54f`, branch `feat/m5-runtime-distribution`.
**Fleet baseline:** `48ef4a5a4625088ac2c392af7906198d2cd4c253`; clean `main`, ten commits ahead of its local remote-tracking ref at planning time. This is not a fresh remote-publication check.
**Review evidence:** `issues/m5-review-2026-09-19/REVIEW.md`, `audit-regressions.patch`, and the captured results in that directory.

This plan does not authorize a production deployment, live tunnel restart, push, merge, release tag, or version bump. Work on Aira; do not modify Mirin. Preserve the flat `bin/` and separate `runtime/` layout. Safety journals belong to M5 transaction recovery, not deferred M6 historical storage. Unattended release updates and periodic reconciliation remain outside scope.

## 1. Closure model and work order

| Work package | Scope | Depends on | Required exit evidence |
| --- | --- | --- | --- |
| M5.R0 | Permanent regressions and fixed-tree verification runner | Review baseline | Eight original reproductions retained; F8 executable counterexample added; test discovery cannot silently pass with zero tests |
| M5.R1 | F4: reconciliation serialization and atomic managed-state commit | R0 | Concurrent check/apply/adopt cannot change an uncommitted baseline; rollback restores files and manifest |
| M5.R2 | F1/F2/F3: Tunnel binding, compensation, and durable crash recovery | R1 coordination contract | Running identity matches target; post-switch failures restore safely; every tested interruption has deterministic recovery |
| M5.R3 | F5: single-authority Studio activation and rollback completion | R1, R2 safety primitives | Fleet alone finalizes activation/rollback after process-bound readiness; real backend/launcher success and failure tests pass |
| M5.R4 | F6/F7: launcher validation, loaded-config identity, and restart UI | R1, R3 readiness contract | Shadowed launcher rejected; pending restart survives checks/reconnects and clears only on proved config activation |
| M5.R5 | F8: independently expected Gateway catalog | R1; execute after R4 | Same-count wrong-name catalog rejected; valid allowlist/prefix catalogs accepted; local and remote proof remain separate |
| M5.R6 | Cross-feature verification, documentation, and closure report | R0–R5 | All finding-specific gates green on the fixed tree; remaining release proofs explicitly passed or blocked |

Run packages sequentially. A package may contain several focused commits; do not combine unrelated fixes merely to reduce commit count. R4 and R5 are separate concerns even though both touch reconciliation.

**Finding ownership:** F1→R2, F2→R2, F3→R2, F4→R1, F5→R3, F6→R4, F7→R4, F8→R5. R6 cannot close a finding solely because the general test suite passes.

## 2. M5.R0 — Preserve reproductions and establish verification

### Changes

Port the eight review assertions from the isolated snapshot into the tracked test suite. Keep their finding mapping even when names or test scaffolding change. Do not copy the renamed review package into production, and do not weaken assertions to accept the defective behavior.

Replace the review snapshot's global fsync fault flag with transaction-scoped injectable filesystem operations or an equivalent test-only dependency. Injection must support deterministic before/after-rename and durability failures without interfering with parallel tests. Do not expose a production environment variable or browser parameter for fault injection.

Add the F8 regression before implementing the catalog fix: independently configured child tools are correct, while a protocol fixture reports a different aggregate name set with exactly the same count and otherwise healthy child status. The current verifier must fail the assertion that it rejects this response. Add a positive fixture so a verifier that rejects everything cannot pass.

Introduce the planned `scripts/verify-m5-remediation.py` runner. It must test the current implementation tree, not `.tmp/m5-review-8a70401`. Leave the old pinned runner and snapshot unchanged as historical defect evidence.

### Original regression mapping

| Finding | Original assertion name |
| --- | --- |
| F1 | `audit_tunnel_running_binary_must_match_activated_target` |
| F2 | `audit_tunnel_fsync_failure_must_restore_current` |
| F3 | `audit_tunnel_restart_must_recover_unverified_same_version_swap` |
| F4 | `audit_check_must_respect_mutation_guard` |
| F4 | `audit_concurrent_check_must_not_poison_rollback_baseline` |
| F5 | `audit_self_update_requires_health_before_completed` |
| F6 | `audit_launcher_shadowing_must_fail_closed` |
| F7 | `audit_studio_restart_flag_must_survive_check` |

### Acceptance

Record expected red results against the unfixed baseline. Test discovery must confirm that each required case exists and actually ran; a missing filter, zero executed tests, or ignored required regression is a setup failure, not success. Subsequent packages turn their assigned regressions green. Overall release readiness stays blocked while any required case remains red.

## 3. M5.R1 — F4: reconciliation isolation and managed-state commit

**Source anchors:** `src/update/reconciliation.rs:656–675,715–716,826–870,948–953,1043–1048`; API reconciliation handlers in `src/api/mod.rs`.

### Design

Separate evaluation from mutation. Evaluation returns a plan/view without persisting a manifest or changing committed status. `status` remains a snapshot read. `check`, `adopt`, and `apply` share the same reconciliation operation guard whenever they could evaluate transient files, adopt a baseline, or publish state. A conflicting operation receives a typed conflict/busy response and must not replace an active operation's progress.

Preserve existing byte-for-byte synchronized legacy adoption only as an explicit guarded commit step after validation, not a hidden evaluator side effect. Unknown local edits still require explicit operator adoption.

Include the previous manifest, generation, and relevant pending-runtime metadata in the rollback snapshot. The commit boundary is reached only after file writes, rerender comparison, runtime verification, catalog verification, and durable next-manifest persistence. Failure before that boundary restores both configuration and baseline. A manifest persistence failure after file mutation must enter compensation, not escape through an unhandled early return. Cleanup failure after a durable successful commit is a cleanup warning, not permission to claim a rollback occurred.

Add a small shared runtime-operation coordinator. Control-path updates, reconciliation, Fleet bundle replacement, and conflicting lifecycle/config mutations must not overlap. Inventory reads remain available. Preserve concurrency between unrelated ordinary MCP operations through per-component leases; document a fixed acquisition order. Handlers must not recursively acquire a lease already held by their transaction. External file edits are not covered by an in-process lock: recheck recorded hashes immediately before mutation/commit and fail closed on unexpected edits.

### Tests and acceptance

Use barriers, not timing-only sleeps, to interleave check with post-write apply. Inject runtime failure and require the exact original files, baseline generation, and `ManagedSafeDrift` classification. Cover check/adopt/apply overlap, manifest-write failure, rollback-write failure, cancellation, and error/status publication. The API and two independent callers must obey the same guard; a UI pending flag is not sufficient.

No reconciliation result may report a committed generation whose fingerprints describe unverified candidate bytes. Unrecoverable restoration remains explicit and retains diagnostic/rollback material.

## 4. M5.R2 — F1/F2/F3: Tunnel transaction safety

**Source anchors:** `src/update/tunnel_update.rs:45–81,241–254,340–355,431–490,655–695,960–978,1211–1246`; `src/tunnel/mod.rs:305–325`; `src/update/inventory.rs`.

### R2.1 — F1: validate the launch contract and spawned identity

Before stopping anything, require the managed Tunnel launcher to use the server-derived `current/tunnel-client-runtime-cloudflared` indirection, canonical runtime working directory, and approved configuration binding. Comparing resolved paths once is insufficient: a fixed `releases/v1.0.0/...` path resolves to the same file before an upgrade but remains stale afterward. Reject mismatches without rewriting operator configuration.

Capture launch evidence when the supervisor spawns the child: process generation, owned child identity, resolved executable identity, and artifact fingerprint. After restart, require a new owned generation using the expected activated artifact and a successful bounded health window. Same-version reinstall must compare fingerprints, not only semantic version. Installed version, launch-attested version, and independently observable runtime health remain distinct facts; do not describe launch evidence as kernel-level attestation.

A stopped Tunnel still stays stopped. Verify installed identity and the stable future launch binding without inventing a running version.

### R2.2 — F2: compensate every possible pointer switch

Model the pointer rename as a filesystem side effect even if the following directory sync fails. On an error after a possible switch, inspect/reconcile the actual pointer under the operation guard, restore the predecessor, verify that restoration, and only then quarantine or remove candidate material. Never delete a release that `current` might still reference.

Stop ignoring restoration, restart, and cleanup errors. If rollback cannot be proven, report `RollbackFailed` or explicit recovery-required state and retain both release identities. Apply the same boundary reasoning to the same-version directory swap.

### R2.3 — F3: add a durable recovery journal

Store a schema-versioned journal outside replaceable release trees, provisionally `runtime/studio/data/tunnel-update/<transaction>.json`. Bind the journal to trusted roots and server-generated identities; use restricted permissions, regular-file checks, bounded parsing, atomic replacement, and durability checks.

Record source/target versions and fingerprints, previous pointer, candidate/rollback identities, pre-update owner state, mutation intent/substep, verification outcome, and journal revision. Paths must be validated or derived, never trusted merely because they appear in JSON.

Persist mutation intent before destructive operations. Distinguish prepared, activation-in-progress, health-verifying, committed, rolling-back, rolled-back, and recovery-failed states. Do not infer commitment from a directory's existence. Persist terminal success before removing recovery material.

On startup, recover under exclusive ownership. The default for a provably uncommitted activation is restoration of the verified predecessor; any alternate resume path must repeat identity/health verification before commitment. Restore a previously running owner only as part of the identified interrupted transaction; never start a previously stopped owner. Repeated recovery must be idempotent. Never terminate an unrelated process based on a stale PID.

Legacy scratch without a trustworthy journal must be classified separately. Preserve ambiguous candidates/backups and require operator repair rather than guessing which is known-good. An inability to persist failure state must still prevent successful activation reporting.

### Tests and acceptance

Keep the real `TunnelSupervisor` stale-binding reproduction. Add valid-current binding, same-version changed fingerprint, stale child generation, stopped ownership, failed restart, and failed rollback cases.

Inject errors and simulated process interruption before and after each destructive rename, pointer switch, sync, health check, and terminal-journal write. Reconstruct the manager from disk for every recovery case. Cover missing/corrupt journal, unsafe paths, multiple rollback candidates, and cleanup interruption.

Exit only when every tested boundary yields one of: verified target; verified restored predecessor; explicit recoverable failure with material retained. A dangling `current` or an unverified replacement labeled successful is never acceptable.

## 5. M5.R3 — F5: one authority for Studio completion

**Source anchors:** `src/update/self_update.rs:457–495,716–751,784–861`; `src/main.rs:99–121`; sibling `fleet/scripts/fleetctl.py:940–987,1094–1253`.

### Design

Make the external Fleet launcher the sole authority for terminal activation and rollback completion. Studio startup may establish local executable/configuration identity but must remain nonterminal until the external readiness proof is complete. Remove both premature `Completed` and premature `RolledBack` transitions.

Bind readiness to the intended transaction and launched process instance, exact version, and release fingerprint. A matching version returned by an unrelated listener is insufficient. Preserve the public health API; any additional activation proof is server/launcher-controlled and must not grant the browser authority over paths, PIDs, or commands.

Use a cross-process operation lock and versioned journal transition contract. Atomic file replacement alone does not serialize competing writers. Persist the handoff/fence so replacing the parent process cannot allow a second activation to overlap. Prevent stale startup data or a duplicate launcher from overwriting a newer terminal result. Acquire the launcher lease before making destructive changes, and validate launcher availability before shutdown.

Keep target and rollback readiness symmetric. Apply post-rename compensation to the Fleet Studio pointer-switch path as well; it must not rely on a boolean assigned only after a potentially failing sync returns.

Version/capability-check the Studio–Fleet contract before handoff. Define compatible handling of existing journal schemas, interrupted records, and legacy flat rollback layouts. Never reinterpret an old pending record as healthy simply because its target is running. Test the supported legacy-source readiness path; unsupported combinations must fail before shutdown instead of silently downgrading health proof.

The new backend should read external terminal transitions and publish/refetch them. Add bounded read-only polling or a journal-change notification for nonterminal UI transactions, since HTTP reconnect can occur before Fleet completes its health check. Polling status is not unattended update policy.

### Tests and acceptance

Test the real Studio startup path together with the Fleet launcher in a temporary runtime root: successful target; delayed readiness; listener bind failure; startup/config failure; target crash; wrong-process health response; launcher interruption; duplicate resume; valid rollback; rollback health failure; and supported legacy flat migration.

Before readiness, transaction lookup must remain nonterminal even if the executable/version/pointer match. Only the owner of the activation journal lease can commit success. No early terminal state may cause the UI to discard a still-active transaction.

**Repository/deployment dependency:** Implement Fleet changes on a dedicated support branch from its verified current HEAD; do not edit protected `main` directly or rewrite its existing ten local commits. Record the tested Studio/Fleet commit pair. Upgrading a deployed Fleet contract, when later authorized, precedes enabling the corresponding Studio self-update path.

## 6. M5.R4 — F6/F7: launcher and restart-state truthfulness

### F6 — Strict validate-only launcher contract

**Source anchor:** `src/update/reconciliation.rs:1540–1583`.

Replace substring acceptance with a small explicitly supported launcher template/structured contract. Validate the complete executable form with only server-derived root substitutions. Accept a legacy form only when the entire form and root derivation are understood; do not implement a general shell interpreter.

Reject canonical text in comments, later assignments, changed root derivation, alternate exec paths, additional overriding arguments, shell expansion/eval constructs, and unrecognized forms. A failure reports a validate-only launcher conflict with a safe reason. Do not rewrite or adopt `run.sh` automatically; this plan does not change its ownership.

Test legitimate supported fixtures as well as shadowing, comment-only matches, duplicate arguments, symlinks, and wrong-root forms. A missing helper must not be presented as proof that an existing helper was inspected.

### F7 — Loaded configuration is separate from generated files

**Source anchors:** `src/update/reconciliation.rs:798–808,1031–1051,1099–1114`; `src/main.rs`; configuration loading; `web/src/UpdatesPanel.tsx`, `web/src/types.ts`, `web/src/App.tsx`.

Record the canonical config source and digest of the exact bytes parsed at startup, together with the process instance. Avoid hashing a second read that might differ from the parsed content.

Track the reconciled Studio-config digest/generation separately from the running instance's loaded digest. Compare the Studio surface itself, not just the global reconciliation generation: an unrelated Gateway-only change must not create a false Studio restart requirement.

Persist pending restart state across checks and reconnects. Clear it only when a ready process proves it loaded the expected configuration. A restart with the wrong config path or old bytes cannot clear it. Keep file convergence, runtime activation, and remote-client freshness separate in the API.

Render a visible Studio restart-required warning in Updates/Fleet, including the affected surface and applicable safe operator action. Do not add an unimplemented restart button. Also expose rollback/recovery-required and nonterminal self-update states accurately.

### Acceptance

The original F6/F7 assertions pass, plus backend/API/UI tests for repeated checks, browser reconnect, process restart with correct/incorrect config, and Gateway-only changes. Unknown launcher/config identity never appears as verified synchronization.

## 7. M5.R5 — F8: independently verify the expected Gateway catalog

**Source anchors:** `src/update/gateway.rs:98–110,325–409,417–497`; `src/update/reconciliation.rs:779–785,1031–1051`; contract in `docs/plans/m5-runtime-distribution/M5.9A-runtime-reconciliation.md:210–229`. Reference semantics: sibling `gateway/src/main.rs`, especially `ChildConfig`, `start_child`, `tool_is_exposed`, and `exposed_tool_name`.

### Design

Build the expected catalog independently from the Gateway under test. Snapshot the trusted child definitions, including enabled state, command, arguments, environment contract, timeout, `tool_allowlist`, and `tool_prefix`. Directly initialize enabled child MCPs and retrieve their complete `tools/list` using bounded handshakes, pagination, output budgets, concurrency, and cleanup. Disabled children must not start. Use only trusted deployed runtime executables; do not call user tools or require source/build toolchains on the runtime host.

Match the actual contract: an empty allowlist exposes all original names; a nonempty allowlist matches original names before prefixing; the prefix is concatenated exactly, with no invented separator. Unknown allowlist entries are errors. Reject duplicate original/exposed names, invalid names, cross-child collisions, and collisions with Gateway builtins before reducing results to a set. Match effective argv/environment/working context between child discovery and the isolated Gateway probe without logging secrets.

Construct the expected builtin-plus-child set and compare it with the independently observed Gateway set. Capture bounded missing/unexpected-name diagnostics and deterministic expected/observed fingerprints. Check configuration/artifact identity across the probe window. If discovery is unstable or unavailable, report unverified/failure rather than silently accepting counts.

An observed fingerprint change is not automatically a defect: a legitimate child release may change its catalog. Accept it only when independent expected and observed sets agree. An equal-count but wrong-name aggregate must fail and trigger transaction rollback where mutation has occurred.

Reuse the verifier for Gateway update health and reconciliation. Identify its proof scope as an isolated local Gateway probe; do not claim direct live tunnel-owned Gateway inspection or remote-client refresh without a real acknowledgement. Keep the Gateway repository read-only unless an implementation necessity is separately justified; contract fixtures can guard compatibility without adding a source dependency to deployed Studio.

### Tests and acceptance

The R0 same-count substitution fixture now fails verification as intended. Cover missing/extra names, allowlist-before-prefix behavior, disabled children, builtin collisions, duplicates, paginated children, timeouts, changed config during probing, and legitimate catalog changes.

Add an isolated real Gateway plus child-protocol integration test and require clean process teardown. Missing expected tool identity must prevent successful update/reconciliation commitment. Correct expected/observed equality must still pass while remote freshness remains unknown or refresh-pending.

## 8. M5.R6 — Verification, evidence, and readiness

### Fixed-tree runner contract (to implement in R0)

The new runner is a planned deliverable, not a script already supplied by this planning change. It must capture Studio and Fleet commits, branch/worktree state, tracked diffs or hashes for development runs, lockfile hashes, toolchain versions, OS/architecture, selected profile, command arguments, exit status, duration, and raw logs. Do not dump the entire environment or host secrets.

Write each run under `issues/m5-remediation/results/<unique-run-id>/`, with `summary.json`, `summary.tsv`, `run.log`, per-command logs, and an archive suitable for handoff. Save partial results on failure/interruption. Run commands serially, continue after failed gates, and terminate only processes created for that run on timeout. Detached fixture children need explicit cleanup; broad production process-name matching is forbidden.

Use a dedicated fixed-tree Cargo target directory or a fresh per-run target. Never silently reuse the renamed pinned review package. Before/after provenance must detect source changes during execution. Development runs can record dirty trees; closure/release evidence requires a clean committed pair. Missing tools or mandatory tests are failures, not skips presented as PASS.

Provide separate core, isolated-integration, security, and native-package profiles. Smokes are a reviewed allowlist with explicit temporary-root safeguards, not blanket `cargo test -- --ignored`. Network downloads and native package builds are opt-in. Run only lightweight checks through MCP during implementation; provide the longer command as a script for the operator and read its saved output before marking the corresponding gate complete.

### Gates

| Layer | Required evidence |
| --- | --- |
| Finding-specific | All eight original assertions plus F8 and added boundary tests pass on the fixed tree; none are ignored |
| Rust | Format, check, Clippy with warnings denied, all-target/all-feature tests and build; locked dependencies |
| Frontend | Lint, typecheck, tests, production build; restart/reconnect/recovery UI assertions |
| Fleet | Python compile and full unit suite; launcher contract compatibility tests |
| Cross-feature | Gateway↔Tunnel, Fleet↔reconciliation, self-update↔other mutations, and MCP update↔catalog-probe interleavings |
| Recovery | Fault/interruption matrix, stopped/running preservation, rollback failure, journal corruption, repeated recovery and teardown |
| Real isolated runtime | Real Studio+Fleet launcher and real Gateway+children in fresh temporary roots |
| Security | Trusted paths, tampered journal/artifacts, symlink/link rejection, secret-safe responses/logs, same-origin mutation protection, dependency audit |
| Release qualification | Native darwin-amd64 and darwin-arm64 packages; extracted backend/web identity; published-artifact source-less bootstrap and a genuine controlled update/rollback |

A simulated syscall error does not prove real hardware power-loss durability. Record the tested failure model, filesystem/platform, and native coverage explicitly. Never describe one architecture or copied runtime fixtures as proof of both native packages or published-artifact bootstrap.

### Documentation and Git

Update architecture/threat-model documentation and ADRs for transaction journals, operation ownership, single-authority self-update completion, loaded-config identity, and catalog verification. Record user-visible changes in the unreleased changelog. Preserve the original review report and evidence as historical facts.

Continue Studio on `feat/m5-runtime-distribution` with focused Conventional Commits. Create the Fleet support branch only when implementing its changes. No history rewriting or pushing is implicit. Reconcile with the current base and check merge readiness only at approved final closure; do not force an arbitrary commit-count limit by rewriting the existing M5 history.

Correct `docs/milestone-5-status.md` when implementation work starts to show review remediation open. After fixes, distinguish **IMPLEMENTATION VERIFIED** from **RELEASE QUALIFIED**. Previously recorded release-endpoint failures remain historical until freshly rechecked. Unavailable published assets block release qualification, not the ability to complete and verify code fixes. No release version, tag, publication, or merge follows automatically from this plan.

## 9. Definition of done and next-session handoff

Each finding was kept OPEN until its corrective code, permanent regression, relevant integration proof, and documentation were all present. The closure matrix records the fixing commit(s) and exact final evidence directories; R6 does not close any condition by citing the old passing baseline.

| Finding | Status | Fix package | Fix commit / evidence |
| --- | --- | --- | --- |
| F1 | CLOSED | R2 | Studio `d778b58`; regression `issues/m5-remediation/results/20260919T140643Z-18307` + core/security closure |
| F2 | CLOSED | R2 | Studio `d778b58`; regression `issues/m5-remediation/results/20260919T140643Z-18307` + core/security closure |
| F3 | CLOSED | R2 | Studio `d778b58`; crash-recovery regression `issues/m5-remediation/results/20260919T140643Z-18307` + isolated closure |
| F4 | CLOSED | R1 | Studio `49e1a9f`; isolation regressions `issues/m5-remediation/results/20260919T140643Z-18307` + core closure |
| F5 | CLOSED | R3 | Studio `e8506ce`, `57f6240`; Fleet `5e8b3b6`, `8b82752`; regression `issues/m5-remediation/results/20260919T140643Z-18307`, core `issues/m5-remediation/results/20260919T140854Z-21177`, isolated launcher proofs `issues/m5-remediation/results/20260919T141541Z-31839` |
| F6 | CLOSED | R4 | Studio `d30a4ca`; launcher regression `issues/m5-remediation/results/20260919T140643Z-18307` + core/security closure |
| F7 | CLOSED | R4 | Studio `d30a4ca`; restart regression `issues/m5-remediation/results/20260919T140643Z-18307` + core/UI closure `issues/m5-remediation/results/20260919T140854Z-21177` |
| F8 | CLOSED | R5 | Studio `39bb9cf`; exact-set regression `issues/m5-remediation/results/20260919T140643Z-18307` + real Gateway/children proof `issues/m5-remediation/results/20260919T141541Z-31839` |

### Final R6 closure — 2026-09-19

The fixed-tree closure pair is Studio `a388cf2611fead432e79ae041112d761632cbc00` and Fleet `8b827521fdcb423b1c87e70d3885d518536842fd`. Every final profile started and ended with clean worktrees and `source_changed_during_run=false`.

| Profile | Result | Evidence |
| --- | --- | --- |
| Finding regressions | PASS — all 10 required exact tests executed | `issues/m5-remediation/results/20260919T140643Z-18307` |
| Core + Fleet | PASS — fmt/check/Clippy/tests/build, frontend lint/typecheck/test/build, Fleet compile + 20 tests | `issues/m5-remediation/results/20260919T140854Z-21177` |
| Security | PASS — `cargo audit`, tamper, symlink and origin filters | `issues/m5-remediation/results/20260919T141313Z-27968` |
| Isolated integration | PASS — 5 reviewed real/isolated smokes | `issues/m5-remediation/results/20260919T141541Z-31839` |
| Native package | PASS for **darwin-arm64 only** | `issues/m5-remediation/results/20260919T141810Z-37530` |

Native packaging produced `mcp-studio-v0.4.0-darwin-arm64.tar.gz` and verified the extracted backend/web identity. The manifest explicitly records `cross_architecture_release_qualified=false`; no darwin-amd64 native proof was run in this closure.

A final source review after the green matrix found no additional remediation finding: the F1–F8 invariants remain present, no TODO/FIXME/HACK marker exists in the remediation-critical paths, both worktrees are clean, and local `main` is an ancestor of both topic branches with no upstream-side divergence at review time. This establishes **implementation verification and local merge readiness**, not release qualification.

Remaining release qualification work is intentionally outside remediation closure: native darwin-amd64 packaging on its native runner, current published-project release endpoint/artifact availability, source-less bootstrap from published artifacts, a genuine published project update/rollback, and the eventual version/tag/publication sequence. No push, merge, tag, deploy, or version bump was performed by remediation.

For each implementation session, read this plan, `docs/plans/m5-runtime-distribution/COMMON.md`, the original review, and the current code for the active package. Inspect both repositories before editing. Implement only the requested package, record red/green test evidence and remaining blockers, then stop with a handoff. Long verification work goes into the logged operator-run script rather than an unbounded tool call.

**M5.R0–M5.R6 are now closed for implementation remediation.**
