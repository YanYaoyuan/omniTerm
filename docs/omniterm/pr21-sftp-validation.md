# PR #21 independent file connection validation

Date: 2026-11-09. Platform: macOS arm64. Base: `8ba899af0b` on
`fix/bastion-ssh-macos`.

The native file panel now accepts an AccessClient WinSCP export bound to an SSH
asset. The interface retains the existing file browser and transfer controls;
the dedicated backend opens an independent SFTP connection for that binding.
Unbound SSH/SFTP connections retain their existing transport behavior.

## Completed locally

- Rust 1.95: `cargo test --locked --offline`: **120 passed, 9 ignored**.
- Rust 1.95: `cargo clippy --locked --offline --all-targets -- -D warnings`: passed.
- Rust 1.95: debug build and `git diff --check`: passed.
- macOS preview bundle: ad-hoc signing and strict signature verification passed.
- The built executable and the executable inside the signed application bundle
  both completed `--verify-file-connection <private-export.json>` against the
  authorized QiZhi file endpoint. Both returned success, directory `/root`, and
  33 entries including hidden items. This uses the same authentication, SFTP
  client, connection manager and directory listing path as the native file panel.
- The real endpoint check performed authentication and directory reads only. It
  did not write or transfer files, start metrics, execute a remote shell, or use
  the existing terminal's control socket. No additional OTP was requested by
  the file endpoint in these checks.
- Source inspection found no captured credential, private endpoint, account or
  asset-title values in the changes prepared for submission.

The automated regression coverage includes independent pane bindings and initial
directories, password/OTP separation, credential cleanup, authentication longer
than the ordinary protocol timeout, cancellation, stale connection results,
explicit retries, ordinary shared-channel arguments and lifecycle, and terminal
master ownership after replacing or closing only a file connection. It also
checks source-file identity changes and another client editing the same saved
SSH profile ID; neither may silently attach the old asset's files to a new target.

## Remaining manual/platform checks

- Interactive GUI acceptance is pending: the local computer-use bridge could not
  start, including after resetting it. Import-dialog clicks, rendered file rows,
  interactive tab switching and GUI reconnect are not claimed as verified.
- Automated real-server upload/download, overwrite/delete, native drag/drop and
  expired launch credentials were not exercised. Use a disposable controlled server for
  write operations; do not use the read-only endpoint verification as permission
  to mutate production files.
- The nine ignored integration tests require their dedicated fixture/platform
  setup; they were not run by the ordinary macOS unit-test invocation.
- Windows and Linux runtime/GUI verification remain for their respective systems
  and CI. A local macOS build does not establish cross-platform GUI compatibility.

Captured parameter exports and application test data are local only and are not
included in the repository. The separate preview data directory leaves the
running application's sessions and saved workspace unchanged.

## macOS Finder drop follow-up

The user confirmed downloads work in the preview, then reported that dropping a
local file onto the **left file list** inserted its path into SSH instead. The
cached winit 0.30.13 macOS backend delivers `HoveredFile` / `DroppedFile` paths
without updating the normal pointer position. Both native drop handlers had
used that stale pointer position.

The fix samples the current AppKit position in the application's content view,
including Retina scaling and UI zoom. A shared per-frame payload is consumed
once by the visible target. The complete file-panel area accepts uploads to its
current directory and displays an upload preview. Direct terminal drops retain
quoted-path insertion. Missing native coordinates never fall back to a previous
terminal position. Ordinary shared SFTP and QiZhi file channels use the same UI
routing; their authentication and transfer implementations are unchanged.

Automated regressions cover a stale SSH pointer with a native drop over files,
single-consumer routing, direct terminal drops, clipping, missing coordinates,
blocking dialogs, and coordinate conversion. The follow-up Rust 1.95 run passed
**128 tests (9 ignored)** and `clippy --all-targets -- -D warnings`. The original
Finder gesture still
needs manual confirmation on the rebuilt application: the local computer-use
bridge remains unavailable. User-reported download success is not an automated
upload verification.

## Reconnection authentication follow-up

After restarting the preview, the user reported that SSH logged in while the
file connection requested another password and did not open. A single read-only
comparison with macOS `/usr/bin/sftp` and the same exported lowercase `pwd`
also failed during authentication (exit 255). TCP, SSH negotiation and the
existing trusted host key succeeded; no SFTP directory operation completed.
The export's `receivedAt` was 17:07:35 Asia/Shanghai on 2026-11-09, and the file
had last changed at 17:08:44. It had previously authenticated successfully.

This does not establish a specific credential lifetime or distinguish expiry,
revocation, and additional authentication requirements. The comparison helper
supplied the export only once and declined further interactive challenges. A
fresh export is required for the next comparison; the old credential is not
being retried automatically. The Finder-drop commit did not change transport
or authentication code.

Repeated account-password prompts now explain that the exported credential was
already attempted and offer updating the export as the recovery path. Explicit
OTP prompts and manual additional authentication remain available. Authentication
errors in the file panel explain the distinction from the saved terminal login.
This improves diagnosis; it is **not evidence that the live connection has been
restored**. The application currently reloads a user-provided export on explicit
reconnect; it does not obtain new launch credentials from the bastion portal.
