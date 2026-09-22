# M8 Recovery and Safety Contract

## 1. Recovery hierarchy

M8 must not invent one generic recovery engine.

Order of authority:

1. component-specific durable recovery journal/scratch;
2. active runtime truth and version/health probes;
3. Studio automation policy state;
4. M6 historical evidence.

A lower layer never overrides a higher owner.

## 2. Unknown-outcome safety holds

Unsafe classes:

```text
mutation
long-running
control
```

When Gateway reaches terminal `Unknown` after dispatch:

1. persist a bounded safety hold;
2. fsync/commit the hold;
3. emit sanitized telemetry/history projection;
4. return structured non-retryable unknown result.

If hold persistence fails, Gateway must expose automation safety as unavailable; Studio auto mutation fails closed.

### Resolution

Operator resolution enum:

```text
effect-observed
no-effect-observed
abandoned-no-retry
```

Resolution:

- requires explicit same-origin operator action in Studio;
- is forwarded to the owning Gateway safety ledger;
- records audit/history;
- never invokes the original tool;
- never synthesizes original success/failure result;
- permits future **new** manual operation only after the hold closes.

## 3. Gateway restart

Gateway startup:

- validates safety-hold state before advertising M8 automation capability;
- corrupt/unsupported/unsafe hold file causes `automation_safety_available=false`;
- open holds survive restart;
- resolved holds may be retained under bounded retention or compacted after history projection.

Studio startup does not use M6 history as the live hold owner. It cross-checks history for operator visibility only.

## 4. Prepared staging recovery

Generic/Gateway/Fleet:

- ready staging is not an activation authorization after Studio restart;
- current process inventories ready staging;
- valid unreferenced staging may be retained until TTL for diagnostics or safely deleted under cleanup policy;
- auto policy re-prepares under a fresh transaction;
- tampered/unsafe staging is never activated;
- cleanup never follows symlinks/outside staging root.

Studio/Tunnel keep their existing stronger durable recovery flows.

## 5. Interrupted activation

Automation cancellation or Studio crash during physical mutation delegates to the component's existing transaction recovery.

M8 must add fault tests at every durable boundary:

- before rollback material;
- after rollback material before replacement;
- after replacement before restart;
- after restart before health verify;
- after health verify before cleanup;
- during rollback.

The scheduler must not create a second replacement attempt until recovery reaches a known safe terminal state.

## 6. Reconciliation recovery

The reconciler already snapshots changed managed surfaces and rolls back on apply failure.

M8 adds:

- persisted failure/circuit counters;
- no automatic apply while previous rollback outcome is unknown/failed;
- startup check after interrupted Studio process;
- unknown local edit never reclassified as managed-safe only because a timer fired.

## 7. Restart-loop recovery

Studio-managed MCP/Tunnel automatic restart:

- explicit stop clears/suppresses auto restart;
- update/reconciliation ownership suppresses independent restart;
- crash attempts use bounded backoff;
- stability window resets consecutive failure count only after sustained health;
- circuit-open stops restart attempts;
- operator restart may be allowed as an explicit action but is audited separately.

## 8. Stale locks

In-process mutexes disappear with process death and are not treated as durable locks.

Cross-process/file locks:

- validate owner metadata where available;
- only the owning subsystem may clear a stale lock;
- PID reuse is not sufficient proof of ownership;
- self-update launcher lock continues to use its frozen contract;
- ambiguous lock state blocks auto mutation and enters diagnostics/operator workflow.

## 9. Disk pressure

Before auto-prepare:

- enforce staging aggregate byte budget;
- require component-specific free-space preflight with bounded reserve;
- cleanup eligible expired staging first;
- never delete current/rollback/recovery authority to satisfy budget.

Disk-full during state/journal write fails before side effects where possible. After side effects begin, rollback/recovery takes precedence over audit/cleanup.

## 10. Diagnostics bundle

Proposed command/API produces a bounded archive with:

- Studio/Gateway/Fleet versions and exact runtime paths represented only as approved relative/safe metadata;
- inventory;
- automation policy state with secrets omitted;
- open safety holds (sanitized);
- reconciliation status;
- Gateway capability/status/child summary;
- transaction terminal summaries;
- hashes/names of recovery artifacts, not sensitive contents;
- recent sanitized logs under explicit byte caps;
- history health/coverage summary, not whole DB by default.

Exclude:

- credentials;
- token files;
- `CONTROL_PLANE_API_KEY`;
- raw env;
- auth headers;
- MCP arguments/results;
- arbitrary file contents;
- source repository contents.

## 11. Recovery operator priority

Safety recovery actions are not maintenance-window gated.

If automatic policy is disabled/circuit-open, the system still permits reviewed:

- safe stop;
- rollback;
- recovery finalization;
- diagnostics;
- hold resolution.

Disabling automation must never strand a transaction that already crossed the physical mutation boundary.
