# ADR 0020 — Audit Admission and Failure

## Status

Accepted 2026-09-20.

## Context

M6 must prove manual destructive operations were durably admitted before their
physical side effect, without allowing history failure to strand an owner in a
dangerous state or turn a successful rollback into a false domain failure.

## Decision

Discretionary start/restart, registry/discovery mutation, update, and explicit
reconciliation operations require a bounded committed admission receipt before
dispatch. Typed admission and terminal receipts carry one operation identity.
The domain outcome and audit delivery outcome are separate fields.

Emergency stop, compensation, recovery, and unsolicited-exit handling use a
safety exception: they attempt reserved capture without waiting, then proceed
with existing owner safety behavior. Post-effect history failure marks audit
incomplete and blocks later discretionary admissions; it never short-circuits
rollback, stop, restart compensation, or journal recovery.

## Alternatives considered

- Fail open with dropped audit rows: rejected because it hides missing proof.
- Fail closed for every action: rejected because storage failure must not
  prevent safe stop or recovery.
- A second file outbox: rejected because it adds a competing history authority
  and unbounded recovery obligation.

## Consequences

- Admission timeout never authorizes a later dispatch, even if a late commit
  exists.
- Owner locks are released before SQL receipt waits.
- M6.2 must add cancellation, queue-reserve, and owner safe-point tests before
  lifecycle/update/config producers use this protocol.
