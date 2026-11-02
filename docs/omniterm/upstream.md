# Upstream provenance

omniTerm begins with the OpenAI Codex source tree at commit
[`f021168bde5357009fceb0477835ad90bd54c17c`](https://github.com/openai/codex/commit/f021168bde5357009fceb0477835ad90bd54c17c).
The original Git history is retained.

The root `LICENSE` remains Apache-2.0 and is unchanged. `NOTICE` retains the
upstream notices and adds omniTerm attribution. Third-party components retain
their individual licenses and notices.

## omniTerm changes (2026-10-30)

- Replaced the product README and added a Chinese README.
- Added the independent `omniterm-cli/` launcher and tests.
- Added the `omniterm-desktop/` Electron/React application with xterm.js,
  node-pty sessions, connection forms, saved profiles, and a Codex TUI pane.
- Added desktop packaging that includes the complete source-built Agent package,
  a native interaction test plan, and separate desktop code CI.
- Renamed root workspace metadata, added launcher commands, and registered the
  dependency-free package in the workspace and lockfile.
- Added `chat` and `anthropic_messages` wire APIs in `codex-api`, and the
  `dynamic_tools_only` feature that limits a thread to client-supplied tools.
- Added architecture and roadmap documents.
- Adapted contribution, installation, security, ownership, and issue reporting
  documentation for omniTerm.
- Archived upstream GitHub workflows and added a portable launcher CI matrix.

The original README is preserved in [upstream-readme.md](upstream-readme.md).
Its relative links reflect its original location; use the
[upstream copy](https://github.com/openai/codex/blob/f021168bde5357009fceb0477835ad90bd54c17c/README.md)
for navigable historical documentation. Its installer and product links refer
to OpenAI Codex, not omniTerm releases.

## Maintaining the Agent foundation

Internal crate names, SDK package identities, environment variables, and wire
protocols remain compatible with the imported source. In particular,
`codex-cli/package.json` still describes inherited Codex packaging machinery;
it is not the omniTerm package and must not be published as an omniTerm release.

Keep the OpenAI repository as the `upstream` Git remote and publish omniTerm work
to `origin`. Review and test upstream changes before merging. Archived release
workflows require adaptation of credentials, infrastructure, artifact names,
and publishing destinations before they can be used for omniTerm.
