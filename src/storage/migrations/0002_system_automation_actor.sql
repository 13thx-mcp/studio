CREATE TABLE operations_v2 (
  operation_id TEXT PRIMARY KEY,
  parent_operation_id TEXT REFERENCES operations_v2(operation_id) ON DELETE SET NULL,
  run_id TEXT REFERENCES studio_runs(run_id) ON DELETE SET NULL,
  subject_id TEXT NOT NULL REFERENCES subjects(subject_id),
  action_code TEXT NOT NULL CHECK(length(action_code)<=64),
  actor_kind TEXT NOT NULL CHECK(actor_kind IN
    ('local_operator','system_startup','system_automation','owner_compensation','external_fleet','system_shutdown')),
  admitted_at_ms INTEGER,
  admitted_seq INTEGER,
  terminal_seq INTEGER,
  effect_status TEXT NOT NULL CHECK(effect_status IN
    ('not_dispatched','pending','succeeded','failed','rolled_back','rollback_failed','indeterminate')),
  audit_status TEXT NOT NULL CHECK(audit_status IN ('pending','complete','incomplete')),
  error_code TEXT,
  CHECK(error_code IS NULL OR length(error_code)<=64)
) STRICT;
INSERT INTO operations_v2 SELECT * FROM operations;
DROP TABLE operations;
ALTER TABLE operations_v2 RENAME TO operations;
