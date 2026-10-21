---
id: decision-9
title: An interrupted line is not proof of what the caller heard.
type: decision
status: current
claimKey: voice.interrupted-playback
kind: decision
holdsWhile: The wire has no word-aligned playback acknowledgements.
recheck: When clients and providers supply reliable word-aligned playback acknowledgement.
tags:
  - voice
createdAt: "2026-10-21T05:17:40.843Z"
---

## Description
Record interrupted speech as a cut notice containing the generated line and reported playback duration when available; retain the written reply. Without word-aligned playback acknowledgements, explicitly leave the exact last heard word unknown. Pass that playback context separately from the caller's saved words to the next Legacy thread turn; live providers receive truncation or interruption context. Generated text or a proportion of audio duration must never be presented as an exact heard transcript.
