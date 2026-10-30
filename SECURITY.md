# Security policy

<!-- Adapted for omniTerm from the upstream Codex security policy. -->

omniTerm is an early-stage project. Security fixes currently target the latest
`main` branch; there are no supported stable release lines yet.

## Reporting a vulnerability

Report vulnerabilities using
[GitHub private vulnerability reporting](https://github.com/YanYaoyuan/omniTerm/security/advisories/new).
Include affected versions or commits, reproduction steps, impact, and any
suggested mitigations. Remove secrets and personal data from attachments.
Please do not disclose exploit details in a public issue before maintainers
have had an opportunity to investigate and coordinate a fix.

This is the security contact for omniTerm. The inherited Codex source does not
make omniTerm part of OpenAI's security or bug bounty programs. Maintainers can
coordinate upstream disclosure when a report affects unmodified Codex code.

## Runtime boundaries

The launcher starts local tools with the current user's permissions. Shell,
SSH, SFTP, and serial sessions are not automatically inside the Codex sandbox.
Codex Agent actions retain the selected Codex approvals and sandbox settings.
Review the target host, working directory, and requested action before granting
an Agent access to a local or remote environment.

Keep SSH host-key verification enabled, use narrowly scoped credentials, and
do not include private keys, tokens, passwords, or production session logs in
issues or pull requests.
