#!/usr/bin/env python3
from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(argv: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )


def main() -> int:
    binary = ROOT / "target" / "release" / "mcp-studio"
    web_dist = ROOT / "web" / "dist"
    if not binary.is_file():
        raise SystemExit(f"release binary missing: {binary}")
    if not (web_dist / "index.html").is_file():
        raise SystemExit("web/dist/index.html missing")

    with tempfile.TemporaryDirectory(prefix="mcp-studio-m6-runtime-only-") as name:
        host = Path(name) / "mcp-server"
        (host / "bin").mkdir(parents=True)
        studio = host / "runtime" / "studio"
        (studio / "web").mkdir(parents=True)
        (studio / "releases" / "v0.6.0-alpha").mkdir(parents=True)
        installed = studio / "mcp-studio"
        shutil.copy2(binary, installed)
        shutil.copytree(web_dist, studio / "web" / "dist")

        forbidden = [
            studio / "Cargo.toml",
            studio / "package.json",
            studio / "node_modules",
            studio / "src",
            studio / "target",
        ]
        if any(path.exists() for path in forbidden):
            raise SystemExit("runtime-only fixture unexpectedly contains source/build inputs")

        version = run([str(installed), "--version"], studio)
        if version.returncode != 0 or "mcp-studio" not in version.stdout:
            raise SystemExit(
                f"source-less backend version probe failed: {version.returncode} {version.stderr}"
            )

        for _ in range(2):
            verified = run([str(installed), "history", "verify"], studio)
            if verified.returncode != 0 or "history verification passed" not in verified.stdout:
                raise SystemExit(
                    f"source-less history verify failed: {verified.returncode} "
                    f"{verified.stdout} {verified.stderr}"
                )

        backup = run([str(installed), "history", "backup"], studio)
        if backup.returncode != 0:
            raise SystemExit(
                f"source-less history backup failed: {backup.returncode} {backup.stderr}"
            )
        backup_path = Path(backup.stdout.strip()).resolve()
        if not backup_path.is_file():
            raise SystemExit(f"history backup missing: {backup_path}")

        history_root = host / "runtime" / "studio" / "data" / "history"
        database = history_root / "studio.sqlite3"
        if not database.is_file():
            raise SystemExit(f"history database missing: {database}")
        if database.is_relative_to(studio / "releases"):
            raise SystemExit("history database incorrectly depends on versioned release directory")
        if not (studio / "web" / "dist" / "index.html").is_file():
            raise SystemExit("source-less web asset identity missing")

        canonical_host = host.resolve()
        result = {
            "backend_version": version.stdout.strip(),
            "database_relative": str(database.resolve().relative_to(canonical_host)),
            "backup_relative": str(backup_path.relative_to(canonical_host)),
            "web_index_relative": str((studio / "web" / "dist" / "index.html").resolve().relative_to(canonical_host)),
            "source_checkout_required": False,
            "node_modules_required": False,
        }
        print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
