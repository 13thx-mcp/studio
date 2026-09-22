# ADR 0044 — M8 Recovery Cleanup, Disk/Lock Safety and Diagnostics

- **Status:** Accepted for M8.6/M8.7 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-08, P-16, cleanup eligibility, stale locks, disk pressure, diagnostics privacy

## Context

M8 creates recurring staging/cleanup activity and must survive interrupted transactions without deleting rollback authority or filling disk indefinitely.

Existing domains already own different recovery material, especially Tunnel and Studio self-update. A generic cleanup worker cannot safely infer that every old directory is disposable.

## Decision

M8 defines an ownership-first cleanup hierarchy.

## Cleanup ownership

The following are never deleted by generic automation cleanup:

- active runtime/release directories;
- finalized rollback material referenced by an active/recoverable transaction;
- Tunnel recovery journals and referenced candidate/rollback/failed directories;
- Studio self-update journals/candidates/releases controlled by Fleet launcher contract;
- Fleet rollback material needed by interrupted-swap recovery;
- Gateway safety-hold state;
- reconciliation backup while a transaction is active/failed recovery is unresolved.

Generic cleanup is limited to material that its owning subsystem classifies as cleanup-eligible.

## Ready staging cleanup

Ready staging can be removed automatically only when all are true:

- below canonical staging root;
- not symlink/escape/special file;
- structurally valid;
- not referenced by current-process prepared authorization;
- not referenced by component recovery authority;
- older than configured TTL or selected for budget reclamation under the same safety checks.

Unknown/tampered/unsafe entries are preserved and block affected automatic staging until operator repair.

Default ready-staging TTL: six hours.
Default aggregate budget: 1 GiB.

## Free-space preflight

Before auto prepare/activation, the owning update manager/policy adapter computes:

```text
required_free =
  staged candidate bytes
+ rollback baseline estimate
+ safety reserve
```

Initial safety reserve: 256 MiB.

If exact size cannot be bounded confidently, automatic mutation fails closed rather than guessing small.

Cleanup may run before mutation but may never delete recovery authority to satisfy the reserve.

## Locks

In-process mutexes are not persisted and need no stale-lock cleanup.

Existing cross-process locks, such as Studio self-update activation lock, remain owner-specific.

A future file-lock record may be cleared automatically only by its owning subsystem with an accepted ownership proof. PID existence/reuse alone is insufficient.

Ambiguous lock state blocks unattended mutation and enters diagnostics/operator workflow.

## Disk-full behavior

Where possible, durable state/journal/rollback reservation occurs before destructive side effects.

After a side effect begins:

- rollback/recovery has priority over history and generic cleanup;
- failure to write optional audit projection cannot justify abandoning recovery;
- failure to persist mandatory recovery/safety state causes explicit degraded/blocked automation status.

## Diagnostics bundle

Studio adds a bounded local diagnostics exporter.

Default archive cap: 16 MiB.

Allowed categories:

- component/version/platform identities;
- sanitized inventory;
- effective automation mode and circuit/deferral codes;
- Gateway capability/safety/child summaries;
- sanitized open/resolved hold records;
- reconciliation status;
- transaction terminal summaries;
- names/hashes/status of recovery artifacts, not secret-bearing contents;
- history health/coverage summary;
- bounded recent sanitized logs.

Excluded by construction:

- credentials/token files;
- authorization headers;
- raw environment;
- MCP arguments/results;
- arbitrary source/runtime file contents;
- unrestricted absolute-path dumps;
- full history DB by default.

Export writes into a private Studio diagnostics directory and rejects symlink escape.

Permanent tests seed canary secrets into every excluded source and byte-scan the produced archive.

## Alternatives considered

### Delete everything older than a TTL

Rejected because age does not determine recovery authority.

### Include complete config/log/database for easier support

Rejected because diagnostics would become a secret-exfiltration surface.

### Use low disk as reason to skip rollback material

Rejected because that increases corruption risk exactly under pressure.

## Consequences

- unattended staging cannot grow without bound;
- recovery ownership remains explicit;
- disk pressure fails safe;
- diagnostics become useful without broad filesystem authority.
