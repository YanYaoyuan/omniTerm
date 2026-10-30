#!/usr/bin/env python3
"""Verify a Linux Agent package locally, without a model, account, or network."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time


REPOSITORY = Path(__file__).resolve().parents[2]
BINARIES = (
    "bin/codex",
    "bin/codex-code-mode-host",
    "codex-resources/bwrap",
    "codex-path/rg",
)
MAX_GLIBC = (2, 35)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def run(arguments, *, environment=None, directory=None, check=True, timeout=30):
    process = subprocess.Popen(
        [str(value) for value in arguments],
        env=environment,
        cwd=directory,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        stop_process(process)
        process.communicate(timeout=3)
        raise RuntimeError(f"Command timed out: {arguments[0]}") from None
    result = subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)
    if check and result.returncode:
        raise RuntimeError(
            f"Command failed ({result.returncode}): {arguments[0]}\n"
            f"{result.stdout[-4000:]}{result.stderr[-4000:]}"
        )
    return result


def digest(filename: Path) -> str:
    with filename.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def verify_manifest(package: Path) -> dict:
    metadata = json.loads((package / "codex-package.json").read_text())
    for name, expected in {
        "layoutVersion": 1,
        "target": "x86_64-unknown-linux-gnu",
        "variant": "codex",
        "entrypoint": "bin/codex",
        "resourcesDir": "codex-resources",
        "pathDir": "codex-path",
    }.items():
        require(metadata.get(name) == expected, f"Invalid package manifest {name}: {metadata.get(name)!r}")
    require(isinstance(metadata.get("version"), str) and bool(metadata["version"]), "Package version is missing")
    for relative in BINARIES:
        filename = package / relative
        require(filename.is_file() and os.access(filename, os.X_OK), f"Missing executable: {filename}")
        require(filename.resolve().is_relative_to(package), f"Package executable escapes its resource directory: {filename}")
    print(f"PASS manifest: {metadata['target']}, package version {metadata['version']}", flush=True)
    return metadata


def verify_elf(package: Path) -> None:
    for relative in BINARIES:
        filename = package / relative
        with filename.open("rb") as source:
            header = source.read(20)
        require(len(header) == 20 and header[:4] == b"\x7fELF", f"Not ELF: {relative}")
        require(header[4:6] == b"\x02\x01" and struct.unpack("<H", header[18:20])[0] == 62, f"Not little-endian ELF64 x86_64: {relative}")
        versions = run(["readelf", "--version-info", filename]).stdout
        requirements = {tuple(map(int, value.split("."))) for value in re.findall(r"\bGLIBC_([0-9]+(?:\.[0-9]+)+)\b", versions)}
        newest = max(requirements, default=(0, 0))
        require(newest <= MAX_GLIBC, f"{relative} requires GLIBC_{'.'.join(map(str, newest))}, newer than Ubuntu 22.04's 2.35")
        program_headers = run(["readelf", "--program-headers", filename]).stdout
        if "INTERP" in program_headers:
            dependencies = run(["ldd", filename])
            require("not found" not in dependencies.stdout + dependencies.stderr, f"Unresolved libraries for {relative}:\n{dependencies.stdout}{dependencies.stderr}")
        baseline = ".".join(map(str, newest)) if requirements else "static/no versioned glibc dependency"
        print(f"PASS ELF / loader: {relative}, glibc {baseline}", flush=True)


def isolated_environment(package: Path, state_directory: Path) -> dict[str, str]:
    config_directory = state_directory / "codex-config"
    config_directory.mkdir()
    (config_directory / "config.toml").write_text(
        'approval_policy = "never"\n'
        'sandbox_mode = "read-only"\n'
        '[features]\napps = false\nplugins = false\nmemories = false\nuse_legacy_landlock = false\n'
        '[analytics]\nenabled = false\n'
        '[otel]\nexporter = "none"\ntrace_exporter = "none"\nmetrics_exporter = "none"\n'
    )
    # Deliberately omit host credentials, startup hooks, proxies, and system bwrap.
    # These overrides belong only to child processes; the caller is unchanged.
    return {
        "PATH": str(package / "codex-path"),
        "CODEX_HOME": str(config_directory),
        "SHELL": "/bin/sh",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TERM": "dumb",
    }


def stop_process(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=3)


def verify_code_mode_host(executable: Path, environment: dict[str, str], directory: Path) -> None:
    """Exercise protocol V1 and the actual bundled V8 runtime over stdin/stdout."""
    with tempfile.TemporaryFile() as errors:
        process = subprocess.Popen(
            [str(executable), "--listen", "stdio"],
            env=environment,
            cwd=directory,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=errors,
            bufsize=0,
            start_new_session=True,
        )

        def send(message):
            payload = json.dumps(message).encode()
            process.stdin.write(struct.pack("<I", len(payload)) + payload)
            process.stdin.flush()

        def read_exact(count, deadline):
            data = bytearray()
            while len(data) < count:
                remaining = deadline - time.monotonic()
                require(remaining > 0, "Timed out waiting for the code-mode host")
                ready, _, _ = select.select([process.stdout], [], [], remaining)
                require(bool(ready), "Timed out waiting for the code-mode host")
                chunk = os.read(process.stdout.fileno(), count - len(data))
                require(bool(chunk), "Code-mode host closed stdout unexpectedly")
                data.extend(chunk)
            return bytes(data)

        def receive(expected_type, request_id=None):
            deadline = time.monotonic() + 30
            for _ in range(64):
                length = struct.unpack("<I", read_exact(4, deadline))[0]
                require(length <= 1024 * 1024, "Unexpectedly large code-mode smoke response")
                message = json.loads(read_exact(length, deadline))
                require(message.get("type") != "connection/rejected", f"Code-mode handshake rejected: {message}")
                result = message.get("result", {})
                require(result.get("status") != "error", f"Code-mode request failed: {message}")
                if message.get("type") == expected_type and (request_id is None or message.get("id") == request_id):
                    return message
            raise RuntimeError("Code-mode host produced too many unrelated responses")

        def request(identifier, method, **fields):
            send({"type": "operation/request", "id": identifier, "request": {"method": method, "sessionId": "omniterm-smoke", **fields}})

        try:
            send({"type": "connection/hello", "supportedVersions": [1], "requiredCapabilities": [], "optionalCapabilities": []})
            require(receive("connection/ready").get("selectedVersion") == 1, "Unexpected code-mode protocol version")
            request(1, "session/open")
            require(receive("operation/response", 1)["result"]["value"]["type"] == "session/ready", "Code-mode session did not open")
            request(2, "session/execute", request={
                "tool_call_id": "offline-package-smoke",
                "enabled_tools": [],
                "source": 'text("OMNITERM_CODE_MODE_HOST_" + (6 * 7));',
                "yield_time_ms": 10000,
                "max_output_tokens": 256,
            })
            require(receive("operation/response", 2)["result"]["value"]["type"] == "execution/started", "Code-mode execution did not start")
            execution = receive("execute/initialResponse", 2)["result"]["value"].get("Result")
            require(execution is not None and not execution.get("error_text"), f"Code-mode V8 execution failed: {execution}")
            output = "".join(item.get("text", "") for item in execution["content_items"])
            require("OMNITERM_CODE_MODE_HOST_42" in output, f"Code-mode V8 returned the wrong result: {output!r}")
            request(3, "session/shutdown")
            require(receive("operation/response", 3)["result"]["value"]["type"] == "session/closed", "Code-mode session did not close")
            process.stdin.close()
            require(process.wait(timeout=10) == 0, "Code-mode host did not exit cleanly")
            print("PASS code-mode-host: protocol handshake, session, V8 evaluation, clean shutdown", flush=True)
        except Exception as error:
            stop_process(process)
            errors.seek(0)
            detail = errors.read().decode(errors="replace")[-4000:]
            raise RuntimeError(f"{error}\n{detail}") from error
        finally:
            stop_process(process)
            if not process.stdin.closed:
                process.stdin.close()
            process.stdout.close()


def verify_sandbox(package: Path, environment: dict[str, str], directory: Path, allow_unavailable: bool) -> bool:
    sandbox_args = ["-c", 'sandbox_mode="read-only"', "sandbox", "--", "/bin/sh", "-c"]
    # Hard-link the trusted executable into an isolated package, changing only
    # the copied resource. This proves Codex enforces the embedded digest before
    # running bwrap, even on a host whose namespace policy blocks the happy path.
    altered = directory / "tampered-package"
    (altered / "bin").mkdir(parents=True)
    (altered / "codex-resources").mkdir()
    (altered / "codex-path").mkdir()
    shutil.copyfile(package / "codex-package.json", altered / "codex-package.json")
    try:
        os.link(package / "bin/codex", altered / "bin/codex")
    except OSError:
        shutil.copy2(package / "bin/codex", altered / "bin/codex")
    resource = altered / "codex-resources/bwrap"
    resource.write_text("#!/bin/sh\nprintf 'OMNITERM_INVALID_BWRAP_WAS_EXECUTED\\n' >&2\nexit 97\n")
    resource.chmod(0o755)
    negative = run([altered / "bin/codex", *sandbox_args, "exit 0"], environment=environment, directory=directory, check=False)
    error_output = negative.stdout + negative.stderr
    require(negative.returncode != 0 and "bundled bubblewrap digest mismatch" in error_output, f"Codex did not reject a changed bundled bwrap:\n{error_output}")
    require("OMNITERM_INVALID_BWRAP_WAS_EXECUTED" not in error_output, "The changed bwrap was executed")
    print("PASS sandbox integrity: Codex rejected a changed bwrap before execution", flush=True)

    marker = "OMNITERM_SANDBOX_READ_ONLY_OK"
    positive = run(
        [package / "bin/codex", *sandbox_args, f"if (printf forbidden > smoke-write-probe) 2>/dev/null; then exit 91; fi; printf '{marker}\\n'"],
        environment=environment,
        directory=directory,
        check=False,
    )
    require(not (directory / "smoke-write-probe").exists(), "Read-only sandbox allowed a write to its working directory")
    if positive.returncode == 0 and marker in positive.stdout:
        print("PASS sandbox execution: bundled bwrap runs commands and denies workspace writes", flush=True)
        return True
    detail = positive.stdout + positive.stderr
    namespace_failure = re.search(r"(namespace|unshare|mount|proc).*(Operation not permitted|Permission denied|not allowed)|No permissions to create new namespace", detail, re.IGNORECASE)
    if allow_unavailable and namespace_failure:
        print(f"SKIP sandbox execution: this host prohibits required namespaces/mounts\n{detail[-2000:]}", flush=True)
        return False
    raise RuntimeError(f"Sandbox execution failed ({positive.returncode}); no bypass was used:\n{detail[-4000:]}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", nargs="?", type=Path, default=REPOSITORY / "dist/omniterm-agent")
    parser.add_argument("--expected-bwrap-sha256", help="Optional build-time bwrap digest to compare against the package")
    parser.add_argument("--allow-unavailable-sandbox", action="store_true", help="Report a clear SKIP when the host blocks namespaces; still require integrity verification")
    args = parser.parse_args()
    require(sys.version_info >= (3, 11), "Python 3.11 or newer is required")
    require(sys.platform == "linux" and platform.machine() == "x86_64", "Run verification on Linux x86_64")
    for tool in ("readelf", "ldd"):
        require(shutil.which(tool) is not None, f"Missing verification tool: {tool}")
    package = args.package.resolve(strict=True)
    verify_manifest(package)
    verify_elf(package)
    actual_digest = digest(package / "codex-resources/bwrap")
    if args.expected_bwrap_sha256:
        require(actual_digest == args.expected_bwrap_sha256.lower(), "Packaged bwrap differs from the supplied build digest")
    print(f"bwrap SHA-256: {actual_digest}", flush=True)
    print(f"Verifier host: {platform.platform()}, libc {platform.libc_ver()}", flush=True)

    with tempfile.TemporaryDirectory(prefix="omniterm-agent-verify-") as temporary:
        directory = Path(temporary)
        environment = isolated_environment(package, directory)
        cli = package / "bin/codex"
        checks = [([cli, "--version"], "codex-cli"), ([cli, "--help"], "Usage:"), ([cli, "features", "list"], "code_mode"), ([package / "codex-path/rg", "--version"], "ripgrep"), ([package / "codex-resources/bwrap", "--version"], "bubblewrap")]
        for arguments, expected in checks:
            output = run(arguments, environment=environment, directory=directory).stdout
            require(expected in output, f"Unexpected output from {arguments}: {output!r}")
        print("PASS CLI discovery and packaged ripgrep / bwrap", flush=True)
        verify_code_mode_host(package / "bin/codex-code-mode-host", environment, directory)
        sandbox_available = verify_sandbox(package, environment, directory, args.allow_unavailable_sandbox)
    if sandbox_available:
        print("PASS all offline Linux Agent package checks; no model credentials or external requests were used.")
    else:
        print("PARTIAL: package, runtime, and integrity checks passed; sandbox execution remains unverified on this host.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"FAIL Linux Agent verification: {error}", file=sys.stderr)
        raise SystemExit(1)
