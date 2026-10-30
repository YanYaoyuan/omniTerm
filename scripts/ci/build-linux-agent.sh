#!/usr/bin/env bash
# Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
# Build on Ubuntu 22.04 x86_64 so GNU-linked binaries retain its glibc baseline.
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
target="x86_64-unknown-linux-gnu"
profile="${OMNITERM_CARGO_PROFILE:-dev-small}"
python="${PYTHON:-python3}"

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "Build the Linux Agent on an x86_64 Linux host (Ubuntu 22.04 for this package)." >&2
  exit 1
fi
case "$profile" in
  dev-small|release) ;;
  *) echo "OMNITERM_CARGO_PROFILE must be dev-small or release." >&2; exit 1 ;;
esac
for dependency in cargo rustc git pkg-config sha256sum; do
  command -v "$dependency" >/dev/null || { echo "Missing build dependency: $dependency" >&2; exit 1; }
done
"$python" -c 'import sys; assert sys.version_info >= (3, 11), "Python 3.11 or newer is required"'
pkg-config --exists libcap || { echo "Install libcap-dev and pkg-config before building the Linux sandbox." >&2; exit 1; }

export CODEX_REPO_ROOT="$repository_root"
# Cargo still needs the exact locked rusty_v8 Git sources and submodules even
# when the package builder downloads verified V8 archives and bindings.
export CARGO_NET_GIT_FETCH_WITH_CLI=true
export CARGO_NET_RETRY="${CARGO_NET_RETRY:-5}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export STABLE_GIT_COMMIT="$(git -C "$repository_root" rev-parse HEAD)"

target_directory="${CARGO_TARGET_DIR:-$repository_root/codex-rs/target}"
if [[ "$target_directory" != /* ]]; then
  target_directory="$repository_root/codex-rs/$target_directory"
fi
export CARGO_TARGET_DIR="$target_directory"

retry_downloads() {
  local attempt status
  for attempt in 1 2 3; do
    if "$@"; then return 0; else status=$?; fi
    if [[ "$attempt" == 3 ]]; then return "$status"; fi
    echo "Dependency/package command failed (exit $status); retrying ($attempt/3)." >&2
    sleep "$((attempt * 5))"
  done
}

cd "$repository_root/codex-rs"
rustc --version
cargo --version
retry_downloads cargo fetch --locked --target "$target"

# Compile the resource first. Its final bytes are hashed into Codex; the package
# builder receives exactly this binary and must not rebuild or strip it later.
cargo build --locked --target "$target" --profile "$profile" -p codex-bwrap --bin bwrap
bwrap_binary="$CARGO_TARGET_DIR/$target/$profile/bwrap"
[[ -x "$bwrap_binary" ]] || { echo "Missing source-built bwrap: $bwrap_binary" >&2; exit 1; }
export CODEX_BWRAP_SHA256="$(sha256sum "$bwrap_binary" | cut -d ' ' -f 1)"
echo "Source commit: $STABLE_GIT_COMMIT"
echo "Bundled bwrap SHA-256: $CODEX_BWRAP_SHA256"

# The shared package builder has no --locked option. Wrap only its Cargo command
# so both source builds honor Cargo.lock without changing shared build helpers.
temporary_directory="$(mktemp -d "${TMPDIR:-/tmp}/omniterm-linux-agent.XXXXXX")"
trap 'rm -rf -- "$temporary_directory"' EXIT
export OMNITERM_LOCKED_CARGO="$(command -v cargo)"
cat > "$temporary_directory/cargo-locked" <<'CARGO_WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
exec "${OMNITERM_LOCKED_CARGO:?}" "$@" --locked
CARGO_WRAPPER
chmod +x "$temporary_directory/cargo-locked"

cd "$repository_root"
retry_downloads "$python" scripts/build_codex_package.py \
  --target "$target" \
  --cargo-profile "$profile" \
  --cargo "$temporary_directory/cargo-locked" \
  --bwrap-bin "$bwrap_binary" \
  --package-dir dist/omniterm-agent \
  --force

package_directory="$repository_root/dist/omniterm-agent"
"$python" scripts/ci/stage-linux-agent-notices.py "$package_directory" --cargo-profile "$profile"
printf '%s  %s\n' "$CODEX_BWRAP_SHA256" "$package_directory/codex-resources/bwrap" | sha256sum --check -
"$package_directory/bin/codex" --version
"$package_directory/codex-path/rg" --version
"$package_directory/codex-resources/bwrap" --version
echo "Source-built Linux Agent package: $package_directory"
