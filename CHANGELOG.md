# Changelog

All notable changes to MCP Studio will be documented here.

The project follows Semantic Versioning once public/pre-release artifacts begin.

## [0.0.1] - 2026-09-16

### Milestone

- Milestone 0 — Foundation & SDLC Bootstrap — completed and closed.

### Added

- Milestone 0 SDLC and production-readiness roadmap.
- Rust/Axum foundation service.
- Localhost-only validated configuration baseline.
- Structured JSON logging.
- `/health` endpoint.
- Typed error boundary.
- Module boundaries for API, supervisor, tunnel, registry, discovery, storage, and metrics.
- Architecture document.
- Initial threat model.
- ADR process and foundation ADR.
- Contribution/development conventions.
- CI baseline for format, lint, tests, build, and dependency audit.
- Expanded `.gitignore` for Rust, runtime state, logs, frontend artifacts, secrets, local configuration, IDE/OS metadata, and generated artifacts.
- `rust-toolchain.toml` pinned to Rust `1.98.1` with `rustfmt` and `clippy`.
- Explicit MSRV policy of Rust `1.98.1`.

### Changed

- CI Rust toolchain pinned to `1.98.1` instead of the moving `stable` channel.
- CI clippy, test, and build gates use the committed Cargo lockfile.
- Logging initialization now converts subscriber initialization errors explicitly for compatibility with the selected toolchain.
- `StudioConfig` uses derived `Default` where appropriate to satisfy the zero-warning clippy gate.

### Verified

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo build --all-targets --all-features`
- `cargo audit`
- Runtime startup on `127.0.0.1:18100`
- Unit tests: `3 passed; 0 failed`

### Security

- Non-loopback listen addresses are rejected during the current foundation baseline.
- Privileged MCP process and tunnel actions are intentionally deferred to later milestones.
- Arbitrary shell execution is outside the allowed Studio control model.
