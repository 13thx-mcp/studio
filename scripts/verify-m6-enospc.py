#!/usr/bin/env python3
from __future__ import annotations

import os
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TEST = "storage::tests::live_filesystem_enospc_smoke"


def run(argv: list[str], *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=180,
    )


def main() -> int:
    if platform.system() != "Darwin":
        raise SystemExit("real disposable ENOSPC fixture currently requires Darwin hdiutil")
    hdiutil = shutil.which("hdiutil")
    if not hdiutil:
        raise SystemExit("hdiutil is unavailable")

    with tempfile.TemporaryDirectory(prefix="mcp-studio-m6-enospc-") as name:
        root = Path(name)
        image = root / "full.dmg"
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
                "M6ENOSPC",
                "-ov",
                str(image),
            ]
        )
        if created.returncode != 0:
            raise SystemExit(f"hdiutil create failed: {created.stderr}")

        attached = False
        try:
            attached_result = run(
                [
                    hdiutil,
                    "attach",
                    "-nobrowse",
                    "-mountpoint",
                    str(mount),
                    str(image),
                ]
            )
            if attached_result.returncode != 0:
                raise SystemExit(f"hdiutil attach failed: {attached_result.stderr}")
            attached = True

            host = mount / "host"
            (host / "bin").mkdir(parents=True)
            studio = host / "runtime" / "studio"
            studio.mkdir(parents=True)
            binary = ROOT / "target" / "release" / "mcp-studio"
            if not binary.is_file():
                raise SystemExit(f"release binary missing: {binary}")
            installed = studio / "mcp-studio"
            shutil.copy2(binary, installed)

            initialized = subprocess.run(
                [str(installed), "history", "verify"],
                cwd=studio,
                text=True,
                capture_output=True,
                check=False,
                timeout=30,
            )
            if initialized.returncode != 0:
                raise SystemExit(
                    f"could not initialize history on disposable volume: {initialized.stderr}"
                )

            free = shutil.disk_usage(mount).free
            reserve = 512 * 1024
            fill_bytes = max(0, free - reserve)
            filler = mount / "filler.bin"
            chunk = b"\0" * (256 * 1024)
            with filler.open("wb") as handle:
                remaining = fill_bytes
                while remaining:
                    piece = chunk if remaining >= len(chunk) else chunk[:remaining]
                    handle.write(piece)
                    remaining -= len(piece)
                handle.flush()
                os.fsync(handle.fileno())

            runtime_root = host / "runtime"
            env = os.environ.copy()
            env["M6_ENOSPC_RUNTIME_ROOT"] = str(runtime_root)
            tested = run(
                [
                    "cargo",
                    "test",
                    "--locked",
                    TEST,
                    "--",
                    "--ignored",
                    "--exact",
                    "--nocapture",
                ],
                env=env,
            )
            print(tested.stdout, end="")
            if tested.returncode != 0:
                raise SystemExit(
                    f"filesystem ENOSPC test failed: {tested.returncode}\n{tested.stderr}"
                )
            marker = f"test {TEST} ... ok"
            if marker not in tested.stdout:
                raise SystemExit("filesystem ENOSPC exact test did not execute")
        finally:
            if attached:
                detached = run([hdiutil, "detach", str(mount)])
                if detached.returncode != 0:
                    run([hdiutil, "detach", "-force", str(mount)])

    print("real disposable filesystem ENOSPC qualification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
