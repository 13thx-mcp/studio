# ADR 0043 — Fleet Automation Policy Schema and Bounded Automatic Reconciliation

- **Status:** Accepted for M8.5 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-09, P-17, P-18, Fleet host schema v3, managed policy authority

## Context

Fleet owns deterministic desired-state rendering for `runtime/studio/studio.toml`. M8 automation policy cannot become a second Studio-local desired-state file without creating reconciliation churn and split authority.

RuntimeReconciler already classifies managed-safe drift and performs transactional backup/apply/rollback, but has no periodic retry/circuit policy.

## Decision

Fleet target 0.5.0 introduces **host schema v3**.

Fleet 0.5.0 may read host schemas 1, 2 and 3 for compatibility.

- schema v1/v2 render no explicit M8 automation section; Studio missing-field defaults remain manual/disabled;
- schema v3 may declare automation policy and is required for non-default Fleet-managed automation.

## Host schema v3 automation shape

Normalized conceptual TOML:

```toml
schema_version = 3

[automation]
startup_grace_seconds = 30
health_interval_seconds = 15
cleanup_interval_seconds = 3600

[automation.updates]
policy = "manual"
check_on_startup = false
check_interval_seconds = 21600
max_consecutive_failures = 5
circuit_cooldown_seconds = 21600
prepared_ttl_seconds = 21600
max_staging_bytes = 1073741824

[automation.reconciliation]
policy = "manual"
check_on_startup = true
check_interval_seconds = 300
max_consecutive_failures = 5
circuit_cooldown_seconds = 21600

[automation.restart]
mcp = "disabled"
tunnel = "disabled"
max_attempts = 3
stability_window_seconds = 30
initial_backoff_seconds = 1
max_backoff_seconds = 30
circuit_cooldown_seconds = 60

[automation.maintenance]
weekdays_utc = ["sat", "sun"]
start_utc = "02:00"
duration_minutes = 120
```

Exact parser ranges follow ADR 0038/0039 and reject unknown/unsupported enums or future schema.

Fleet renders this deterministically into the existing `studio.config` managed surface. It does not create a second automation policy file.

## Loaded-policy freshness

Studio binds AutomationController to the SHA-256 identity of the exact loaded `studio.toml`.

Prepared automation references also record that fingerprint.

If reconciliation changes the Fleet-managed Studio config:

- `studio_restart_required=true`;
- current AutomationController enters observation-only mode for destructive policy;
- auto activation and auto reconciliation apply are blocked;
- new Studio process must prove it loaded the reconciled canonical config bytes;
- old prepared authorization is re-evaluated and cannot inherit across the policy change.

## Periodic reconciliation

RuntimeReconciler remains a one-shot domain owner.

AutomationController may schedule:

- check only;
- check + apply only when `ManagedSafeDrift && safe_to_reconcile`.

It never invokes `adopt` automatically.

Unknown, unmanaged conflict, broken state, local/secret ambiguity or rollback failure is operator-only.

## Retry/circuit

Reconciliation circuit metadata lives in AutomationState, not `reconciliation.json` or SQLite history.

Default circuit follows ADR 0038.

- check/render failures count according to policy;
- an apply that begins and fails/rolls back counts as failure;
- rollback failure opens a hard operator blocker immediately;
- safe deferral does not count as failure.

A circuit reset changes policy state only; it does not apply drift immediately.

## Safety gate

Auto reconcile additionally requires:

- active loaded policy fingerprint;
- audit admission;
- RuntimeOperationCoordinator availability;
- Gateway M8 safety capability/status;
- no open unsafe hold;
- stable required runtime owners.

## Alternatives considered

### Store automation config in a separate Studio JSON file

Rejected for Fleet-managed hosts because Fleet would no longer be the single desired-state authority.

### Automatically adopt local differences

Rejected because adopt changes the desired baseline and could legitimize unknown edits.

### Persist reconciliation retry state in M6 history

Rejected because history cannot authorize live mutation.

## Consequences

- automation policy participates in the same Fleet drift/restart proof as other Studio config;
- legacy Fleet host profiles remain manual-safe;
- policy changes cannot silently take effect in an old process;
- reconciliation loops are bounded across restart.
