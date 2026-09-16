# MCP Studio Architecture

## Purpose

MCP Studio is a local-first control plane for MCP servers and secure tunnels under `mcp-server/`.

## Milestone 0 boundaries

This milestone establishes structure only. It does **not** manage child processes, tunnels, persistent registries, or historical metrics yet.

## Components

- `api`: HTTP/WebSocket boundary. Milestone 0 exposes only `/health`.
- `config`: typed configuration and validation.
- `supervisor`: future MCP process lifecycle manager.
- `tunnel`: future secure-tunnel lifecycle manager.
- `registry`: future MCP registration model.
- `discovery`: future project scanner.
- `metrics`: future live/historical metrics.
- `storage`: future persistence layer.
- `logging`: structured JSON logging initialization.
- `error`: shared typed error boundary.

## Security boundary

Studio is privileged because future milestones will spawn and stop processes. Therefore:

1. Default bind address is loopback only.
2. Milestone 0 rejects non-loopback addresses.
3. Browser APIs must never accept arbitrary shell command strings.
4. Discovered MCP projects must never auto-execute.
5. Secret values must stay server-side and be redacted from logs/API responses.
6. Studio may terminate only processes that it owns or explicitly adopts through a future audited mechanism.

## Runtime model

```text
Browser
  |
HTTP / WebSocket
  |
MCP Studio
  |-- API
  |-- Config
  |-- Supervisor (M1)
  |-- Tunnel Manager (M3)
  |-- Registry/Discovery (M4)
  |-- Storage/Metrics (M5)
  `-- Gateway (M7)
```

## Configuration

Configuration is TOML in the initial foundation. Built-in defaults are intentionally safe:

```toml
[server]
listen_addr = "127.0.0.1:18100"
```

Configuration must be validated before listeners or privileged components start.

## Error handling

- Library boundaries return typed errors.
- Top-level startup may use `anyhow` for context aggregation.
- Operational failures must be logged with structured context.
- Expected user/config errors must not panic.

## Logging

Structured JSON logs are the baseline. Secrets, authorization material, raw environment values, and full MCP payloads are prohibited by default.

## Evolution rule

Changes that materially affect security, persistence, process ownership, gateway behavior, authentication, or external API contracts require an ADR.
