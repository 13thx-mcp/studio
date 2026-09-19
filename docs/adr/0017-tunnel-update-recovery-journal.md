# ADR 0017 — Tunnel Update Launch Identity and Durable Recovery Journal

## Status

Accepted and implemented by M5 review remediation R2.

## Context

The original M5 Tunnel updater could switch the installed `current` release while the configured supervisor still launched a fixed old release, could delete a newly promoted release after a post-rename directory-sync error, and had no durable authority to distinguish a verified same-version reinstall from a process interruption after promotion.

Installed version, launch identity, runtime health, and durable transaction commitment are separate facts. A version string alone cannot prove which bytes an owned child is running, and release-directory presence cannot prove that an activation completed health verification.

## Decision

Tunnel update requires the server-owned launcher to use the lexical `runtime/tunnel-client/current/tunnel-client-runtime-cloudflared` indirection, the canonical Tunnel runtime working directory, and the canonical `config.yaml`. A fixed `releases/vX.Y.Z/...` launcher is rejected before the control path is stopped.

`TunnelSupervisor` records launch evidence for every owned spawn: process generation, owned PID, resolved working directory, resolved runtime executable, resolved configuration path, and SHA-256 of the executable bytes. A running update succeeds only when restart produces a new owned generation whose launch evidence matches the activated runtime fingerprint and binding through the health window. Same-version reinstall therefore compares bytes, not only SemVer. A stopped Tunnel remains stopped and is validated only as installed/future-launch identity.

Every Tunnel mutation has a schema-versioned recovery journal under:

```text
runtime/studio/data/tunnel-update/<transaction>.json
```

The journal contains only server-generated/validated identities: source/target versions, source and target tree/runtime fingerprints, host-local fingerprint, expected previous `current`, derived candidate/rollback/failed names, prior running/stopped ownership, phase, revision, verification state, and sanitized error state. Paths are derived again from trusted roots; JSON path strings are never activation authority.

Journal states are:

```text
prepared
activation_in_progress
health_verifying
committed
rolling_back
rolled_back
recovery_failed
```

Mutation intent is durable before destructive renames. `committed` is durable only after target identity and health verification and before rollback material is removed. Failure to persist a post-mutation phase or terminal commit enters rollback; success is never inferred from directory existence.

The `current` rename is treated as a possible filesystem side effect even when the following directory sync fails. Compensation restores and verifies the predecessor before target removal. Rollback and cleanup errors are not discarded. If restoration cannot be proven, the transaction becomes `rollback_failed` / `recovery_failed` and recovery material is retained.

On Studio startup, journaled nonterminal activation defaults to restoring the fingerprint-verified predecessor. The previously running owner is restarted only when the journal records that it was running; a previously stopped owner remains stopped. Recovery never terminates a process from a persisted PID. Repeated recovery is idempotent.

Journal-less candidate/rollback/failed scratch is not guessed or deleted. It is classified as ambiguous legacy recovery material and blocks activation until an operator repairs it. Corrupt/unsafe journals, unsafe recovery-directory links, multiple concurrent recovery journals, or fingerprint ambiguity fail closed.

Cleanup after a durable verified commit is not part of the commit decision. A cleanup failure leaves the terminal journal/material for a later retry and is reported as a warning rather than pretending the activation rolled back.

## Consequences

- Tunnel runtime success now means verified installed bytes, verified launch binding, a new owned process generation when running, and a completed health window.
- Same-version integrity repair cannot pass on version equality alone.
- Crash recovery is part of M5 runtime safety rather than deferred historical M6 storage.
- The journal contains no browser-selected paths, commands, PIDs, URLs, or secrets.
- A simulated fsync/process-interruption test proves the modeled failure boundary, not physical power-loss durability.
