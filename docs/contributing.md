# Contributing to omniTerm

<!-- Adapted for omniTerm from the upstream Codex contribution guide. -->

We welcome bug reports, documentation, design proposals, and pull requests at
[YanYaoyuan/omniTerm](https://github.com/YanYaoyuan/omniTerm).
For larger changes, open an issue first so we can agree on the user problem,
scope, and cross-platform behavior.

## Development

The current omniTerm command launcher lives in `omniterm-cli/`. It requires
Node.js 22 or newer and has no npm runtime dependencies. From the repository
root, run:

```sh
node omniterm-cli/bin/omniterm.js --help
node --test omniterm-cli/test/cli.test.js
```

Read the [architecture](omniterm/architecture.md) and
[roadmap](omniterm/roadmap.md) before adding a new integration. Be explicit about
which features are implemented and which remain proposals. Changes should
account for macOS, Linux, and Windows; identify any platform limitations in the
pull request and relevant documentation.

The inherited Rust Agent engine remains under `codex-rs/`. Keep changes to that
engine focused so upstream improvements remain practical to merge. For Rust
changes, follow the build instructions in [install.md](install.md), run
formatting and the affected crate's tests, and report the results. The current
omniTerm launcher CI does not replace Rust-core validation.

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
