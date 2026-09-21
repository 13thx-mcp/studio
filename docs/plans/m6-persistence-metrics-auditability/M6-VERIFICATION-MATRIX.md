# M6 requirement → implementation → test → evidence matrix

**Final execution state (2026-09-21):** M6 is **VERIFIED / CLOSED** and merged to `main` at `e10ee57b6c1629744e45ad7eda63cebf368b7504`. Full clean-source qualification passed on native darwin-arm64 and consumed independently qualified M5 publication evidence.

**Final local evidence target:** `.tmp/m6-history/final-review/summary.json` and `.tmp/m6-history/final-review/manifest.json`. This path is intentionally bound in documentation before the final rerun so the evidence can be generated after the last tracked source/doc change without editing the tested source afterward.

## Requirements

| ID | Requirement | Current implementation / authority | Permanent proof | Disposition |
|---|---|---|---|---|
| R01 | Authority split and no registry migration | `src/storage` records projections only; `src/registry` remains live/file authority | `storage::tests::history_never_rehydrates_registry_authority`; `written_config_is_not_loaded_config` | **PASS (local)** |
| R02 | Live state is not historic PID/state | Separate historical DTOs; no action fields/PID signaling path | `storage::tests::historical_pid_never_authorizes_signal` | **PASS (local)** |
| R03 | Recovery journals remain independent | Tunnel/Self-update journals remain M5 authority; audit failure cannot retain a journal only for history | `update::tunnel_update::tests::history_failure_does_not_retain_recovery_journal`; `tunnel_terminal_receipt_precedes_journal_cleanup` | **PASS (local)** |
| R04 | Bundled qualified SQLite and pinned dependency | `rusqlite=0.40.2` bundled SQLite; startup version/source qualification | `storage::tests::linked_sqlite_version_and_source_id_are_qualified` | **PASS (native arm64/current host)** |
| R05 | One writer, two bounded readers, no executor blocking | Dedicated writer + 2 readers; bounded queues; Axum receipt waits use `spawn_blocking` | `writer_serializes_concurrent_producers`; strict Clippy/source review | **PASS (local)** |
| R06 | Safe DB path, mode and store lock | Fixed runtime history path, private modes, symlink/hardlink/owner checks, owner lock | `unsafe_history_paths_fail_closed`; `hardlinked_history_database_is_rejected`; `second_writer_is_rejected_while_owner_lock_is_held` | **PASS (local)** |
| R07 | Empty/prior schema migration and checksums | Frozen v1 migration/checksum/application-id gate; future schema rejected | `migration_checksum_and_atomic_upgrade`; `rejects_future_schema_and_tampered_migration` | **PASS (local)** |
| R08 | Busy/corrupt/ENOSPC/read-only/timeout failure behavior | Degraded fail-closed admission; no source auto-repair; real disposable filesystem tests | `store_faults_leave_live_safety_available`; `sqlite_full_degrades_history_and_blocks_new_admission`; `corrupted_main_db_fails_closed_without_auto_repair`; `malformed_wal_fails_closed_without_auto_repair`; `second_writer_is_rejected_while_owner_lock_is_held`; `scripts/verify-m6-enospc.py`; `scripts/verify-m6-readonly.py` | **PASS (Darwin current host)** |
| R09 | Durable admission before discretionary action | Typed operation admission transaction precedes dispatch | `storage::tests::admission_failure_prevents_discretionary_effect` | **PASS (local)** |
| R10 | Post-effect audit failure does not roll back/retry domain effect | Effect/audit headers separated; terminal receipt failure is warning/incomplete audit | `post_effect_history_failure_does_not_retry_action`; API audit-header test | **PASS (local)** |
| R11 | Safe stop/recovery/exit bypass failed history | MCP/Tunnel stop/shutdown do not require history availability | `tests/supervisor_lifecycle.rs::shutdown_completes_with_stalled_history`; `tests/tunnel_lifecycle.rs::tunnel_shutdown_completes_with_stalled_history` | **PASS (local integration)** |
| R12 | No SQL while owner state mutexes are held | Owner facts captured, locks released, then history writer called; storage has no owner callbacks | lifecycle terminal-before-start/stalled-history tests + final lock-order source review | **PASS (review + behavior)** |
| R13 | P-01 generic MCP apply coordinator | `McpUpdateManager::apply` acquires shared component lease before local transaction guard | `update::transaction::tests::mcp_apply_respects_control_lease`; `mcp_apply_blocks_reconciliation_until_terminal` | **PASS** |
| R14 | P-02 registry whole-mutation serialization | Registry mutation mutex spans snapshot → persist → replace | `registry::tests::registry_concurrent_mutations_preserve_both_changes` | **PASS** |
| R15 | Stable typed event/phase taxonomy | `src/storage/events.rs` closed enums + separate requested/admitted/terminal evidence | `request_is_never_terminal_success`; `all_required_action_families_have_typed_receipts` | **PASS** |
| R16 | Idempotency/conflicting duplicate integrity | Unique stream/ordinal + journal revision/digest checks | `conflicting_source_ordinal_is_rejected`; `validated_journal_revision_conflicts_and_gaps_are_explicit` | **PASS** |
| R17 | Concurrent ordered state projections | Terminal-before-start reducer cannot reopen session | `terminal_before_start_cannot_reopen_session` | **PASS** |
| R18 | Session start/stop/restart persistence | Supervisor/Tunnel generation hooks and durable sessions across restart | `sessions_survive_studio_restart`; supervisor/tunnel lifecycle integrations | **PASS** |
| R19 | Interrupted observer run without invented child exit | Old open run/session/operation/apply become interrupted/indeterminate; explicit unknown coverage interval | `interrupted_observer_does_not_invent_child_exit`; `interrupted_apply_becomes_indeterminate_after_observer_restart` | **PASS** |
| R20 | Time and numeric safety | Checked conversions; decimal-string browser sequences; wall-clock discontinuity explicit | `clock_jump_and_large_sequence_are_explicit`; web >2^53 ordering test | **PASS** |
| R21 | Required audit action families integrated | Typed action list + Axum admissions/terminal receipts for discretionary action families | `all_required_action_families_have_typed_receipts`; API audit tests | **PASS (local)** |
| R22 | Prepare/apply success/failure history | All current update domains dispatch through typed observation; prepare failure normalized | `storage::update_history::tests::prepare_failure_is_not_install_failure`; full update-manager regressions | **PASS (local)** |
| R23 | Rollback normalization and verified LKG | Failed target vs verified restoration separated; proof scope preserved | `rollback_success_is_not_target_install_success`; artifact lineage test | **PASS** |
| R24 | Same-version content identity and P-03 cleanup ordering | Validated Tunnel journal watermark before cleanup when healthy; history failure never retains recovery journal | `tunnel_terminal_receipt_precedes_journal_cleanup`; `history_failure_does_not_retain_recovery_journal`; M5 F1–F3 regressions | **PASS** |
| R25 | Self-update across processes / Fleet terminal authority | Startup finalization consumes Fleet terminal authority only; external launcher success/failure exercised | `startup_does_not_finalize_external_activation_without_fleet_terminal_state`; `audit_self_update_requires_health_before_completed`; ignored external-launcher success/failure smokes run explicitly | **PASS (native arm64/current host)** |
| R26 | Artifact/install/active/previous lineage | Safe hash-only artifact identity + members; staged/verified-active/restored observations and honest unknown predecessor | `artifact_stage_and_verified_lineage_are_content_addressed`; `active_lineage_reports_unknown_predecessor` | **PASS** |
| R27 | Registry/config revision and loaded-vs-written identity | Loaded Studio digest separate from committed registry safe projection | `written_config_is_not_loaded_config`; `config_revision_return_to_old_content_is_new_observation` | **PASS** |
| R28 | Reconciliation commits/drift P-04 | Production check/adopt/apply API observations linked to admitted operation | `api::tests::reconciliation_check_side_effect_is_audited`; M5 reconciliation regressions | **PASS** |
| R29 | MCP/Tunnel crash/restart definitions P-05 | MCP spontaneous 0 = clean; Tunnel unrequested 0 = crash; persisted lifecycle facts feed metrics | `mcp_clean_zero_exit_is_not_crash_in_history`; `tunnel_clean_zero_exit_is_still_crash_in_history`; `aggregation_retry_is_idempotent` | **PASS** |
| R30 | Update check counts / stale cached result | Failed provider check emits error without claiming cached availability as fresh | `failed_check_cached_latest_is_not_fresh_availability` | **PASS** |
| R31 | Transactional aggregation/no double count | Metrics contributions + watermark commit atomically; retries idempotent | `aggregation_retry_is_idempotent`; `housekeeping_crash_before_commit_is_all_or_nothing` | **PASS** |
| R32 | Unknown coverage/exact duration/LKG scope | Interrupted coverage is partial/unknown; no exact duration fabricated | `partial_history_is_not_zero_or_exact_duration` | **PASS** |
| R33 | Finite retention/audit pressure | 90/365-day policy, 500-row batches, retention epoch, active-record protection, crash-before-commit rollback | `retention_preserves_unexpired_audit_and_bounds_lineage`; `housekeeping_crash_before_commit_is_all_or_nothing` | **PASS (local)** |
| R34 | Bounded long history/WAL/readers | 200-row max pages, 2×16 read queues, page/event admission limits, real SQLITE_FULL/ENOSPC | `long_reader_cannot_create_unbounded_history_work`; `sqlite_full_degrades_history_and_blocks_new_admission`; real ENOSPC fixture | **PASS (current host)** |
| R35 | Pagination snapshot/retention stability | Cursor binds DB epoch, retention epoch, high-water sequence, frozen default range and filter digest | `history_cursor_survives_insert_and_expires_on_prune` | **PASS** |
| R36 | Bounded filters/no query authority | Fixed SQL templates, bounded limits/ranges/IDs, opaque cursors | `history_query_abuse_is_bounded` | **PASS** |
| R37 | Secret/privacy/origin DTO boundaries | Closed history DTOs; no raw path/env/argv/error; same-origin/Host/no-store/CORP | `history_redaction_covers_db_api_and_backup`; API origin tests; web tests | **PASS** |
| R38 | Realtime post-commit/reconnect | Coalesced committed watermark; subscribe before snapshot; REST remains authoritative | `history_reconnect_has_no_missed_commit` | **PASS** |
| R39 | Timeline/charts/update/config/drift/lineage UI | Read-only History panel with tabular/SVG views and partial/expired labels | `web/src/history/history.test.tsx` (4 tests); full web suite | **PASS** |
| R40 | M5 bootstrap/no invented past | Bootstrap event is observation only; no fake operation/update/session | `m5_bootstrap_creates_observations_not_fake_events` | **PASS** |
| R41 | Consistent backup/restore/no auto repair | Online backup + verify CLI; corruption/WAL fail closed | `wal_backup_restore_preserves_committed_history`; source-less package backup smoke | **PASS** |
| R42 | Binary rollback/self-update DB compatibility | Future schema rejected without modification; actual committed pre-M6 binary built and tested against current v1 DB | `binary_rollback_never_downgrades_history_schema`; `scripts/verify-m6-binary-rollback.py` | **PASS (native arm64/current host)** |
| R43 | Source-less legacy/versioned runtime | Fixed release-independent history path; release binary/web package runs with no source/node_modules; runtime-only reconciliation/real binary smokes | `runtime_only_history_path_is_release_independent`; `scripts/verify-m6-runtime-only.py`; explicit ignored runtime-only/real-binary smokes | **PASS (native arm64/current host)** |
| R44 | M7/M8 exclusions and M5 publication separation | Housekeeping storage-only; no auto-update/reconcile policy; M5 publication gate remains independent | `history_housekeeping_never_invokes_runtime_policy`; status/runner Q16 handling | **PASS; Q16 independently qualified** |

## Fault/restart/update evidence notes

R08 now has explicit current-host proof for empty/prior/future/tampered/foreign/corrupt DB, malformed WAL, owner-lock contention, SQLite page-limit FULL, real disposable filesystem ENOSPC, real disposable read-only filesystem, unsafe path and post-failure admission behavior. The real filesystem fixtures use user-owned temporary HFS+ disk images and never touch installed runtime state.

R18/R19 include clean restart, interrupted observation, terminal-before-start ordering and safe shutdown while history is unavailable. R22–R25 are backed by current update-manager regressions plus explicit Tunnel source-journal separation and external self-update launcher success/failure smokes. No SQLite state is permitted to resume update/recovery actions.

R31/R33 inject a failure after aggregation/retention mutations but before COMMIT and prove event membership, metrics watermark and retention epoch all roll back. R34 combines bounded query pagination with finite page/event pressure and real full-filesystem behavior; this is a correctness/boundedness claim, not an unmeasured throughput claim.

R35 binds cursors to immutable high-water/range/filter/epoch state. R37 checks DB event payload, history API DTO and SQLite backup against a secret/path corpus. R43 runs the release binary and web assets from a disposable source-less runtime and explicitly runs the existing runtime-only reconciliation/real-binary smokes.

## Existing M5 permanent regressions

These exact tests are executed fresh by the M6 `regressions` profile; a zero-test filter is a verifier failure.

| Finding | Exact test |
|---|---|
| F1 | `update::tunnel_update::tests::audit_tunnel_running_binary_must_match_activated_target` |
| F2 | `update::tunnel_update::tests::audit_tunnel_fsync_failure_must_restore_current` |
| F3 | `update::tunnel_update::tests::audit_tunnel_restart_must_recover_unverified_same_version_swap` |
| F4 | `update::reconciliation::tests::audit_check_must_respect_mutation_guard` |
| F4 | `update::reconciliation::tests::audit_concurrent_check_must_not_poison_rollback_baseline` |
| F5 | `update::self_update::tests::audit_self_update_requires_health_before_completed` |
| F6 | `update::reconciliation::tests::audit_launcher_shadowing_must_fail_closed` |
| F7 | `update::reconciliation::tests::audit_studio_restart_flag_must_survive_check` |
| F8 | `update::gateway::tests::audit_gateway_equal_count_wrong_names_must_fail` |
| F8 | `update::gateway::tests::audit_gateway_valid_catalog_shape_still_passes` |

## Qualification gates

| Gate | Current disposition | Evidence / rule |
|---|---|---|
| Q01 Rust format | **LOCAL PASS** | `cargo fmt --all -- --check` |
| Q02 Rust check | **LOCAL PASS** | `cargo check --locked --all-targets --all-features` |
| Q03 Rust Clippy | **LOCAL PASS** | strict `-D warnings`; no broad allowlist |
| Q04 Rust tests | **LOCAL PASS** | full suite + exact M5/M6 tests; ignored required smokes executed explicitly |
| Q05 Rust build | **LOCAL PASS** | debug/all-target + release build |
| Q06 Schema/storage/faults | **LOCAL PASS (native arm64 host)** | bundled SQLite qualification, migration/backup/corrupt/WAL/lock/SQLITE_FULL plus real ENOSPC/read-only scripts |
| Q07 Frontend | **LOCAL PASS** | frozen install, lint, typecheck, 31-test suite, build |
| Q08 Security/dependency | **LOCAL PASS** | cargo audit, pnpm high audit, redaction/origin/path/query tests |
| Q09 Restart/crash | **LOCAL PASS** | MCP/Tunnel integrations, interrupted observation, stalled-history stop/shutdown |
| Q10 Update/recovery | **LOCAL PASS (native arm64 host)** | update regressions, P-03 source-journal separation, external self-update launcher smokes |
| Q11 Source-less | **LOCAL PASS (native arm64 host)** | release binary + web source-less package smoke; runtime-only reconciliation/real-binary tests |
| Q12 Long history/retention | **LOCAL PASS** | bounded pages/readers, SQLite/full-filesystem pressure, crash-atomic retention/aggregation |
| Q13 Native | **PASS (required target)** | required M6 native target is `darwin-arm64`; current Rust host/release tests are native arm64. `darwin-amd64` is NOT_REQUIRED for M6. M5-dependent native package fixture remains part of independent Q16 evidence. |
| Q14 Provenance | **PASS** | clean-source `--require-clean` qualification passed |
| Q15 Review/closure | **PASS** | full gate set passed with native arm64 and Q16 evidence |
| Q16 M5 publication dependency | **PASS (independent)** | qualified M5 publication evidence was hash-validated and consumed by M6 |

## Foreground runner

The implemented interface is:

`python3 scripts/verify-m6-history.py --profile <foundation|regressions|security|web|restart|runtime-only|retention|native|full> [--require-clean]`

It records UTC start/end, exact commands and test counts, Studio/Fleet Git state and source identity, lock/toolchain hashes, Rust host triple, runner Rosetta state, native coverage, per-command logs, `summary.json`, `summary.tsv` and a SHA-256 manifest. Exact required test symbols must execute at least one passing test. Missing native/M5 inputs are BLOCKED, not PASS.

Qualification evidence is generated into immutable untracked `.tmp/m6-history/*` directories. Operators rerun the exact committed source on a native arm64 host and validate Q14 plus the arm64 target set without publishing a release.
