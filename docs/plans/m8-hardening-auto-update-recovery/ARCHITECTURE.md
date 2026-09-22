# M8 Architecture

## 1. Control-plane shape

```text
                         trusted release providers
                                  │
                                  ▼
┌────────────────────────── MCP Studio ───────────────────────────┐
│                                                                │
│  AutomationController                                          │
│  ├─ ScheduleEngine                                             │
│  ├─ PolicyEvaluator                                            │
│  ├─ SafetyGate                                                 │
│  └─ AutomationStateStore                                       │
│          │                                                     │
│          ├──────── observations/checks ────────┐               │
│          │                                     │               │
│          ▼                                     ▼               │
│  InventoryService                       RuntimeReconciler       │
│          │                                     │               │
│          └──────────────┬──────────────────────┘               │
│                         ▼                                      │
│               RuntimeOperationCoordinator                      │
│                         │                                      │
│        ┌────────────────┼────────────────────┐                 │
│        ▼                ▼                    ▼                 │
│  Update Managers   Tunnel/Studio owners   GatewayControlClient │
│        │                                     │                 │
└────────┼─────────────────────────────────────┼─────────────────┘
         │                                     │ private socket
         ▼                                     ▼
 verified runtime                     rust-mcp-gateway
                                      ├─ live request registry
                                      ├─ drain/concurrency
                                      ├─ child recovery
                                      ├─ durable safety holds
                                      └─ targeted child control
```

## 2. AutomationController

One Studio process owns exactly one controller.

Responsibilities:

- compute due observations/actions;
- apply policy modes;
- maintain bounded retry/backoff/circuit state;
- request shared audit admission;
- request live safety snapshots;
- call existing domain services directly;
- never execute arbitrary command strings;
- never bypass RuntimeOperationCoordinator;
- never synthesize component success from history.

It does **not** own:

- Gateway request execution;
- release verification;
- component binary replacement;
- reconciliation rendering;
- rollback;
- Tunnel/Studio recovery journals.

## 3. ScheduleEngine

Use Tokio timers only for in-process waiting. Persist only bounded wall-clock schedule metadata required to resume conservatively.

Rules:

- startup grace before first network/policy work;
- `MissedTickBehavior::Skip` equivalent semantics;
- one outstanding evaluation per schedule class;
- no unbounded task spawn;
- no durable request queue;
- after restart, overdue work becomes one evaluation, not N missed jobs.

Schedule classes:

```text
health
update_check
reconciliation_check
cleanup
```

Mutating activation is event/policy driven after a successful check/prepare and maintenance-window admission, not a separate unbounded timer.

## 4. PolicyEvaluator

Inputs:

- config policy;
- installed/desired/latest component state;
- prepared transaction owned by current process;
- health snapshot + freshness;
- RuntimeOperationCoordinator snapshot;
- Gateway safety status/capabilities;
- source-present Git hygiene;
- maintenance window;
- rollback readiness;
- reconciliation state;
- automation circuit/hold state.

Output is typed:

```text
ObserveOnly
Prepare(component, version)
Defer(reason, retry_not_before)
Activate(component, transaction_id)
Reconcile
RequireOperator(reason)
Noop(reason)
```

No output directly contains shell commands.

## 5. SafetyGate

Every automatic mutation passes one final synchronous logical gate immediately before calling a mutating owner.

Minimum global checks:

1. policy still permits action;
2. current process owns prepared authorization and its policy/config fingerprint still matches;
3. loaded managed Studio config is active (no pending policy/config restart);
4. no active automation hazard hold;
5. Gateway required capabilities are available/fresh;
6. RuntimeOperationCoordinator is not busy;
7. maintenance window is open when required;
8. source hygiene is safe for source-present managed component;
9. rollback baseline/material is available;
10. installed version still equals prepare source version;
11. staged identity still matches;
12. component health is stable enough for its contract;
13. audit admission is available;
14. no conflicting self-update/reconciliation recovery is pending.

Component-specific gates are additional, never replacements for global gates.

## 6. Gateway M8 control extension

Additive response fields should include a capability list and safety data while preserving older clients.

Proposed response additions:

```json
{
  "capabilities": [
    "m8_safety_holds",
    "m8_targeted_child_restart",
    "m8_child_status"
  ],
  "active_requests": 0,
  "queued_requests": 0,
  "safety_hold_count": 0,
  "children": []
}
```

Old Studio ignores new response fields. New Studio treats missing required capability as automatic-mutation unavailable.

New actions are called only after capability advertisement.

### Durable safety hold

Gateway creates a hold for terminal `Unknown` on classes:

- mutation;
- long-running;
- control.

Hold payload is bounded and sanitized:

```text
hold_id
gateway_instance_id
request_id
child
tool
class
observed_at_ms
catalog_generation
profile_generation
state=open|resolved
resolution?  # enum only
resolved_at_ms?
```

No args/results/payload/path/env.

### Targeted child activation

The M8 contract must support an operator/control action that:

1. rejects when target child has unsafe active work;
2. drains target/global mutations according to M7 policy;
3. replaces/restarts only the intended child generation where possible;
4. verifies candidate initialization/catalog;
5. preserves healthy siblings;
6. exposes post-restart generation/health/identity evidence;
7. never replays interrupted work.

Exact wire shape is frozen in ADR before coding.

## 7. Component activation topology

| Component | Activation owner | M8 automatic activation requirement |
|---|---|---|
| Filesystem/Git/Exec/Blender | Studio update manager + possible Gateway child | coordinate Studio-owned instance and Gateway child; verify new binary identity and child generation |
| Gateway | GatewayUpdateManager | full Gateway drain/update/reconnect/catalog verification |
| Fleet | FleetUpdateManager | validate bundle/profile/render contract; no blind host-local overwrite |
| Tunnel | TunnelUpdateManager | preserve daemon state/config/credentials and native health contract |
| Studio | SelfUpdateManager + Fleet launcher | external activation; new process reconciles intent |
| unmanaged/external child | external owner | never auto-update unless an explicit trusted adapter is added |

## 8. Health model

```text
HealthSnapshot {
  component,
  observed_at,
  freshness,
  state: healthy|degraded|unhealthy|unknown,
  liveness,
  readiness,
  identity,
  ownership,
  required_checks[],
  failed_checks[]
}
```

Examples:

- Gateway: control socket + expected state + catalog + child summaries;
- generic MCP: installed binary version + direct owner state + Gateway child generation/initialization where applicable;
- Tunnel: liveness/readiness/MCP discovery/poll health kept distinct;
- Studio: process + /health + version + loaded config identity;
- Fleet: schema/render validation, not a daemon ping.

Automation requires fresh evidence under package-defined age limits.

## 9. Restart policy

Gateway child restart remains Gateway-owned.

Studio Supervisor/Tunnel may gain:

```text
disabled
on-failure
```

with:

- max attempts;
- stability window;
- bounded exponential backoff;
- circuit-open cooldown;
- no restart after explicit stop;
- no restart during update/reconciliation ownership;
- no competing scheduler task per crash.

State needed only for policy/restart safety may be persisted if restart survival is required; process PID ownership remains in-memory and re-probed on startup.

## 10. Persistence boundaries

### Studio automation state

`runtime/studio/data/automation/state.json`

- schema v1;
- owner-only permissions;
- no symlink;
- atomic temp + fsync + rename + parent fsync;
- size cap;
- fail closed for automatic mutations when corrupt/unsupported;
- manual status/diagnostics still available.

### Gateway safety holds

Private Gateway runtime state, with equivalent confinement/atomicity requirements.

### M6 history

Projection only. History failure after a physical side effect never rewinds the side effect. Admission failure before a discretionary automatic mutation blocks it according to M6 policy.

## 11. Startup order

Proposed M8 startup sequence:

```text
load + validate config
open history
recover component-owned journals/scratch
load automation state
construct runtime owners
finalize Studio/Tunnel recovery
construct reconciler
perform startup reconciliation CHECK only
query Gateway safety/capabilities
reconcile open safety holds/history projections
bind API
mark ready
start AutomationController after startup grace
```

No automatic mutation occurs before readiness and startup recovery complete.

## 11A. Fleet policy authority

For Fleet-managed hosts, the host profile is the desired-state source for M8 automation policy. Fleet schema/rendering must:

- validate automation enums/ranges/window values;
- render deterministic Studio `[automation]` sections;
- preserve runtime-only behavior;
- reject unsupported future policy schema;
- make policy changes visible as managed Studio config drift.

When reconciliation installs changed Studio policy/config, current Studio sets/observes `studio_restart_required`. AutomationController enters observation-only mode until the restarted process proves the loaded config fingerprint.

Gateway safety-hold files are runtime state, not Fleet-rendered config, and must be preserved across Fleet/Gateway updates.
## 12. Shutdown

AutomationController receives cancellation before component shutdown.

It must:

- stop admitting new background work;
- allow currently owned safe observation to end;
- not begin a new mutation;
- persist scheduler/circuit state;
- leave component recovery authority to component owners;
- avoid converting shutdown cancellation into a retryable mutation request.
