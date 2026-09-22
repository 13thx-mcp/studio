# M7 Studio ↔ Gateway Integration

## Scope

Studio treats Fleet-rendered `servers.d/*.yaml` as trusted desired launch
configuration. Gateway remains authoritative for live child processes,
request dispatch, cancellation and outcomes. Studio never retries a Gateway
operation after it has been dispatched but its terminal result is unavailable.

## Verification contract

Before a Gateway binary is activated, Studio captures the complete child
configuration and verifies all of these facts:

- child config rejects unknown fields and includes `restart.policy`;
- every configured command, including a Fleet trusted launcher, is a regular,
  non-symlink direct child of the canonical trusted `bin_root`;
- managed child commands still resolve to their exact catalog binary;
- `gateway_list_servers` reports each child enabled/running/error state and
  the exact configured restart policy;
- Studio sends `gateway_list_servers` and `gateway_reload` through a
  cancellable rmcp request handle;
- Studio checks the reload summary, then re-reads and revalidates the catalog
  and tool identity after reload.

The post-reload comparison proves public catalog continuity and detects a
failed changed/dead child. Exact child-process/generation reuse is Gateway
internal authority; Studio does not infer it from a count. Gateway M7.3 tests
must prove sibling generation preservation from its own runtime generation
records.

## Outcome safety

`gateway_reload` changes runtime lifecycle state. If its response times out
after dispatch, Studio sends best-effort MCP cancellation, reports the result
as unknown and does not retry it. A `child_outcome_unknown` structured Gateway
error receives the same treatment. Operators must inspect/reconcile live
Gateway state before another mutation.

`gateway_list_servers` is a read-only verification request. Its timeout is
cancelled and reported as a failed verification; it is not treated as proof of
any child outcome.

## Regression evidence

`src/update/gateway.rs` unit coverage rejects:

- restart-policy mismatch between `servers.d` and Gateway status;
- reload summaries inconsistent with a healthy catalog;
- launcher commands outside trusted `bin_root`.

The Gateway package owns integration proof for physical child reuse and
post-dispatch mutation outcome semantics. Studio's role is to consume those
outcomes without widening the executable trust boundary or retrying unknown
mutations.
