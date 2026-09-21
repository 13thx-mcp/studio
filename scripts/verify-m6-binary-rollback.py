#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import os
import shutil
import sqlite3
import subprocess
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BASELINE_REF = "026753ba3737e6d04b71973240f77d15879b79e3"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout: int = 300,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
    )


def main() -> int:
    current = ROOT / "target" / "release" / "mcp-studio"
    if not current.is_file():
        raise SystemExit(f"current release binary missing: {current}")

    with tempfile.TemporaryDirectory(prefix="mcp-studio-m6-rollback-") as name:
        temp = Path(name)
        source = temp / "baseline"
        source.mkdir()
        archive = temp / "baseline.tar"

        baseline_ref = os.environ.get("M6_ROLLBACK_BASELINE", DEFAULT_BASELINE_REF)
        resolved = run(
            ["git", "rev-parse", "--verify", f"{baseline_ref}^{{commit}}"],
            cwd=ROOT,
            timeout=30,
        )
        if resolved.returncode != 0:
            raise SystemExit(
                f"rollback baseline is unavailable: {baseline_ref}: {resolved.stderr}"
            )
        baseline_commit = resolved.stdout.strip()

        archived = run(
            ["git", "archive", "--format=tar", "-o", str(archive), baseline_commit],
            cwd=ROOT,
            timeout=60,
        )
        if archived.returncode != 0:
            raise SystemExit(f"git archive failed: {archived.stderr}")
        with tarfile.open(archive, "r") as tf:
            tf.extractall(source, filter="data")

        old_target = ROOT / ".tmp" / "m6-history" / "rollback-target" / baseline_commit
        old_target.mkdir(parents=True, exist_ok=True)
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(old_target)
        built = run(
            ["cargo", "build", "--release", "--locked"],
            cwd=source,
            env=env,
            timeout=300,
        )
        if built.returncode != 0:
            raise SystemExit(
                f"baseline release build failed: {built.returncode}\n{built.stderr[-4000:]}"
            )
        old = old_target / "release" / "mcp-studio"
        if not old.is_file():
            raise SystemExit("baseline binary missing after build")

        host = temp / "host"
        (host / "bin").mkdir(parents=True)
        studio = host / "runtime" / "studio"
        studio.mkdir(parents=True)
        current_installed = studio / "mcp-studio-current"
        old_installed = studio / "mcp-studio-old"
        shutil.copy2(current, current_installed)
        shutil.copy2(old, old_installed)

        initialized = run([str(current_installed), "history", "verify"], cwd=studio)
        if initialized.returncode != 0:
            raise SystemExit(f"current history initialization failed: {initialized.stderr}")
        database = studio / "data" / "history" / "studio.sqlite3"
        if not database.is_file():
            raise SystemExit("current binary did not create history database")

        with sqlite3.connect(database) as connection:
            original_version = connection.execute("PRAGMA user_version").fetchone()[0]
            if original_version != 1:
                raise SystemExit(f"unexpected current schema version: {original_version}")
            migration = connection.execute(
                "SELECT version, sha256 FROM schema_migrations ORDER BY version"
            ).fetchall()
            connection.execute("PRAGMA user_version=2")
            connection.commit()

        incompatible_before = sha256(database)
        rejected = run([str(old_installed), "--history-verify"], cwd=studio)
        if rejected.returncode == 0:
            raise SystemExit("baseline binary unexpectedly accepted future history schema")
        incompatible_after = sha256(database)
        if incompatible_after != incompatible_before:
            raise SystemExit("baseline binary modified incompatible future history database")
        with sqlite3.connect(database) as connection:
            if connection.execute("PRAGMA user_version").fetchone()[0] != 2:
                raise SystemExit("baseline binary downgraded future schema version")

        # Restore the schema marker to the frozen compatible v1 and prove an actual
        # rollback binary can verify it without changing migration identity.
        with sqlite3.connect(database) as connection:
            connection.execute("PRAGMA user_version=1")
            connection.commit()
        compatible = run([str(old_installed), "--history-verify"], cwd=studio)
        if compatible.returncode != 0 or "history verification passed" not in compatible.stdout:
            raise SystemExit(
                "baseline binary could not verify compatible v1 history: "
                + compatible.stderr[-2000:]
            )
        with sqlite3.connect(database) as connection:
            if connection.execute("PRAGMA user_version").fetchone()[0] != 1:
                raise SystemExit("baseline binary changed compatible schema version")
            after_migration = connection.execute(
                "SELECT version, sha256 FROM schema_migrations ORDER BY version"
            ).fetchall()
            integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
        if after_migration != migration:
            raise SystemExit("baseline binary changed frozen migration identity")
        if integrity != "ok":
            raise SystemExit(f"history integrity failed after rollback binary: {integrity}")

        print(
            "actual baseline binary rollback qualification passed "
            f"(baseline_commit={baseline_commit}, baseline={old_installed}, "
            "schema=v1, future-schema rejection preserved bytes)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
