# Ubuntu 22.04 amd64 package

Build the `.deb` natively on Ubuntu 22.04 x64. It contains Electron, the
Electron-compatible node-pty module, and the complete GNU x86-64 Agent package.
Recipients do **not** install Node.js, Rust, or npm. Model use still requires
Agent authentication; local terminals work without a model account.

## Build

Use the pinned lockfile and Electron version. On the build machine, install
Node.js 22.12+ (24 recommended), npm, the Rust toolchain pinned by this checkout,
Python 3.11+, C/C++ build tools, CMake, `pkg-config`, `dpkg`, `binutils`, and the
native dependencies required by the Agent. Build the Agent from the repository
root with the [Linux build script](../../scripts/ci/README.md), which embeds
the bundled sandbox's integrity hash before building Codex:

```sh
bash scripts/ci/build-linux-agent.sh
bash scripts/ci/verify-linux-agent.sh dist/omniterm-agent
cd omniterm-desktop
npm ci
npm run rebuild
npm test
npm run build
npm run test:pty
npm run package
node scripts/package-deb.mjs --version 0.1.0-1
```

Do not reuse macOS/Windows `node_modules` or Agent binaries. `package-deb.mjs`
consumes `dist/desktop/omniTerm-linux-x64/`; it does not rebuild it. Rebuild that
bundle after any source, native-module, or Agent change.

Commit source changes before packaging. With Git metadata present, the script
checks `git status --porcelain` and rejects a dirty checkout by default. For an
intentional preview only, pass `--allow-dirty`; the build record reports
`sourceDirty: true` and the explicit override. A source archive without Git
metadata reports `sourceCommit: null` and `sourceDirty: null`, not a verified
clean checkout.

The script rejects other build platforms, a non-GNU-x64 Agent manifest, wrong
ELF architectures, a missing native node-pty module, escaping symlinks, and
missing libraries reported by `ldd`. A compatible ELF header does not prove an
Electron ABI match: `npm run test:pty` above is required to check real loading.

For source archives without `.git`, set `SOURCE_DATE_EPOCH` to the source
commit timestamp before packaging. With Git metadata available it is read from
HEAD. This stabilizes archive timestamps for identical input files; it does not
claim that arbitrary recompilations are byte-for-byte reproducible.

Outputs in `dist/deb/`:

- `omniterm_<version>_amd64.deb`
- A `.sha256` checksum file.
- A `.build.json` record with architecture, dependency list, source commit when
  available, and hashes of the application/Agent/native module.

Compression uses `dpkg-deb --threads-max=4` when supported. Stock Ubuntu 22.04
dpkg predates that option, so the script uses `taskset` (from `util-linux`) to
restrict compression to at most four CPUs from the builder's allowed CPU list.
The build record includes the applied thread limit or CPU affinity.

Use `--app-dir`, `--output-dir`, `--version`, or `--maintainer` for explicit
build metadata. Run `node scripts/package-deb.mjs --help` for options.

## Runtime dependencies

The initial dependency list covers Ubuntu 22.04's common Electron/GTK/X11,
NSS, audio, graphics, C/C++ and Agent support libraries, plus CA certificates,
`xdg-utils`, OpenSSH, Python 3, and distribution-packaged pyserial. All are APT
packages; no install hook downloads code or compiles a native module.

Before distribution, review the actual GNU Agent, Electron, helper binaries,
`.so` files and `pty.node` with `dpkg-shlibdeps` on the target system. Add missing
packages or stronger minimum versions through a file containing comma-separated
Debian relationships, or a single `shlibs:Depends=...` line:

```sh
node scripts/package-deb.mjs --version 0.1.0-1 --depends-file /tmp/omniterm-shlibs.txt
```

Those relationships are merged with the baseline and recorded in the build
report. `ldd` checks catch missing libraries on the build host; a clean-machine
installation must also prove that the declared APT dependencies are sufficient.
A successful build on a provisioned development machine alone is not sufficient.

## Installed layout and sandbox

The bundle is installed under `/opt/omniterm/`. `/usr/bin/omniterm` clears
`ELECTRON_RUN_AS_NODE` and executes the bundled application. The package adds
`/usr/share/applications/omniterm.desktop` and the existing icon under the
hicolor icon theme. The desktop entry has no file/URL placeholders because the
application does not implement those opening semantics.

Only `/opt/omniterm/chrome-sandbox` is installed setuid: owner `root:root`, mode
`4755`. The package archive records this ownership and mode; the builder verifies
it after creating the `.deb`. Other files have no set-id bits. Installation hooks
only update desktop/icon caches and do not override administrator
`dpkg-statoverride` policy. Renderer sandboxing remains enabled; this package does
not add sandbox-disabling flags.

## Install, validate, and remove

On a clean Ubuntu 22.04 desktop, verify the checksum and install through APT:

```sh
sha256sum -c omniterm_0.1.0-1_amd64.sha256
sudo apt install ./omniterm_0.1.0-1_amd64.deb
stat -c '%U:%G %a' /opt/omniterm/chrome-sandbox
# Expected: root:root 4755
```

Start **as a normal desktop user**, from the application menu and `/usr/bin/omniterm`.
Do not launch the GUI with `sudo`. Verify:

- Real local shell input, `pwd`, Unicode, Ctrl+C, resize, and process cleanup.
- The bundled Agent starts; record authenticated task testing separately.
- The installed node-pty module loads under bundled Electron.
- No missing-library or sandbox-configuration failure appears.
- Renderer sandbox remains enabled; no sandbox-disabling command-line flags.
- The launcher icon/menu entry works, and SSH/SFTP/serial clients are present.

For headless smoke tests, use a normal user with an X server/Xvfb and a session
D-Bus. Headless startup is not a substitute for keyboard, input-method, and
window-manager checks. Capture the actual WM_CLASS before adding any
`StartupWMClass` override to the desktop entry.

Close the app, then verify removal and reinstallation:

```sh
sudo apt remove omniterm
# The /opt application, /usr/bin launcher, desktop entry and icon should be removed.
sudo apt purge omniterm
```

User configuration and Agent credentials are deliberately retained in the user's
home directory. Removal hooks do not delete user data or terminate unrelated
processes. Also test an upgrade/reinstall and confirm sandbox ownership, menu
integration, and saved configuration remain correct.
