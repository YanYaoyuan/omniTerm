# Install the omniTerm agent Skill

The [v0.2.0 Native Preview release](https://github.com/YanYaoyuan/omniTerm/releases/tag/v0.2.0) publishes `omniterm-control-skill-0.2.0.zip`, its SHA-256 checksum and a source/file-hash manifest. The same complete Skill is bundled in every native package under `omniterm-cli/skills/omniterm-control/`. The standalone Skill does not contain the application or CLI.

Install the native application, Node.js 22+ and the coding agent on the same operating system. Open omniTerm and connect the asset terminal or SFTP page. Older applications without external CLI support must be upgraded and restarted; no discovery-directory setting can add that capability.

Verify the ZIP against its `.sha256` sidecar, then extract it. Copy the complete `omniterm-control/` directory, preserving `SKILL.md`, `scripts/` and `references/`:

| Agent | User skill directory |
| --- | --- |
| Codex | `~/.codex/skills/omniterm-control/`, or `$CODEX_HOME/skills/omniterm-control/` if customized |
| Claude Code | `~/.claude/skills/omniterm-control/` |

Restart the coding agent to discover the Skill. Ask it to use `omniterm-control` for the intended connected asset. To check the installation first:

```sh
node <skill-directory>/scripts/omniterm.mjs --locate
node <skill-directory>/scripts/omniterm.mjs session list
node <skill-directory>/scripts/omniterm.mjs files list
```

The locator prints the selected CLI entry. If it cannot find a unique supported CLI, set `OMNITERM_CLI_ENTRY` to the absolute installed `omniterm-cli/bin/omniterm.js` path. On macOS it is inside `omniTerm.app/Contents/Resources/`; Linux Debian installs it under `/usr/lib/omniterm-native/`; portable Windows/Linux packages keep it beside the executable. The agent and app must run as the same user. WSL, containers and remote coding hosts do not automatically share the desktop's files or loopback endpoint.

The agent confirms the asset and binds its instance/session generation before operating. It checks completion and remote exit codes, and queries original request IDs after timeouts rather than resubmitting unknown operations. Read the [CLI guide](../README.md) for the command reference; [SKILL.md](omniterm-control/SKILL.md) is the concise agent entry.
