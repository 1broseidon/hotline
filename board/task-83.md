---
id: task-83
title: "Settings: About and Shortcuts move to a quiet footer at the rail's foot"
column: todo
position: 4
priority: medium
tags:
  - settings
  - ui
  - 0.39.0
contract:
  status: ready
  deliverables:
    - type: file
      path: ui/src/components/Settings.tsx
      description: Footer with version and Shortcuts
    - type: test
      path: ui/tests/settings.test.tsx
      description: List has no About or Shortcuts; footer opens both
  validation:
    commands:
      - make ui-check
  constraints:
    - Work on release/0.39.0; do not touch main
    - Tooltips are 2-3 words (brainfile decision ui.tooltip-length); no fake UI (ui.no-fake-ui)
  metrics:
    readyAt: "2026-10-16T15:25:13.483Z"
createdAt: "2026-10-16T15:25:13.490Z"
landsOn: release/0.39.0
updatedAt: "2026-10-16T15:25:14.912Z"
dependsOn:
  - task-82
---

## Description
George 2026-10-16 (screenshot): in the Settings rail, 'Keyboard shortcuts' and 'About Hotline' sit in the list of one-word sections and look out of place. Move them to a footer at the bottom of the Settings rail (ui/src/components/Settings.tsx SettingsRail; styles in ui/src/index.css).

Spec:
- Remove 'shortcuts' and 'about' from the rail's section list (SECTIONS keeps them as routable sections; the list just does not show them). The list is then the one-word sections only.
- Footer at the rail's foot, where the team rail's dock stands: same horizontal inset as the rows (px-2 + row padding, so its text starts on the rows' text edge) and the dock's bottom margin (16px off the foot), so the window's bottom line matches the team view. One row, 32px tall, items centred vertically; it does not scroll with the list (the list scrolls above it).
- Left: the version, 'Hotline 0.39.0' (appVersion(), as About shows it), in --text-sm, ink-3, tabular figures. It is a button that opens About. Tooltip 'About Hotline' (ui.tooltip-length). With no version (dev build without one) it reads 'About'.
- Right: 'Shortcuts', same type and colour, a button that opens Keyboard shortcuts. Tooltip 'Keyboard shortcuts', with the chord appended the way other tooltips do when one is bound.
- States: hover and keyboard focus lift the text to ink-2 with the rows' hover fill on a 6px-radius pill around the word (padding 4px 6px), not an underline; :focus-visible shows the app's focus ring; pressed uses the rows' active fill. While its section is open, the item is ink and carries aria-current, matching an open row.
- If an update is ready (the Updates section's state), a 6px accent dot sits after the version, and its tooltip becomes 'Update ready'; clicking still opens About, which links Updates. (Skip if the update state is not reachable from the rail without new plumbing; note it.)
- Keyboard: Tab reaches the list, then the version, then Shortcuts. Both are real <button>s with accessible names 'About Hotline, version 0.39.0' and 'Keyboard shortcuts'.
- Narrow window (settings full-width, no rail): the footer sits at the end of the section list instead.
- No other entry changes: the ⌘, and any existing chords still open the same sections.
Check in the dev window: list shows only single-word sections; footer aligned to the rows' text edge and 16px off the foot; About and Shortcuts open from it with the right item lit; Tab order; tooltips 2-3 words.
