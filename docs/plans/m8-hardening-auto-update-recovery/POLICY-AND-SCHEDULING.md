# M8 Policy and Scheduling Contract

## 1. Configuration

Proposed additive shape:

```toml
[automation]
startup_grace_seconds = 30
health_interval_seconds = 15
cleanup_interval_seconds = 3600

[automation.updates]
policy = "manual" # manual | notify-only | auto-prepare | auto-update-safe
check_on_startup = false
check_interval_seconds = 21600
max_consecutive_failures = 5
circuit_cooldown_seconds = 21600
prepared_ttl_seconds = 21600
max_staging_bytes = 1073741824

[automation.reconciliation]
policy = "manual" # manual | notify-only | auto-reconcile-safe
check_on_startup = true
check_interval_seconds = 300
max_consecutive_failures = 5
circuit_cooldown_seconds = 21600

[automation.restart]
mcp = "disabled"      # disabled | on-failure
tunnel = "disabled"   # disabled | on-failure
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

Final names/ranges are frozen in M8.0 and may be narrower. On Fleet-managed hosts these values originate in the Fleet host profile and are rendered into Studio config; Studio does not maintain a competing policy file.

## 2. Backward compatibility

Missing `[automation]` is equivalent to:

```text
updates.policy = manual
reconciliation.policy = manual
restart.mcp = disabled
restart.tunnel = disabled
```

No upgrade to 0.8 may silently start network polling or mutation.

## 3. Update modes

### manual

- no startup/periodic release checks;
- no automatic prepare/apply;
- existing manual REST/UI behavior remains.

### notify-only

- startup/periodic release check according to config;
- emit inventory/history/UI notification;
- never stage or activate.

### auto-prepare

- all notify-only behavior;
- target selection;
- verify/download/stage through existing manager;
- never stop/restart/replace runtime;
- prepared authorization is valid only in the current Studio process;
- restart causes re-evaluation/reprepare, not reuse.

### auto-update-safe

- all auto-prepare behavior;
- activation only inside maintenance window;
- all global/component safety gates green at activation time;
- safe deferral is normal and does not consume failure budget.

## 4. Version selection

Order:

1. explicit `updates.desired[component]` if present;
2. otherwise latest trusted stable release.

Rules:

- target must be newer than installed;
- no automatic prerelease selection in initial M8 policy unless exact desired version explicitly names it and ADR allows it;
- no automatic downgrade;
- no automatic same-version repair;
- provider metadata error preserves previous observed latest only as stale display evidence, never activation authority.

## 5. Periodic update checking

- one concurrent check batch maximum;
- interval lower bound to prevent provider hammering;
- bounded per-provider timeout inherited from provider;
- skipped ticks collapse;
- provider failure increments observation failure state but does not mutate runtime;
- successful check resets check circuit;
- M6 history records checks as existing update-check observations.

## 6. Prepare scheduling

A component may auto-prepare only when:

- newer selected version exists;
- no current process prepared transaction for that component/version;
- staging budget permits it;
- no component update/control mutation currently owns the coordinator;
- provider and platform are trusted;
- component is supported by automatic policy.

Dirty source does **not** block auto-prepare because runtime is unchanged, but it must be surfaced as an activation blocker.

## 7. Maintenance window

Only destructive activation is window-gated.

At activation admission:

- current UTC time must be in a configured window;
- state clock sanity must pass;
- if the window closes before first destructive phase, defer;
- once physical mutation begins, complete/rollback even if window closes;
- safety rollback/recovery is never blocked by a maintenance window.

## 7A. Policy/config freshness

Prepared automation state records the loaded Studio config/policy fingerprint.

Before destructive action, require:

- current loaded config fingerprint equals the prepared policy fingerprint or the action is fully re-evaluated under the new policy;
- reconciliation does not report a pending managed Studio config restart;
- Fleet desired render for the Studio policy surface is synchronized.

A policy change never inherits an already admitted destructive action automatically.
## 8. Reconciliation scheduling

### manual

No background drift checks beyond the existing safe startup check required for runtime state visibility.

### notify-only

Periodic `reconciliation.check()`; publish state/history; no repair.

### auto-reconcile-safe

If and only if:

```text
state == managed_safe_drift
safe_to_reconcile == true
circuit closed
no safety hold
runtime coordinator available
Gateway capability/safety snapshot valid
audit admission available
```

then call the existing one-shot apply.

Unknown/local/unmanaged/broken drift always requires operator action.

## 9. Failure versus deferral

### Failure increments retry/circuit counters

- provider/staging error;
- health verification error;
- reconciliation attempt that begins and fails/rolls back;
- invalid automation state write;
- cleanup safety validation failure;
- Gateway capability contract unexpectedly breaks after previously qualifying.

### Deferral does not increment failure counter

- maintenance window closed;
- active runtime operation;
- open unknown-outcome hold;
- dirty/conflicted source;
- update already prepared;
- component transitional;
- operator disabled policy;
- Gateway old version lacks M8 capability.

Deferrals are still auditable and bounded in log/history cardinality.

## 10. Circuit semantics

Per domain and optionally component:

```text
closed
  └─ failures >= threshold → open
open
  └─ cooldown expires → half-open
half-open
  ├─ one successful action → closed
  └─ failure → open
```

No background loop performs repeated half-open probes concurrently.

Operator reset is explicit/audited and does not execute the blocked action.

## 11. Development-host source hygiene

For auto activation of a project-owned component when its source checkout exists:

- resolve authoritative repo path from component catalog;
- require valid Git worktree;
- require clean index/worktree;
- require no unmerged entries;
- do not pull/rebase/merge automatically;
- remote divergence is informational unless release policy explicitly requires current source; local dirt/conflict is a hard blocker.

Runtime-only hosts skip source hygiene because source is intentionally absent.

## 12. Self-update

Studio auto-prepare is allowed if policy permits.

Auto activation additionally requires:

- external Fleet launcher contract qualified;
- maintenance window open;
- no runtime mutation owner;
- no safety hold;
- current process identity matches deployed versioned layout;
- pending self-update journal absent or reconcilable;
- restart intent durably recorded before shutdown.

The old scheduler never assumes activation success. The new process finalizes through existing self-update recovery and then resumes policy evaluation.
