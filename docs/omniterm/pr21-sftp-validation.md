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
- Real-server upload/download, overwrite/delete, native drag/drop and expired
  launch credentials were not exercised. Use a disposable controlled server for
  write operations; do not use the read-only endpoint verification as permission
  to mutate production files.
- The nine ignored integration tests require their dedicated fixture/platform
  setup; they were not run by the ordinary macOS unit-test invocation.
- Windows and Linux runtime/GUI verification remain for their respective systems
  and CI. A local macOS build does not establish cross-platform GUI compatibility.

Captured parameter exports and application test data are local only and are not
included in the repository. The separate preview data directory leaves the
running application's sessions and saved workspace unchanged.
