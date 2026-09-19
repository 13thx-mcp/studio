# ADR 0012 — Transactional flat Rust MCP update and rollback

## Status

Accepted for Milestone 5.7.

## Context

M5.5 can produce a verified `StagedArtifact`, and M5.6 can report runtime-artifact inventory/drift, but no milestone before M5.7 mutates an active installation. The first active update path must replace one project-owned flat Rust MCP binary below `bin_root` without affecting sibling binaries, preserve the MCP's actual running/stopped state, coordinate through the existing `Supervisor`, and retain a deterministic rollback path if activation or health verification fails.

Gateway is not a generic target because updating the control-path binary has extra availability requirements owned by M5.8. Studio, Fleet, and tunnel use different installation/lifecycle models and remain out of scope.

## Decision

Introduce `McpUpdateManager` for generic non-Gateway `McpBinary` components.

The public mutation surface is two-phase:

- `POST /api/updates/{component}/prepare` accepts only a semantic version. Studio resolves the trusted provider/repository/platform itself and produces a verified ready staging transaction.
- `POST /api/updates/{component}/apply` accepts only the server-generated transaction ID. It accepts no path, URL, binary name, repository coordinate, checksum, or command from the browser.

`GET /api/update-transactions/{transaction_id}` returns browser-safe in-memory transaction progress. Transaction events use the same safe DTO over realtime.

Preparation snapshots the installed source version and a staging fingerprint. Before apply, Studio reloads `staged.json` only from the server-owned staging root and revalidates component/provider/release/asset identity, archive SHA-256, extracted executable SHA-256 values, staging confinement, and the prepared fingerprint. The installed source version must also remain unchanged since preparation.

Apply obtains a per-component in-process lock. Unrelated components may update concurrently, while duplicate operations on one component fail with conflict.

For a generic flat MCP, Studio requires the registered supervisor executable to resolve to the same catalog-derived target path. It then:

1. snapshots actual supervisor running/stopped and enabled state;
2. copies the old binary to a server-controlled rollback sibling and writes a SHA-256 sidecar;
3. stops the MCP only when it was running;
4. copies the staged executable to a same-directory activation scratch file, fsyncs it, preserves executable permissions, and renames it atomically over only the target binary;
5. restarts only if it was running before and remains enabled;
6. verifies the activated binary's semantic version with bounded `--version` execution;
7. when restarted, requires the supervisor to report `Running` immediately and after a bounded health window;
8. marks the target version desired and removes rollback material only after verification succeeds.

On activation, restart, or verification failure, Studio enters rollback: stop the newly started runtime if necessary, checksum-validate the prepared rollback binary, atomically restore it, restore the prior running/stopped state, and re-run source-version plus running-health verification. Rollback failure is explicit.

Startup cleanup removes only unfinished activation scratch files and rollback `.partial` files. Finalized rollback binaries are preserved because they may be the only recoverable known-good state after an interrupted transaction.

## Security consequences

- Activation authority remains server-owned; browser input cannot select a filesystem or network target.
- M5.5 verification is not blindly trusted across time: apply re-hashes both the verified archive and extracted executable bytes before stopping a process.
- The target must be a regular non-symlink file directly under the configured `bin_root` and must equal the registered supervisor executable.
- Rollback material is a sibling of the exact target under trusted `bin_root`, uses create-new scratch semantics, and has its own digest sidecar.
- Activation never shells out and never mutates unrelated flat-bin files.
- API/realtime transaction errors are projected to safe categories; raw internal paths/provider details are not returned to the browser.

## Verification boundary

M5.7 mandatory verification is stronger than process existence: exact semantic `--version` plus a bounded supervisor running-health window for previously running MCPs. The current Supervisor does not expose an MCP JSON-RPC stdin request channel, so M5.7 does not claim protocol-level `initialize` verification. A later lifecycle/protocol abstraction can strengthen this without changing the transaction authority model.

## M5.8 reuse

M5.8 should reuse trusted release preparation, staging revalidation/fingerprinting, per-component locking, rollback material helpers, atomic same-directory replacement, safe transaction DTO/events, and version/health verification primitives. It must override the orchestration policy for Gateway so the control path is not treated as an ordinary child MCP restart.
