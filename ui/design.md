# Design — the Toad window

A locked design system for the window in `ui/`. Every screen reads this
file before it is drawn; a screen that drifts from it is a bug, and the
cure is to amend this file, never to override it locally. The values it
names live in `src/tokens.css`, with the reasons beside them. The
architecture record for the tree is `docs/design.md`; this file is only
how the window looks and moves.

## Genre

Modern-minimal instrument. A desk you sit at all day, not a page you
visit. Nothing decorates; everything on screen is a thing you can read or
press.

## Macrostructure

One family, **Workbench**, for every screen:

- On Linux and Windows the shell draws no frame. The window's top strip is
  the well itself: 32px, the window's title in the centre in ink-3, the
  mark in the left corner where a frame keeps its app icon, and minimize,
  maximize and close flush to the right corner as marks on the chrome, not
  keys. A hairline a shade lighter and bluer than the well outlines the
  whole window, so it has an edge against a dark desktop. On Linux the
  window is see-through and the page rounds its own corners to the pane
  radius, so the window is one more pane on the desk; maximised, it is
  square. macOS keeps its traffic lights over the rail band instead.
- The **mark** is one shape in one colour, `ui/src/ui/ToadMark.tsx`. In
  chrome it is ink-3, the colour of a reading, never the accent: the
  accent is for what is happening, and the mark is furniture. The app
  tile is the same drawing in the accent on a dark rounded square
  (`assets/toad-tile.svg`).
- The **well** is the window's ground. The rail stands directly in it.
- A **pane** is a rounded canvas set 8px into the well with one hairline
  at its edge and a 1px light along its top: the conversation, the
  inspector, settings, a thread. Panes sit 8px apart. A pane never has a
  border-line under its band; what scrolls fades out beneath the band and
  above the composer.
- The **composer** floats over the conversation on the one shadow in the
  window, and it is a pill: one 40px row that grows a line at a time,
  the way a message to a person is typed, with attach at the left end and
  send at the right. Send arrives with the first character and is the
  stop key while the teammate works; nothing in the pill waits greyed
  out, and there is no hint line under it. Focus is the caret: the pill
  brightens its edge, never rings. It is always open: a teammate is
  always there, and whether a session is up behind them is plumbing the
  window never reports. There is no start key and no "not running"; a
  message to a resting teammate starts the session on its way. While
  they work the mark floats above the composer — not in a bubble, because
  it is not a message — with one word for what kind of work: Reading,
  Searching, Editing, Running, Thinking, or Waiting on you. Nothing
  streams. A reply lands whole, the way a text does, and the mark
  collapses into three dots while it is on its way. A press on the mark
  opens the work behind it for that turn. The band is the name, the
  model, and the tools — everything else about a teammate lives under the
  inspector.
- Menus and the search panel are pop-plane surfaces that borrow the
  composer's shadow while they are open, and nothing else.

A window narrower than 720px holds one thing at a time, the way a phone
does: the rail, or what was chosen in it, with a back key at the head of
the pane's band and Escape as the same step. Nothing is re-laid out in
place; a pane replaces the rail rather than squeezing beside it, and a
wide window never shows the back key. The conversation's band keeps the
name, the model and its three keys; mode, effort and the schedule line
fold into the More menu as checked groups — the same choices, one press
further away, rather than a band that clips them. The inspector has its own, wider
cut-off, because it needs the conversation beside it and the rail does not.

Providers and Tools are one shape, because they do one job: a list of
things the room has. An add row heads the list in accent — the one accent
in a pane, because adding is what an empty list is for — and opens the
form in place, or for Providers the rows of what is not yet connected,
in place too: nothing on these pages pops over them. Each
row is the thing's name over one detail line (a key or a login, a command
or a URL) and opens the thing's own page, with its fields and the way out
(Remove, Sign out) at the foot. Nothing is edited on the row.

The computer is two places. Settings › Computer is the room's word: the
runtimes this machine could have as radio rows, the missing ones greyed
with the sentence that names what is missing, and the image the desk
wakes by default. The teammate's pane is that teammate's word: the switch,
its own image, and one status row that reads the container every few
seconds while the pane is open — Open desktop and Stop while it runs,
Remove once it has stopped. A capture on the tape is a thumbnail card that
grows when pressed; it is evidence beside the words, not a message.

Settings, New teammate, Keyboard shortcuts and About are panes in the
conversation's place. Settings also takes the rail: its sections stand
where the team stood, one row each, and the band carries the way back
where the team's plus was — it is a place you go, and the room steps
aside until you return. The inspector and a peer thread are panes beside
the conversation.

## Theme

Dark is the designed theme; light re-values the same names. One hue lane,
cool slate at 250 with a trace of chroma, so green reads as a signal on
the material rather than a part of it.

Depth is lightness, five steps: well 12 · canvas 16 · raised 20 · pop 24 ·
pressed 28. A raised or pop surface carries the top-light. Shadows are not
structure: the composer rests on one, an open menu borrows it, nothing
else casts one.

Ink is four voices: ink says it, ink-2 explains, ink-3 annotates, ink-4 is
furniture. Fills and lines are alpha so one rule reads right on every
plane. A hairline is one device pixel and is a plane's edge, never a
divider between things already on different planes.

The accent is wet leaf, `oklch(76% 0.17 142)`, on a budget under 5% of any
screen, meaning exactly: an agent at work, the send key, the primary
action, the card waiting on you. It is never a background or a decoration.
Your own words are a slate wash, not the accent. Warn and danger exist
and are text or a soft wash, never a fill. Faces are seven hues at one
lightness and one chroma, kept below the accent's so the working beat is
the loudest colour in the rail; red is absent.

## Typography

- Body and display: IBM Plex Sans 400 / 500 / 600, always roman.
- Instrument: IBM Plex Mono 400 / 500, at 11px with +0.04em, for anything
  measured or typed: a time, a path, a tool's name, a count, a state, a
  card's kind. Sentence case; nothing is shouted in caps.
- Six sizes with fixed leading: 11/14 · 12/16 · 13/18 · 14/22 (speech) ·
  15/20 · 20/26. Tabular figures everywhere.
- Emphasis is weight or the accent, never italic in a heading.

## Spacing and shape

A 4px grid, expressed as Tailwind's spacing. Radii are concentric from the
OS window inward: pane 10 · card 8 · control 6 · chip 4; a circle for a
face or a mark; the one bubble is 12 because it is a shape, not a
container. Nothing is rounder than what holds it.

Controls are 26px high; a field is 28px and is sunk into its surface (the
well's colour, an inset hairline), while a key is raised (the pop plane,
the top-light). You press a key and type into a well.

## Motion

One curve, `cubic-bezier(0.16, 1, 0.3, 1)`, three durations: 90ms answers
a press, 130ms opens something small, 220ms moves something across a pane.
No overshoot, no hover scale, no transition-all. A working teammate's dot
breathes at 1800ms, and the mark above the composer moves only because of
something: a read sweeps the eyes, a search darts them, an edit presses,
a command ratchets, a permission request stops it dead to look at you.
Every pose is a function of the phase and the time in it; the blink is
the one motion that is not caused. Reduced motion removes every
transition and animation, and holds the mark still.

## Microinteractions stance

- Silent success. The tape is the confirmation; there are no toasts inside
  the window. A desktop notification fires only while the window is not
  focused.
- Nothing rings. A pointer never leaves focus on a key, so no key wears a
  ring after being pressed; keyboard focus shows as the control's hover
  fill, and a field or the composer brightens its own hairline. The
  accent is not spent on focus.
- Every state is a glyph and a colour, never colour alone.
- Hover appears on the row, never on the whole card, and fills are alpha.

## Component voice

- Primary: accent fill, on-accent ink, 26px, radius 6. One per screen.
- Key: pop plane, top-light, one hairline. Hover goes to pressed.
- Quiet: text, hover fill only. Icon buttons are 26px squares.
- Destructive: danger ink, no fill, and a system dialog before anything
  goes.
- Speech: bubbles on two sides, the way a messages app draws a 1:1.
  Theirs on the left in the raised plane, yours on the right in the slate
  wash — told apart by side and weight, never hue, and never a tail. A run
  from one speaker tightens the corners that face each other. A reply
  arrives paced, as a few bubbles in one run. Length is the agent's to
  keep down, not the window's to fold: a long reply is a long bubble,
  and the fix is the prompt, not a card. What happened between two
  messages is one quiet caption in the instrument
  voice — a count, closed until pressed; a transcript that hides it
  would lie, one that shouts it is a log.
- Needs you: the one card that is a request of the person, live until
  answered. It says what is wanted and offers the answers — Done and
  Decline — and nothing else by default; a note is a quiet affordance
  that opens a field, not a field waiting to be ignored. While the
  teammate's desktop is running the card leads with Open the screen and
  says the screen is the person's until Done, so the way to do the thing
  is the first thing on the card. The desktop opens in a window of its
  own, one per teammate; the band carries Screen while it runs.
- Machinery between messages is folded into instrument rows on a 2px rule.
- The inspector is 320px, so it shows state and folds detail. The name is
  the band's heading and edits in place; the goal is the one field; the
  working directory is a row of its own, the folder's name and three
  quiet keys (the full path, choose, reveal), never a path field. Every
  grant is one row of one Access list, ordered authority, equipment,
  outcome: reach, background work and collaboration, each with an info
  key for its risk; the computer and MCP servers; then what attached at
  the last start. A row is a title, a value after a dot if there is one,
  and a switch, a chevron or quiet keys at the right edge and nothing
  else. A row's second line is a
  value or nothing — what a grant means is the docs' job, not a sentence
  under every row. A chevron opens in place, set in from its row; what
  attached at the last start is the list's last row, with the failures
  always out. Schedules and threads take the same shape. Nothing sits
  beside a text field, and the destructive key is the footer, asking
  through the system dialog.

## What every screen MUST share

The well and the pane shape; the planes and the accent's meaning; Plex in
both faces; the control heights and radii; the band at 40px with no line
under it; the scroll fade.

## Per-surface allowances

- The conversation alone has the floating composer.
- The rail alone has no pane round it.
- Settings may carry raised grouped lists; the tape may carry raised cards.
- Nothing may carry an illustration, a gradient other than a scroll fade,
  or a shadow other than the float.

## Log

- 2026-09-02 LOCKED. Hue lane 250, accent kept green, Plex bundled, dark
  first with light re-valued. Ancestor: the previous Toad's `tokens.css`.
