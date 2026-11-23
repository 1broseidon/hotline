# Security

## Reporting a vulnerability

Please report it privately, not in a public issue or discussion. On this
repository's **Security** tab, press **Report a vulnerability**, and say what
you found, how to reproduce it, and what version you ran. Only the
maintainers can see the report.

We'll reply there, work out a fix with you, and credit you when it ships
unless you'd rather not be named.

## In scope

- The desktop app, on macOS, Linux and Windows
- The server edition (`hotline serve`, `hotline update`)
- The computer image (`ghcr.io/1broseidon/hotline-computer`, from
  [hotline-computer](https://github.com/1broseidon/hotline-computer)); a
  report for it is welcome here
- The install scripts at `https://hotline.dev/install` and
  `https://hotline.dev/install.ps1`

Fixes land in the latest release.

## Known limits

These are choices, described in [docs/security.md](docs/security.md#accepted-risks-and-known-limits),
not vulnerabilities on their own:

- The network is on for every teammate. Hotline's wall is around files.
- Claude Code, Codex, Cursor and other harnesses run with their own
  permissions, outside Hotline's sandbox, and so does a teammate given the
  whole machine.
- Windows has no confined shell.

A way around one of these that Hotline claims to stop is in scope.
