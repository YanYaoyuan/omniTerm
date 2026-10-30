#!/usr/bin/env python3
"""Stage source notices after an Agent build; never rewrite executable files."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile


REPOSITORY = Path(__file__).resolve().parents[2]
TARGET = "x86_64-unknown-linux-gnu"
LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)(?:[._-].*)?$", re.I)
SKIP_DIRECTORIES = {".git", ".cache", "target", "node_modules", "__pycache__"}
ROOT_PACKAGES = {"codex-cli", "codex-code-mode-host", "codex-bwrap"}


def sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def command(*arguments: str) -> str:
    return subprocess.check_output(arguments, cwd=REPOSITORY, text=True)


def write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def license_files(root: Path):
    for directory, children, filenames in os.walk(root, followlinks=False):
        children[:] = sorted(name for name in children if name not in SKIP_DIRECTORIES)
        for name in sorted(filenames):
            path = Path(directory) / name
            if LICENSE_NAME.fullmatch(name) and path.is_file() and path.resolve().is_relative_to(root):
                # Keep notices, not generated binaries with similar names.
                if path.stat().st_size <= 8 * 1024 * 1024 and b"\0" not in path.read_bytes():
                    yield path


def dependency_closure(metadata: dict) -> list[dict]:
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [package["id"] for package in packages.values() if package["name"] in ROOT_PACKAGES]
    if len(roots) != len(ROOT_PACKAGES):
        raise RuntimeError("Cargo metadata is missing an Agent root package")
    pending, seen = roots[:], set()
    while pending:
        identifier = pending.pop()
        if identifier in seen:
            continue
        seen.add(identifier)
        for dependency in nodes[identifier]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    return sorted((packages[identifier] for identifier in seen), key=lambda package: (package["name"], package["version"], package.get("source") or ""))


def collect_cargo_licenses(metadata: dict, destination: Path) -> list[dict]:
    records = []
    for package in dependency_closure(metadata):
        root = Path(package["manifest_path"]).parent.resolve()
        source = package.get("source") or "workspace"
        key = f"{package['name']}-{package['version']}"
        if source.startswith("git+"):
            key += "-" + source.rsplit("#", 1)[-1][:12]
        output = destination / "cargo" / key
        paths = list(license_files(root))
        # Workspace members share the project license; do not copy arbitrary
        # host ancestor files or publish Cargo's absolute manifest paths.
        if not paths and root.is_relative_to(REPOSITORY):
            paths = [REPOSITORY / "LICENSE", REPOSITORY / "NOTICE"]
        declared = package.get("license_file")
        if declared:
            declared = (root / declared).resolve()
            if declared.is_file() and declared not in paths:
                paths.append(declared)
        texts = []
        for path in paths:
            relative = path.relative_to(root) if path.is_relative_to(root) else Path("declared-or-shared") / path.name
            copied = output / relative
            copied.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, copied)
            texts.append({"path": copied.relative_to(destination).as_posix(), "sha256": sha256(copied)})
        records.append({"name": package["name"], "version": package["version"], "license": package.get("license"), "source": source, "texts": texts})
        if package["name"] == "v8":
            for required in (root / "LICENSE", root / "v8/LICENSE"):
                if required not in paths:
                    raise RuntimeError(f"Required rusty_v8/V8 license missing: {required.name}")
    if not any(record["name"] == "v8" for record in records):
        raise RuntimeError("Resolved code-mode dependencies did not include rusty_v8")
    return records


def collect_ripgrep_licenses(destination: Path, archive_override: Path | None) -> dict:
    # Reuse the same checksum-pinned release archive as the package builder.
    # No new network fetch or unpinned upstream license lookup is needed.
    manifest_text = (REPOSITORY / "scripts/codex_package/rg").read_text()
    manifest = json.loads(manifest_text[manifest_text.index("{"):])
    artifact = manifest["platforms"]["linux-x86_64"]
    url = artifact["providers"][0]["url"]
    archive_path = archive_override or Path(tempfile.gettempdir()) / "codex-package" / f"{TARGET}-rg" / url.rsplit("/", 1)[-1]
    if archive_path.stat().st_size != artifact["size"] or sha256(archive_path) != artifact["digest"]:
        raise RuntimeError("The ripgrep release archive differs from the locked DotSlash manifest")
    prefix = PurePosixPath(artifact["path"]).parent
    output = destination / "ripgrep"
    output.mkdir(parents=True, exist_ok=True)
    files = []
    with tarfile.open(archive_path, "r:gz") as archive:
        for name in ("COPYING", "LICENSE-MIT", "UNLICENSE"):
            member = archive.getmember(str(prefix / name))
            if not member.isfile():
                raise RuntimeError(f"ripgrep license is not a regular file: {name}")
            extracted = archive.extractfile(member)
            if extracted is None:
                raise RuntimeError(f"Missing ripgrep license: {name}")
            with extracted:
                (output / name).write_bytes(extracted.read())
            files.append({"path": f"ripgrep/{name}", "sha256": sha256(output / name)})
    return {"name": "ripgrep", "artifact": url, "artifactSha256": artifact["digest"], "texts": files}


def source_archive(package: Path, commit: str, profile: str) -> dict:
    archive_directory = package / "sources"
    archive_directory.mkdir(exist_ok=True)
    filename = archive_directory / "bubblewrap-codex-source.tar.gz"
    prefixes = ["codex-rs/bwrap", "codex-rs/vendor/bubblewrap"]
    tracked = command("git", "ls-files", "-z", "--", *prefixes).split("\0")
    tracked = [name for name in tracked if name and ".git" not in Path(name).parts]
    tracked += ["LICENSE", "NOTICE", "codex-rs/Cargo.toml", "codex-rs/Cargo.lock", "codex-rs/rust-toolchain.toml"]
    readme = f"""Bubblewrap and Codex Rust launcher corresponding source
======================================================

omniTerm source revision: {commit}
Target: {TARGET}
Cargo profile: {profile}
The original wrapper, build.rs, Cargo.toml, configuration, entire tracked
bubblewrap source tree, workspace manifest and lockfile are preserved here.
Bubblewrap's COPYING/LICENSE and per-file notices retain their original terms.
The wrapper's project license and notices are at LICENSE and NOTICE.

Rebuild on Ubuntu 22.04 x86_64 with the Rust version specified in the included
rust-toolchain.toml, a C compiler, pkg-config and libcap-dev. The wrapper is a
workspace member, so obtain the rest of the exact workspace before building:

  git clone https://github.com/YanYaoyuan/omniTerm.git omniterm-source
  cd omniterm-source
  git checkout {commit}
  # Overlay this archive's preserved files from its containing directory:
  tar -xzf /path/to/bubblewrap-codex-source.tar.gz --strip-components=1
  cd codex-rs
  cargo build --locked --target {TARGET} --profile {profile} -p codex-bwrap --bin bwrap

The output is target/{TARGET}/{profile}/bwrap. Dependency downloads require
network access unless Cargo's locked dependencies are already cached. This is
a source rebuild recipe, not a promise of identical compiler-produced bytes.
Codex checks the SHA-256 of its bundled bwrap. If replacing this resource,
rebuild Codex with CODEX_BWRAP_SHA256 set to the replacement's SHA-256 and
assemble the matching package; simply changing the resource is rejected.
"""

    def add_bytes(archive, name, payload, executable=False):
        entry = tarfile.TarInfo("bubblewrap-codex-source/" + name)
        entry.size, entry.mode, entry.mtime = len(payload), 0o755 if executable else 0o644, 0
        entry.uid = entry.gid = 0
        entry.uname = entry.gname = ""
        archive.addfile(entry, io.BytesIO(payload))

    with filename.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            add_bytes(archive, "REBUILD.txt", readme.encode())
            for relative in sorted(set(tracked)):
                path = REPOSITORY / relative
                if not path.resolve().is_relative_to(REPOSITORY) or not path.is_file():
                    raise RuntimeError(f"Invalid source archive member: {relative}")
                add_bytes(archive, relative, path.read_bytes(), bool(path.stat().st_mode & 0o111))
    (archive_directory / "REBUILD-bubblewrap.txt").write_text(readme)
    return {"path": filename.relative_to(package).as_posix(), "sha256": sha256(filename), "sourceCommit": commit}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", nargs="?", type=Path, default=REPOSITORY / "dist/omniterm-agent")
    parser.add_argument("--cargo-metadata", type=Path, help="Existing locked Linux cargo metadata JSON; never copied into the package")
    parser.add_argument("--rg-archive", type=Path, help="Existing ripgrep archive matching the checked-in Linux SHA-256")
    parser.add_argument("--cargo-profile", choices=("dev-small", "release"), default="dev-small")
    args = parser.parse_args()
    if sys.version_info < (3, 11):
        raise RuntimeError("Python 3.11 or newer is required")
    package = args.package.resolve(strict=True)
    if json.loads((package / "codex-package.json").read_text())["target"] != TARGET:
        raise RuntimeError(f"Expected an Agent package for {TARGET}")
    binary = package / "codex-resources/bwrap"
    before = sha256(binary)
    metadata = json.loads(args.cargo_metadata.read_text()) if args.cargo_metadata else json.loads(command(
        "cargo", "metadata", "--manifest-path", str(REPOSITORY / "codex-rs/Cargo.toml"),
        "--locked", "--offline", "--format-version", "1", "--filter-platform", TARGET,
    ))
    for name in ("LICENSE", "NOTICE"):
        shutil.copyfile(REPOSITORY / name, package / name)
    destination = package / "third-party-licenses"
    destination.mkdir(exist_ok=True)
    records = collect_cargo_licenses(metadata, destination)
    ripgrep = collect_ripgrep_licenses(destination, args.rg_archive)
    bubblewrap_directory = destination / "bubblewrap"
    bubblewrap_directory.mkdir(exist_ok=True)
    for name in ("COPYING", "LICENSE"):
        shutil.copyfile(REPOSITORY / "codex-rs/vendor/bubblewrap" / name, bubblewrap_directory / name)
    commit = command("git", "rev-parse", "HEAD").strip()
    source = source_archive(package, commit, args.cargo_profile)
    missing = [f"{record['name']} {record['version']}" for record in records if not record["texts"]]
    write_json(destination / "index.json", {
        "schemaVersion": 1, "target": TARGET, "sourceCommit": commit,
        "scope": "Cargo metadata normal/build dependency closure for codex-cli, codex-code-mode-host and codex-bwrap; workspace feature unification may include additional packages. This is a collected notice inventory, not a claim that all resolved code is linked.",
        "cargoPackages": records, "ripgrep": ripgrep, "bubblewrapSource": source,
        "packagesWithoutDiscoveredLicenseText": missing,
    })
    (package / "THIRD_PARTY_NOTICES.txt").write_text(
        "omniTerm Agent third-party notices\n\n"
        "Project LICENSE and NOTICE accompany this package. Actual dependency\n"
        "license/notice texts are under third-party-licenses/, with names, versions,\n"
        "source identifiers and file hashes in third-party-licenses/index.json.\n"
        "The inventory includes rusty_v8, V8 and nested third-party notices found\n"
        "in the locked source tree, and the checksum-pinned ripgrep release notices.\n"
        "Bubblewrap COPYING/LICENSE are under third-party-licenses/bubblewrap/.\n"
        "Its corresponding source, Rust launcher and rebuild instructions are in\n"
        "sources/bubblewrap-codex-source.tar.gz and sources/REBUILD-bubblewrap.txt.\n"
        "Packages without discoverable separate license texts are listed explicitly\n"
        "in index.json; Cargo license expressions are preserved without fabrication.\n"
    )
    if sha256(binary) != before:
        raise RuntimeError("The bundled bwrap changed while staging notices")
    print(f"Staged notices for {len(records)} Cargo packages, ripgrep and bubblewrap; bwrap bytes unchanged.")
    if missing:
        print("No separate license text discovered for: " + ", ".join(missing), file=sys.stderr)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(f"Failed to stage Agent notices: {error}")
