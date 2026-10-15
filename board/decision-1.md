---
id: decision-1
title: "Never build fake UI for looks: everything displayed works"
type: decision
status: current
claimKey: ui.no-fake-ui
kind: rule
stance: forbids
subject: decorative, non-functional UI
tags:
  - ui
  - design
createdAt: "2026-10-15T22:55:40.518Z"
---

## Description
George's rule (2026-10-15): never build fake UI for the sake of looks. Every control, field and indicator the window or phone displays must do what it appears to do. Example that broke it: the rail dock's Search looked like a field but only opened the old search panel; it is now a real field with results. A control that cannot work yet is left out, not drawn.
