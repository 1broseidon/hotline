---
id: task-279
title: "hotline-computer acceptance: native controls' Ctrl+A then paste sometimes appends instead of replacing (CI amd64)"
column: backlog
position: 34
priority: low
tags:
  - flaky
  - tests
  - hotline-computer
createdAt: "2026-10-23T05:03:24.408Z"
---

## Description
In ~/Projects/personal/hotline-computer. On PR #20 (2026-10-23, 0.11.0) the desktop (amd64, ubuntu-24.04) job's 'native controls and window identity' failed once at the paste assertion: the field read 'Editable native valueNative café 🐸', so Ctrl+A had not selected the text when the paste landed. The rerun passed, arm64 passed, and on rc.8 on the Mac the same click / Ctrl+A / paste replaced the text every time, as separate calls and as one batch.

Likely the test's click and key arrive before GTK has finished focusing the entry on a loaded runner.

Done when:
- The test waits until the entry reports focused (or the selection covers the whole value) before pasting, or sends the three steps as one input batch, and still fails if Ctrl+A is broken: named test change.
- 10 consecutive acceptance runs pass on amd64.
