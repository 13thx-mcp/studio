# ADR 0016 — Studio Self-Update External Activation Contract

## Status

Accepted and implemented in M5.10.

## Context

Studio cannot safely replace the executable that is currently serving its own update request and then claim success from the same process. A self-update must cross a process-ownership boundary, preserve host-local config/state, activate backend and dashboard assets as one release unit, survive reconnect/restart, and retain deterministic rollback authority.

The Aira runtime inspection for M5.10 found a legacy flat deployment:

```text
runtime/studio/
├── mcp-studio
├── studio.toml
└── data/
```

No active `mcp-studio` process and no Studio launchd label were present during inspection. M5.10 therefore cannot assume a pre-existing service supervisor. The plan permits either launchd or a small trusted launcher outside the Studio process.

## Decision

Studio owns release discovery, verified staging, candidate preparation, transaction persistence and the handoff request. The deployed Fleet control bundle owns the external activation step through a fixed `fleetctl studio-activate` command.

The browser never supplies launcher path, executable path, config path, PID, argv, release directory or rollback path. Studio derives the active Fleet host profile and passes only:

```text
studio-activate
--host <server-selected-host>
--transaction <server-generated-id>
--parent-pid <current-studio-pid>
```

### Runtime layout

M5.10 introduces a versioned release layout while preserving the legacy flat binary as first-migration rollback material:

```text
runtime/studio/
├── current -> releases/vX.Y.Z
├── releases/
│   ├── vX.Y.Z/
│   │   ├── mcp-studio
│   │   └── web/dist/
│   └── .candidate-<transaction>/
├── studio.toml
├── data/
│   └── self-update/<transaction>.json
└── mcp-studio
```

`studio.toml` and `data/` are host-local and outside every release unit. Release archives never overwrite them.

After versioned activation exists, Fleet source-built `install --component studio` is bootstrap-only and refuses to overwrite the legacy fallback binary.

### Prepare contract

Studio reuses the trusted 13thx release provider, strict platform asset selection and M5.5 verified staging.

Prepare:

1. requires a strictly newer semantic target than installed Studio;
2. stages the trusted Studio archive;
3. requires exact package structure including `mcp-studio` and `web/dist/index.html`;
4. copies only staged release content into `releases/.candidate-<transaction>`;
5. independently validates the candidate binary version;
6. computes a bounded full-tree SHA-256 fingerprint over relative file names and file digests;
7. persists a schema-versioned durable transaction journal.

The transaction journal lives below `runtime/studio/data/self-update/`, is atomically replaced, and is mode 0600 on Unix.

### Apply contract

Apply revalidates:

- installed source version for the initial staged transition;
- M5.5 ready staging identity and archive/executable hashes;
- candidate or already-promoted target release fingerprint;
- target binary semantic version;
- matching dashboard `web/dist/index.html`;
- current process identity.

Self-update is accepted only when the calling process is the deployed Studio runtime: either the legacy `runtime/studio/mcp-studio` binary or `runtime/studio/current/mcp-studio`. A source/debug executable cannot update another deployed Studio instance.

Studio then persists `activation_pending`, starts the fixed Fleet launcher, returns the durable transaction view, and requests graceful shutdown after a short response window. Studio itself never swaps `current`, kills its own PID, or starts its replacement.

### External activation

The Fleet launcher:

1. reads and validates durable transaction metadata;
2. waits for the supplied Studio PID to exit;
3. validates source rollback identity and candidate fingerprint/version/web assets;
4. promotes the candidate to `releases/vX.Y.Z`;
5. atomically repoints the relative `current` symlink;
6. launches `current/mcp-studio --config runtime/studio/studio.toml` with the release directory as working directory so Axum serves the matching `web/dist`;
7. creates a cryptographically random activation nonce and passes only the transaction ID + nonce to the spawned Studio through the launcher-controlled environment;
8. after successfully binding its configured listener, that exact Studio process atomically writes a server-derived `data/self-update/<transaction>.ready.json` proof containing the nonce, its PID, canonical loaded-config path and SHA-256 of the exact parsed config bytes;
9. Fleet requires the proof nonce/PID/config identity to match the `Popen` instance and requires the expected release fingerprint to remain unchanged while loopback `/health` reports `status=ok`, `service=mcp-studio` and the exact target semantic version through the stability window;
10. removes the transient proof and marks the durable transaction completed or rolls back.

The launcher is a constrained operation, not a general command runner.

### Rollback and interruption recovery

Before activation, the launcher records whether the previous Studio layout was versioned or legacy-flat. That previous identity is persisted and reused on resume; it is not recomputed from a partially switched `current`.

Target health failure or an exception after `current` was switched stops the target process, restores the prior versioned pointer or removes `current` to return to the legacy flat binary, starts the prior Studio, verifies its source version through `/health`, and marks `rolled_back`.

Failure to prove rollback is `rollback_failed`. A rollback-failed transaction blocks creation of a later self-update until an operator repairs runtime state.

The journal is durable across process restart. Transaction lookup after reconnect reads the journal rather than process memory. If activation is interrupted, repeated apply can re-arm the same transaction with the new source Studio PID after validating the prepared/promoted target fingerprint. Fleet resume logic preserves the originally persisted rollback identity.

Fleet is the sole writer of terminal external activation states (`completed`, `rolled_back`, `rollback_failed`). Studio startup may observe and publish durable state, but it never promotes `external_activating`, `external_activated`, or `rolling_back` to a terminal result from local executable/pointer identity alone. Schema v2 adds a monotonic journal revision and an activation-protocol field; Fleet holds an OS-level activation lock and rejects stale journal revisions. Before handoff, Studio queries `fleetctl studio-contract --json` and requires protocol v2, schema-v2 support, process-bound readiness, and cross-process locking.

A host reboot does not imply unattended launch policy: Aira currently has no Studio launchd owner. The journal remains recoverable, and a later deployed Studio/service start plus repeated apply can resume the transaction. Automatic boot service installation is outside M5.10.

## Security consequences

- Browser input cannot select launcher/executable/activation/rollback paths or PID.
- Candidate and release trees reject symlinks and unsupported entries and are bounded by file count/bytes.
- The release fingerprint binds backend and dashboard files as one unit.
- `current` must be a relative two-component symlink into the trusted releases directory.
- Launcher environment is minimal and stdout/stderr are not exposed through the browser.
- Local config/data are not release-owned.
- A source/debug Studio cannot initiate self-update for the deployed runtime.
- External activation runs with the same user privileges as Studio/Fleet and adds no privilege escalation.
- Failure after destructive pointer switch attempts rollback before reporting terminal failure.

## Consequences

Studio self-update now has a durable asynchronous transaction boundary suitable for the M5.11 Updates UI. The browser should treat `activation_pending` as a reconnect boundary and query the same transaction ID after Studio returns.

The current implementation intentionally uses the deployed Fleet Python control bundle instead of installing a launchd job. This satisfies the small trusted external-launcher design while keeping service-installation policy outside M5.10.
