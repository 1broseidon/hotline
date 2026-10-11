---
id: task-28
title: "Reset Hotline: erase all data and return to the welcome wizard"
column: todo
position: 30
priority: medium
tags:
  - feature
  - bro-258
createdAt: "2026-10-11T21:05:37.080Z"
description: |-
  Target: **0.36**

  0.35 decides whether to show the welcome from room history: a room that has ever had a teammate counts as set up (`room::has_had_a_teammate`). There's no way back to a fresh start without deleting files by hand.

  ## Shape

  * Settings › General gets a **Reset Hotline** section at the bottom, styled as destructive.
  * It lists what will be erased: teammates and their conversations, workspaces, provider keys and sign-ins (keychain entries and token files), voice models, teammate computers and their volumes, paired phones, schedules and settings.
  * Confirm by typing "reset", or something equally deliberate. Offer to export or back up first if that's cheap.
  * Afterwards Hotline restarts into the full-window welcome wizard, exactly as on a first launch.

  ## Notes

  * Erase in a set order and never touch anything outside Hotline's data dir and its own keychain items. Extra folders a teammate was given are the person's own files and must survive.
  * Stop and remove teammate computers through their runtime first; don't just forget them.
  * A headless harness checks that after a reset, the core reports "not set up" and the data dir, keychain entries and computer volumes are gone, while extra folders are untouched.
  * Follow `docs/security.md`, since this touches secrets and capabilities.

  Migrated from Linear BRO-258: https://linear.app/1broseidon/issue/BRO-258/reset-hotline-erase-all-data-and-return-to-the-welcome-wizard (2026-10-11).
updatedAt: "2026-10-11T21:05:37.362Z"
---
