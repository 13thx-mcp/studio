# Milestone 5 Status — Runtime Distribution, Update Manager & Fleet State

- Target release: `v0.5.0`
- Implementation branch: `feat/m5-runtime-distribution`
- Status: PUBLICATION QUALIFIED
- Date: 2026-09-19

> Superseded support policy (2026-09-21): product/runtime support is now macOS Apple Silicon (`darwin-arm64`) only. Earlier amd64 references in this historical M5 closeout describe the policy at that time.

## 2026-09-21 publication qualification

`scripts/verify-m5-publication.py` completed with qualified arm64 publication evidence, including verified project artifacts, source-less bootstrap, and the Fleet `0.2.0` → `0.2.1` transition. Historical blocker notes below record the prior state and are superseded by this result.

## Review remediation status

The 2026-09-19 Mission 5 review found F1–F8 safety/correctness gaps. Corrective implementation is now present for all eight findings and every finding-specific regression has been turned green in targeted verification, including real isolated Studio/Fleet launcher and Gateway/child-protocol proofs. The re-review also found and closed an additional F5 readiness gap by requiring a Fleet-generated nonce plus PID/config proof written by the exact Studio process only after listener bind.

R6 fixed-tree verification is complete on Studio `a388cf2` and Fleet `8b82752`: regressions, core/Fleet, security, isolated integration and native darwin-arm64 package profiles all passed with clean before/after worktrees and no source change during any run. Evidence: `issues/m5-remediation/results/20260919T140643Z-18307`, `issues/m5-remediation/results/20260919T140854Z-21177`, `issues/m5-remediation/results/20260919T141313Z-27968`, `issues/m5-remediation/results/20260919T141541Z-31839`, `issues/m5-remediation/results/20260919T141810Z-37530`.

The 2026-09-19 R6 evidence by itself supported **LOCAL RELEASE CLOSED** only. Publication qualification was subsequently established on 2026-09-21 by `scripts/verify-m5-publication.py`, which verified the required published project artifacts, source-less bootstrap, and Fleet 0.2.0 → 0.2.1 transition on darwin-arm64.

## Outcome

Milestone 5 implements the runtime distribution/update architecture for source-present and source-less macOS hosts: trusted release providers, strict platform asset selection, verified staging, runtime inventory, transactional MCP/Gateway/Fleet/Tunnel updates, runtime-config reconciliation, Studio self-update through an external Fleet launcher, and the Updates/Fleet dashboard.

The implementation, local/source-less fixture verification, and publication qualification are green. The earlier 2026-09-19 GitHub Releases 404 condition is historical and was cleared by the 2026-09-21 publication qualification run.

Studio and web package versions are `0.5.0`; the verified release is tagged `v0.5.0`, and the publication evidence verifies the published Studio artifact together with the other required project-owned artifacts.

## Implemented architecture

### Trusted release domain

- Closed component catalog: Filesystem, Git, Exec, Gateway, Blender, Studio, Fleet, Tunnel.
- Project-owned provider confined to `13thx-mcp/*` repositories selected by server policy.
- Tunnel provider confined to official `openai/tunnel-client` GitHub Releases.
- Supported runtime target: `darwin-arm64`.
- Exact asset-name matching; missing/ambiguous assets fail closed.
- Browser never chooses repository, release URL, checksum URL, executable path, install path, launcher path, PID, config path, or arbitrary command.

### Verified staging and tamper resistance

- Bounded HTTPS download with trusted redirect policy.
- SHA-256 manifest verification before extraction.
- Archive traversal, absolute path, duplicate entry, symlink, hardlink, and special-file rejection.
- Component-specific package structure validation.
- Staged executable/archive identity is revalidated immediately before activation.
- Startup removes stale partial staging directories while preserving finalized rollback material.

### Runtime inventory

- Installed/running/desired/latest identities are independent dimensions.
- Installed truth comes from deployed runtime artifacts rather than Git HEAD.
- Drift states include current, update available, installed restart required, drifted, unknown, and broken.
- Source-present and runtime-only host modes are explicit.
- Release checks fan out concurrently over the closed component catalog while preserving trusted provider/error policy.

### Transaction classes

- Ordinary Rust MCP: stop-if-running, atomic flat-bin activation, restart-if-running, verification, automatic rollback.
- Gateway: tunnel-owned control-path stop/restart, exact servers.d preservation, standalone MCP protocol/catalog verification, reconnect verification, automatic rollback.
- Fleet: control-bundle replacement preserving active host profile/state and validating generated runtime config.
- Tunnel: official OpenAI verified ZIP, versioned `releases/vX.Y.Z`, atomic relative `current` switch, stopped/running state preservation, config/credentials fingerprint preservation, upgrade rollback, and same-version verified force reinstall.
- Studio: durable self-update transaction, versioned backend+web release unit, external Fleet launcher activation, target `/health` version proof, restart/reconnect recovery, and rollback.

### Runtime reconciliation

- Fleet pure-render contract is treated as desired-state inventory.
- Managed-safe drift is distinguished from unknown/unmanaged local edits.
- Canonical Gateway child configs, Studio config, and tunnel YAML are checked independently of helper/CLI override paths.
- Exact Gateway tool-name identity is fingerprinted; equal tool counts with different names are detected.
- Connected-client freshness remains unknown/refresh-pending unless externally acknowledged.
- Post-write catalog verification failure rolls config/runtime ownership back.

### Updates/Fleet UI

- Shows installed/running/desired/latest versions, host mode, platform, integrity, drift, release check, runtime state, and transaction phases.
- Supports backend-proven check/prepare/apply operations for all update-capable components, including Tunnel.
- Tunnel same-version selection is presented as verified force reinstall.
- Gateway/Studio reconnect phases are explicitly distinguished from terminal failure.
- Multiple non-terminal transaction IDs are retained independently and refetched after reconnect.
- Reconciliation apply is gated to server-reported managed-safe drift.
- No unattended update/reconciliation policy exists in M5.

## M5.12 incident-derived reconciliation drill

The explicit closure drill reproduced the 2026-09-18 incident class inside an isolated runtime fixture:

```text
trusted managed baseline
→ desired tunnel YAML + Exec child config advance
→ helper/launcher remains canonical and could mask stale generated bytes
→ managed-safe drift detected
→ transactional repair
→ exact catalog fingerprint proof
→ equal-count/different-tool identity produces refresh_pending
→ forced post-write catalog verification failure
→ old config bytes + running tunnel state restored
→ unknown operator edit rejected without overwrite
→ separate stopped-tunnel leg remains stopped
```

Result: PASS.

## Source-less runtime proof

Explicit Aira source-less reconciliation smoke uses a temporary runtime root with a deliberately missing source root and copied deployed runtime artifacts/Fleet contract. It starts the real Gateway and configured child binaries only from the flat runtime layout.

Observed real Gateway summary:

```text
configured_servers: 3
enabled_servers: 3
running_servers: 3
exposed_child_tools: 35
failed_servers: []
```

Source-less reconciliation, local catalog fingerprinting, and runtime operation passed without project source inside the fixture runtime root.

## Tunnel update closure

Roadmap §5.9 required Tunnel activation/rollback, not only version visibility. M5.12 therefore added `TunnelUpdateManager` and wired it through the existing update transaction API/UI.

Targeted Tunnel suite:

```text
8 normal tests passed
1 official-network smoke ignored by default
```

The explicit official-network smoke performed:

```text
official OpenAI release discovery
→ v0.0.14 asset selection
→ SHA-256 verification
→ safe ZIP extraction
→ binary version verification
→ temporary versioned install
→ same-version force reinstall
→ current remains releases/v0.0.14
→ stopped state remains stopped
→ config.yaml preserved
→ credentials preserved
```

Result: PASS.

## Rollback / failure-injection evidence

Verified independently:

- running MCP update success and health verification;
- stopped MCP update remains stopped;
- MCP verification failure restores prior binary and running state;
- Gateway reconnect/catalog success;
- Gateway new-binary protocol failure restores prior Gateway and owner state;
- Fleet bundle update preserves host profile/state;
- Studio external launcher target-health failure restores legacy source and local config/data;
- Tunnel restart failure restores prior release/current pointer and running state;
- Tunnel rollback failure is explicit;
- reconciliation post-write catalog failure restores previous generated bytes and tunnel state.

## Security / tamper evidence

Verified rejection of:

- checksum mismatch;
- wrong/unsupported architecture/platform;
- traversal and absolute archive entries;
- duplicate entries;
- archive symlinks/hardlinks;
- missing expected executable/package structure;
- browser authority fields such as install path;
- unknown update component/transaction;
- staged/candidate tamper before lifecycle stop;
- unsafe Tunnel `current` pointer;
- unmanaged local reconciliation edits.

## Restart/recovery evidence

- ordinary MCP startup cleanup removes partial activation scratch and retains known-good rollback;
- stale Studio `preparing` transaction is made terminal and no longer blocks future update;
- interrupted Studio activation can be re-armed after candidate promotion;
- Tunnel startup recovery can restore a single same-version rollback if `current` target is missing;
- UI persists/refetches multiple pending transaction IDs after reconnect.

## Soak / cleanup

Key inventory/reconciliation/recovery tests were repeated 5 consecutive iterations with no failure.

Post-run live Aira inspection:

```text
stale .partial-* directories: none
Studio activation scratch: none
Tunnel candidate/failed/rollback scratch: none
test orphan processes: none
staging root: empty
```

Live processes were only the expected production tunnel runtime, Gateway, Exec, Filesystem, and Git children.

## Full quality gates

Final remediation fixed-tree pair:

```text
Studio: a388cf2611fead432e79ae041112d761632cbc00
Fleet:  8b827521fdcb423b1c87e70d3885d518536842fd
```

Core/Fleet profile `issues/m5-remediation/results/20260919T140854Z-21177`:

```text
cargo fmt --all -- --check                              PASS
cargo check --locked --all-targets --all-features       PASS
cargo clippy --locked --all-targets --all-features -- -D warnings PASS
cargo test --locked --all-targets --all-features        PASS
cargo build --locked --all-targets --all-features       PASS
pnpm --dir web lint                                     PASS
pnpm --dir web typecheck                                PASS
pnpm --dir web test                                     PASS — 7 files / 27 tests
pnpm --dir web build                                    PASS
Fleet py_compile                                        PASS
Fleet unittest                                          PASS — 20/20
```

Final Rust test counts from the core profile:

```text
181 unit tests passed
10 explicit/real smokes ignored by the default all-target test run
4 registry/discovery integration tests passed
7 supervisor lifecycle tests passed
7 tunnel lifecycle tests passed
0 failures
```

Finding regression profile `issues/m5-remediation/results/20260919T140643Z-18307` executed all ten required exact tests and passed. The isolated profile `issues/m5-remediation/results/20260919T141541Z-31839` then explicitly ran five reviewed ignored tests — Studio external activation success, Studio health-failure rollback, real Gateway+children catalog probe, the incident-derived reconciliation drill, and runtime-only reconciliation — all PASS.

Security profile `issues/m5-remediation/results/20260919T141313Z-27968` passed `cargo audit` (1251 advisories loaded; 268 locked dependencies scanned) plus required tamper, symlink and origin test filters.

Native package profile `issues/m5-remediation/results/20260919T141810Z-37530` passed on the current **Darwin arm64** host. It built release backend + production web assets, packaged and re-extracted `mcp-studio-v0.4.0-darwin-arm64.tar.gz`, and verified backend version/web identity. Archive SHA-256:

```text
1cef290d8066c77ef687a75a7bf60473efb3d6569f5f75e45c859a97d4fac706
```

The native manifest records `darwin-arm64` coverage, the sole supported runtime target.

## Historical release-publication blocker — RESOLVED 2026-09-21

Before publication qualification was completed, on 2026-09-19, the runtime provider's required unauthenticated GitHub Releases endpoints were checked directly for:

```text
13thx-mcp/filesystem
13thx-mcp/git
13thx-mcp/exec
13thx-mcp/gateway
13thx-mcp/blender
13thx-mcp/studio
13thx-mcp/fleet
```

Every `/releases/latest` request returned HTTP 404.

At that time this blocked two release-grade proofs:

1. clean source-less bootstrap/update using published project-owned release artifacts rather than copied deployed fixtures;
2. final `v0.5.0` bump/tag/publication verification and artifact/checksum download from the authoritative release endpoints.

The official OpenAI tunnel release path is not blocked; `v0.0.14` discovery/staging/force-reinstall passed against the live official release.

## Exit-criteria matrix

| M5 exit criterion | Status | Evidence / blocker |
| --- | --- | --- |
| 1. Source-less host operates without source/build toolchains | PASS | isolated runtime-only fixture + real Gateway/children |
| 2. Trusted installed/running/desired/latest identities | PASS | inventory + UI tests/smoke |
| 3. Project/OpenAI providers policy-confined | PASS | provider security tests plus qualified published project/OpenAI release evidence |
| 4. Download/checksum/safe extraction | PASS | staging/tamper tests + official Tunnel smoke |
| 5. Ordinary MCP update + rollback | PASS | running/stopped/rollback drills |
| 6. Gateway reconnect/catalog + rollback | PASS | protocol/catalog + failure rollback drills |
| 7. Fleet update preserves host profile/config | PASS | Fleet transactional tests |
| 8. Runtime reconciliation + rollback | PASS | incident-derived closure drill |
| 9. Exact Gateway tool identity/fingerprint | PASS | exact names/fingerprint + refresh_pending proof |
| 10. Studio external self-update + rollback | PASS | success + health-failure launcher smokes |
| 11. Updates/Fleet UI | PASS | 26 frontend tests + reconnect model |
| 12. Host-local config/secrets/state preserved | PASS | Fleet/Studio/Tunnel/reconciliation proofs |
| 13. No unattended auto-update/reconciliation | PASS | no scheduler/policy exposed |
| 14. Full backend/frontend/security/release gates | PASS | all local gates green |
| 15. Real end-to-end source-less smoke + fault injection | PASS | runtime-only/fault drills plus published-artifact source-less bootstrap qualified 2026-09-21 |
| 16. `v0.5.0` artifacts reproducible/publishable | PASS | published darwin-arm64 project artifacts/checksums and Studio v0.5.0 bootstrap evidence qualified 2026-09-21 |

## Publication-unblock sequence — COMPLETED 2026-09-21

The previously required sequence was completed by the publication qualification runner: stable project releases/checksums were available, release metadata was verified, a fresh source-less runtime was bootstrapped from published artifacts plus the official Tunnel runtime, and Fleet was updated from published 0.2.0 to 0.2.1. M5 is **PUBLICATION QUALIFIED** and may be used as the qualified predecessor for M6.
