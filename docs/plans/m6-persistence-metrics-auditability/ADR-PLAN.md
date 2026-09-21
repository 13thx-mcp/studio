# ADR freeze plan

Existing ADRs end at 0017 on the audited source. Reserve the following numbers provisionally; recheck the directory before creating them in M6.0. These are proposed decisions, not accepted ADRs. Use the existing `docs/adr/TEMPLATE.md` convention and link tests/requirement IDs.

| Proposed ADR | Decision to freeze | Rejected alternative / consequence | Acceptance evidence |
|---|---|---|---|
| 0018-history-authority-boundaries | Files/live owners/recovery remain authoritative; SQLite owns only history/projections | Registry migration or SQL-driven recovery would create competing authorities | R01–R03, history cannot mutate sources, M5 regression mapping |
| 0019-sqlite-history-store | Rusqlite candidate 0.40.2, bundled patched engine, one writer/two readers, stable private runtime path, WAL/FULL and finite bounds | Multiwriter pool, executor-blocking connection, system-dependent old SQLite | R04–R08, exact build/engine qualification is a M6.1 exit gate |
| 0020-audit-admission-and-failure | Durable discretionary admission, safe-action exception, bounded typed receipts, domain/audit outcomes separate; no file outbox | Unconditional fail-open audit, history-induced rollback or infinite durable guarantee during disk failure | R09–R14, cancellation/late-commit/fault-state model |
| 0021-history-event-identity-schema | Complete candidate schema v1, opaque IDs, source ordinals, commit sequence, checksummed embedded migrations | Timestamp ordering, raw Serialize persistence, inferred success from request | R15–R21, schema fixture validation and exact event enum catalogue |
| 0022-history-lineage-and-journal-observation | Typed receipt before safety cleanup, read-only revisioned journal import; Fleet sole self-update terminal authority | Keeping journals alive solely for history; SQL resumption of volatile updates | R22–R28, two-process self-update and same-version Tunnel fixtures |
| 0023-observed-metrics-and-retention | Definition version 1, MCP/Tunnel-specific crash semantics, unknown intervals, bounded aggregates/audit retention/lineage boundaries | Global-counter snapshots as totals, zero-filling gaps, unlimited pinned ancestry | R29–R34, golden metrics/retention/quota fixtures |
| 0024-history-api-and-realtime | Additive read-only routes, deterministic cursor epochs, sanitized DTOs, post-commit invalidations and separate live state | Client-side authoritative history, arbitrary filtering, stale history action controls | R35–R39, API/UI/reconnect/privacy fixtures |
| 0025-history-bootstrap-and-operations | Empty M5 history bootstrap, private backup/restore, no auto repair/downgrade, independent readiness | Fake pre-M6 events, main-file-only WAL backup, DB failure vetoing safe recovery | R40–R44, native upgrade/downgrade/source-less qualification |

## M6.0 decision checklist

Freeze the schema SQL checksum, application_id `0x4d435348`, migration/user_version policy, domain/audit response headers, store root, queue/read bounds, synchronous/FSYNC settings, row/size/retention defaults, closed event payload families, metric definition version and finite self-update observation protocol. Confirm the P-01/P-02 safety prerequisites are scheduled before history integration, not silently marked repaired by an ADR.

UUID v4 requires a minimal production dependency rather than an unreviewed home-grown generator. Select and lock an appropriate UUID/getrandom graph in M6.1; M6.0 freezes identity format and collision/entropy failure behavior (no action admitted without a valid unique identity). Similarly the rusqlite exact candidate is decided now, while resolved transitive versions, bundled engine source ID and cross-platform build results are measured in M6.1. A failed build/security gate requires a focused ADR amendment, not an unresolved architecture choice handed to each package independently.

Record decision owner/review date, alternatives and residual limitations: no distributed transaction with filesystem state, no exact history under total storage failure, no same-user tamper-proof audit, no exact cross-process monotonic duration and no release-publication claim. Those limits must appear in operator docs and UI coverage, not only the ADR appendix.

Do not create new Fleet/Mirin policies or extend the Fleet launcher protocol for M6. A later protocol change would require a separate coordinated proposal and compatibility qualification; this design observes the existing validated terminal records.
