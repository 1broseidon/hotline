---
id: task-19
title: "Core I/O: in-memory fold cache, roster rows from memory, cheap list calls, per-stream write locks"
column: todo
position: 21
priority: high
tags:
  - improvement
  - bro-235
createdAt: "2026-10-11T21:05:29.635Z"
description: |-
  `Log::load` reads and folds the whole file on every call (`log/mod.rs` \~128-136). The room is re-parsed by roster/settings on most commands, and whole tapes are loaded on hot paths. Measured on George's machine: active tape 29 MB, two more 8-10 MB.

  Hot paths that fold a whole tape today:

  * every tape subscribe (teammate switch) folds the whole tape to send 400 lines (`wire/mod.rs` public_snapshot)
  * each prompt loads the tape twice (`went_quiet` + `open_chapter`, `session/mod.rs` \~4063, \~2173)
  * `tape.page`/`thread.page`, `file.read` (once per 512 KB chunk, `sent.rs`), permission/human cards, `mark_read`, `react`, `open_link`, `thread_agent` (on the executor under a spawn_blocking that says why it shouldn't be)
  * `living()` before every file/page/computer command loads the room

  Fix: a `Log`-owned cache of folded streams (room always, tapes LRU), updated inside `append`/`migrate_*` under the writer lock (Log is the only writer). Failing that, tail reads for snapshots plus an id→offset index for single-line lookups. Track each teammate's open chapter id and last activity in memory.

  From the 0.33.4 responsiveness audit.

  ---

  ## Merged in from BRO-237: roster rows from memory instead of tape tails

  Every tool event re-sends a teammate's roster row, and building it reads the tape tail: 64 KB, then 1 MB, then 8 MB when the tail holds no message (`store/previews.rs` \~20-24, \~129-149). It also deserializes every tail event for `activity_on`/`waiting_on` and fully loads each live side and pair stream for `threads_waiting`.

  Fix: keep each teammate's preview, open tool and open cards in memory, updated from the tape broadcast the roster pump already receives (`wire/roster.rs` \~237-252); read disk only for the first snapshot. Also send models/slashCommands only when they change.

  ## Merged in from BRO-239: models.list, thread.list and side.list do far too much I/O

  * `models.list` reads the OS keychain 3× and loads the room 4× per call (`Vault::connections`, `vault.rs` \~336-382; `driver/rig.rs` \~299-343 repeats it). Fix: compute connections once per call; cache decoded secrets in the Vault, invalidated by its own writes.
  * `thread.list` is quadratic (`thread/store.rs` \~53-97, `wire/threads.rs` \~319-357). Fix: scan once per call, load each stream once.
  * `side.list` loads every side file then filters (`session/sides.rs` \~1130-1142). Fix: read only the head link line, or keep a teammate→sides index.

  ## Merged in from BRO-240: narrower write locks and background search indexing

  * One global log writer lock for every stream (`log/mod.rs` \~105, \~192-200); each append also does create_dir_all/metadata/open (`tape.rs` \~88-93), and `compact` holds the lock while rewriting a 29 MB file. Fix: lock per StreamId plus cached append handles.
  * The indexer lock is held across a full tape load and a SQLite commit on every message write (`session/mod.rs` \~4275-4283, `search.rs` \~747-754). Fix: seed open chapters at startup and index on a background mpsc worker.
  * `policy_update_lock`: settings.update still holds it across restarts.

  Still true at 0.36 (backlog review): `Log::load` reads and folds the whole file; previews still use the 64 KB → 1 MB → 8 MB reach; `vault.rs:366` still reads secrets per call; one global writer.

  Migrated from Linear BRO-235: https://linear.app/1broseidon/issue/BRO-235/core-io-in-memory-fold-cache-roster-rows-from-memory-cheap-list-calls (2026-10-11).
updatedAt: "2026-10-11T21:20:20.431Z"
---
