# Event taxonomy, write points and crash consistency

**Proposed contract.** Existing-source anchors refer to the audited HEAD; future module names below are planned. The bus carries live UI events; it is not the input to this recorder.

## 1. Envelope and closed payload families

Every envelope carries an event ID, source stream/ordinal, Studio run ID when observed locally, opaque subject incarnation, optional operation/parent operation and M5 transaction reference, observed UTC time, optional source time/monotonic elapsed time, time quality, evidence kind, payload version and typed payload. Use stable dot-separated lowercase names. Category is explicit, not inferred from strings. No unbounded metadata/value map is permitted.

Allowed payload families:

| Family | Permitted fields | Explicitly prohibited |
|---|---|---|
| Operation | action enum, actor enum, subject/operation IDs, effect outcome, audit state, safe error enum, safety exception | user-supplied actor/user-agent/IP/name, arbitrary request body |
| Lifecycle | session ID, owner kind, generation, observed PID, old/new state enum, reason enum, exit code, exact/observed duration, crash classification | command/argv/env/cwd, raw last_error, stdout/stderr |
| Update | domain/component enum, validated versions, transaction ID, source revision, phase, rollback outcome, was_running, verification scope | staged/candidate/current/rollback paths, raw journal, launcher nonce/credentials/PID signalling instructions |
| Artifact | opaque ID, version/provider/platform, verified archive/content/member digests and fixed role enum | local paths, arbitrary filenames/URLs, copied package metadata |
| Config/registry | opaque revision/subject IDs, changed-category enums, present/enabled/runtime kind; exact digest only in private record | config bodies, env names/values, args, project/working/executable paths, arbitrary display names |
| Drift | opaque host key, managed generation, safe surface enums, state/phase, desired/active revision IDs, config-activation/client-freshness enums | host profile body, arbitrary rendered paths, remote-client identity or claimed remote acknowledgement |
| Coverage | reason enum, observed sequence/time interval, optional known lost count | guessed exit times, guessed successful operations, invented actors |

Byte limits: envelope 8 KiB; payload 4 KiB; safe codes 64 ASCII chars; event name 96; source key 256; transaction ID 160 and existing safe-ID validation; version 128 with existing Version parser; lists at most 16 artifact roles or 32 fixed surface/change categories. A field that fails validation becomes a safe rejection/error category, not truncated command text. Event JSON must round-trip through the exact typed variant with unknown fields denied. Never persist `Debug` or `Display` of an error.

## 2. Names and semantics

| Event names | Category | Emission fact |
|---|---|---|
| `operation.requested`, `operation.rejected` | intent / outcome | Validly parsed supported request or typed rejection; neither implies admission. Rejection recording is rate-bounded. |
| `operation.admitted` | admission | Owner lease/reserved capacity obtained and durable admission committed; caller may still fail/cancel before dispatch. |
| `operation.finished`, `operation.indeterminate` | outcome | Actual owner result and separate audit durability; missing evidence never converted to success. |
| `studio.run.started`, `studio.run.ready`, `studio.run.closed` | observation / transition / outcome | Observing process start; listener readiness after bind; verified owned shutdown and history drain. |
| `studio.run.interrupted` | observation | Previous observer ended without clean closure; not a child crash assertion. |
| `mcp.start.requested`, `mcp.stop.requested`, `mcp.restart.requested` | intent | Typed action intent before effect; correlate one operation. |
| `mcp.state.changed`, `mcp.session.started`, `mcp.session.ended`, `mcp.start.failed` | transition / outcome | Owner transition, successful child spawn, observed wait result or spawn failure. |
| `tunnel.start.requested`, `tunnel.stop.requested`, `tunnel.restart.requested` | intent | Same intent separation for Tunnel owner. |
| `tunnel.state.changed`, `tunnel.session.started`, `tunnel.session.ended`, `tunnel.start.failed` | transition / outcome | Tunnel owner evidence with its own exit-0 crash classification. |
| `registry.change.committed`, `discovery.approval.committed` | outcome | File registry mutation completed; discovery validation alone is not approval completion. |
| `registry.snapshot.observed`, `config.revision.observed`, `config.loaded.observed` | observation | Safe current metadata, validated on-disk revision or exact bytes loaded by this process. |
| `config.change.committed`, `config.restore.verified` | outcome / rollback | Owner file/manifest commit or verified compensation, not render-plan creation. |
| `update.check.requested`, `update.check.component.finished` | intent / observation | One explicit batch request and one per-component provider result. Failed fetch with cached latest is not fresh availability. |
| `update.prepare.requested`, `update.prepare.started`, `update.artifact.staged`, `update.prepare.failed` | intent / transition / observation / outcome | Owner preparation; staged only after validation/metadata/ready rename. Not an install attempt yet. |
| `update.apply.requested`, `update.apply.started`, `update.phase.changed` | intent / transition | Accepted apply entered the owner's mutation workflow; phase does not imply verified success. |
| `update.install.verified`, `update.install.failed` | outcome | Terminal owner verification of target or definitive apply failure. |
| `update.rollback.started`, `update.rollback.verified`, `update.rollback.failed` | rollback | Compensation began/restored predecessor verified/compensation failed. |
| `artifact.installation.observed` | observation | Current content identity seen; keeps unknown/partial verification scope. |
| `fleet.drift.observed`, `fleet.baseline.adopted`, `fleet.reconcile.committed` | observation / outcome | Evaluated drift, actual manifest adoption, or verified apply plus manifest commit. |
| `fleet.reconcile.rolled_back`, `fleet.reconcile.rollback_failed` | rollback | State files, baseline and runtime/catalog verification after compensation. |
| `studio.config_activation.observed` | observation | Existing loaded-config/manifest proof; never clears a marker itself. |
| `studio.self_update.requested`, `studio.self_update.journal.observed` | intent / observation | Explicit handoff or validated actual persisted revision. |
| `studio.self_update.finalized` | outcome | Validated terminal Fleet-owned journal result; bootstrap/readiness alone cannot emit it. |
| `tunnel.recovery.observed`, `tunnel.recovery.verified`, `tunnel.recovery.failed` | recovery | Existing owner recovery path evidence, captured before terminal journal cleanup. |
| `history.coverage.changed`, `history.retention.boundary` | observation | Known incompleteness or declared expiry; no operational control. |

One physical fact produces one canonical fact event; audit/history/metrics projections reference it rather than each incrementing counters. For example, `mcp.session.started` counts a launch, while `mcp.state.changed` to Running does not count a second launch. Request aliases and operation lifecycle records are correlated, not additional actions.

## 3. Owner integration map

| Owner / exact source point | Integration and record timing | Failure rule |
|---|---|---|
| `main.rs:40-118` load/construct/recover | Construct HistoryHandle after root resolution, pass it to every owner. Capture exact LoadedConfigIdentity once. Begin run/bootstrap before owner recovery when healthy. | History open failure supplies degraded handle; preserve existing M5 recovery flow. |
| `main.rs:117-132` recovery/startup check | Observe outcomes from actual Tunnel/self-update/reconciliation owners. Do not independently replay the same action from history. | Safety exception; DB failure cannot block compensating behavior. |
| `main.rs:140-145` bind/readiness | `run.ready` only after bind; use existing exact process/config proof unchanged. | History error is not a Fleet readiness failure. |
| `main.rs:173-181` shutdown | Signal/stop owners in existing order, then bounded drain and run end. | Never await DB before sending stop; incomplete shutdown is not clean. |
| `supervisor/mod.rs:212-262` start | Admission before setting Starting; start-failure event after owner failure is known. Include operation context so nested update start is correlated, not a second admission. | No new `?` path from history can strand Starting/owned child. |
| `supervisor/mod.rs:398-429` successful spawn | Allocate session UUID before spawn; commit session only once spawn yields owned PID. Capture start/ordinal under existing owner synchronization; enqueue a bounded typed event without SQLite I/O. | A spawned child remains owned even if event delivery fails. |
| `supervisor/mod.rs:458-503` child wait | Capture prior start Instant/duration and was_stopping before clearing fields; classify exact source semantics. | Wait handling/counters do not wait for or depend on DB. |
| `supervisor/mod.rs:264-335,338-351` stop/restart/shutdown | Stop intent safety path; restart admission before stop, attempt counter at existing increment; child wait is session end authority. | No duplicate end event from HTTP stop and wait monitor; reserved capture cannot defer TERM/KILL. |
| `tunnel/mod.rs:336-489,492-550` | Equivalent hooks; capture only safe subset of launch evidence, no paths/env. Count unrequested exit 0 as crash. | Owner state/fingerprint proof and F1 cannot be bypassed by an old session. |
| `registry/mod.rs:179-254,502-534` | After P-02, exact serialized whole-document commit receipt with sanitized change categories and digest. SQL outside registry mutation mutex. | Failed file mutation is not `registry.change.committed`; DB failure never overwrites file with a historical snapshot. |
| `api/mod.rs:295-317` discovery approval | Validate/rescan candidate, register, then approval receipt tied to registry commit. Raw candidate path is not persisted. | Same correlation ID; no “approved” event before register succeeds. |
| `inventory.rs:475-521` check tasks | Record explicit batch once; per-component start/result inside task, including cancelled/join failure for every expected component. Observation snapshot identifies installed version used for availability. | Cache fallback/error never increments available count; do not run new periodic checks. |
| `staging.rs:336-359` | Staged artifact receipt only after ready-directory rename returns. Hash/role projection excludes paths. | A persisted stage row never authorizes apply without current revalidation. |
| `transaction.rs:350-464,528-690,721-925` | P-01 before integration; reserve admission, capture preparing/staged, apply start and owner phases. Terminal success at existing post-verification Completed; failed/rollback outcome from dedicated branches. | History phase hooks are non-failing to physical compensation; capture pre-cleanup verification separately from Completed when cleanup is still a required owner step; never make history a new rollback cause. |
| `gateway.rs:944-968,1013-1437` | Capture owner stop/reconnect/catalog verification and terminal result after catalog preservation. Record proof scope `catalog_probe` or owned control-path evidence, not HTTP request telemetry. | Retain independent expected names F8. DB must never supply expected catalog set. |
| `fleet.rs:474-576,627-881` | Stage bundle, activation/verification, local preservation, terminal/rollback receipt. | No synthetic process/session for bundle validation subprocesses. |
| `tunnel_update.rs:383-417,726-750,919-951,1061-1072,1185-1196` | Observe validated journal revisions and verified terminal outcome **before** owner journal deletion. For same version include target/source content fingerprints, not equality of versions. | Bounded safe-point flush; do not retain or change a journal solely for DB delivery. Preserve independent journal errors and compensation. |
| `tunnel_update.rs:456-561` startup recovery | Capture sanitized validated journal before cleanup plus exact recovery verification result. Existing recovered transaction is historical only; volatile status lookup behavior stays separate. | No DB-driven choice of predecessor/target or use of recorded PID. |
| `self_update.rs:441-498,501-558,775-863` | Before handoff admit request; observe persisted revision returned by validated reader, not caller's stale revision. Old process records activation_pending, not completed. | Never insert DB calls into Fleet's terminal decision. Nonterminal external journal remains external-owned. |
| `reconciliation.rs:815-829,845-892,939-1013,1029-1094` | Emit adoption/manifest commit/rollback only from actual successful persistence/verification branches. Publish drift observation from evaluation; loaded marker changes get distinct receipts. | `set_status` alone is not commit proof. Preserve original rollback manifest and F4/F7. |
| `realtime.rs`; `api/mod.rs:496-549` | Writer publishes `history_changed` after SQL COMMIT and `history_health_changed` on degradation; retain original live events. Subscribe before initial history watermark snapshot to avoid a new reconnect gap. | Lost notices cause a bounded history refetch, never loss of stored records. |

## 4. Lock and cancellation protocol

Order: existing shared runtime operation lease → manager-local logical lease → bounded durable admission → short owner state/registry critical sections → physical owner operation → safe-point terminal receipt. A logical lease is not a held mutex; coordinator/manager mutexes are released by their acquire functions. Storage cannot call back into registry/supervisor/update managers, so the wait-for graph has no DB→owner edge.

P-01 correction applies the shared component lease inside generic apply. Nested calls from updates/reconciliation pass an operation context with an existing-admission token; they do not recursively acquire the same coordinator lease or independently reject required compensation because SQLite degraded mid-operation. Unsolicited exits and emergency shutdown use the explicit safety context.

Allocate source ordinal while state transition ownership is serialized. Capture bounded values there; publish without blocking after releasing the lock (or use a strictly nonblocking reserved-channel enqueue, never an await). The recorder may receive different-source events in either order. Reducers use source ordinal and terminal-state precedence, so delayed messages cannot reopen sessions or regress update outcomes. SQLite seq is commit order, not a claim of global physical causality. Tests must deliver end-before-start and terminal-before-stale-phase envelopes.

Dropping an HTTP connection must not cancel an admitted destructive operation halfway through compensation. Preserve existing ownership until safe completion; a scoped owner task with bounded admission and tracked join state may be required during M6.2. No fire-and-forget retry from a history worker. An admission timeout is irrevocably non-dispatchable even if its SQL transaction later commits. After-action delivery retries reuse the same event identity and never replay the physical action.

## 5. Crash/failure matrix

| Crash/failure window | Durable evidence / truthful next state |
|---|---|
| Before admission COMMIT | No physical action allowed. A request may be absent/rejected; no phantom success. |
| COMMIT completed but caller timed out | Late admission may exist; caller never dispatches. Resolve as not-dispatched when proven, otherwise indeterminate. |
| After admission, before effect | Pending operation; after restart terminal unknown unless validated evidence proves non-dispatch. |
| Child spawned, start event not committed | Run/operation admission reveals incomplete coverage. Never adopt or kill a child from old DB PID. |
| File/config activation occurred, receipt not committed | Existing files/journal remain sole authority. Bootstrap/current identity is a new observation, not proof of this operation's success. |
| Tunnel terminal journal persisted, history not yet committed | M5 owner recovery can validate and observe surviving journal. Hook before normal cleanup narrows the normal crash gap. |
| DB fails before Tunnel journal cleanup | Report incomplete history and preserve normal journal cleanup semantics. No indefinitely audit-held journal. |
| Self-update spans exit/start and Fleet terminal write | New Studio observes validated revision after readiness; terminal result only from Fleet journal. Missing revisions remain partial. |
| SQL failure halfway through event/projection | Entire SQL transaction rolls back; retry identical envelope once writer healthy. No half-session/half-counter. |
| Exit observed but DB unavailable | Actual owner state and signal processing continue; audit/coverage degraded. Unknown crash totals until actual evidence exists. |
| Disk full/corruption and shutdown | No new discretionary admission; safety stop/rollback/recovery unaffected. Retain error code/last committed cursor, not raw OS path error. |
| DB healthy but metrics compaction fails | Domain/audit results stay intact. Metrics health/watermark lag and retention pressure reported. |

## 6. Cross-process self-update observer

The observer only reads validated files for a known nonterminal, already-authorized self-update. It does not call a release provider, prepare/apply/reconcile/restart or edit a journal. Start after listener readiness; query at most once per second, max one active self-update, for a maximum 120 s. Stop on terminal, invalid record, shutdown or deadline. After the deadline, report pending/partial and allow explicit status refresh or next startup to observe later completion. This is finite observation, not unattended recovery policy.

Import actual current journal revisions on first M6 startup as `bootstrap` observations. Do not backdate a success event to untrusted updated_at; keep the source time separate. Across processes use `(studio, transaction_id, journal_revision)` for deduplication. Existing activation proof nonce, launcher ownership details and internal paths stay out of DB/browser payloads.
