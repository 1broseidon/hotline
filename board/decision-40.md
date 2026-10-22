---
id: decision-40
title: Only a seat whose write exceeds the deadline selects zero TCP linger, and its transport closes…
type: decision
status: current
claimKey: wire.abortive-close
kind: decision
supersedes: decision-24
tags:
  - wire
  - remote
createdAt: "2026-10-22T22:53:24.309Z"
---

## Description
Only a seat whose write exceeds the deadline selects zero TCP linger, and its transport closes before admission is released.
The seat owns its writer future directly; it cannot leave a detached writer holding the socket after cancellation. A timeout reunites the reader and writer and consumes the transport synchronously, without a join or another suspension before the enclosing session can release admission. Ordinary closes retain normal drain behavior for final pairing and authentication replies, including on Windows. This preserves decision-24's timeout-only rule and makes its lifetime ordering explicit.
