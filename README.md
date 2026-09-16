# MCP Studio

Local-first web control plane for monitoring and managing MCP servers and secure tunnels under `mcp-server/`.

## Current status

Milestone 1 core process supervisor is implemented and awaiting runtime verification. Studio can supervise the statically configured Blender and Filesystem MCP servers, expose lifecycle APIs, capture recent stdout/stderr logs, and stop owned child processes during Studio shutdown.

Tunnel control, persistent registry/discovery, historical metrics, and the web dashboard remain deferred to later milestones in `ROADMAP.md`.

## Requirements

- Rust 1.98.1
- Cargo
- Unix platform (macOS/Linux) for Milestone 1 signal-based graceful process control

The pinned toolchain is declared in `rust-toolchain.toml`.

## Build the managed MCP servers first

Milestone 1 defaults expect debug binaries at:

```text
../blender/target/debug/rust-mcp-blender
../filesystem/target/debug/rust-mcp-filesystem
```

Build them before starting those MCPs through Studio:

```bash
cd mcp-server/blender
cargo build

cd ../filesystem
cargo build
```

## Run Studio

```bash
cd mcp-server/studio
cargo run
```

Default listen address:

```text
127.0.0.1:18100
```

## API

```text
GET  /health
GET  /api/status
GET  /api/mcp
GET  /api/mcp/{id}
POST /api/mcp/{id}/start
POST /api/mcp/{id}/stop
POST /api/mcp/{id}/restart
GET  /api/mcp/{id}/logs
```

Examples:

```bash
curl http://127.0.0.1:18100/api/mcp
curl -X POST http://127.0.0.1:18100/api/mcp/blender/start
curl http://127.0.0.1:18100/api/mcp/blender/logs
curl -X POST http://127.0.0.1:18100/api/mcp/blender/stop
```

## Configuration

See `studio.example.toml`.

Run with a config file:

```bash
cargo run -- --config studio.example.toml
```

or:

```bash
MCP_STUDIO_CONFIG=studio.example.toml cargo run
```

Relative `command` and `working_dir` paths are resolved against Studio's process working directory.

Milestone 1 continues to reject non-loopback bind addresses by design.

## Process lifecycle

```text
STOPPED
   │ start
   ▼
STARTING
   │ spawn
   ▼
RUNNING ───────────────┐
   │ stop              │ unexpected non-zero exit / wait error
   ▼                   ▼
STOPPING             FAILED
   │                   │
   └──── exit ─────► STOPPED
```

Stop behavior on Unix:

1. Send `SIGTERM` to the owned child PID.
2. Wait up to `stop_timeout_ms`.
3. Escalate to `SIGKILL` when the process does not exit.

Studio only signals PIDs that came from processes it spawned and currently tracks.

## Logging

Recent stdout/stderr lines are stored in an in-memory ring buffer per MCP server. The default capacity is 500 entries and can be changed with `log_capacity`.

Historical/persistent logs are intentionally out of scope for Milestone 1.

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --all-targets --all-features
cargo audit
```

## Engineering documents

- `ROADMAP.md` — milestones from foundation to production grade.
- `docs/architecture.md` — component boundaries and design principles.
- `docs/threat-model.md` — security baseline.
- `docs/adr/` — architecture decision records.
- `CONTRIBUTING.md` — SDLC and development workflow.

## Security baseline

- Localhost-only HTTP control plane.
- No arbitrary shell-command API.
- Static MCP registry for Milestone 1.
- Studio signals only tracked child PIDs.
- No auto-execution of discovered projects.
- No secret values returned through the lifecycle API.
