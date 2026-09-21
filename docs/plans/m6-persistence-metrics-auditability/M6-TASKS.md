# M6 dependency-ordered execution checklist

**Planning baseline:** Studio main `6a6122abb41591f22874fb439eb8427edd649e53`. **Current execution:** implementation is committed on `feature/m6`; local native-arm64 qualification passes and a clean local qualification rerun is required before the independent M5 publication gate. Package acceptance does not clear M5 publication qualification. [COMMON](COMMON.md) and each package contract are normative after M6.0 review.

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
- [ ] **M6.1 — IMPLEMENTED / LOCAL PASS; closure evidence incomplete** — [SQLite foundation, migration engine and safe operations](M6.1-sqlite-foundation.md). Provide a bounded, private history service that opens/migrates safely, reports failures honestly and cannot interfere with existing runtime recovery.
- [ ] **M6.2 — IMPLEMENTED / LOCAL PASS** — [Typed audit/event persistence and safety prerequisites](M6.2-audit-events.md). Establish a single typed admission/receipt protocol and remove the two source-inspected serialization gaps before injecting asynchronous history work into owners.
- [ ] **M6.3 — IMPLEMENTED / LOCAL PASS** — [Studio runs and MCP/Tunnel lifecycle history](M6.3-runtime-lifecycle.md). Record actual owned process generations and observation boundaries across Studio restarts without restoring stale live state.
- [ ] **M6.4 — IMPLEMENTED / LOCAL PASS (native arm64 current host)** — [Update/check/artifact/install/release lineage](M6.4-update-history.md). Explain observed updates and active/previous/verified-good artifacts for every current update domain without changing M5 transaction/recovery authority.
- [ ] **M6.5 — IMPLEMENTED / LOCAL PASS** — [Registry/config revisions and Fleet drift history](M6.5-config-drift-history.md). Explain observed configuration and managed drift without treating historical snapshots as the source of current configuration.
- [ ] **M6.6 — IMPLEMENTED / LOCAL PASS** — [Metrics aggregation, retention and bounded growth](M6.6-metrics-retention.md). Compute truthful, replay-safe operational metrics and keep history finite without silently deleting unexpired required audit.
- [ ] **M6.7 — IMPLEMENTED / LOCAL PASS** — [Historical read APIs, realtime and UI](M6.7-history-api-ui.md). Expose timeline/charts/update/config/drift history with bounded sanitized reads and an unmistakable distinction from live state.
- [ ] **M6.8 — RUNNER IMPLEMENTED / CLOSURE BLOCKED** — [Full qualification, migration/release gates and closure](M6.8-verification-closure.md). Prove end-to-end persistence and explainable lineage through real restart/update/failure scenarios, with clean provenance and honest release status.

## Cross-package checklist

- [ ] Authority map, recovery separation and M5 F1–F8 preserved with permanent tests.
- [ ] All required action families have request/admission/actual transition/terminal evidence or explicit safety exception/unknown coverage.
- [ ] Current artifact/config lineage has recorded proof scopes and honest bootstrap/retention/interruption boundaries.
- [ ] Each schema change is immutable/versioned; backup, future schema and downgrade cases tested.
- [ ] Every R01–R44 requirement maps to source, a permanent test and current qualification evidence.
- [ ] Required native darwin-arm64 and source-less layouts qualified; implementation and publication states separate. darwin-amd64 is not an M6 required target.

## Fresh-session handoff: M6.0 then M6.1

Start in Aira `mcp-server/studio`; report branch, HEAD, worktree and unchanged M5 publication gate. Read SOURCE-AUDIT, COMMON, SCHEMA, SECURITY-AND-OPERATIONS, ADR-PLAN and M6.0/M6.1 in that order. Check the source anchors P-01/P-02 and main.rs startup ordering before confirming no base drift. Do not infer source freshness from a prior PASS archive.

M6.0 first freezes decisions/ADR/schema fixture checksum without production changes. Then an explicitly authorized implementation session can execute M6.1: dependency resolution → safe path/store ownership → typed worker/health/clock → exact DDL/migrations → failure/backup tests → main composition → native engine/build verification. Do not begin with registry migration or serializing existing owner structs. Add foundational fault seams before wiring actual control operations.

The M6.2 P-01/P-02 fixes and the M6.4 P-03 cleanup/audit separation are now implemented with permanent tests. The remaining unchecked packages above are not “not implemented”; they remain unchecked because package VERIFIED requires the missing real/native closure evidence listed in the matrix. Keep production release/version/tag/push/deploy separate from package commits; use focused Conventional Commits only when authorized/useful.

## Completion record format

For each checked package record implementation commit(s), dependency HEADs, final test symbols, qualification manifest path/hash, requirement IDs, review disposition and residual limitations. Do not check a package merely because its document was written. If native/storage-fault evidence is unavailable, record BLOCKED/NOT RUN with the specific missing capability; do not convert it to PASS.
