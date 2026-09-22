# ADR 0036: Workspace Skill Runtime and MCP Invocation Boundary

- Status: Proposed
- Date: 2026-09-22

## Context

Aira Workspace increasingly needs reusable workflows that compose existing typed MCP capabilities such as filesystem, Git, execution, Gateway, Fleet and hardware-oriented MCPs. ChatGPT may reach the private Aira Gateway through the supported secure tunnel, but network reachability alone does not provide a safe workflow execution contract.

Representing every workflow as a separate MCP server would multiply process, configuration, release, authentication, discovery and audit surfaces. Letting a skill directly expand into arbitrary shell commands would bypass the M7 request coordination, tool classification, concurrency, unknown-outcome and audit boundaries.

The design therefore needs to distinguish:

- primitive typed MCP tools, which remain the authority boundary for concrete operations;
- reusable skills, which describe and coordinate higher-level workflows;
- the Gateway, which remains the external MCP invocation and request-safety boundary;
- Studio, which manages operator-visible skill inventory, policy and lifecycle without becoming a generic remote executor.

## Decision

Introduce a Workspace Skill Runtime as a capability-composition layer behind the Aira Gateway.

A skill is **not** an MCP server and is **not** trusted executable authority. A skill is a versioned declarative capability package containing instructions, a validated manifest, declared dependencies/capabilities, optional confined assets, and an input/output contract.

The external MCP surface should remain intentionally small:

- `skill.search` — discover skills by intent/capability and workspace compatibility;
- `skill.describe` — return the normalized server-owned contract, required capabilities, side-effect/risk class and preconditions;
- `skill.invoke` — invoke one validated skill against an explicit workspace context;
- `skill.status` — inspect live/terminal invocation state without turning persisted history into live authority.

Primitive deterministic operations such as filesystem read/write, Git operations, tests, release qualification and Gateway control remain first-class MCP tools. Skills compose those primitives; they do not replace them.

Every skill invocation must pass through Gateway-owned request classification, bounded admission/concurrency, cancellation/outcome semantics, capability/policy resolution and audit/evidence projection. A skill may not bypass Gateway to gain broader filesystem, process, network, Git, Fleet or hardware authority than its manifest and active server policy allow.

Studio owns the operator control-plane view of skills: inventory, versions, validation status, enable/disable state, compatibility, policy projection, evidence/history views and future trusted distribution. Studio does not accept raw skill-provided shell commands from the browser and does not execute a skill as an unbounded worker.

The initial manifest contract should declare at least:

- stable skill id and semantic version;
- human-readable description;
- supported workspace/project classes;
- typed input/output schema references;
- required primitive capabilities;
- side-effect/risk class;
- allowed resource/path scope where applicable;
- explicitly denied high-risk capabilities;
- timeout/resource bounds;
- approval requirements;
- evidence produced;
- optional executable assets plus their confinement and integrity requirements.

Skill packages are server-validated before becoming discoverable. Runtime policy is authoritative over manifest requests: a manifest can request capability but cannot grant it.

## Alternatives considered

### One MCP server per skill

Rejected as the default because it causes tool/server proliferation, repeated lifecycle/configuration/release surfaces, larger model tool catalogs and fragmented audit/policy enforcement. Dedicated MCP servers remain appropriate only when a capability is independently deployable and needs its own typed authority boundary.

### Let ChatGPT read SKILL.md and execute the steps directly

Rejected for mutation-capable workflows. Instructions may be exposed as resources or documentation, but translating arbitrary skill prose into unrestricted execution would bypass M7 tool-safety and unknown-outcome guarantees.

### Put the Skill Runtime entirely inside Studio

Rejected. Studio is the local operator control plane and release/runtime manager. Gateway already owns remote MCP request admission, classification, concurrency and dispatch, so externally invoked skill execution belongs behind Gateway. Studio may manage and display the registry/policy state but is not the request execution boundary.

### Expose only primitive tools and keep no skill abstraction

Safe but inefficient for repeated multi-step workflows. It forces clients to rediscover orchestration logic, increases token and repair cost, and makes workflow-level evidence and policy harder to standardize.

## Consequences

Positive effects:

- preserves typed MCPs as concrete capability/security boundaries;
- keeps the public skill MCP surface small even with many skills;
- reuses M7 scheduling, drain, cancellation, no-replay and audit semantics;
- supports semantic discovery without injecting hundreds of tools into the client catalog;
- gives Studio a coherent inventory/policy/evidence surface;
- enables versioned reusable workflows without making prose or scripts implicit authority.

Trade-offs and follow-up work:

- requires a normalized manifest schema and registry lifecycle;
- requires capability resolution between skills and primitive tools;
- requires invocation/evidence IDs that remain distinct from historical SQLite authority;
- requires package integrity/version compatibility rules before distributed skill installation;
- requires explicit approval semantics for destructive, remote-write, deploy and hardware operations;
- requires tests proving skills cannot expand authority beyond active Gateway/tool policy;
- requires a later decision on whether long-running skill orchestration is in-process, delegated to a bounded worker, or represented as resumable state without violating M7 no-replay guarantees.
