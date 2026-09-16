SHELL := /bin/bash

PNPM ?= pnpm
CARGO ?= cargo
CONFIG ?= studio.example.toml
HOST ?= 127.0.0.1
STUDIO_PORT ?= 18100
WEB_PORT ?= 5173

.PHONY: help install web-install web-dev web-build build run dev check fmt clippy test audit release clean

help:
	@echo "MCP Studio development commands"
	@echo
	@echo "  make install    Install frontend dependencies"
	@echo "  make dev        Run Rust backend + Vite dev server"
	@echo "  make run        Build frontend and run Studio on $(HOST):$(STUDIO_PORT)"
	@echo "  make build      Build frontend and Rust backend"
	@echo "  make check      Run Rust + frontend quality gates"
	@echo "  make test       Run Rust + frontend tests"
	@echo "  make audit      Run cargo audit"
	@echo "  make release    Build locked Rust release + frontend production assets"
	@echo "  make clean      Remove Rust and frontend generated build output"
	@echo
	@echo "Variables:"
	@echo "  CONFIG=<path>       Studio config file (default: $(CONFIG))"
	@echo "  STUDIO_PORT=<port>  Studio port reference (default: $(STUDIO_PORT))"
	@echo "  WEB_PORT=<port>     Vite dev port (default: $(WEB_PORT))"

install: web-install

web-install:
	$(PNPM) --dir web install --frozen-lockfile

web-dev:
	$(PNPM) --dir web dev --host $(HOST) --port $(WEB_PORT)

web-build:
	$(PNPM) --dir web build

build: web-build
	$(CARGO) build --all-targets --all-features

# Production-style local run: build web/dist, then let Axum serve the SPA and API.
run: web-build
	$(CARGO) run -- --config $(CONFIG)

# Development mode: Rust API on :18100 plus Vite on :5173.
# Compatible with the older /bin/bash shipped by macOS.
dev: web-install
	@set -e; \
	$(CARGO) run -- --config $(CONFIG) & \
	backend_pid=$$!; \
	$(PNPM) --dir web dev --host $(HOST) --port $(WEB_PORT) & \
	web_pid=$$!; \
	cleanup() { \
		kill $$backend_pid $$web_pid 2>/dev/null || true; \
		wait $$backend_pid $$web_pid 2>/dev/null || true; \
	}; \
	trap 'cleanup; exit 130' INT TERM; \
	trap cleanup EXIT; \
	while kill -0 $$backend_pid 2>/dev/null && kill -0 $$web_pid 2>/dev/null; do \
		sleep 1; \
	done; \
	cleanup

fmt:
	$(CARGO) fmt --all -- --check

clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

check: fmt clippy
	$(CARGO) test --all-targets --all-features
	$(CARGO) build --all-targets --all-features
	$(PNPM) --dir web lint
	$(PNPM) --dir web typecheck
	$(PNPM) --dir web test
	$(PNPM) --dir web build

test:
	$(CARGO) test --all-targets --all-features
	$(PNPM) --dir web test

audit:
	$(CARGO) audit

release: web-build
	$(CARGO) build --release --locked

clean:
	$(CARGO) clean
	rm -rf web/dist web/.vite web/coverage web/*.tsbuildinfo
