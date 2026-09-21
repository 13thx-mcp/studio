# Candidate SQLite schema v1

**Status:** implementation-ready design fixture for M6.0 freeze, not production migrations. [COMMON](COMMON.md) defines authority and durability; [EVENTS](EVENTS-AND-INTEGRATION.md) defines allowed enum/payload mappings. Validate the exact DDL against the selected bundled engine in M6.1. The planning session can validate only the fixture's relational syntax/constraints with its available SQLite.

## 1. Migration allocation and conventions

M6.1 installs the complete v1 table shape below in one embedded migration `0001_history_v1.sql`; later packages implement producers/projections, not ad hoc table creation. Empty tables for later producers are deliberate and reduce cross-package FK migration races. Further changes after freeze require a new numbered immutable migration and an upgrade fixture; never edit an already-applied migration.

Names/IDs/digests are internal typed values, not executable instructions. All user-controlled text passes validation before SQL binding. JSON columns contain only closed, versioned DTOs; validate using Rust enums/serde with unknown-field rejection and byte limits. No general metadata map, raw object serialization or JSON query endpoint is allowed. IDs are canonical UUIDs except existing validated M5 transaction IDs; entity identities are opaque to the browser. Private SHA fields for config/registry/host/run identity stay out of public DTOs.

SQLite integers are signed 64-bit. Reject overflow rather than wrap/saturate duplicate identities. Generations remain diagnostic; subject reincarnation changes on unregister/re-register, preventing a generation reset from colliding. A bootstrap cannot prove an unobserved removal/re-add during an M5 gap; split the subject incarnation and label continuity unknown.

`events.seq` is the commit-order pagination key. Columns named `*_seq` outside event-child tables are deliberately **logical provenance references, not foreign keys**, because compact projections/pinned lineage may outlive raw events. Their API detail lookup must return a retention boundary rather than a broken link or fabricated row. Event-child tables explicitly use cascading FKs. Owner reducers enforce allowed state transitions and same-source monotonic ordinals; schema CHECK constraints alone are not the state machine.

## 2. DDL

```sql

CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY CHECK(version > 0),
  name TEXT NOT NULL UNIQUE,
  sha256 TEXT NOT NULL CHECK(length(sha256)=64),
  applied_at_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE history_meta (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  db_epoch TEXT NOT NULL UNIQUE,
  created_at_ms INTEGER NOT NULL,
  retention_epoch INTEGER NOT NULL DEFAULT 0 CHECK(retention_epoch>=0),
  policy_version INTEGER NOT NULL DEFAULT 1,
  metric_definition_version INTEGER NOT NULL DEFAULT 1
) STRICT;
CREATE TABLE studio_runs (
  run_id TEXT PRIMARY KEY,
  started_at_ms INTEGER NOT NULL,
  ready_at_ms INTEGER,
  last_seen_at_ms INTEGER NOT NULL,
  ended_at_ms INTEGER,
  end_state TEXT NOT NULL CHECK(end_state IN
    ('open','clean','interrupted','shutdown_incomplete')),
  version TEXT NOT NULL,
  build_identity TEXT,
  process_identity_digest TEXT NOT NULL,
  loaded_config_digest TEXT,
  last_elapsed_ms INTEGER NOT NULL DEFAULT 0 CHECK(last_elapsed_ms>=0),
  started_seq INTEGER,
  closed_seq INTEGER
) STRICT;
CREATE TABLE subjects (
  subject_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('mcp','tunnel','component','studio','registry','fleet','system')),
  component_code TEXT,
  registry_key_digest TEXT,
  incarnation_id TEXT NOT NULL,
  first_observed_ms INTEGER NOT NULL,
  retired_at_ms INTEGER,
  UNIQUE(kind, incarnation_id)
) STRICT;
CREATE TABLE operations (
  operation_id TEXT PRIMARY KEY,
  parent_operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  run_id TEXT REFERENCES studio_runs(run_id) ON DELETE SET NULL,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  action_code TEXT NOT NULL CHECK(length(action_code)<=64),
  actor_kind TEXT NOT NULL CHECK(actor_kind IN
    ('local_operator','system_startup','owner_compensation','external_fleet','system_shutdown')),
  admitted_at_ms INTEGER,
  admitted_seq INTEGER,
  terminal_seq INTEGER,
  effect_status TEXT NOT NULL CHECK(effect_status IN
    ('not_dispatched','pending','succeeded','failed','rolled_back','rollback_failed','indeterminate')),
  audit_status TEXT NOT NULL CHECK(audit_status IN ('pending','complete','incomplete')),
  error_code TEXT,
  CHECK(error_code IS NULL OR length(error_code)<=64)
) STRICT;
CREATE TABLE events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL UNIQUE,
  run_id TEXT REFERENCES studio_runs(run_id) ON DELETE SET NULL,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  source_stream TEXT NOT NULL CHECK(length(source_stream)<=256),
  source_ordinal INTEGER NOT NULL CHECK(source_ordinal>=0),
  name TEXT NOT NULL CHECK(length(name)<=96),
  category TEXT NOT NULL CHECK(category IN
    ('intent','admission','transition','outcome','recovery','rollback','observation')),
  observed_at_ms INTEGER NOT NULL,
  source_at_ms INTEGER,
  elapsed_ms INTEGER CHECK(elapsed_ms IS NULL OR elapsed_ms>=0),
  time_quality TEXT NOT NULL CHECK(time_quality IN
    ('local','monotonic','source_reported','clock_discontinuity','unknown')),
  evidence_kind TEXT NOT NULL CHECK(evidence_kind IN
    ('owner','validated_journal','bootstrap','recovery_observation','gap')),
  payload_version INTEGER NOT NULL CHECK(payload_version=1),
  payload_json TEXT NOT NULL CHECK(length(CAST(payload_json AS BLOB))<=4096),
  payload_sha256 TEXT NOT NULL CHECK(length(payload_sha256)=64),
  retention_class TEXT NOT NULL CHECK(retention_class IN ('audit','operational','observation')),
  UNIQUE(source_stream, source_ordinal)
) STRICT;
CREATE INDEX events_subject_seq ON events(subject_id, seq DESC);
CREATE INDEX events_name_seq ON events(name, seq DESC);
CREATE INDEX events_operation_seq ON events(operation_id, seq);
CREATE INDEX events_time_seq ON events(observed_at_ms, seq);
CREATE INDEX events_retention_time ON events(retention_class, observed_at_ms, seq);
CREATE TABLE audit_events (
  event_seq INTEGER PRIMARY KEY REFERENCES events(seq) ON DELETE CASCADE,
  operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  actor_kind TEXT NOT NULL,
  safety_exception INTEGER NOT NULL CHECK(safety_exception IN (0,1)),
  disposition TEXT NOT NULL CHECK(disposition IN
    ('requested','admitted','rejected','succeeded','failed','indeterminate','safety_executed'))
) STRICT;
CREATE TABLE runtime_sessions (
  session_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES studio_runs(run_id),
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  launch_operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  owner_kind TEXT NOT NULL CHECK(owner_kind IN ('mcp','tunnel')),
  generation INTEGER NOT NULL CHECK(generation>=0),
  pid INTEGER NOT NULL CHECK(pid>0),
  started_at_ms INTEGER NOT NULL,
  started_seq INTEGER NOT NULL,
  last_observed_ms INTEGER NOT NULL,
  ended_at_ms INTEGER,
  closed_seq INTEGER,
  observed_duration_ms INTEGER NOT NULL DEFAULT 0 CHECK(observed_duration_ms>=0),
  exact_duration_ms INTEGER CHECK(exact_duration_ms IS NULL OR exact_duration_ms>=0),
  end_kind TEXT NOT NULL CHECK(end_kind IN
    ('open','requested_stop','clean_exit','unexpected_exit','wait_error','interrupted')),
  exit_code INTEGER,
  is_crash INTEGER CHECK(is_crash IS NULL OR is_crash IN (0,1)),
  last_source_ordinal INTEGER NOT NULL,
  UNIQUE(run_id, subject_id, generation),
  CHECK(end_kind!='interrupted' OR (ended_at_ms IS NULL AND exit_code IS NULL AND is_crash IS NULL))
) STRICT;
CREATE INDEX sessions_subject_seq ON runtime_sessions(subject_id, started_seq DESC);
CREATE INDEX sessions_run_state ON runtime_sessions(run_id, end_kind);
CREATE TABLE lifecycle_events (
  event_seq INTEGER PRIMARY KEY REFERENCES events(seq) ON DELETE CASCADE,
  session_id TEXT REFERENCES runtime_sessions(session_id) ON DELETE SET NULL,
  owner_kind TEXT NOT NULL CHECK(owner_kind IN ('mcp','tunnel')),
  from_state TEXT,
  to_state TEXT NOT NULL,
  reason_code TEXT NOT NULL,
  is_crash INTEGER CHECK(is_crash IS NULL OR is_crash IN (0,1))
) STRICT;
CREATE TABLE update_attempts (
  attempt_id TEXT PRIMARY KEY,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  transaction_id TEXT NOT NULL,
  domain TEXT NOT NULL CHECK(domain IN ('mcp','gateway','fleet','studio','tunnel')),
  originating_run_id TEXT REFERENCES studio_runs(run_id) ON DELETE SET NULL,
  prepare_operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  apply_operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  source_version TEXT,
  target_version TEXT NOT NULL,
  first_observed_seq INTEGER NOT NULL,
  apply_started_seq INTEGER,
  terminal_seq INTEGER,
  last_phase TEXT NOT NULL,
  last_source_ordinal INTEGER NOT NULL DEFAULT 0,
  install_outcome TEXT NOT NULL CHECK(install_outcome IN
    ('not_attempted','pending','succeeded','failed','interrupted','indeterminate')),
  rollback_outcome TEXT NOT NULL CHECK(rollback_outcome IN
    ('not_needed','pending','succeeded','failed','unknown')),
  monotonic_apply_duration_ms INTEGER CHECK(monotonic_apply_duration_ms IS NULL OR monotonic_apply_duration_ms>=0),
  observation_duration_ms INTEGER CHECK(observation_duration_ms IS NULL OR observation_duration_ms>=0),
  verification_scope TEXT NOT NULL CHECK(verification_scope IN
    ('none','artifact_probe','owned_runtime','catalog_probe','external_fleet')),
  was_running INTEGER CHECK(was_running IS NULL OR was_running IN (0,1)),
  completeness TEXT NOT NULL CHECK(completeness IN ('complete','partial','bootstrap','interrupted')),
  UNIQUE(domain, transaction_id)
) STRICT;
CREATE INDEX attempts_subject_seq ON update_attempts(subject_id, first_observed_seq DESC);
CREATE TABLE update_phases (
  event_seq INTEGER PRIMARY KEY REFERENCES events(seq) ON DELETE CASCADE,
  attempt_id TEXT NOT NULL REFERENCES update_attempts(attempt_id) ON DELETE CASCADE,
  phase TEXT NOT NULL,
  source_revision INTEGER,
  error_code TEXT,
  owner_verified INTEGER NOT NULL CHECK(owner_verified IN (0,1))
) STRICT;
CREATE INDEX phases_attempt_seq ON update_phases(attempt_id, event_seq);
CREATE TABLE update_checks (
  event_seq INTEGER PRIMARY KEY REFERENCES events(seq) ON DELETE CASCADE,
  operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  check_outcome TEXT NOT NULL CHECK(check_outcome IN ('ok','error','cancelled')),
  latest_version TEXT,
  installed_version_observed TEXT,
  update_available INTEGER CHECK(update_available IS NULL OR update_available IN (0,1)),
  error_code TEXT
) STRICT;
CREATE INDEX checks_subject_seq ON update_checks(subject_id, event_seq DESC);
CREATE TABLE artifacts (
  artifact_id TEXT PRIMARY KEY,
  component_code TEXT NOT NULL,
  version TEXT,
  provider_code TEXT,
  platform_code TEXT,
  archive_sha256 TEXT,
  content_sha256 TEXT NOT NULL CHECK(length(content_sha256)=64),
  evidence_scope TEXT NOT NULL CHECK(evidence_scope IN
    ('verified_stage','installed_identity','owned_runtime','external_release','bootstrap_identity')),
  first_observed_seq INTEGER NOT NULL,
  UNIQUE(component_code, content_sha256, evidence_scope)
) STRICT;
CREATE TABLE artifact_members (
  artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
  member_role TEXT NOT NULL CHECK(member_role IN ('executable','web_bundle','control_bundle','runtime_tree')),
  ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<16),
  sha256 TEXT NOT NULL CHECK(length(sha256)=64),
  PRIMARY KEY(artifact_id, member_role, ordinal)
) STRICT;
CREATE TABLE install_observations (
  observation_id TEXT PRIMARY KEY,
  event_seq INTEGER NOT NULL UNIQUE,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  attempt_id TEXT REFERENCES update_attempts(attempt_id) ON DELETE SET NULL,
  artifact_id TEXT REFERENCES artifacts(artifact_id),
  layout_kind TEXT NOT NULL CHECK(layout_kind IN ('flat_bin','legacy_flat','versioned_release','control_bundle')),
  state TEXT NOT NULL CHECK(state IN ('staged','installed','verified_active','restored','unknown')),
  proof_scope TEXT NOT NULL,
  previous_observation_id TEXT REFERENCES install_observations(observation_id) ON DELETE SET NULL,
  boundary_reason TEXT,
  observed_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX installs_subject_seq ON install_observations(subject_id, event_seq DESC);
CREATE TABLE observed_lineage_heads (
  subject_id TEXT PRIMARY KEY REFERENCES subjects(subject_id),
  latest_observation_id TEXT REFERENCES install_observations(observation_id),
  previous_observation_id TEXT REFERENCES install_observations(observation_id),
  last_verified_good_id TEXT REFERENCES install_observations(observation_id),
  as_of_seq INTEGER NOT NULL
) STRICT;
CREATE TABLE config_revisions (
  revision_id TEXT PRIMARY KEY,
  surface_code TEXT NOT NULL CHECK(length(surface_code)<=64),
  exact_bytes_sha256 TEXT,
  safe_projection_sha256 TEXT NOT NULL,
  schema_code TEXT,
  observed_seq INTEGER NOT NULL,
  observed_at_ms INTEGER NOT NULL,
  provenance TEXT NOT NULL CHECK(provenance IN ('loaded','committed','bootstrap','external_observation','restored')),
  change_categories_json TEXT NOT NULL CHECK(length(CAST(change_categories_json AS BLOB))<=1024),
  previous_revision_id TEXT REFERENCES config_revisions(revision_id) ON DELETE SET NULL,
  boundary_reason TEXT
) STRICT;
CREATE INDEX config_surface_seq ON config_revisions(surface_code, observed_seq DESC);
CREATE TABLE config_links (
  link_id TEXT PRIMARY KEY,
  event_seq INTEGER NOT NULL,
  revision_id TEXT NOT NULL REFERENCES config_revisions(revision_id),
  run_id TEXT REFERENCES studio_runs(run_id) ON DELETE SET NULL,
  attempt_id TEXT REFERENCES update_attempts(attempt_id) ON DELETE SET NULL,
  relation TEXT NOT NULL CHECK(relation IN ('loaded_by','committed_by','restored_by','observed_for'))
) STRICT;
CREATE TABLE registry_entry_revisions (
  event_seq INTEGER NOT NULL,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  registry_revision_id TEXT NOT NULL REFERENCES config_revisions(revision_id),
  present INTEGER NOT NULL CHECK(present IN (0,1)),
  enabled INTEGER CHECK(enabled IS NULL OR enabled IN (0,1)),
  runtime_kind TEXT CHECK(runtime_kind IS NULL OR runtime_kind IN ('rust','node','python')),
  PRIMARY KEY(event_seq, subject_id)
) STRICT;
CREATE INDEX registry_subject_seq ON registry_entry_revisions(subject_id, event_seq DESC);
CREATE TABLE drift_observations (
  observation_id TEXT PRIMARY KEY,
  event_seq INTEGER NOT NULL UNIQUE,
  operation_id TEXT REFERENCES operations(operation_id) ON DELETE SET NULL,
  host_key_digest TEXT,
  generation INTEGER,
  manifest_digest TEXT,
  state TEXT NOT NULL,
  phase TEXT NOT NULL,
  studio_config_activation TEXT NOT NULL CHECK(studio_config_activation IN ('unknown','active','restart_required')),
  client_freshness TEXT NOT NULL CHECK(client_freshness IN ('unknown','refresh_pending')),
  rollback_outcome TEXT NOT NULL CHECK(rollback_outcome IN ('not_needed','succeeded','failed','unknown')),
  observation_kind TEXT NOT NULL CHECK(observation_kind IN ('check','adopt','apply','rollback','startup'))
) STRICT;
CREATE INDEX drift_seq ON drift_observations(event_seq DESC);
CREATE TABLE drift_surfaces (
  observation_id TEXT NOT NULL REFERENCES drift_observations(observation_id) ON DELETE CASCADE,
  surface_code TEXT NOT NULL,
  desired_revision_id TEXT REFERENCES config_revisions(revision_id) ON DELETE SET NULL,
  active_revision_id TEXT REFERENCES config_revisions(revision_id) ON DELETE SET NULL,
  changed INTEGER NOT NULL CHECK(changed IN (0,1)),
  PRIMARY KEY(observation_id, surface_code)
) STRICT;
CREATE TABLE journal_watermarks (
  domain TEXT NOT NULL CHECK(domain IN ('studio','tunnel')),
  transaction_id TEXT NOT NULL,
  highest_revision INTEGER NOT NULL CHECK(highest_revision>=0),
  payload_sha256 TEXT NOT NULL,
  observed_seq INTEGER NOT NULL,
  last_seen_at_ms INTEGER NOT NULL,
  PRIMARY KEY(domain, transaction_id)
) STRICT;
CREATE TABLE coverage_intervals (
  coverage_id TEXT PRIMARY KEY,
  subject_id TEXT REFERENCES subjects(subject_id) ON DELETE SET NULL,
  run_id TEXT REFERENCES studio_runs(run_id) ON DELETE SET NULL,
  reason TEXT NOT NULL CHECK(reason IN
    ('bootstrap','interrupted','write_failure','queue_overflow','revision_gap','clock_discontinuity','retention','restored_backup')),
  from_seq INTEGER,
  to_seq INTEGER,
  from_ms INTEGER,
  to_ms INTEGER,
  lost_count INTEGER CHECK(lost_count IS NULL OR lost_count>=0),
  completeness TEXT NOT NULL CHECK(completeness IN ('unknown','partial','retained_boundary'))
) STRICT;
CREATE TABLE projection_state (
  projection_code TEXT PRIMARY KEY,
  definition_version INTEGER NOT NULL,
  through_seq INTEGER NOT NULL CHECK(through_seq>=0),
  updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE metrics_buckets (
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  metric_code TEXT NOT NULL,
  definition_version INTEGER NOT NULL,
  resolution_ms INTEGER NOT NULL CHECK(resolution_ms IN (3600000,86400000)),
  bucket_start_ms INTEGER NOT NULL,
  count_value INTEGER NOT NULL CHECK(count_value>=0),
  sum_value INTEGER NOT NULL CHECK(sum_value>=0),
  min_value INTEGER,
  max_value INTEGER,
  through_seq INTEGER NOT NULL,
  quality TEXT NOT NULL CHECK(quality IN ('complete','partial','unknown')),
  PRIMARY KEY(subject_id, metric_code, definition_version, resolution_ms, bucket_start_ms)
) STRICT;
CREATE INDEX buckets_time ON metrics_buckets(resolution_ms, bucket_start_ms);
CREATE TABLE metrics_totals (
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  metric_code TEXT NOT NULL,
  definition_version INTEGER NOT NULL,
  count_value INTEGER NOT NULL CHECK(count_value>=0),
  sum_value INTEGER NOT NULL CHECK(sum_value>=0),
  through_seq INTEGER NOT NULL,
  coverage_start_ms INTEGER NOT NULL,
  quality TEXT NOT NULL CHECK(quality IN ('complete','partial','unknown')),
  PRIMARY KEY(subject_id, metric_code, definition_version)
) STRICT;


CREATE INDEX operations_parent ON operations(parent_operation_id);
CREATE INDEX operations_run ON operations(run_id);
CREATE INDEX operations_subject ON operations(subject_id);
CREATE INDEX events_run ON events(run_id);
CREATE INDEX audit_operation ON audit_events(operation_id);
CREATE INDEX sessions_launch_operation ON runtime_sessions(launch_operation_id);
CREATE INDEX lifecycle_session ON lifecycle_events(session_id);
CREATE INDEX attempts_origin_run ON update_attempts(originating_run_id);
CREATE INDEX attempts_prepare_operation ON update_attempts(prepare_operation_id);
CREATE INDEX attempts_apply_operation ON update_attempts(apply_operation_id);
CREATE INDEX checks_operation ON update_checks(operation_id);
CREATE INDEX installs_attempt ON install_observations(attempt_id);
CREATE INDEX installs_artifact ON install_observations(artifact_id);
CREATE INDEX installs_previous ON install_observations(previous_observation_id);
CREATE INDEX configlinks_revision ON config_links(revision_id);
CREATE INDEX configlinks_run ON config_links(run_id);
CREATE INDEX configlinks_attempt ON config_links(attempt_id);
CREATE INDEX registry_revision ON registry_entry_revisions(registry_revision_id);
CREATE INDEX drift_operation ON drift_observations(operation_id);
CREATE INDEX driftsurfaces_desired ON drift_surfaces(desired_revision_id);
CREATE INDEX driftsurfaces_active ON drift_surfaces(active_revision_id);
CREATE INDEX coverage_subject ON coverage_intervals(subject_id);
CREATE INDEX coverage_run ON coverage_intervals(run_id);

CREATE INDEX heads_latest ON observed_lineage_heads(latest_observation_id);
CREATE INDEX heads_previous ON observed_lineage_heads(previous_observation_id);
CREATE INDEX heads_good ON observed_lineage_heads(last_verified_good_id);
CREATE INDEX revisions_previous ON config_revisions(previous_revision_id);
```

## 3. Transaction recipes

**Admission:** validate the typed request, acquire existing logical owner lease, reserve terminal capacity, allocate operation ID, BEGIN IMMEDIATE; insert operation plus admitted event/audit row; set admitted_seq and pending/not-dispatched fields; COMMIT; acknowledge permit. No domain side effects occur before acknowledgement. The request event may be in the same transaction with a preceding seq. Admission timeouts may leave a record without execution and must never dispatch later from the DB thread.

**Lifecycle:** one immutable event envelope carries subject incarnation, session ID, run ID, generation and the captured transition data. In one transaction, insert event, insert/update session, insert lifecycle row, update projection ordinal/coverage. Start/terminal ordering must be tested under queue reordering; a terminal envelope carries the known original start identity/duration so that reducer completion is not dependent on a delayed start notification. A late older transition cannot reopen a closed session. Domain events are not synthesized from a stored snapshot.

**Update terminal:** event + phase + attempt outcome/rollback outcome + verified artifact/installation observation + lineage-head projection + operation audit terminal all commit together. Unknown source fields stay NULL. Private staged identifiers/paths never enter the schema. Prepare failure leaves install_outcome=not_attempted. Source rollback success can be reported under public Failed; normalize outcome from phase AND rollback_succeeded, not phase alone.

**Journal observation:** validated source `(domain, transaction_id, revision)` and sanitized payload digest are deduplicated before inserting. Same key/same payload is no-op; same key/different payload marks integrity failure. Insert event/projection and advance journal_watermarks atomically. Never ignore a revision conflict. A gap in observed revisions creates coverage, not intermediate phase rows. Watermarks are history ingestion state, not instructions for journal recovery.

**Registry/config:** owner file commit returns an immutable safe receipt; one SQL transaction records config revision, registry-entry revisions and audit terminal. Repeated identical reads do not create a new revision, but a later return to the same digest after another revision does: revision IDs are observation identities, not content digests. `loaded_by` versus `committed_by` links keep loaded/running config distinct from current file contents.

**Metrics:** read `(through_seq, definition_version)`, select a bounded event batch in ascending seq, update both hourly/daily buckets and totals, then advance the watermark in one writer transaction. Retrying a rolled-back batch cannot double-count. Update-counter identity is attempt/action based; a replayed phase cannot increment it twice. See [metrics](METRICS-AND-RETENTION.md).

**Retention:** finish aggregations through the deletion boundary first. Increment global retention_epoch atomically with a bounded deletion batch. Remove expired dependent observations first, detach an explicitly marked lineage/config boundary, then raw events, unused operations/runs/subjects/artifacts. Never blindly cascade an operational event whose audit retention requirement has not expired. Pinned latest/previous/last-known-good artifact/config anchors retain a bounded summary, not every ancestor.

## 4. Required indexes and query checks

The DDL includes FK/query indexes for event timeline, subjects, operation details, session/update ordering, phase history, release checks, config/drift history and metric windows. M6.1 tests `PRAGMA foreign_key_check`; M6.7 adds EXPLAIN QUERY PLAN assertions for supported queries against large fixtures. The candidate also freezes child-FK indexes for retention joins. Verify all remaining query plans on the selected engine before applying v1; later index changes use the migration engine. Do not solve performance by accepting user SQL/index hints or retaining a read transaction across HTTP pages.

State/detail pagination uses immutable first-observed seq; phase/status filters must be evaluated at the cursor's snapshot seq from event/phase records, not against a later mutable projection. Any response that explicitly returns a latest projection must declare a separate projection_as_of_seq and cannot claim snapshot-exact state. Metrics expose their own aggregation watermark and quality.

## 5. Retained-lineage example

An MCP operator apply links `operation A` → `attempt U` → verified stage artifact `T` → installed observation `I2`, previous `I1`. Config revision `C2` is linked to its committing operation and a later Studio run's loaded observation where actually proven. A rollback creates a new observation referencing restored content, not a deletion/rewrite of I2. Same-version Tunnel reinstall creates distinct artifact content identity when bytes differ. On retention, a compact I1/C1 boundary explains the retained starting point; the UI states that earlier actions expired.

Fresh M5 bootstrap creates only `bootstrap_identity`/config observations at first M6 observation time; it cannot create operation A or an installation date from the presence of a file. A crash between action and receipt leaves an admitted operation with indeterminate terminal outcome plus independently observed current identity. That is evidence of uncertainty, not proof that a particular action succeeded.

## 6. Reducer/idempotency details frozen for implementation

`runtime_sessions.started_seq` is the immutable first durable observation of the proven spawn identity. Normally this is the start event; when a terminal envelope containing the original observed spawn facts arrives first, it anchors the session at that first durable observation. A later start envelope does not change page membership, reopen the session or invent a second spawn. Exact `started_at_ms` remains the captured original observation time, not the commit time. Test this explicitly rather than assuming channel order.

The first qualifying `update_attempts.terminal_seq` is immutable for metric contribution. Higher-revision repeated terminal journal observations may update diagnostic observation coverage but do not emit a new qualifying operation outcome. A contradictory terminal result is an integrity/coverage finding, not a silent overwrite or second success. Bootstrap imported terminal evidence has no new M6 apply contribution. Metrics count only canonical qualifying facts; later audit-delivery resolution is not another physical outcome.

For journal revisions below the retained watermark, never insert/recount an old phase. If the exact older event is retained, compare its payload digest and reject a conflict. If it was pruned, classify it stale/unverifiable and leave the watermark/projections unchanged; do not claim its old bytes were verified from a highest-revision digest alone. At the highest revision, same digest is idempotent and different digest is an integrity failure. New owner streams are created only by the current authorized producer/run, not by an HTTP event ingestion endpoint. This avoids unbounded permanent duplicate tombstones and old-journal replay into totals.
