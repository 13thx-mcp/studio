#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import shutil
import signal
import socket
import stat
import subprocess
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath

USER_AGENT = "mcp-studio-m5-publication-qualification"
MAX_DOWNLOAD_BYTES = 256 * 1024 * 1024
MAX_ARCHIVE_FILES = 8192
MAX_EXTRACTED_BYTES = 512 * 1024 * 1024

PROJECT_LATEST = {
    "filesystem": ("0.1.0", "rust-mcp-filesystem", "rust-mcp-filesystem"),
    "git": ("0.1.0", "rust-mcp-git", "rust-mcp-git"),
    "exec": ("0.1.0", "rust-mcp-exec", "rust-mcp-exec"),
    "gateway": ("0.1.0", "rust-mcp-gateway", "rust-mcp-gateway"),
    "blender": ("0.1.0", "rust-mcp-blender", "rust-mcp-blender"),
    "studio": ("0.5.0", "mcp-studio", "mcp-studio"),
    "fleet": ("0.2.1", "mcp-fleet", None),
}
FLEET_BOOTSTRAP_VERSION = "0.2.0"
FLEET_TARGET_VERSION = "0.2.1"
TUNNEL_VERSION = "0.0.14"


class QualificationError(RuntimeError):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str, *, timeout: int = 60) -> bytes:
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": USER_AGENT,
            "Accept": "application/vnd.github+json",
        },
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        data = response.read(MAX_DOWNLOAD_BYTES + 1)
    if len(data) > MAX_DOWNLOAD_BYTES:
        raise QualificationError(f"download exceeded limit: {url}")
    return data


def fetch_json(url: str) -> dict:
    try:
        return json.loads(fetch(url).decode("utf-8"))
    except (urllib.error.URLError, json.JSONDecodeError, UnicodeDecodeError) as error:
        raise QualificationError(f"could not fetch JSON {url}: {error}") from error


def project_release(repo: str, *, tag: str | None = None) -> dict:
    suffix = f"/releases/tags/{tag}" if tag else "/releases/latest"
    return fetch_json(f"https://api.github.com/repos/13thx-mcp/{repo}{suffix}")


def official_tunnel_release() -> dict:
    return fetch_json("https://api.github.com/repos/openai/tunnel-client/releases/latest")


def asset_map(release: dict) -> dict[str, str]:
    assets: dict[str, str] = {}
    for item in release.get("assets", []):
        name = item.get("name")
        url = item.get("browser_download_url")
        if not isinstance(name, str) or not isinstance(url, str) or name in assets:
            raise QualificationError("release contains invalid or duplicate asset metadata")
        assets[name] = url
    return assets


def parse_checksum(manifest: bytes, asset_name: str) -> str:
    try:
        text = manifest.decode("utf-8")
    except UnicodeDecodeError as error:
        raise QualificationError("checksum manifest is not UTF-8") from error
    matches: list[str] = []
    for line in text.splitlines():
        parts = line.strip().split()
        if len(parts) >= 2 and parts[-1].lstrip("*") == asset_name:
            matches.append(parts[0].lower())
    if len(matches) != 1 or len(matches[0]) != 64:
        raise QualificationError(f"checksum manifest has no unique SHA-256 for {asset_name}")
    try:
        int(matches[0], 16)
    except ValueError as error:
        raise QualificationError(f"invalid SHA-256 for {asset_name}") from error
    return matches[0]


def download_verified_release_asset(
    release: dict,
    asset_name: str,
    download_dir: Path,
) -> tuple[Path, dict]:
    assets = asset_map(release)
    if asset_name not in assets or "SHA256SUMS.txt" not in assets:
        raise QualificationError(
            f"release {release.get('tag_name')} missing {asset_name} or SHA256SUMS.txt"
        )
    manifest = fetch(assets["SHA256SUMS.txt"])
    expected = parse_checksum(manifest, asset_name)
    blob = fetch(assets[asset_name])
    actual = sha256_bytes(blob)
    if actual != expected:
        raise QualificationError(
            f"checksum mismatch for {asset_name}: expected {expected}, got {actual}"
        )
    destination = download_dir / asset_name
    destination.write_bytes(blob)
    return destination, {
        "tag": release.get("tag_name"),
        "asset": asset_name,
        "sha256": actual,
        "asset_url": assets[asset_name],
        "checksum_url": assets["SHA256SUMS.txt"],
    }


def safe_relative(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if path.is_absolute() or not path.parts or ".." in path.parts or "." in path.parts:
        raise QualificationError(f"unsafe archive member: {name}")
    return path


def safe_extract_tar(archive: Path, destination: Path) -> None:
    count = 0
    total = 0
    with tarfile.open(archive, "r:gz") as handle:
        for member in handle.getmembers():
            relative = safe_relative(member.name)
            count += 1
            if count > MAX_ARCHIVE_FILES:
                raise QualificationError("tar archive contains too many entries")
            target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            if not member.isfile():
                raise QualificationError(f"tar archive contains non-regular entry: {member.name}")
            total += member.size
            if total > MAX_EXTRACTED_BYTES:
                raise QualificationError("tar archive extracted bytes exceed limit")
            target.parent.mkdir(parents=True, exist_ok=True)
            source = handle.extractfile(member)
            if source is None:
                raise QualificationError(f"could not read archive member: {member.name}")
            with target.open("wb") as output:
                shutil.copyfileobj(source, output)
            os.chmod(target, member.mode & 0o777)


def safe_extract_zip(archive: Path, destination: Path) -> None:
    count = 0
    total = 0
    with zipfile.ZipFile(archive) as handle:
        for member in handle.infolist():
            relative = safe_relative(member.filename)
            count += 1
            if count > MAX_ARCHIVE_FILES:
                raise QualificationError("zip archive contains too many entries")
            unix_mode = (member.external_attr >> 16) & 0xFFFF
            if unix_mode and not (
                stat.S_ISREG(unix_mode) or stat.S_ISDIR(unix_mode)
            ):
                raise QualificationError(
                    f"zip archive contains unsafe entry: {member.filename}"
                )
            target = destination.joinpath(*relative.parts)
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            total += member.file_size
            if total > MAX_EXTRACTED_BYTES:
                raise QualificationError("zip archive extracted bytes exceed limit")
            target.parent.mkdir(parents=True, exist_ok=True)
            with handle.open(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
            mode = unix_mode & 0o777 if unix_mode else 0o644
            os.chmod(target, mode)


def package_root(archive: Path) -> Path:
    name = archive.name
    if not name.endswith(".tar.gz"):
        raise QualificationError(f"not a tar.gz package: {name}")
    return Path(name[:-7])


def run_checked(argv: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> str:
    proc = subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        timeout=60,
        check=False,
    )
    if proc.returncode != 0:
        raise QualificationError(
            f"command failed ({proc.returncode}): {argv}\nstdout={proc.stdout}\nstderr={proc.stderr}"
        )
    return proc.stdout.strip()


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def http_json(
    url: str,
    *,
    method: str = "GET",
    payload: dict | None = None,
    timeout: int = 60,
) -> tuple[int, dict | list]:
    data = None
    headers = {"User-Agent": USER_AGENT}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read(4 * 1024 * 1024)
            return response.status, json.loads(body.decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise QualificationError(
            f"HTTP {error.code} from {url}: {body}"
        ) from error


def toml_string(value: Path | str) -> str:
    return json.dumps(str(value))


def wait_for_health(base_url: str, process: subprocess.Popen[bytes], log_path: Path) -> dict:
    deadline = time.monotonic() + 30
    last_error = "not attempted"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            tail = log_path.read_text(errors="replace")[-6000:]
            raise QualificationError(
                f"published Studio exited before health became ready: rc={process.returncode}\n{tail}"
            )
        try:
            status, payload = http_json(f"{base_url}/health", timeout=2)
            if status == 200 and isinstance(payload, dict):
                return payload
        except Exception as error:  # bounded retry during process startup
            last_error = str(error)
        time.sleep(0.1)
    raise QualificationError(f"timed out waiting for published Studio health: {last_error}")


def stop_process(process: subprocess.Popen[bytes]) -> int:
    if process.poll() is None:
        process.send_signal(signal.SIGTERM)
        try:
            return process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            return process.wait(timeout=5)
    return int(process.returncode or 0)


def copy_project_binary(extracted: Path, root_name: str, binary: str, bin_root: Path) -> Path:
    source = extracted / root_name / binary
    if not source.is_file():
        raise QualificationError(f"published package missing binary {binary}")
    destination = bin_root / binary
    shutil.copy2(source, destination)
    os.chmod(destination, 0o755)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = (args.output or Path(".tmp") / "m5-publication" / stamp).resolve()
    if output.exists():
        raise SystemExit(f"output already exists: {output}")
    output.mkdir(parents=True)
    downloads = output / "downloads"
    downloads.mkdir()

    summary: dict = {
        "schema_version": 1,
        "publication_qualified": False,
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "required_platform": "darwin-arm64",
        "project_releases": {},
        "fleet_transition": {},
        "tunnel_release": {},
        "source_less_bootstrap": {},
    }

    project_assets: dict[str, Path] = {}
    try:
        for repo, (version, stem, binary) in PROJECT_LATEST.items():
            release = project_release(repo)
            expected_tag = f"v{version}"
            if release.get("tag_name") != expected_tag or release.get("draft") or release.get("prerelease"):
                raise QualificationError(
                    f"{repo} latest release is not stable {expected_tag}: {release.get('tag_name')}"
                )
            asset_name = (
                f"{stem}-v{version}.tar.gz"
                if repo == "fleet"
                else f"{stem}-v{version}-darwin-arm64.tar.gz"
            )
            archive, evidence = download_verified_release_asset(
                release, asset_name, downloads
            )
            evidence["binary"] = binary
            summary["project_releases"][repo] = evidence
            project_assets[repo] = archive

        fleet_source_release = project_release("fleet", tag=f"v{FLEET_BOOTSTRAP_VERSION}")
        fleet_source_asset, fleet_source_evidence = download_verified_release_asset(
            fleet_source_release,
            f"mcp-fleet-v{FLEET_BOOTSTRAP_VERSION}.tar.gz",
            downloads,
        )
        summary["fleet_transition"]["source_release"] = fleet_source_evidence

        tunnel_release = official_tunnel_release()
        if tunnel_release.get("tag_name") != f"v{TUNNEL_VERSION}":
            raise QualificationError(
                f"official tunnel latest is {tunnel_release.get('tag_name')}, expected v{TUNNEL_VERSION}"
            )
        tunnel_asset_name = (
            f"tunnel-client-runtime-cloudflared-v{TUNNEL_VERSION}-darwin-arm64.zip"
        )
        tunnel_asset, tunnel_evidence = download_verified_release_asset(
            tunnel_release, tunnel_asset_name, downloads
        )
        summary["tunnel_release"] = tunnel_evidence

        with tempfile.TemporaryDirectory(prefix="m5-publication-runtime-") as fixture_name:
            fixture = Path(fixture_name)
            bin_root = fixture / "bin"
            runtime_root = fixture / "runtime"
            missing_source = fixture / "missing-source"
            registry_root = fixture / "registry-root"
            bin_root.mkdir()
            runtime_root.mkdir()
            registry_root.mkdir()
            if missing_source.exists():
                raise QualificationError("source-less fixture source root unexpectedly exists")

            deployed_versions: dict[str, str] = {}
            for repo in ("filesystem", "git", "exec", "gateway", "blender"):
                version, stem, binary = PROJECT_LATEST[repo]
                extracted = fixture / f"extract-{repo}"
                extracted.mkdir()
                safe_extract_tar(project_assets[repo], extracted)
                root_name = project_assets[repo].name[:-7]
                installed = copy_project_binary(extracted, root_name, str(binary), bin_root)
                file_description = run_checked(["file", str(installed)], cwd=fixture)
                if "arm64" not in file_description:
                    raise QualificationError(f"{repo} published binary is not arm64: {file_description}")
                version_output = run_checked([str(installed), "--version"], cwd=fixture)
                if version not in version_output:
                    raise QualificationError(
                        f"{repo} published binary version mismatch: {version_output}"
                    )
                deployed_versions[repo] = version

            fleet_extract = fixture / "extract-fleet"
            fleet_extract.mkdir()
            safe_extract_tar(fleet_source_asset, fleet_extract)
            fleet_package = fleet_extract / f"mcp-fleet-v{FLEET_BOOTSTRAP_VERSION}"
            fleet_root = runtime_root / "fleet"
            shutil.copytree(fleet_package, fleet_root)
            if (fleet_root / "VERSION").read_text().strip() != FLEET_BOOTSTRAP_VERSION:
                raise QualificationError("Fleet bootstrap package VERSION mismatch")

            host_id = "publication"
            port = find_free_port()
            host_profile = fleet_root / "hosts" / f"{host_id}.toml"
            host_profile.write_text(
                "\n".join(
                    [
                        "schema_version = 1",
                        f'host_id = "{host_id}"',
                        f"workspace_root = {toml_string(fixture)}",
                        f"source_root = {toml_string(missing_source)}",
                        f"bin_root = {toml_string(bin_root)}",
                        f"runtime_root = {toml_string(runtime_root)}",
                        "",
                        "[gateway]",
                        'server_dir = "gateway/servers.d"',
                        "",
                        "[studio]",
                        f'listen_addr = "127.0.0.1:{port}"',
                        "",
                        "[servers.filesystem]",
                        "enabled = true",
                        "timeout_ms = 30000",
                        "tool_allowlist = []",
                        "",
                        "[servers.git]",
                        "enabled = true",
                        "timeout_ms = 30000",
                        "extra_args = []",
                        "tool_allowlist = []",
                        "",
                        "[servers.exec]",
                        "enabled = true",
                        "timeout_ms = 30000",
                        "extra_args = []",
                        "tool_allowlist = []",
                        "",
                    ]
                )
            )
            (fleet_root / "state").mkdir(exist_ok=True)

            render_env = {
                "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
                "PYTHONDONTWRITEBYTECODE": "1",
            }
            render_json = run_checked(
                [
                    "python3",
                    str(fleet_root / "scripts/fleetctl.py"),
                    "render-plan",
                    "--host",
                    host_id,
                    "--json",
                ],
                cwd=fleet_root,
                env=render_env,
            )
            render_plan = json.loads(render_json)
            for command_name in ("render-gateway", "render-studio", "render-tunnel"):
                run_checked(
                    [
                        "python3",
                        str(fleet_root / "scripts/fleetctl.py"),
                        command_name,
                        "--host",
                        host_id,
                    ],
                    cwd=fleet_root,
                    env=render_env,
                )

            tunnel_release_root = (
                runtime_root / "tunnel-client" / "releases" / f"v{TUNNEL_VERSION}"
            )
            tunnel_release_root.mkdir(parents=True)
            safe_extract_zip(tunnel_asset, tunnel_release_root)
            tunnel_binary = tunnel_release_root / "tunnel-client-runtime-cloudflared"
            if not tunnel_binary.is_file():
                raise QualificationError("published tunnel package missing runtime binary")
            os.chmod(tunnel_binary, 0o755)
            tunnel_root = runtime_root / "tunnel-client"
            (tunnel_root / "current").symlink_to(
                Path("releases") / f"v{TUNNEL_VERSION}"
            )
            if not (tunnel_root / "config.yaml").is_file():
                raise QualificationError("Fleet did not render tunnel config")
            tunnel_version_output = run_checked(
                [str(tunnel_binary), "--version"], cwd=tunnel_root
            )

            studio_extract = fixture / "extract-studio"
            studio_extract.mkdir()
            safe_extract_tar(project_assets["studio"], studio_extract)
            studio_version = PROJECT_LATEST["studio"][0]
            studio_package = (
                studio_extract / f"mcp-studio-v{studio_version}-darwin-arm64"
            )
            studio_root = runtime_root / "studio"
            studio_release_root = studio_root / "releases" / f"v{studio_version}"
            studio_release_root.parent.mkdir(parents=True)
            shutil.copytree(studio_package, studio_release_root)
            studio_binary = studio_release_root / "mcp-studio"
            os.chmod(studio_binary, 0o755)
            (studio_root / "current").symlink_to(
                Path("releases") / f"v{studio_version}"
            )
            (studio_root / "data").mkdir(exist_ok=True)
            studio_version_output = run_checked(
                [str(studio_binary), "--version"], cwd=studio_root
            )
            if studio_version not in studio_version_output:
                raise QualificationError(
                    f"published Studio version mismatch: {studio_version_output}"
                )

            config_path = studio_root / "studio.toml"
            if not config_path.is_file():
                raise QualificationError("Fleet did not render Studio config")
            for rendered in (
                runtime_root / "gateway/servers.d/filesystem.yaml",
                runtime_root / "gateway/servers.d/git.yaml",
                runtime_root / "gateway/servers.d/exec.yaml",
                runtime_root / "tunnel-client/config.yaml",
            ):
                if not rendered.is_file():
                    raise QualificationError(
                        f"Fleet did not render managed config: {rendered}"
                    )

            studio_log = output / "published-studio.log"
            with studio_log.open("wb") as log:
                process = subprocess.Popen(
                    [str(studio_binary), "--config", str(config_path)],
                    cwd=studio_root,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    env={**os.environ, "RUST_LOG": "info", "PYTHONDONTWRITEBYTECODE": "1"},
                    start_new_session=True,
                )
                try:
                    base_url = f"http://127.0.0.1:{port}"
                    health = wait_for_health(base_url, process, studio_log)
                    if str(health.get("version")) != studio_version:
                        raise QualificationError(f"Studio health version mismatch: {health}")

                    status, inventory = http_json(
                        f"{base_url}/api/updates/check",
                        method="POST",
                        timeout=120,
                    )
                    if status != 200 or not isinstance(inventory, list):
                        raise QualificationError("updates/check did not return inventory list")
                    by_component = {
                        item.get("component"): item
                        for item in inventory
                        if isinstance(item, dict)
                    }
                    required_components = set(PROJECT_LATEST) | {"tunnel"}
                    if not required_components.issubset(by_component):
                        raise QualificationError(
                            f"inventory missing components: {sorted(required_components - set(by_component))}"
                        )
                    host_modes = {item.get("host_mode") for item in inventory if isinstance(item, dict)}
                    if host_modes != {"runtime_only"}:
                        raise QualificationError(f"inventory did not prove runtime_only mode: {host_modes}")
                    fleet_before = by_component["fleet"]
                    if fleet_before.get("installed_version") != FLEET_BOOTSTRAP_VERSION:
                        raise QualificationError(f"unexpected Fleet installed version: {fleet_before}")
                    if fleet_before.get("latest_version") != FLEET_TARGET_VERSION:
                        raise QualificationError(f"unexpected Fleet latest version: {fleet_before}")
                    if fleet_before.get("update_available") is not True:
                        raise QualificationError("Fleet published update was not reported available")

                    status, prepared = http_json(
                        f"{base_url}/api/updates/fleet/prepare",
                        method="POST",
                        payload={"version": FLEET_TARGET_VERSION},
                        timeout=120,
                    )
                    if status != 200 or not isinstance(prepared, dict) or prepared.get("phase") != "staged":
                        raise QualificationError(f"Fleet prepare did not stage published target: {prepared}")
                    transaction_id = prepared.get("transaction_id")
                    if not isinstance(transaction_id, str) or not transaction_id:
                        raise QualificationError("Fleet prepare returned no transaction id")

                    try:
                        status, applied = http_json(
                            f"{base_url}/api/updates/fleet/apply",
                            method="POST",
                            payload={"transaction_id": transaction_id},
                            timeout=120,
                        )
                    except QualificationError as apply_error:
                        _, failed_transaction = http_json(
                            f"{base_url}/api/update-transactions/{transaction_id}",
                            timeout=30,
                        )
                        scratch = sorted(
                            path.name
                            for path in runtime_root.iterdir()
                            if path.name.startswith(".mcp-studio-fleet-")
                        )
                        summary["fleet_transition"].update(
                            {
                                "transaction_id": transaction_id,
                                "failed_transaction": failed_transaction,
                                "scratch_after_failure": scratch,
                            }
                        )
                        raise QualificationError(
                            f"{apply_error}; transaction={failed_transaction}; scratch={scratch}"
                        ) from apply_error
                    if status != 200 or not isinstance(applied, dict) or applied.get("phase") != "completed":
                        raise QualificationError(f"Fleet published update did not complete: {applied}")

                    status, inventory_after = http_json(
                        f"{base_url}/api/updates/check",
                        method="POST",
                        timeout=120,
                    )
                    if status != 200 or not isinstance(inventory_after, list):
                        raise QualificationError("post-update check did not return inventory list")
                    fleet_after = next(
                        (
                            item
                            for item in inventory_after
                            if isinstance(item, dict) and item.get("component") == "fleet"
                        ),
                        None,
                    )
                    if not isinstance(fleet_after, dict) or fleet_after.get("installed_version") != FLEET_TARGET_VERSION:
                        raise QualificationError(f"Fleet installed version did not advance: {fleet_after}")
                    if (fleet_root / "VERSION").read_text().strip() != FLEET_TARGET_VERSION:
                        raise QualificationError("Fleet VERSION on disk did not advance")
                    if host_profile.read_text() == "":
                        raise QualificationError("Fleet host-local profile was not preserved")

                    summary["fleet_transition"].update(
                        {
                            "source_version": FLEET_BOOTSTRAP_VERSION,
                            "target_version": FLEET_TARGET_VERSION,
                            "transaction_id": transaction_id,
                            "prepare_phase": prepared.get("phase"),
                            "apply_phase": applied.get("phase"),
                            "installed_before": fleet_before.get("installed_version"),
                            "latest_before": fleet_before.get("latest_version"),
                            "installed_after": fleet_after.get("installed_version"),
                            "profile_sha256_after": sha256_bytes(host_profile.read_bytes()),
                        }
                    )
                    summary["source_less_bootstrap"] = {
                        "source_root_exists": missing_source.exists(),
                        "host_mode": "runtime_only",
                        "studio_version": studio_version,
                        "studio_health": health,
                        "studio_binary_version_output": studio_version_output,
                        "tunnel_binary_version_output": tunnel_version_output,
                        "fleet_render_plan_host": render_plan.get("host_id"),
                        "deployed_versions": deployed_versions,
                    }
                finally:
                    studio_exit = stop_process(process)

            if studio_exit != 0:
                tail = studio_log.read_text(errors="replace")[-6000:]
                raise QualificationError(
                    f"published Studio did not shut down cleanly: rc={studio_exit}\n{tail}"
                )

            if missing_source.exists():
                raise QualificationError("source-less fixture unexpectedly created source root")

        summary["publication_qualified"] = True
        summary["finished_at_utc"] = datetime.now(timezone.utc).isoformat()
        summary_bytes = json.dumps(summary, indent=2, sort_keys=True).encode("utf-8") + b"\n"
        (output / "publication-summary.json").write_bytes(summary_bytes)
        (output / "publication-summary.sha256").write_text(
            f"{sha256_bytes(summary_bytes)}  publication-summary.json\n"
        )
        print(f"M5_RESULT_DIR={output}")
        print("PUBLICATION_QUALIFIED")
        return 0
    except Exception as error:
        summary["publication_qualified"] = False
        summary["finished_at_utc"] = datetime.now(timezone.utc).isoformat()
        summary["error"] = str(error)
        summary_bytes = json.dumps(summary, indent=2, sort_keys=True).encode("utf-8") + b"\n"
        (output / "publication-summary.json").write_bytes(summary_bytes)
        (output / "publication-summary.sha256").write_text(
            f"{sha256_bytes(summary_bytes)}  publication-summary.json\n"
        )
        print(f"M5_RESULT_DIR={output}")
        print(f"PUBLICATION_FAILED: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
