# M6 common contracts — proposed normative freeze

**Status:** proposed for M6.0 acceptance; implementation NOT STARTED. Baseline and caveats: [SOURCE-AUDIT](SOURCE-AUDIT.md).

## 1. Scope and vocabulary

M6 persists operational, update, configuration/drift history and lifecycle-derived metrics. A **live owner** decides actual state; an **observation** describes evidence seen at a time; an **operation** is a request/admission/execution episode; a **session** is one successfully spawned, owned child generation; a **Studio run** is one observation process instance. A **release identity** includes verified content evidence, not just a version string. A **history gap** means completeness cannot be asserted. A **retention boundary** means evidence was deliberately expired under the declared policy.

M7 unattended update/check/reconciliation/restart/backoff/circuit-breaker/maintenance/recovery policies are excluded. Existing M5 compensating rollback/startup recovery remains unchanged. Housekeeping of this history store and finite observation of an already-authorized self-update are not policies that operate components. No later Gateway request/tool-call/usage telemetry, CPU or memory sampling is invented.

## 2. Invariants

| ID | Contract |
|---|---|
| M6-I01 | SQLite is authoritative only for committed historical records and derived query projections, never current configuration, desired policy, executable activation or recovery decisions. |
| M6-I02 | Registry TOML/config files/Fleet manifest/M5 journals retain their existing authority and formats. No migration of registered servers to SQLite in M6. |
| M6-I03 | Live owner state wins over all historical observations; old PID/generation/version fields cannot authorize a signal, restart, rollback or “running” status. |
| M6-I04 | Fleet/Tunnel/Gateway/Studio ownership and F1–F8 remain intact. Preserve coordinator-before-manager-local lease order; close P-01/P-02 before integration. |
| M6-I05 | Mandatory discretionary admission evidence is committed before a physical side effect. Requested/admitted is not success. Terminal success requires the authoritative owner's verified outcome. |
| M6-I06 | No SQLite I/O or await while holding runtime/state/registry/coordinator mutexes. A logical operation lease may remain held for bounded admission/terminal flush; its underlying mutex is already released and the writer never calls an owner. |
| M6-I07 | Once side effects begin, history failure never short-circuits rollback, safe stop, restart compensation or journal recovery. Never propagate a new history error with `?` into an existing destructive phase chain. |
| M6-I08 | Failures of required audit delivery are visible in operation durability, health and coverage. No success-shaped durable-audit acknowledgement when COMMIT is unconfirmed. Physical success and audit success are separate values. |
| M6-I09 | Ingestion occurs at typed owner boundaries, not through the lossy EventHub or raw logs. SQL event + projection + idempotency state commit atomically. |
| M6-I10 | Ordering uses commit sequence plus source stream/ordinal; timestamps are observations, not unique IDs or causality. Browser 64-bit sequences/counters are decimal strings. |
| M6-I11 | No secrets, config bodies, env, argv, internal paths, raw exception/command output or credential hashes in public history. Private config digests are never serialized to browser DTOs. |
| M6-I12 | Explicit coverage/unknown fields for crashes, missing phases, clock discontinuity, bootstrap, degraded persistence and retention. No invented events from current state. |
| M6-I13 | Finite queries, queues, records, transactions, DB/WAL growth and retention from the foundation onward. Blocking new audit-dependent actions is preferable to silent audit deletion. |
| M6-I14 | Embedded checksummed migrations fail closed for the history store; no automatic delete/recreate, external SQL loading, destructive downgrade or repair from untrusted bytes. |
| M6-I15 | Source-less execution and both supported Studio layouts locate the same private history root independently of cwd and source checkout. |
| M6-I16 | M6 implementation completion and publication qualification are distinct; M5 publication gate cannot be erased by M6 closure. |

## 3. Data-authority allocation

**Remains file-backed:** registry schema 1; Studio config and configured desired versions; Fleet desired host/schema/rendered surfaces and last-managed manifest; self-update and Tunnel safety journals; installed binaries/releases/current pointer/checksum material. Runtime owners/caches retain their current in-memory responsibilities.

**SQLite-authoritative:** historical event sequence, audit admissions/delivery status, historical Studio/child sessions, sanitized registry/config observations, observed update phases/checks/artifacts/install/lineage, historical drift observations, coverage markers, aggregation checkpoints and metrics buckets/totals. These can be queried after restart, not used to resume a volatile update.

**Mirrored only:** exact loaded config digest (private), safe registry metadata, source journal revision/phase with validated identity, verified artifact fingerprints, live owner transition facts, managed manifest generation/digests (private), local catalog verification scope. Mirrors never repair or overwrite their source files.

There is no distributed transaction across TOML/JSON/filesystem actions and SQLite. The protocol is durable intent, independently authoritative action, durable receipt when possible, explicit indeterminate interval otherwise. Current state alone cannot prove an unobserved past success.

## 4. Store decision

Use `rusqlite = =0.40.2` as the concrete candidate, `default-features = false`, selected native features `libsqlite3-sys`, `bundled`, `backup`, `hooks`, `limits`. Pin the resolved transitive graph in Cargo.lock in M6.1, not in this planning session. Do not choose `bundled-full`, SQLCipher, load_extension, runtime migrations, sqlx compile-time query DBs or a multi-writer pool. A build must report the linked SQLite version and source ID and satisfy the patched-engine rule in [security/operations](SECURITY-AND-OPERATIONS.md). A candidate that fails those checks must be replaced under ADR review before M6.1 exit; selection is not a claim that this exact dependency has already been built here.

One dedicated OS thread exclusively owns the writable connection; two dedicated read workers own read-only/query-only connections. Tokio tasks use bounded channels/oneshot receipts, never `Arc<Mutex<Connection>>` across awaits. No SQLite on the Tokio executor. The storage service owns all connections, checkpoints and maintenance. An advisory store-owner lock prevents concurrent M6 writers/boot-repair of the same history DB; it never authorizes runtime control or PID signalling.

Location: `ComponentCatalog::runtime_root()/studio/data/history/studio.sqlite3`; private sibling `owner.lock` and `backups/`. No configurable database pathname or browser path parameter. No file in `bin`, `current`, `releases`, source tree or a user-selected arbitrary filesystem location. History-specific directory 0700; files/backups/sidecars 0600. Do not chmod unrelated existing runtime directories merely to install history.

Default pragmas (read back and verify): new DB `page_size=4096`, `auto_vacuum=INCREMENTAL` before schema creation; `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`, `trusted_schema=OFF`, `busy_timeout=250`, `wal_autocheckpoint=1000`, `journal_size_limit=16777216`, `mmap_size=0`, `temp_store=MEMORY`; Darwin `fullfsync=ON`, `checkpoint_fullfsync=ON`. Read workers additionally `query_only=ON`, no CREATE/URI/ATTACH capability. Set defensive connection configuration where supported and verify it. FULL is the selected durability policy, not a promise against broken hardware or malicious same-user filesystem changes.

## 5. Boundedness / failure protocol

Writer requests: 512 regular slots + 64 safety/terminal reserve slots, each at most 8 KiB; typed event payload at most 4 KiB. At most 32 admitted concurrent logical operations; completion slots are reserved at admission. Overflowed low-value observations are coalesced with a coverage-loss count; terminal obligations never silently disappear. No unbounded task spawning to evade channel capacity.

Admission has a 750 ms end-to-end acknowledgement budget including queueing; SQLite busy wait is at most 250 ms. A timeout does not cancel an in-progress SQLite fsync and does not prove rollback. Late admission commits must NEVER dispatch the action: only the live caller holding an unexpired permit may dispatch. They remain admitted/not-dispatched or indeterminate observations, with recovery resolution when supported. Idempotency keys identify history records, not authority to replay operations.

After any action reaches a safe stable point, a terminal flush may await up to 750 ms with no runtime mutex held. On failure, return/report the real domain result plus `audit_status=incomplete|pending`; block further discretionary admissions and expose the affected operation/coverage. A post-action audit failure is not a retryable domain failure. Shutdown signals/compensation are issued first; final history draining is bounded to 2 s after existing owner shutdown. A stalled worker must not hold process exit hostage.

Required admissions: manual start/restart, registry/discovery changes, update check/prepare/apply, explicit reconciliation check/adopt/apply. Recovery, spontaneous exits and safe stop have a **safety exception**: attempt capture through the reserved channel without waiting before action, complete independently, report degradation. Existing startup reconciliation effects remain safety/startup observations under that exception; no new periodic checks are added.

If all writable storage is unavailable, the system cannot guarantee a terminal row. M6 does not claim otherwise. Durable admission/open-run records reveal unresolved intervals on next healthy startup; coverage is marked unknown rather than inventing a terminal event. In-memory emergency notices (max 64) help the current UI but are not claimed durable. There is deliberately no second historical file outbox or audit-driven retention of recovery journals.

## 6. Startup / shutdown ordering

After config load and root resolution, start history initialization before constructing owners so every owner receives one `HistoryHandle`. The read/write store checks ownership, schema and migration safety before publishing a healthy handle. Limit startup database initialization to a 5 s readiness budget; a worker may finish only while holding its store lock, without triggering runtime actions. On open/corruption/migration/disk failure, owners receive an explicitly degraded handle, not a silent no-op success sink. Known store-owner contention rejects a second M6 writer; never mark its live run interrupted.

Keep existing Tunnel recovery → self-update startup observation → reconciliation check ordering (`src/main.rs:117-132`), regardless of history availability. Readiness proof remains strictly after listener bind (`:140-145`); history health is not silently added to Fleet's binary/config readiness predicate. History schema incompatibility therefore degrades audit-dependent features rather than causing a DB-driven artifact rollback.

On successful open, start a unique run, identify prior nonterminal observation runs as interrupted, and create bootstrap observations from current validated sources. Do not set old children stopped or crashed from that act. A clean `run.closed` is persisted only after owned child shutdown and accepted history writes are drained. If owner shutdown failed, record `shutdown_incomplete`, not a clean terminal result.

## 7. Identity, time and cross-store uncertainty

Use cryptographically random opaque run/session/subject-incarnation/operation IDs (UUID v4 encoded canonically; dependency locked during M6.1). `events.seq` is SQLite INTEGER PRIMARY KEY AUTOINCREMENT; gaps are legal and sequence values never reset within a DB epoch. Each event has a unique `(source_stream, source_ordinal)` and payload digest to detect conflicting duplicates. A stream includes run + owner/entity incarnation; journal replay keys instead include domain + transaction + persisted revision. Conflicting same-key bytes are a source-integrity failure, not `INSERT OR IGNORE` success.

Wall time is UTC epoch milliseconds with checked conversion; monotonic elapsed times are captured within one process. Persist source-reported journal time separately and flag backwards/out-of-range values. Use `observed_at_ms`, `source_at_ms` nullable and `time_quality`; never sort deterministic pages by wall time alone. New history IDs do not replace existing M5 transaction IDs or process config identity.

Closed session averages exclude open/interrupted sessions. Cross-process self-update end-to-end latency is a labeled observation interval, not an exact monotonic duration. Config revisions contain safe change categories and private exact-byte digests, never credential values. Registry name/id cannot be presumed secret-free free text: public history uses opaque subject IDs and catalog labels; currently authorized live UI may join its existing registry display separately.

## 8. Acceptance discipline

Every implementation package must keep its requirement IDs and permanent test IDs mapped to evidence. Proposed schema SQL in documentation is a design fixture, not an installed migration. No automatic authority migration, no release/version change and no production qualification claim is implied by acceptance of this plan.
