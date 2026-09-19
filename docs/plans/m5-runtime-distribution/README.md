# M5 Runtime Distribution — Session Submission Index

Use this directory to implement M5 across multiple ChatGPT sessions without carrying the whole project context in one prompt.

## Session rule

For every new session:

1. Work only on Aira unless the active submission explicitly says otherwise.
2. Read `COMMON.md` first.
3. Read only the active `M5.x.md` file plus any source/docs it references.
4. Inspect the current code before implementation; the files here describe intent, not a substitute for current source truth.
5. Complete the active submilestone end-to-end including tests, smoke verification, docs, Git branch/commit/merge/push when appropriate.
6. Do not start the next submilestone in the same session unless explicitly requested.

## New-session starter prompt

Use this for M5.1 first:

```text
@Aira Workspace continue MCP Studio M5 — Runtime Distribution, Update Manager & Fleet State.

Read these files first:
1. mcp-server/studio/docs/plans/m5-runtime-distribution/COMMON.md
2. mcp-server/studio/docs/plans/m5-runtime-distribution/M5.1-component-models.md

Then inspect the current Studio/Fleet code relevant to M5.1 and implement that submilestone completely.

Requirements:
- Treat the current repository/code as source of truth if it differs from planning notes.
- Work only on Aira. Do not modify Mirin.
- Preserve the current root-project + flat MCP bin + runtime layout.
- Follow the Git/quality/security workflow in COMMON.md.
- Do not begin M5.2 until M5.1 acceptance criteria are fully verified.
- At completion, provide the handoff required by COMMON.md.
```

For later sessions, replace the second file and submilestone number, for example:

```text
@Aira Workspace continue MCP Studio M5.
Read:
- mcp-server/studio/docs/plans/m5-runtime-distribution/COMMON.md
- mcp-server/studio/docs/plans/m5-runtime-distribution/M5.7-mcp-transactional-update.md

Implement M5.7 completely, verify it, merge/push when ready, and stop before M5.8.
```

## Submission order

| Submission | File | Outcome |
|---|---|---|
| M5.1 | `M5.1-component-models.md` | Stable release/provider/component/update domain models |
| M5.2 | `M5.2-13thx-release-provider.md` | Read-only `13thx-mcp` GitHub Release discovery/asset metadata |
| M5.3 | `M5.3-tunnel-release-provider.md` | Official OpenAI tunnel-client provider integrated behind common abstraction |
| M5.4 | `M5.4-platform-resolver.md` | Strict OS/arch normalization and supported asset selection |
| M5.5 | `M5.5-staging-integrity.md` | Download, checksum, safe extraction and immutable staging layer |
| M5.6 | `M5.6-inventory-drift-api.md` | Installed/available/running/desired inventory + drift API |
| M5.7 | `M5.7-mcp-transactional-update.md` | Safe flat-bin MCP update + rollback transaction |
| M5.8 | `M5.8-gateway-update.md` | Gateway-specific update/reconnect/catalog verification |
| M5.9 | `M5.9-fleet-update.md` | Fleet bundle update preserving host profile/config |
| M5.9A | `M5.9A-runtime-reconciliation.md` | Runtime config drift detection, transactional repair and exact Gateway catalog verification |
| M5.10 | `M5.10-studio-self-update.md` | Staged Studio self-update + external supervisor contract |
| M5.11 | `M5.11-updates-ui.md` | Updates/Fleet UI + realtime progress/error surfaces |
| M5.12 | `M5.12-runtime-only-smoke.md` | Source-less host bootstrap/update/rollback proof and M5 closure |

## Dependency graph

```text
M5.1
 ├─ M5.2
 ├─ M5.3
 └─ M5.4
      ↓
     M5.5
      ↓
     M5.6
      ↓
     M5.7
      ↓
     M5.8
      ↓
     M5.9
      ↓
    M5.9A
      ↓
    M5.10
      ↓
    M5.11
      ↓
    M5.12
```

M5.2–M5.4 may share models from M5.1, but execute them sequentially unless a later implementation review explicitly changes the order.

## M5 release rule

Do not create the Studio `v0.5.0` release merely because an intermediate submilestone passes. M5 release preparation occurs only after M5.12 closure verifies the complete source-less runtime workflow and all roadmap exit criteria.
