# Mission 6 — Persistence, Metrics & Auditability

**State:** architecture frozen; production implementation substantially complete; local qualification green; final VERIFIED closure BLOCKED only by the independent M5 publication/native-package gate after clean-source qualification; M6 native support is darwin-arm64 only.
**Planning source baseline:** Studio `main`, `6a6122abb41591f22874fb439eb8427edd649e53`.
**Implementation worktree:** `feature/m6`, uncommitted as of 2026-09-20. **Roadmap target:** `v0.6.0-alpha` (not a version change authorization).

## Purpose and boundaries

Persist operational/update history, audit evidence and useful lifecycle metrics across Studio restarts, while explaining the observed artifact/configuration lineage. SQLite is a history authority, never a substitute for live owners or recovery journals. Retention boundaries, interrupted observations and unavailable evidence remain explicit.

This package began as a read-only audit of the Studio repository and remains the normative M6 contract. It has now been implemented against the current `feature/m6` worktree. Historical line references still describe the planning baseline; current implementation truth is the source plus [milestone status](../../milestone-6-status.md) and [verification matrix](M6-VERIFICATION-MATRIX.md). Real production credentials/runtime were not used as destructive fixtures.

M5 is **locally closed/tagged, publication qualification blocked**. Its publication gate remains independent. M6 local implementation/testing does not satisfy or weaken that gate. Native/package qualification that depends on M5 evidence must consume a real qualified `M5_RESULT_DIR`; missing evidence is BLOCKED, never synthesized.

## Read order / implementation entry point

1. [Source audit](SOURCE-AUDIT.md), then [common contracts](COMMON.md).
2. [Schema](SCHEMA.md), [events and integration](EVENTS-AND-INTEGRATION.md), [metrics and retention](METRICS-AND-RETENTION.md).
3. [API and UI](API-AND-UI.md), [security and operations](SECURITY-AND-OPERATIONS.md), [ADR plan](ADR-PLAN.md).
4. [Task ordering](M6-TASKS.md), [verification matrix](M6-VERIFICATION-MATRIX.md), [adversarial review](ADVERSARIAL-REVIEW.md).
5. For current execution state, read [milestone status](../../milestone-6-status.md), [task ordering](M6-TASKS.md) and [verification matrix](M6-VERIFICATION-MATRIX.md). The implementation follows the frozen M6.0→M6.8 dependency order; unresolved real/native evidence remains explicit.

The common documents are the proposed normative contract; package plans specialize them. Resolve contradictions in favor of safety/non-authority, then update the ADR and matrix explicitly. Do not treat an unchecked implementation item as completed because its design exists.

## Main decisions

- Keep `registry.toml`, `studio.toml`, Fleet managed-state files and all M5 journals authoritative in their existing domains. SQLite stores sanitized historical projections only.
- Introduce a private, stable `runtime/studio/data/history/studio.sqlite3`, independent of cwd, `current`, release directories and source checkout.
- Use `rusqlite` with bundled, qualified SQLite, one dedicated writer thread and two bounded read workers. WAL/FULL, finite queues, query deadlines and bounded compaction are part of the foundation, not optional follow-up work.
- Record intent/admission separately from physical transitions, terminal outcomes, recovery and observations. Do not derive persisted facts from the lossy websocket bus.
- Fail closed for new discretionary action admission when audit persistence is unavailable. After side effects, preserve the actual result and expose incomplete audit evidence; never skip safe stop/rollback/recovery or roll back a healthy install merely because history failed.
- Define counters from the current source: MCP spontaneous exit 0 is not a crash; Tunnel unrequested exit 0 is. Existing restart counters measure attempts before `start`, not only successful replacements.
- Historical DTOs are separate from live DTOs. No paths, argv/env, credential bodies, raw errors or arbitrary query authority enter historical APIs.
- M7 automatic control policies and later Gateway request/usage telemetry remain out of scope.

## Source-discovered prerequisites

`P-01`: generic `McpUpdateManager::apply` lacks the shared coordinator lease acquired by the other apply owners. `P-02`: registry whole-document read/modify/write is not serialized across distinct component leases. These source-inspection risks were reproduced/covered by permanent tests and corrected narrowly in M6.2: generic MCP apply now acquires the shared coordinator lease and registry snapshot→persist→replace mutations are serialized. Their current status is tracked in R13/R14.

`P-03`: Tunnel journals are deleted at successful completion/recovery. M6.4 must capture terminal verified evidence at the owner before cleanup, not discover it later from a directory scan. On history failure, do not retain a journal solely for audit and thereby change recovery behavior.

## Package map

| Package | Result | Depends on |
|---|---|---|
| M6.0 | Accepted ADRs, authority/event/schema/failure freeze | Current source audit |
| M6.1 | Safe SQLite store, migrations, boot/health, bounded queues | M6.0 |
| M6.2 | Audit protocol, sanitized DTOs, P-01/P-02 regressions and corrections | M6.1 |
| M6.3 | Studio/MCP/Tunnel sessions and lifecycle evidence | M6.2 |
| M6.4 | Update/check/staging/install/release and self-update history | M6.3 |
| M6.5 | Registry/config revision and reconciliation/drift history | M6.3; integrate M6.4 lineage |
| M6.6 | Exact-once metrics, coverage, bounded retention/compaction | M6.4 + M6.5 |
| M6.7 | Bounded history APIs, commit invalidation, historical UI | M6.6 |
| M6.8 | Native/restart/source-less/security/full qualification, closure | M6.7 |

Production M6 code and tests are now present in the uncommitted `feature/m6` worktree. [Planning validation](PLANNING-VALIDATION.md) remains historical design evidence; it is not production qualification. Current implementation/qualification truth is recorded separately in [milestone status](../../milestone-6-status.md) and the matrix. No version/tag/push/release/deployment is implied.
