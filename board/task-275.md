---
id: task-275
title: Update available isn't offered for a computer the desk didn't start itself (after a desk restart)
column: in-progress
position: 0
priority: high
tags:
  - bug
  - computer
  - 0.43.0
relatedFiles:
  - crates/hotline-core/src/computer.rs
  - crates/hotline-core/src/session/computer.rs
createdAt: "2026-10-22T16:12:00.496Z"
landsOn: release/0.43.0
updatedAt: "2026-10-22T16:12:41.110Z"
dispatch:
  agent: codex
  by: cli
  at: "2026-10-22T16:12:02.063Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59533
    at: "2026-10-22T16:12:03.081Z"
assignee: codex
---

## Description
George, 2026-10-22 11:10: a teammate's computer on rc.7, image set to rc.8, and Settings showed no update even after Check now, so it couldn't be updated in place.

Cause: Room::computer_status (session/computer.rs ~384–414) sets status.available only when status.release is known, and the running release is learned only from the guide, in finish_grant, by the desk process that started the container (Computer::learned_release, held in Inner.containers). After the desk restarts (an app update, a relaunch, the dev window rebuilding), an existing container isn't in that map: inspect_status takes its second branch, status_of(inspection, None), and release stays None, so no Update row. It also stays None for a held container whose guide fetch failed. On 2026-10-22 the dev window was rebuilt at 11:03; the rc.7 container predated it. Browsers and activity play no part.

Fix: record the release on the container itself and read it back. Add a label at creation (e.g. hotline.release=<tag of the image>, beside the folders and limits the inspection already reads), read it in Inspection, and use it when the desk holds no learned release. For containers made before the label, fall back to the image reference the runtime reports for the container (Apple container, Docker and Podman all report it), passed through release_of.

Done when:
- A container this desk did not start (simulated restart: a fresh Computer over an existing container) reports its release and offers the update when the image differs: named test.
- A held container whose guide fetch failed still reports its release from the label: named test.
- A container made before the label falls back to the image reference: named test with an inspection fixture per runtime.
- Check now on such a computer shows 'X is available' and Update works: manual check in the dev window after a restart.
- make check passes.
