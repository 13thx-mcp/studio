# Source audit and dependency register

## 1. Provenance and scope

Read-only repository inspection on 2026-09-19 established:

| Item | Observed value / qualification |
|---|---|
| Studio branch | `main`, tracking `origin/main` |
| HEAD | `6a6122abb41591f22874fb439eb8427edd649e53` |
| Locally cached origin/main | Same HEAD; no fetch or assertion about a subsequently changed remote |
| Initial worktree | Clean (`git status --porcelain=v1` empty) |
| Local tag at HEAD | `v0.5.0` |
| Rust / web versions | `0.5.0` in `Cargo.toml` / `web/package.json` |
| Toolchain | Edition 2024; `rust-toolchain.toml` and rust-version `1.98.1` |
| SQLite today | No dependency in Cargo.toml; `src/storage/mod.rs` is a placeholder |
| Metrics today | `src/metrics/mod.rs` is a placeholder; runtime counters are in supervisors |
| Adjacent Fleet HEAD | Read-only inspection reported `8b827521fdcb423b1c87e70d3885d518536842fd`; no Fleet writes |
| Repository instructions | No AGENTS.md at Studio, its checked parent/grandparent, or docs |

Reviewed `ROADMAP.md`, `docs/architecture.md`, `docs/threat-model.md`, `docs/milestone-5-status.md`, `docs/plans/m5-runtime-distribution/COMMON.md`, `docs/plans/m5-review-remediation/README.md`; ADRs 0004–0006, 0011–0017; Cargo/toolchain/release workflow; relevant registry/config/supervisor/tunnel/update/API/realtime source; existing lifecycle tests; web API/types/state/realtime/update UI; qualification runner definitions. Source inventory was enumerated before selecting owner write points.

### Source versus older documentation

- `ROADMAP.md:420` still identifies the released baseline as v0.4.0. `docs/milestone-5-status.md:5-22` distinguishes local v0.5.0 closure/tagging from blocked publication qualification. Preserve that distinction; do not repeat already-completed local version/tag steps.
- M5 status references remediation verification on Studio `a388cf2611fead432e79ae041112d761632cbc00`, not the current release HEAD. Recorded evidence is not a fresh execution in this session.
- Status `:247-301` documents historical unauthenticated release endpoint failures and missing native/published-artifact proof. This session did not test public release endpoints.
- Old `src/storage/mod.rs`/ADRs reserve SQLite for an earlier-numbered milestone. The current roadmap places it in M6 (`ROADMAP.md:765-839`). `src/realtime.rs` is a file, not a directory.
- Older threat-model text that treats Tunnel as inventory-only is superseded by `src/update/tunnel_update.rs`. History must cover actual supported update owners, including same-version Tunnel reinstall.
- Some live views expose path/argument or free-form error fields. Their existence is NOT approval to persist or return those fields in a new history API.

### Observed runtime metadata, not deployment qualification

The adjacent runtime was inspected by names, file types, permissions and sizes only. Flat Rust MCP/Gateway binaries exist under `../bin`. `../runtime/studio` currently contains a legacy flat `mcp-studio`, `studio.toml`, `releases` and `data`; no `current` entry appeared in that directory listing. `data` is mode 0755 and contains a file registry and a private self-update directory. `../runtime/fleet/state` was absent. Tunnel has a `current` symlink, releases and private config. This is **not proof of which binary is running, its health or its version**. M6 must support both supported legacy-flat and versioned Studio layouts without migrating them.

## 2. Existing authorities

| Authority | Current source / contract | M6 disposition |
|---|---|---|
| Registry configuration | `src/registry/mod.rs:17-47,87-123,179-254,502-534`; schema 1 TOML; configured path defaults to `data/registry.toml` (`config:169-170`) | Keep file + existing in-memory registry control state. Add sanitized revisions and entry-incarnation observations only. |
| Loaded Studio configuration | `src/config/mod.rs:215-231`; same bytes parsed and hashed, canonical identity retained | Keep exact loaded identity; separate from newer on-disk revision. No body copies in history. |
| Root layout / desired pins | `src/config/mod.rs:95-139,175-182`; `ComponentCatalog` | DB derived from catalog runtime root, never browser input. File pins remain file authority. |
| Process ownership | `src/supervisor/mod.rs:62-105,212-335,379-503` | In-memory owned children/generations govern actions. Historical PIDs are display evidence only. |
| Tunnel ownership | `src/tunnel/mod.rs:86-106,336-550` | Preserve launch generation/hash proof. Do not infer child ownership from a historical session. |
| Release checks / desired override | `src/update/inventory.rs:388-449,471-576` | Cache and process-local override remain live behavior. DB observations never silently hydrate desired controls. |
| MCP update state | `src/update/transaction.rs:105-109,350-477,871-925` | Volatile transaction authority; history of phases is not an executable resume ticket. |
| Gateway / Fleet updates | `src/update/gateway.rs:944-968,1186-1198,1392-1437`; `fleet.rs:627-719,830-881` | Owner terminal result after its verification; Gateway catalog probe is not a managed Gateway session; Fleet is a bundle, not a process. |
| Staging integrity | `src/update/staging.rs:38-50,336-359` | Observe verified staging only; metadata/ready directory does not prove install success. |
| Reconciliation commit | `src/update/reconciliation.rs:483-492,895-1094,1408-1473`; managed manifest under `runtime/fleet/state/reconciliation.json` | Keep file authority, generations and rollback baseline. Mirror safe observations after commit/verified compensation. |
| Loaded-vs-converged marker | `reconciliation.rs:753-786,970-1013` | Preserve different-process + exact-loaded-hash activation proof. No DB row may clear restart pending. |
| Studio external activation | `src/update/self_update.rs:99-126,441-558,775-863`; schema/revisioned journal below `runtime/studio/data/self-update/` | Fleet owns terminal activation/rollback finalization. Observe actual persisted revisions; never write them from history. |
| Tunnel recovery | `src/update/tunnel_update.rs:53-73,383-417,456-561,1876-2011` | Independent safety journal below `runtime/studio/data/tunnel-update/`; never load recovery instructions from SQLite. |
| Websocket bus | `src/realtime.rs`; `src/api/mod.rs:496-549` | Lossy invalidation/live transport, not an audit ingestion channel. |
| Browser storage | `web/src/updates-state.ts:225-302`; `App.tsx:156-174` | Pending transaction hints only; new history obtained from the server. |

## 3. Important source-level findings and decisions

### P-01 — Generic MCP apply coordinator gap

The documented order is shared coordinator lease, then manager-local lease (`src/update/mod.rs:224-227`). MCP prepare takes both (`transaction.rs:355-360`), but generic apply (`:435-464`) takes only its manager-local guard. The generic API branch (`api/mod.rs:416-420`) does not supply the missing shared lease. Gateway, Fleet, Studio and Tunnel acquire control leases themselves.

**Classification:** source-inspection concurrency risk; no reproducer was run here. **Plan:** M6.2 first adds `mcp_apply_respects_control_lease` and `mcp_apply_blocks_reconciliation_until_terminal`, reproduces the current behavior, and acquires the existing component lease before local apply acquisition. Do not acquire the same lease again from nested supervisor calls. Re-run F1–F8. M6.4 cannot declare concurrent history ordering safe before this is closed.

### P-02 — Registry lost-update / revision-order risk

`register/update/set_enabled/unregister` clone a snapshot before `persist_and_replace`; the registry RwLock protects individual reads/replacements, not the complete read/modify/write transaction. Different component leases may coexist. An added DB write between snapshots would enlarge this window.

**Classification:** source-inspection risk; not a reproduced data-loss incident. **Plan:** M6.2 adds an internal whole-registry mutation mutex around snapshot/validate/file write/replace. Capture a bounded immutable commit receipt before releasing it. No SQL or DB wait while holding this mutex. History admission occurs before entering the registry mutation mutex, under the existing logical operation lease. Direct registry entry points must share the same mutex. Retry semantics must not reuse a stale snapshot.

`persist_document` syncs the file, renames, and attempts directory sync but ignores directory-sync errors (`:523-525`). Do not label this as a new, cross-file/DB atomic transaction or guaranteed power-loss durability. Preserve existing behavior for M6; report observed commit versus hardware durability honestly. Strengthening that legacy contract requires a separately reviewed change, not an undocumented history refactor.

### P-03 — Tunnel terminal evidence disappears on normal cleanup

Successful upgrade commits a recovery journal at `tunnel_update.rs:726-745`, deletes it at `:746-748`, and only then sets public Completed. Same-version completion is `:919-951`. Rollbacks remove journals at `:1061-1064` and `:1185-1188`; startup cleanup is `:540-559`.

**Plan:** add an owner-local typed terminal capture after the authoritative verified journal transition and before deletion, with a bounded post-action history flush. Never keep or repurpose a terminal recovery journal just because SQLite is down: that could change next-start recovery/ownership behavior. Durable admission remains the missing-terminal indicator; mark incomplete audit/lineage when delivery fails. No full-journal archival or duplicate recovery store is introduced.

### P-04 — “Check” is not always read-only

Reconciliation check can adopt a legacy baseline (`:815-823`) and reconcile the loaded-config restart marker (`:824-826`). Record those existing effects separately after `state_store.persist` (`:783-784`, `:820-821`). This is existing explicit/startup behavior, not authorization for periodic reconciliation. Inventory check preserves a previous cached latest version on provider failure (`inventory.rs:484-491`); that stale value must not count as a newly available update.

### P-05 — Observation semantics must follow each owner

MCP monitor treats unrequested exit 0 as stopped (`supervisor.rs:471-480`); Tunnel treats every unrequested exit as a crash (`tunnel/mod.rs:462-480`). Both count wait errors as crashes and increment restart_count before a new start attempt. Do not redefine public counters silently. Duration is captured before clearing `started_at`. Unobserved Studio death is not proof every child crashed.

### P-06 — Self-update observation can miss intermediate revisions

`SelfUpdateManager::write_record` increments `journal_revision` on a persisted clone (`:787-798`); the caller's original record is not the authoritative revision. Read back through the validated journal reader or return a typed persisted receipt. A startup scan/status observation may see revision N then N+3; store a revision-gap marker, not fabricated phases. Startup can observe external activation before Fleet writes a terminal result. Add a finite, read-only observer for that already-requested transaction after listener readiness.

## 4. Existing test/qualification anchors to retain

- `tests/registry_discovery.rs`: file persistence round trip, approval, duplicate project, traversal/symlink rejection.
- `tests/supervisor_lifecycle.rs`: starts/stops, restart count, unexpected nonzero exit, duplicate start, disabled MCP, shutdown owned processes.
- `tests/tunnel_lifecycle.rs`: including `unexpected_zero_exit_is_still_a_crash`, runtime validation and secret-redacted live logs.
- `scripts/verify-m5-remediation.py:24-33` lists the ten named F1–F8 regressions. Some independent audit tests are ignored in ordinary cargo execution; the full runner uses explicit `--ignored --exact` for those evidence cases. Never substitute a zero-match filtered test PASS.
- `scripts/verify-native-package.py` requires native Darwin, `M5_RESULT_DIR`, a release binary and built web bundle. It is not a restart-history smoke by itself.
- `.github/workflows/release.yml:65-72` specifies native darwin-amd64 and darwin-arm64. Its publish job is explicitly out of scope in this planning session.
- `web/src/App.tsx:103-174` merges live events and refreshes pending hints on reconnect; it is not a historical state cache or persistent transaction owner.

## 5. Audit limits

No production tests, fault injection, builds, package qualification, public endpoint checks, runtime lifecycle actions or credential/config-body inspections were performed for this plan. A combined additional read-only inspection request was denied by tool safety screening; subsequent Studio-only reads completed the required runner/UI/gate inspection. The plan does not claim a full new audit of Fleet's launcher implementation. Its preserved contract is supported by Studio's readers/validators, existing ADRs and M5 tests. Production P-01/P-02 closure remains an implementation gate.
