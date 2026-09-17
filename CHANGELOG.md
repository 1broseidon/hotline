# Changelog

All notable changes to the Hotline desk are recorded here. Entries before
0.14.0 were written when the app was called Toad and are left as they were.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/). A tag
`desktop-vX.Y.Z` on this repository is what builds and publishes a release.

## [Unreleased]

### Changed

- Toad is now Hotline. The crates are `hotline-core` and `hotline-app`, the app is `Hotline`, its bundle identifier `dev.hotline.app`, its data directory `Hotline` (`~/.local/share/hotline` on Linux), its keychain service `dev.hotline.credentials` and its environment `HOTLINE_*`. The agent-facing names move with it: the built-in agent is `hotline`, the tools are `hotline_*`, the skills `hotline-room` and `hotline-computer`, the marker `.managed-by-hotline`, the shell `hotline-shell`. Computers, their volumes and the image are `hotline-computer`, `hotline-home-*`, `hotline-src-*`, `hotline-nix-glibc` and `ghcr.io/1broseidon/hotline-computer`. Nothing carries over from a Toad install: the room, the keys and a paired phone are all addressed by names that changed, so 0.14.0 starts clean. A 0.13.x room is the same layout under a different directory name, so moving it is a rename — `~/.local/share/toad` to `~/.local/share/hotline` — with the provider keys entered again, because the keychain service moved too.
- The manual-pairing domain string is `hotline-manual-pair-v1` and its vectors are repinned, so a phone still speaking the Toad string cannot pair until it is updated.

### Fixed

- The window's generated contract is whole again. It was written by a filtered test run in 0.13.1 and kept only the pairing types, which left every other type the window imports undeclared.

## [0.13.1] - 2026-09-17

### Added
- A paired phone can reach a teammate's computer through the desk: one
  door on the Remote listener carries the viewer's frames and input
  between the phone and the container, with the desk presenting the
  bearer it holds. The phone never learns the port or the token, and
  revoking the device drops the connection.

## [0.13.0] - 2026-09-16

### Added

- A welcome pane in place of the empty room: connect a provider or pick a harness on this machine, add the first teammate with a suggested goal, then say hello. Each step is read from what the room already knows — its credentials, the harnesses it can start, its roster — so the pane is there exactly while there is no teammate and never needs dismissing. The provider forms are the ones Settings renders.
- A teammate's first conversation opens with a few things to try, fitted to whether their folder is a project or one Toad made for them, and one sentence on chapters. A prompt fills the composer and does not send. The card is gone once anything has been said.
- Settings → Computer offers every published toad-computer release from the desk's floor up as the room's image, Newest by default with its version on the line, and a Check now button that asks the releases list at once instead of on the six-hour clock. Custom image stays for other registries.
- The new-teammate form asks the access choices most people decide at the start: Whole machine, Background work and, where a container runtime is ready, Computer, in the same words as the teammate's pane. They can always be changed later on the pane, which keeps the MCP and skill grants.

### Fixed

- A fresh Toad Agent teammate runs at high effort when its model offers levels, and the strip says so before the first session instead of showing an empty picker. An effort you chose always wins; a switch to a model that does not list it falls back to that model's high.
- A Linux desk launched from the desktop session now recovers the login shell's PATH, as the Mac one did, so a tool source or harness named by a bare command (`ketch mcp serve`, anything from Linuxbrew or a language's own bin) starts instead of failing with "could not be started". The Tools list also says when a stdio source's arguments are stored securely rather than showing the bare command as if the rest had been dropped.

### Changed

- Apple's container runtime is listed under Settings → Computer only in a macOS build. A Linux or Windows desk no longer shows it as unsupported.
- A teammate's own skills are listed on its pane one per row, with the description it gave each.
- Claude Code and Codex teammates start on current adapter packages. The pinned ones were a month old and refused the newer models those accounts now default to.

## [0.12.1] - 2026-09-16

### Added

- A teammate's computer hands over the guide of the release it is actually running, and Toad writes it into the workspace as the `toad-computer` skill. The catalog lists it as the computer's, with that release, so what the agent reads is never a bundled copy that could drift from the container it drives.
- A computer running an older release than the newest says so on the teammate's pane, with an Update button and a note on what recreating the container keeps and what it does not.

### Changed

- A new computer is created on the newest published toad-computer release rather than the one this desk was built against, which is now the floor and the offline fallback. Toad checks for releases when the room opens and every six hours after. A pinned image, the teammate's or the room's, is used as written and never offered an update.
- Teammates work quietly. What an agent says between its tool calls is shown as thinking, not as chat, so an exchange is an acknowledgement, the work, and the result: two or three messages unless the answer genuinely needs more. The house style asks for the same.

## [0.12.0] - 2026-09-15

### Added
- Skills. A skill is a procedure a teammate reads when the task calls for
  it, in the Agent Skills format: a folder holding `SKILL.md` with a name
  and a description, under `.agents/skills/` in the teammate's working
  directory. Settings → Skills is the gateway: add a skill by picking its
  folder, open one to read what it is for, remove it at the foot; a folder
  that is not a skill stays in the list with the reason. Each teammate is
  granted none, selected, or all of the gateway's skills from its pane,
  the way MCP servers are, and the grant is copied into its workspace at
  every start so a Toad Agent and an ACP teammate read the same files.
  Skills the teammate writes for itself are listed under the grant.
- The first built-in skill, `toad-room`, is in every workspace: when to
  close a chapter and what to write in its note, how to shape a schedule,
  when to ask a colleague or the person, and the habit of saving a task it
  will be asked for again as a skill. The teammate's preamble names its
  skills and where each is; the body is read when the task calls for it.
- A teammate's computer keeps its home on a named volume,
  `toad-home-<persona id>`, so the environments it prepared, its jobs and
  their output, its shell history and its browser profile survive the
  container the hibernate cycle removes.

### Changed
- New computers run Computer 0.5.0: a desktop bar with menus, an observer
  that names each job, a terminal for the person, a viewer that shows the
  teammate's pointer and pastes from the person's own computer, and
  generic Nix environments prepared from package names or a flake.
- A computer's clock shows the host's time zone.

## [0.11.1] - 2026-09-14

### Changed
- A desk introduces itself to a phone by the machine's name instead of
  "Toad desktop", so two desks on one phone can be told apart.

## [0.11.0] - 2026-09-14

### Added
- On macOS, a teammate on workspace reach now has a confined shell too.
  The shell runs under a default-deny Seatbelt policy with a private home,
  scratch and caches, no host environment, and the installed toolchains
  read-only; it is only offered once a probe has shown the policy is
  really enforced. Network stays on, as on Linux.
- A failed turn shows a short card saying what went wrong, with the
  provider's own details a click away instead of a wall of JSON.
- `docs/security.md` is the standing method for any change to what a
  teammate can reach, and the index of the tests that prove each boundary.

### Changed
- A model request that fails for a passing reason (a dropped connection,
  throttling, a provider outage) is retried with backoff, and Stop still
  works while it waits. What the turn had already done is kept: completed
  tool calls and their results stay, and a message you sent while it was
  failing is not lost. A request the provider refuses, or a switch of
  model or provider mid-conversation, starts a fresh continuation from
  the facts instead of replaying history the new model cannot read.
- A chapter closes before the model's context limit, including between
  tool rounds, so a long turn no longer dies on context length.
- A teammate on an external harness whose child died or forgot its
  session gets a fresh one on your next message, after the old child is
  shut down, with a fresh briefing.
- Photos attached to a message are checked and resized off the runtime
  thread to at most 2000 px and 1 MiB each, within one budget for the
  whole request. A photo that cannot be read, including HEIC, goes to the
  model as a path with a note saying why.

### Fixed
- On Linux, a Rust toolchain installed through rustup works in the
  confined shell again: its metadata is written to the private home
  instead of the read-only host installation.
- A background runner that vanishes is reported as an unknown outcome,
  not as still running, and its result is written down once.

## [0.10.2] - 2026-09-14

### Changed
- A message appears the moment you send it. The core writes the line only
  once a session is up, and starting one is a second or two in which the
  composer has already emptied and nothing has appeared. The window draws
  the line itself until the core's own replaces it.

### Fixed
- A refused message no longer disappears. Its words and files go back to
  the composer, the reply it answered is restored, and the band says why.

## [0.10.1] - 2026-09-14

### Fixed
- A phone can choose a model and effort for a teammate whose session is
  resting. A resting session reports neither, so the phone now reads what
  this desk can reach the way the window does.

## [0.10.0] - 2026-09-14

### Added
- A phone can answer what a teammate is waiting on. Both kinds of card —
  a permission request and a `request_human` — are answered from the
  phone, with the options the desk worded and a place to type whatever
  goes back to the teammate. Which model answers and how hard it thinks
  can be set from the phone too, and the choice is kept on the teammate's
  record as it is at the desk.
- The agent can react to your last message with one emoji instead of
  replying, when a reaction says everything a reply would.

### Unchanged on purpose
- What a teammate is allowed to do is still decided at the desk: how far
  it reaches, which tools and servers it has, whether it keeps a
  computer, and a harness mode, which for some harnesses is that standing
  posture under another name.

### Fixed
- A reply that is only a reaction no longer ends with "Turn failed: The
  model stream ended without a complete response." The model's empty
  completion after a tool round is the turn finishing. When that round's
  step failed and the agent said nothing, one quiet line says so.

## [0.9.0] - 2026-09-12

### Added
- Notifications on the phone: the phone registers where to notify it when
  it connects, and the desk sends a glance when a reply lands or a card
  needs you, through Expo's push service. A tap opens that conversation.
  The phone stays quiet for the conversation it is already showing.
- Bubbles land to a beat, on the desk and the phone: a reply being written
  shows each bubble once it is whole, one per reading beat, and the mark
  above the composer stays up until the last has landed. What was written
  before you were looking lands at once.
- Your messages carry a receipt on the desk and the phone: one tick once
  a message is on the tape, two once the agent has produced anything with
  it in context. A message queued behind a turn, or cancelled, stays at
  one tick. Steering a running turn earns the second tick the moment the
  agent picks it up.
- The agent can react to your last message with one emoji, in place of a
  reply, when a reaction says everything a reply would.
- The rule that cuts a reply into bubbles, and the beat they land to, live
  in one file the phone copies with its contract.

### Removed
- The "Your update is now in the agent's context" notice after a steer;
  the second tick says it.

## [0.8.1] - 2026-09-12

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
