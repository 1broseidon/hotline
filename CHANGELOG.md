# Changelog

All notable changes to the Hotline desk are recorded here. Entries before
0.14.0 were written when the app was called Toad and are left as they were.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/). A tag
`desktop-vX.Y.Z` on this repository is what builds and publishes a release.

## [Unreleased]

### Fixed

- On Linux, installing a `.deb` or `.rpm` update no longer hangs after you
  type your password. With Homebrew's polkit installed, Hotline ran a copy of
  `pkexec` that cannot ask for admin rights. It then fell back to `sudo`,
  and one failed try left a `sudo` waiting on a terminal nobody could see.
  Every later update queued behind it. Hotline now uses the system's own
  password prompt directly, stops waiting after ten minutes, and says why
  an install did not go through.
- A teammate's model menu shows a provider change straight away. Refreshing
  a provider's list, adding or removing a model by hand, connecting a
  provider, or changing "Models shown" used to reach a running teammate only
  after it restarted. The menu now updates without a restart, for idle
  teammates too.

## [0.17.6] - 2026-09-22

### Changed

- The bundled model list is refreshed from models.dev. xAI gains Grok 4.7.
  ChatGPT now offers what a ChatGPT sign-in serves today: GPT-6 Sol and
  GPT-6 Luna join GPT-6 Astra, and GPT-5.6 and GPT-5.4 leave the list. A
  teammate already set to one of the two that left keeps it.

## [0.17.5] - 2026-09-22

### Fixed

- A teammate whose computer cannot start answers anyway. With Docker not
  running, a teammate with its computer turned on failed to start, and the
  next message was met with "That teammate is not running". It now starts
  without the computer and is not told it has one. Its tape says why the
  computer did not start and how to bring it back: fix the cause, choose
  Stop the session, and the next message starts it again with the computer.
- A teammate that cannot be started says why in its conversation, instead
  of the reason being dropped and the next message being refused.

### Changed

- A teammate no longer waits for its computer's image to download before
  it answers. It starts straight away without the computer while the bar
  fills, and knows its computer is on the way: a new `computer_status` tool
  says how far the download has got and can wait for it. Once the image is
  down, the computer joins between turns, never in the middle of one, and
  the conversation carries on.

## [0.17.4] - 2026-09-22

### Fixed

- A GitHub Copilot sign-in from before 0.17.3 no longer needs Refresh
  before Grok and the newer OpenAI models answer. The first Copilot turn
  on such a sign-in reads the account's model list itself and records
  which endpoint each model uses. If Copilot cannot be reached, the turn
  goes ahead on the old route and the list is tried again a few minutes
  later.
- Tools given to a Copilot model on the Responses route are sent as strict
  schemas, as Copilot's Responses endpoint expects, so a Grok or gpt-5.x
  teammate's file reads and commands arrive well-formed.

## [0.17.3] - 2026-09-22

### Fixed

- GitHub Copilot's Grok and newer OpenAI models answer again. Copilot
  serves those only on its Responses endpoint and refused every request
  with "not accessible via the /chat/completions endpoint", while Rig sent
  everything but the Codex family there. Discovery now records which
  endpoints the account's list offers for each model, beside the login,
  and a model offered `/responses` alone is driven there, signed as
  Copilot expects and with the effort in the Responses shape. Sign in
  again or press Refresh under Settings → Providers once, so the list is
  read with its endpoints; a login made before this release has no
  endpoints on file and stays on the old route until then.

## [0.17.2] - 2026-09-21

### Added

- A computer image being pulled is a line on the teammate's tape that
  fills in, not a notice and then a minute of silence. The runtime names
  each layer as it starts and finishes it, and the line counts them on a
  bar, rewriting itself in place; done, it says what came down and how long
  it took, and a failed pull says so where it stood. Docker and Podman are
  counted; Apple's runtime shows the bar without a count.
- The teammate pane's Desktop fold has Check now under Updates, so a newer
  computer release can be asked for at once instead of on the desk's own
  schedule. The offer to update follows straight away when there is one.

## [0.17.1] - 2026-09-21

### Fixed

- A Claude Code teammate sees its Hotline tools and its computer again.
  Hotline's loopback server and the computer's server both listed their
  tools without the cache hints MCP 2026-07-28 requires, and Claude Code,
  which negotiates that version and validates the reply, refused the whole
  listing and retried until it gave up: `request_human`, `list_teammates`,
  `schedule` and every `computer__` tool were absent while stdio servers
  such as ketch and recoil worked. The listing now says it may not be cached
  and belongs to one teammate. The computer side ships in Computer 0.9.1,
  which the desk now pins.

## [0.17.0] - 2026-09-21

### Added

- A passkey is made only once you approve the site's request. Under an
  arming, the site's call to make a passkey waits in the teammate's
  browser; a card on the teammate's tape says which site asks, from which
  origin and for which account; Approve lets the browser make it and the
  room stores and ticks it as before, Deny ends the arming, and a request
  that goes away with its page expires the card. A paired phone is told,
  and may answer the card too. Needs Computer 0.9.0; a 0.8.x computer still
  makes the passkey under the arming without asking.
- Your own skills, without importing them. Settings → Skills lists what is
  in `~/.agents/skills`, the standard folder other agents read too, or a
  folder you point it at; a switch on a row offers that skill to teammates,
  which are then granted it in their panes like a gateway skill and read it
  fresh from your folder at every start. Nothing is copied into Hotline, and
  an entry that is not a skill says why. The gateway stays for a folder from
  anywhere else.

### Changed

- Adding a passkey moved from Settings → Secrets to the teammate's pane,
  under its computer's Browser fold, where the arming belongs; Settings →
  Secrets lists what is stored.
- A teammate's computer section is three folds: Desktop (its state, Stop,
  Remove and Update, then image, memory, processes and folders), Browser
  (the cookies brought over and the passkeys made for it) and Secrets (the
  ticks).
- New computers are created on Hotline Computer 0.9.0.

## [0.16.1] - 2026-09-21

### Added

- A teammate's pane lists what **Bring over browser cookies** brought over: each browser and profile, when, and the sites with their counts. Every site has a remove, and every browser a **Remove all**; the computer's browser drops those cookies at once, whether it is running or is started to do it, and the list follows.

### Changed

- The secrets on a teammate's computer are a plain list now, titled **Secrets**: what is stored, with a tick on what this computer may use. The paragraph above it is gone; an info icon says where secrets are added.
- Bringing over cookies leaves expired ones behind. The browser would drop them on its next look and a site would refuse them, so they are neither counted in the picker nor handed to the computer. Firefox keeps such rows until its own sweep, so a profile not opened in a while showed far more sites than it was signed in to.
- New computers are created on Hotline Computer 0.8.1 or newer, the first release that takes cookies back. An existing computer keeps the release it runs until it is updated or removed; on an older one, removing says so and points at the teammate's Update.

### Fixed

- A passkey made for a teammate was stored only if Settings → Secrets happened to be open at that moment. It never is: the passkey is added from the teammate's screen, or by the teammate, so the credential ended up in the site and the teammate's browser but in no list on the desk, and the next arming threw it out of the browser. The room now watches the arming itself, stores the passkey the moment it is made, ticks it for the teammate, and says so on the teammate's tape; Settings → Secrets, opened later, is told once. The ticks on a teammate also re-read the stored list when a tick lands from elsewhere, so a fresh passkey does not show as "not stored".
- Bringing over cookies from any Chrome profile but the first, or from Firefox, failed with "The computer could not load the cookies" and a note about the name: the computer's saved login was named after the profile's directory, which is `Profile 1` on Chrome and `k3j2x9.default-release` on Firefox, and the computer takes letters, digits and dashes only in a login's name. The name is made of those now.

## [0.16.0] - 2026-09-20

### Added

- Settings → Secrets now keeps three kinds of secret. **Store a login** takes the sites it is for, a username, a password and, when the site asks for six digits, the code seed; a teammate's computer types it into a sign-in form only on a page of those sites, by name (`NAME.username`, `NAME.password`, `NAME.code`), and refuses anywhere else. **Add a passkey for a teammate** names a site and a teammate and arms that teammate's computer for ten minutes: open its screen, sign in to the site as the teammate should be, and add a passkey in the site's security settings, or ask the teammate to; the computer's browser makes the passkey, the desk stores it the moment it is made and ticks it for that teammate, and from then on its browser signs in with it by itself. Nothing is made outside that arming, and a teammate cannot give itself one. Any of three places takes a passkey back: untick it on the teammate, remove it under Secrets, or delete it in the site's security settings. Variables are as before. Every value is still written once, never shown again, and redacted from what the computer's tools answer.
- The stored list and the ticks on a teammate say what each secret is for: a variable, a login for which sites, a passkey for which site. The teammate is told the same by name, and told how a login is typed for it.

### Changed

- New computers are created on Hotline Computer 0.8.0 or newer, the first release that types a login and makes a passkey. An existing computer keeps the release it runs until it is updated or removed; on an older one, storing a login or arming a passkey says so and points at the teammate's Update.

### Fixed

- Remote turns on again in a room moved over from Toad. That edition kept the
  listener's certificate in its own credential store, which this app cannot
  read, and the switch answered "stored by an earlier edition of the app".
  The certificate is now made afresh, as it is when there is none, and the
  phones that pinned the old one pair again; a locked keychain is still
  reported rather than replaced.

## [0.15.0] - 2026-09-20

### Added

- Settings → Secrets keeps keys and tokens in this machine's keychain, each under the name that becomes an environment variable, and a teammate's computer uses them without ever seeing one. A value is written once and never shown again, here or to a teammate. On the teammate's pane, under its computer, the stored names are one tick each, nothing ticked until you tick it; a ticked secret is in the environment of every job that computer runs, the teammate is told the names, and the computer replaces every value with `[redacted NAME]` in what its tools answer. That keeps a value out of the model's context when a command prints it; it is not a wall against a command written to get one out, and the security notes say so. A stored value that is replaced or removed reaches a running computer at once. Needs Hotline Computer 0.7.0: on an older release the teammate's tape says its secrets are not in its shell and to update the computer from its pane.
- Under a teammate's computer, **Bring over browser cookies** picks a browser on this machine, a profile, and the exact sites, and hands those cookies to the computer so its browser starts signed in to them. The browser decrypts its own jar, so nothing is reimplemented on the desk; the window only ever sees site names and counts, and the agent has no way to start any of this.

### Changed

- New computers are created on Hotline Computer 0.7.0 or newer, the first release that takes secrets and reads the desk's bearer as a header. An existing computer keeps the release it runs until it is updated or removed.

## [0.14.2] - 2026-09-19

### Fixed

- A room moved over from Toad has its teammates on the built-in agent again. 0.14.0 renamed the stored agent id at open, but the pass looked for the name before Toad's rather than Toad's own, so on a moved room it changed nothing and recorded itself as done; every teammate then stood on an agent this machine did not know. The pass now covers both earlier names, runs once more on a room the first one passed over, and rewrites the chapter headers in the transcripts as well as the roster.

## [0.14.1] - 2026-09-18

### Fixed

- A credential record written by an earlier edition is refused rather than read as though it were the secret itself. 0.14.0 treated a reference it did not recognise as plaintext to be migrated, so it stored the pointer under the new name and overwrote the file that said where the real chunks were, while the desk went on listing the provider as signed in. Such a record now says it cannot be read here and leaves itself intact; signing in again replaces it.
- The conversation band no longer carries the reason the previous chapter cannot be reopened. That is a standing fact about a chapter rather than something that just happened, and it is already on the greyed menu item; on the band it was a sentence that never went away, which after the rename is what every chapter recorded under the old agent name produced.

## [0.14.0] - 2026-09-17

### Changed

- Toad is now Hotline. The crates are `hotline-core` and `hotline-app`, the app is `Hotline`, its bundle identifier `dev.hotline.app`, its data directory `Hotline` (`~/.local/share/hotline` on Linux), its keychain service `dev.hotline.credentials` and its environment `HOTLINE_*`. The agent-facing names move with it: the built-in agent is `hotline`, the tools are `hotline_*`, the skills `hotline-room` and `hotline-computer`, the marker `.managed-by-hotline`, the shell `hotline-shell`. Computers, their volumes and the image are `hotline-computer`, `hotline-home-*`, `hotline-src-*`, `hotline-nix-glibc` and `ghcr.io/1broseidon/hotline-computer`. Nothing carries over from a Toad install: the room, the keys and a paired phone are all addressed by names that changed, so 0.14.0 starts clean. A 0.13.x room is the same layout under a different directory name, so moving it is a rename — `~/.local/share/toad` to `~/.local/share/hotline` — with the provider keys entered again, because the keychain service moved too.
- A 0.13.x install is still offered this release: the updater's old address redirects to this repository's new name. It arrives as Hotline with an empty room, because the data directory and the keychain service are the ones above. Quit and delete Toad once the room is moved; the old `toad-computer-*` containers and the `toad-home-*`, `toad-src-*` and `toad-nix-glibc` volumes are not used again and can go when nothing in them is wanted.
- The manual-pairing domain string is `hotline-manual-pair-v1` and its vectors are repinned, so a phone still speaking the Toad string cannot pair until it is updated.
- New computers are created on Hotline Computer 0.6.0 or newer: the floor moved with the name, because a desk that passes `HOTLINE_COMPUTER_TOKEN` cannot drive a computer that reads the Toad one. An existing computer keeps the release it runs until it is removed.

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
