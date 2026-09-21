# Security, bootstrap, migrations and operator behavior

**Status:** accepted M6 security/operations design; final implementation disposition is **VERIFIED / CLOSED**. Existing M5 threat model and ownership remain normative; this document adds history-specific decisions, not a new remote-access security model.

## 1. Threat model and boundaries

| Threat | Required control | Permanent verification |
|---|---|---|
| Secrets in rich owner structures | Hand-built typed allowlist projections; no Serialize of RegistryDocument, StagedArtifact, journal, status error or config body; no env/argv/logs | Inject distinct sentinel secrets into every omitted field and inspect DB, WAL-visible rows, DTOs, logs and backups |
| Free-text names / command output | Opaque subject IDs and fixed catalog labels; bounded enum error codes; no raw command/exception body | Control characters, oversized Unicode, HTML, path-looking names and malicious errors never enter public history |
| Config-hash guessing | Private exact-byte hashes only where needed for config lineage; never hash or store individual credentials; no public hashes | Public DTO projection omits private digests; raw secret fields absent from schema producers |
| DB path/symlink replacement | Derived fixed private directory, component-wise lstat/ownership/type checks, no-follow open where applicable, no URI filenames; validate DB, lock and existing WAL/SHM/backup names | Symlink at each component, dangling link, hardlink count>1, nonregular device/FIFO, wrong owner and writable-by-other directory rejected |
| Same-user tampering | Validate imported records/types/revisions/identities; detect conflicting idempotency key; bounded integrity checks; never trust history for runtime control | Forged history cannot trigger launch/apply; corrupted same-key payload fails; no nonrepudiation claim |
| SQL injection/query abuse | Prepared bindings and finite reviewed SQL templates; no ATTACH/load_extension/SQL endpoint, defensive mode/trusted_schema OFF | SQL-shaped values remain values; arbitrary filters/order rejected; deadlines and size caps enforced |
| Migration corruption | Embedded immutable checksummed migrations, contiguous versions, transaction and schema validation, no filesystem SQL loading | Wrong checksum, missing migration, foreign application_id, unexpected future version and injected failure leave old schema unchanged |
| Clock confusion / audit phase confusion | Server observation time, source time distinct, source ordinal plus commit sequence, explicit intent/outcome taxonomy | Clock jump, repeated timestamp, delayed phase and source timestamp outside limits cannot change causal result |
| Browser enumeration | Loopback/Host/origin protections and no CORS for new routes; opaque IDs, no-store; no history action endpoint | Cross-origin/script/Host tests and schema snapshot allowlists |
| Resource exhaustion | Finite queues, payloads, query deadlines, row/page budgets, retention and admission pressure | Hot readers, long history, many subjects, disk-full and blocked checkpoint cases |

The filesystem trust boundary is the local Studio OS identity and trusted runtime-root ownership. A same-UID attacker who can replace the parent directory can race pathname-based SQLite open; lstat plus private permissions is **not** a claim of race-free confinement against that attacker. Reject unsafe ownership/write permissions instead of chmod-ing an attacker-controlled tree. SQLite's normal pathname API and OS lock do not make this an independently tamper-proof audit log. Do not claim cryptographic nonrepudiation, encrypted storage, protection from root, or multi-user authentication in M6.

## 2. Store creation/open sequence

Resolve runtime_root through existing configuration/catalog logic; derive `studio/data/history` without reading browser input or following Studio's release pointer. Validate all existing components under the trusted canonical runtime root and create only missing history-specific directories privately. Reject symlink/nonregular/hardlinked DB, lock or sidecar files before open. Existing private files must belong to the current effective OS user. Create new files with restrictive modes at creation, not a later broad permissions window; verify sidecar modes after SQLite open and checkpoint. Preserve unrelated runtime files and parent permissions.

Acquire a retained advisory `owner.lock` using the pinned Rust toolchain's nonblocking file-lock API. It serializes history writer startup/migration/interruption marking, not runtime control. Known lock contention is **not evidence that a previous run died**: leave its runs untouched, expose history unavailable in the second process, and preserve the existing application's separate runtime admission/listener behavior. M6 does not add a second runtime owner or signal a PID from the lock file. Support only local filesystems with reliable locking/WAL; network filesystem deployment is unsupported and must be documented/detected where practical, not silently fall back to unsafe mode.

Open one writer, verify SQLite version/source ID and connection options, application identity and schema checksums, run bounded quick/integrity checks as specified below, migrate, and then open read workers. Establish a new run and conservative interrupted-observer coverage before admitting new history-dependent actions. `PRAGMA application_id` is a fixed project constant chosen/frozen in M6.0 (`0x4d435348`, ASCII MCSH); `user_version` tracks the history migration number. Never assign SQLite's internal `schema_version` pragma manually. An unrelated nonempty DB is not a blank M6 DB.

Foundation must return a degraded HistoryService handle on history-only failure within a 5 s readiness budget; there is no silent production Noop sink. Existing Tunnel recovery/self-update observation/reconciliation startup still executes through its original authority. Listener binding, `/health` and process-bound readiness are not made contingent on a history migration/backup job. If a blocking OS write cannot be cancelled, abandon waiting without authorizing an action from a late result, reject admissions, and ensure no stale worker can later take ownership of a new store epoch.

## 3. Failure policy

| Failure | History behavior | Runtime/action behavior |
|---|---|---|
| DB busy / queue full before admission | Finite wait, return `history_busy` or `history_admission_timeout`; record rejection only when possible | Do not dispatch discretionary action; safe stop/recovery continues |
| Read-only filesystem / disk full | Degraded, invalidate audit-completeness claim, preserve known pending obligations | No new audit-dependent action; already-running action reaches safe terminal/compensation without history errors short-circuiting it |
| Corrupt DB / invalid checksum / wrong application identity | Preserve bytes in place; disable store; bounded sanitized reason and operator repair guidance | No history-controlled replay, rollback or fail-open mutation admission |
| Newer schema than this binary | Incompatible, do not write or auto-downgrade | Live read/safe shutdown and existing M5 recovery retain their behavior; discretionary M6 admission denied |
| Unsafe/symlinked directory | Never follow/repair/create through it; store unavailable | No DB path authority reaches owners; safety unchanged |
| Post-effect COMMIT timeout | Domain outcome remains true; audit pending/incomplete; late receipt may resolve only history | Do not return a retryable domain-failure instruction and do not run operation twice |
| Mandatory source journal persistence failure | Existing owner-defined error/rollback/repair contract wins | M6 history must not hide or downgrade it to a mere telemetry warning |
| Retention/backup failure | Old committed store remains source; no automatic delete/recreate; health warning | Do not invent missing evidence or change active config/artifact |

Expose safe error codes in history health and structured mutation response metadata. For minimal compatibility, M6.2 adds operation correlation/durability response headers to existing mutation APIs; M6.7 UI consumes them and fetches the operation view. A success body means the existing domain operation succeeded, while the headers independently state audit delivery. New audit errors before effects return an error with `effect_started=false`. Post-effect warnings must not be rethrown as ordinary retryable failures. Header names and exact tests are frozen in M6.0 (`X-Studio-Operation-Id`, `X-Studio-Audit-Status`, `X-Studio-Effect-Status`). Realtime health is an additional warning, not the sole acknowledgment.

## 4. Migrations and bootstrap

Migration files are compile-time embedded and listed in Rust with version/name/SHA-256. On a new DB apply page/autovacuum policy, application_id, migration ledger and `user_version` atomically with the complete schema v1. On upgrade verify every recorded checksum and contiguous version before running one `BEGIN IMMEDIATE` transaction per migration, updating the ledger/version in that transaction. Test DDL failures and process interruption; never mark a failed migration applied. Foreign keys and CHECK constraints are on for all writes. Run `quick_check` with an operational deadline on open; run full `integrity_check` plus `foreign_key_check` for migration/backup qualification and operator verify. A timeout is unavailable/unverified, not corruption proven or integrity PASS.

There is no historical M6 schema in M5. Test M5→v1 as an empty-history bootstrap, not a fictitious migration of registry TOML. For previous-schema testing, M6.1 adds a test-only v1→v2 additive migration fixture and failure/rollback case; when an actual schema v2 exists, use the real shipped v1 fixture. Do not advertise test-only v2 as released production schema.

First M6 startup reads existing authorities only through their validated owner readers: registry safe projection, actual loaded Studio config identity, known installed identities and managed manifest observations; capture legacy/versioned layout without moving files. Create `bootstrap.observed` evidence at the current observation time. Existing unknown launches/updates/crashes are not reconstructed. Pre-M6 terminal journal evidence can be imported as a historical source observation with its source timestamp, not as a new M6 install success, new run launch or exact duration. Journals lacking observed intermediate revisions get a gap. Registry/server metadata is not written back from SQLite.

After an M5 downgrade interval, DB last-seen state cannot prove registry identity continuity or config lineage. On return create a new observation run and coverage gap, compare current sources for diagnostic observations and use new subject incarnations where continuity cannot be proven. Do not restore old desired versions from historical success rows. For self-update to a candidate with a newer schema, prefer additive read compatibility; history incompatibility alone must not falsify the new process's M5 health proof or force a DB-driven rollback. Safety journals keep their exact existing protocols.

## 5. Backups, restore, corruption and downgrade

A normal operator backup uses SQLite Online Backup API into a new private fixed-name candidate; never copy only the main WAL database file. Bound each backup step and yield to active terminal writes. Validate application ID, migrations, full integrity and foreign keys on the candidate, fsync file/parent, write a bounded manifest with store epoch/schema/SQLite source ID/build and retained coverage, then atomically publish the completed backup. At most two completed backups plus one candidate; refuse insufficient space rather than deleting the only good backup to begin an upgrade.

Before a schema upgrade of a nonempty DB, require a successful backup. If migration/backup cannot fit the 5 s startup readiness envelope, do not hold readiness; history remains unavailable and operator runs the offline maintenance command before restarting history-dependent use. Never migrate concurrently through an abandoned worker plus a new writer. Provide a local CLI design in M6.1: `mcp-studio history verify` and `mcp-studio history backup` operating only on the derived configured store and fixed backup directory, mutually exclusive with online ownership unless explicitly routed through the running store; no arbitrary SQL/path browser interface. Freeze exact CLI parsing before implementation so current `--config` invocation is preserved.

Restore/reset is offline operator maintenance, not an HTTP endpoint or unattended repair. Stop the history owner, preserve the damaged DB/WAL/SHM as one identified set without following links, validate the backup in a candidate location, and atomically install only under store ownership. Change db_epoch so old browser cursors cannot cross a restore. Record the backup cutoff, restore observation and lost interval; no claim that events after the snapshot survived. Restoration never rolls registry/config/release pointers or source journals back. A forensic archive may contain private digests and must remain private.

Do not automatically downgrade SQL. An older M6 binary that sees a newer incompatible history schema leaves it untouched and denies history-dependent admissions. M5 does not consult the separate history directory; it continues using its own files, but creates an observation gap while M6 is absent. Native qualification must exercise candidate update then binary rollback with preserved DB, not infer compatibility solely from an additive DDL description.

## 6. Primary technical references and dependency qualification

These references support library/engine behavior, not a claim of current project implementation. Consulted 2026-09-19:

- Rusqlite 0.40.2 connection and feature documentation: https://docs.rs/rusqlite/0.40.2/rusqlite/struct.Connection.html and https://docs.rs/crate/rusqlite/0.40.2/features . Candidate uses the native bundled engine and explicit backup/hooks/limits features; verify the actual Cargo.lock dependency graph during M6.1.
- SQLite WAL: https://www.sqlite.org/wal.html . Concurrent readers do not provide multiple concurrent writers; WAL requires same-host shared memory, checkpointing can be delayed by readers, and the live WAL is part of durable data.
- **Patched engine gate:** the WAL documentation identifies a WAL-reset corruption bug through SQLite 3.51.2, fixed in 3.51.3, with selected backports. This plan requires linked SQLite **>=3.51.3** (or a specifically reviewed patched alternative) and records `sqlite_version()` and `sqlite_source_id()`. The candidate rusqlite version has not been built here; do not assume its bundled engine passes without checking.
- SQLite pragmas: https://www.sqlite.org/pragma.html ; application/user schema metadata and defensive options are explicit, and journal_size_limit is not a hard live WAL quota.
- SQLite Online Backup API: https://www.sqlite.org/backup.html . Use consistent snapshots rather than main-file-only copies.
- Rust File locks: https://doc.rust-lang.org/std/fs/struct.File.html . Retain the actual file handle for advisory ownership; PID text is not lock authority.
- Tokio blocking work: https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html . Blocking work is not cancelled simply by dropping an awaiting future; long-lived storage workers use explicit dedicated threads.

Run RustSec/cargo audit and frontend dependency audit at implementation qualification with timestamped advisory data. Pin and document native SQLite source/compile options on darwin-arm64 and darwin-amd64. No arbitrary latest-library upgrade is part of this documentation session.
