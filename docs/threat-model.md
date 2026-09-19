# MCP Studio Threat Model

## Scope

MCP Studio is a privileged local control plane. As of Milestone 5.10 it can supervise registered MCP child processes, manage one secure tunnel runtime, persist MCP registration/configuration locally, discover/select/stage trusted releases, expose runtime inventory/drift, transactionally update generic flat Rust MCP binaries and Gateway, update Fleet, reconcile generated runtime config, and hand Studio self-update activation to a constrained external Fleet launcher. Tunnel runtime update remains later M5 scope.

SQLite history/metrics/audit persistence, automatic restart/backoff, remote authentication/RBAC, and MCP gateway traffic telemetry remain later milestones.

## Assets

- Host process execution capability.
- Persistent MCP registry/configuration.
- Configured MCP-root authority boundary.
- Managed child identity/PIDs and transient runtime state.
- MCP source trees and executable artifacts.
- Tunnel runtime/configuration and secret references.
- Operational stdout/stderr logs.
- Browser lifecycle/registry/discovery-control capability.

## Trust boundaries

1. Browser/operator ↔ Studio localhost HTTP/WebSocket API.
2. Studio registry/discovery service ↔ configured MCP root and project manifests.
3. Persistent registry file ↔ live registry service.
4. Registry service ↔ MCP supervisor launch configuration.
5. Studio ↔ managed MCP child processes.
6. Studio ↔ managed tunnel child process and server-side secret references.
7. Tunnel runtime ↔ external tunnel provider/control plane.
8. Future update orchestration ↔ server-owned component catalog and trusted host runtime roots.
9. `github_13thx` release provider ↔ fixed GitHub API origin plus server-owned `13thx-mcp` repository policy.
10. `github_openai` tunnel release provider ↔ fixed GitHub API origin plus exact `openai/tunnel-client` repository policy.

## Active M5.12 security properties

- Studio remains loopback-only.
- Privileged browser mutations and WebSocket upgrades enforce same-origin `Origin`/`Host` alignment.
- No API accepts raw shell commands or PIDs.
- Discovery never executes project code.
- Discovery never auto-registers or auto-starts a project.
- Registration requires explicit operator action and server-side candidate rescan.
- Persistent registry uses an explicit schema version and fails safely on malformed/unsupported state.
- Registry updates are complete-document atomic replacements; failed writes do not publish the candidate in memory.
- MCP project, working-directory, and executable paths are project/root confined.
- Absolute paths, traversal/root/prefix components, and symlink components are rejected.
- Executable/working-directory policy is revalidated immediately before spawn.
- Executable and arguments are structured data passed directly to `tokio::process::Command`; no shell interpolation is used.
- Disabled MCPs cannot start/restart.
- Edit, disable, and unregister are rejected while the MCP is active.
- Unregister removes Studio registration only and accepts no caller-supplied delete path.
- Studio signals only PIDs returned by children it spawned and currently tracks.
- Registry API/realtime payloads omit stored environment values.
- Tunnel remains a separate constrained lifecycle/secret domain with M3 log redaction controls.
- Recent logs and realtime fan-out remain bounded.
- Update component IDs form a closed typed set; unknown IDs fail closed.
- Release provider/repository ownership and install targets are server-side catalog policy.
- Install paths derive from trusted `bin_root`/`runtime_root`, not browser fields.
- Browser-safe update projections omit internal repository URLs, download/checksum URLs, install paths, binary names, and secrets.
- Release versions use validated semantic-version parsing and explicit precedence comparison that ignores build metadata for freshness.
- The `github_13thx` provider re-resolves component identity through `ComponentCatalog`; caller-mutated repository coordinates are ignored.
- Production release discovery uses fixed HTTPS GitHub API endpoints with bounded connect/request timeouts, bounded response bodies, and a fixed User-Agent.
- Stable-channel discovery rejects draft releases and prereleases, including explicitly requested prerelease versions.
- Release metadata must contain exactly one `SHA256SUMS.txt` uploaded asset.
- Returned release asset URLs must be HTTPS `github.com` release-download URLs under the catalog-owned owner/repository path.
- Release JSON is capped at 1 MiB and checksum-manifest retrieval at 256 KiB.
- No authorization token is accepted from the browser or logged; public release discovery requires no credentials in M5.2/M5.3.
- The `github_openai` provider accepts only the catalog-owned `tunnel` component and exact `openai/tunnel-client` source; a caller-mutated source cannot redirect it to the archived project placeholder or another repository.
- Tunnel release metadata must contain an unambiguous runtime-cloudflared ZIP family whose embedded version matches the release tag; evidence sidecars and source archives are not treated as runtime candidates.
- M5.3 exposes no update/download/activation API, downloads no release archive, performs no extraction, and changes no runtime installation.
- Host platform normalization is centralized: only Darwin/macOS plus amd64/arm64 aliases are accepted; unsupported OS/architecture values fail closed.
- Every component has a server-owned release asset contract; Fleet architecture independence is explicit policy rather than inferred from filenames.
- Asset selection derives one exact expected filename from trusted component policy, semantic release version, and typed platform. It never falls back across architectures or uses substring/closest matching.
- Missing exact assets, duplicate exact assets, and release/component mismatches fail closed before artifact download.
- M5.4 adds no browser asset-selection input; M5.5 consumes only the exact asset selected from server-owned policy.
- Artifact staging is confined below trusted `runtime_root/.mcp-studio-staging` and never writes into active component install paths.
- Artifact download is HTTPS-only, streamed to a partial file, bounded to 256 MiB compressed bytes, and hashed while written.
- `SHA256SUMS.txt` must contain exactly one valid SHA-256 entry for the selected asset and the exact downloaded digest must match before extraction.
- Extraction is bounded to 4,096 entries / 1 GiB expanded bytes and rejects absolute/traversal paths, ambiguous separators, duplicate paths, symlinks, hardlinks, and special/unsupported file types.
- Package validators require the exact current MCP/Studio/Fleet/tunnel structures; they never recursively search for a plausible executable.
- Expected executable permissions are normalized only inside the staging tree.
- The only downloaded-code execution in M5.5 is checksum-verified official tunnel runtime `--version`, with an empty environment and a five-second timeout.
- Failed partial staging transactions are automatically removed and stale `.partial-*` directories are cleaned before new staging work.
- Successful staging records typed `staged.json` identity in a unique `ready-*` transaction; M5.5 does not stop/restart/activate/rollback any runtime.
- Installed inventory truth comes from catalog-derived deployed artifacts (`--version`, Fleet `VERSION`, tunnel `current`) rather than source manifests or Git HEAD.
- Update roots and desired-version pins are server-side configuration only; browser requests cannot supply probe paths, repositories, versions, or executable locations.
- Running version remains separate and `Unknown` unless Studio can prove it; inventory never assumes installed equals running for managed processes.
- `POST /api/updates/check` is same-origin protected and performs release metadata checks only; it does not download, stage, stop, restart, or activate components.
- Release check failures preserve local inventory and are serialized only as sanitized categories/status codes, never raw provider URLs or response details.
- Browser inventory DTOs omit install paths, repository owner/name, download/checksum URLs, binary names, headers, and secrets.
- Source-present/runtime-only mode is informational only and does not change installed-version truth.
- M5.7 prepare/apply endpoints accept only typed component/version or server-generated transaction ID; no browser path, URL, repository, binary name, checksum, or shell command is accepted.
- Gateway and non-`McpBinary` components are rejected by the generic M5.7 transaction path.
- Apply revalidates the server-owned ready staging record, archive SHA-256, extracted executable SHA-256 values, path confinement, prepared staging fingerprint, and unchanged source version before stopping a process.
- The registered supervisor executable must resolve to the same catalog-derived flat-bin target; source-build or alternate registry executables cannot be silently updated.
- Update locking is per component: duplicate operations on one MCP conflict while unrelated component locks are independent.
- Rollback material is created before stop/activation, is server-controlled beside the target, and is checksum validated before restoration.
- Activation copies to a same-directory create-new scratch file and atomically renames only the exact target binary; sibling `bin_root` files are untouched.
- Previously stopped or disabled MCPs are never auto-started; running/stopped state comes from the Supervisor, not desired policy.
- Successful activation requires exact semantic `--version` verification and, when restarted, a bounded Supervisor running-health window. Successful rollback re-verifies the source version and prior running-health state.
- Transaction/release/staging errors exposed through HTTP/realtime are sanitized categories and do not include internal paths or release URLs.
- Startup deletes unfinished activation and rollback `.partial` scratch files but preserves finalized rollback material for interrupted-transaction recovery.
- Gateway activation is owned by a dedicated manager; the generic M5.7 MCP Supervisor path still rejects Gateway.
- Tunnel ownership is verified from server-owned YAML: exactly one `main` command must bind the trusted flat Gateway binary to the trusted `runtime/gateway/servers.d`; the browser cannot choose either path.
- Gateway `servers.d` is snapshotted by filename/content SHA-256 plus child enabled/command metadata and must be byte-for-byte unchanged before success or rollback success.
- Known managed Gateway child commands must canonicalize to their catalog-derived flat `bin_root` binaries, preventing source-tree/nested-bin fallback.
- A running tunnel is stopped only through `TunnelSupervisor`; a stopped tunnel is never auto-started merely for Gateway update.
- The replacement Gateway is protocol-probed as a temporary standalone stdio server before tunnel reconnect: initialize, tools/list, built-in control tools, structured child status, configured/enabled counts, enabled-child running health, and non-empty child tools where required.
- Intentional `Starting`/`Stopping` owner states are bounded reconnect states, not immediate permanent failures; reconnect has a timeout and a stable-running health window.
- Gateway rollback re-probes the restored source binary/catalog before restoring the prior tunnel ownership state; reconnect/rollback failure is explicit.

## Primary threats and controls

### Untrusted release/update authority

**Risk:** a browser or later API caller supplies an arbitrary repository, download URL, checksum URL, executable path, or install target and turns Studio into a generic downloader/installer.

**Controls:**

- `ComponentId` is a closed set of managed identities.
- `ComponentCatalog` owns provider, repository, install target, channel, and restart policy server-side.
- `HostRuntimeRoots` supplies trusted flat `bin_root` and `runtime_root`; install paths are derived from catalog policy.
- Browser-safe component views do not contain internal URL/path authority.
- Provider interfaces receive `ComponentPolicy` from the trusted catalog rather than caller-provided repository coordinates.

**Residual risk:** M5.2–M5.4 validate provider identity, repository binding, uploaded asset URLs, required asset families, supported host identity, and exact platform/archive selection, but they do not yet verify downloaded archive checksums, archive contents, or activation safety. Those controls remain M5.5 and later transactional-update work.

### Malicious or malformed GitHub release metadata

**Risk:** a compromised repository release, malformed API response, or redirect/asset metadata attempts to make Studio consume an unexpected host, oversized body, draft/prerelease artifact, ambiguous checksum manifest, or invalid tag.

**Controls:**

- API request URLs are constructed internally from a fixed `https://api.github.com` base plus catalog-owned owner/repository names.
- Provider lookup re-fetches trusted policy by `ComponentId`; it does not trust the `source` fields on a supplied `ComponentPolicy` instance.
- Release tags must use the trusted provider `vMAJOR.MINOR.PATCH` convention and parse as semantic versions.
- Stable-channel operations reject draft/prerelease metadata and explicit prerelease requests.
- Release asset metadata requires non-zero immutable GitHub release/asset IDs, non-empty names, exactly one `SHA256SUMS.txt`, and trusted HTTPS release-download URLs under the expected repository path.
- Metadata/checksum responses have explicit byte caps and network timeouts.
- Provider errors distinguish unreachable transport, HTTP status, malformed metadata, invalid tag, no stable release, requested-version absence, checksum-manifest ambiguity, untrusted asset URL, missing required asset family, and ambiguous required asset family.

**Residual risk:** M5.2/M5.3 trust GitHub and the configured repository publishers (`13thx-mcp/*` for project components and only `openai/tunnel-client` for tunnel). Cryptographic integrity of archives is not established until M5.5 verifies the published checksum manifest against downloaded artifacts. Tunnel-client publishes additional SPDX/provenance evidence, but verification of those artifacts is not introduced by M5.3.

### Wrong-platform or lookalike archive selection

**Risk:** release metadata contains multiple architectures, stale versions, sidecars, similarly named products, or duplicate filenames and Studio selects an archive that is not the exact host/runtime target.

**Controls:**

- Supported host identity is a closed typed set for M5.4: Darwin amd64 and Darwin arm64 only.
- Component packaging policy is server-owned and includes the exact asset stem and packaging kind.
- Expected archive names are generated from trusted policy plus `Version` and `Platform`; the selector compares complete filenames for equality.
- No amd64/arm64 fallback, substring matching, or sidecar/source/full-client substitution is permitted.
- Architecture-independent Fleet packaging is explicitly declared and tested for both supported host architectures.
- Zero and duplicate exact matches have distinct typed failures.

**Residual risk:** M5.4 selects metadata only. M5.5 must verify the selected bytes against `SHA256SUMS.txt` and validate archive structure/content before staging.

### Malicious or corrupt release archive staging

**Risk:** a selected release archive is truncated, oversized, checksum-mismatched, contains path traversal/links/special files, duplicates paths, or uses an unexpected package layout to write outside staging or prepare attacker-controlled activation material.

**Controls:**

- Staging revalidates provider identity and the selected GitHub release-download URL against catalog owner/repository policy before download.
- Compressed and expanded resource limits bound disk/resource consumption per transaction.
- SHA-256 is computed over exact streamed bytes and compared to the unique matching manifest entry before extraction.
- Archive paths are normalized into a newly created partial transaction and unsafe entry types are rejected before writing.
- Extraction uses create-new file semantics and duplicate normalized paths are rejected.
- Component-specific validators enforce closed package roots/required files; unexpected MCP, Studio, Fleet, and tunnel entries fail closed.
- Partial directories are temporary-owned and removed on failure; active install paths are never staging destinations.
- A ready transaction receives typed verification metadata only after download, checksum, extraction, structure, permission, and required tunnel version checks all succeed.

**Residual risk:** M5.5 does not provide cryptographic publisher signatures beyond the release SHA-256 manifest and trusted GitHub repository boundary. It retains verified staged archives, so M5.7/later cleanup policy must manage disk retention. Activation TOCTOU protection and rollback remain M5.7+.

### Flat MCP update activation and rollback race

**Risk:** a caller races staging/activation, changes a staged executable after verification, redirects the update to another binary, loses the previous executable, restarts a previously stopped MCP, or leaves the host with an unverifiable new binary after failure.

**Controls:**

- Preparation resolves the release/provider/platform from the closed catalog and records a process-local staged fingerprint.
- Apply accepts only the transaction ID, reloads only a `ready-*` transaction below the trusted staging root, and verifies archive plus extracted executable hashes before lifecycle mutation.
- Installed source version is re-probed and must still match the preparation snapshot.
- Registry executable and catalog target must canonicalize to the same regular non-symlink file directly under trusted `bin_root`.
- Per-component locks exclude duplicate prepare/apply operations.
- Known-good rollback bytes and their SHA-256 sidecar are finalized before the target is stopped or replaced.
- New bytes are copied/fsynced to a same-directory scratch path before atomic rename.
- Lifecycle stop/start is delegated to the existing Supervisor; only a previously running, enabled MCP is restarted.
- Activated and restored binaries must report the expected semantic version; running services must remain `Running` through a bounded health window.
- Failure after activation enters rollback, and failure of rollback itself becomes explicit `rollback_failed` state.

**Residual risk:** transaction records and locks are process-local in M5.7. A host crash after finalized rollback preparation may require operator/later-milestone recovery using the preserved rollback material; M5.7 startup removes only clearly disposable partial scratch files. Verification is version plus Supervisor health, not MCP JSON-RPC `initialize`, because the current Supervisor owns stdin without a request/response protocol channel. Gateway control-path continuity remains M5.8.

### Gateway control-path ownership and reconnect failure

**Risk:** updating Gateway through the wrong process owner can create two supervisors, replace a binary without reconnecting the actual tunnel-owned process, silently use a source-tree or attacker-selected config path, lose generated `servers.d`, or report success while children/tools are unavailable. A reconnect request can also be mistaken for a crash or hang indefinitely.

**Controls:**

- M5.8 proves the deployed ownership relation before mutation: tunnel-client/TunnelSupervisor owns the Gateway process, and its server-owned YAML must contain exactly one trusted `main` Gateway command.
- Browser requests carry only component/version or transaction ID; Gateway/config/tunnel paths are derived from trusted catalog/runtime configuration.
- Generated `servers.d` files are regular non-symlink files under the trusted runtime directory, are bounded in count/size, are SHA-256 snapshotted, and are never overwritten by release activation.
- Known managed child commands are canonicalized against flat catalog targets before stopping the control path.
- Only a previously running tunnel is stopped/restarted. Transitional reconnect is bounded; stopped/failed terminal states fail closed.
- A temporary standalone Gateway probe performs MCP initialize, tools/list, built-in control-tool validation, and child catalog/status verification before the production tunnel is reconnected.
- New Gateway probe failure, tunnel start failure, reconnect timeout/health failure, or config mutation triggers known-good binary rollback and old Gateway protocol/catalog verification.
- Rollback failure is explicit and cannot be converted to success.

**Residual risk:** Gateway transaction records and locks remain process-local. The standalone probe briefly launches a second child catalog while the production tunnel-owned Gateway is stopped; child MCPs therefore need to tolerate that bounded probe. Reconnect health proves the tunnel owner reaches/stays `Running`, but Studio does not independently query the remote OpenAI control plane for end-to-end request delivery. A process/host crash after destructive activation may still require recovery from preserved finalized rollback material.

### Fleet release payload versus host-local state confusion

**Risk:** a Fleet release or update transaction overwrites the active host profile with an example, loses local state/secrets, trusts source Git as installed identity, silently migrates an incompatible schema, or activates tooling that would generate different Gateway/Studio/tunnel runtime config.

**Controls:**

- Release-owned Fleet paths are closed and already structure/checksum validated by M5.5; browser requests cannot provide bundle/profile paths.
- M5.9 fingerprints every generic staged file and rechecks those hashes immediately before candidate creation.
- Active host profile selection is server-side and deterministic: exactly one non-example TOML profile, schema 1, filename matching host_id, and trusted root coordinates.
- Release example profiles never replace the active profile; local profile bytes and optional bounded state files are copied after generic release content and compared byte-for-byte before success and after rollback.
- Unknown Fleet/host schema fails before activation. There is no implicit migration path in M5.9.
- Staged and activated fleetctl.py must render-check current Gateway/Studio/tunnel configs with no write mode before the transaction can succeed.
- A missing legacy VERSION may be repaired only through an explicit trusted release transaction; source Git state is not substituted for runtime identity.
- Activation retains the old complete Fleet directory under a server-generated rollback path. Failed post-activation validation restores that directory and re-runs local-state/render verification.

**Residual risk:** M5.9 assumes a single active host profile per deployed runtime bundle and Python 3 availability because Fleet itself is Python tooling. A schema migration engine is intentionally absent; future schema changes need an explicit migration/backup ADR. Directory swap uses two same-filesystem renames, so a crash between renames can temporarily leave runtime/fleet absent; startup restores the single unambiguous finalized rollback directory.

### Fleet-generated runtime config drift / split launch-path masking

**Risk:** Fleet desired state, active generated config, helper launch arguments and live Gateway catalog diverge. A correct CLI override can mask stale canonical tunnel YAML, or Gateway can report a plausible aggregate tool count while the exposed tool identity/set differs from policy. Blind auto-repair could also overwrite legitimate local/secret-bearing edits or enter a restart loop.

**Active M5.9A controls:**

- desired bytes come only from the deployed trusted Fleet `render-plan` pure-render contract plus the server-selected active host profile; Studio validates schema, host, trusted runtime root, closed surface paths, ownership/effects and SHA-256 before use;
- generated targets are confined to trusted runtime roots and symlink/special-file targets fail closed;
- an atomically persisted managed-state manifest records last-known-managed hashes and generation; active bytes that do not equal either desired bytes or the managed baseline become an explicit unmanaged conflict;
- legacy auto-adoption occurs only when all active managed surfaces already exactly match trusted desired bytes; explicit adoption records current bytes without rewriting them;
- all supported Gateway launch surfaces must converge on the same flat-bin Gateway and trusted `runtime/gateway/servers.d` binding; the current non-Fleet `run.sh` is not emitted by Fleet, is validate-only, is accepted only as a complete explicitly supported canonical template, and is never automatically rewritten; comment-only matches, shadow assignments, alternate roots, duplicate/overriding args, symlinks and unrecognized shell forms fail closed;
- Studio records the canonical config source, digest of the exact bytes parsed at startup, and a process-instance identity. A persisted restart-pending marker clears only when a different process proves the canonical path and exact reconciled Studio-config digest; wrong path/old bytes/unknown identity cannot masquerade as active configuration, and Gateway-only changes cannot manufacture Studio restart drift;
- exact local Gateway catalog identity is proved from two independent observations: Studio directly probes every enabled trusted child MCP, consumes the complete paginated tool catalog, applies allowlist-before-prefix policy itself and builds the child+builtin expected set; a separately launched Gateway must expose exactly that set. Missing, extra, substituted or duplicate names, prefix collisions, invalid allowlists, disabled-child leakage and child commands outside trusted flat `bin/` fail closed;
- Gateway watcher reload/tunnel restart occurs only when the affected config changed and preserves prior running/stopped state;
- changed config is snapshotted into permission-confined rollback material before mutation; post-write failure restores bytes/runtime ownership state and re-verifies the catalog before rollback succeeds;
- reconciliation mutation endpoints are same-origin protected and the browser DTO omits config bytes, internal paths and execution authority;
- local Gateway synchronization is not represented as remote-client refresh unless an acknowledgement is observable;
- periodic unattended repair must add bounded retry/backoff and a drift-loop circuit breaker in M7.

**Residual risk:** connected ChatGPT/plugin catalog refresh may remain unobservable from Studio. Until a protocol acknowledgement exists, Studio can prove local Gateway/catalog state and emit refresh-required telemetry but cannot prove a remote session consumed the new catalog.

### Tunnel release activation and same-version repair

**Risk:** an upstream-runtime update replaces Tunnel with an untrusted or tampered artifact, mutates host credentials/config, escapes the versioned release root through `current`, restarts a previously stopped Tunnel, loses the previous runtime during same-version reinstall, or claims success after restart/rollback without proving runtime identity.

**Active M5.12 controls:**

- Tunnel release authority remains fixed to the catalog-owned official `openai/tunnel-client` provider; browser input is only semantic target version plus server-generated transaction ID.
- M5.5 verifies exact platform ZIP, SHA-256 manifest, safe extraction structure, executable permissions and runtime semantic version before prepare completes.
- apply reloads/revalidates ready staging and a bounded full candidate-tree fingerprint before any stop.
- activation mutates only `runtime/tunnel-client/releases/*` and the relative `current` pointer; host-local `config.yaml`, credential/state siblings and other non-release files are fingerprinted and must remain unchanged.
- `current` must be a relative `releases/vX.Y.Z` symlink resolving directly below the trusted releases root.
- a newer-version update promotes a new release directory and atomically switches `current`; downgrade remains rejected.
- same-version prepare/apply is accepted only for Tunnel as an explicit verified force-reinstall path and retains the prior release as transaction rollback material until health succeeds.
- before stop, the launcher must use the stable `current/tunnel-client-runtime-cloudflared` indirection plus canonical working/config paths; a fixed release binding is rejected.
- every owned spawn records generation, resolved executable/config/working directory and runtime SHA-256; a running update requires a new generation whose launch evidence matches the activated bytes through the health window, including same-version reinstall.
- previously running Tunnel is stopped/restarted only under the journaled transaction; previously stopped Tunnel remains stopped.
- the schema-versioned recovery journal under `runtime/studio/data/tunnel-update/` records trusted source/target fingerprints, prior ownership and mutation phase before destructive operations; terminal commit is persisted before rollback material cleanup.
- a post-pointer-rename sync failure is treated as a possible switch and enters compensation; the predecessor must be restored and verified before target deletion.
- activation/restart/verification/journal failure automatically restores the prior release and ownership state; inability to prove recovery is `rollback_failed` / `recovery_failed` with material retained.
- startup restores journaled uncommitted activation idempotently. Corrupt journals, unsafe journal paths, multiple recovery journals, and journal-less candidate/rollback/failed scratch fail closed and are preserved for operator repair.
- official-network closure smoke staged and force-reinstalled OpenAI `v0.0.14` in an isolated runtime while preserving config/credentials.

**Residual risk:** historical update persistence and retained-release garbage-collection policy remain later-milestone concerns. M5 keeps only transaction-local rollback material needed for safe activation and does not enable unattended Tunnel updates.

### Updates/Fleet browser authority and reconnect state

**Risk:** the dashboard invents client-side update authority, exposes internal paths/secrets, submits duplicate control-path applies after reconnect, loses one of several unrelated pending transactions, or presents unsupported rollback/tunnel actions as safe.

**Active M5.11 controls:**

- frontend update requests contain only the typed component path plus approved semantic version for prepare or server-generated transaction ID for apply;
- API tests assert prepare/apply bodies contain no path, URL, command, PID, or checksum authority;
- inventory/reconciliation/transaction DTOs remain browser-safe and the UI never renders raw config bytes, release URLs, install paths, or secret-bearing fields;
- transaction progress is rendered from backend phases, never a fake percentage or client-derived activation state;
- Gateway/Studio control-path phases are marked as expected reconnect states and duplicate apply is disabled while a transaction is non-terminal;
- every pending transaction reference is stored by transaction ID/component and all references are refetched after initial load/realtime reconnect; terminal completion removes only the matching reference;
- rollback is represented only from backend transaction results; no synthetic rollback mutation is exposed because no public rollback route exists;
- Tunnel remains inventory/release-check visibility-only because the backend has no tunnel activation manager;
- reconciliation apply is enabled only for server-reported `managed_safe_drift` with `safe_to_reconcile=true`; unmanaged conflict is inspect-only;
- sanitized backend error categories are mapped to operator-facing messages without reflecting internal request metadata;
- release checks run concurrently only over the closed server-owned component catalog/provider map, preserving the same trust/error policy.

**Residual risk:** browser local storage is convenience state, not update authority or durable audit history. A cleared browser store can lose automatic UI refetch hints, but server-owned durable Studio self-update state and live transaction APIs remain authoritative. Persisted historical/audit views belong to M6.
### Studio self-update activation ownership / rollback

**Risk:** Studio replaces its own executable in-process, a browser selects activation authority, backend and dashboard assets are activated from different releases, a candidate changes after verification, an unsafe current pointer escapes the runtime root, or failure after switching leaves Studio unable to recover.

**Active M5.10 controls:**

- Studio owns trusted release discovery/staging and a bounded server-named candidate; it never swaps or kills itself directly.
- Candidate identity binds M5.5 staging plus a bounded full release-tree fingerprint and required `web/dist/index.html`.
- Apply is accepted only from the deployed Studio executable (legacy flat or versioned `current`), never a source/debug process.
- Browser requests supply typed component/version and a server-generated transaction ID only; launcher path, executable, host profile, PID, release path and argv remain server-owned.
- Durable transaction metadata lives under `runtime/studio/data/self-update`, is atomically replaced, and survives reconnect/restart.
- Fleet `studio-activate` is a fixed external operation with minimal environment; it waits for the old PID, promotes only the trusted candidate, switches a relative `current -> releases/vX.Y.Z`, and launches with host-local `studio.toml`.
- Success requires loopback `/health` to report `status=ok`, `service=mcp-studio`, and the exact target version; backend and `web/dist` therefore activate as one release unit.
- `studio.toml` and `data/` remain outside release directories and are preserved across update/rollback.
- Fleet is the sole terminal-state writer for external Studio activation/rollback. Schema v2 uses a monotonic journal revision plus an OS-level activation lock to reject stale/concurrent writers. Every target/rollback spawn gets a fresh Fleet-generated nonce; after successfully binding the configured listener, the exact Studio process writes a server-derived regular proof file with nonce, PID, canonical loaded-config path and SHA-256 of the exact parsed config bytes. Fleet requires that proof, the expected release/binary fingerprint, the owned `Popen` instance and the stable health window to agree. An unrelated matching `/health` listener cannot complete the transaction even if another spawned child remains alive. Post-switch health, proof, launcher, journal, or readiness failure restores the persisted prior versioned or legacy-flat identity; rollback failure is explicit and blocks later self-update.
- Interruption recovery preserves the original previous-layout metadata and repeated apply re-arms the same durable transaction after fingerprint revalidation.
- Versioned installed identity validates `current` remains below `runtime/studio/releases` and that directory version, binary version, and web assets agree.

**Residual risk:** Aira currently has no Studio launchd owner. M5.10 uses a small trusted one-shot Fleet launcher and durable journal rather than installing a boot service. A host reboot may require the deployed Studio/service to be started again and the transaction re-armed; unattended boot/service policy is outside M5.10.
### Inventory path/version authority confusion

**Risk:** a browser request, source checkout, or stale provider result causes Studio to probe an arbitrary path or report a source/build version as the installed/running runtime identity.

**Controls:**

- `ComponentId` selects a server-owned catalog policy; inventory derives install paths only from trusted update roots and catalog install targets.
- Installed versions are read only from deployed runtime artifacts with strict semantic-version parsing.
- Source presence is reported separately and never contributes to installed version.
- Running version is independent and omitted when it cannot be proven; no installed==running assumption is made for MCP/tunnel processes.
- Desired versions are validated server-side semantic-version pins; absent pins preserve current installed identity for manual M5 operation.
- Latest release checks use the existing trusted provider mapping and keep a process-local check timestamp/result.
- Public inventory projections intentionally exclude internal path/repository authority and sanitize check errors.

**Residual risk:** M5.6 process-local inventory/check cache is not persisted and running identity is incomplete for MCP/tunnel processes until transactional lifecycle code captures stronger artifact identity. Persisted audit/history belongs to M6; M5.7 owns transaction-time state transitions.

### Version confusion and downgrade/replay

**Risk:** string comparison or unvalidated version text causes incorrect ordering, accidental downgrade, or inconsistent drift reporting.

**Controls:**

- Update-domain versions are parsed through a semantic-version type and freshness uses SemVer precedence rather than lexicographic or build-metadata ordering.
- Installed, running, desired, and latest version dimensions are modeled separately.
- Drift precedence is deterministic and unit-tested.

**Residual risk:** downgrade authorization, release freshness/replay policy, and persisted update history belong to later M5/M6 work.

### Malicious project manifest / discovery-triggered execution

**Risk:** a project under the MCP root uses manifest content to cause Studio to execute code during scan or approval.

**Controls:**

- Rust discovery parses `Cargo.toml` as data only.
- Node discovery parses `package.json` as JSON only and never executes scripts/install hooks.
- Python discovery parses `pyproject.toml` as data only and never imports modules.
- Discovery invokes no shell, package manager, compiler, interpreter, or project executable.
- Registration is separate from discovery and never starts the project.
- One malformed/unreadable project is isolated from unrelated scan results.

**Residual risk:** manifest parsing still consumes attacker-controlled local data; parser/library vulnerabilities and resource-exhaustion cases remain supply-chain/robustness concerns.

### Path traversal / MCP-root escape

**Risk:** editable project/executable/working-directory paths escape the configured authority root.

**Controls:**

- MCP root is canonicalized by Studio.
- Persisted project paths are relative to MCP root.
- Executable and working-directory paths are relative to the registered project.
- Absolute paths and `..`, root, or platform-prefix components are rejected.
- Canonical project must remain under MCP root.
- Canonical executable/working directory must remain under the project.
- The same policy is revalidated before every spawn.

### Symlink escape

**Risk:** a path that appears confined lexically resolves through a symlink to another host location.

**Controls:**

- M4 rejects symlink components in registered project, working-directory, and executable paths rather than treating a mutable symlink target as an authorization boundary.
- Canonical containment is checked as a second layer.

**Residual risk:** a local actor with write access to ordinary path components can race filesystem replacement between validation and spawn. Revalidation narrows the window; descriptor-based execution/openat confinement is a possible later hardening step.

### Executable-path abuse / arbitrary host execution

**Risk:** registry editing turns Studio into a general process launcher.

**Controls:**

- Browser configuration uses a project-relative executable path, not a shell command.
- Executable must remain inside the registered project.
- Discovery registration initially accepts only executable candidates produced by the server-side metadata scan.
- Arguments are a structured list; no shell interpolation is performed.
- Executable must be a regular file at activation time.

**Residual risk:** code inside an explicitly registered project is operator-approved execution authority. Registering or editing an executable should therefore be treated as a privileged action.

### Argument injection

**Risk:** structured arguments are interpreted as shell syntax or mutate process selection.

**Controls:**

- Arguments are passed directly as argv elements to `Command`.
- No shell is invoked by Studio.
- Executable selection is independent of argument text and remains path-confined.

**Residual risk:** a managed MCP may itself interpret dangerous arguments. M4 confines executable authority but does not understand every child-specific option grammar; operators remain responsible for reviewed argument values.

### Environment/secret leakage

**Risk:** inherited environment values become visible through registry REST, WebSocket events, or UI.

**Controls:**

- Public registry DTOs omit the environment map entirely.
- M4 adds no generic browser environment/secret editor.
- Registry/discovery realtime events are invalidations without full configuration payloads.
- Tunnel secrets retain M3 server-side references/redaction.

**Residual risk:** legacy static MCP environment values may still exist server-side and managed MCP stdout/stderr remains a separate secret-leakage domain without generalized redaction.

### Registry file tampering / malformed persisted config

**Risk:** local modification or interrupted writes corrupt the execution policy.

**Controls:**

- Explicit schema version.
- Full validation on startup.
- Unsupported/malformed registry fails startup instead of falling back or overwriting it.
- Mutation writes a complete temp sibling, syncs it, atomically renames it, then publishes the new in-memory state.
- Deterministic map serialization makes operator review/diffing practical.

**Residual risk:** filesystem permissions remain the primary protection against a local actor who can intentionally rewrite the registry. File authentication/signing is not part of M4.

### Unauthorized registration/config mutation

**Risk:** a malicious webpage reaches localhost Studio and registers or changes executable configuration.

**Controls:**

- Registry/discovery mutations use the same browser same-origin check as lifecycle/tunnel mutations.
- Studio remains loopback-only.
- Foreign browser origins receive `403 Forbidden`.
- Discovery registration resolves the candidate server-side rather than accepting an arbitrary browser path.

**Residual risk:** local non-browser processes may call the API without `Origin` by design. Host account/process isolation is trusted until remote/multi-user authentication is introduced.

### Running-process config mutation / orphaned ownership

**Risk:** configuration changes underneath an active child and causes runtime/registry state to diverge.

**Controls:**

- Edit, disable, and unregister are rejected while state is `starting`, `running`, or `stopping`.
- Operator must explicitly stop first.
- Unregister removes only inactive runtime state.
- Studio continues to signal only the tracked owned PID.

### Duplicate or stale registry entries

**Risk:** multiple IDs refer to the same project or a removed entry remains startable through stale supervisor state.

**Controls:**

- Stable ID uniqueness is enforced.
- Duplicate registered project paths are rejected.
- Supervisor resolves live registry state for lifecycle operations instead of keeping an immutable startup-only registry snapshot.
- Disabled/unregistered state therefore takes effect without supervisor reconstruction.

### Persistence/runtime consistency

**Risk:** persistent mutation succeeds but runtime reconciliation fails, or memory changes before disk durability.

**Controls:**

- The registry object is the shared authority used directly by supervisor lifecycle resolution.
- Candidate state is published in memory only after successful persistence.
- No separate asynchronous supervisor reload step exists.
- Active mutations that would require reconciliation are rejected stop-first.

### TOCTOU between validation and spawn

**Risk:** filesystem contents change after validation but before `exec`.

**Controls:**

- Path/executable checks occur immediately before `Command::spawn`.
- Symlink components are rejected.
- Authorization remains scoped to the registered project root.

**Residual risk:** ordinary files can still be replaced by another local writer in the remaining window. Stronger descriptor/handle-based execution is deferred unless threat review requires it.

### Unregister/delete confusion

**Risk:** an unregister API accidentally deletes source code or an attacker supplies a delete target.

**Controls:**

- Unregister accepts an MCP ID only.
- It removes a registry record and inactive runtime state only.
- No filesystem deletion target exists in the API/domain operation.

### Process ownership / PID misuse

**Risk:** Studio signals an unrelated host process.

**Controls:**

- PID originates only from `Child::id()` for a Studio-spawned child.
- API accepts no PID.
- Generation guards prevent stale monitor tasks from overwriting newer runtime state.
- Stop/restart act only on currently tracked runtime state.

### Cross-origin localhost control

**Risk:** a remote webpage causes privileged requests to localhost Studio.

**Controls:**

- Browser mutations validate `Origin` when present and require matching `Host`.
- WebSocket upgrades use the same policy.
- Wildcard CORS is not enabled.
- Studio rejects non-loopback bind addresses.

### Tunnel secret/exposure threats

Tunnel management remains governed by ADR 0003/M3 controls:

- fixed server-side invocation shape;
- tunnel runtime/config confinement;
- server-side secret references;
- exact-secret/common-field log redaction;
- explicit lifecycle start;
- loopback-only Studio exposure.

M4 does not merge tunnel configuration into the MCP registry.

### Denial of service / log pressure

Current controls:

- duplicate starts are rejected;
- incompatible lifecycle/config mutations are rejected;
- MCP/tunnel recent log buffers are bounded;
- realtime uses bounded broadcast channels and resync on lag;
- one bad discovery candidate does not abort a whole scan.

Deferred controls include rate limiting, restart backoff/circuit breaking, and more explicit scan/manifest resource limits.

### Supply-chain compromise

Controls:

- `Cargo.lock` committed.
- Rust toolchain pinned to 1.98.1.
- release gates require formatting, clippy with warnings denied, tests, builds, dependency audit, and locked release build.
- frontend lockfile is committed and lint/typecheck/test/build are release gates.

## M4 security release gate

Milestone 4 must not close with an unresolved known Critical/High finding in:

- discovery-triggered execution;
- MCP-root/path/symlink confinement;
- executable/argument handling;
- secret exposure;
- registry persistence/tampering behavior;
- same-origin privileged mutations;
- process ownership;
- running-state registry mutation;
- unregister/delete separation;
- existing tunnel security guarantees.

## Review triggers

Update this threat model whenever materially changing process spawn/kill behavior, project/executable roots, symlink policy, discovery parsers, registry schema/persistence, browser-origin/CORS/CSRF policy, secret handling, remote access/authentication, gateway/proxying, plugins/adapters, or multi-host control.