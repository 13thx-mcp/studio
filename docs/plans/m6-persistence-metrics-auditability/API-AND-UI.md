# Historical API and UI contracts

**Status:** proposed M6.0 contract, implemented incrementally in M6.2/M6.7. Existing live APIs remain owner-backed. Historical DTOs never deserialize into action requests.

## 1. Endpoints

All new routes are read-only and under `/api/history/v1`. No arbitrary SQL, full-text body search, filesystem export/download path, database reset, or history-based apply/resume endpoint.

| Route | Purpose / result | Allowed filters |
|---|---|---|
| `GET /status` | HistoryHealth; can be returned from an in-memory health snapshot when DB fails | None |
| `GET /events` | Unified sanitized event timeline | subject, category, event-name enum, operation, from_ms, to_ms, cursor, limit |
| `GET /subjects` | Paged opaque subject catalog, including retained retired subjects | kind, cursor, limit |
| `GET /subjects/{subject_id}/sessions` | Observed process/Tunnel sessions, never current status | from_ms, to_ms, cursor, limit |
| `GET /updates` | Update attempt summaries | component enum, outcome enum, from_ms, to_ms, cursor, limit |
| `GET /updates/{history_id}` | One update's summary and bounded phase page | cursor, limit |
| `GET /operations/{operation_id}` | Effect and audit disposition for an admitted operation | None |
| `GET /config-revisions` | Safe surface revision/change categories and activation observations | surface enum, subject, from_ms, to_ms, cursor, limit |
| `GET /drift` | Reconciliation observations; not authorization to reconcile | subject, state enum, from_ms, to_ms, cursor, limit |
| `GET /subjects/{subject_id}/lineage` | Current-observation match plus latest/previous/last-verified-good history links and boundary | cursor, limit |
| `GET /metrics` | Time-series or totals from a closed metric-name enum | subject, metric, resolution=hour/day/total, from_ms, to_ms |

Default page size 50; maximum 200. Default time window last 24 hours, maximum 366 days per request; explicit finite windows can select older retained coverage. Subject/operation/lineage lookups remain bounded independently. At most 1,000 points in total across one metrics response; reject excessive requests with `range_too_large`, never silently truncate or run a huge intermediate query. Responses are capped at 1 MiB; if the byte cap requires a smaller page, return a valid next cursor. Limit filters to eight categories/event names, one subject and one component, rather than arbitrary AND/OR expressions. Unknown fields, unsupported enum values, malformed UTF-8/IDs, duplicate scalar query keys, nonfinite/overflowed timestamps, reverse ranges and cursors over 2 KiB return 400.

The route table uses a historical update `history_id` (opaque operation identity); the validated existing M5 transaction ID may be included for display/correlation. A historical transaction ID **does not** make a volatile manager transaction resumable. Existing `/api/update-transactions/{id}` continues consulting existing owners; history fallback is a separate read-only view with `actionable=false`. UI action controls use current authoritative transaction and inventory responses only.

## 2. DTO boundaries

Proposed JSON shapes, not implemented responses:

```json
{
  "items": [{
    "sequence": "812",
    "event_id": "opaque-uuid",
    "name": "update.completed",
    "category": "outcome",
    "subject_id": "opaque-uuid",
    "component": "tunnel",
    "operation_id": "opaque-uuid",
    "observed_at_ms": 1789826400000,
    "source_at_ms": null,
    "evidence_kind": "owner",
    "time_quality": "monotonic",
    "payload": {"target_version": "1.2.3", "verification_scope": "owned_runtime"}
  }],
  "next_cursor": "opaque-versioned-token",
  "snapshot": {"db_epoch": "opaque-uuid", "through_seq": "812", "retention_epoch": "4"},
  "coverage": {"state": "partial", "reasons": ["pre_m6_unknown"], "since_ms": 1789800000000}
}
```

`HistoryHealth`: `state=initializing|healthy|degraded|unavailable|incompatible|capacity_limited`, stable `reason_code`, `admission_available`, `pending_obligations` decimal string, `db_epoch` nullable, `latest_committed_seq` decimal string nullable, `metrics_through_seq` nullable, `coverage_state`, `observed_at_ms`. Never expose the DB path, raw SQLite messages, schema SQL or host permissions. History health is independent of `/health` process readiness and M5's activation-ready proof.

`HistoricalSession`: opaque IDs, owner kind, historical PID explicitly labeled `observed_pid`, start/end observation times, end kind, exact duration nullable, observed-duration lower bound, exit code nullable, start/stop operation IDs and coverage. It has **no `running` boolean**. An open historical row means no terminal observation, not a currently running child.

`HistoricalUpdate`: component/catalog label, validated source/target versions, last observed phase, normalized `effect_status`, separate `audit_status`, verification scope, rollback result, observation interval quality, related config/install identities, missing-phase indicator, and `actionable=false`. PIDs, launcher ownership tokens, candidate names, journal path/revision internals, credential-bearing error strings and file hashes are not browser payloads. Public artifact identity is an opaque ID; a reviewed public archive checksum may be shown only if needed, never a host/config/credential digest.

`ConfigRevision`: opaque revision/surface IDs, approved change-field codes and enabled/runtime enum values where applicable, observation source, predecessor link and `loaded|written|restart_required|unknown` evidence. Do not serialize raw registry names, registry keys, argv, relative paths, config digest or body. Current live UI may join a permitted display label from its existing registry response; history itself does not persist that free text.

All 64-bit sequence, generation and counter values are decimal strings in new DTOs. Safe epoch milliseconds and bounded durations may be JSON numbers only after explicit JavaScript-safe integer checks. Existing live DTOs retain their current types; do not silently replace their per-run counters with historical totals.

## 3. Deterministic pagination and mutable projections

Events sort descending by `seq`, not by clock. On the first page capture `db_epoch`, high-water sequence H, global retention epoch R and normalized filters; fetch `seq<=H ORDER BY seq DESC LIMIT n+1`. The opaque, versioned base64url cursor contains H, last sequence, R, normalized filter digest and last sort key. It is not a capability or authenticated user token: validate every field and enforce all ordinary filters/limits on every use. Tampering can only select another permitted bounded read, never SQL or file authority. Do not put secrets or DB paths inside cursors.

Subsequent pages use `seq<last AND seq<=H` plus unchanged normalized filters. New commits above H do not move existing rows between pages. Repeated timestamps and sequence gaps are valid. A restore changes db_epoch; any mismatch or retention_epoch change returns 410 `history_expired` and a fresh-head hint. Malformed cursors return 400. Do not hold an SQLite transaction between HTTP requests.

Session/update/drift/config pages use immutable event membership keys as of H. Mutable latest-summary tables may otherwise reveal a terminal state committed after the page snapshot. Therefore query the retained latest event <=H for each member, or return a separately labeled latest projection with its own `as_of_seq` and do not advertise that projection as a frozen page. M6.7 must implement the former for list pages. If retention removed the evidence required for an as-of projection, return a boundary/expired cursor, not a reconstructed fictional state.

Read work is restricted to two workers with a bounded queue of 32 total requests; queue admission deadline 100 ms, total query deadline 500 ms, progress handler/interrupt for bounded cancellation and maximum result size. Bind all values. Routes select from reviewed SQL templates; column names/order/joins are never client strings. Saturation returns 503 `history_busy`; do not spawn extra workers. `EXPLAIN QUERY PLAN` fixtures verify useful indexes at the declared long-history scale.

## 4. Realtime integration

Persistence does not subscribe to EventHub. The history writer publishes a small `history_committed` notification **after COMMIT**, containing db_epoch, latest_seq and retention_epoch; no raw event payload or control authority. An independent `history_health` notice reports degraded/admission state. Existing live lifecycle events are still emitted without waiting for historical metric writes.

For new history notifications, subscribe before obtaining the handshake watermark. The client reads pages through that watermark and requests a fresh head when a later notification arrives. A notification below/equal to the current watermark is a duplicate; ignore it. On lag/reconnect/epoch change, refetch history health, head and current live snapshots independently. Fix the relevant subscribe/snapshot race when adding the new subscription in `src/api/mod.rs::websocket_session`; do not rely on the current snapshot-before-subscribe sequence for guaranteed history delivery.

Coalesce notifications to at most one per 250 ms, with monotonic latest committed sequence. Notification loss cannot lose data; REST remains the persisted source. History never uses the old log sequence, which resets per runtime process. A late persistence completion updates audit status but must not issue a second action or turn a currently stopped owner into Running.

## 5. UI implementation

Add `web/src/history/{types,api,state}.ts`, `HistoryPanel.tsx`, `HistoryTimeline.tsx`, `HistoryCharts.tsx`, `UpdateHistory.tsx`, `DriftHistory.tsx` and focused tests; adapt to existing import conventions rather than introducing a second frontend framework. Wire a History section into `App.tsx` and safe links from `UpdatesPanel.tsx`. Keep current cards driven by live APIs. Distinct badges: Live, Historical observation, Imported baseline, Partial, Pruned boundary, Audit incomplete.

Use small accessible React/SVG line/bar charts with a tabular alternative, no new chart dependency in the initial plan. Break lines over unknown intervals; never plot missing evidence as zero. Charts cover launch/restart/crash trends, closed-session duration, apply success/failure/rollback and drift observations. Do not introduce request throughput, Gateway tool usage or invented health metrics. Display the current live uptime separately from recorded historical duration and show exact/lower-bound labels.

Required states: first startup/no observed history; loading first page; loading more; empty filtered range; DB unavailable while live controls remain; partial coverage; metrics projection lag; offline/reconnecting with stale historical data; cursor expired; missing/pruned detail; removed subject; and pending external self-update with no terminal proof. Unknown config ancestry is shown explicitly, not inferred from a matching version or content digest.

State reducer de-duplicates by event ID/sequence and pins pagination to its snapshot. It does not mix newly arrived head events into the middle of older pages. Cancel stale fetches on filter changes; ignore responses for an old db_epoch/filter request. Cap browser retained rows at 1,000 with explicit pagination controls, not endless accumulated memory. LocalStorage may remember harmless filters and existing pending-transaction hints, never history authority or terminal outcomes. Escape all text; no raw HTML rendering.

## 6. Browser privacy and verification

Apply loopback/Host validation, same-origin browser checks, no CORS, `Cache-Control: no-store`, and `Cross-Origin-Resource-Policy: same-origin` to new history responses. Reject foreign Origin/Sec-Fetch-Site where present; same-origin browser GETs may omit Origin, and loopback operator requests may omit browser metadata. Test allowed host and port rather than trusting forwarded headers. Existing legacy routes are not silently declared fixed by this addition; document any separately discovered exposure.

Permanent test families: stable pages during concurrent insert; expiration during retention/restore; mutable-summary as-of correctness; malformed/oversized filters and SQL-like values; query timeout/queue limit; cross-origin/Host rejection; no path/env/argv/digest/error-body leaks; history replay cannot enable apply; reconnect no duplicates; zero versus unknown charts; empty/degraded/pruned UI; stale fetch cancellation; decimal sequence above 2^53.
