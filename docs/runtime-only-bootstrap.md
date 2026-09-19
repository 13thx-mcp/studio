# Runtime-Only Bootstrap Guide

## Purpose

This guide describes the M5 runtime-host layout and the release-only bootstrap sequence. A runtime-only host must not require project source, Rust/Cargo, Node/pnpm, `target/`, or `node_modules` after deployment.

> Current release note (2026-09-19): the M5 implementation is ready, but the expected public `13thx-mcp/*` project release endpoints currently return HTTP 404. Do not treat copied development-host binaries as a substitute for release-grade bootstrap proof.

## Required runtime layout

```text
mcp-server/
├── bin/
│   ├── rust-mcp-filesystem
│   ├── rust-mcp-git
│   ├── rust-mcp-exec
│   ├── rust-mcp-gateway
│   └── rust-mcp-blender          # when managed
└── runtime/
    ├── gateway/servers.d/
    ├── studio/
    │   ├── current -> releases/vX.Y.Z
    │   ├── releases/
    │   ├── studio.toml
    │   └── data/
    ├── tunnel-client/
    │   ├── current -> releases/vX.Y.Z
    │   ├── releases/
    │   └── config.yaml
    └── fleet/
        ├── fleet.toml
        ├── scripts/fleetctl.py
        ├── hosts/<host>.toml
        └── state/
```

`studio.toml`, Studio `data/`, Tunnel `config.yaml`/credentials, and Fleet host/state files are host-local. Release archives do not own them.

## Artifact authorities

- Filesystem/Git/Exec/Gateway/Blender/Studio/Fleet: server-owned `13thx-mcp/*` GitHub Release mapping.
- Tunnel: official `openai/tunnel-client` releases only.
- Never bootstrap from a browser-provided release URL.
- Every artifact must have the exact expected name for the selected OS/architecture and a verified `SHA256SUMS.txt` entry.

## Bootstrap sequence

1. Detect/normalize platform (`Darwin`, `amd64` or `arm64`).
2. Create the flat `bin/` and `runtime/` roots.
3. Fetch project-owned stable releases from their server-policy repositories.
4. Verify checksum manifest and archive digest before extraction.
5. Reject traversal, absolute paths, links, special files, missing executables, and unexpected package layout.
6. Install MCP binaries into the flat `bin/` root.
7. Install Fleet control bundle below `runtime/fleet/` and create the active `hosts/<host>.toml` profile.
8. Fetch and verify the official Tunnel release; install under `runtime/tunnel-client/releases/vX.Y.Z` and atomically create `current`.
9. Generate Gateway/Studio/Tunnel config through Fleet's pure render contract; validate before activation.
10. Install Studio backend + matching `web/dist` as one versioned release below `runtime/studio/releases/vX.Y.Z`, preserving `studio.toml` and `data/`.
11. Point `runtime/studio/current` at the release and start Studio through the documented launcher/service contract.
12. Start Tunnel/Gateway/MCP children according to the host profile.
13. Verify Studio inventory is `runtime_only` and installed identities come from deployed artifacts.
14. Verify Gateway child identity and tool catalog fingerprint rather than only child/tool counts.

## Verification checklist

From Studio/API verify:

- host mode is `runtime_only`;
- all required installed versions are readable;
- desired/running/installed/latest dimensions are explicit;
- Tunnel points only into `runtime/tunnel-client/releases`;
- Gateway child commands point only at the flat `bin_root`;
- generated config is synchronized with the active Fleet host profile;
- no source-tree/`target/release` path remains in active generated config;
- source Git HEAD is not used as installed runtime truth.

## Runtime operation

After bootstrap, normal operation requires only the installed runtime artifacts and host-local configuration. Update transactions consume verified release artifacts; they do not build source locally.

## Current blocker

Release-grade bootstrap must remain blocked while required project-owned release endpoints are unavailable. The correct response is to publish/restore those releases, not to relax provider trust or add arbitrary fallback URLs.
