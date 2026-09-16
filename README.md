# MCP Studio

Local-first web control plane for monitoring and managing MCP servers and secure tunnels under `mcp-server/`.

## Current status

Milestone 0 foundation only. Privileged process supervision, tunnel control, registry persistence, discovery, metrics history, and the web dashboard are intentionally deferred to later milestones defined in `ROADMAP.md`.

## Requirements

- Rust 1.85+
- Cargo

## Run

```bash
cd mcp-server/studio
cargo run
```

The foundation service listens on:

```text
127.0.0.1:18100
```

Health endpoint:

```text
GET /health
```

## Configuration

Optional TOML file:

```toml
[server]
listen_addr = "127.0.0.1:18100"
```

Run with:

```bash
cargo run -- --config studio.toml
```

or set:

```bash
MCP_STUDIO_CONFIG=studio.toml
```

Milestone 0 rejects non-loopback bind addresses by design.

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

If `cargo-audit` is installed:

```bash
cargo audit
```

## Engineering documents

- `ROADMAP.md` — milestones from foundation to production grade.
- `docs/architecture.md` — component boundaries and design principles.
- `docs/threat-model.md` — security baseline.
- `docs/adr/` — architecture decision records.
- `CONTRIBUTING.md` — SDLC and development workflow.

## Security baseline

- Localhost-only by default.
- No arbitrary shell-command API.
- No auto-execution of discovered projects.
- No secret values in browser responses or logs.
- Privileged features require explicit design and test gates before release.
