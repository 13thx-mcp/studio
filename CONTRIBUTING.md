# Contributing to MCP Studio

## SDLC

Every change follows: plan → requirements → design → implementation → verification → security review → release → operations feedback.

## Branch/change discipline

- Keep changes scoped to one concern.
- Add or update tests with behavior changes.
- Update docs when behavior, API, config, security boundaries, or operations change.
- Add an ADR for decisions affecting process ownership, persistence, remote access/authentication, gateway behavior, external API compatibility, or security boundaries.

## Required checks

Before merge:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Dependency audit is required before release.

## Error handling

- Prefer typed errors in library/module boundaries.
- Add context at orchestration boundaries.
- Do not panic for expected runtime/configuration failures.
- Never silently swallow privileged-operation failures.

## Logging

Use structured `tracing` events. Include useful identifiers, state transitions, and errors. Never log credentials, authorization headers, tunnel tokens, secret environment values, or full MCP payloads by default.

## Configuration

- Validate before activation.
- Defaults must be safe.
- Configuration schemas require versioning once persistent MCP configuration is introduced.
- Secrets should be referenced rather than embedded where possible.

## Security review triggers

Security review is mandatory for changes involving process spawning/termination, paths, discovery, tunnel exposure, credentials, browser authentication, persistence of sensitive data, gateway routing, or remote access.

## Testing expectations

- Unit tests for pure logic and validation.
- Integration tests for OS/process/database boundaries.
- API tests for contract/error behavior.
- Failure injection for lifecycle/recovery logic.
- Security regression tests for discovered vulnerabilities.

## Definition of Done

A change is done only when acceptance criteria pass, tests and docs are current, required verification passes, security implications are addressed, and rollback/migration impact is understood.
