#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

STUDIO = Path(__file__).resolve().parents[1]
MCP_ROOT = STUDIO.parent
RESULTS = STUDIO / ".tmp" / "m7-final-closure"
REQUIRED_RUST = "1.98.1"

COMPONENTS = {
    "studio": STUDIO,
    "gateway": MCP_ROOT / "gateway",
    "filesystem": MCP_ROOT / "filesystem",
    "exec": MCP_ROOT / "exec",
    "git": MCP_ROOT / "git",
    "fleet": MCP_ROOT / "fleet",
}

RUST_COMPONENTS = ("studio", "gateway", "filesystem", "exec", "git")


@dataclass(frozen=True)
class CommandSpec:
    gate: str
    name: str
    argv: tuple[str, ...]
    cwd: Path
    timeout: int = 600
    require_tests: bool = False


def qualification_env() -> dict[str, str]:
    env = os.environ.copy()
    cargo_bin = Path.home() / ".cargo" / "bin"
    path_parts = [str(cargo_bin)] if cargo_bin.is_dir() else []
    path_parts.extend(
        part
        for part in env.get(
            "PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
        ).split(":")
        if part
    )
    env["PATH"] = ":".join(dict.fromkeys(path_parts))
    env["CARGO_TERM_COLOR"] = "never"
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    return env


def capture(argv: list[str], cwd: Path, env: dict[str, str]) -> dict:
    try:
        proc = subprocess.run(
            argv,
            cwd=cwd,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )
        return {
            "argv": argv,
            "exit": proc.returncode,
            "stdout": proc.stdout.strip(),
            "stderr": proc.stderr.strip(),
        }
    except FileNotFoundError as error:
        return {
            "argv": argv,
            "exit": 127,
            "stdout": "",
            "stderr": str(error),
        }


def git_state(path: Path, env: dict[str, str]) -> dict:
    head = capture(["git", "rev-parse", "HEAD"], path, env)
    branch = capture(["git", "branch", "--show-current"], path, env)
    status = capture(["git", "status", "--porcelain=v1"], path, env)
    upstream = capture(
        ["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        path,
        env,
    )
    ahead_behind = None
    if upstream["exit"] == 0:
        ahead_behind = capture(
            ["git", "rev-list", "--left-right", "--count", f"{upstream['stdout']}...HEAD"],
            path,
            env,
        )
    return {
        "head": head["stdout"],
        "branch": branch["stdout"],
        "clean": status["exit"] == 0 and not status["stdout"],
        "status": status["stdout"],
        "upstream": upstream["stdout"] if upstream["exit"] == 0 else None,
        "ahead_behind": ahead_behind["stdout"] if ahead_behind else None,
    }


def component_version(name: str) -> str:
    root = COMPONENTS[name]
    if name == "fleet":
        return (root / "VERSION").read_text().strip()
    data = tomllib.loads((root / "Cargo.toml").read_text())
    return str(data["package"]["version"])


def provenance(env: dict[str, str]) -> dict:
    return {
        name: {
            **git_state(path, env),
            "version": component_version(name),
        }
        for name, path in COMPONENTS.items()
    }


def source_changed(before: dict, after: dict) -> list[str]:
    changed = []
    for name in COMPONENTS:
        for key in ("head", "status"):
            if before[name][key] != after[name][key]:
                changed.append(name)
                break
    return changed


def parse_test_count(output: str) -> int:
    total = 0
    for line in output.splitlines():
        marker = " passed;"
        if "test result:" not in line or marker not in line:
            continue
        prefix = line.split(marker, 1)[0]
        token = prefix.rsplit(" ", 1)[-1]
        if token.isdigit():
            total += int(token)
    return total


def run_command(
    spec: CommandSpec,
    index: int,
    out: Path,
    env: dict[str, str],
) -> dict:
    started = time.monotonic()
    proc = subprocess.run(
        list(spec.argv),
        cwd=spec.cwd,
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=spec.timeout,
    )
    duration = round(time.monotonic() - started, 3)
    combined = proc.stdout + ("\n" if proc.stdout and proc.stderr else "") + proc.stderr
    tests = parse_test_count(combined)
    status = "PASS" if proc.returncode == 0 else "FAIL"
    if spec.require_tests and tests < 1:
        status = "FAIL"
    log_name = f"{index:03d}-{spec.gate}-{spec.name}.log".replace("/", "-")
    (out / log_name).write_text(combined)
    return {
        "gate": spec.gate,
        "name": spec.name,
        "argv": list(spec.argv),
        "cwd": str(spec.cwd),
        "exit": proc.returncode,
        "duration_s": duration,
        "tests_passed": tests,
        "status": status,
        "log": log_name,
    }


def toolchain_evidence(env: dict[str, str]) -> tuple[dict, list[str]]:
    checks = {
        "cargo": (["cargo", "--version"], STUDIO),
        "rustc": (["rustc", "--version"], STUDIO),
        "rustfmt": (["rustfmt", "--version"], STUDIO),
        "clippy": (["cargo", "clippy", "--version"], STUDIO),
        "cargo_audit": (["cargo", "audit", "--version"], STUDIO),
        "python3": (["python3", "--version"], STUDIO),
        "node": (["node", "--version"], STUDIO),
        "pnpm": (["pnpm", "--version"], STUDIO),
        "git": (["git", "--version"], STUDIO),
    }
    evidence = {}
    failures = []
    for name, (argv, cwd) in checks.items():
        item = capture(argv, cwd, env)
        evidence[name] = item
        if item["exit"] != 0:
            failures.append(f"{name} unavailable: {item['stderr'] or item['stdout']}")
    for name in ("cargo", "rustc"):
        text = evidence[name]["stdout"]
        if evidence[name]["exit"] == 0 and REQUIRED_RUST not in text:
            failures.append(
                f"{name} must be Rust {REQUIRED_RUST}; observed {text or 'unknown'}"
            )
    return evidence, failures


def validate_fleet_plan(host: str, env: dict[str, str]) -> dict:
    fleet = COMPONENTS["fleet"]
    proc = subprocess.run(
        ["python3", "scripts/fleetctl.py", "render-plan", "--host", host, "--json"],
        cwd=fleet,
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=60,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"render-plan failed ({proc.returncode}): {proc.stderr.strip()}"
        )
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"render-plan returned invalid JSON: {error}") from error
    if data.get("schema_version") != 1:
        raise RuntimeError("render-plan schema_version must be 1")
    if data.get("host_id") != host:
        raise RuntimeError("render-plan host_id mismatch")
    outputs = data.get("outputs")
    if not isinstance(outputs, list) or not outputs:
        raise RuntimeError("render-plan outputs must be a non-empty list")

    surfaces: set[str] = set()
    paths: set[str] = set()
    for item in outputs:
        relative = item.get("relative_path")
        surface = item.get("surface")
        encoded = item.get("content_b64")
        digest = item.get("sha256")
        if not all(isinstance(value, str) and value for value in (relative, surface, encoded, digest)):
            raise RuntimeError("render-plan output is missing required string fields")
        candidate = Path(relative)
        if candidate.is_absolute() or ".." in candidate.parts:
            raise RuntimeError(f"render-plan path escapes runtime root: {relative}")
        if relative in paths or surface in surfaces:
            raise RuntimeError("render-plan output paths/surfaces must be unique")
        paths.add(relative)
        surfaces.add(surface)
        try:
            content = base64.b64decode(encoded, validate=True)
        except Exception as error:
            raise RuntimeError(f"invalid base64 for {surface}: {error}") from error
        actual = hashlib.sha256(content).hexdigest()
        if actual != digest:
            raise RuntimeError(f"render-plan sha256 mismatch for {surface}")

    return {
        "host_id": host,
        "schema_version": data["schema_version"],
        "runtime_root": data.get("runtime_root"),
        "output_count": len(outputs),
        "surfaces": sorted(surfaces),
        "paths": sorted(paths),
    }


def component_specs() -> list[CommandSpec]:
    specs: list[CommandSpec] = []
    for name in RUST_COMPONENTS:
        root = COMPONENTS[name]
        specs.extend(
            [
                CommandSpec(name, "fmt", ("cargo", "fmt", "--all", "--", "--check"), root),
                CommandSpec(
                    name,
                    "check",
                    ("cargo", "check", "--all-targets", "--all-features"),
                    root,
                ),
                CommandSpec(
                    name,
                    "clippy",
                    (
                        "cargo",
                        "clippy",
                        "--all-targets",
                        "--all-features",
                        "--",
                        "-D",
                        "warnings",
                    ),
                    root,
                ),
                CommandSpec(
                    name,
                    "test",
                    ("cargo", "test", "--all-targets", "--all-features"),
                    root,
                    require_tests=True,
                ),
                CommandSpec(
                    name,
                    "build",
                    ("cargo", "build", "--all-targets", "--all-features"),
                    root,
                ),
                CommandSpec(name, "audit", ("cargo", "audit"), root),
            ]
        )
    specs.extend(
        [
            CommandSpec("studio-web", "lint", ("pnpm", "--dir", "web", "lint"), STUDIO),
            CommandSpec(
                "studio-web", "typecheck", ("pnpm", "--dir", "web", "typecheck"), STUDIO
            ),
            CommandSpec(
                "studio-web",
                "test",
                ("pnpm", "--dir", "web", "test", "--", "--run"),
                STUDIO,
            ),
            CommandSpec("studio-web", "build", ("pnpm", "--dir", "web", "build"), STUDIO),
            CommandSpec(
                "fleet",
                "tests",
                ("python3", "-m", "unittest", "discover", "-s", "tests", "-p", "test_*.py", "-v"),
                COMPONENTS["fleet"],
            ),
        ]
    )
    return specs


def targeted_specs() -> list[CommandSpec]:
    return [
        CommandSpec(
            "studio-targeted",
            "fleet-pure-render-plan",
            ("cargo", "test", "fleet_validation_uses_pure_render_plan"),
            STUDIO,
            require_tests=True,
        ),
        CommandSpec(
            "studio-targeted",
            "tunnel-runtime-loss-stop",
            (
                "cargo",
                "test",
                "--test",
                "tunnel_lifecycle",
                "stop_remains_available_when_runtime_disappears",
                "--",
                "--exact",
            ),
            STUDIO,
            require_tests=True,
        ),
        CommandSpec(
            "gateway-targeted",
            "bounded-scheduler-soak",
            ("cargo", "test", "bounded_scheduler_soak_releases_all_capacity_without_leaks"),
            COMPONENTS["gateway"],
            require_tests=True,
        ),
        CommandSpec(
            "filesystem-targeted",
            "revision-cas-external-change",
            ("cargo", "test", "metadata_range_and_revision_cas_detect_external_change"),
            COMPONENTS["filesystem"],
            require_tests=True,
        ),
        CommandSpec(
            "filesystem-targeted",
            "revision-cas-concurrency",
            ("cargo", "test", "cas_rechecks_before_rename_and_serializes_same_revision_writers"),
            COMPONENTS["filesystem"],
            require_tests=True,
        ),
        CommandSpec(
            "exec-targeted",
            "mcp-cancellation",
            ("cargo", "test", "cancellation_terminates_running_process_promptly"),
            COMPONENTS["exec"],
            require_tests=True,
        ),
    ]


def full_only_specs() -> list[CommandSpec]:
    return [
        CommandSpec(
            "studio-runtime-only",
            "live-runtime-only-reconciliation",
            (
                "cargo",
                "test",
                "live_aira_runtime_only_reconciliation_smoke",
                "--",
                "--ignored",
            ),
            STUDIO,
            timeout=900,
            require_tests=True,
        ),
        CommandSpec(
            "m6-regression",
            "m5-m6-regression-profile",
            ("python3", "scripts/verify-m6-history.py", "--profile", "regressions"),
            STUDIO,
            timeout=1200,
        ),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(
        description="M7 final reconciliation and pre-M8 qualification runner"
    )
    parser.add_argument(
        "--profile",
        choices=("preflight", "fleet-contract", "components", "targeted", "full"),
        default="preflight",
    )
    parser.add_argument("--host", default="aira")
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    env = qualification_env()
    started_at = datetime.now(timezone.utc)
    stamp = started_at.strftime("%Y%m%d-%H%M%S")
    out = args.output.resolve() if args.output else RESULTS / stamp
    if out.exists():
        print(f"OVERALL: BLOCKED\nEVIDENCE_DIR: {out}\nBLOCKED: evidence directory exists")
        return 2
    out.mkdir(parents=True, exist_ok=False)

    before = provenance(env)
    toolchains, preflight_failures = toolchain_evidence(env)
    blocked_reasons: list[str] = []
    if args.require_clean:
        dirty = [name for name, state in before.items() if not state["clean"]]
        if dirty:
            blocked_reasons.append(
                "--require-clean requested but dirty repositories exist: " + ", ".join(dirty)
            )

    fleet_plan = None
    try:
        fleet_plan = validate_fleet_plan(args.host, env)
    except Exception as error:
        preflight_failures.append(f"Fleet render-plan contract failed: {error}")

    commands: list[dict] = []
    specs: list[CommandSpec] = []
    if args.profile in ("components", "full"):
        specs.extend(component_specs())
    if args.profile in ("targeted", "full"):
        specs.extend(targeted_specs())
    if args.profile == "full":
        specs.extend(full_only_specs())

    stop = bool(preflight_failures or blocked_reasons)
    for index, spec in enumerate(specs, start=1):
        if stop:
            commands.append(
                {
                    "gate": spec.gate,
                    "name": spec.name,
                    "argv": list(spec.argv),
                    "status": "NOT RUN",
                }
            )
            continue
        try:
            result = run_command(spec, index, out, env)
        except subprocess.TimeoutExpired as error:
            result = {
                "gate": spec.gate,
                "name": spec.name,
                "argv": list(spec.argv),
                "cwd": str(spec.cwd),
                "exit": None,
                "duration_s": spec.timeout,
                "tests_passed": 0,
                "status": "FAIL",
                "error": f"timed out after {error.timeout}s",
            }
        commands.append(result)
        if result["status"] != "PASS":
            stop = True

    after = provenance(env)
    changed = source_changed(before, after)
    if changed:
        blocked_reasons.append(
            "source changed while qualification was running: " + ", ".join(changed)
        )

    failures = [item for item in commands if item.get("status") == "FAIL"]
    if failures or preflight_failures:
        overall = "FAIL"
    elif blocked_reasons:
        overall = "BLOCKED"
    else:
        overall = "PASS"

    finished_at = datetime.now(timezone.utc)
    summary = {
        "schema_version": 1,
        "profile": args.profile,
        "host": args.host,
        "status": overall,
        "started_at_utc": started_at.isoformat(),
        "finished_at_utc": finished_at.isoformat(),
        "duration_s": round((finished_at - started_at).total_seconds(), 3),
        "required_rust": REQUIRED_RUST,
        "require_clean": args.require_clean,
        "preflight_failures": preflight_failures,
        "blocked_reasons": blocked_reasons,
        "toolchains": toolchains,
        "fleet_plan": fleet_plan,
        "before": before,
        "after": after,
        "source_changed_during_run": changed,
        "commands": commands,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    print(f"OVERALL: {overall}")
    print(f"EVIDENCE_DIR: {out}")
    if preflight_failures:
        for reason in preflight_failures:
            print(f"FAIL: {reason}")
    if blocked_reasons:
        for reason in blocked_reasons:
            print(f"BLOCKED: {reason}")
    return 0 if overall == "PASS" else 1 if overall == "FAIL" else 2


if __name__ == "__main__":
    raise SystemExit(main())
