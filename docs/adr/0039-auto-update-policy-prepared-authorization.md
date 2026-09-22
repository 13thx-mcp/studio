# ADR 0039 — M8 Update Policy Modes, Target Selection and Process-Scoped Prepared Authorization

- **Status:** Accepted for M8.3/M8.4 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-07, P-08, P-12, update modes, staging retention, source hygiene

## Context

M5 provides trusted release discovery, staging and component-specific apply/rollback. Most generic/Gateway/Fleet transaction maps are process-local while verified `ready-*` staging directories persist across Studio restart.

M8 needs automatic checking/preparation without turning stale staged bytes into durable replay authority.

## Decision

Supported update modes:

```text
manual
notify-only
auto-prepare
auto-update-safe
```

Missing M8 configuration means `manual`.

### manual

No background release check, prepare or activation.

### notify-only

Background release checks are allowed. No artifact staging or runtime mutation.

### auto-prepare

Trusted target may be downloaded, verified and staged through the existing manager. It may not stop/restart/replace runtime.

### auto-update-safe

Activation is allowed only after the M8.4 SafetyGate revalidates every global and component condition immediately before physical mutation.

## Target selection

Order:

1. explicit `updates.desired[component]`;
2. otherwise latest trusted stable release.

Automatic policy never:

- downgrades;
- performs same-version force reinstall;
- selects a prerelease unless an exact desired pin explicitly names it and the component/update ADR permits it.

Provider check cache is display evidence only. Apply authority is the prepared/staged identity revalidated at activation.

## Prepared authorization

For Generic MCP, Gateway and Fleet, automatic prepared authorization is process-scoped.

AutomationState may record:

- component;
- target version;
- transaction ID;
- staged ID/fingerprint;
- prepared timestamp;
- current Studio process instance;
- loaded policy/config fingerprint.

After Studio restart, a previous-process prepared reference is not valid for automatic apply.

The controller may:

- re-check policy;
- safely cleanup eligible orphan staging;
- re-prepare a fresh transaction.

It may not reconstruct authorization from M6 history or the presence of a `ready-*` directory.

Studio self-update and Tunnel retain their stronger existing durable transaction/recovery contracts.

## Staging retention

Default prepared TTL: six hours.

Configured TTL must remain bounded by M8.0 validation.

Automatic cleanup considers only entries:

- below the trusted staging root;
- regular/non-symlink;
- unreferenced by current-process prepared state;
- not referenced by any component recovery authority;
- structurally valid enough to classify as cleanup-eligible.

Unknown/tampered/unsafe entries are not guessed away. They block affected automatic staging and require diagnostics/operator repair.

Default aggregate ready-staging budget: 1 GiB.

Before staging, policy must prove the aggregate budget and free-space preflight.

## Development source hygiene

For a project-owned component whose authoritative source checkout exists, automatic activation requires:

- valid Git worktree;
- no unmerged entries;
- clean index/worktree;
- no source-inspection failure.

M8 never pulls, rebases, merges, resets or cleans source automatically.

Runtime-only hosts with intentionally absent source skip this gate.

Dirty source does not block notify-only or auto-prepare because active runtime is unchanged.

## Alternatives considered

### Make every prepared transaction durable

Rejected for initial M8. It would duplicate specialized recovery semantics and require a new generic durable transaction journal before safe automation is necessary.

### Automatically delete every old ready staging directory at startup

Rejected because stale/unknown material may be evidence of interruption or tamper.

### Auto-repair dirty source before update

Rejected because source control is not runtime activation authority.

## Consequences

- auto-prepare is independently safe and useful;
- restart may re-download/reprepare, trading efficiency for authority clarity;
- staging disk use becomes bounded;
- runtime-only hosts remain first-class;
- M8.4 never treats cached latest/staged presence alone as permission to mutate.
