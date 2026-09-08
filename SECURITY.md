# Security Policy

## Supported versions

| Version | Supported |
| --- | --- |
| `0.1.x` | Yes |
| Earlier versions | No |

## Reporting a vulnerability

Do **not** report vulnerabilities through a public issue.

Use GitHub's **Report a vulnerability** button in the repository's Security
tab to create a private security advisory. Include:

- Coconut version and operating system;
- a concise reproduction path;
- the impact you observed or expect;
- logs or configuration only after removing personal paths, tokens and secrets.

If private reporting is not enabled for the repository yet, ask a maintainer to
enable it without publishing exploit details. A public issue is appropriate only
after a fix is available and the maintainer agrees that disclosure is safe.

## Scope

Reports are especially useful for issues involving privileged Linux input
integration, keyboard event handling, process launching, package installers,
GitHub Actions or accidental exposure of user data.

