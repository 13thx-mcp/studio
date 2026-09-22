# M7 Common Contracts — Draft Normative Freeze

**Status:** DRAFT. Becomes normative only after M7.0 review/acceptance.

## 1. Authority map

- Gateway in-memory coordination state is authoritative for live request admission, queueing, dispatch, drain and child circuit state.
- Typed child MCPs remain authoritative for their domain side effects.
- Studio/Fleet remain authoritative for version, verified artifact activation, rollback, desired configuration and operator lifecycle policy.
- M6 SQLite is authoritative only for committed historical records/projections; it never dispatches, retries, resumes or authorizes requests.
- Workspace/session aliases are not part of M7. Explicit target MCP roots/capabilities remain authoritative.
- Request/correlation IDs are observability identity only, never authorization or idempotent replay authority.

## 2. Request lifecycle

Minimum state machine:

```text
RECEIVED
  -> ADMITTED
  -> QUEUED
  -> DISPATCHED
  -> COMPLETED
```

Terminal/exceptional outcomes:

```text
REJECTED_POLICY
REJECTED_CAPACITY
CANCELLED_BEFORE_DISPATCH
DEADLINE_EXPIRED_BEFORE_DISPATCH
CHILD_ERROR
RESPONSE_REJECTED
OUTCOME_PENDING
OUTCOME_UNKNOWN
COMPLETED
```

Invariants:

- Before `DISPATCHED`, cancellation/deadline expiry guarantees the child was not called.
- The `DISPATCHED` transition is recorded atomically in Gateway coordination state before handing the call to the child.
- After `DISPATCHED`, cancellation/deadline/disconnect never claims rollback.
- Mutations are never automatically replayed after dispatch failure, timeout, cancellation or disconnect.
- A caller may receive `OUTCOME_PENDING` while Gateway continues bounded terminal observation.
- `OUTCOME_UNKNOWN` means Gateway can no longer prove the child outcome; it is not equivalent to failure.
- Retry guidance for unknown mutation outcome must explicitly require state inspection/reconciliation before another mutation.
- Reads may have separate retry advice, but the server must not silently replay them inside the same request unless the exact policy is frozen and observable.

## 3. Request identity and metadata

Generate a server-owned opaque `request_id` for every accepted request. Normalize or generate a `correlation_id`; never trust caller-provided identity as authorization.

Active registry fields:

```text
request_id
correlation_id
child
tool
tool_class
received_at
admitted_at
queued_at
dispatched_at
completed_at
deadline
state
outcome
catalog_generation
profile_generation
```

Do not retain raw tool arguments/results in the active registry after terminal handling unless explicit bounded debug mode is enabled.

## 4. Scheduler and backpressure

ADR 0028 freezes one bounded coordination scheduler rather than unbounded spawned semaphore waiters.

M7.1 implementation defaults:

```text
global_active = 16
global_queue = 64
queue_wait_ms = 30000
per_child_active = 4

per_tool default:
  read         = 4
  mutation     = 1
  long-running = 1
  control      = 1
```

An explicit trusted per-tool concurrency value overrides the class default. The effective dispatch limit remains the minimum of global, child and tool capacity; zero is invalid configuration.

Every admitted request receives a monotonically increasing scheduler sequence. Dispatch is work-conserving **oldest eligible** selection: scan in sequence order, dispatch the first request whose global/child/tool/policy/drain constraints are currently satisfied, and repeat while capacity remains. A blocked child/tool therefore cannot head-of-line block unrelated work, but once an older request is eligible no later eligible request may overtake it for the next compatible dispatch opportunity.

The finite queue deadline bounds cases where required capacity never becomes available. Cancellation or deadline observed before the atomic `DISPATCHED` reservation guarantees zero child invocation.

All queue structures are finite. Capacity exhaustion returns deterministic `REJECTED_CAPACITY`; queue expiry returns `DEADLINE_EXPIRED_BEFORE_DISPATCH`.

Tool class and concurrency policy come from trusted server configuration. Clients cannot self-declare a read to bypass mutation serialization.

The private ADR 0027 lifecycle control socket is outside normal scheduler permits so `status/drain/resume/reload` remains reachable during normal request saturation; that control server still requires its own finite connection/request bounds in M7.2.

## 5. Child-call ownership

Once dispatched, Gateway owns a bounded child-call observation task independent of the upstream response future.

The upstream wait and child execution are separate:

```text
dispatch
  -> owned child-call task continues
  -> upstream response wait may finish, timeout, cancel or disconnect
  -> active registry retains terminal-observation obligation
```

If protocol cancellation is propagated after dispatch, the result still remains pending/unknown until a trustworthy terminal child/protocol outcome is observed.

No unbounded detached tasks are allowed. Active call count, drain and shutdown include these owned observation tasks.

## 6. Drain

Gateway states:

```text
RUNNING -> DRAINING -> DRAINED -> RESTARTING/UPDATING -> RUNNING
```

Drain contract:

- records a server-owned reason and drain generation;
- rejects new mutations immediately once draining begins;
- safe-read admission during drain is explicit policy, not default assumption;
- queued mutations are cancelled-before-dispatch or held only if the requested drain mode explicitly permits it;
- waits for dispatched mutations to reach a known terminal state;
- may optionally wait for reads/long-running calls under policy;
- exposes active count, queued count, oldest request age and blocking request IDs (opaque);
- has a finite deadline;
- timeout fails closed and prevents destructive restart/update;
- never kills active mutation work merely to satisfy the deadline.

Studio update, Gateway reload, reconciliation-triggered reload and unsafe child restart must consume this contract.

## 7. Per-child runtime resilience

ADR 0029 freezes independent per-child runtime ownership.

Each child tracks a process-local monotonically increasing `runtime_generation`, config fingerprint, runtime state, restart episode, consecutive failures and optional `retry_at`.

Reload is a per-child diff:

- unchanged + healthy => reuse the exact generation/process;
- changed => child-scoped drain, validate a non-routable candidate, atomically promote that child's routes, retire old generation;
- added => validate then publish;
- removed/disabled => drain then retire;
- dead => classify outstanding outcomes first, then enter bounded recovery.

Healthy sibling generations do not change merely because another child reloads or fails.

Default recovery contract:

```text
max_attempts = 3
stability_window = 30s
backoff = 500ms, 1000ms, 2000ms (exponential; hard cap 30s)
circuit cooldown = 60s
half-open candidates = 1
```

A quick crash before the stability window does not reset the failure episode. Three failed automatic attempts open the circuit. Mutation calls with unknown outcome are never replayed on a replacement generation.

## 8. Configuration surfaces

ADR 0031 freezes the split:

```text
runtime/gateway/
  gateway.yaml        # coordination/policy authority
  servers.d/*.yaml    # child launch/executable authority
```

`gateway.yaml` schema v1 owns scheduler/drain/payload/profile policy plus per-child/per-tool coordination metadata. M7-only policy is not added directly to legacy child launch YAML.

Fleet host schema v2 renders a deterministic Fleet-managed `gateway.policy` surface. Studio reconciliation includes that surface in drift detection, snapshots, rollback and runtime verification.

Unknown schema fields, child/tool/profile references or tool classes fail closed. Tool policy keys use original child tool names before Gateway prefixing. Profile expansion never exceeds the child launch allowlist.

Gateway 0.2.0 derives `gateway.yaml` from the trusted config-dir parent. Missing policy is explicit `legacy_compat` migration mode, never M7 strict readiness. A host with active Gateway policy schema v1 cannot downgrade to a Gateway that does not declare support for that schema; staged support is checked with side-effect-free `--capabilities-json`.

## 9. Tool profiles

Initial names: `inspect`, `develop`, `release`, `ops`, `hardware`.

Profile selection is server/operator state. A normal client cannot switch profile unless an explicitly privileged control-plane path is later authorized.

Control-plane tools such as Gateway reload/enable-disable are excluded from the normal coding profile by default.

A profile change:

- increments `profile_generation`;
- atomically recomputes routable tools;
- emits `notifications/tools/list_changed`;
- records sanitized audit evidence;
- does not alter child executable capability.

## 10. Filesystem v2

ADR 0030 freezes Filesystem target 0.2.0.

```text
revision = "sha256:" + lowercase_hex(SHA-256(exact_file_bytes))
```

Frozen surface:

- `file_metadata`;
- revision-bearing `read_text_file`;
- byte-bounded `read_text_file_range`;
- literal bounded `search_text`;
- replacement `write_text_file` requiring `expected_revision` for existing targets;
- deterministic `patch_text_file` using sorted, non-overlapping UTF-8 byte-range edits.

Existing-file replacement without an expected revision fails closed. Revision mismatch returns typed `STALE_REVISION` without replacing content.

Same-path in-process mutations are serialized. Replacement preserves cap-std confinement and sibling-temp + sync + atomic rename, with a second destination revision check immediately before rename.

The contract explicitly does not overclaim kernel-level compare-and-swap against an arbitrary non-cooperating external process in the final check-to-rename instruction window.

## 11. Payload budgets and ephemeral artifacts

Gateway applies budgets independent of tunnel transport:

- request bytes;
- response bytes;
- structured content bytes;
- text preview bytes;
- binary/base64 bytes.

An oversized result returns bounded metadata + preview + narrowing guidance. It does not forward the full payload by default.

Optional spill artifacts:

- local private directory under runtime Gateway state;
- opaque random artifact ID plus content hash;
- TTL;
- per-artifact size limit;
- total disk limit;
- bounded retrieval/range;
- atomic creation;
- cleanup on expiry/startup;
- no default persistence into M6.

## 12. Resources, progress and workspace context

Resources are read-oriented and bounded. Aggregation preserves child identity and rejects/renames collisions according to a frozen deterministic rule.

Progress forwarding is informational. It must never mark a request committed or terminal.

Workspace aliases are deliberately absent from M7. There is no alias/session binding layer between Gateway routing and the target MCP. A future alias feature requires a separate ADR and must re-prove underlying MCP confinement.

## 13. M6 telemetry boundary

Gateway emits sanitized typed observations, not SQL and not raw payload blobs.

At minimum record:

- request/outcome counts;
- queue depth/wait;
- active calls;
- latency buckets/p50/p95 projections;
- cancellation/timeout/pending/unknown counts;
- child/tool opaque/catalog identity under policy;
- response size and guard action;
- restart/circuit events;
- catalog/profile generation/fingerprint.

History ingestion failure must not become authority to replay or roll back a Gateway child tool call.

## 14. Tunnel adapter

ADR 0032 freezes a dual-artifact release unit from one verified OpenAI tunnel-client release.

For the validated v0.0.14 target:

- `tunnel-client-runtime-cloudflared` remains the Studio-owned production daemon;
- full `tunnel-client` is co-installed for bounded diagnostics/preflight;
- both assets must match the same upstream release version/commit and checksum manifest;
- `runtimes connect` does not take ownership of the production daemon in M7;
- liveness, readiness, MCP discovery health and control-plane poll health are distinct;
- M5 versioned activation, launch evidence, config/credential preservation and rollback remain authoritative.

## 15. Version/release discipline

Independent M7 targets:

```text
Studio      0.7.0-beta
Gateway     0.2.0
Filesystem  0.2.0
Fleet       0.3.0
Tunnel      upstream v0.0.14 validated target
```

These are not lockstep versions. Implementation completion, release publication and deployment remain separate states.
