---
id: task-23
title: "Linux: opaque window to stop resize flicker (needs A/B)"
column: backlog
position: 15
priority: medium
tags:
  - bug
  - bro-243
createdAt: "2026-10-11T21:05:32.959Z"
description: |-
  The main window is frameless and transparent (`hotline-app/src/lib.rs` \~476-482), so WebKitGTK composites the whole window with alpha every frame; transparent windows are prone to black/garbage edges and flicker on resize. A fixed full-window `body::after` overlay at max z-index (`index.css` \~113-141) adds a full-size layer repainted on any change.

  Try: `transparent(false)` with square corners (rounded only when not maximized and a compositor is present), draw the hairline on `#root` and drop the overlay. Needs a side-by-side look with George before shipping.

  Migrated from Linear BRO-243: https://linear.app/1broseidon/issue/BRO-243/linux-opaque-window-to-stop-resize-flicker-needs-ab (2026-10-11).

  ## Status (2026-10-11 review)

  Needs the A/B with George.
updatedAt: "2026-10-14T20:05:20.336Z"
parentId: epic-5
---
