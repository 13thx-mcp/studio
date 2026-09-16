# Milestone 0 Status

## Status

**COMPLETE — closed on 2026-09-16.**

Milestone 0 has passed its implementation and verification gates and is now the frozen foundation baseline for Milestone 1.

## Completed

- Rust project bootstrap.
- Axum/Tokio foundation service.
- Safe localhost-only configuration default and validation.
- Structured JSON logging baseline.
- Typed error boundary.
- `/health` endpoint and unit test.
- Module boundaries for API, config, supervisor, tunnel, registry, discovery, storage, and metrics.
- Architecture document.
- Threat model.
- ADR template and initial foundation ADR.
- Contribution/SDLC conventions.
- Changelog and example config.
- Expanded `.gitignore` for Rust, runtime state, logs, frontend artifacts, secrets, IDE/OS metadata, and local config.
- Explicit MSRV policy set to Rust `1.98.1`.
- `rust-toolchain.toml` pinned to Rust `1.98.1` with `rustfmt` and `clippy`.
- CI pinned to Rust `1.98.1` instead of moving `stable`.
- CI quality gates use the committed lockfile for clippy/test/build.

## Verification Results

The following quality gates were rerun successfully on Rust `1.98.1`:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
cargo audit
cargo run
```

Observed test result:

```text
3 passed; 0 failed
```

The Studio runtime started successfully on the loopback address:

```text
127.0.0.1:18100
```

The security baseline remains:

- loopback-only bind for Milestone 0;
- no arbitrary shell execution from the web/API surface;
- no MCP process supervision yet;
- no tunnel lifecycle control yet;
- secrets remain server-side and must not be exposed in logs or API responses.

## Toolchain Baseline

```text
Rust / MSRV: 1.98.1
Edition:     2024
```

The project toolchain is pinned by `rust-toolchain.toml`, and CI uses the same Rust version to avoid drift from the moving stable channel.

## Closure Decision

Milestone 0 exit criteria are satisfied.

Milestone 1 — Core Process Supervisor — may now begin.

Any future change to the Milestone 0 architecture baseline that materially changes process ownership, security boundaries, configuration format, or deployment model should be documented through an ADR before implementation.
