# rmcp 3.4.0 Protocol Spike — M7.0 Evidence

**Status:** VERIFIED test evidence for the M7.0 protocol prerequisite.  
**Gateway branch:** `feature/m7-protocol-spike`  
**Gateway baseline:** `02a46af100c0cee5e011b8bc74079d7a78dd3658`  
**Dependency under test:** `rmcp 3.4.0` from the committed Gateway lockfile.  
**Rust toolchain:** `rustc 1.98.1`.

## Purpose

M7 cannot define safe request outcome, cancellation, progress or resource aggregation from protocol assumptions. This spike exercises the exact rmcp version linked by Gateway and preserves the observed behavior as permanent integration tests.

Permanent test source:

`mcp-server/gateway/tests/m7_protocol_spike.rs`

## Verified behaviors

### 1. External timeout does not cancel an already-dispatched child request

Current Gateway production routing wraps:

```rust
tokio::time::timeout(child_timeout, peer.call_tool(request))
```

The test `outer_timeout_drops_waiter_but_does_not_cancel_dispatched_child_call` proves that when this outer timeout expires:

- the upstream waiter returns timeout;
- no `notifications/cancelled` reaches the child request;
- the child continues executing;
- the child can complete after the caller has stopped waiting.

**M7 consequence:** the current timeout result cannot be interpreted as “the child operation failed” or “the mutation did not happen”. Post-dispatch timeout must use pending/unknown outcome semantics and must never trigger automatic mutation replay.

### 2. Explicit RequestHandle cancellation reaches the child

`explicit_request_handle_cancel_reaches_child_request_context` proves:

- `Peer::send_cancellable_request` exposes a `RequestHandle`;
- the handle exposes the generated child request ID;
- `RequestHandle::cancel(reason)` sends `notifications/cancelled`;
- the child sees its `RequestContext::ct` cancelled.

Cancellation remains cooperative. Receipt of cancellation is not evidence that a mutation was rolled back.

### 3. rmcp-owned timeout emits cancellation

`rmcp_owned_request_timeout_sends_cancel_notification_to_child` proves that a timeout configured through `PeerRequestOptions::with_timeout` and consumed by `RequestHandle::await_response` emits `notifications/cancelled` for the generated request ID.

**M7 consequence:** Gateway should own the cancellable child request handle rather than wrap a high-level child future in an unrelated `tokio::time::timeout`.

### 4. Request identity and protocol version are visible in RequestContext

`request_context_exposes_request_identity_and_negotiated_protocol` proves that server handlers receive:

- `RequestContext.id`;
- a negotiated protocol version through `RequestContext::protocol_version()`.

The default `ServiceExt::serve` pair in rmcp 3.4.0 negotiates **2025-11-25**.

Gateway will still generate its own opaque request/correlation IDs for operational identity. Protocol request IDs are transport correlation, not authorization or replay authority.

### 5. 2026-07-28 is supported but is not the default lifecycle

rmcp 3.4.0 knows `2026-07-28`, but `ProtocolVersion::LATEST` is still `2025-11-25`.

`protocol_2026_07_28_requires_discover_lifecycle_opt_in` proves that `2026-07-28` is negotiated when the client explicitly uses:

```text
ClientLifecycleMode::Discover
preferred_versions = [2026-07-28]
```

Merely setting a client config version while continuing to use the legacy `ServiceExt::serve` lifecycle does not switch the connection to the 2026 lifecycle.

**M7 decision for the first coordination implementation:** preserve the existing default/legacy child lifecycle and its 2025-11-25 behavior. A move to the 2026-07-28 discover lifecycle is a separate explicit compatibility change with dedicated child-MCP qualification; it is not an incidental part of request-coordinator refactoring.

### 6. Progress is available and keyed by an rmcp-generated child token

`child_progress_uses_generated_progress_token_visible_to_client_handler` proves:

- rmcp creates a child-side progress token for a cancellable request;
- the token is inserted into request metadata;
- child `notifications/progress` reaches `ClientHandler::on_progress`;
- the observed token matches the generated child token.

Current Gateway starts each child using the unit client handler (`()`), so it currently has no place to capture and forward child progress.

**M7 consequence:** progress forwarding requires a real child-side ClientHandler and an explicit mapping:

```text
upstream request/progress token
        ↕ Gateway request registry
child request/progress token
```

Tokens remain correlation only; progress is not terminal state.

### 7. Resource list/read APIs work in the locked SDK

`rmcp_resources_list_and_read_are_available_for_gateway_aggregation` proves `resources/list` and `resources/read` can be served and consumed with the current dependency.

M7 may therefore implement bounded resource aggregation without upgrading rmcp solely for these APIs.

## Quality evidence

After adding the seven protocol tests:

- `cargo fmt --all -- --check`: PASS
- `cargo check --all-targets --all-features`: PASS
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `cargo test --all-targets --all-features`: PASS
  - existing Gateway unit tests: 6 passed
  - M7 protocol spike: 7 passed
  - total: 13 passed, 0 failed

## P-09 disposition

**P-09 — RESOLVED.**

The locked-version protocol spike is reproducible and its permanent tests cover the request identity, cancellation, timeout, progress, resources and protocol-lifecycle behaviors needed to freeze the first M7 request-coordination contract.

Further M7 tests are still required for Gateway-owned queueing, pre-dispatch cancellation, post-dispatch outcome tracking, drain, caller disconnect, progress forwarding and resource collision policy. P-09 resolution does not claim those implementation requirements are complete.
