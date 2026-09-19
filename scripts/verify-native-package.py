#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    if platform.system() != "Darwin":
        raise SystemExit("native Studio package verification requires a Darwin host")

    machine = platform.machine()
    platform_name = {
        "arm64": "darwin-arm64",
        "aarch64": "darwin-arm64",
        "x86_64": "darwin-amd64",
    }.get(machine)
    if platform_name is None:
        raise SystemExit(f"unsupported native Darwin machine: {machine}")

    with (ROOT / "Cargo.toml").open("rb") as handle:
        version = tomllib.load(handle)["package"]["version"]

    result_root_text = os.environ.get("M5_RESULT_DIR")
    if not result_root_text:
        raise SystemExit("M5_RESULT_DIR is required")
    result_root = Path(result_root_text)
    output = result_root / "native-package"
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)

    cargo_target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = cargo_target / "release" / "mcp-studio"
    web_dist = ROOT / "web" / "dist"
    if not binary.is_file():
        raise SystemExit(f"release binary missing: {binary}")
    if not (web_dist / "index.html").is_file():
        raise SystemExit("web/dist/index.html missing")

    name = f"mcp-studio-v{version}-{platform_name}"
    package_root = output / name
    (package_root / "web").mkdir(parents=True)
    shutil.copy2(binary, package_root / "mcp-studio")
    shutil.copytree(web_dist, package_root / "web" / "dist")
    for filename in ("README.md", "LICENSE"):
        source = ROOT / filename
        if source.is_file():
            shutil.copy2(source, package_root / filename)

    archive = output / f"{name}.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(package_root, arcname=name)

    with tempfile.TemporaryDirectory() as temp_name:
        temp = Path(temp_name)
        with tarfile.open(archive, "r:gz") as tf:
            members = tf.getmembers()
            for member in members:
                if member.issym() or member.islnk() or member.name.startswith("/") or ".." in Path(member.name).parts:
                    raise SystemExit(f"unsafe packaged member: {member.name}")
            tf.extractall(temp)
        extracted = temp / name
        extracted_binary = extracted / "mcp-studio"
        extracted_web = extracted / "web" / "dist" / "index.html"
        if not extracted_binary.is_file() or not extracted_web.is_file():
            raise SystemExit("extracted package is missing backend or web identity")
        proc = subprocess.run(
            [str(extracted_binary), "--version"],
            text=True,
            capture_output=True,
            check=False,
            timeout=10,
        )
        if proc.returncode != 0 or version not in proc.stdout:
            raise SystemExit("extracted package backend version mismatch")

    manifest = {
        "version": version,
        "platform": platform_name,
        "native_machine": machine,
        "archive": archive.name,
        "archive_sha256": sha256(archive),
        "backend_sha256": sha256(package_root / "mcp-studio"),
        "web_index_sha256": sha256(package_root / "web" / "dist" / "index.html"),
        "native_architecture_covered": platform_name,
        "cross_architecture_release_qualified": False,
        "note": "This proves only the current native Darwin architecture. The other Darwin architecture requires its own native runner.",
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
