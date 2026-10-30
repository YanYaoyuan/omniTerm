# Linux preview Agent build

Run `bash scripts/ci/build-linux-agent.sh` from a native Ubuntu 22.04 x86_64
runner after installing the repository's Rust 1.95.0 toolchain and Python 3.11+.
The build host needs `build-essential`, `clang`, `libclang-dev`, `cmake`,
`pkg-config`, `libcap-dev`, `libssl-dev`, `binutils`, Git, and download utilities.
Ubuntu 22.04's default Python 3.10 is too old for the package builder.

The script uses the `x86_64-unknown-linux-gnu` target and `dev-small` profile
for a fast preview build (`OMNITERM_CARGO_PROFILE=release` enables release
optimizations). Cargo fetches and builds with `--locked`; transient dependency
downloads are retried without changing dependency revisions. Cargo's Git CLI
transport also fetches the locked rusty_v8 submodules.

`bwrap` is built from the `codex-bwrap` crate first, and the SHA-256 of its final
bytes is embedded in the subsequent Codex build. The package helper compiles
`codex` and `codex-code-mode-host` from this checkout and stages that same bwrap.
It downloads checksum-verified third-party V8 archives/bindings and ripgrep;
it does not download a replacement Codex executable. Do not strip or otherwise
change the packaged bwrap after this step.

Output is the complete `dist/omniterm-agent/` directory, ready for the desktop
packager's `extraResource`. It includes `bin/codex`, `bin/codex-code-mode-host`,
`codex-resources/bwrap`, `codex-path/rg`, and `codex-package.json`. Rerunning the
script replaces that generated package directory. Cache Cargo's registry, Git
sources, target directory, and `$TMPDIR/codex-package` between CI builds when
available; do not reuse macOS build binaries for the Linux target.

## Offline package verification

Run `bash scripts/ci/verify-linux-agent.sh [path/to/omniterm-agent]` on Linux
x86_64 with Python 3.11+, `readelf`, and `ldd`. It checks the complete manifest,
ELF architecture, required glibc versions (at most 2.35), and library resolution.
The bundled CLI, ripgrep, and bwrap must start. The code-mode host must negotiate
its real stdio protocol, execute JavaScript in V8, and shut down cleanly.

The sandbox check uses a temporary package to prove that Codex rejects a changed
bwrap before executing it, then uses the original package to run a read-only
command and verify that writing into its working directory is denied. The
original package is never modified. `--expected-bwrap-sha256 HEX` additionally
compares the packaged resource to a digest retained from the build log.

All child configuration/state is temporary, credentials and shell startup hooks
are omitted, and no model or external server is contacted. A host that prohibits
unprivileged user namespaces fails verification by default. Passing
`--allow-unavailable-sandbox` permits only that host limitation and prints a
visible `SKIP` / `PARTIAL` result; it does not disable the sandbox or claim full
verification. Kernel namespace availability must still be checked on the target
Ubuntu desktop.

## Included source notices

The Linux build stages project `LICENSE` / `NOTICE`, Cargo dependency license
texts, rusty_v8 / V8 notices, checksum-pinned ripgrep release notices, and
bubblewrap's licenses and corresponding source into the Agent package. The
source archive contains the original Rust wrapper and build files, workspace
lockfile, and a recipe for rebuilding from the same repository revision.
It excludes Git metadata, caches and host paths. Executable bytes are unchanged.

For an already built package, this step can run independently without rebuilding:

```sh
python3 scripts/ci/stage-linux-agent-notices.py dist/omniterm-agent
```

This uses `cargo metadata --locked --offline` and the package builder's existing,
SHA-256-checked ripgrep archive cache. Use `--rg-archive /path/to/archive.tar.gz`
if that archive has moved. `third-party-licenses/index.json` records provenance,
copied files and any packages without separate discoverable license texts.
