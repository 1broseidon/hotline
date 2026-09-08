# Changelog

All notable changes to the Toad desk are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). A tag `desktop-vX.Y.Z` on this
repository is what builds and publishes a release.

## [0.5.0] - 2026-09-08

The first release of the Rust desk, replacing the Electrobun edition on
this repository. The previous Toad shipped up to desktop-v0.4.1 and is
archived privately; the version line continues from it, so an installed
desk always sees the Rust one as newer.

### Added
- A room: a named roster of teammates, each with its own project directory,
  conversation, and tape, kept in a local data directory the desk owns.
- The built-in agent on a model key the person holds, and any harness
  over the Agent Client Protocol (Claude Code, cursor-agent, opencode) in
  its own lane.
- MCP servers from settings, granted per teammate; OAuth and pasted tokens
  live in the vault and ride one proxy.
- Workspace tools that obey a teammate's reach; revoked capabilities stay
  revoked.
- A computer per teammate: the toad.computer image, with the viewer served
  on the machine's one port and memory, process, and mount limits of the
  teammate's own.
- Chapters, a search index over every tape, permission and human-action
  cards, and teammate-to-teammate requests behind the person's consent.
- The Tauri 2 shell: tray, notifications, window state, and a signed and
  notarized macOS build from GitHub Actions.
