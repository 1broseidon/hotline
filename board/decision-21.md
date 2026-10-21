---
id: decision-21
title: ThreadStore keeps raw streamed text and publishes only the visible projection.
type: decision
status: current
claimKey: ui.streaming-projection
kind: decision
sourcePath: ui/src/tape.ts
sourceHash: sha256:616e99f5b2418f465c5563772019d8d3adf47b6f4846d389588b5a5a10916b33
createdAt: "2026-10-21T10:27:29.837Z"
---

## Description
ThreadStore keeps raw streamed text and publishes only the visible projection.
Whole reply bubbles, every growing thought, and reply presence (writing activity) are observable; unchanged projections do not notify. Queued deltas settle with their durable event in one publication. First-arrival timestamps remain stable and cadence still owns reveal timing. Raw text stays private so final bubble settlement loses nothing.
