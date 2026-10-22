---
id: decision-35
title: Unix shell termination stops the owned process group before killing it.
type: decision
status: current
claimKey: shell.group-cancellation
kind: decision
holdsWhile: Hotline owns shell jobs through a Unix process group.
tags:
  - shell
  - security
createdAt: "2026-10-22T07:43:08.456Z"
---

## Description
Cancellation, deadlines and dropped shell tasks share one ProcessGroup termination path: send SIGSTOP to the owned group, then SIGKILL to the same group, always attempting the kill even if stopping fails. A single group kill is not simultaneous signal delivery: XNU killpg1 iterates members, so killing sleep first can wake its waiting shell into the next statement. Managed cancellation must still observe exit before reporting completion. Already performed effects are not rolled back; processes that deliberately leave the owned group remain outside this guarantee.
