#!/usr/bin/env python3
from __future__ import annotations

import platform
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(argv: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=cwd or ROOT,
        text=True,
        capture_output=True,
        check=False,
        timeout=60,
    )


def main() -> int:
    if platform.system() != "Darwin":
        raise SystemExit("read-only filesystem qualification currently requires Darwin hdiutil")
    hdiutil = shutil.which("hdiutil")
    if not hdiutil:
        raise SystemExit("hdiutil is unavailable")

    binary = ROOT / "target" / "release" / "mcp-studio"
    if not binary.is_file():
        raise SystemExit(f"release binary missing: {binary}")

    with tempfile.TemporaryDirectory(prefix="mcp-studio-m6-readonly-") as name:
        root = Path(name)
        image = root / "readonly.dmg"
        mount = root / "mount"
        mount.mkdir()

        created = run(
            [
                hdiutil,
                "create",
                "-size",
                "32m",
                "-fs",
                "HFS+",
                "-volname",
                "M6READONLY",
                "-ov",
                str(image),
            ]
        )
        if created.returncode != 0:
            raise SystemExit(f"hdiutil create failed: {created.stderr}")

        # Initialize a valid history DB while the disposable volume is writable.
        attached = run(
            [hdiutil, "attach", "-nobrowse", "-mountpoint", str(mount), str(image)]
        )
        if attached.returncode != 0:
            raise SystemExit(f"hdiutil attach failed: {attached.stderr}")
        try:
            host = mount / "host"
            (host / "bin").mkdir(parents=True)
            studio = host / "runtime" / "studio"
            studio.mkdir(parents=True)
            installed = studio / "mcp-studio"
            shutil.copy2(binary, installed)
            initialized = run([str(installed), "history", "verify"], cwd=studio)
            if initialized.returncode != 0:
                raise SystemExit(f"history initialization failed: {initialized.stderr}")
        finally:
            detached = run([hdiutil, "detach", str(mount)])
            if detached.returncode != 0:
                run([hdiutil, "detach", "-force", str(mount)])

        # Reattach the exact same filesystem read-only and verify Studio fails closed.
        readonly = run(
            [
                hdiutil,
                "attach",
                "-readonly",
                "-nobrowse",
                "-mountpoint",
                str(mount),
                str(image),
            ]
        )
        if readonly.returncode != 0:
            raise SystemExit(f"read-only attach failed: {readonly.stderr}")
        try:
            studio = mount / "host" / "runtime" / "studio"
            installed = studio / "mcp-studio"
            verified = run([str(installed), "history", "verify"], cwd=studio)
            if verified.returncode == 0:
                raise SystemExit("history unexpectedly opened writable service on read-only filesystem")
            combined = (verified.stdout + "\n" + verified.stderr).lower()
            if not any(token in combined for token in ("history", "read-only", "readonly", "permission")):
                raise SystemExit(
                    "read-only failure did not surface a history/storage diagnostic: "
                    + combined[-1000:]
                )
        finally:
            detached = run([hdiutil, "detach", str(mount)])
            if detached.returncode != 0:
                run([hdiutil, "detach", "-force", str(mount)])

    print("real disposable read-only filesystem qualification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
