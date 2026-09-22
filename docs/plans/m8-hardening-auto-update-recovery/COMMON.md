# M8 Common Contracts and Invariants

These invariants are normative for every M8 package.

## Authority

**M8-I01 — one owner per live concern.** Gateway owns live MCP request state; component managers own update transactions; RuntimeReconciler owns one-shot managed config repair; AutomationController owns scheduling/policy only.

**M8-I02 — SQLite is history, never a lock/queue/scheduler.** M6 may veto unattended action when audit/admission is unavailable, but historical rows never authorize runtime mutation.

**M8-I03 — durable safety veto is allowed, durable replay queue is not.** Unknown side-effect holds may survive restart; arbitrary tool requests/arguments may not.

**M8-I04 — one restart authority per process.** Gateway child backoff remains Gateway-owned. Studio must not add a competing child restart loop.

## Automation admission

**M8-I05 — manual is default.** Existing configurations gain no background mutation by upgrade.

**M8-I06 — every automatic mutation revalidates immediately before side effects.** A prior check/prepare result is insufficient.

**M8-I07 — unknown is unsafe.** Missing/stale/unavailable evidence blocks automatic mutation.

**M8-I08 — conflict means defer, not spin.** Runtime-operation lease conflicts use bounded backoff and circuit accounting.

**M8-I09 — no catch-up bursts.** Missed periodic ticks collapse to at most one due evaluation after restart/wake.

**M8-I10 — clock anomalies fail safe.** Persisted wall-clock regressions outside tolerance block maintenance-window mutation until state is sane/operator-reviewed.

## Updates

**M8-I11 — no automatic downgrade or force reinstall.** Tunnel same-version repair remains an explicit manual path.

**M8-I12 — desired exact version wins over latest.** Without a desired pin, M8 initial automatic selection uses trusted stable releases only.

**M8-I13 — prepare is not activation.** `auto-prepare` may download/verify/stage but must not stop/restart/replace active runtime.

**M8-I14 — process-scoped staged authorization.** Generic/Gateway/Fleet staged state prepared by another Studio process is never auto-applied; it is re-prepared or cleaned under the current process.

**M8-I15 — development-source conflict blocks activation.** Dirty/conflicted/uninspectable managed source blocks auto activation but not notify-only.

**M8-I16 — rollback material is mandatory when the component contract requires it.** Lack/ambiguity blocks activation.

## Gateway and unknown outcomes

**M8-I17 — historical request class is server-owned.** Unknown outcome safety uses the class recorded when Gateway admitted the request.

**M8-I18 — unsafe unknown outcome creates a durable hold.** Classes `mutation`, `long-running` and `control` block unattended mutations until explicit resolution.

**M8-I19 — resolving a hold never replays a request.** Resolution only changes the safety veto state and records audit evidence.

**M8-I20 — old Gateway capability blocks auto mutation.** New Studio may operate manually with an older Gateway, but `auto-update-safe`/auto-reconcile must fail closed when required safety capabilities are absent.

## Reconciliation

**M8-I21 — only managed-safe drift may auto-repair.** Unknown/unmanaged/local/secret-ambiguous/broken drift is notify/operator-only.

**M8-I22 — repeated repair failure opens a circuit.** Circuit state survives Studio restart and requires cooldown or explicit operator reset according to policy.

## Recovery and diagnostics

**M8-I23 — recovery authority files are never deleted by generic cleanup.** Component journals/scratch that may authorize rollback are parsed/validated by their owner only.

**M8-I24 — diagnostics are bounded and sanitized by construction.** No credentials, raw environment, authorization headers, source file contents, MCP arguments/results or unrestricted filesystem dumps.

## Policy state versus evidence

Live M8 policy state belongs under a private Studio automation state root:

```text
runtime/studio/data/automation/
├── state.json
└── diagnostics/
```

The state file is schema-versioned, bounded, atomically replaced, fsynced, symlink-rejected and contains only:

- scheduler due/backoff/circuit metadata;
- last attempted policy action identifiers;
- prepared-action references that are safe to discard/reprepare;
- operator resolution references;
- no secrets or raw payloads.

Gateway unknown-outcome holds are Gateway-owned under its private runtime state and are surfaced through the private control contract.

M6 history receives projections of both domains after the live owners commit them.

**M8-I25 — Fleet is desired-state authority for Fleet-managed automation config.** Studio must not invent a second persisted policy source beside the rendered host profile.

**M8-I26 — stale loaded policy blocks destructive automation.** If reconciliation says the managed Studio config changed and requires restart, automatic activation/reconciliation mutation is disabled until a new process proves the loaded config identity.

**M8-I27 — Gateway safety state survives Gateway binary update.** A Gateway update must preserve and validate the private safety-hold state; rollback cannot discard open holds.
## Maintenance windows

M8 initial window semantics:

- explicit UTC weekdays;
- `HH:MM` UTC start;
- bounded duration 1–1440 minutes;
- end is exclusive;
- an action admitted inside the window may finish outside it;
- a new destructive phase may not begin after the window closes;
- no DST/IANA inference in M8.

## Retry/backoff baseline

Unless an ADR/package chooses a stricter component rule:

```text
initial: 30 s
multiplier: 2
max: 30 min
circuit after: 5 consecutive policy failures
circuit cooldown: 6 h
```

Policy deferral caused by maintenance window, active operation, safety hold or dirty source is **not** counted as a component failure. It is recorded separately.

## Component target versions

Frozen independent M8 targets from M8.0:

```text
Studio      0.8.0-beta
Gateway     0.3.0      # durable safety holds/control capability/child activation
Fleet       0.5.0      # host schema v3 + automation policy rendering
Git MCP     0.2.0      # guarded SemVer prerelease release-tag support
Filesystem  0.2.x      # no planned M8 capability change
Exec        0.1.x      # no planned M8 capability change
Tunnel      upstream release re-confirmed at implementation/qualification time
```

Versions are not lockstep. A version target does not imply publication/deployment.
## Compatibility rule

New policy/config fields are additive and default to manual. A new Studio talking to an older Gateway may expose status/read-only operation, but must not claim unattended mutation safety without advertised M8 capabilities.
