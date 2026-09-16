# ADR 0001: Foundation Architecture

- Status: Accepted
- Date: 2026-09-16

## Context

MCP Studio must manage local MCP processes and a secure tunnel while providing a web control plane. Future milestones will introduce privileged process execution, persistence, discovery, and gateway capabilities.

## Decision

- Use Rust for the backend/control plane.
- Use Tokio for asynchronous runtime.
- Use Axum for HTTP/WebSocket APIs.
- Use structured JSON logging via `tracing`.
- Use typed configuration with validation before startup.
- Bind to loopback by default and reject non-loopback binds during Milestone 0.
- Keep privileged subsystems behind explicit module boundaries.
- Do not auto-execute discovered MCP projects.
- Do not expose arbitrary shell execution through the API.
- Introduce SQLite only when persistence is required in Milestone 5.
- Introduce React/TypeScript/Vite when the dashboard begins in Milestone 2.

## Consequences

Positive:
- Strong type safety and process-control fit.
- Shared language/ecosystem with existing Rust MCP servers.
- Security boundaries are explicit before privileged features arrive.

Trade-offs:
- Frontend and backend will use separate toolchains after Milestone 2.
- Remote access requires a future security design instead of simply changing the bind address.
