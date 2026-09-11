---
title: Import a previous Toad
description: Bring the roster, conversations and keys from an earlier Toad data directory.
---

**Settings → Import → Bring over a previous Toad** takes the data directory
of an earlier Toad and copies its teammates, conversations, threads,
schedules, settings and keys into this one. The source is only read.

Running it twice is the same as running it once: a teammate already in the
room, a conversation already here and a setting already set are left alone.
Imported teammates keep the working directories they had. A key this Toad
has no provider for, or a server entry it cannot read, is listed in the
report rather than dropped in silence.

The same importer is a command line tool, shipped with the source:

```bash
cargo run -p toad-core --bin toad-import -- <from> <to>
```

It prints a JSON report and exits 0, or the error and exits 1. The two paths
must be different directories and the destination must not sit inside the
source.
