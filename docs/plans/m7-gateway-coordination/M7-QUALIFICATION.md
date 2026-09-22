# M7 Qualification and Closure Audit

**Audit date:** 2026-09-22
**Status:** IMPLEMENTATION QUALIFIED / CLOSURE FIXES INTEGRATED ON MAIN / RELEASE PUBLICATION PENDING
**Publication:** local release tags exist but are not published to `origin`; deployment is not claimed.

## Qualified implementation commits

| Component | Qualified implementation | Version | Publication state |
|---|---|---:|---|
| Studio | `1c5dac50849134364c2fec6e70225b9027265812` + closure fix `543a6c4` | 0.7.0-beta | unpublished |
| Gateway | `5b886c95509b63d6d31c1ba903825b8a385027ff` + qualification test `da432d7` | 0.2.0 | unpublished |
| Filesystem | `ec68a043d9c67763c83d32e3a0b4f0a9f8e8c73f` | 0.2.0 | unpublished |
| Fleet | `7bad5e2d4623e9867010572679dfc597a7dc4680` | 0.3.0 | unpublished |

The additional Studio/Gateway closure commits are descendants of the listed M7 merge commits. They do not change the frozen protocol/policy contracts; they close qualification gaps discovered by this audit.

Final integration is now complete: Studio `main` is `3423d8e` and contains `543a6c4`; Gateway `main` is `36aea90` and contains `da432d7`.

## Clean-source gates

### Gateway

Detached clean worktree at merge `5b886c9`:

- `cargo fmt --check` — PASS
- `cargo check --all-targets --all-features` — PASS
- `cargo clippy --all-targets --all-features -- -D warnings` — PASS
- `cargo test --all-targets --all-features` — PASS, 51 tests

Permanent coverage includes scheduler capacity/fairness/deadline/cancellation, drain, post-dispatch unknown outcome/no replay, child generation/recovery/circuit, payload/artifact bounds, profile routing, resource identity, progress mapping, telemetry sanitation and control-socket lifecycle.

Qualification branch `da432d7` adds a permanent bounded scheduler soak: 100 rounds × 40 concurrent admissions (4,000 total) with global active bound 8, queue bound 32, and zero active/queued leaks after every round. Focused soak and the full Gateway suite pass; the full suite becomes 52 tests.

### Filesystem

Clean `main` at `ec68a04`:

- fmt/check/Clippy — PASS
- full tests — PASS, 10/10

Coverage includes deterministic SHA-256 revision identity, stale external edit detection, same-revision writer serialization/recheck, bounded range/search/patch behavior, symlink-component confinement, and injected replacement-failure cleanup.

### Fleet

Detached clean worktree at merge `7bad5e2`:

- `python3 -m unittest discover -s tests -p 'test_*.py' -v` — PASS, 19/19

Coverage includes host schema v2, deterministic `gateway.policy` rendering, invalid active-profile rejection, and exact dual tunnel asset selection.

### Studio

Detached clean worktree at merge `1c5dac5`:

- fast quality profile — PASS, 6/6 checks
- Rust fmt/check/Clippy/tests — PASS
- Studio Rust closure suite — 261 passed / 11 explicitly ignored in the main library suite, plus all integration test binaries PASS
- web Vitest — 31/31 PASS

Coverage includes sanitized Gateway history ingestion, history-unavailable behavior, Gateway drain/update verification, managed Gateway policy reconciliation, dual tunnel asset staging, full-client diagnostics, and distinct liveness/readiness/MCP/poll health.

## Runtime-only closure finding and fix

The explicit runtime-only reconciliation smoke was run against the clean Studio merge `1c5dac5` and initially **FAILED**:

```text
Fleet render-plan output set is incomplete or excessive
```

Root cause: Studio hard-coded the schema-v2 `gateway.policy` surface into reconciliation validation, while a supported schema-v1 runtime-only Fleet profile legitimately renders no global policy surface.

Closure fix `543a6c4` derives the expected managed surfaces from the trusted active Fleet host:

- exact active server names determine `gateway.<server>` surfaces;
- `gateway.policy` is required only when the host declares Gateway policy;
- server names are validated before constructing managed paths;
- managed-surface path validation remains fail-closed.

Permanent tests for dynamic active-host surfaces and path-escape rejection PASS. The explicit runtime-only smoke then PASSes using copied runtime artifacts, a deliberately absent source root, real Gateway/Filesystem/Git/Exec binaries, and preserved tunnel runtime.

This is the R45/Q13 closure evidence. The failed pre-fix run is retained here because it materially changed the closure result.

## R46/Q14 bounded soak and fault evidence

R46 is defined as bounded scheduler obligations under sustained admission rather than process-RSS benchmarking.

Permanent Gateway qualification `da432d7` proves:

- 4,000 admissions across 100 rounds;
- active calls never exceed the configured global bound;
- queued calls never exceed the configured queue bound;
- every round returns to zero active and zero queued obligations;
- no scheduler capacity leak remains after the soak.

Q14 is a composite fault qualification rather than one monolithic test. The soak is combined with permanent Gateway tests for cancellation, queue expiry, child crash-loop/circuit behavior, failed child catalog refresh, drain, and payload/artifact disk-pressure cleanup.

## Workspace aliases

ADR 0034 explicitly **does not retain workspace aliases in M7**. No alias registry, alias binding lifetime, or alias-derived authority is implemented.

Therefore former R34/R35 alias requirements are retired by contract, not reported as implementation PASS. Any future workspace-alias feature requires a separate ADR and new verification rows before it can expand the M7 surface.

## Clean provenance

The audit used detached worktrees at the exact Studio/Gateway/Fleet merge commits; all three were clean. Filesystem `main` at `ec68a04` was clean. Qualification changes were isolated on dedicated branches.

Concurrent unrelated working-tree changes in Studio/Fleet/Gateway feature worktrees were not used as clean-source evidence and were not staged into the qualification commits.

## Release/publication boundary

Local release tags currently exist:

- Studio `v0.7.0-beta`
- Gateway `v0.2.0`
- Filesystem `v0.2.0`
- Fleet `v0.3.0`

Remote-refresh publication guards prove that all four M7 merge commits and all four tags are currently **unpublished** on `origin`.

Main integration is complete for both post-tag closure fixes. Current local tag targets are now asymmetric: Gateway `v0.2.0` points at qualified `main` `36aea90`, Filesystem `v0.2.0` points at `ec68a04`, and Fleet `v0.3.0` points at `7bad5e2`; Studio `v0.7.0-beta` still points at pre-closure merge `1c5dac5` rather than qualified `main` `3423d8e`. All four tags and the qualified main commits were rechecked against `origin` and remain unpublished. Studio tag reconciliation is therefore the only local tag correction still required before release publication. No push, public release, or deployment is part of this audit.

## Closure disposition

- M7.0 architecture freeze: PASS.
- P-01..P-09 dispositions: PASS.
- M7.1–M7.8 implementation evidence: PASS on the qualified commits above.
- R45 runtime-only closure: PASS after Studio fix `543a6c4`.
- R46 bounded soak: PASS on Gateway qualification commit `da432d7`.
- R49 clean provenance: PASS.
- R50 implementation/tag/publication state separation: PASS.
- Final implementation/main integration readiness: **PASS**.
- Final release/tag publication readiness: **PENDING Studio `v0.7.0-beta` local tag reconciliation and an explicit publication/deployment decision**.
