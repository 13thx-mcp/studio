# M8 Verification Matrix

All rows are active unless explicitly retired by an accepted ADR. Planned tests are not PASS until implemented and executed.

| ID | Requirement | Permanent proof | Package |
|---|---|---|---|
| R01 | missing automation config produces manual/no background mutation | config default test | M8.1 |
| R02 | corrupt/future automation state blocks auto mutation | state-store fault tests | M8.1 |
| R03 | only one AutomationController mutation evaluation runs at a time | controller concurrency test | M8.1 |
| R04 | missed ticks collapse; no catch-up burst | fake-time scheduler test | M8.1 |
| R05 | UTC maintenance window boundaries are deterministic/end-exclusive | window table tests | M8.1 |
| R06 | backward clock anomaly blocks destructive automation | persisted-clock test | M8.1 |
| R07 | runtime-operation conflict becomes bounded defer, not spin | lease/backoff test | M8.1 |
| R08 | background mutation requires shared audit admission | history unavailable/admission test | M8.1 |
| R09 | Studio consumes Gateway active/queued/generation/capability status | control contract test | M8.2 |
| R10 | Gateway request telemetry records server-owned ToolClass | DTO/privacy test | M8.6 |
| R11 | unsafe Unknown persists hold before response becomes terminal | ordering/fault test | M8.6 |
| R12 | read-class Unknown does not create unsafe mutation hold | class test | M8.6 |
| R13 | open safety hold survives Gateway restart | real restart fixture | M8.6 |
| R14 | corrupt/future hold store advertises automation safety unavailable | hold-store fault test | M8.6 |
| R15 | resolving hold never invokes original child/tool | no-replay spy test | M8.6 |
| R16 | older Gateway lacking M8 capability blocks auto mutation only | compatibility matrix | M8.2/M8.4 |
| R17 | targeted child restart rejects/defers unsafe active work | active mutation fixture | M8.4 |
| R18 | targeted child activation changes target generation without sibling generation churn | sibling isolation test | M8.4 |
| R19 | post-activation child identity/generation/health matches target | version/generation integration | M8.4 |
| R20 | explicit MCP stop suppresses automatic restart | lifecycle test | M8.2 |
| R21 | Studio-managed MCP crash restart is bounded/backoff/circuit | fake-time crash-loop test | M8.2 |
| R22 | Tunnel crash restart is bounded/backoff/circuit | tunnel crash-loop test | M8.2 |
| R23 | update/reconciliation ownership suppresses competing restart | ownership integration | M8.2 |
| R24 | stability window resets consecutive restart failure only after sustained health | fake-time stability test | M8.2 |
| R25 | notify-only performs no staging/runtime write | filesystem snapshot test | M8.3 |
| R26 | auto-prepare stages verified bytes without runtime mutation | prepare integration | M8.3 |
| R27 | explicit desired target wins over latest release | selection test | M8.3 |
| R28 | auto policy never downgrades or same-version force-reinstalls | selection rejection test | M8.3 |
| R29 | prior-process generic/Gateway/Fleet staging is never auto-applied | restart after prepare test | M8.3 |
| R30 | expired eligible ready staging is cleaned under TTL policy | staging retention test | M8.3/M8.6 |
| R31 | aggregate staging budget blocks preparation before overrun | disk-budget test | M8.3 |
| R32 | dirty/conflicted managed source blocks activation | Git hygiene test | M8.4 |
| R33 | absent source on runtime-only host does not block activation | runtime-only test | M8.4 |
| R34 | closed maintenance window defers without failure increment | policy test | M8.4 |
| R35 | activation revalidates installed source version and staged identity | tamper/stale test | M8.4 |
| R36 | missing required rollback baseline blocks auto activation | rollback preflight test | M8.4 |
| R37 | generic MCP activation coordinates Gateway child and verifies new running identity | full integration | M8.4 |
| R38 | Gateway full update preserves M7 drain/no-replay/reconnect proofs | regression suite | M8.4 |
| R39 | Fleet auto activation preserves host-local profile and render validation | Fleet integration | M8.4 |
| R40 | Tunnel auto activation preserves running/stopped state and health distinctions | Tunnel matrix | M8.4 |
| R41 | Studio self-update resumes policy only after new process finalization | two-process test | M8.4 |
| R42 | notify-only reconciliation never applies drift | reconciler spy test | M8.5 |
| R43 | only ManagedSafeDrift + safe flag can auto-apply | state matrix | M8.5 |
| R44 | unmanaged/local/broken/unknown drift is never auto-written | adversarial drift tests | M8.5 |
| R45 | repeated reconciliation failure opens persisted circuit | restart/circuit test | M8.5 |
| R46 | rollback-failed reconciliation requires operator; no auto retry | failure injection | M8.5 |
| R47 | restart with overdue schedule causes one evaluation only | process restart test | M8.1/M8.5 |
| R48 | ambiguous stale cross-process lock blocks auto mutation | lock fixture | M8.6 |
| R49 | cleanup never removes component recovery-authority journal/scratch | recovery fixture | M8.6 |
| R50 | disk full fails before side effect where possible; after side effect recovery wins | ENOSPC matrix | M8.6 |
| R51 | diagnostics bundle contains no configured secrets/raw payloads | secret canary scan | M8.7 |
| R52 | unsupported automation schema never silently resets to defaults | state migration test | M8.1 |
| R53 | background and API mutation use same audited admission semantics | operation evidence test | M8.1/M8.7 |
| R54 | failure and deferral counters/reasons remain distinct | policy accounting test | M8.1 |
| R55 | circuit reset changes policy state only and executes no action | reset spy test | M8.7 |
| R56 | hold resolution is same-origin protected and typed | API security test | M8.7 |
| R57 | automation status/UI DTOs omit secrets/paths/raw env/payload | serialization test | M8.7 |
| R58 | full supported operation works source-less without build toolchain | runtime-only qualification | M8.8 |
| R59 | long soak keeps task count/state/staging/disk bounded | soak harness | M8.8 |
| R60 | M5/M6/M7 safety, rollback, history and Gateway regressions remain green | exact regression profile | M8.8 |

| R61 | Fleet host automation policy renders deterministic Studio config/runtime-only bytes | Fleet schema/render test | M8.0/M8.5 |
| R62 | invalid/future Fleet automation policy fails closed and is not silently dropped | Fleet validation test | M8.0 |
| R63 | Gateway update/rollback preserves open durable safety holds | Gateway update integration | M8.4/M8.6 |
| R64 | changed managed Studio policy/config blocks auto mutation until restarted process proves loaded identity | reconciliation/restart test | M8.4/M8.5 |

| R65 | guarded release tag accepts exact valid prerelease and rejects mismatch/invalid/bypass | Git MCP release-tag tests | M8.0/M8.8 |

## Mandatory qualification scenarios

### Q01 — manual compatibility

Upgrade an M7 config with no automation section. Observe no automatic provider request, prepare, apply, reconcile apply, restart-policy change or source mutation.

### Q02 — scheduler determinism

Fake clock + process restart covers missed ticks, startup grace, interval bounds, UTC window edges, backward wall clock and one half-open probe.

### Q03 — unknown mutation hold

Dispatch unsafe mutation, force post-dispatch unknown, prove hold durability before result, restart Gateway and Studio, prove auto mutation blocked, resolve hold, prove original tool call count remains one.

### Q04 — old Gateway compatibility

Use Studio M8 against Gateway M7 control contract. Manual status/check remains possible where compatible; automatic mutation reports explicit capability blocker.

### Q05 — generic MCP activation

Prepare Filesystem/Git/Exec fixture used by Gateway child; create concurrent safe/unsafe work; activate target; verify drain/quiesce rules, target child generation/version, sibling isolation and no request replay.

### Q06 — source hygiene

Development host dirty/unmerged source blocks auto activation. Runtime-only host with absent source is unaffected. No Git pull/rebase/reset occurs.

### Q07 — prepare/restart

Auto-prepare, terminate Studio, restart, prove old generic/Gateway/Fleet prepared transaction cannot auto-apply. Reprepare safely or clean under policy.

### Q08 — reconciliation circuit

Inject repeatable managed-safe drift plus apply failure. Verify bounded attempts, rollback, persisted circuit, restart behavior and operator reset with no immediate hidden mutation.

### Q09 — maintenance/defer

Hold update prepared across closed/open windows, active coordinator lease, safety hold and transient lifecycle state. Verify deferrals do not consume failure budget and no busy loop.

### Q10 — disk and cleanup

Fill staging budget / inject ENOSPC at state, staging, rollback and cleanup boundaries. Verify recovery authority is preserved and active runtime never replaced by unverified bytes.

### Q11 — Studio self-update

Auto-prepare Studio, enter window, external launcher activation, old scheduler exits, new process finalizes transaction and resumes one controller without duplicate activation.

### Q12 — runtime-only

Run released Studio/Gateway/Fleet/Tunnel/generic MCP artifacts with source/build roots absent. Exercise notify, prepare, one safe activation, one safe reconciliation and diagnostics.

### Q13 — diagnostics privacy

Seed canary credentials, env, MCP argument/result payloads and private source content. Export diagnostics and byte-scan archive for canaries/unsafe absolute paths.

### Q14 — long soak

Sustained update/reconciliation/health scheduling with intermittent provider outage, busy runtime, child crashes and drift. Assert bounded tasks, queueing, staging, state file size, disk and event rate.

### Q15 — regression closure

Run M5 publication/update rollback regressions, M6 audit/history authority regressions and M7 no-replay/drain/concurrency/CAS/cancellation/runtime-only qualification against exact clean source.
