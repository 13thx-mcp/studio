#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FLEET = ROOT.parent / "fleet"
RESULTS = ROOT / ".tmp" / "m6-history"

M5_REGRESSIONS = [
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

M6_EXACT = {
    "foundation": [
        "storage::tests::linked_sqlite_version_and_source_id_are_qualified",
        "storage::tests::unsafe_history_paths_fail_closed",
        "storage::tests::migration_checksum_and_atomic_upgrade",
        "storage::tests::admission_failure_prevents_discretionary_effect",
        "storage::tests::writer_serializes_concurrent_producers",
        "storage::tests::history_never_rehydrates_registry_authority",
        "storage::tests::m5_bootstrap_creates_observations_not_fake_events",
        "storage::tests::artifact_stage_and_verified_lineage_are_content_addressed",
        "storage::tests::sqlite_full_degrades_history_and_blocks_new_admission",
        "storage::tests::corrupted_main_db_fails_closed_without_auto_repair",
        "storage::tests::malformed_wal_fails_closed_without_auto_repair",
        "storage::tests::second_writer_is_rejected_while_owner_lock_is_held",
    ],
    "restart": [
        "storage::tests::sessions_survive_studio_restart",
        "storage::tests::interrupted_observer_does_not_invent_child_exit",
        "storage::tests::interrupted_apply_becomes_indeterminate_after_observer_restart",
        "storage::tests::terminal_before_start_cannot_reopen_session",
        "storage::tests::historical_pid_never_authorizes_signal",
        "storage::tests::history_reconnect_has_no_missed_commit",
    ],
    "runtime-only": [
        "storage::tests::runtime_only_history_path_is_release_independent",
        "storage::tests::wal_backup_restore_preserves_committed_history",
        "storage::tests::binary_rollback_never_downgrades_history_schema",
        "update::tunnel_update::tests::tunnel_terminal_receipt_precedes_journal_cleanup",
        "update::tunnel_update::tests::history_failure_does_not_retain_recovery_journal",
        "update::self_update::tests::startup_does_not_finalize_external_activation_without_fleet_terminal_state",
        "update::self_update::tests::audit_self_update_requires_health_before_completed",
    ],
    "retention": [
        "storage::tests::aggregation_retry_is_idempotent",
        "storage::tests::retention_preserves_unexpired_audit_and_bounds_lineage",
        "storage::tests::history_cursor_survives_insert_and_expires_on_prune",
        "storage::tests::history_query_abuse_is_bounded",
        "storage::tests::history_housekeeping_never_invokes_runtime_policy",
        "storage::tests::partial_history_is_not_zero_or_exact_duration",
        "storage::tests::long_reader_cannot_create_unbounded_history_work",
        "storage::tests::housekeeping_crash_before_commit_is_all_or_nothing",
    ],
    "security": [
        "storage::tests::written_config_is_not_loaded_config",
        "storage::tests::validated_journal_revision_conflicts_and_gaps_are_explicit",
        "storage::update_history::tests::failed_check_cached_latest_is_not_fresh_availability",
        "storage::tests::active_lineage_reports_unknown_predecessor",
        "storage::tests::history_redaction_covers_db_api_and_backup",
        "api::tests::reconciliation_check_side_effect_is_audited",
    ],
}

M6_IGNORED = [
    "update::self_update::tests::external_launcher_success_smoke_preserves_local_state_and_switches_release",
    "update::self_update::tests::external_launcher_health_failure_smoke_rolls_back_legacy_release",
    "update::transaction::tests::live_safe_real_binary_activation_smoke",
    "update::reconciliation::tests::live_aira_runtime_only_reconciliation_smoke",
]

PROFILE_ORDER = [
    "foundation",
    "regressions",
    "security",
    "web",
    "restart",
    "runtime-only",
    "retention",
    "native",
]
VALID_PROFILES = set(PROFILE_ORDER + ["full"])


@dataclass(frozen=True)
class CommandSpec:
    gate: str
    name: str
    argv: tuple[str, ...]
    cwd: Path = ROOT
    exact_test: str | None = None
    timeout: int = 300


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def capture(argv: list[str], cwd: Path = ROOT) -> dict:
    try:
        proc = subprocess.run(argv, cwd=cwd, text=True, capture_output=True, check=False)
        return {
            "argv": argv,
            "exit": proc.returncode,
            "stdout": proc.stdout.strip(),
            "stderr": proc.stderr.strip(),
        }
    except FileNotFoundError as error:
        return {"argv": argv, "exit": 127, "stdout": "", "stderr": str(error)}


def git_provenance(cwd: Path) -> dict:
    commands = {
        "head": ["git", "rev-parse", "HEAD"],
        "branch": ["git", "branch", "--show-current"],
        "status": ["git", "status", "--porcelain=v1"],
        "diff": ["git", "diff", "--binary"],
        "staged": ["git", "diff", "--cached", "--binary"],
    }
    result = {name: capture(argv, cwd) for name, argv in commands.items()}
    raw = (
        result["head"]["stdout"]
        + "\n"
        + result["status"]["stdout"]
        + "\n"
        + result["diff"]["stdout"]
        + "\n"
        + result["staged"]["stdout"]
    ).encode()
    result["source_identity_sha256"] = sha256_bytes(raw)
    result["clean"] = not bool(result["status"]["stdout"].strip())
    return result


def provenance() -> dict:
    locks = {}
    for path in (ROOT / "Cargo.lock", ROOT / "web" / "pnpm-lock.yaml"):
        if path.is_file():
            locks[str(path.relative_to(ROOT))] = sha256_file(path)
    fleet_prov = git_provenance(FLEET) if FLEET.is_dir() else None
    return {
        "captured_at_utc": datetime.now(timezone.utc).isoformat(),
        "studio": git_provenance(ROOT),
        "fleet": fleet_prov,
        "locks": locks,
        "toolchains": {
            "rustc": capture(["rustc", "--version"]),
            "cargo": capture(["cargo", "--version"]),
            "python": capture(["python3", "--version"]),
            "node": capture(["node", "--version"]),
            "pnpm": capture(["pnpm", "--version"]),
            "cargo_audit": capture(["cargo", "audit", "--version"]),
            "rust_host_triple": rust_host_triple(),
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
    }


def exact_test(symbol: str, *, ignored: bool = False, gate: str) -> CommandSpec:
    suffix = ["--", "--exact", "--nocapture"]
    if ignored:
        suffix = ["--", "--ignored", "--exact", "--nocapture"]
    return CommandSpec(
        gate=gate,
        name=symbol,
        argv=tuple(["cargo", "test", "--locked", symbol, *suffix]),
        exact_test=symbol,
        timeout=300,
    )


def specs_for(profile: str) -> list[CommandSpec]:
    specs: list[CommandSpec] = []
    if profile == "foundation":
        specs.extend(
            [
                CommandSpec("Q01", "rust-fmt", ("cargo", "fmt", "--all", "--", "--check")),
                CommandSpec(
                    "Q02",
                    "rust-check",
                    ("cargo", "check", "--locked", "--all-targets", "--all-features"),
                ),
                CommandSpec(
                    "Q03",
                    "rust-clippy",
                    (
                        "cargo",
                        "clippy",
                        "--locked",
                        "--all-targets",
                        "--all-features",
                        "--",
                        "-D",
                        "warnings",
                    ),
                ),
                CommandSpec(
                    "Q04",
                    "history-tests",
                    ("cargo", "test", "--locked", "storage::"),
                    timeout=420,
                ),
                CommandSpec(
                    "Q05",
                    "rust-build-debug",
                    ("cargo", "build", "--locked", "--all-targets", "--all-features"),
                    timeout=420,
                ),
                CommandSpec(
                    "Q05",
                    "rust-build-release",
                    ("cargo", "build", "--release", "--locked"),
                    timeout=600,
                ),
            ]
        )
        specs.extend(exact_test(name, gate="Q06") for name in M6_EXACT["foundation"])
        if platform.system() == "Darwin":
            specs.extend(
                [
                    CommandSpec(
                        "Q06",
                        "real-filesystem-enospc",
                        ("python3", "scripts/verify-m6-enospc.py"),
                        timeout=180,
                    ),
                    CommandSpec(
                        "Q06",
                        "real-readonly-filesystem",
                        ("python3", "scripts/verify-m6-readonly.py"),
                        timeout=120,
                    ),
                ]
            )
    elif profile == "regressions":
        specs.append(
            CommandSpec(
                "Q04",
                "rust-full-suite",
                ("cargo", "test", "--locked", "--all-targets", "--all-features"),
                timeout=600,
            )
        )
        specs.extend(exact_test(name, gate="Q04") for name in M5_REGRESSIONS)
    elif profile == "security":
        specs.extend(exact_test(name, gate="Q08") for name in M6_EXACT["security"])
        specs.extend(
            [
                CommandSpec("Q08", "cargo-audit", ("cargo", "audit"), timeout=300),
                CommandSpec(
                    "Q08",
                    "pnpm-audit",
                    ("pnpm", "--dir", "web", "audit", "--audit-level", "high"),
                    timeout=300,
                ),
            ]
        )
    elif profile == "web":
        specs.extend(
            [
                CommandSpec(
                    "Q07",
                    "web-install",
                    ("pnpm", "--dir", "web", "install", "--frozen-lockfile"),
                    timeout=300,
                ),
                CommandSpec("Q07", "web-lint", ("pnpm", "--dir", "web", "lint")),
                CommandSpec("Q07", "web-typecheck", ("pnpm", "--dir", "web", "typecheck")),
                CommandSpec("Q07", "web-test", ("pnpm", "--dir", "web", "test")),
                CommandSpec("Q07", "web-build", ("pnpm", "--dir", "web", "build")),
            ]
        )
    elif profile == "restart":
        specs.extend(exact_test(name, gate="Q09") for name in M6_EXACT["restart"])
        specs.extend(
            [
                CommandSpec(
                    "Q09",
                    "supervisor-lifecycle-integration",
                    ("cargo", "test", "--locked", "--test", "supervisor_lifecycle"),
                ),
                CommandSpec(
                    "Q09",
                    "tunnel-lifecycle-integration",
                    ("cargo", "test", "--locked", "--test", "tunnel_lifecycle"),
                ),
            ]
        )
    elif profile == "runtime-only":
        specs.extend(
            [
                CommandSpec(
                    "Q11",
                    "runtime-only-release-build",
                    ("cargo", "build", "--release", "--locked"),
                    timeout=600,
                ),
                CommandSpec(
                    "Q11",
                    "runtime-only-web-build",
                    ("pnpm", "--dir", "web", "build"),
                    timeout=300,
                ),
                CommandSpec(
                    "Q11",
                    "runtime-only-package-smoke",
                    ("python3", "scripts/verify-m6-runtime-only.py"),
                    timeout=120,
                ),
            ]
        )
        specs.extend(exact_test(name, gate="Q11") for name in M6_EXACT["runtime-only"])
        specs.extend(exact_test(name, ignored=True, gate="Q11") for name in M6_IGNORED)
    elif profile == "retention":
        specs.extend(exact_test(name, gate="Q12") for name in M6_EXACT["retention"])
    elif profile == "native":
        specs.extend(
            [
                CommandSpec("Q13", "native-release-build", ("cargo", "build", "--release", "--locked"), timeout=600),
                CommandSpec(
                    "Q13",
                    "actual-baseline-binary-rollback",
                    ("python3", "scripts/verify-m6-binary-rollback.py"),
                    timeout=300,
                ),
                CommandSpec(
                    "Q13",
                    "native-package-fixture",
                    ("python3", "scripts/verify-native-package.py"),
                    timeout=600,
                ),
            ]
        )
    else:
        raise ValueError(profile)
    return specs


def selected_profiles(profile: str) -> list[str]:
    return PROFILE_ORDER if profile == "full" else [profile]


def terminate_group(pgid: int) -> bool:
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        return False
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            os.killpg(pgid, 0)
        except ProcessLookupError:
            return True
        time.sleep(0.05)
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    return True


TEST_RESULT_RE = re.compile(
    r"test result: (?:ok|FAILED)\. (?P<passed>\d+) passed; (?P<failed>\d+) failed; "
    r"(?P<ignored>\d+) ignored;"
)
VITEST_RE = re.compile(r"Tests\s+(?P<passed>\d+) passed")


def parse_counts(text: str) -> dict:
    rust = list(TEST_RESULT_RE.finditer(text))
    if rust:
        return {
            "framework": "rust",
            "passed": sum(int(match.group("passed")) for match in rust),
            "failed": sum(int(match.group("failed")) for match in rust),
            "ignored": sum(int(match.group("ignored")) for match in rust),
        }
    vitest = VITEST_RE.search(text)
    if vitest:
        return {
            "framework": "vitest",
            "passed": int(vitest.group("passed")),
            "failed": 0,
            "ignored": 0,
        }
    return {"framework": None, "passed": None, "failed": None, "ignored": None}



M5_PUBLICATION_EXPECTED = {
    "filesystem": ("v0.1.0", "rust-mcp-filesystem-v0.1.0-darwin-arm64.tar.gz"),
    "git": ("v0.1.0", "rust-mcp-git-v0.1.0-darwin-arm64.tar.gz"),
    "exec": ("v0.1.0", "rust-mcp-exec-v0.1.0-darwin-arm64.tar.gz"),
    "gateway": ("v0.1.0", "rust-mcp-gateway-v0.1.0-darwin-arm64.tar.gz"),
    "blender": ("v0.1.0", "rust-mcp-blender-v0.1.0-darwin-arm64.tar.gz"),
    "studio": ("v0.5.0", "mcp-studio-v0.5.0-darwin-arm64.tar.gz"),
    "fleet": ("v0.2.1", "mcp-fleet-v0.2.1.tar.gz"),
}


def validate_m5_publication_evidence(
    env: dict[str, str],
) -> tuple[bool, str | None, dict | None]:
    root_text = env.get("M5_RESULT_DIR")
    if not root_text:
        return (
            False,
            "M5_RESULT_DIR is not set; Q16 requires independently qualified published M5 evidence",
            None,
        )
    root = Path(root_text)
    try:
        root = root.resolve(strict=True)
    except FileNotFoundError:
        return False, f"M5_RESULT_DIR does not exist: {root_text}", None

    summary_path = root / "publication-summary.json"
    manifest_path = root / "publication-summary.sha256"
    if (
        not summary_path.is_file()
        or summary_path.is_symlink()
        or not manifest_path.is_file()
        or manifest_path.is_symlink()
    ):
        return False, "M5 publication summary/hash files are missing or unsafe", None

    raw = summary_path.read_bytes()
    digest = sha256_bytes(raw)
    if manifest_path.read_text().strip() != f"{digest}  publication-summary.json":
        return False, "M5 publication summary hash manifest does not match", None
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as error:
        return False, f"M5 publication summary is invalid JSON: {error}", None

    if data.get("schema_version") != 1 or data.get("publication_qualified") is not True:
        return False, "M5 publication summary is not schema v1 qualified evidence", None
    if data.get("required_platform") != "darwin-arm64":
        return False, "M5 publication evidence does not cover required darwin-arm64", None

    releases = data.get("project_releases")
    if not isinstance(releases, dict) or set(releases) != set(M5_PUBLICATION_EXPECTED):
        return False, "M5 publication evidence does not cover the exact project release set", None

    for repo, (tag, asset) in M5_PUBLICATION_EXPECTED.items():
        item = releases.get(repo)
        if not isinstance(item, dict):
            return False, f"M5 publication evidence missing {repo}", None
        if item.get("tag") != tag or item.get("asset") != asset:
            return False, f"M5 publication identity mismatch for {repo}", None
        checksum = item.get("sha256")
        if not isinstance(checksum, str) or not re.fullmatch(r"[0-9a-f]{64}", checksum):
            return False, f"M5 publication SHA-256 missing for {repo}", None
        prefix = f"https://github.com/13thx-mcp/{repo}/releases/download/{tag}/"
        if not str(item.get("asset_url", "")).startswith(prefix):
            return False, f"M5 publication asset authority mismatch for {repo}", None
        if item.get("checksum_url") != prefix + "SHA256SUMS.txt":
            return False, f"M5 publication checksum authority mismatch for {repo}", None

    tunnel = data.get("tunnel_release")
    if not isinstance(tunnel, dict):
        return False, "M5 publication evidence missing official Tunnel release", None
    if (
        tunnel.get("tag") != "v0.0.14"
        or tunnel.get("asset")
        != "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip"
    ):
        return False, "M5 publication Tunnel identity mismatch", None
    if not str(tunnel.get("asset_url", "")).startswith(
        "https://github.com/openai/tunnel-client/releases/download/v0.0.14/"
    ):
        return False, "M5 publication Tunnel authority mismatch", None

    bootstrap = data.get("source_less_bootstrap")
    if not isinstance(bootstrap, dict):
        return False, "M5 source-less bootstrap evidence is missing", None
    if (
        bootstrap.get("source_root_exists") is not False
        or bootstrap.get("host_mode") != "runtime_only"
        or bootstrap.get("studio_version") != "0.5.0"
    ):
        return False, "M5 source-less bootstrap did not prove runtime_only Studio v0.5.0", None

    transition = data.get("fleet_transition")
    if not isinstance(transition, dict):
        return False, "M5 published Fleet transition evidence is missing", None
    if (
        transition.get("source_version") != "0.2.0"
        or transition.get("target_version") != "0.2.1"
        or transition.get("prepare_phase") != "staged"
        or transition.get("apply_phase") != "completed"
        or transition.get("installed_before") != "0.2.0"
        or transition.get("latest_before") != "0.2.1"
        or transition.get("installed_after") != "0.2.1"
    ):
        return False, "M5 published Fleet transition is incomplete or inconsistent", None

    return (
        True,
        None,
        {
            "result_dir": str(root),
            "summary_sha256": digest,
            "qualified": True,
            "studio_release": releases["studio"]["tag"],
            "fleet_transition": "0.2.0->0.2.1",
            "platform": "darwin-arm64",
        },
    )


def run_command(spec: CommandSpec, index: int, out: Path, env: dict[str, str]) -> dict:
    log_path = out / f"{index:03d}-{spec.name.replace('/', '_').replace(':', '_')}.log"
    if spec.name == "native-package-fixture":
        qualified, evidence_error, _ = validate_m5_publication_evidence(env)
        if not qualified:
            detail = (
                "native package proof depends on independently qualified M5 publication "
                f"evidence: {evidence_error}"
            )
            log_path.write_text(detail + "\n")
            return {
                "gate": spec.gate,
                "name": spec.name,
                "argv": list(spec.argv),
                "cwd": str(spec.cwd),
                "status": "BLOCKED",
                "exit": None,
                "duration_s": 0.0,
                "timed_out": False,
                "validation_error": detail,
                "test_counts": {
                    "framework": None,
                    "passed": None,
                    "failed": None,
                    "ignored": None,
                },
                "log": log_path.name,
                "log_sha256": sha256_file(log_path),
            }
    started = time.monotonic()
    timed_out = False
    try:
        with log_path.open("wb") as log:
            proc = subprocess.Popen(
                list(spec.argv),
                cwd=spec.cwd,
                stdout=log,
                stderr=subprocess.STDOUT,
                env=env,
                start_new_session=True,
            )
            try:
                rc = proc.wait(timeout=spec.timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                terminate_group(proc.pid)
                rc = 124
                log.write(f"\nM6 verifier timeout after {spec.timeout}s\n".encode())
            if proc.poll() is None:
                terminate_group(proc.pid)
    except FileNotFoundError as error:
        log_path.write_text(str(error) + "\n")
        rc = 127

    duration = time.monotonic() - started
    text = log_path.read_text(errors="replace")
    counts = parse_counts(text)
    validation_error = None
    if rc == 0 and spec.exact_test:
        marker = f"test {spec.exact_test} ... ok"
        if marker not in text or not counts["passed"]:
            rc = 125
            validation_error = (
                f"required exact test did not execute successfully: {spec.exact_test}"
            )

    if rc == 0:
        status = "PASS"
    elif rc == 127:
        status = "BLOCKED"
    else:
        status = "FAIL"
    return {
        "gate": spec.gate,
        "name": spec.name,
        "argv": list(spec.argv),
        "cwd": str(spec.cwd),
        "status": status,
        "exit": rc,
        "duration_s": round(duration, 3),
        "timed_out": timed_out,
        "validation_error": validation_error,
        "test_counts": counts,
        "log": log_path.name,
        "log_sha256": sha256_file(log_path),
    }


def rust_host_triple() -> str | None:
    result = capture(["rustc", "-vV"])
    if result["exit"] != 0:
        return None
    for line in result["stdout"].splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    return None


def native_coverage() -> dict:
    system = platform.system().lower()
    rust_host = rust_host_triple()
    translated = capture(["sysctl", "-in", "sysctl.proc_translated"])["stdout"].strip()

    coverage = {
        "darwin-arm64": "BLOCKED",
        "darwin-amd64": "NOT_REQUIRED",
        "required_native_targets": ["darwin-arm64"],
        "rust_host": rust_host,
        "runner_process_machine": platform.machine(),
        "runner_process_translated": translated == "1",
    }
    if system != "darwin":
        coverage["reason"] = (
            f"M6 native qualification requires darwin-arm64; current host is {platform.system()}"
        )
        return coverage

    if rust_host == "aarch64-apple-darwin":
        coverage["darwin-arm64"] = "AVAILABLE_CURRENT_NATIVE_RUST_HOST"
        coverage["reason"] = (
            "required darwin-arm64 native target is available; darwin-amd64 is outside "
            "the current M6 supported target set"
        )
        return coverage

    coverage["reason"] = (
        "required darwin-arm64 native target is unavailable; Rosetta/cross-build evidence "
        "does not satisfy native qualification"
    )
    return coverage

def source_changed(before: dict, after: dict) -> bool:
    keys = ("head", "status", "diff", "staged")
    return any(
        before["studio"][key]["stdout"] != after["studio"][key]["stdout"]
        for key in keys
    )


def write_manifest(out: Path) -> dict:
    files = {}
    for path in sorted(out.rglob("*")):
        if path.is_file() and path.name != "manifest.json":
            files[str(path.relative_to(out))] = {
                "bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
    manifest = {"files": files}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description="M6 history foreground qualification runner")
    parser.add_argument("--profile", choices=sorted(VALID_PROFILES), default="foundation")
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    started_at = datetime.now(timezone.utc)
    stamp = started_at.strftime("%Y%m%d-%H%M%S")
    out = args.output.resolve() if args.output else RESULTS / stamp
    before = provenance()

    if args.require_clean and not before["studio"]["clean"]:
        out.mkdir(parents=True, exist_ok=True)
        summary = {
            "profile": args.profile,
            "status": "BLOCKED",
            "reason": "--require-clean requested but Studio worktree is dirty",
            "started_at_utc": started_at.isoformat(),
            "before": before,
            "commands": [],
        }
        (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        write_manifest(out)
        print(f"OVERALL: BLOCKED\nEVIDENCE_DIR: {out}")
        return 2

    if out.exists():
        print(f"OVERALL: BLOCKED")
        print(f"EVIDENCE_DIR: {out}")
        print("BLOCKED: requested evidence directory already exists")
        return 2
    out.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    commands = []
    index = 1
    stop_after_failure = False
    for profile in selected_profiles(args.profile):
        for spec in specs_for(profile):
            if stop_after_failure:
                commands.append(
                    {
                        "gate": spec.gate,
                        "name": spec.name,
                        "argv": list(spec.argv),
                        "status": "NOT RUN",
                    }
                )
                continue
            result = run_command(spec, index, out, env)
            commands.append(result)
            index += 1
            if result["status"] == "FAIL":
                stop_after_failure = True

    after = provenance()
    changed = source_changed(before, after)
    dirty = not before["studio"]["clean"]
    native = native_coverage()

    failures = [item for item in commands if item["status"] == "FAIL"]
    blocked_commands = [item for item in commands if item["status"] == "BLOCKED"]
    blocked_reasons = []
    if dirty:
        blocked_reasons.append("Q14 clean-source provenance blocked: Studio worktree was dirty at start")
    if changed:
        blocked_reasons.append("Q14 source changed while qualification was running")
    if args.profile in {"native", "full"} and "BLOCKED" in native.values():
        blocked_reasons.append(native["reason"])
    m5_publication_ok, m5_publication_error, m5_publication = (
        validate_m5_publication_evidence(env)
    )
    if args.profile == "full" and not m5_publication_ok:
        blocked_reasons.append(
            "Q16 M5 publication qualification blocked: "
            f"{m5_publication_error}"
        )

    if failures:
        overall = "FAIL"
    elif blocked_commands or blocked_reasons:
        overall = "BLOCKED"
    else:
        overall = "PASS"

    finished_at = datetime.now(timezone.utc)
    summary = {
        "profile": args.profile,
        "status": overall,
        "started_at_utc": started_at.isoformat(),
        "finished_at_utc": finished_at.isoformat(),
        "duration_s": round((finished_at - started_at).total_seconds(), 3),
        "require_clean": args.require_clean,
        "commands": commands,
        "blocked_reasons": blocked_reasons,
        "native_coverage": native,
        "m5_publication_evidence": m5_publication,
        "source_changed_during_run": changed,
        "before": before,
        "after": after,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    rows = ["gate\tstatus\texit\tduration_s\ttests_passed\tname\tlog"]
    for item in commands:
        counts = item.get("test_counts", {})
        rows.append(
            "\t".join(
                [
                    str(item.get("gate", "")),
                    str(item.get("status", "")),
                    str(item.get("exit", "")),
                    str(item.get("duration_s", "")),
                    str(counts.get("passed", "") if counts else ""),
                    str(item.get("name", "")),
                    str(item.get("log", "")),
                ]
            )
        )
    (out / "summary.tsv").write_text("\n".join(rows) + "\n")
    write_manifest(out)

    print(f"OVERALL: {overall}")
    print(f"EVIDENCE_DIR: {out}")
    print(f"SUMMARY_FILE: {out / 'summary.json'}")
    print(f"MANIFEST_FILE: {out / 'manifest.json'}")
    if blocked_reasons:
        for reason in blocked_reasons:
            print(f"BLOCKED: {reason}")
    return 1 if overall == "FAIL" else (2 if overall == "BLOCKED" else 0)


if __name__ == "__main__":
    sys.exit(main())
