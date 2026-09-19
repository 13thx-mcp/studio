# Update and Rollback Operator Guide

## Principles

- Updates are manual in M5.
- Release/component authority is server-owned.
- Verify and stage before lifecycle mutation.
- Preserve prior running/stopped state.
- Prepare rollback material before destructive activation.
- Never use source Git HEAD as installed runtime truth.
- Host-local config/secrets/state are outside release payload ownership.

## Operator flow

1. Run **Check releases** in Updates/Fleet.
2. Inspect installed/running/desired/latest versions and drift.
3. Select an approved semantic version.
4. Review the confirmation dialog for lifecycle/reconnect impact.
5. Prepare/stage the release.
6. Apply only the staged server-generated transaction ID.
7. Observe typed transaction phases and final rollback result.

## Component behavior

### Ordinary MCP binaries

Running MCPs are stopped, atomically replaced, restarted, and verified. Stopped MCPs remain stopped. Verification/start failure restores the known-good binary and previous running state.

### Gateway

Gateway production ownership remains with the Tunnel runtime. Gateway update stops/restarts that owner as needed, preserves `servers.d`, runs a standalone MCP protocol/catalog verification, verifies reconnect, and automatically rolls back on failure.

### Fleet

Fleet is a control bundle, not a persistent child process. Bundle updates preserve active host profile/state and validate generated Gateway/Studio/Tunnel config before success.

### Tunnel

Tunnel updates use official OpenAI releases only. The transaction:

```text
verified staging
→ preserve host-local config/credential fingerprint
→ stop only if currently running
→ activate versioned release/current pointer
→ restart only if previously running
→ verify installed version + stable lifecycle state
→ complete or rollback
```

Selecting the installed Tunnel version is intentionally supported as a **verified same-version force reinstall**. This replaces only the versioned runtime release, preserves `config.yaml` and credentials, and is useful for integrity repair when no newer official version exists.

### Studio

Studio cannot safely replace itself in-process. Studio prepares and persists the transaction, then hands activation to Fleet `studio-activate`; the external launcher switches the versioned backend+web release and verifies `/health`. Browser disconnect/reconnect is expected.

## Reconciliation

Only `managed_safe_drift` with `safe_to_reconcile=true` may be applied directly. Unknown local edits are not overwritten. The reconciliation transaction snapshots generated config, applies the Fleet pure render, verifies local Gateway catalog identity, preserves Tunnel running/stopped state, and rolls back on post-write verification failure.

## Rollback interpretation

There is no generic manual browser rollback endpoint in M5. Rollback is part of the backend transaction contract.

- `failed` + `rollback_succeeded=true`: attempted update failed; prior runtime was restored.
- `rollback_failed`: automatic recovery could not be proven. Stop further updates and repair runtime state before retrying.
- Studio may report `rolled_back` from its durable external launcher journal.

## Rollback-failed response

1. Do not submit another update for the affected component.
2. Inspect installed/current pointer and component health using trusted local operator access.
3. Preserve host-local config/state before manual repair.
4. Restore the last verified release/pointer only from trusted release/rollback material.
5. Re-run inventory/health/catalog verification.
6. Clear the incident only after the backend reports a coherent installed/running identity.

## Never do

- Do not paste arbitrary release URLs into the browser/API.
- Do not move a release outside the trusted runtime root and point `current` at it.
- Do not overwrite Fleet host profiles, Studio `data/`, Tunnel credentials, or generated config from release archives.
- Do not treat a healthy process as proof that canonical generated config is synchronized.
- Do not bypass checksum/package validation for an emergency update.
