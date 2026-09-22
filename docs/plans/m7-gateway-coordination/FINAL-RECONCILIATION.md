# M7 Final Reconciliation & Pre-M8 Gate

**Opened:** 2026-09-22
**State:** QUALIFIED / CLOSURE PATCH 0.7.1 PREPARED / FINAL MAIN EVIDENCE REQUIRED
**Purpose:** reconcile post-qualification M7 changes, re-qualify the current authoritative source heads, repair release/document truth, and establish a clean M8 entry gate.

## Frozen starting baseline

| Component | Starting HEAD | Version | Why requalification is required |
|---|---|---:|---|
| Studio | `08a3fb4fcb06eac391a79a5ab8050d35908be887` | 0.7.0-beta | post-tag Fleet validation and tunnel lifecycle fixes |
| Gateway | `469ec9258855d39d9a3f57bae60ff4810ffeafb1` | 0.2.1 | post-M7 artifact/release hygiene changes |
| Filesystem | `df98dc38c969c92f25c778d37691e7a3f1914367` | 0.2.0 | final M7 revision-aware operations merged after older qualification evidence |
| Exec | `50d5b4df9d3ddcbf0203f47321ce90efda15ca47` | 0.1.0 | MCP request cancellation propagation added after older qualification evidence |
| Git | `31ceadd79751c3be7aed197697c88d74c594b812` | 0.1.0 | canonical release-tag identity repair |
| Fleet | `2ccdf54495e23c86da5bc8f6799b44d225c2e136` | 0.4.0 | current Fleet includes render-plan and Sonar release-quality integration |

All six authoritative repositories were clean and aligned with their configured upstream at plan start.

## Closure invariants

1. Qualification evidence is valid only for the exact source identities captured at the beginning and end of a run.
2. A source change during qualification blocks closure even if every command returned zero.
3. Rust qualification requires the pinned 1.98.1 toolchain. The runner explicitly prepends `~/.cargo/bin` because the MCP execution environment intentionally exposes a restricted PATH.
4. Fleet desired-state validation uses the side-effect-free `render-plan --json` contract. Legacy render-and-check commands are not the authoritative Studio validation contract.
5. Component qualification, release/tag identity, publication, and deployment are separate states.
6. No M8 unattended policy, Workspace Skill Runtime implementation, remote authentication, Linux support, or other scope expansion is allowed into this closure branch.

## Execution gates

### C0 — provenance

- capture branch, exact HEAD, version, clean state and upstream relation for Studio/Gateway/Filesystem/Exec/Git/Fleet;
- fail closure when any required repository is dirty under `--require-clean`;
- compare all source identities after qualification.

### C1 — qualification infrastructure

- version-control the M7 closure runner in Studio;
- require Cargo/Rust/Rustfmt/Clippy/Cargo Audit/Python/Node/pnpm/Git preflight;
- require Rust 1.98.1;
- validate Fleet render-plan JSON, path confinement, unique surfaces/paths, base64 payloads and SHA-256;
- make root orchestration delegate Fleet validation to this contract.

### C2 — component gates

- Studio Rust + web;
- Gateway Rust;
- Filesystem Rust;
- Exec Rust;
- Git Rust;
- Fleet Python suite;
- dependency audit for every Rust component.

### C3 — post-qualification regressions

- Studio pure Fleet render-plan validation;
- running tunnel remains stoppable if its runtime executable disappears;
- Gateway bounded scheduler soak;
- Filesystem revision/CAS external-change and concurrent-writer proofs;
- Exec cancellation terminates the owned direct child.

### C4/C5 — integration and fault evidence

- runtime-only reconciliation against deployed binaries;
- M5/M6 regression profile;
- Fleet malformed/unsafe render-plan rejection remains covered by permanent tests;
- no automatic mutation replay or unbounded scheduler obligation is introduced.

### C6 — documentation truth

Only after exact-head qualification passes:

- set M7 as the current verified baseline;
- update current component versions;
- change immediate-next-step to M8;
- regenerate qualification/task/planning evidence from the final source identities.

### C7 — release/tag reconciliation

- verify publication state before changing any local release tag;
- an unpublished local tag may be atomically reconciled;
- a published/shared tag is immutable and requires a new version.

### C8/C9 — hygiene and final audit

- preserve useful facts from stale worktrees, then remove them;
- leave authoritative repositories clean;
- final closure requires reproducible evidence and no known Critical/High blocker;
- publication/deployment remain explicit later operator decisions.

## Authoritative runner

`scripts/verify-m7-closure.py`

Profiles:

- `preflight` — provenance, toolchain and Fleet contract;
- `fleet-contract` — same fast contract gate for root orchestration;
- `components` — all component/static/unit/build/audit gates;
- `targeted` — post-qualification regression proofs;
- `full` — components + targeted + runtime-only + M5/M6 regression profile.

A final M7 closure claim requires:

```bash
python3 scripts/verify-m7-closure.py --profile full --require-clean
```

and a PASS summary generated from a committed clean branch/tree.


## Reconciliation outcome

The first clean `full --require-clean` run on the new qualification infrastructure caught a stale runtime-only fixture because Fleet 0.4 requires an explicit valid `tunnel.tunnel_id`. Studio now preserves bounded Fleet render-plan stderr diagnostics, the fixture carries the current tunnel identity contract, and the same runtime-only smoke passes.

A subsequent clean full run passed all component, targeted, runtime-only, and M5/M6 regression gates with no source change during execution.

Remote refresh also corrected prior release-state assumptions: Studio `v0.7.0-beta` and `v0.7.0` are already present on origin. They are not rewritten. The closure release identity therefore advances to `0.7.1`.

The final closure remains conditioned on repeating the same clean full runner on the exact final main/release head. Its generated `summary.json` is the authoritative exact-head evidence.
