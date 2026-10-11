---
id: task-27
title: "Teammate computer: controls apply for real (memory, CPUs, folders) and a remake keeps the home, on all three runtimes"
column: todo
position: 29
priority: high
tags:
  - bug
  - bro-256
createdAt: "2026-10-11T21:05:36.250Z"
description: |-
  ## What happened (2026-10-08)

  * Raising a teammate computer's memory with the slider, from both the desktop and the phone, did **not** take effect on its running/existing computer. Only removing the computer and starting fresh applied the new limit (observed: the container stayed at 4 GB, and builds were OOM-killed, until it was removed and recreated with 16 GB).
  * When it was recreated on **Apple container**, the computer's whole home (`/home/agent`, including cloned repos, build output and prepared workspaces) was gone. On Docker and Podman the named volumes (home, store, scratch) survive a remake; on Apple container only the rw layer exists, so a remake starts over.
  * Related: 0.35's extra folders also remake the computer when its folders change, which on Apple container loses the desktop session the same way.

  ## Expected

  * A resource change (memory, CPUs) either applies on the next start or clearly says it needs a restart/remake — and does it for you — rather than silently keeping the old limit.
  * A remake keeps the computer's home and caches on every runtime, so changing memory or folders never costs the person their work.

  ## Audit — all three runtimes (Docker, Podman, Apple container)

  For each runtime, verify and fix:

  1. Memory and CPU changes from desktop Settings and the phone: applied on restart? on remake? reported back accurately in the UI?
  2. Persistence across stop/start, restart, and remake: home, Nix store, scratch, prepared workspaces, job history.
  3. What triggers a remake (resources, folders, image tag/upgrade) and whether each trigger preserves state.
  4. Apple container specifically: use volumes or a host-backed directory for home/store/scratch so a remake keeps them, if the runtime allows it.
  5. The UI tells the truth about what a change will do ("applies after restart", "remakes the computer").

  Headless harnesses per runtime where possible; the fake runtime alone doesn't catch this.

  ## Merged in from BRO-257 (controls take effect immediately)

  * **Memory and CPUs:** update the running container live where the runtime can (`docker update` / `podman update`). Where it can't, restart or remake for you after a clear "this restarts the computer" confirmation, without losing its home.
  * **Every other control** on the Computer settings page and the phone (folders, network, image or upgrade, on/off): each applies right away or says what it will do and then does it.
  * The UI shows the computer's **actual** limits read back from the runtime, not the slider's last value. The phone sees the same state.

  **Done when:** each control has a headless harness per runtime (Docker, Podman, Apple container) proving the running computer reflects the change, and there are no "saved but not applied" states left in the UI.

  Code as of 0.36 (backlog review): `computer/mod.rs` \~388-440 `ensure_running` only remakes on folder change and nothing compares memory/CPU; `create_args` gives Apple container no named volumes.

  Migrated from Linear BRO-256: https://linear.app/1broseidon/issue/BRO-256/teammate-computer-controls-apply-for-real-memory-cpus-folders-and-a (2026-10-11).
updatedAt: "2026-10-11T21:05:36.547Z"
---
