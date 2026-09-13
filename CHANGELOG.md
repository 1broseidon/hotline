# Changelog

All notable changes to the Toad desk are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). A tag `desktop-vX.Y.Z` on this
repository is what builds and publishes a release.

## [Unreleased]

### Fixed
- A Windows desk can update again. Checking for updates refused every
  release with "This release does not include a matching update package"
  because the desk verified the Windows package against the Mac package
  name; each installer now verifies the name its own release ships.

## [0.8.0] - 2026-09-12

### Added
- Remote access for the mobile app: Settings → Remote runs an opt-in TLS
  listener, separate from the desk's loopback door, and a two-minute QR
  invitation pairs a phone. The phone reads the roster and a bounded recent
  window of a conversation, sends messages and attachments, replies to a
  message with a quote, and cancels a response.
- Manual pairing for a phone that cannot scan: the desktop's address and a
  six-digit code, used as the password of a PAKE whose confirmation is bound
  to the certificate the phone actually connected to. A fresh install
  listens on port 8788.
- A phone sees a teammate's computer: frames arrive as small JPEGs, and the
  phone can ask the computer's status and stop it. Removing a computer stays
  a desktop action.
- Attached images reach the model as pixels beside the file path, on every
  provider; large photos are shrunk to a 2000-pixel JPEG first.

### Changed
- The built-in Toad Agent's stored backend id is `toad`; it was `pi`.
  Existing data directories are rewritten once when the desk opens, and
  imports from the previous Toad still translate.

### Fixed
- A reply item whose id the provider refuses on replay no longer breaks
  every later request of a session.

## [0.7.0] - 2026-09-10

### Added
- Mid-turn steering for Toad Agent: send corrections, change direction or ask
  a question while work is running in the same conversation. The conversation
  shows when the agent receives the update. Steering works through Toad's Rig
  loop across providers.
- Managed shell jobs that keep the conversation responsive during long
  commands. The agent can inspect progress, wait for results or cancel work
  when the operator changes direction. Stop also terminates owned shell
  processes and their descendants.

### Fixed
- macOS launches recover the user's shell PATH so ACP harnesses and Docker
  helpers can be found when Toad opens from Finder or the Dock.

## [0.6.0] - 2026-09-09

### Added
- Ollama local and cloud connections, OpenRouter sign-in, Z.ai API keys and
  coding-plan connections, and Grok subscription sign-in. Model requests
  continue to use Rig's provider clients.
- Named OpenAI-compatible connections with a custom URL, optional key,
  Responses or Chat Completions, model discovery and manual model IDs.
- Provider model refresh and manual additions. Provider discovery supplies
  available models; the bundled models.dev catalogue supplies metadata.
  Refresh preserves manual entries and the user's model selections.
- An updater in Settings that checks GitHub every six hours, shows release
  notes, and installs a signed package for the current OS and architecture.
  Installation waits for an idle room and preserves local data.
- A Windows x86_64 installer and updater support. Windows publisher signing
  is deferred, so Windows may still show an installation reputation warning.

### Security
- Toad-owned API keys, OpenRouter and Grok sign-ins, and MCP credentials use
  macOS Keychain, Windows Credential Manager or Linux Secret Service. Existing
  credentials migrate with verified writes and no plaintext fallback on failure.
  ChatGPT and Copilot OAuth tokens remain in permission-restricted files
  managed by Rig.
- MCP command arguments and environment values move out of saved room settings
  into native credential storage. Settings responses hide unmigrated values;
  successful migration removes their superseded room history. Legacy HTTP
  URLs with credentials, queries or fragments must be re-entered with secrets
  in the authentication fields.
- Windows vault files receive private ACLs, and shell, ACP and MCP processes
  run in owned jobs so stopping a session also terminates their descendants.

To upgrade from 0.5.0, download and install this release manually; the in-app
updater is new in 0.6.0. Linux credential storage needs an unlocked Secret
Service session. Credential references do not transfer secrets to another
machine: reconnect providers and tool sources after moving the data directory.

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
- Settings say less. A runtime that cannot host a computer is "Not
  installed" or "Not running", with what it said and what to do behind an
  info key; a refusal anywhere is one sentence with its details folded
  away; the hints that repeated their titles are gone, and the section list
  is titles alone. The image field shows the pinned image it stands for.

### Fixed
- Apple container reads as ready when it is: the probe asks it `ls`, since
  its CLI has no `version` and called the missing one a missing plugin.
- The strip's model, effort and search stay at the right edge of a narrow
  window instead of sliding into the title's empty column.
- An agent that closes its pipe at the handshake is refused with what it
  said on stderr, not only "transport closed".

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
