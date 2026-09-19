# ADR 0014 — Fleet Control Bundle Update Ownership and Local-State Preservation

## Status

Accepted for M5.9.

## Context

Fleet is an architecture-independent control bundle installed under `runtime/fleet/`. Its published release archive contains generic control content:

- `VERSION`
- `fleet.toml`
- `README.md`
- `scripts/fleetctl.py`
- `hosts/mirin.example.toml`

The deployed runtime also contains host-local identity and mutable state, notably an active `hosts/<host>.toml` profile and optionally `state/`. Those files are not release payload and must survive an update unchanged.

Aira currently has a legacy deployed Fleet bundle created by the older `deploy-control` path that lacks `VERSION`, even though the published release package includes it. Therefore a safe updater must be able to repair a version-unidentified legacy bundle without using source Git state as installed-version truth.

Fleet also renders Gateway, Studio and tunnel runtime configuration. A replacement bundle cannot be considered compatible merely because its files have a valid checksum; its current schema/tooling must prove that the active host profile still renders the already-deployed runtime configuration model.

## Decision

MCP Studio owns Fleet update as a **data/control-bundle transaction**. It does not invoke Fleet to update itself and Fleet does not update Studio inside the same transaction.

### Trusted release and staging

Fleet uses the existing trusted `13thx-mcp/fleet` provider, architecture-independent asset resolver and M5.5 staging/integrity path. Prepare stores both the common staged identity and SHA-256 fingerprints for every generic Fleet control file, not only `fleetctl.py`.

Apply reloads the server-owned `ready-*` staging record and requires those fingerprints to remain unchanged before any runtime mutation.

### Generic versus local boundary

Release-owned generic paths are exactly:

```text
VERSION
fleet.toml
README.md
scripts/fleetctl.py
hosts/mirin.example.toml
```

Host-local mutable state is:

- exactly one active non-example `hosts/<host>.toml` profile;
- optional regular-file content below `state/`.

Unexpected top-level runtime entries, symlinks and unsupported state entries fail closed instead of being silently discarded.

The active host profile and local state are copied byte-for-byte into the candidate bundle after the verified release generic files are copied, and their permission bits are preserved. The release example profile can never overwrite the active profile.

### Schema compatibility

M5.9 supports:

- Fleet schema version `2`;
- host profile schema version `1`.

The active profile filename must equal `<host_id>.toml`, and its `bin_root` and `runtime_root` must canonicalize to Studio's trusted catalog roots.

No automatic schema migration is implemented in M5.9. A different Fleet or host schema is rejected **before destructive activation**. Any future migration must be an explicit versioned migrator with a durable local-profile backup; silent profile rewriting is prohibited.

### Render compatibility proof

Before activation, Studio runs the staged candidate's own `fleetctl.py` with the preserved active profile:

```text
render-gateway --host <host> --check
render-studio  --host <host> --check
render-tunnel  --host <host> --check
```

The commands run with fixed arguments, bounded timeouts and a minimal environment. They are read-only checks; generated runtime configuration is not rewritten by Fleet update.

The same validation is repeated against the activated bundle before success and against the restored bundle during rollback verification.

### Activation and rollback

Studio builds a complete candidate directory as a sibling of `runtime/fleet`. Activation uses same-filesystem directory renames:

```text
runtime/fleet
  -> .mcp-studio-fleet-rollback-<txn>

.mcp-studio-fleet-candidate-<txn>
  -> runtime/fleet
```

The old bundle therefore remains intact as rollback material. Success removes rollback material only after version, local-state and render verification pass.

If post-activation verification fails, the new bundle is quarantined, the prior directory is renamed back into place, local state is compared byte-for-byte, the prior optional version identity is checked, and the old tooling reruns render validation. Failure to prove rollback becomes `rollback_failed`.

Startup removes stale pre-activation candidate/failed directories. If `runtime/fleet` is missing and exactly one finalized Fleet rollback directory exists, it is restored automatically. Ambiguous finalized rollback material is preserved rather than guessed.

### Legacy bundle repair

If the active Fleet directory exists and its local profile is valid but `VERSION` is absent, prepare is allowed to stage an explicit trusted version. Apply still captures and preserves the entire prior runtime directory for rollback. This repairs the current legacy Aira deployment without reading source Git metadata.

When an installed `VERSION` exists, normal semantic-version downgrade/equal-version rejection applies.

## Consequences

- Fleet update works on source-less runtime hosts with Python but without Rust/Cargo, Node.js or source repositories.
- Host identity/configuration stays outside release ownership.
- Render compatibility is validated by the exact staged Fleet tooling rather than reimplemented incompletely in Studio.
- Generated Gateway/Studio/tunnel configs remain untouched during Fleet update.
- Directory activation has a very small rename interval where `runtime/fleet` may not exist if the process is interrupted; startup recovery restores the single known rollback directory when unambiguous.
- M5.9 intentionally has no host-profile migration engine. Schema changes require a future explicit migration contract rather than implicit conversion.
