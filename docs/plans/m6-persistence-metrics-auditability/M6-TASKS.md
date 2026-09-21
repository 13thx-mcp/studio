# M6 dependency-ordered execution checklist

**Planning baseline:** Studio main `6a6122abb41591f22874fb439eb8427edd649e53`. **Final disposition:** M6.0–M6.8 are **VERIFIED / CLOSED** on darwin-arm64; full clean-source qualification consumed independently qualified M5 publication evidence. [COMMON](COMMON.md) and each package contract remain the frozen design/verification contract.

## Dependency order

```text
M6.0 architecture/schema freeze
  → M6.1 SQLite/migration/path/failure foundation
  → M6.2 typed audit/admission + P-01/P-02 prerequisites
  → M6.3 Studio/process/Tunnel observation sessions
  → M6.4 update/check/artifact/lineage
  → M6.5 config/registry/drift (can develop after M6.3, joins M6.4 before exit)
  → M6.6 metric definitions/aggregation/retention
  → M6.7 bounded history APIs/realtime/UI
  → M6.8 full qualification/independent review/closure
```

Storage safety, bounds, migrations, privacy and failure tests start in M6.1/M6.2, not at final closure. Minimal history-health/operation acknowledgment is delivered with audit admission, not delayed until the historical UI. Integration prerequisites P-01/P-02 are added to M6.2 rather than a separate unrelated hardening mission because new history await points require the existing ownership contract to hold.

## Packages

- [x] **M6.0 — VERIFIED** — [Architecture/common contracts and schema freeze](M6.0-architecture-schema.md). Convert this proposed plan into a reviewed, internally consistent contract before adding a database dependency.
- [x] **M6.1 — VERIFIED** — [SQLite foundation, migration engine and safe operations](M6.1-sqlite-foundation.md). Provide a bounded, private history service that opens/migrates safely, reports failures honestly and cannot interfere with existing runtime recovery.
- [x] **M6.2 — VERIFIED** — [Typed audit/event persistence and safety prerequisites](M6.2-audit-events.md). Establish a single typed admission/receipt protocol and remove the two source-inspected serialization gaps before injecting asynchronous history work into owners.
- [x] **M6.3 — VERIFIED** — [Studio runs and MCP/Tunnel lifecycle history](M6.3-runtime-lifecycle.md). Record actual owned process generations and observation boundaries across Studio restarts without restoring stale live state.
- [x] **M6.4 — VERIFIED (native darwin-arm64)** — [Update/check/artifact/install/release lineage](M6.4-update-history.md). Explain observed updates and active/previous/verified-good artifacts for every current update domain without changing M5 transaction/recovery authority.
- [x] **M6.5 — VERIFIED** — [Registry/config revisions and Fleet drift history](M6.5-config-drift-history.md). Explain observed configuration and managed drift without treating historical snapshots as the source of current configuration.
- [x] **M6.6 — VERIFIED** — [Metrics aggregation, retention and bounded growth](M6.6-metrics-retention.md). Compute truthful, replay-safe operational metrics and keep history finite without silently deleting unexpired required audit.
- [x] **M6.7 — VERIFIED** — [Historical read APIs, realtime and UI](M6.7-history-api-ui.md). Expose timeline/charts/update/config/drift history with bounded sanitized reads and an unmistakable distinction from live state.
- [x] **M6.8 — VERIFIED / CLOSED** — [Full qualification, migration/release gates and closure](M6.8-verification-closure.md). Prove end-to-end persistence and explainable lineage through real restart/update/failure scenarios, with clean provenance and honest release status.

## Cross-package checklist

- [x] Authority map, recovery separation and M5 F1–F8 preserved with permanent tests.
- [x] All required action families have request/admission/actual transition/terminal evidence or explicit safety exception/unknown coverage.
- [x] Current artifact/config lineage has recorded proof scopes and honest bootstrap/retention/interruption boundaries.
- [x] Each schema change is immutable/versioned; backup, future schema and downgrade cases tested.
- [x] Every R01–R44 requirement maps to source, a permanent test and current qualification evidence.
- [x] Required native darwin-arm64 and source-less layouts qualified; implementation and publication states separate. darwin-amd64 is not an M6 required target.

## Historical implementation handoff — COMPLETED

The original fresh-session sequence was M6.0 architecture/schema freeze → M6.1 storage foundation → M6.2–M6.8 implementation/qualification. That sequence is complete and must not be restarted as current work.

For new sessions, treat `docs/milestone-6-status.md`, `M6-VERIFICATION-MATRIX.md`, and `M6.8-verification-closure.md` as the final M6 execution disposition. M7 is the next implementation milestone; preserve the M6 authority split and all P-01/P-02/P-03 regressions.

The M6.2 P-01/P-02 fixes and the M6.4 P-03 cleanup/audit separation are implemented with permanent tests. Keep future production release/tag push/publication/deploy actions separate from implementation commits.

## Completion record format

For each checked package record implementation commit(s), dependency HEADs, final test symbols, qualification manifest path/hash, requirement IDs, review disposition and residual limitations. Do not check a package merely because its document was written. If native/storage-fault evidence is unavailable, record BLOCKED/NOT RUN with the specific missing capability; do not convert it to PASS.
