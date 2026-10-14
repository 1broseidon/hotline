---
id: task-39
title: "Review iggyuga's teammate-face PRs: photo or per-mood video (#176), and drawn SVG faces (#178)"
column: backlog
position: 23
tags:
  - bro-275
createdAt: "2026-10-11T21:06:02.309Z"
description: |-
  Teammates are drawn as their initial, and the only way to give one a picture is to have it generate one (`avatar.generate`) or set one itself (`set_avatar`). The person has no way to choose a teammate's face, even though `AvatarBy::Person` already exists for exactly that.

  I'd like to let the person choose, per teammate, from the teammate pane:

  * **Initial** — today's default.
  * **Photo** — upload their own image, stored like any other avatar with `AvatarBy::Person`.
  * **Video** — a short clip per mood (idle, sleep, attentive, think, work, done, grumpy, crossed-arms, needy). The window picks the mood from what the teammate is doing (thinking, working, finished, errored, stopped, waiting on the person) and from the person (typing, window in the background, long idle).

  A new Settings → Avatars section turns moving faces on or off (off by default) and chooses where they appear: a large face at the top of the conversation, the face in place of the activity mark while the teammate thinks, and whether faces doze while the window is in the background.

  The choice lives on the persona's record as a `look`, so every window draws the same face. Only the owner can set a look; any seat can read clips.

  I have this working and will open a PR alongside this issue. Happy to reshape it however fits the project.

  Migrated from Linear BRO-275: https://linear.app/1broseidon/issue/BRO-275/let-the-person-choose-a-teammates-face-photo-or-per-mood-video (2026-10-11).

  ## Status (2026-10-11 review)

  External PRs hotline#176 and #178 (iggyuga) are open; need a review decision.
updatedAt: "2026-10-14T20:05:24.258Z"
priority: medium
---

## Log
- 2026-10-12T01:51:40.634Z: [claude] Merged task-40 (drawn SVG faces: the risks) into this card: #178 builds on #176, so they are one review. Issues #175 and #177 describe them.
