---
id: task-280
title: "Computer files get: a missing file under the home answers 'path must be under /home/agent/'"
column: backlog
position: 35
priority: low
tags:
  - bug
  - hotline-computer
relatedFiles:
  - src/tools/files.rs
createdAt: "2026-10-23T05:25:55.116Z"
---

## Description
In ~/Projects/personal/hotline-computer, src/tools/files.rs ~line 210: the path is canonicalized and any error, including 'No such file or directory', is reported as 'path must be under /home/agent/'. An agent asking for a file that does not exist yet (seen in the acceptance clipboard test on the v0.11.0 image build, 2026-10-23) is told its path is outside the home, which sends it the wrong way.

Done when:
- A missing file under the home answers 'no such file: <path>' and a path outside the home still answers 'path must be under /home/agent/': unit tests for both, and for a symlink that leaves the home.
- make check passes.
