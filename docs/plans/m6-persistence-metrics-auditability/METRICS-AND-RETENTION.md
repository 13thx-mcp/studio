# Metrics semantics, coverage and bounded retention

**Definition version:** 1 (proposed). Metrics derive from canonical owner facts, never from websocket delivery, UI requests, status polling frequency or repeated journal scans. The live APIs keep their existing per-process counters; M6 adds separately labeled historical counters. Do not hydrate live counters from SQLite.

## 1. Process/Tunnel measures

| Measure | Exact definition | Boundary/unknown handling |
|---|---|---|
| Current state / PID | Current owner `ProcessStatus` / `TunnelStatus` only | Historical rows show “observed at”; no last-row substitution when owner unavailable. |
| Live uptime | Existing owner's `Instant::elapsed()` for the currently owned generation | Null after Studio restart until a new owned session starts; never current_time minus old DB start. |
| Total launches | Count unique successful `*.session.started` facts | Spawn/validation failure is not a launch; probe/validation subprocesses do not count. |
| Restart attempts | Count the owner restart attempt at its existing increment immediately before start (`supervisor:333`, `tunnel:547-548`) | Includes restart while stopped and failed restart-start. Does not automatically count update stop+start; record that launch cause separately. |
| Successful replacements | Optional explicitly named counter: restart operation with old owned session ended and new session successfully started | Not interchangeable with existing restart_count; this is derived, never relabelled. |
| Crashes | MCP: unrequested nonzero exit or wait error. Tunnel: any unrequested exit, including zero, or wait error | Requested TERM/KILL observed under Stopping is not a crash unless existing wait-error handling says Failed. Studio disappearance alone does not increment child crashes. |
| Last exit code | Most recent actually observed wait result that had a code | Signal-only/unknown result is null; no invented -1/0. A newer interrupted session makes “latest session exit” unknown even though an earlier exit remains queryable. |
| Last start / stop | Last observed successful spawn / observed terminal stop | Stop request timestamp is separate; incomplete timeout is not a stop. |
| Average session duration | Sum of exact observed closed-session monotonic durations / count of those sessions | Exclude open/interrupted sessions. Return denominator, excluded count, coverage start and quality. Include both clean and observed-crash sessions. |
| Observed running time | Sum of recorded within-run monotonic observation increments or exact closed durations without double-counting | Never report interrupted-session lower bounds as exact uptime; expose lower_bound_ms separately. |

A lightweight internal heartbeat every 15 s updates the current run/session last-observed monotonic lower bound. It does not emit a full event per heartbeat or create Gateway request telemetry. It is best effort/coalescible and never required before shutdown. On next healthy startup, an old open session becomes `interrupted`, with no invented end timestamp/exit code/crash classification; preserve its observed lower bound and lost-observation interval. Current UI may show historical interrupted status without controlling anything.

Clock jumps affect chart placement, not monotonic durations. Negative/unrepresentable source time is rejected or stored as unknown. If local wall time regresses materially, persist a clock-discontinuity coverage marker and mark affected time buckets partial. Ordering remains sequence-based.

## 2. Update measures

| Measure | Definition |
|---|---|
| Update checks | One admitted explicit check-batch operation; also expose per-component check results separately. Never sum component responses and call it the number of user checks. |
| Updates available | Count successful per-component check observations that actually found a newer available target relative to the recorded installed observation. An error with stale cached latest contributes no fresh availability. This is a count of observations, not distinct available releases. |
| Prepare attempts / failures | Independent counters from prepare start and definitive preparation failure; not installs. |
| Attempted installs | Unique apply operation entering the owner apply workflow after validated admission. Repeated HTTP retries or phases do not increment twice; rejected stale transaction does not count. |
| Successful installs | Unique attempted install with owner-verified Completed / Fleet terminal Completed. Staged, activation_pending, pointer equality or readiness alone is not success. |
| Failed installs | Unique attempted install with definitive failed/rolled-back/rollback-failed terminal result. A failed preparation is excluded. |
| Interrupted/indeterminate installs | Admitted/started apply with no provable terminal outcome across lost observation; separate counter, not fabricated failure or success. |
| Rollback attempts | Distinct compensation episode entering rolling back for an install. |
| Rollback count | UI default “verified rollbacks”: unique compensation verified successful. Show failed and unknown compensation separately. Failed phase + rollback_succeeded=true counts one failed install and one verified rollback. |
| Apply duration | Owner monotonic elapsed from actual apply start through terminal verification including compensation, within one process. Preparation/download duration is separate. |
| Cross-process duration | Self-update observed interval from request/handoff to observed authoritative terminal result, labeled observation latency; source journal timestamps are not a shared monotonic clock. Never mix this into exact apply-duration averages. |
| Last-known-good version | Most recent persisted verified-good installation identity at its declared proof scope, not desired/latest/staged. A verified rollback can restore a previous good identity. Unknown before M6 unless independently verified now with bootstrap evidence. |

Version strings are not artifact identity: same-version Tunnel reinstall can change bytes. Keep content fingerprint/verification scope with last-known-good and previous identity. “Last verified good” may differ from **currently observed installed** after external/manual changes; display both and the observation time. A stopped component can pass artifact/probe verification without proof it is a presently running session. Fleet has bundle verification, not uptime.

## 3. Aggregation implementation

Use a single writer transaction for a maximum 256 canonical events per aggregation batch. Read the versioned projection watermark; update hourly and daily buckets plus per-subject totals; advance watermark only with the updates. Count each canonical identity once. Replaying the same event or journal revision cannot re-increment counters. Rollback leaves both bucket values and watermark unchanged.

Counter events add 1; duration observations add count=1 and sum=duration_ms. Store integer sums/denominators, calculate averages in bounded response code; return null when denominator=0. Reject integer overflow with explicit metric health error, never wrap. Metric names are a fixed enum; no user labels, command names, arbitrary host/server tags or query-provided dimensions. Global totals use the fixed system subject, not nullable PK dimensions.

Hourly buckets use UTC floor(observed_at_ms / 3,600,000); daily buckets use 86,400,000. Cross-process/source-reported times remain distinct. Compute daily buckets directly from the same canonical facts, not by summing hourly buckets and risking double counting. Bucket quality is partial when its subject/time window intersects an evidence gap; a zero is rendered only for a fully covered window, otherwise null/unknown. Metrics report `through_seq` separately from the event-store high watermark.

Totals are **since recorded coverage began**, not lifetime totals for software before M6. Existing totals can outlive raw events but must retain quality=partial if the underlying observation period had gaps. Do not “rebuild all-time totals” from already-retained fragments and label them complete. A definition-version change requires a separate series and explicit coverage/reset; rebuilding needs retained source evidence.

## 4. Retention defaults and limits

Defaults are explicit operator policy, not a compliance/legal retention promise. They are stored in local typed config with validated ranges; no browser setter or arbitrary retention SQL.

| Class | Default policy |
|---|---|
| Mandatory audit events / operation terminal summaries | 365 days; do not delete earlier merely to make new admissions succeed |
| Detailed sessions, lifecycle/phase/check/drift observations | 90 days, unless required by a currently pinned lineage/active operation |
| Hourly metric buckets | 90 days |
| Daily metric buckets | 365 days |
| Per-subject totals | While registered/active; removed subject retained at most 365 days after last referenced evidence, then collect |
| Latest/previous/last-verified-good identity and latest/loaded/previous config anchors | Compact bounded summaries per active subject/surface, with explicit pruned predecessor boundary; not an unlimited ancestor chain |
| Incomplete operations | Retain 365 days as indeterminate; unresolved historical rows never keep a runtime lease or recovery action alive |
| Journal watermarks | Retain while the validated source record exists or linked update summary is retained; bounded by DB cap. Never delete/change source journals as retention work. |
| Backups | At most two completed snapshots plus one in-progress candidate; failed candidate cleaned only by known filename/identity rules |

Defaults: DB soft pressure 384 MiB, hard main-file cap 512 MiB (`max_page_count=131072` at 4096-byte pages); WAL warning 16 MiB, stop new discretionary admissions at 64 MiB until checkpoint/reader pressure clears; raw event soft count 1,000,000 and hard count 2,000,000. These are deliberate finite limits, not throughput claims. `journal_size_limit` does NOT enforce a hard live WAL bound. Enforce capacity before batches and reserve 16 MiB logical budget for already-admitted terminal/coverage writes. Filesystem ENOSPC can still defeat reservations; keep failure semantics honest.

Total expected storage envelope includes DB + WAL/SHM + up to two 512 MiB backups + one in-progress backup, approximately 2.1 GiB plus small metadata; verify available space before backup/migration, and do not pretend max_page_count bounds the whole directory. A filesystem quota is required for a strict external physical cap; M6 enforces bounded records, transactions, in-process producers and admissions, not limits on hostile external writers or arbitrary OS allocation behavior.

Housekeeping runs at startup when history is healthy and at most every 60 s thereafter, plus capacity-triggered checks. It only aggregates, checkpoints and prunes history; it never checks/releases/reconciles/restarts components. Per maintenance transaction delete at most 500 rows or 4 MiB of candidate payload, whichever is reached first; transaction budgeting and query deadlines prevent a long retention lock. Yield to admitted terminal writes. Do not use an unbounded DELETE/VACUUM inside an owner operation.

## 5. Pressure behavior and cursor interaction

First compact metrics and expire eligible low-value detail; checkpoint with short-lived readers. At soft limits, warn and disable nonessential extra observations. At hard thresholds, deny new discretionary actions with history-capacity error before side effects rather than delete unexpired required audit. Safety transitions still occur and may be only partially captured; emit health/coverage notices. Operator can expand an approved finite local limit, explicitly export/expire under policy, or move to a fresh archived history epoch during offline maintenance. No automatic data-loss “repair”.

Preserve latest/previous/last-good heads with compact evidence copied into their bounded summary before removing ancestry. Every detached predecessor link receives `boundary_reason=retention`; do not leave a plausible complete chain. Active operation/session evidence cannot be pruned; a runaway backlog causes admission pressure, not limitless exceptions.

Increment a **global retention_epoch** for each deletion/compaction batch that changes paginated membership. Any older cursor returns 410 history_expired, even if its next event might still exist. This deliberately sacrifices some pagination continuity to avoid false stable history over holes. Never hold an SQLite snapshot/reader open for a browser cursor. Long-duration UI queries use coarser buckets with bounded returned points.

## 6. Golden test fixture

Two observed MCP sessions: 10,000 ms clean exit and 6,000 ms nonzero exit; one third interrupted after a 2,000 ms heartbeat lower bound; two successful launches plus the third observed launch = 3 launches; exact average = (10,000+6,000)/2 = 8,000 ms; known crashes = 1; interrupted count=1; lower-bound observed running time=18,000 ms, exact closed running time=16,000 ms. A failed spawn adds no launch. A restart attempted from Stopped that fails increments restart-attempt count but not launch/replacement count.

One successful prepare never applied, one failed prepare, one successful apply, one failed apply with verified rollback, one rollback-failed apply and one interrupted apply yield attempted installs=4, successes=1, definitive failures=2, indeterminate=1, verified rollbacks=1, rollback failures=1. Repeated phase scans and duplicate delivery leave every value unchanged. A provider failure retaining yesterday's cached latest contributes a failed component check, not fresh availability.

## 7. Imported evidence and terminal contribution identity

Pre-M6/bootstrap imported terminal journals may explain an observed identity but do not count as a new M6 update check, apply attempt, successful install, launch or exact duration. A terminal journal for a recorded M6 apply across two processes can contribute once through that existing attempt's immutable first qualifying terminal_seq. Later revisions repeating the terminal state, source re-scans, audit-delivery resolution and retention replay do not contribute again. See SCHEMA reducer rules; do not count every row whose current phase happens to be Completed.
