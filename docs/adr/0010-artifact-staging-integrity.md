# ADR 0010: Verified artifact staging before activation

- Status: Accepted
- Date: 2026-09-17

## Context

M5.2–M5.4 establish trusted release discovery, exact provider/repository identity, host-platform normalization, and deterministic archive selection. The update system still needs a boundary that converts selected remote metadata into local bytes that are safe enough for later activation without touching the active installation.

Release archives are untrusted input even when their URL came from trusted provider policy. They may be truncated, checksum-mismatched, oversized, contain path traversal, links/special files, ambiguous paths, or a package layout that does not match the component contract. Tunnel ZIPs additionally lose executable permission information in common workflows and require a narrowly scoped staged binary version check.

## Decision

Studio adds `ArtifactStager` in `src/update/staging.rs`.

The staging root is derived only from trusted `ComponentCatalog::runtime_root()` and is fixed below:

```text
<runtime_root>/.mcp-studio-staging/
```

An individual staging transaction is created as `.partial-*`. Partial directories are automatically removed on failure by temporary-directory ownership and stale `.partial-*` directories are cleaned when the stager is constructed. A successful transaction is renamed to a unique `ready-*` directory. `ArtifactStager` exposes no API that mutates a ready directory after validation.

The staging pipeline is:

1. Re-resolve trusted component policy and provider identity.
2. Use the existing M5.4 `ReleaseProvider::select_asset()` exact selector.
3. Revalidate the selected GitHub release-download URL against trusted owner/repository policy.
4. Retrieve the bounded checksum manifest through the provider.
5. Require exactly one valid SHA-256 manifest entry for the selected asset.
6. Stream the HTTPS artifact into the partial staging directory with a 256 MiB compressed-byte limit while computing SHA-256 over exact downloaded bytes.
7. Reject checksum mismatch before extraction.
8. Extract into the same partial transaction with a 4,096-entry and 1 GiB expanded-byte limit.
9. Reject absolute/traversal paths, ambiguous separators, duplicate paths, symlinks, hardlinks, special file types, and unsupported ZIP entry types.
10. Validate the exact component package layout without recursive binary searching.
11. Normalize expected executable permissions.
12. For official tunnel runtime-cloudflared only, execute the checksum-verified staged binary as `<binary> --version` with a five-second timeout and an empty environment, requiring the reported semantic version to match the release.
13. Atomically write `staged.json` and rename the partial transaction to a unique `ready-*` directory.

`StagedArtifact` records component, semantic version, provider identity, release tag, platform, selected asset name, archive SHA-256, final staging path, validated package root, validated executable paths, and verification timestamp.

Package validation is closed by current component class:

- project MCP tarballs: exact top-level archive root, expected binary, optional `README.md`/`LICENSE` only;
- Studio tarball: exact top-level root, `mcp-studio`, and `web/dist/index.html`, with dashboard content confined under `web/`;
- Fleet tarball: exact current bundle files/directories and `VERSION` equal to release semantic version;
- official tunnel runtime-cloudflared ZIP: exact root-level runtime binary, `cloudflared`, manifest, `LICENSE`, `NOTICE`, release-specific license report, and SPDX file.

No active install path is used as a staging destination and M5.5 performs no process stop/restart, activation, or rollback.

## Alternatives considered

### Extract directly into the active install location

Rejected. A corrupt or malicious archive could partially replace the running installation before verification completed and would violate the M5 rollback/atomicity invariants.

### Trust the checksum manifest but skip archive structure validation

Rejected. A valid checksum authenticates bytes relative to the release manifest; it does not make traversal, links, special files, or an unexpected package layout safe for local installation.

### Search recursively for a plausible executable after extraction

Rejected. Recursive discovery makes ambiguous/unexpected package layouts executable. The expected archive structure is derived from trusted component packaging policy and release contract instead.

### Execute every staged project binary for version discovery

Rejected. M5.5 minimizes execution of downloaded code. The explicit exception is the official tunnel runtime where the existing Fleet updater already requires staged `--version` validation and the milestone contract calls for it.

## Consequences

M5.7 may treat a returned `StagedArtifact` as checksum-verified, safely extracted, structurally validated material located outside active install paths. It must still re-check that the staging path/metadata is intact before destructive activation and must own stop/activation/health/rollback transaction semantics.

M5.5 retains the verified archive and checksum manifest inside the ready transaction. Later cleanup/retention policy must account for disk usage; automatic retention pruning is not introduced here.

M5.5 does not add a browser/API staging endpoint. Staging remains an internal backend capability until later M5 orchestration/API milestones.