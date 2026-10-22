---
id: decision-37
title: Computer status prefers the guide's learned release, then the creation-time hotline.release label…
type: decision
status: current
claimKey: computer.release-recovery
kind: decision
sourcePath: docs/design.md
sourceHash: sha256:920dd86d44d3661e2d68b6ee099fe15931411759ab1e212919234d29ff16bf9c
createdAt: "2026-10-22T16:18:13.138Z"
---

## Description
Computer status prefers the guide's learned release, then the creation-time hotline.release label, then the runtime's image reference for older containers. The desired image setting never identifies an existing container's release, so restarting the desk or failing to fetch a guide cannot hide an available update.
