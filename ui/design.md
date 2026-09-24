# Design — the Hotline window

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

- The window's top strip, on every platform, is the well itself: 32px,
  the key that shows and hides the rail in the left corner, the mark
  alone in the centre, and the search at the right. It is the window's,
  not a teammate's: who is open, and their model and effort, are in the
  pane's band, and only the task bar, which has no band, names them. On
  Linux and Windows the shell draws no frame, and minimize, maximize and
  close sit flush to the right corner beyond them as marks on the chrome,
  not keys. A hairline a
  shade lighter and bluer than the well outlines the whole window there,
  so it has an edge against a dark desktop. On Linux the window is
  see-through and the page rounds its own corners to the pane radius, so
  the window is one more pane on the desk; maximised, it is square. macOS
  keeps its traffic lights in the left corner, on the strip's centre
  line, and the strip leaves room for them.
- The **mark** is one shape in one colour, `ui/src/ui/HotlineMark.tsx`:
  the toad with a telephone receiver resting across its eyes, where a
  desk phone's handset sits in the cradle, cut from them by a gap. At
  24px and under, and on the mark that moves, it draws the brow: a slim,
  straight handset whose wider cut survives as a pixel
  (`ui/src/ui/receiver.ts` holds both). The titlebar and the tray wear
  the toad alone; the handset is what the mark does, and chrome is at
  rest. In chrome it is ink-3, the colour of a reading, never the accent:
  the accent is for what is happening, and the mark is furniture. The app
  tile is the drawing with its handset in the accent on a dark rounded
  square (`assets/hotline-tile.svg`).
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
  Searching, Editing, Running, Thinking, or Waiting on you. It wakes up
  from behind the composer when the work starts, and when the turn is over
  it settles on the cradle and sinks back behind it to sleep. Nothing
  streams. A reply lands whole, the way a text does; while it is on its
  way the receiver lifts off into a wave running above the toad, which
  stays and watches it — on the line — and when it lands the line reels
  back into the handset, which hangs up and winks before the mark goes. A
  press on the mark opens the work behind it for that turn. The band is
  the one place that says who this is: the face, the name and one quiet
  line of what they are doing or, at rest, what they are for; then their
  model and effort and the tools. The name opens the inspector, where
  everything else about a teammate — its mode included — lives.
- Menus and the search panel are pop-plane surfaces that borrow the
  composer's shadow while they are open, and nothing else.

In a wide window the rail is yours to size: its edge is the gutter
beside it, dragged between 180 and 420px. Dragged past its narrowest it
does not squeeze its rows; it drops to faces, 52px, each name and line
waiting on a hover card on the menus' surface; dragged past the faces
it closes. The key in the left corner of the titlebar and Ctrl/⌘+B close
and open it; closed, its edge stays in the window's left gutter so it
can be pulled back out, and it returns at the width it had. Settings'
sections stand in the same place at the same width.

A window narrower than 720px keeps the pane and shows the rail as faces
only beside it, opened and closed from the same titlebar key; it is
never dragged there, and there is no view of the rail alone and no back
key. Settings' sections stand at the rail's narrowest beside their pane.
The band's schedule line folds into the More menu, one press further
away, rather than a band that clips it. The inspector has its own, wider
cut-off, because it needs the conversation beside it and the rail does not:
it is measured on the room the rail leaves, not the window, so a closed
rail makes space for both and a wide one hands the inspector the
conversation's place.

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
something, and each kind of work differs from every other on at least two
of the receiver, the eyes and the body: thinking holds the receiver at
the ear and looks up, a read parks it low and squints down the page, a
search snaps the head about with the receiver swinging against it, an
edit tucks it at the shoulder and takes each keystroke, a command hops,
and a permission request puts it back on the cradle, rings twice, and
stares at you. A blink marks each change into a new kind of work. Every
pose is a function of the phase and the time in it; the idle blink is
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
- The inspector is 320px, so it shows state and folds detail. Its band
  says only Teammate. It opens on who they are, the way a contact card
  does: the face, the name at the heading's size and the goal under it
  like a bio, each plain text until pointed at, when it shows the field
  it is. The working directory is a row of its own, the folder's name
  (Its own folder when it is the teammate's) and three quiet keys (the
  full path, choose, reveal), never a path field. Every grant is one row
  in one of three lists, ordered authority, equipment, outcome:
  Permissions (reach, background work and collaboration, each with an
  info key for its risk), Computer, and Tools (MCP servers, skills, then
  what attached at the last start). A row is a title, a value after a
  dot if there is one, and a switch, a chevron or quiet keys at the right
  edge and nothing else. A row's second line is a value or nothing —
  what a grant means is the docs' job, not a sentence under every row. A
  chevron opens in place, set in from its row; what attached at the last
  start is the Tools list's last row, with the failures always out. Schedules and threads take the same shape. Nothing sits
  beside a text field, and the destructive key is the footer, asking
  through the system dialog.

## What every screen MUST share

The well and the pane shape; the planes and the accent's meaning; Plex in
both faces; the control heights and radii; the band at 40px with no line
under it; the scroll fade.

## Words

A title says what a thing is and its control shows what it is set to; a
hint is one sentence, and only where the title cannot carry it. A refusal
is one sentence of ours. Someone else's words — a daemon's, a provider's, a
process's — go behind a disclosure, selectable, with a page to read when
there is one, and never in the row.

## Per-surface allowances

- The conversation alone has the floating composer.
- The rail alone has no pane round it.
- Settings may carry raised grouped lists; the tape may carry raised cards.
- Nothing may carry an illustration, a gradient other than a scroll fade,
  or a shadow other than the float.

## Log

- 2026-09-02 LOCKED. Hue lane 250, accent kept green, Plex bundled, dark
  first with light re-valued. Ancestor: the previous edition's `tokens.css`.
