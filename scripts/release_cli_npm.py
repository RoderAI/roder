#!/usr/bin/env python3
"""Prepare and publish @roderai/roder from one checksummed CLI release."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
NAME = "@roderai/roder"
REGISTRY = "https://registry.npmjs.org"
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "aarch64-apple-darwin": "darwin-arm64",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def identity(version: str, commit: str) -> None:
    require(re.fullmatch(r"\d+\.\d+\.\d+", version) is not None, "Expected an x.y.z release version")
    require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "Expected a full release commit SHA")


def digest(path: Path, algorithm: str = "sha256") -> str:
    with path.open("rb") as source:
        hasher = hashlib.new(algorithm)
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
        return hasher.hexdigest()


def cli_version(root: Path = ROOT) -> str:
    manifest = (root / "crates/roder-cli/Cargo.toml").read_text()
    match = re.search(r'^version = "([^\"]+)"$', manifest, re.MULTILINE)
    require(match is not None, "CLI manifest has no explicit version")
    return match[1]


def write_manifest(artifacts: Path, target: str, commit: str) -> None:
    version = cli_version()
    identity(version, commit)
    archive = artifacts / f"roder-{target}.tar.gz"
    checksum = archive.with_name(archive.name + ".sha256").read_text().split()
    require(checksum == [digest(archive), archive.name], "Archive checksum does not match")
    data = {"product": "roder", "version": version, "commit_sha": commit,
            "target": target, "archive": archive.name, "sha256": checksum[0]}
    (artifacts / f"roder-{target}.json").write_text(json.dumps(data, indent=2) + "\n")


def extract_binary(artifacts: Path, target: str, destination: Path, version: str, commit: str) -> dict:
    manifest = json.loads((artifacts / f"roder-{target}.json").read_text())
    archive = artifacts / f"roder-{target}.tar.gz"
    expected = {"product": "roder", "version": version, "commit_sha": commit,
                "target": target, "archive": archive.name}
    require(all(manifest.get(k) == v for k, v in expected.items()), f"Wrong release identity: {target}")
    require(manifest.get("sha256") == digest(archive), f"Archive checksum mismatch: {target}")
    with tarfile.open(archive, "r:gz") as source:
        entries = [m for m in source.getmembers() if m.name == f"roder-{target}/roder"]
        require(len(entries) == 1 and entries[0].isfile(), f"Expected one regular Roder binary: {target}")
        with source.extractfile(entries[0]) as binary, destination.open("wb") as output:
            shutil.copyfileobj(binary, output)
    destination.chmod(0o755)
    return {"target": target, "sha256": digest(destination), "archive_sha256": manifest["sha256"]}


def build_package(artifacts: Path, output: Path, commit: str, root: Path = ROOT) -> Path:
    version = cli_version(root)
    identity(version, commit)
    output = output.resolve()
    require(not output.is_relative_to(root.resolve()), "Keep generated packages outside the checkout")
    output.mkdir(parents=True, exist_ok=True)
    package = output / "package"
    package.mkdir()
    template = root / "crates/roder-cli/npm"
    metadata = json.loads((template / "package.json").read_text())
    require(metadata["version"] == version and metadata["name"] == NAME, "Template and CLI version/name disagree")
    require(metadata.pop("private", None) is True, "Source template must be private")
    metadata["roder"] = {"commit_sha": commit}
    (package / "package.json").write_text(json.dumps(metadata, indent=2) + "\n")
    shutil.copyfile(template / "README.md", package / "README.md")
    shutil.copyfile(root / "LICENCE", package / "LICENSE")
    (package / "bin").mkdir()
    shutil.copyfile(template / "bin/roder.cjs", package / "bin/roder.cjs")
    (package / "bin/roder.cjs").chmod(0o755)
    platforms = {}
    for target, platform in TARGETS.items():
        directory = package / "bin" / platform
        directory.mkdir()
        platforms[platform] = extract_binary(artifacts, target, directory / "roder", version, commit)
    (package / "release.json").write_text(json.dumps({
        "product": "roder", "version": version, "commit_sha": commit, "platforms": platforms,
    }, indent=2) + "\n")
    packed = json.loads(subprocess.check_output([
        "npm", "pack", "--json", "--ignore-scripts", "--pack-destination", str(output),
    ], cwd=package, text=True))
    expected = {"package.json", "README.md", "LICENSE", "release.json", "bin/roder.cjs"} | {
        f"bin/{platform}/roder" for platform in TARGETS.values()
    }
    require(len(packed) == 1 and {f["path"] for f in packed[0]["files"]} == expected,
            "Unexpected files in the npm tarball")
    return output / packed[0]["filename"]


def fetch_json(url: str) -> dict | None:
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise


def registry_version(version: str) -> dict | None:
    return fetch_json(f"{REGISTRY}/{urllib.parse.quote(NAME, safe='')}/{version}")


def integrity(tarball: Path) -> str:
    return "sha512-" + base64.b64encode(bytes.fromhex(digest(tarball, "sha512"))).decode()


def package_identity(tarball: Path) -> tuple[str, str]:
    with tarfile.open(tarball, "r:gz") as source:
        metadata = json.load(source.extractfile("package/package.json"))
        release = json.load(source.extractfile("package/release.json"))
    version, commit = metadata["version"], metadata["roder"]["commit_sha"]
    identity(version, commit)
    require(metadata["name"] == NAME and not metadata.get("private"), "Wrong npm package")
    require(release["product"] == "roder" and release["version"] == version and release["commit_sha"] == commit,
            "Inconsistent package release identity")
    return version, commit


def verify_published(metadata: dict, tarball: Path) -> None:
    version, commit = package_identity(tarball)
    require(metadata.get("name") == NAME and metadata.get("version") == version and
            metadata.get("roder", {}).get("commit_sha") == commit, "npm version belongs to a different release")
    require(metadata.get("dist", {}).get("integrity") == integrity(tarball),
            "Published npm tarball differs; never overwrite an npm version")


def publish_package(tarball: Path) -> None:
    version, _ = package_identity(tarball)
    existing = registry_version(version)
    if existing is not None:
        verify_published(existing, tarball)
        print(f"Already published and verified: {NAME}@{version}", flush=True)
        return
    latest = registry_version("latest")
    if latest is not None:
        identity(latest["version"], "0" * 40)
        require(tuple(map(int, latest["version"].split("."))) <= tuple(map(int, version.split("."))),
                "Refusing to move npm latest backwards")
    subprocess.run(["npm", "publish", str(tarball), "--access", "public", "--ignore-scripts",
                    "--registry", REGISTRY, "--tag", "latest"], check=True)
    for attempt in range(91):
        published = registry_version(version)
        if published is not None:
            verify_published(published, tarball)
            print(f"Published and verified: {NAME}@{version}", flush=True)
            return
        if attempt < 90:
            if attempt % 6 == 0:
                print("npm accepted publication; waiting for public registry visibility.", flush=True)
            time.sleep(10)
    raise RuntimeError("npm accepted publication but it is not public after fifteen minutes; check before retrying")



def publication_source_check(commit: str) -> None:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(head == commit, "Package source commit differs from publishing source")
    require(not subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True),
            "npm publication requires clean source")
    if os.environ.get("GITHUB_ACTIONS") == "true":
        require(os.environ.get("GITHUB_REF") == "refs/heads/master", "Publish from the master workflow")
    else:
        subprocess.run(["git", "fetch", "origin", "master"], cwd=ROOT, check=True)
    result = subprocess.run(["git", "merge-base", "--is-ancestor", commit, "origin/master"], cwd=ROOT)
    require(result.returncode == 0, "Publish only landed release commits")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    manifest = commands.add_parser("manifest")
    manifest.add_argument("--artifacts", type=Path, required=True)
    manifest.add_argument("--target", choices=TARGETS, required=True)
    manifest.add_argument("--commit", required=True)
    pack = commands.add_parser("pack")
    pack.add_argument("--artifacts", type=Path, required=True)
    pack.add_argument("--output", type=Path, required=True)
    pack.add_argument("--commit", required=True)
    publish = commands.add_parser("publish")
    publish.add_argument("--tarball", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action in ("manifest", "pack"):
            head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
            require(args.commit == head, "Release commit differs from packaging source")
        if args.action == "manifest":
            write_manifest(args.artifacts, args.target, args.commit)
        elif args.action == "pack":
            print(build_package(args.artifacts, args.output, args.commit))
        else:
            _, commit = package_identity(args.tarball)
            publication_source_check(commit)
            publish_package(args.tarball)
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f"npm release stopped: {error}\n")


if __name__ == "__main__":
    main()
