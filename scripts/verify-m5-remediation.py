#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FLEET = ROOT.parent / "fleet"
RESULTS = ROOT / "issues" / "m5-remediation" / "results"

REQUIRED_REGRESSIONS = [
    "update::tunnel_update::tests::audit_tunnel_running_binary_must_match_activated_target",
    "update::tunnel_update::tests::audit_tunnel_fsync_failure_must_restore_current",
    "update::tunnel_update::tests::audit_tunnel_restart_must_recover_unverified_same_version_swap",
    "update::reconciliation::tests::audit_check_must_respect_mutation_guard",
    "update::reconciliation::tests::audit_concurrent_check_must_not_poison_rollback_baseline",
    "update::self_update::tests::audit_self_update_requires_health_before_completed",
    "update::reconciliation::tests::audit_launcher_shadowing_must_fail_closed",
    "update::reconciliation::tests::audit_studio_restart_flag_must_survive_check",
    "update::gateway::tests::audit_gateway_equal_count_wrong_names_must_fail",
    "update::gateway::tests::audit_gateway_valid_catalog_shape_still_passes",
]

ISOLATED_SMOKES = [
    "update::self_update::tests::external_launcher_success_smoke_preserves_local_state_and_switches_release",
    "update::self_update::tests::external_launcher_health_failure_smoke_rolls_back_legacy_release",
    "update::gateway::tests::live_safe_gateway_protocol_probe_smoke",
    "update::reconciliation::tests::m5_12_incident_reconciliation_closure_drill",
    "update::reconciliation::tests::live_aira_runtime_only_reconciliation_smoke",
]

PROFILES = {
    "core": [
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "check", "--locked", "--all-targets", "--all-features"],
        ["cargo", "clippy", "--locked", "--all-targets", "--all-features", "--", "-D", "warnings"],
        ["cargo", "test", "--locked", "--all-targets", "--all-features"],
        ["cargo", "build", "--locked", "--all-targets", "--all-features"],
        ["pnpm", "--dir", "web", "lint"],
        ["pnpm", "--dir", "web", "typecheck"],
        ["pnpm", "--dir", "web", "test"],
        ["pnpm", "--dir", "web", "build"],
    ],
    "isolated-integration": [
        ["cargo", "test", "--locked", name, "--", "--ignored", "--exact", "--nocapture"]
        for name in ISOLATED_SMOKES
    ],
    "security": [
        ["cargo", "audit"],
        ["cargo", "test", "--locked", "--all-targets", "--all-features", "tamper"],
        ["cargo", "test", "--locked", "--all-targets", "--all-features", "symlink"],
        ["cargo", "test", "--locked", "--all-targets", "--all-features", "origin"],
    ],
    "native-package": [
        ["pnpm", "--dir", "web", "build"],
        ["cargo", "build", "--release", "--locked"],
        ["python3", "scripts/verify-native-package.py"],
    ],
}

FLEET_COMMANDS = [
    ["python3", "-m", "py_compile", "scripts/fleetctl.py", "tests/test_fleetctl.py"],
    ["python3", "-m", "unittest", "discover", "-s", "tests", "-v"],
]


def capture(cmd: list[str], cwd: Path) -> dict:
    try:
        proc = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True, check=False)
        return {
            "argv": cmd,
            "exit": proc.returncode,
            "stdout": proc.stdout.strip(),
            "stderr": proc.stderr.strip(),
        }
    except FileNotFoundError as error:
        return {"argv": cmd, "exit": 127, "stdout": "", "stderr": str(error)}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def provenance() -> dict:
    files = [
        ROOT / "Cargo.lock",
        ROOT / "web" / "pnpm-lock.yaml",
        FLEET / "requirements.txt",
    ]

    def repo(cwd: Path) -> dict:
        commands = {
            "head": ["git", "rev-parse", "HEAD"],
            "branch": ["git", "branch", "--show-current"],
            "status": ["git", "status", "--porcelain=v1"],
            "diff": ["git", "diff", "--binary"],
            "staged": ["git", "diff", "--cached", "--binary"],
        }
        return {key: capture(argv, cwd) for key, argv in commands.items()}

    return {
        "studio": repo(ROOT),
        "fleet": repo(FLEET),
        "locks": {
            str(path.relative_to(ROOT.parent)): sha256(path)
            for path in files
            if path.is_file()
        },
        "toolchains": {
            "rustc": capture(["rustc", "--version"], ROOT),
            "cargo": capture(["cargo", "--version"], ROOT),
            "python": capture(["python3", "--version"], ROOT),
            "node": capture(["node", "--version"], ROOT),
            "pnpm": capture(["pnpm", "--version"], ROOT),
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
    }


def process_group_exists(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def terminate_process_group(pgid: int) -> bool:
    if not process_group_exists(pgid):
        return False
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        return False
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline:
        if not process_group_exists(pgid):
            return True
        time.sleep(0.05)
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    return True


def run(
    cmd: list[str],
    cwd: Path,
    log_path: Path,
    env: dict[str, str],
    timeout_seconds: int,
) -> tuple[int, float, bool, bool]:
    start = time.monotonic()
    timed_out = False
    cleanup_performed = False
    with log_path.open("wb") as log:
        proc = subprocess.Popen(
            cmd,
            cwd=cwd,
            stdout=log,
            stderr=subprocess.STDOUT,
            env=env,
            start_new_session=True,
        )
        try:
            rc = proc.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            cleanup_performed = terminate_process_group(proc.pid)
            rc = 124
            log.write(
                f"\nM5 verifier: command timed out after {timeout_seconds}s\n".encode()
            )
        except KeyboardInterrupt:
            terminate_process_group(proc.pid)
            raise

    # Clean up only the process group created for this command. This catches
    # ordinary fixture children without broad process-name matching.
    if process_group_exists(proc.pid):
        cleanup_performed = terminate_process_group(proc.pid) or cleanup_performed
        with log_path.open("ab") as log:
            log.write(b"\nM5 verifier: cleaned leftover command process group\n")

    return rc, time.monotonic() - start, timed_out, cleanup_performed


def validate_test_execution(cmd: list[str], log_path: Path, rc: int) -> tuple[int, str | None]:
    if rc != 0 or len(cmd) < 2 or cmd[0] != "cargo" or cmd[1] != "test":
        return rc, None

    text = log_path.read_text(errors="replace")
    if "--exact" in cmd and "--" in cmd:
        split = cmd.index("--")
        if split > 2:
            expected = cmd[split - 1]
            marker = f"test {expected} ... ok"
            if marker not in text:
                return 125, f"required exact Rust test did not execute successfully: {expected}"

    term = cmd[-1] if cmd[-1] in {"tamper", "symlink", "origin"} else None
    if term is not None:
        pattern = re.compile(rf"^test .*{re.escape(term)}.* \.\.\. ok$", re.MULTILINE)
        if not pattern.search(text):
            return 125, f"security filter matched no successful Rust test: {term}"

    return rc, None


def archive_result(out: Path) -> Path:
    archive = out.with_suffix(".tar.gz")
    with tarfile.open(archive, "w:gz") as tf:
        for child in sorted(out.iterdir(), key=lambda path: path.name):
            if child.name == "cargo-target":
                continue
            tf.add(child, arcname=f"{out.name}/{child.name}")
    return archive


def source_changed(before: dict, after: dict) -> bool:
    for repo in ("studio", "fleet"):
        for key in ("head", "status", "diff", "staged"):
            if before[repo][key]["stdout"] != after[repo][key]["stdout"]:
                return True
    return False


def write_setup_failure(out: Path, before: dict, detail: str) -> Path:
    summary = {
        "setup_failure": detail,
        "before": before,
        "passed": False,
        "commands": [],
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (out / "summary.tsv").write_text("index\texit\tduration_s\tcwd\targv\tlog\n")
    (out / "run.log").write_text(json.dumps({"setup_failure": detail}) + "\n")
    return archive_result(out)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--profile",
        choices=["regressions", *PROFILES],
        default="regressions",
    )
    parser.add_argument("--include-fleet", action="store_true")
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--command-timeout-seconds", type=int, default=1800)
    args = parser.parse_args()

    stamp = (
        datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        + "-"
        + str(os.getpid())
    )
    out = RESULTS / stamp
    out.mkdir(parents=True)

    before = provenance()
    if args.require_clean and (
        before["studio"]["status"]["stdout"] or before["fleet"]["status"]["stdout"]
    ):
        archive = write_setup_failure(out, before, "dirty worktree")
        print(f"RESULT_DIR={out}")
        print(f"ARCHIVE={archive}")
        print("FAIL")
        return 2

    if args.profile == "regressions":
        commands = [
            ["cargo", "test", "--locked", name, "--", "--exact", "--nocapture"]
            for name in REQUIRED_REGRESSIONS
        ]
    else:
        commands = list(PROFILES[args.profile])

    work: list[tuple[Path, list[str]]] = [(ROOT, command) for command in commands]
    if args.include_fleet:
        work += [(FLEET, command) for command in FLEET_COMMANDS]

    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(out / "cargo-target")
    env["CARGO_TERM_COLOR"] = "never"
    env["M5_RESULT_DIR"] = str(out)

    rows: list[dict] = []
    try:
        for index, (cwd, cmd) in enumerate(work, 1):
            prefix = "-".join(Path(item).name for item in cmd[:3])
            log_path = out / f"{index:02d}-{prefix}.log"
            if shutil.which(cmd[0]) is None:
                rc, duration, timed_out, cleanup = 127, 0.0, False, False
                log_path.write_text(f"missing mandatory tool: {cmd[0]}\n")
            else:
                rc, duration, timed_out, cleanup = run(
                    cmd,
                    cwd,
                    log_path,
                    env,
                    args.command_timeout_seconds,
                )
                validated_rc, validation_error = validate_test_execution(
                    cmd, log_path, rc
                )
                if validation_error:
                    with log_path.open("a") as log:
                        log.write(f"\nM5 verifier validation failure: {validation_error}\n")
                rc = validated_rc

            rows.append(
                {
                    "index": index,
                    "cwd": str(cwd),
                    "argv": cmd,
                    "exit": rc,
                    "duration_s": round(duration, 3),
                    "timed_out": timed_out,
                    "cleanup_performed": cleanup,
                    "log": log_path.name,
                }
            )
            (out / "summary.partial.json").write_text(
                json.dumps({"before": before, "commands": rows}, indent=2) + "\n"
            )
    finally:
        after = provenance()
        changed = source_changed(before, after)
        summary = {
            "run_id": stamp,
            "profile": args.profile,
            "before": before,
            "after": after,
            "source_changed_during_run": changed,
            "commands": rows,
            "passed": len(rows) == len(work)
            and all(row["exit"] == 0 for row in rows)
            and not changed,
        }
        (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        with (out / "summary.tsv").open("w") as handle:
            handle.write(
                "index\texit\tduration_s\ttimed_out\tcleanup\tcwd\targv\tlog\n"
            )
            for row in rows:
                handle.write(
                    f'{row["index"]}\t{row["exit"]}\t{row["duration_s"]}\t'
                    f'{row["timed_out"]}\t{row["cleanup_performed"]}\t'
                    f'{row["cwd"]}\t{json.dumps(row["argv"])}\t{row["log"]}\n'
                )
        (out / "run.log").write_text(
            "\n".join(json.dumps(row) for row in rows) + "\n"
        )
        shutil.rmtree(out / "cargo-target", ignore_errors=True)
        archive = archive_result(out)

    print(f"RESULT_DIR={out}")
    print(f"ARCHIVE={archive}")
    print("PASS" if summary["passed"] else "FAIL")
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
