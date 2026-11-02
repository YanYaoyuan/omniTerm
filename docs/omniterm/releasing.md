# Building downloadable omniTerm releases

The desktop version lives in `omniterm-desktop/package.json`. A release contains
the repository-built Codex Agent, code-mode host, pinned ripgrep, and required
platform helpers. It does not substitute an upstream stock Codex executable.
The current targets are macOS arm64, Ubuntu 22.04 amd64, and Windows x64.

These builds use the pinned Rust 1.95.0 toolchain, Python 3.11+, Node 24, the
native compiler toolchain, and the desktop npm lockfile. `dev-small` is a preview
profile with optimizations disabled; the manifest records it explicitly.
Use a clean checkout of the intended release commit. Do not package personal
configuration, authentication files, SSH keys, or a populated Electron userData.

## Native builds

Install platform prerequisites from [the source build guide](local-build.md).
Ubuntu additionally needs `libcap-dev`, `libssl-dev`, `pkg-config`, CMake,
Clang/libclang, build-essential, and the Electron runtime libraries. Build on
Ubuntu 22.04 to retain its glibc baseline. Windows needs the x64 MSVC developer
environment; the workflow uses the repository's existing setup-msvc-env action.

From the repository root, replace TARGET with one of
`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, or `x86_64-pc-windows-msvc`:

```sh
python scripts/release/build-agent.py --target TARGET
python scripts/release/stage-agent-notices.py dist/omniterm-agent --target TARGET
cd omniterm-desktop
npm ci
npm run build:native
npm test
npm run package
cd ..
node scripts/release/verify-desktop.mjs
python scripts/release/verify-agent-offline.py dist/omniterm-agent
node omniterm-desktop/scripts/package-release.mjs
```

Use `python3` instead of `python` where appropriate. The Agent builder invokes
`cargo build --locked` for the required binaries only, authenticates the V8 and
ripgrep downloads against checked-in checksums, and records source/binary
provenance in `dist/omniterm-agent/omniterm-source.json`. Linux first builds bwrap
and embeds its exact hash into Codex. Windows includes both command-runner and
sandbox-setup executables. No whole-workspace build is necessary.

The notice collector uses locked online Cargo metadata by default to fetch any
additional dependency source needed for license collection, without updating
Cargo.lock. A complete cache can use `--offline`. It includes project, Cargo,
V8 and ripgrep notices; Linux also includes the corresponding bubblewrap source.

For release automation, set `OMNITERM_REQUIRE_AGENT_PROVENANCE=1` before
`npm run package`. If Electron's matching ZIP is already cached, set
`OMNITERM_ELECTRON_ZIP_DIR` to its containing directory to avoid another download.

Outputs under `dist/release/`:

- `omniTerm-VERSION-macos-arm64.zip`: an application bundle.
- `omniterm_VERSION-1_amd64.deb`: installs `/opt/omniterm` and `/usr/bin/omniterm`.
- `omniTerm-VERSION-windows-x64.zip`: portable directory; extract the entire ZIP
  and launch `omniTerm.exe`.
- Per-artifact `.sha256` and `.manifest.json` files. Linux also has Debian build
  metadata and its checksum record.

Recipients do not need Node, Rust, npm, or a separate Codex installation. Linux
APT installs declared runtime/SSH/serial dependencies and recommends Noto CJK
fonts. Its launcher clears ELECTRON_RUN_AS_NODE and gives effective empty/C/POSIX
character locales a UTF-8 fallback, including LC_ALL precedence. Standard UTF-8
locales are preserved. The chrome-sandbox helper is root-owned mode 4755; no
sandbox-disabling launch flag is added.

## Reusing an existing local Agent

An existing complete Agent may be reused only with its known build revision and
an identical Agent source fingerprint. The destination must not already exist:

```sh
python scripts/release/build-agent.py --target TARGET \
  --reuse-directory /path/to/known-source-built-agent \
  --reuse-source-commit ORIGINAL_BUILD_COMMIT
```

The fingerprint covers the entire codex-rs tree, package builder and V8 pins.
The manifest keeps the Agent build commit separate from the current desktop
commit and records all required binary hashes. Reuse is not a claim of binary
reproducibility across different compilers or operating systems. Collect notices
and run the normal desktop packaging/validation steps afterward.

## Native GitHub Actions

[omniterm-release.yml](../../.github/workflows/omniterm-release.yml) builds
Windows x64, macOS arm64 and Ubuntu 22.04 amd64 on native GitHub-hosted runners.
It does not publish a release or use signing secrets. Dispatch it on the release
preparation branch after committing version and release notes.

Each job uploads the complete source-built Agent before desktop packaging so
later packaging failures do not discard a successful Rust build. Optional
`reuse_agent_run_id`, `reuse_macos_agent_run_id` and `reuse_linux_agent_run_id`
inputs select previous Windows, macOS and Linux Agent artifacts respectively.
The target, exact Agent source fingerprint and every required binary hash must
match. Windows also retains the `OMNITERM_WINDOWS_AGENT_RUN_ID` repository-variable
fallback; clear it after use. No stock Codex fallback is accepted.

Cargo uses two workers without incremental output. Only dependency downloads
are cached; compiler objects are removed from ephemeral runner storage after
license notices are collected. Every platform builds the terminal host, runs
desktop tests, verifies its packaged runtime, and produces the native artifact.

The final job downloads all three desktop artifacts and validates completeness,
version, source commit, Agent provenance, sizes and SHA-256 hashes. It adds
`SHA256SUMS` and uploads `omniterm-complete-release-COMMIT`. Download that complete
set, rerun `python scripts/release/validate-release.py dist/release` from the exact
release checkout, and attach it to a draft GitHub release. Review the assets and
validation record before publishing. Do not label an incomplete platform set as
a complete release.

## Signatures and verification scope

These preview assets have no Developer ID, Windows Authenticode signature, or
Apple notarization. macOS gets an ad-hoc signature with no certificate or secret
so the final bundle is sealed. The original Agent build hashes remain in its
provenance; changed Mach-O hashes after signing are recorded separately as
`packagedSignedBinarySha256`. The release packager verifies both the source
fingerprint and final signature/hash records. In manifests, `signed: false`
means no publisher certificate; `adHocSigned` and `developerIDSigned` clarify
macOS state. OS trust prompts may appear for these unsigned previews.

The packaged-runtime smoke check runs the artifact's own Electron and terminal
host, executes shell commands and Unicode input, requests a resize, tests
Ctrl+C/close, and runs the bundled Agent's `--version`. A separate offline host
check negotiates the actual framed stdio protocol, opens a session, evaluates
`6 * 7` in bundled V8, and shuts down cleanly with tools disabled. It sends no
model request. These checks do not establish GUI
keyboard/IME behavior, authenticated model tasks, production SSH access, serial
hardware support, or complete functionality across every supported OS version.
A successful CI artifact should not be described as a full end-to-end acceptance.
