---
id: task-11
title: "Composer: @ autocompletes teammates, and / commands work anywhere in a message (desktop and mobile)"
column: todo
position: 13
priority: high
tags:
  - hotline-mobile
  - feature
  - bro-136
  - 0.37.0
createdAt: "2026-10-11T21:05:22.161Z"
description: |-
  **Target: 0.37.** Updated 2026-10-11 from George: add @ teammate autocomplete, and let / commands sit anywhere in the sentence (this replaces "a / later in a message is ordinary text" below).

  ## @ teammates (new)

  Typing `@` pulls up the room's teammates by public name and autocompletes them, so "ask @The Dude to do this" is a keystroke or two.

  * Opens on `@` at any position, filters as you type, handles names with spaces ("The Dude"); arrows/tap to pick, Tab or Enter to accept, Esc to dismiss.
  * Rows show the face (or initial) and state: idle, working, waiting, stopped.
  * An accepted mention is a token, not loose text: a name with spaces stays one unit, and backspace removes the whole thing.
  * The mention reaches the teammate resolved to the teammate it names, so "ask @The Dude…" can go straight to `message_teammate` without guessing from prose.

  ## / anywhere, and more than four commands (new)

  * A `/command` can appear **anywhere in the sentence**, not only at the start ("tidy this up and /new chapter after"). Decide how a mid-sentence command combines with the rest of the message: run it after the message sends, or before.
  * The picker lists Hotline's own commands (new chapter, chapters, schedules, search, …), the teammate's **skills** (`/skill-name`), and the **agent's own commands**.
  * Agent commands: the contract already has `SessionSnapshot.slash_commands` / `SlashCommand { name, description, hint }`, but it's always empty (`session/mod.rs`, `wire/roster.rs`). Fill it from ACP `available_commands_update`.
  * A `/` that isn't a known command (paths, "and/or", URLs) stays plain text.

  **Done when:** both pickers work in the desktop and phone composers with keyboard and touch; mentions resolve to teammate ids end to end; ACP agent commands appear; CHANGELOG entry under 0.37.

  ---

  ## Original spec (2026-09-26)

  Typing `/` at the start of the composer opens a short list of app utilities. They act straight away, in the app, and nothing is sent to the teammate. Anything you'd *ask* the teammate stays plain chat. Things that already have a button (stop, the computer viewer) get no command.

  ## The set

  | Command | Does |
  | -- | -- |
  | `/chapter` | Start a new chapter with this teammate. The thread folds away, and a toast offers **Undo** for about 5 s. There's no confirmation dialog. |
  | `/chapters` | A sheet of this teammate's chapters (name, when, message count), with the current one marked. Tap one to resume it. |
  | `/schedules` | A sheet of what this teammate has scheduled, each with **Cancel**. The empty state says to ask the teammate in chat to schedule something. |
  | `/search` | A search field under the header. It finds text in the current chapter, highlights matches, dims the rest, and jumps to the first match. **Done** closes it. |

  Keep it to these four. If it grows, add `/help` at that point.

  ## Behaviour

  * The palette opens on `/` as the first character, or from a `/` key inside the capsule on mobile. The capsule keeps its 44pt height.
  * Typing filters by prefix and highlights the top match. Enter runs it, and Esc or tapping outside closes the palette.
  * While the box holds a command, the send button is hidden, so a command can never be sent as a message.
  * A `/` later in a message is ordinary text.

  ## One list for both apps

  Define the commands once, where the desktop app knows the teammate's chapters and schedules, and have both apps draw the palette from that one definition. The actions reuse what already exists: chapter start and resume, schedule list and cancel, thread search.

  ## Mockup

  See concept B in the attached mockup, "Slash utilities". It's clickable: open `index.html` in a browser. Made 2026-09-26, and the conversations are mock ones.

  Migrated from Linear BRO-136: https://linear.app/1broseidon/issue/BRO-136/composer-autocompletes-teammates-and-commands-work-anywhere-in-a (2026-10-11).

  ## Status (2026-10-11 review)

  Target 0.37 (already in the description).
updatedAt: "2026-10-11T21:24:53.016Z"
---
