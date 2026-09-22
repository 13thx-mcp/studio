# M7 Planning and Qualification Validation

## Scope

M7 planning is complete. This document now records the transition from the original implementation lineage to the final current-head reconciliation used as the M8 entry gate.

See:

- [M7-QUALIFICATION.md](M7-QUALIFICATION.md) for current qualification and release-state evidence;
- [FINAL-RECONCILIATION.md](FINAL-RECONCILIATION.md) for the pre-M8 closure program;
- [M7-VERIFICATION-MATRIX.md](M7-VERIFICATION-MATRIX.md) for R01–R52.

## Frozen planning lineage

P-01 through P-09 were dispositioned through:

- `4f3f0d2` — initial M7 source audit/planning package;
- `7c93d1c` — drain/control-socket and scheduler contracts;
- `653d2e6` — remaining contract freeze;
- `57bff30` — P-gate closure checkpoint.

Gateway protocol/control viability originated in `6397661` and `60a0c00`. ADR 0026–0035 remain the frozen M7 contract set.

## Historical implementation lineage

Historical M7 implementation evidence includes:

- Gateway implementation merge `5b886c9`, soak proof `da432d7`;
- Filesystem M7.4 line including `ec68a04`;
- Fleet M7 policy line including `7bad5e2`;
- Studio implementation `1c5dac5`, runtime-only surface fix `543a6c4`, and subsequent integration.

Those commits remain useful lineage, but final closure does not equate historical qualification with current HEAD.

## Current reconciliation baseline

Current component lines requalified for the pre-M8 gate are:

- Gateway `469ec9258855d39d9a3f57bae60ff4810ffeafb1`, version 0.2.1;
- Filesystem `df98dc38c969c92f25c778d37691e7a3f1914367`, version 0.2.0;
- Exec `50d5b4df9d3ddcbf0203f47321ce90efda15ca47`, version 0.1.0;
- Git `31ceadd79751c3be7aed197697c88d74c594b812`, version 0.1.0;
- Fleet `2ccdf54495e23c86da5bc8f6799b44d225c2e136`, version 0.4.0;
- Studio closure patch version 0.7.1, with exact final HEAD captured by the final qualification summary.

## Current qualification results

The version-controlled `scripts/verify-m7-closure.py` runner proves in one foreground execution:

- pinned Rust 1.98.1 toolchain and required development tools;
- clean-source provenance and no source identity change during the run;
- all Rust component fmt/check/Clippy/test/build/audit gates;
- Studio frontend lint/typecheck/test/build;
- Fleet Python tests and pure render-plan contract;
- Gateway bounded scheduler soak;
- Filesystem external-change and concurrent CAS proofs;
- Exec MCP cancellation;
- tunnel runtime-loss Stop behavior;
- source-less/runtime-only reconciliation;
- M5/M6 regression profile.

The first full reconciliation run caught a stale runtime-only fixture missing the current Fleet `tunnel.tunnel_id` requirement. Bounded stderr diagnostics exposed the exact failure. The fixture was corrected without weakening production validation, and the full run then passed.

## Workspace alias disposition

Workspace aliases remain outside M7. ADR 0034 retires the proposed alias surface; R34/R35 remain retired by contract.

## Release-state correction

Remote refresh proved earlier documentation was stale:

- Studio `v0.7.0-beta` and `v0.7.0` are published on origin and immutable;
- current post-release reconciliation must use a new patch identity, `0.7.1`;
- other component tag publication is not uniform and is recorded repository-by-repository in M7-QUALIFICATION.

No remote tag is rewritten for cosmetic consistency.

## Current disposition

- architecture/protocol freeze: complete;
- M7 implementation: complete;
- current-head reconciliation: complete;
- full clean-source pre-M8 qualification: required on the exact final main/release head and enforced by the runner;
- M7 closure patch identity: 0.7.1;
- next implementation milestone: M8 Hardening, Auto-Update Policy & Recovery;
- Workspace Skill Runtime remains a separate design track;
- remote publication/deployment remain explicit later actions.
