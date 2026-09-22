# M7 Source Audit

**Audit date:** 2026-09-22
**Purpose:** record current implementation facts that constrain M7 design. This is planning evidence, not qualification evidence.

## 1. Baselines

- Studio: `48906d892fb37275f8b2d10b73b7cad1eec355c9`, clean `main`.
- Gateway: `02a46af100c0cee5e011b8bc74079d7a78dd3658`, clean `main`.
- Filesystem: `0f09a665bb94af8388e01051d87cb3f865d4310d`, clean `main`.
- Fleet worktree: `feature/sonarqube-main-gate`; M7 work must not be layered onto that unrelated branch.
- Studio M6 is VERIFIED/CLOSED. SQLite is historical evidence only.
- Local tunnel runtime contains `tunnel-client-runtime-cloudflared` v0.0.14 artifacts.

## 2. Gateway current shape

Gateway is currently a single `src/main.rs` implementation. Its central `Snapshot` owns tools, routes, child runtimes and child statuses behind one `RwLock`.

Current request path:

```text
upstream call_tool
  -> resolve route from snapshot
  -> clone child Peer
  -> tokio::time::timeout(child_timeout, peer.call_tool(...))
  -> return child result OR generic child error/timeout
```

Consequences relevant to M7:

- there is no request ID/correlation registry owned by Gateway;
- there is no explicit pre-dispatch queue state;
- there is no distinction between timeout before dispatch and timeout after dispatch;
- after dispatch, a timeout result cannot prove whether a mutation happened;
- no pending/unknown outcome is retained for later classification;
- cancellation/disconnect semantics are not represented in Gateway state;
- request arguments/results are not intentionally persisted, which is compatible with the desired M7 privacy boundary.

## 3. Reload and child recovery behavior

Current `reload()`:

1. loads every child config;
2. starts every enabled child and rebuilds the full tool catalog;
3. atomically replaces the whole snapshot;
4. cancels the old snapshot.

The supervisor detects any dead restartable child and invokes the same full reload path.

M7 implications:

- child recovery can unnecessarily replace healthy siblings;
- reload has no drain handshake with active requests;
- active work can be interrupted by snapshot cancellation;
- manual reload/update/reconciliation currently lacks request-aware safety;
- M7 needs per-child generation/health ownership instead of treating whole-snapshot replacement as the normal recovery primitive.

## 4. Restart policy gap

Current schema supports only:

```yaml
restart:
  policy: never | on-failure
```

There are no max attempts, backoff bounds, consecutive failure counters, crash-loop classification, circuit-open state or retry timestamp.

## 5. Gateway test/tooling baseline

Rust quality at audit baseline:

- `cargo fmt --all -- --check`: PASS
- `cargo check --all-targets --all-features`: PASS
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `cargo test --all-targets --all-features`: PASS, 6 tests

The aggregate workspace quality helper reports FAIL because this working copy also contains local Node markers (`package.json`, `pnpm-lock.yaml`, `node_modules/`). They are not tracked, have no repository history, and are explicitly excluded by `.git/info/exclude`. The placeholder test exits 1, so auto-detection misclassifies this Rust-only repository. M7 qualification therefore uses explicit Rust gates; it does not mutate or publish these local excluded files.

The original six Gateway tests cover config/name/prefix/allowlist behavior. M7.0 adds seven permanent protocol-spike tests under `tests/m7_protocol_spike.rs`; after that addition the full Rust suite is 13/13 PASS. Concurrency, drain, restart isolation and production forwarding remain later M7 requirements.

## 6. Filesystem current shape

Filesystem currently exposes:

- `list_directory`
- `read_text_file`
- `write_text_file`
- `create_directory`

Safety already present:

- capability confinement through `cap-std`;
- rejection of absolute paths and `..`;
- bounded file size and directory entries;
- read-only mode;
- sibling-temp + sync + atomic rename for overwrite.

Missing M7 requirements:

- range reads;
- bounded text search;
- file metadata;
- content revision identity;
- expected-revision CAS;
- patch application;
- explicit stale-write conflict;
- concurrent mutation conflict tests.

Rust quality at audit baseline passes; current unit coverage is only three path-normalization tests.

## 7. Studio Gateway update integration

The current `GatewayUpdateManager::apply`:

- acquires the shared runtime-operation control lease;
- validates staged/current identity;
- if Gateway owner is running, calls owner `stop()`;
- replaces/verifies the binary/catalog;
- restarts and verifies owner state.

It does **not** first ask Gateway to drain active request state because no drain contract exists yet.

M7 must insert a drain prerequisite before destructive Gateway stop/update and must keep M5 rollback/catalog guarantees intact.

## 8. Fleet config/rendering gap

Current Fleet host schema v1 renders per-child:

- enabled;
- command/args/env;
- `tool_allowlist`;
- `timeout_ms`;
- `restart.policy = on-failure`.

It has no global Gateway policy surface for queue limits, active profile, payload budgets or artifact policy, and no per-tool class/concurrency metadata.

Adding a new policy file changes the M5 reconciliation surface set and therefore requires coordinated Studio/Fleet design, migration and rollback tests.

## 9. Tunnel upstream mismatch

The installed `tunnel-client-runtime-cloudflared` binary is intentionally run-only: its CLI exposes `run` and does not expose `doctor` or `runtimes`.

The official tunnel-client v0.0.14 release also publishes the full client for onboarding/administration/profile/runtime management. Current upstream documentation describes native `runtimes connect`, `runtimes status --json`, `runtimes stop`, `/healthz`, `/readyz`, and separate `control_plane_poll_health`.

ADR 0032 resolves the architecture: install the checksum-verified full `tunnel-client` alongside the co-versioned `tunnel-client-runtime-cloudflared`; keep runtime-cloudflared as the Studio-owned production daemon and use the full client only for bounded diagnostics/preflight. Do not invoke `doctor/runtimes` against the run-only runtime binary, and do not silently migrate production ownership to `runtimes connect`.

Upstream references reviewed and full-client binary re-probed on 2026-09-22:

- https://github.com/openai/tunnel-client/releases/tag/v0.0.14
- https://github.com/openai/tunnel-client/blob/master/docs/onboarding.md
- https://github.com/openai/tunnel-client/blob/master/docs/end-user-guide.md

## 10. Protocol/SDK freeze prerequisite — RESOLVED

Gateway locks `rmcp 3.4.0`. The reproducible M7.0 spike is recorded in [RMCP-PROTOCOL-SPIKE.md](RMCP-PROTOCOL-SPIKE.md) and implemented as permanent Gateway integration tests in `tests/m7_protocol_spike.rs`.

Verified facts:

- `RequestContext` exposes request ID, cancellation token and negotiated protocol;
- an external `tokio::time::timeout(peer.call_tool(...))` drops the waiter **without cancelling the dispatched child request**;
- explicit `RequestHandle::cancel` and rmcp-owned request timeout propagate `notifications/cancelled` to the child's `RequestContext::ct`;
- cancellation is cooperative and cannot prove a side effect was undone;
- rmcp generates a child progress token and delivers matching progress to a real `ClientHandler`;
- `resources/list` and `resources/read` work with the locked SDK;
- default `ServiceExt::serve` negotiates `2025-11-25`;
- `2026-07-28` requires explicit Discover-lifecycle opt-in and is not silently enabled by the current Gateway child startup.

The first M7 request-coordination implementation therefore preserves the current default child protocol lifecycle. A 2026-07-28 migration requires separate compatibility qualification.

## 11. Blocking prerequisites

| ID | Finding | Blocking action |
|---|---|---|
| P-01 | post-dispatch timeout has no outcome tracking | **RESOLVED (contract):** ADR 0026 + rmcp 3.4.0 spike freeze `OUTCOME_PENDING -> terminal | OUTCOME_UNKNOWN`; no mutation replay |
| P-02 | reload/recovery replaces whole snapshot | **RESOLVED (contract):** ADR 0029 freezes per-child generation + prepare/drain/promote; healthy siblings remain unchanged |
| P-03 | restart can loop without bounded backoff/circuit state | **RESOLVED (contract):** ADR 0029 freezes 3-attempt bounded exponential backoff, stability window, circuit-open/cooldown/half-open |
| P-04 | local excluded Node markers make aggregate auto-detection false-fail | **RESOLVED:** use explicit Rust qualification gates; do not publish/mutate local excluded files |
| P-05 | Filesystem lacks revision/CAS/patch/range/search | **RESOLVED (contract):** ADR 0030 freezes SHA-256 revision, typed errors, bounded range/search, required expected-revision CAS and byte-range patch |
| P-06 | Studio Gateway update stops without drain | **RESOLVED (design/viability):** ADR 0027 private UDS handshake; Gateway `m7_control_socket_viability` 3/3 PASS; production wiring remains M7.2 |
| P-07 | Fleet has no Gateway global policy/tool-class schema | **RESOLVED (contract):** ADR 0031 freezes `gateway.yaml`, Fleet host schema v2, `gateway.policy` managed surface and downgrade guard |
| P-08 | installed tunnel runtime is run-only | **RESOLVED (contract + binary proof):** ADR 0032 freezes co-versioned full-client + runtime-cloudflared release unit while Studio retains daemon ownership |
| P-09 | rmcp protocol details not source-frozen | **RESOLVED:** locked rmcp 3.4.0 spike + 7 permanent tests; see RMCP-PROTOCOL-SPIKE.md |

All P-01 through P-09 must be dispositioned in M7.0 before broad production implementation.
