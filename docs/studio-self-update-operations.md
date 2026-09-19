# Studio Self-Update Operations

## Ownership

Studio owns trusted release discovery/staging, candidate validation, durable transaction metadata, and the handoff request. The deployed Fleet control bundle owns external activation through the fixed `studio-activate` operation.

Studio does not kill or replace itself directly.

## Runtime layout

```text
runtime/studio/
├── current -> releases/vX.Y.Z
├── releases/vX.Y.Z/
│   ├── mcp-studio
│   └── web/dist/
├── studio.toml
└── data/
    └── self-update/<transaction>.json
```

`studio.toml` and `data/` remain host-local.

## Normal self-update

1. Studio verifies/stages the selected project release.
2. Candidate backend version and matching `web/dist/index.html` are validated and fingerprinted.
3. Studio persists the durable transaction.
4. Studio starts the Fleet launcher with server-selected host ID, transaction ID, and Studio PID.
5. Studio returns `activation_pending` and gracefully exits.
6. Fleet waits for the old PID, promotes the candidate, switches `current`, and starts the replacement.
7. Fleet requires loopback `/health` to report `status=ok`, `service=mcp-studio`, and the exact target version.
8. The new Studio finalizes durable state after startup/reconnect.

## Failure / recovery

- Target launch/health failure restores the persisted previous versioned release or the legacy flat fallback.
- `rollback_failed` blocks later self-update until operator repair.
- Stale `preparing` is recovered at startup and no longer blocks future updates.
- Interrupted `activation_pending` can be re-armed with a new source PID after candidate/release revalidation.
- Browser state is only a reconnect hint; the durable self-update journal remains authoritative.

## launchd status

M5 defines a launchd-compatible external-supervisor boundary but does not automatically install a Studio launchd job on Aira. Current M5 activation uses the constrained one-shot Fleet launcher. Automatic service installation/unattended restart policy remains outside M5.

## Source install compatibility

Fleet source-built `install --component studio` is bootstrap-only once `runtime/studio/current` exists. It refuses to overwrite the legacy fallback binary after versioned self-update ownership has been established.
