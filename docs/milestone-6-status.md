# Mission 6 — Persistence, Metrics & Auditability

- Target/package version: `v0.6.0-alpha`.
- Status: **VERIFIED / CLOSED**.
- Planning baseline: Studio `main` at `6a6122abb41591f22874fb439eb8427edd649e53`.
- Closure baseline: merged `main` at `e10ee57b6c1629744e45ad7eda63cebf368b7504`, with local annotated tag `v0.6.0-alpha`. The pre-implementation M6 branch was based on `026753ba3737e6d04b71973240f77d15879b79e3`.
- Local review date: 2026-09-21.
- Planning/contract source: [M6 implementation plan](plans/m6-persistence-metrics-auditability/README.md).
- Release dependency: **M5 PUBLICATION QUALIFIED** by independent evidence. M6 consumed that evidence during full qualification.

## Scope and authority

M6 now implements private SQLite-backed operational history, typed audit admission/outcome evidence, Studio/MCP/Tunnel observation sessions, update/check/artifact/install lineage, registry/config/drift history, replay-safe historical metrics, bounded retention, historical REST/realtime/UI, and a foreground qualification runner.

The authority split remains unchanged:

- `registry.toml`, `studio.toml`, Fleet managed-state files and M5 recovery journals remain authoritative for current configuration/recovery;
- Supervisor/Tunnel/update owners remain authoritative for live state and side effects;
- SQLite is historical evidence only and never rehydrates live registry/process/update authority;
- safe stop/shutdown/recovery must continue when history is unavailable.

## Package status

| Package | Current status | Local evidence / residual gate |
|---|---|---|
| M6.0 — Architecture/common contracts and schema freeze | **VERIFIED** | ADR/schema freeze accepted before implementation |
| M6.1 — SQLite foundation, migration engine and safe operations | **VERIFIED** | native-arm64 fault qualification covers ENOSPC/read-only/corrupt/WAL/lock bounds |
| M6.2 — Typed audit/event persistence and safety prerequisites | **VERIFIED** | P-01/P-02 and admission/terminal receipt tests pass |
| M6.3 — Studio runs and MCP/Tunnel lifecycle history | **VERIFIED** | restart/interruption/terminal ordering and safe shutdown tests pass |
| M6.4 — Update/check/artifact/install/release lineage | **VERIFIED** | P-03 and external self-update/Fleet-authority proofs pass on native arm64 |
| M6.5 — Registry/config revisions and Fleet drift history | **VERIFIED** | config authority and reconciliation-check audit integration pass |
| M6.6 — Metrics aggregation, retention and bounded growth | **VERIFIED** | aggregation, retention, pagination and storage-pressure qualification pass |
| M6.7 — Historical read APIs, realtime and UI | **VERIFIED** | REST/realtime/UI and source-less package smoke pass |
| M6.8 — Full qualification, migration/release gates and closure | **VERIFIED** | full clean-source arm64 qualification consumed qualified M5 publication evidence |

## Implemented safety prerequisites and review findings

The three source-audit prerequisites are resolved in current source:

- **P-01:** generic MCP apply acquires the shared RuntimeOperationCoordinator component lease before its local transaction guard.
- **P-02:** registry whole-document mutation is serialized across snapshot → persist → replace.
- **P-03:** Tunnel terminal journal evidence is attempted before source-journal cleanup when history is healthy, but audit failure no longer retains a recovery-authority journal. Malformed/unsafe source journals remain fail-closed.

Additional review findings resolved during implementation:

- terminal-before-start lifecycle reduction previously allowed negative observed duration and rolled back the start transaction; it now clamps observation duration to zero while preserving exact owner monotonic duration separately;
- default history cursor time ranges are frozen in the opaque cursor so page two does not self-invalidate as wall time advances;
- API test fixture roots use UUID identity to prevent parallel SQLite/runtime collisions;
- history admission/terminal receipt waits run through `spawn_blocking`, not Tokio reactor threads;
- interrupted Studio runs create explicit unknown coverage intervals and never invent child exit/crash/duration;
- staged artifact history now persists only safe hash identity/member digests and links verified-active observations to the same artifact;
- history failure during Tunnel journal cleanup is audit incompleteness only and does not change M5 recovery cleanup behavior.

## Final qualification

`scripts/verify-m6-history.py --profile full --require-clean` passed on native `aarch64-apple-darwin` with qualified M5 publication evidence. It ran format/check/Clippy, full Rust and web suites, dependency audits, real ENOSPC/read-only fixtures, restart/recovery, source-less/runtime-only, retention, native package, rollback, and exact M5 regressions. The runner recorded no failed, blocked, or skipped mandatory command.

## Requirement disposition

[M6 verification matrix](plans/m6-persistence-metrics-auditability/M6-VERIFICATION-MATRIX.md) now maps R01–R44 to implemented source and permanent proof. All R01–R44 requirements have local/current-host implementation evidence; architecture-specific rows are qualified on the current **native darwin-arm64 Rust host**. Q13 requires only native darwin-arm64 for M6; darwin-amd64 is explicitly outside the current M6 supported target set.

Current-host evidence now includes real disposable filesystem ENOSPC and read-only mounts, corrupt main DB and malformed WAL rejection without auto-repair, owner-lock contention, SQLite page-limit FULL, crash-before-COMMIT retention/aggregation rollback, external self-update success/failure across launcher process boundaries, source-less release binary/web/history operation, runtime-only reconciliation/real-binary activation, and actual committed-baseline binary rollback compatibility.

## Closure evidence

- Q13: native `darwin-arm64` host and binaries qualified; Intel is not supported.
- Q14: clean-source provenance passed.
- Q16: M5 publication evidence was independently verified and consumed.

## Closure / release status

**Implementation:** verified.
**M6 VERIFIED closure:** **CLOSED**.
**Release:** package version is `v0.6.0-alpha` and the local tag exists. Remote tag push and release publication remain separate authorized actions.
