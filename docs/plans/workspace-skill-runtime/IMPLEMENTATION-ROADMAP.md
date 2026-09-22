# Workspace Skill Runtime — Implementation Roadmap

**Status:** proposed; implementation must not begin until ADR 0036 is accepted and M7 closure remains green.

## Phase S0 — Contract and source audit

Owners: Studio + Gateway.

Deliverables:

- inventory existing tool/profile/request classification contracts;
- inventory any existing skill conventions or `SKILL.md` packages;
- freeze terminology: skill, capability, invocation, evidence, approval, workspace context;
- define schema versioning and compatibility rules;
- define where registry data lives and which process owns live state;
- define exact interaction with M7 request classes and unknown-outcome semantics.

Exit gate: no skill path can bypass typed MCP authority or create a second request scheduler.

## Phase S1 — Manifest and registry

Primary owner: Gateway; Studio consumes safe projections.

Implement:

- schema-v1 manifest parser/validator;
- stable skill id + semantic version;
- capability/risk/evidence declarations;
- workspace compatibility;
- enable/disable and active-profile filtering;
- deterministic registry snapshot;
- package digest binding;
- bounded search metadata.

No executable skill assets in S1.

Verification:

- malformed/unknown critical fields fail closed;
- duplicate id/version rejected;
- capability names resolved from a server-owned catalog;
- disabled/non-compatible skills are not invokable;
- manifest cannot widen policy.

## Phase S2 — Read-only discovery API

Primary owner: Gateway.

Implement:

- `skill.search`;
- `skill.describe`;
- bounded result count/metadata;
- exact version resolution;
- safe resource/document exposure when useful.

Verification:

- search is read-only and bounded;
- untrusted prose cannot alter returned authority metadata;
- hidden/profile-denied skills do not leak sensitive implementation details.

## Phase S3 — Invocation core

Primary owner: Gateway.

Implement:

- `skill.invoke`;
- `skill.status`;
- server-generated invocation id;
- step state machine;
- bounded max steps/time/output;
- capability check before every primitive call;
- cancellation propagation;
- M7 request classification/admission integration;
- confirmed/failed/unknown mutation outcome aggregation.

Recommended initial execution model: declarative orchestration of existing typed MCP tools only.

Verification:

- no raw shell path;
- no undeclared tool call;
- no automatic replay after unknown mutation outcome;
- cancellation does not misreport side-effect absence;
- conflicting mutations respect Gateway scheduling/resource policy.

## Phase S4 — Studio operator surface

Primary owner: Studio.

Implement safe read/admin projections:

- skill inventory;
- exact version/digest;
- validation status;
- enabled/disabled state;
- workspace compatibility;
- required capabilities/risk;
- active profile visibility;
- recent invocation evidence/history linkage.

Administrative mutation must remain narrow and same-origin protected. Do not expose arbitrary manifest text editing as execution authority.

Verification:

- browser DTOs omit internal paths/secrets;
- UI cannot invent capability grants;
- history is clearly non-authoritative for live execution.

## Phase S5 — Evidence and audit integration

Owners: Gateway + Studio/M6 history.

Implement:

- typed invocation admitted/started/step/terminal events;
- sanitized skill/version/workspace/risk metadata;
- evidence references rather than unbounded raw output;
- mutation outcome classification;
- metrics for latency, queueing, cancellation, policy denial and unknown outcomes.

Verification:

- replay-safe history event identity;
- no raw secret-bearing arguments/results by default;
- live invocation authority remains in Gateway only.

## Phase S6 — Trusted distribution

Owners: Studio + Fleet.

Only after S1-S5 are stable:

- trusted skill package source;
- package checksum/digest verification;
- versioned install directories;
- compatibility validation;
- atomic activation/rollback;
- Fleet desired-state rendering/reconciliation;
- source-less host support.

Do not permit browser-supplied package URLs, install paths or arbitrary providers.

## Phase S7 — Privileged skills

Examples: release publication, deployment, device/hardware operations.

Prerequisites:

- explicit approval contract;
- capability-specific policy;
- audit evidence;
- idempotency/reconciliation contract;
- resource lease/locking where required;
- failure and unknown-outcome recovery runbook.

Privileged skills should be added individually with dedicated threat review; a generic "allow privileged" switch is not sufficient.

## Qualification matrix

At minimum:

| Area | Required proof |
| --- | --- |
| Manifest | fuzz/negative schema and capability validation |
| Discovery | bounded search; hidden/disabled/profile-denied filtering |
| Invocation | read/write workflows, cancellation, timeout, unknown outcome |
| Authority | caller/skill cannot self-grant capability |
| Workspace | traversal/symlink/cross-workspace denial |
| Concurrency | conflicting mutation workflows serialize/reject as designed |
| Evidence | bounded/sanitized results; history non-authoritative |
| Distribution | tamper/downgrade/rollback/source-less host tests |
| UI | no raw execution authority exposed to browser |
| Regression | full M7 coordination/tool-safety qualification stays green |

## Recommended first production skills

Start with deterministic, evidence-heavy workflows rather than deployment:

1. Rust quality qualification (fmt/clippy/test/build/audit as policy permits).
2. Release-readiness inspection with optional confined remediation.
3. Repository/status evidence collection.
4. Workspace architecture/review checklist execution.

Delay remote push/publication, deployment and hardware actuation until approval/resource/outcome contracts are proven.
