# Changelog

All notable changes to the Toad desk are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). A tag `desktop-vX.Y.Z` on this
repository is what builds and publishes a release.

## [Unreleased]

### Changed
- The window's top strip is the same on every platform: the mark, the
  title, the open teammate's model and effort, and the search. The rail's
  Team band goes, New Teammate joins Settings at the rail's foot, and the
  mode picker leaves the conversation's band for the inspector, which
  already had it.
- On a Mac the chord key is Cmd, not Ctrl: Cmd+, opens Settings, Cmd+N a
  new teammate, Cmd+F the search, Cmd+1–9 a seat. Help and the menu bar
  say so.
- The Mac menu bar has what every Mac app has: Hide, Hide Others, Show All
  and Services under Toad; New Teammate and Close under File; Minimize,
  Zoom and Bring All to Front under Window; a search field under Help.
- The traffic lights sit on the top strip's centre line, and their gutter
  goes in fullscreen, where they do.
- The dock badge counts the rail's unread teammates, and a teammate that
  blocks while the window is not in focus bounces the dock once.
- The window is shown once the page has loaded, so a dark theme no longer
  opens on a white frame.
- A click into an inactive Toad window lands on what it hit.
- The Mac bundle asks for macOS 13, which the window's CSS needs.
- Mac toasts come from the notification center Apple keeps, not the one it
  retired: they thread by teammate, and a click raises the window and opens
  that teammate. A `make dev` run, being no bundle, posts none.

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
