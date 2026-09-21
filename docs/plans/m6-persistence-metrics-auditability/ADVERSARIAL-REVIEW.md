# Internal adversarial review of the proposed M6 design

**Review scope:** source-backed architecture and documentation, not implemented M6 code. Design issues below were addressed in the proposed contracts. Source prerequisites P-01/P-02 still require implementation/test proof. This is not an independent production security certification.

| ID | Attack / failure question | Design finding and revision | Required proof before closure |
|---|---|---|---|
| A01 | Could SQL registry/config rows become a competing authority? | Rejected registry migration and SQL rehydration. Files/live owners remain source; history is observation-only, including desired versions. | R01–R03 |
| A02 | Can a request or healthy replacement be logged as successful update? | Separate request/admission/transition/outcome; Fleet terminal journal only for Studio; domain Completed remains owner-defined. | R15/R22/R25 |
| A03 | Does durable audit block stop/recovery when the disk fails? | Safety exception and reserved nonblocking capture; physical stop/compensation precedes bounded history drain. No history error short-circuits a destructive phase. | R08–R12 |
| A04 | Can mandatory terminal audit be guaranteed on completely failed storage? | No. Removed any implied universal guarantee; commit admission before discretionary action, expose incomplete/indeterminate evidence, deny subsequent admissions. No history-only file outbox or hidden infinite retry. | R09/R10/R19 |
| A05 | Does waiting for audit keep a recovery journal alive and alter the next recovery? | Rejected holding journals solely for history. Capture terminal receipt before normal cleanup; existing cleanup/safety semantics continue under history failure. | R03/R24 |
| A06 | Does the lock graph contain DB → owner → DB deadlock? | One-direction typed capture; writer has no owner references; no actual mutex held during SQL await. Logical existing lease may cover bounded admission. P-01/P-02 scheduled first. | R05/R12–R14 |
| A07 | Could HTTP cancellation after admission cancel required compensation? | Owner task retains physical operation through its existing terminal/rollback path. Late DB admission never dispatches from a worker. Cancellation tested before and after effects. | R09/R10/R12 |
| A08 | Can order invert across workers/restarts or wall-clock jumps? | Commit sequence plus source ordinal; terminal envelopes carry known start identity; immutable first-observation membership; nullable source time/quality. No timestamp-only IDs/sorting. | R16–R20 |
| A09 | Can reimport after retention increment totals twice? | Journal watermark rejects stale revisions without insertion; first qualifying outcome seq is immutable. Repeated higher-revision terminal observations are not new operation outcomes. Retained duplicate payload conflicts are checked; expired older payload cannot be claimed verified. | R16/R25/R31/R33 |
| A10 | Are process/crash/uptime metrics truthful? | Preserve MCP clean-zero versus Tunnel unrequested-zero distinction; failed spawn not launch; restart attempts distinct from successful replacement; interrupted sessions lack exact duration/crash. | R18/R19/R29/R32 |
| A11 | Is stale cached latest treated as a successful fresh check? | Per-component check outcome is explicit; fetch failure does not create fresh availability. Bootstrap imported past terminals are not new M6 installs. | R30/R40 |
| A12 | Can a finite table cap still create infinite WAL/backups/pinned lineage? | Account separately for main DB/WAL/backups, short readers and checkpoint pressure; bounded lineage summaries, not full ancestors; hard pressure rejects new discretionary work. No claim journal_size_limit is a hard WAL quota. | R33/R34/R41 |
| A13 | Can pruning create plausible but false complete lineage or unstable pages? | Retention boundary records, global cursor epoch invalidation and snapshot as-of filters; no browser-long SQL read transaction. | R26/R33/R35 |
| A14 | Do source structs leak argv/env/paths/errors via Serialize? | Closed allowlist DTOs only; no free-text registry labels in history, private config digest excluded publicly; stored and exported representations tested. | R37 |
| A15 | Is symlink validation claimed to stop a malicious same-UID race? | Explicit local OS-identity trust boundary. Validate path/owner/types/sidecars and use private creation; no claim of hostile same-UID tamper-proof storage. History cannot authorize control even if tampered. | R06/R37 |
| A16 | Could migrations/backup break M5 self-update or downgrade? | Independent history readiness; additive checksummed schema; backup requirement; incompatible old binary leaves SQL untouched; M5 file protocols unchanged. Native binary rollback is a gate. | R07/R25/R41/R42 |
| A17 | Does history work require source checkout or a versioned Studio layout? | Fixed catalog runtime data path outside releases, tested for observed legacy-flat and supported versioned layout. | R43 |
| A18 | Does finite self-update observation smuggle in M7 automatic behavior? | It only reads the known already-requested journal after readiness, at most 1 Hz for 120 s; no release check/apply/reconcile/restart and no generic recovery policy. | R25/R44 |
| A19 | Does the UI misrepresent old PID/state as live or re-enable Apply? | Separate history/live models and actionable=false historical attempts; all actions consult existing live authority. | R02/R38/R39 |
| A20 | Can local docs or an old PASS archive clear publication gates? | Separate planning/implementation/native/publication status. Current M5 publication qualification remains blocked; no release work performed. | Q14–Q16 |

## Revisions specifically resulting from this review

The plan now explicitly includes the MCP-apply coordinator and whole-registry mutation prerequisites rather than assuming older common docs prove those source paths safe. It does not make Tunnel journal cleanup depend on historical storage. It uses independent effect/audit results instead of returning a retryable error after a physically successful operation. It freezes size/retention/read/queue limits early, and fixes cursor projections to their high-water snapshot instead of joining later mutable outcomes.

For MCP/Gateway/Fleet, a health/catalog/identity proof before rollback-material cleanup is **verification evidence**, not automatically the existing Completed outcome. For Tunnel, a committed verified safety-journal state supports its terminal receipt before warning-only cleanup. These source-specific boundaries must not be flattened into one generic “emit success before cleanup” wrapper.

M6 bootstrap and post-downgrade observations are explicitly not backfilled actions. Historical last-known-good is proof-scoped, not necessarily currently running or currently configured. Unknown predecessor intervals remain explainable as unknown rather than being concealed by a fabricated event.

## Residual limits / qualification obligations

Total storage failure can lose spontaneous events and post-effect terminal details; the design exposes degraded/unknown intervals rather than promising perfect durable audit. Same-UID/root tampering is outside an independently secured audit boundary. Cross-process exact monotonic duration and pre-M6 history cannot be invented. A cursor may expire during legitimate retention; the UI handles it explicitly.

The rusqlite candidate and linked patched SQLite engine have not been compiled or audited in this planning session. Native two-process self-update, real disk-full behavior, both Darwin architectures and all production regression suites remain NOT RUN here. P-01/P-02 are source-inspected risks, not reproduced live incidents or repaired code. Accepting the architecture does not close these implementation/evidence obligations.
