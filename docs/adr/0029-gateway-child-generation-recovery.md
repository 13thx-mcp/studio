# ADR 0029 — Per-Child Runtime Generation, Recovery and Circuit Ownership

- **Status:** Accepted for M7.3 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-02/P-03 child reload, restart, generation identity, backoff and circuit breaking

## Context

The committed Gateway baseline rebuilds a complete child snapshot on reload and cancels the previous snapshot after replacement. The watcher also uses the same reload path when any restartable child is observed dead. That couples unrelated children and makes a single-child failure/reconfiguration capable of replacing healthy siblings.

The current restart policy is only `never | on-failure`; there is no generation identity, bounded retry episode, backoff, stability window, circuit-open state or retry-at timestamp.

M7 request/outcome semantics make broad restart more dangerous: a dispatched mutation may have completed even if its response is lost. Child lifecycle logic therefore cannot use restart as an implicit retry or discard active outcome-observation obligations.

## Decision

Gateway owns an independent runtime record for every configured child.

### Child identity

Each child has:

```text
child_name
config_fingerprint
runtime_generation
runtime_state
catalog_identity
restart_episode
consecutive_failures
retry_at
last_started_at
last_healthy_at
```

`runtime_generation` is a Gateway-owned monotonically increasing integer scoped to one child name and one Gateway process lifetime. It increments only when a newly initialized child incarnation is successfully promoted into the routable state.

Generation identity is observability/correlation only. It is never an idempotency key and never authorizes replay.

### Runtime states

```text
STOPPED
STARTING
HEALTHY
DRAINING
FAILED
BACKING_OFF
RESTARTING
CIRCUIT_OPEN
HALF_OPEN
DISABLED
```

Only `HEALTHY` generations receive new normal tool dispatch.

### Reload diff

A reload first validates the complete candidate child configuration and Gateway policy without mutating live routing.

Every child is classified:

```text
UNCHANGED_HEALTHY
CHANGED
REMOVED
ADDED
FAILED/DEAD
DISABLED
```

Rules:

- unchanged + healthy => preserve the exact process and generation;
- added => start and validate one new child, then publish its routes;
- changed => child-scoped drain, start/validate replacement, atomically switch that child's routes, then retire the old generation;
- removed/disabled => child-scoped drain, remove routes, then retire the old generation;
- failed/dead => classify outstanding dispatched calls first, then enter the recovery state machine;
- failure to prepare one changed/added child must not replace healthy siblings.

Catalog generation may change when one child changes; sibling runtime generations do not.

### Changed-child activation

For a changed child:

1. close new admission to that child;
2. wait for already-dispatched mutations according to ADR 0026/0027 semantics;
3. start the candidate child as a non-routable candidate;
4. complete MCP initialize + bounded `tools/list` and policy validation;
5. atomically publish routes/status for the new generation;
6. retire the old generation;
7. reopen admission.

If candidate startup/catalog validation fails, the old healthy generation remains authoritative and may be resumed. No sibling is restarted.

A removed child has no replacement candidate; drain must complete before its live generation is retired.

### Failure and outcome boundary

Transport closure does not automatically mean every in-flight request failed.

Before replacing a dead generation, Gateway classifies each dispatched request using ADR 0026:

- trustworthy terminal result => terminal outcome;
- terminal evidence unavailable after transport loss => `OUTCOME_UNKNOWN`;
- mutation with unknown outcome is never replayed on the new generation.

The new generation starts only future requests.

## Automatic recovery policy

Initial M7 defaults:

```yaml
restart:
  policy: on-failure
  max_attempts: 3
  stability_window_ms: 30000
  backoff:
    initial_ms: 500
    max_ms: 30000
  circuit:
    cooldown_ms: 60000
    half_open_attempts: 1
```

No jitter is used in the first implementation so fake-time tests are deterministic.

For automatic attempt number `n` starting at 1:

```text
delay(n) = min(initial_ms * 2^(n-1), max_ms)
```

The default sequence for one failed recovery episode is therefore 500 ms, 1000 ms, 2000 ms.

After `max_attempts` failed automatic attempts, the child enters `CIRCUIT_OPEN` and exposes a server-owned `retry_at`.

When cooldown expires, exactly one half-open candidate start is allowed. Failure reopens the circuit for another cooldown. Success enters `HEALTHY` provisionally; the failure counter resets only after the generation remains healthy for `stability_window_ms`. A quick crash before that window continues the existing failure episode.

This prevents a child that repeatedly starts and immediately crashes from resetting its retry budget forever.

### Manual recovery

A privileged manual restart/reload can request recovery while the circuit is open, but it cannot bypass request safety:

- active dispatched mutation obligations require child drain/outcome classification;
- manual recovery never replays a request;
- only one recovery candidate for a child may exist at a time;
- manual failure contributes explicit evidence but does not create an unbounded retry loop.

## Catalog publication

A candidate generation is not routable until its tool catalog is valid.

Publication of one child's replacement is atomic with respect to:

- child generation;
- child routes;
- child status;
- catalog generation/fingerprint.

If publication fails, the candidate is retired and the previous healthy generation remains routable when still valid.

## Shutdown

Gateway shutdown may cancel children after the Gateway-wide drain/shutdown contract has classified active work. Shutdown does not convert unknown mutation outcomes into failures.

## Verification requirements

Permanent M7.3 tests must prove:

- unchanged healthy sibling generation is preserved across another child's reload;
- changed child receives a new generation only after successful candidate validation;
- failed candidate leaves previous healthy generation/routes intact;
- dead-child restart does not replay an in-flight mutation;
- backoff sequence is bounded and deterministic under fake time;
- three failed automatic attempts open the circuit under defaults;
- circuit exposes `retry_at` and permits one half-open attempt;
- quick crash before 30 s stability does not reset the failure episode;
- healthy-for-stability-window resets the episode;
- catalog validation failure cannot fabricate healthy/routable state;
- no more than one recovery candidate exists per child.

These map to R16-R20 and Q06.

## Consequences

- Whole-snapshot restart is no longer an acceptable recovery primitive.
- The scheduler/registry must bind dispatched work to the child generation it actually used.
- Reload becomes a prepare/drain/promote operation per affected child.
- Healthy sibling processes survive unrelated config and crash recovery.
- Circuit state is live in-memory authority; M6 may record history but cannot reopen/close the circuit.

## Rejected alternatives

### Restart the entire snapshot on any child change

Rejected because it unnecessarily interrupts healthy siblings and magnifies ambiguous mutation outcomes.

### Immediate unbounded restart loop

Rejected because persistent child failure can consume CPU/process resources indefinitely.

### Reset failure count on successful initialize only

Rejected because a crash-looping child could repeatedly initialize and immediately die without ever tripping the circuit.

### Persist circuit state as runtime authority in SQLite

Rejected because live restart authority belongs to the Gateway process, not historical storage.
