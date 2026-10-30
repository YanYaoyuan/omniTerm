# Contributing to omniTerm

<!-- Adapted for omniTerm from the upstream Codex contribution guide. -->

We welcome bug reports, documentation, design proposals, and pull requests at
[YanYaoyuan/omniTerm](https://github.com/YanYaoyuan/omniTerm).
For larger changes, open an issue first so we can agree on the user problem,
scope, and cross-platform behavior.

## Development

The desktop app is the main product and lives in `omniterm-desktop/`.
Follow the [desktop guide](../omniterm-desktop/README.md) to install dependencies
and rebuild node-pty for Electron before starting the app. It uses its own npm
lockfile, separately from the inherited Codex pnpm workspace.

The optional launcher lives in `omniterm-cli/` and has no npm runtime
dependencies. From the repository root, run the relevant checks:

```sh
node omniterm-cli/bin/omniterm.js --help
node --test omniterm-cli/test/cli.test.js
npm --prefix omniterm-desktop test
npm --prefix omniterm-desktop run build
```

Read the [architecture](omniterm/architecture.md) and
[roadmap](omniterm/roadmap.md) before adding a new integration. Be explicit about
which features are implemented and which remain proposals. Changes should
account for macOS, Linux, and Windows; identify any platform limitations in the
pull request and relevant documentation.

Desktop backend tests use mocked PTYs. Follow the
[interaction checklist](omniterm/desktop-test-plan.md) for changes to keyboard
handling, terminal layout, connections, or process lifecycle, and record which
platforms and real services/devices were tested.

The inherited Rust Agent engine remains under `codex-rs/`. Keep changes to that
engine focused so upstream improvements remain practical to merge. For Rust
changes, follow the [local package build guide](omniterm/local-build.md), run
formatting and the affected crate's tests, and report the results. Desktop and
launcher CI do not replace Rust-core validation.

## Pull requests

- Describe the concrete problem, resulting behavior, and any user-visible
  limitations.
- Include meaningful tests for changed behavior and the commands you ran.
- Keep credentials, personal configuration, and generated build outputs out of
  commits.
- Preserve upstream copyright notices and third-party license information.
- Treat contributors respectfully and keep feedback focused on the work.

## License

Unless explicitly stated otherwise, contributions are provided under the
repository's [Apache-2.0 license](../LICENSE). Submit only work you have the
right to contribute. omniTerm does not use the upstream OpenAI CLA workflow;
no separate CLA is currently required by this project.

## Security

Follow the [security policy](../SECURITY.md) for vulnerability reports instead
of posting sensitive information in a public issue.
