# Workspace Skill Runtime — Security and Trust Model

## Trust rule

A skill package is data until validated and admitted. Neither `SKILL.md`, `manifest.yaml`, optional scripts/assets nor model interpretation can grant host authority.

```text
client intent
   -> Gateway skill tool
   -> server-owned skill registry
   -> manifest validation
   -> active profile/policy
   -> capability resolver
   -> M7 admission/concurrency
   -> typed primitive MCP call
   -> bounded evidence
```

## Threats and required controls

### Capability escalation

**Threat:** a skill declares broad capabilities or attempts to call tools not declared in its manifest.

**Controls:** server policy intersects requested capabilities with the active Gateway/tool profile. Every runtime step is checked against the resolved capability set. Undeclared or denied calls fail closed.

### Prompt/instruction escalation

**Threat:** `SKILL.md` contains instructions that tell the client/runtime to bypass policy, call raw shell, read secrets or use a different workspace.

**Controls:** instructions are non-authoritative metadata. Only normalized manifest + server policy may authorize tools/resources. Natural-language content never changes capability grants.

### Arbitrary process execution

**Threat:** a skill smuggles `shell -c`, interpreter eval, executable path or unbounded argv into a generic worker.

**Controls:** first implementation should orchestrate existing typed tools only. If executable assets are later supported, executable identity, argv schema, cwd, environment, integrity, timeout and path confinement must be server-owned and separately reviewed.

### Workspace escape

**Threat:** skill input names an absolute path, traversal path, symlink escape or a different workspace.

**Controls:** invocation uses a server-resolved workspace identifier/context. Primitive MCPs keep their existing confinement and CAS/path rules. Raw host paths are not an external skill input.

### Mutation replay / unknown outcome

**Threat:** client timeout causes the same workflow or mutation step to be replayed.

**Controls:** reuse M7 request identity/outcome semantics. Mutation-capable invocations are never automatically replayed after dispatch when terminal outcome is unknown. Recovery begins with status/reconciliation, not blind retry.

### Tool explosion / schema poisoning

**Threat:** thousands of skills become MCP tools or untrusted packages inject tool schemas/descriptions into the global catalog.

**Controls:** expose a fixed generic skill tool set. Search results are bounded. Only validated normalized server-side metadata is returned.

### Malicious package/update

**Threat:** a skill package is replaced, downgraded or modified after validation.

**Controls:** future distribution must use immutable versioned packages, content digests and trusted release/install policy. Invocation binds to an exact validated skill version/digest. Auto-update remains separate policy.

### Secret exfiltration

**Threat:** skill reads credentials or returns sensitive tool output as evidence.

**Controls:** secret capabilities are denied by default. Evidence is typed, bounded and sanitized before persistence or client return. M6 history never stores raw arguments/results by default.

### Approval bypass

**Threat:** skill decomposes a destructive action into lower-level calls to avoid approval.

**Controls:** approval is capability/risk based as well as skill based. Primitive high-risk actions retain their own gates. A skill-level approval cannot waive a stricter primitive-tool requirement.

### Concurrent conflicting workflows

**Threat:** two skills mutate the same workspace/runtime concurrently.

**Controls:** map skill invocations onto M7 scheduler/resource classes and later add explicit resource leases where required. A skill cannot self-declare a lower concurrency class.

## Required invariants

1. Caller cannot grant a capability.
2. Skill content cannot grant a capability.
3. Studio history cannot grant or resume live authority.
4. Gateway profile/policy can always deny a skill or primitive.
5. Primitive MCP validation remains effective even when invoked from a skill.
6. Unknown-outcome mutation is never treated as failed/no-op proof.
7. A terminal skill result must identify which mutations are confirmed, failed or unknown.
8. Remote-write/deploy/hardware/destructive operations require explicit policy and auditable approval semantics.
9. Browser/admin APIs never accept arbitrary executable or shell representations.
10. Skill disable/revocation prevents new invocation without rewriting historical evidence.

## Threat-model integration

Before implementation, update `docs/threat-model.md` with the new trust boundary and add release-gate tests for capability escalation, workspace escape, replay/unknown outcome, package tamper, evidence redaction and concurrent mutation conflicts.
