# M7 Planning Validation

## Scope

The initial planning pass audited current Aira workspace source and created the M7 planning package under:

`mcp-server/studio/docs/plans/m7-gateway-coordination/`

M7.0 execution added permanent Gateway protocol/control-socket viability tests and froze ADR 0026–0032 on dedicated branches: Studio `feature/m7-planning` and Gateway `feature/m7-protocol-spike`. Planning/test commits exist locally; no tag, push, release publication or deployment has been performed. A later uncommitted `Gateway/src/main.rs` change exists in the shared worktree and is treated as concurrent work: P-02/P-03 source evidence is taken from committed Gateway HEAD, not from that unqualified working-tree change.

## Verified planning baselines

- Studio `main`: `48906d892fb37275f8b2d10b73b7cad1eec355c9`, clean before planning writes.
- Gateway `main`: `02a46af100c0cee5e011b8bc74079d7a78dd3658`, clean.
- Filesystem `main`: `0f09a665bb94af8388e01051d87cb3f865d4310d`, clean.
- Fleet worktree: `feature/sonarqube-main-gate`; treated as an unrelated in-progress branch and not modified.

## Commands/checks executed

Gateway:

- locked SDK: `rmcp 3.4.0`; Rust `1.98.1`;
- protocol spike: 7/7 PASS;
- full Rust fmt/check/clippy: PASS;
- full Rust tests after control-socket probe fix: 16/16 PASS (6 existing + 7 protocol + 3 control-socket);
- aggregate workspace auto-detection is intentionally not used as the M7 gate because local excluded Node markers cause a false Node-project classification; the markers are not tracked source.

Filesystem:

- committed 0.1.0 baseline audited for cap-std confinement, whole-file bounds and sibling-temp/sync/rename overwrite;
- fresh 2026-09-22 `cargo fmt --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features`: PASS; 3/3 baseline tests;
- P-05 v2 request/error/CAS/patch contract frozen in ADR 0030;
- production v2 implementation/tests remain M7.4 work.

Tunnel:

- official latest stable release rechecked on 2026-09-22: v0.0.14;
- local deployed `tunnel-client-runtime-cloudflared --version/--help`: PASS and confirms v0.0.14 run-only surface;
- local runtime `doctor --help` and `runtimes --help`: expected unsupported commands;
- official full-client darwin-arm64 archive downloaded to `/tmp` only; SHA-256 matched official manifest (`b540493c5bdbcdbb755700c8e2e16597e28b1569e425007e0f73111047bd6a64`);
- disposable full-client `--version`, `doctor --help`, `runtimes --help`: PASS;
- temporary workspace probe binary removed; Studio worktree was clean immediately afterward;
- dual-artifact ownership/release boundary frozen in ADR 0032.

Studio planning branch qualification:

- fresh 2026-09-22 fast quality profile: 6/6 checks PASS;
- Rust fmt/check/clippy/tests PASS;
- existing Studio Rust suites and web Vitest suite PASS;
- `git diff --check` PASS after contract reconciliation.

## P-01..P-09 closure checkpoint

P-01 through P-09 now have committed design/test dispositions.

Committed evidence:

- Studio `653d2e6`: closes remaining P-02/P-03/P-05/P-07/P-08 contracts and records version targets;
- Studio `7c93d1c`: freezes P-06 drain/control-socket and scheduler/concurrency contracts;
- Studio `4f3f0d2`: initial M7 planning/source-audit package including P-01..P-09 inventory;
- Gateway `6397661` + `60a0c00`: rmcp protocol and corrected control-socket viability evidence.

This closes the **P-gate set**, not all of M7.0. Remaining M7.0 blockers are payload/artifact bounds, profile/workspace-context semantics, Gateway→M6 telemetry/privacy DTOs, and the final ADR-set consistency audit.

## Not executed / not claimed

- no concurrency/drain/cancellation production tests;
- no qualified Gateway scheduler/recovery production implementation is claimed by this planning package;
- no Filesystem v2 production code;
- no Fleet schema v2 production code (and no M7 writes were made on `feature/sonarqube-main-gate`);
- no Studio history schema/event change;
- no persistent Tunnel full-client install or deployed runtime mutation (probe was disposable and checksum-verified);
- no native M7 qualification/soak;
- no source-less M7 package smoke.

These are M7.0+ implementation requirements, not planning-session evidence.
