# M7 Planning and Qualification Validation

## Scope

This file began as planning-only evidence. M7 implementation has since been completed across Gateway, Filesystem, Fleet and Studio, so this revision records the current qualification boundary and points to the detailed closure audit.

See [M7-QUALIFICATION.md](M7-QUALIFICATION.md) for exact commits, gates, the runtime-only defect/fix, soak evidence, provenance and publication state.

## Frozen planning lineage

P-01 through P-09 were dispositioned through the Studio planning commits:

- `4f3f0d2` — initial M7 source audit/planning package;
- `7c93d1c` — drain/control-socket and scheduler contracts;
- `653d2e6` — remaining P-02/P-03/P-05/P-07/P-08 contracts;
- `57bff30` — P-gate closure checkpoint.

Gateway protocol/control viability evidence originated in:

- `6397661` — rmcp protocol + control-socket permanent tests;
- `60a0c00` — corrected LF-framed control-socket probe.

ADR 0026–0035 now form the frozen M7 contract set.

## Qualified implementation lineage

Current implementation baselines:

- Gateway merge `5b886c95509b63d6d31c1ba903825b8a385027ff`, version 0.2.0;
- Filesystem `ec68a043d9c67763c83d32e3a0b4f0a9f8e8c73f`, version 0.2.0;
- Fleet merge `7bad5e2d4623e9867010572679dfc597a7dc4680`, version 0.3.0;
- Studio merge `1c5dac50849134364c2fec6e70225b9027265812`, target 0.7.0-beta.

Closure audit additionally produced:

- Studio `543a6c4` — derive managed reconciliation surfaces from the trusted active host, fixing schema-v1 runtime-only compatibility;
- Gateway `da432d7` — permanent bounded scheduler soak proof.

The two closure commits are qualification descendants of the implementation merges and must be integrated before the existing local Studio/Gateway release tags are treated as final qualified tags.

## Current qualification results

### Gateway

Clean detached merge worktree:

- fmt/check/Clippy — PASS;
- full suite at merge — 51 tests PASS;
- qualification branch with permanent soak — 52 tests PASS;
- 4,000-admission scheduler soak returns active/queued state to zero after every round.

### Filesystem

- fmt/check/Clippy — PASS;
- 10/10 tests PASS;
- revision/CAS, external-edit conflict, patch/range/search bounds, symlink confinement and replacement-failure cleanup are covered.

### Fleet

Clean detached merge worktree:

- 19/19 Python tests PASS;
- host schema v2, Gateway policy rendering and dual tunnel asset selection are covered.

### Studio

Clean detached merge worktree:

- fast quality profile 6/6 PASS;
- Rust suites PASS;
- web Vitest 31/31 PASS.

The explicit runtime-only reconciliation smoke initially failed on merge `1c5dac5` because Studio required the schema-v2 `gateway.policy` surface for a supported schema-v1 host. Closure fix `543a6c4` makes the expected surface set derive from the trusted active host and the same explicit runtime-only smoke then PASSes.

## Workspace alias disposition

Workspace aliases are not implemented or retained in M7. ADR 0034 explicitly retires that proposed surface. Former verification rows R34/R35 are retired by contract rather than presented as implementation PASS.

## Tunnel evidence

The M7 tunnel architecture remains the ADR 0032 dual-artifact boundary:

- runtime-cloudflared stays the Studio-owned daemon;
- full `tunnel-client` is the bounded diagnostics/preflight companion;
- both artifacts are same-release verified.

Studio implementation includes exact dual-asset selection/staging, matching version/commit validation, bounded full-client `doctor`, and distinct liveness/readiness/MCP-discovery/control-plane-poll observations.

## Provenance and publication

Clean-source qualification used detached worktrees at the exact Gateway/Fleet/Studio merge commits; Filesystem main was clean.

Remote-refresh publication guards prove these merge commits are not reachable from `origin` and the local release tags are also unpublished:

- Studio `v0.7.0-beta`;
- Gateway `v0.2.0`;
- Filesystem `v0.2.0`;
- Fleet `v0.3.0`.

Local tagging is therefore distinct from publication/deployment. Because Studio `543a6c4` and Gateway `da432d7` are post-tag closure fixes, final tag reconciliation must happen only after those fixes are integrated. No push, public release or deployment is claimed here.

## Current disposition

- M7.0 architecture freeze: complete.
- P-01..P-09: closed.
- M7.1–M7.8 implementation: qualified on the recorded commits.
- M7.9 runtime-only and bounded-soak gaps discovered by closure audit: fixed/proved on dedicated closure branches.
- Final main integration and local unpublished tag reconciliation: pending.
