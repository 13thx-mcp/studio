# M7 Verification Matrix — Qualified Implementation Baseline

Requirement IDs are stable handles. Current qualification evidence and exact commits are recorded in [M7-QUALIFICATION.md](M7-QUALIFICATION.md). Requirements R34/R35 were explicitly retired by ADR 0034 because workspace aliases are not part of M7; they are not reported as implementation PASS.

| ID | Requirement | Minimum permanent proof | Package |
|---|---|---|---|
| R01 | pre-dispatch cancellation never calls child | instrumented child call count = 0 | M7.1 |
| R02 | pre-dispatch deadline never calls child | queued expiry test | M7.1 |
| R03 | post-dispatch timeout does not claim rollback/failure | pending/unknown outcome test | M7.1 |
| R04 | caller disconnect during mutation does not auto-replay | disconnect + child side-effect fixture | M7.1 |
| R05 | request/correlation IDs are server observability only | spoofed ID cannot alter policy | M7.1 |
| R06 | default global active limit never exceeds 16 | saturation test | M7.1 |
| R07 | default global queue never exceeds 64 | capacity rejection test | M7.1 |
| R08 | default per-child active limit 4 is enforced | two-child concurrency fixture | M7.1 |
| R09 | per-tool class/override limit is enforced; mutation defaults single-flight | mutation serialization + override fixture | M7.1 |
| R10 | scheduler is work-conserving oldest-eligible and starvation-bounded by deadline | blocked-child ordering + queue-expiry test | M7.1 |
| R11 | drain rejects new mutations | drain admission test | M7.2 |
| R12 | drain waits for dispatched mutations | controlled completion fixture | M7.2 |
| R13 | drain timeout fails closed | update/reload remains unexecuted | M7.2 |
| R14 | Gateway update uses drain before stop | Studio integration test | M7.2 |
| R15 | reconciliation reload uses drain | reconciliation integration test | M7.2 |
| R16 | failed/changed child recovery preserves healthy sibling runtime generations | generation identity + failed-candidate test | M7.3 |
| R17 | default restart backoff is 500/1000/2000 ms and bounded by 30000 ms | fake-time sequence test | M7.3 |
| R18 | three failed automatic attempts open the circuit under defaults | crash-loop test | M7.3 |
| R19 | circuit reports retry_at, 60 s cooldown and one half-open candidate | fake-time circuit test | M7.3 |
| R20 | candidate catalog failure cannot fabricate health/routes or replace previous healthy generation | invalid tools/list fixture | M7.3 |
| R21 | exact UTF-8 file bytes expose deterministic `sha256:<hex>` revision | deterministic digest test | M7.4 |
| R22 | external edit observed before CAS commit causes `STALE_REVISION` with no overwrite | external writer fixture | M7.4 |
| R23 | same-revision concurrent server mutations serialize to one success + one stale conflict | two-writer CAS test | M7.4 |
| R24 | patch/replacement preserves old bytes on injected temp/write/sync/rename failure | failure-injection test | M7.4 |
| R25 | byte-range read is bounded and rejects invalid UTF-8 boundaries | byte-boundary tests | M7.4 |
| R26 | literal search is deterministic and bounded by matches/scanned bytes/file caps | match/bytes/order tests | M7.4 |
| R27 | path/capability confinement is preserved | symlink/traversal regression suite | M7.4 |
| R28 | oversized text response is guarded | preview/metadata test | M7.5 |
| R29 | oversized structured/binary response is guarded | JSON/base64 fixture | M7.5 |
| R30 | spill artifacts obey TTL/per-item/total limits | cleanup/disk budget tests | M7.5 |
| R31 | profiles never exceed child base allowlist | policy expansion test | M7.6 |
| R32 | profile change emits tools/list_changed | protocol integration test | M7.6 |
| R33 | control tools absent from normal coding profile | tools/list assertion | M7.6 |
| R34 | **RETIRED by ADR 0034:** workspace aliases are not part of M7 | contract audit confirms no alias surface | M7.6 |
| R35 | **RETIRED by ADR 0034:** no alias binding exists in M7 | target MCP confinement remains authoritative | M7.6 |
| R36 | resource aggregation handles collisions deterministically | two-child resource fixture | M7.6 |
| R37 | progress forwards without becoming terminal state | progress + delayed result fixture | M7.6 |
| R38 | request/outcome metrics persist without tool arguments/results | DB content assertion | M7.7 |
| R39 | M6 history failure does not replay/rollback child call | unavailable-history fixture | M7.7 |
| R40 | queue/latency/pending/unknown metrics survive restart | history restart test | M7.7 |
| R41 | child restart/circuit events enter history | resilience integration test | M7.7 |
| R42 | tunnel adapter keeps liveness/readiness/MCP discovery/poll health distinct | multi-state fixture | M7.8 |
| R43 | full client + runtime-cloudflared are exact same-release verified assets and report matching version/commit | dual-asset/version/hash test | M7.8 |
| R44 | dual-artifact tunnel upgrade preserves config/credentials, Studio daemon ownership and rollback | running/stopped update matrix | M7.8 |
| R45 | runtime-only integration works with absent source root and deployed runtime artifacts | explicit runtime-only reconciliation smoke | M7.9 |
| R46 | scheduler active/queued obligations stay bounded and leak-free under sustained admission | 4,000-admission bounded scheduler soak | M7.9 |
| R47 | M5 transactional update/rollback regressions remain green | exact M5 regression suite | M7.9 |
| R48 | M6 authority/privacy/retention regressions remain green | exact M6 regression suite | M7.9 |
| R49 | clean-source provenance is recorded | qualification manifest | M7.9 |
| R50 | release/version/publication state remain distinct | closure audit | M7.9 |
| R51 | Studio verifies selective reload without inferring sibling process identity | cancellable reload + pre/post catalog and exact summary checks | Studio M7 bridge |
| R52 | Studio accepts only trusted direct-bin launchers and restart-policy parity | launcher/root and status-policy regressions | Studio M7 bridge |

## Mandatory qualification scenarios

### Q01 — Cancellation before dispatch
Fill queue/permits, cancel a queued mutation, then release capacity. The child fixture must prove zero invocation.

### Q02 — Deadline after dispatch
Dispatch a mutation that commits before delaying its reply. Upstream deadline expires. Gateway must return pending/unknown semantics and must not invoke the mutation twice.

### Q03 — Caller disconnect
Disconnect upstream after child dispatch. Gateway must retain bounded terminal observation and expose safe retry guidance.

### Q04 — Drain
Start a long mutation plus reads, request drain, verify mutation admission closes immediately and destructive lifecycle action cannot proceed until safe terminal state.

### Q05 — Saturation/fairness
Saturate one child while another has spare capacity. Eligible unrelated requests must continue without violating per-child/global limits.

### Q06 — Crash loop
Crash one child repeatedly under fake/controlled time. Verify backoff, attempt bound, circuit-open, retry-at and sibling stability.

### Q07 — Filesystem stale write
Read revision, edit file externally, attempt CAS patch/write with old revision. Verify no overwrite.

### Q08 — Payload guard
Return oversized text, structured and base64 content. Verify bounded forwarded payload and optional spill limits.

### Q09 — Policy/profile transition
Render/migrate Gateway policy, switch operator-selected profile, and verify exact tools/list delta, list-changed notification, downgrade guard, managed-surface identity and audit record.

### Q10 — Resource/progress
Aggregate two child resources with a collision and forward a long-running progress stream. Verify bounded collision policy and non-terminal progress.

### Q11 — History privacy
Run representative calls and inspect M6 storage. Tool arguments/results must be absent by default while request/outcome metrics exist.

### Q12 — Tunnel adapter
Exercise selected full-client/native adapter on the supported darwin-arm64 artifact and distinguish liveness, readiness and control-plane poll health.

### Q13 — Runtime-only package
Run supported Studio/Gateway/Filesystem/Tunnel integration from release/runtime artifacts without Rust, Node, source repositories or local target directories.

### Q14 — Soak/fault injection
Use composite permanent evidence: sustained scheduler admission soak plus cancellation, queue-expiry, child crash/circuit, failed catalog refresh, drain, and artifact disk-pressure fault tests. Prove bounded coordinator/disk obligations without requiring one monolithic scenario.

## Closure rule

M7 cannot close with any active R01–R52 row lacking permanent proof and current qualification evidence. R34/R35 are explicitly retired by ADR 0034 rather than silently omitted. If a future contract reintroduces aliases, new active verification rows are required.
