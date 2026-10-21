---
id: decision-18
title: Live work waits for the caller to finish an open dictation phrase.
type: decision
status: current
claimKey: voice.unfinished-dictation
kind: decision
recheck: If provider-native semantic endpointing replaces the lexical guard, or broader language support is added.
createdAt: "2026-10-21T08:52:51.247Z"
---

## Description
Live work waits for the caller to finish an open dictation phrase.
The shared front checks finalized caller words for English draft/say/write requests ending in dangling words. It silently refuses work_on_it while the phrase is open, including guessed completions in tool arguments, and permits stop. Silence does not release the guard; caller continuation does. Tools arriving after speech onset but before transcription wait for that transcript. Provider VAD still owns commits and automatic responses.
