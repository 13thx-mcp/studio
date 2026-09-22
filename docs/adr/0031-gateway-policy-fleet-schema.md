# ADR 0031 — Gateway Global Policy Surface and Fleet Schema Migration

- **Status:** Accepted for M7.1/M7.6 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-07 global Gateway policy, tool classification/profile policy, Fleet rendering, Studio reconciliation and downgrade safety

## Context

Fleet host schema v1 renders only `runtime/gateway/servers.d/*.yaml`. Those child files currently combine launch information with basic exposure/timeout/restart fields.

M7 requires global scheduler/drain/payload/profile policy plus per-child/per-tool concurrency/classification/restart policy. Adding these fields directly to child YAML would make the existing Gateway 0.1.x parser reject the files because child configuration denies unknown fields. It would also make downgrade behavior unsafe and entangle launch authority with coordination policy.

## Decision

M7 introduces a separate Fleet-managed global policy file:

```text
runtime/gateway/
  gateway.yaml
  servers.d/
    *.yaml
```

The existing child files remain the launch/executable authority. `gateway.yaml` is coordination/policy authority.

Gateway 0.2.0 derives the policy path without changing the trusted tunnel launcher:

```text
--config-dir <runtime>/gateway/servers.d
policy path = <runtime>/gateway/gateway.yaml
```

No browser/client-supplied policy path is accepted.

If `gateway.yaml` is absent, Gateway 0.2.0 enters explicit `legacy_compat` mode: it preserves the 0.1.x child-routing behavior needed for staged migration but reports that M7 strict policy is inactive. Studio/Fleet must not report M7 strict readiness while this mode is active.

Gateway target version: **0.2.0**.
Fleet target version: **0.3.0**.
Studio M7 target remains **0.7.0-beta**.

## Gateway policy schema v1

```yaml
schema_version: 1
active_profile: develop

limits:
  global_active: 16
  global_queue: 64
  queue_wait_ms: 30000
  default_child_active: 4

tool_class_defaults:
  read:
    concurrency: 4
  mutation:
    concurrency: 1
  long-running:
    concurrency: 1
  control:
    concurrency: 1

drain:
  deadline_ms: 60000
  allow_safe_reads: false

payload:
  request_bytes: 1048576
  response_bytes: 2097152
  text_preview_bytes: 65536
  structured_bytes: 1048576
  binary_bytes: 1048576

artifacts:
  enabled: true
  ttl_seconds: 900
  max_item_bytes: 8388608
  max_total_bytes: 67108864

profiles:
  inspect: {}
  develop: {}
  release: {}
  ops: {}
  hardware: {}

children:
  filesystem:
    concurrency: 4
    restart:
      policy: on-failure
      max_attempts: 3
      stability_window_ms: 30000
      backoff:
        initial_ms: 500
        max_ms: 30000
      circuit:
        cooldown_ms: 60000
        half_open_attempts: 1
    tools:
      read_text_file:
        class: read
        profiles: [inspect, develop]
        concurrency: 4
      write_text_file:
        class: mutation
        profiles: [develop]
        concurrency: 1
```

## Validation

The policy parser fails closed on:

- unknown top-level fields;
- unsupported `schema_version`;
- zero/overflowing limits;
- unknown tool class;
- duplicate profile names;
- tool concurrency greater than the global active bound;
- unknown child/tool references after child catalog discovery;
- profile references to undeclared profiles;
- a profile/tool rule that attempts to expose a tool excluded by the child launch allowlist.

Tool names in policy are original child tool identities before any Gateway prefix transformation.

Clients cannot provide or override tool class/profile/concurrency policy.

## Fleet host schema v2

Fleet host schema increments from 1 to 2.

Minimum shape:

```toml
schema_version = 2

[gateway]
server_dir = "gateway/servers.d"
policy_file = "gateway/gateway.yaml"

[gateway.policy]
active_profile = "develop"
global_active = 16
global_queue = 64
queue_wait_ms = 30000
default_child_active = 4
drain_deadline_ms = 60000
allow_safe_reads = false

[servers.filesystem]
enabled = true
timeout_ms = 30000
tool_allowlist = []

[servers.filesystem.policy]
concurrency = 4

[servers.filesystem.policy.tools.read_text_file]
class = "read"
profiles = ["inspect", "develop"]
concurrency = 4
```

Fleet may keep TOML ergonomics different from the final YAML representation, but render output is deterministic and semantically equivalent to Gateway policy schema v1.

## Render surface

Fleet `render-plan` adds:

```text
surface: gateway.policy
relative_path: gateway/gateway.yaml
effects: [gateway_reload]
ownership: fleet_managed
```

Managed output set becomes:

- `gateway/gateway.yaml`;
- `gateway/servers.d/*.yaml`;
- Studio config;
- tunnel config.

The policy output participates in content hashes, reconciliation snapshots, rollback, drift classification and post-write verification exactly like existing Fleet-managed surfaces.

## Compatibility and migration

Fleet 0.3.0 supports reading host schema v1 and v2 during migration.

For host schema v1:

- existing child launch files render byte-compatible with the legacy form;
- Fleet may render a safe default `gateway.yaml` only when the target Gateway is known to support policy schema v1;
- otherwise migration remains planning-only and activation is blocked rather than silently changing policy.

Host schema v2 requires a Gateway artifact declaring support for `gateway_policy_schema = 1`.

## Downgrade guard

Because Gateway 0.1.x ignores `gateway.yaml`, allowing it to run after M7 policy activation would silently drop concurrency/profile/drain safety.

Therefore Studio/Fleet must block activation/downgrade to a Gateway that does not declare support for the active Gateway policy schema.

The guard is capability based, not a filename heuristic or semver guess.

Gateway 0.2.0 adds a side-effect-free staging probe:

```text
rust-mcp-gateway --capabilities-json
```

Minimum output:

```json
{
  "gateway_policy_schemas":[1],
  "control_protocol_versions":[1]
}
```

Studio/Fleet use this probe on staged artifacts before activation. Missing/malformed capability output or absence of the active schema blocks activation/downgrade.

Child launch YAML intentionally avoids new M7-only fields, so rollback tooling can still inspect/restore old launch files without parse ambiguity.

## Studio reconciliation

Studio expands its closed managed-surface set to include `gateway.policy`.

A reconciliation transaction:

1. obtains deterministic Fleet render for all managed surfaces;
2. snapshots current managed bytes;
3. writes all intended changes;
4. requests Gateway reload through ADR 0027;
5. verifies returned policy/catalog generation/fingerprint;
6. commits the managed-state manifest only after runtime verification;
7. compensates all changed surfaces on failure.

Fixed watcher sleeps are no longer proof that policy is active.

## Profile/classification completeness

Before M7 strict mode becomes default, every exposed child tool must resolve to a server-owned class.

Unknown classification fails closed by default: the tool is not routable until classified. There is no caller-selected fallback class.

Profiles only reduce the child launch allowlist; they never expand it.

## Verification requirements

Permanent Fleet/Gateway/Studio tests must prove:

- deterministic `gateway.yaml` render;
- host schema v1 migration behavior is explicit and safe, including `legacy_compat` status;
- host schema v2 rejects unsupported/unknown policy;
- `gateway.policy` appears in render-plan and reconciliation snapshots;
- rollback restores both policy and child launch surfaces;
- staged capability probe blocks old/incompatible Gateway activation when policy schema v1 is active;
- unknown child/tool/profile references fail closed;
- profile cannot exceed child launch allowlist;
- tool classification comes from trusted configuration only.

## Consequences

- Launch authority and coordination policy are separate.
- Fleet gains one new managed surface instead of expanding every child launch schema.
- Gateway 0.2.0 is an explicit compatibility boundary.
- Downgrade cannot silently disable M7 safety.
