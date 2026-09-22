# Workspace Skill Runtime — Post-M7 Design Track

**State:** DESIGN PROPOSAL / NOT IMPLEMENTED
**Date:** 2026-09-22
**Primary repos:** gateway, studio; supporting primitives remain in filesystem, git/exec, fleet and specialized MCPs.
**Milestone assignment:** intentionally not frozen. The existing Studio roadmap already assigns M8 to Hardening, Auto-Update & Recovery.

## Goal

Allow ChatGPT or another MCP client connected through the private secure tunnel to discover and invoke reusable Aira Workspace skills without turning skill prose, scripts or packages into unrestricted execution authority.

The target path is:

```text
ChatGPT / MCP client
        |
        | MCP over Secure Tunnel
        v
Aira Gateway
  |-- primitive typed MCP tools
  |
  +-- skill.search
  +-- skill.describe
  +-- skill.invoke
  +-- skill.status
        |
        v
Skill Registry + Runtime
        |
        v
Capability / Policy Resolver
        |
        v
Existing typed MCP primitives
(filesystem, git/exec, fleet, hardware, ...)
```

Studio remains the operator-facing control plane for skill inventory, compatibility, enable/disable state, policy projection, distribution status and evidence/history. Gateway remains the remote request execution and safety boundary.

## Frozen design principles

1. **Skill != MCP server.** A skill is a versioned declarative workflow/capability package.
2. **Skill != trusted executable authority.** Manifest declarations request capabilities; active server policy grants or denies them.
3. **Gateway remains the invocation boundary.** Remote skill requests must pass through M7 admission, classification, concurrency, cancellation and outcome semantics.
4. **Typed MCP primitives remain authoritative.** Skills compose primitives rather than replacing Filesystem/Git/Exec/Fleet/hardware boundaries with a broad worker.
5. **No arbitrary shell expansion.** A skill cannot turn prose into `shell -c` or bypass structured argv/tool contracts.
6. **No implicit mutation replay.** Unknown-outcome mutation semantics from M7 apply to a whole skill invocation and to each mutation step.
7. **Small public surface.** Prefer generic `skill.search/describe/invoke/status` over one tool per skill.
8. **Explicit workspace context.** A skill invocation must identify the workspace/project context using a server-validated reference, never an arbitrary host path.
9. **Evidence, not hidden state.** Invocation results should return bounded structured evidence. M6 SQLite may store sanitized historical projections but never becomes live execution authority.
10. **Operator policy wins.** Disabled skills, denied capabilities, profile restrictions, drain state and destructive-operation approvals fail closed.

## Initial package model

Recommended source layout:

```text
skills/
  <skill-id>/
    SKILL.md
    manifest.yaml
    schemas/
      input.json
      output.json
    references/
    assets/            # optional, bounded and validated
```

Executable assets are optional and require an explicit later execution model. The first implementation should prefer orchestration of existing typed MCP tools over arbitrary embedded executables.

A normalized manifest should cover:

```yaml
schema_version: 1
id: rust.release.qualify
version: 1.0.0
description: Qualify a Rust workspace release.

workspace:
  kinds: [rust]

risk: workspace-write

requires:
  - filesystem.read
  - git.read
  - command.execute

denies:
  - git.push
  - git.force-push
  - secret.read
  - network.public

bounds:
  timeout_ms: 900000
  max_steps: 64

approval:
  destructive: required

evidence:
  - tests
  - lint
  - build
  - git-status
```

The exact schema is not frozen by this proposal; ADR 0036 freezes the authority model only.

## Repository responsibility split

| Domain | Owner | Responsibility |
| --- | --- | --- |
| External MCP skill tools | Gateway | search/describe/invoke/status contract |
| Request admission / concurrency | Gateway | M7 request class, limits, drain, cancellation, outcome |
| Skill registry/runtime | Gateway initially | validated registry, invocation state, step orchestration |
| Skill inventory / policy UI | Studio | operator visibility and administrative policy surface |
| Historical invocation evidence | Studio/M6 history | sanitized projections only |
| Skill package distribution | Studio/Fleet later | verified release/install/reconcile |
| Primitive filesystem authority | Filesystem MCP | confined read/write/search/CAS/patch |
| Primitive Git authority | Git MCP | typed Git operations and publication safety |
| Process/test authority | Exec/workspace MCP | structured executable + argv under server policy |
| Host/device operations | Fleet/specialized MCPs | typed host/hardware authority |

## Public MCP contract sketch

### skill.search

Input:

```json
{
  "query": "audit rust release readiness",
  "workspace": "studio"
}
```

Result includes only server-approved discoverable skills, compatibility and risk metadata.

### skill.describe

Input:

```json
{
  "skill_id": "rust.release.qualify",
  "version": "1.0.0"
}
```

Result includes normalized inputs/outputs, capabilities, side effects, approval requirements, bounds and evidence contract.

### skill.invoke

Input:

```json
{
  "skill_id": "rust.release.qualify",
  "workspace": "studio",
  "arguments": {
    "target": "current_branch",
    "remediate": true
  },
  "execution": {
    "dry_run": false
  }
}
```

The caller never supplies host executable paths, shell command strings, arbitrary capability grants or policy overrides.

### skill.status

Returns live/terminal state for a server-generated invocation id. Persisted M6 history is not sufficient to resume or authorize execution.

## Risk classes

Minimum proposed classes:

- `read` — no externally visible mutation;
- `workspace-write` — confined file/code mutation;
- `runtime-mutation` — process/config/runtime lifecycle changes;
- `remote-write` — push/publish/external state mutation;
- `deploy` — runtime/device deployment;
- `hardware` — physical device/GPIO or equivalent side effects;
- `destructive` — data loss/reset/replacement class.

The Gateway may map these onto M7 request classes, but callers cannot choose or downgrade the class.

## Non-goals

- no general-purpose remote agent shell;
- no one-MCP-per-skill requirement;
- no browser endpoint that uploads arbitrary executable workflows;
- no skill capability inferred from natural-language instructions;
- no authorization based on chat/session identity alone;
- no persistent mutation replay queue;
- no bypass of active Gateway profiles or primitive MCP safety rules;
- no automatic Git push, release publication, deploy or hardware actuation without explicit policy/approval.

## Documents

- [ADR 0036 — Workspace Skill Runtime and MCP Invocation Boundary](../../adr/0036-workspace-skill-runtime-mcp-boundary.md)
- [Security and Trust Model](SECURITY-MODEL.md)
- [Implementation Roadmap](IMPLEMENTATION-ROADMAP.md)
