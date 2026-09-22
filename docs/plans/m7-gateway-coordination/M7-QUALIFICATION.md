# M7 Qualification and Closure Audit

**Audit date:** 2026-09-22
**Status:** VERIFIED / FINAL RECONCILIATION IMPLEMENTED / CLOSURE PATCH 0.7.1
**Publication:** source integration, release tagging, remote publication, and deployment are distinct states. Previously published tags are immutable.

## Current qualified component baseline

The final reconciliation runner records exact Git identity at both the beginning and end of every run. The authoritative exact-head record is the generated `summary.json`; this document records the component line used for the M7 closure patch.

| Component | Current qualified line | Version | Current release-state observation |
|---|---|---:|---|
| Studio | M7 closure branch/main tree derived from `08a3fb4` plus final reconciliation fixes | 0.7.1 | `v0.7.0-beta` and `v0.7.0` already exist on origin and are immutable |
| Gateway | `469ec9258855d39d9a3f57bae60ff4810ffeafb1` | 0.2.1 | current head is on origin/main; `v0.2.0` is published; local `v0.2.1` was not observed on origin during the audit |
| Filesystem | `df98dc38c969c92f25c778d37691e7a3f1914367` | 0.2.0 | current head is on origin/main; local `v0.2.0` was not observed on origin during the audit |
| Exec | `50d5b4df9d3ddcbf0203f47321ce90efda15ca47` | 0.1.0 | current head and `v0.1.0` are published on origin |
| Git | `31ceadd79751c3be7aed197697c88d74c594b812` | 0.1.0 | current head and `v0.1.0` are published on origin |
| Fleet | `2ccdf54495e23c86da5bc8f6799b44d225c2e136` | 0.4.0 | current head is on origin/main; `v0.3.0` is published; local `v0.4.0` was not observed on origin during the audit |

The older M7 implementation/closure commits remain valid historical lineage. They are not used as substitutes for current-head evidence.

## Final reconciliation runner

Studio now owns the authoritative foreground runner:

`scripts/verify-m7-closure.py`

It provides:

- exact branch/HEAD/version/clean-state capture for Studio, Gateway, Filesystem, Exec, Git and Fleet;
- source-change detection across the run;
- explicit Rust 1.98.1 toolchain validation;
- Cargo/Rustfmt/Clippy/Cargo Audit/Python/Node/pnpm/Git preflight;
- Fleet `render-plan --json` validation including schema, host identity, confined relative paths, unique surfaces/paths, base64 decoding and SHA-256 verification;
- full component format/check/Clippy/test/build/audit gates;
- Studio web lint/typecheck/test/build;
- targeted M7 regression proofs;
- explicit runtime-only reconciliation;
- M5/M6 regression execution.

The root `mcp-server/Makefile` is convenience orchestration only. The version-controlled Studio runner is the qualification authority.

## Final component gates

The clean-source reconciliation run covers:

### Studio

- Rust fmt/check/Clippy — PASS;
- Rust tests — 287 tests reported passed across the suite during the reconciliation qualification;
- build — PASS;
- cargo audit — PASS;
- web lint/typecheck/test/build — PASS;
- permanent runtime-loss tunnel stop regression — PASS;
- pure Fleet render-plan validator test — PASS.

### Gateway

- fmt/check/Clippy/build/audit — PASS;
- full suite — 52 tests PASS;
- permanent bounded scheduler soak — PASS.

The soak retains the M7 bound proof: 100 rounds × 40 admissions, global active bound 8, queue bound 32, and zero active/queued obligations after every round.

### Filesystem

- fmt/check/Clippy/build/audit — PASS;
- full suite — 10 tests PASS;
- external-change revision/CAS proof — PASS;
- concurrent same-revision writer serialization/recheck — PASS.

### Exec

- fmt/check/Clippy/build/audit — PASS;
- full suite — 5 tests PASS;
- MCP cancellation/direct-child termination regression — PASS.

The current contract guarantees termination of the owned direct child. M8 must not silently broaden that statement into arbitrary process-tree cancellation without a separately reviewed process-group contract.

### Git

- fmt/check/Clippy/build/audit — PASS;
- full suite — 16 tests PASS;
- canonical release-tag identity repair remains on the current qualified head.

### Fleet

- full Python unittest discovery — PASS;
- current Aira pure render-plan emits six validated managed surfaces:
  - `gateway.exec`;
  - `gateway.filesystem`;
  - `gateway.git`;
  - `gateway.sonarqube`;
  - `studio.config`;
  - `tunnel.config`.

## Runtime-only reconciliation finding

The first new full clean reconciliation run exposed a real stale qualification fixture:

```text
Fleet render-plan returned failure
```

The render-plan provider originally discarded stderr, hiding the actual reason. The reconciliation fix now retains a bounded 512-character stderr diagnostic. The rerun exposed:

```text
fleetctl: invalid or missing tunnel.tunnel_id
```

Root cause: the Studio runtime-only smoke fixture predated the current Fleet tunnel identity contract. The runtime-only host profile was updated with an explicit valid tunnel ID; no production authority was relaxed.

After the fix:

- Fleet pure render-plan validation PASSes;
- the explicit source-less/runtime-only reconciliation smoke PASSes;
- real copied Gateway/Filesystem/Git/Exec binaries load a 35-tool three-child catalog;
- the deliberately absent source root remains absent;
- M5/M6 regression profile PASSes.

This finding is retained because the final runner correctly detected cross-repository contract drift that component-only testing did not expose.

## Tunnel lifecycle regression

The Studio UI already retained Stop while a running tunnel reported `runtime_available=false`. Final reconciliation adds backend lifecycle evidence:

1. start a real test tunnel;
2. remove its configured runtime executable while the process remains running;
3. verify status remains Running with `runtime_available=false`;
4. stop the owned process successfully;
5. verify Stopped state and PID clearing.

Start/restart continue to require an available runtime. Stop remains an ownership operation over the already-running process.

## M5/M6 regression boundary

The final M7 runner executes the existing M6 regression profile after the current M7 component and targeted gates. This preserves the authority split:

- Gateway/runtime owners retain live request authority;
- M6 SQLite remains historical evidence only;
- M5 update/rollback contracts remain authoritative;
- M7 does not enable M8 unattended update/reconciliation policy.

## Release and publication truth

Remote-refresh verification materially corrected earlier M7 documentation.

For Studio:

- `v0.7.0-beta` exists on origin;
- `v0.7.0` exists on origin;
- those tags are immutable and must not be moved;
- the post-release closure patch therefore advances to `0.7.1`.

For other components, current main reachability and tag publication differ by repository; the table above records the observed state. No tag is rewritten merely to make historical M7 documentation look uniform.

A local `v0.7.1` tag may be created only after final clean main qualification. Remote publication and deployment require explicit later action.

## Closure disposition

- M7.0 architecture freeze: PASS.
- P-01..P-09: CLOSED.
- M7.1–M7.8 implementation: PASS.
- M7.9 qualification: PASS on current component lines.
- Runtime-only reconciliation: PASS after current Fleet contract fixture repair.
- Bounded scheduler soak: PASS.
- Filesystem revision/CAS: PASS.
- Exec cancellation: PASS for the owned direct child.
- Tunnel runtime-loss stop: PASS.
- M5/M6 regressions: PASS.
- Exact-source provenance: enforced by the final runner.
- Release-state truth: corrected; published tags are immutable.
- M7 closure patch: `0.7.1`.
- M8 implementation: may begin only after final clean main qualification remains green.
- Remote publication/deployment: separate explicit operator decisions.
