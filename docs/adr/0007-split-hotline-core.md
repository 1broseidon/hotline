# ADR 0007: Split hotline-core into crates, leaves first

- Status: The five crates landed on `release/0.41.0` (epic-16). Of the
  core's split, under Next, steps 1 and 2 are done (`hotline-room`,
  epic-18: task-211 and task-208; `hotline-mcp`, task-209); steps 3 and 4
  are not started.
- Date: 2026-10-19
- Supersedes: the "No new crates" line of [standards.md](../standards.md#module-layout),
  once task-196 rewrites it. The line after it, "Do not break the
  `session`↔`driver`, `session`↔`voice` and `mcp`↔`session` cycles with
  single-implementation traits", stands and shapes the last section.
- Tracking: epic-16 (task-193 this baseline, task-194 `hotline-contract`,
  task-195 `hotline-websearch`, task-196 `hotline-support`, task-197
  `hotline-store`, task-198 `hotline-tools`, task-199 the timings again and
  the plan for the core); epic-18 for the core's split (task-211 and
  task-208 `hotline-room`, task-209 `hotline-mcp`)

## Context

`hotline-core` is one crate of 163,049 lines (10,005 more in its tests/), with
58 direct dependencies. Rust's front end (parsing, name resolution, type and
borrow checking) runs on one thread per crate, so a crate that size cannot use
the machine's cores for the part of the compile that dominates it, and nothing
in the language stops a module reaching into any other. George asked for it
split on 2026-10-19, after the post-0.40 sniff test, for enforced boundaries
and faster incremental builds.

standards.md said to revisit "no new crates" when an incremental
`cargo test -p hotline-core --no-run` took more than 30 seconds, or the
contract got a second consumer. The numbers below do not trip the first: it
takes 12.4 seconds. The reason to split is the second half of the ask, that
boundaries nothing enforces drift, and the cold build; this ADR says what each
stage is measured against so a stage that buys nothing is seen to buy nothing.

## Decision

Split in stages, each a move with no behaviour change, in the order
task-194, 195, 196, 197, 198. Each stage is one crate that holds one
dependency direction, and each is measured. What stays in `hotline-core` is the
strongly connected core below; this epic plans its split (task-199) and does
not start it.

### The crates

| Crate | Owns | Depends on (workspace) |
|---|---|---|
| `hotline-contract` | the wire and tape types, with their ts-rs derives (`contract/`) | none |
| `hotline-websearch` | web search (`websearch/`) | `hotline-contract` |
| `hotline-support` | state-free utilities: `credentials`, `process_windows`, `images`, `thumbs`, `fence`, `room_lock`, `log_file`, `paths` | none |
| `hotline-store` | the log, the tape store and its search index, sent files: `log`, `store`, `sent` | `hotline-contract`, `hotline-support` |
| `hotline-tools` | the shell and workspace tools and the capability lease they check (`tools/`) | `hotline-contract`, `hotline-support` |
| `hotline-room` | the room as the code above the core sees it (Next, step 1) | `hotline-contract`, `hotline-support`, `hotline-store`, `hotline-websearch` |
| `hotline-mcp` | the MCP client (Next, step 2) | `hotline-contract`, `hotline-support`, `hotline-tools`, `hotline-room` |
| `hotline-core` | everything else | all of the above |

`hotline-app` and `hotline-cli` depend on `hotline-core` as now.

Layers: `hotline-contract` and `hotline-support` first; `hotline-websearch`,
`hotline-store` and `hotline-tools` on them; `hotline-core` on all five;
`hotline-app` and `hotline-cli` on it. Edges only go down; no new crate
depends on `hotline-core`, which is the point. Each gets the workspace
`[lints]`, a license, and a crate-level doc saying what it owns and what it
must never depend on.

### The name of the leaf-utilities crate

`hotline-support`. It reads right for what it holds, things the rest stand on
that know nothing about Hotline's state, and the alternatives (`hotline-util`,
`hotline-base`, `hotline-kit`) say less. task-196's contract names it, and
nothing here changes that.

It needs no workspace dependency at all, not even `hotline-contract`: of the
modules it takes, none names `contract` (the map below), and its one link into
the core (`paths` → `linked::is_linked`, see the cuts) is a string test that
should move down with it. task-196 allows `hotline-contract` "at most"; the
expectation is zero.

### The three cuts the stages need

Each is one reference site (the map says where) and each is part of the stage
that needs it, not a stage of its own:

1. **`paths` → `linked`** (`paths.rs:293`, task-196). `safe_thread_id` asks
   `linked::is_linked`, which is `split(id).is_some()`, which is
   `valid_desk_id` and `valid_persona_id` and a `split_once('/')`: the grammar
   of a node-qualified id, pure strings. Move those four functions (with
   `is_linked`) into `paths` and let `linked` re-export them with
   `pub(crate) use`, so the two dozen call sites outside `linked` do not move.
   This frees a three-module cycle, `linked` → `log` → `paths` → `linked`, by
   its cheapest edge.
2. **`store` → `thread`** (`store/search.rs:26`, task-197). The search index
   reads `thread::{Link, ThreadState}`. `Link` (`thread/link.rs`) names only
   `contract`, `ThreadState` and `AgentBinding`, and those two are plain
   enums and structs with no ts-rs derive (`thread.rs:104-146`), so the three
   move down to `hotline-contract` or into `hotline-store` (task-197 picks),
   and `thread` itself, which reaches `room` and `session`, stays in the core.
   task-197 put them, with `Opener`, in `hotline-store`'s `link` module: a
   link is a line on a tape, and none of them is a wire type. `thread`
   re-exports them, so no caller moved.
   If a ts-rs derive turns up on a moved type, `contract.ts` must still come
   out byte-identical.
3. **`tools` → `driver`** (`tools/workspace.rs:3`, task-198).
   `CapabilityLease` and `CapabilityEpoch` move into a capability module the
   tools crate owns, not into `hotline-contract`: they are runtime authority,
   not wire types. That widens them from `pub(crate)` to `pub`; their fields
   stay private so a lease cannot be forged, which is the security.md method's
   residual risk to state in that PR.
   task-198 put them in `hotline-tools`' `capability` module with their
   tests; `driver` re-exports them and the core re-exports the crate as
   `tools`, so no caller moved. With this cut made, `scripts/module-map`
   has no planned cuts left and no longer prints the layers after them.

`websearch` needed no cut: it names only `contract`. task-195 moved it whole
and the core re-exports the crate as `websearch`, so no caller moved. The one
thing that crossed back was test code: the core's tests of the `web_search`
tool point the chain at websearch's mock providers, so the mock left
`tests.rs` for a `mock` module that is public only under a `test-support`
feature, which the core enables as a dev-dependency.

`push` → `remote` (`push.rs:23`, one call to `remote::push_targets`) is also a
single-link cut, but `push` is 379 lines and no stage moves it. The same goes
for `skills`, `spending` and `link_preview`, which are leaves of the leaves
already: a crate for each is the one-crate-per-file shape task-196
refuses, and they cost the core nothing to keep.

## Before

Measured on 2026-10-19 on `release/0.41.0` at 3987d089, an Apple M5 Max (18
cores, 128 GB), macOS 27.0.1, rustc 1.99.0 and cargo 1.99.0, dev profile, in a
fresh worktree with an empty `target/`. The speech-engine archive was put in
`.speech-engine/` first (`scripts/fetch-speech-engine`; `.cargo/config.toml`
points the build at it). Other agents were running on the machine (load average 5.9
when the cold runs began, 9.8 when the last runs began), so read the cold
numbers to a few seconds, and the incremental ones as a floor.

task-199 re-runs these commands verbatim and adds `## After` beside this
section. Each is run twice and the second kept: the first of anything leaves
something cold behind. Wall time is `time`'s, or the `Finished` line's.

```sh
# cold: hotline-core only (its dependencies stay built)
cargo clean -p hotline-core && cargo build -p hotline-core --timings
scripts/timings-summary
cargo clean -p hotline-core && cargo test -p hotline-core --no-run --timings
scripts/timings-summary

# incremental: touch one file, build, repeat; then the same with tests
touch crates/hotline-core/src/voice/dispatcher.rs && cargo build -p hotline-core
touch crates/hotline-core/src/voice/dispatcher.rs && cargo test -p hotline-core --no-run
touch crates/hotline-core/src/store.rs && cargo build -p hotline-core
touch crates/hotline-core/src/store.rs && cargo test -p hotline-core --no-run
touch crates/hotline-core/src/contract/command.rs && cargo build -p hotline-core
touch crates/hotline-core/src/contract/command.rs && cargo test -p hotline-core --no-run

# incremental with a real edit: a new private function appended to the file,
# undone after, so the tree is left clean
printf '\n#[allow(dead_code)]\nfn timing_probe() -> u32 { %s }\n' "$RANDOM" >> crates/hotline-core/src/voice/dispatcher.rs && cargo build -p hotline-core
printf '\n#[allow(dead_code)]\nfn timing_probe() -> u32 { %s }\n' "$RANDOM" >> crates/hotline-core/src/voice/dispatcher.rs && cargo test -p hotline-core --no-run
# ... and the same two for store.rs and contract/command.rs
git checkout -- crates/hotline-core/src/voice/dispatcher.rs crates/hotline-core/src/store.rs crates/hotline-core/src/contract/command.rs
```

`--timings` writes `target/cargo-timings/cargo-timing.html`; the summary is
what `scripts/timings-summary` prints from it, and only the summary is kept.
Its `OWN` set names `hotline-core`; task-199 adds the new crates there, so
their units are listed apart from the dependencies.

| What | `cargo build -p hotline-core` | `cargo test -p hotline-core --no-run` |
|---|---|---|
| Cold, hotline-core only | 41.7 s | 78.8 s |
| Cold, and the dependencies not yet built (first run) | 71.1 s | 90.5 s |
| Touch `voice/dispatcher.rs` | 4.9 s | 12.4 s |
| Touch `store.rs` | 5.0 s | 12.4 s |
| Touch `contract/command.rs` | 5.0 s | 12.6 s |
| Edit `voice/dispatcher.rs` | 5.0 s | 12.4 s |
| Edit `store.rs` | 5.0 s | 12.6 s |
| Edit `contract/command.rs` | 5.0 s | 12.6 s |

Two things about reading it. The first `cargo build` after a cold
`cargo test --no-run` recompiled the library in full (41.7 s, not 4.9 s), which
is why every figure is the second run. And the incremental figures do not
depend on the file or on touching against editing: across these three files and
both ways of changing them the build is 5 s and the test build 12.4 to 12.6 s.
That floor belongs to the crate and not to the file (what it is made of was not
taken apart here), and it is what a smaller crate should lower.

### hotline-core against its dependencies

From the summaries of the cold runs (`scripts/timings-summary`):

- **Build, dependencies not yet built**: 71.0 s wall, 615 units. The 611
  dependency units add up to 287 s of compile work, spread over 18 cores.
  `hotline-core`'s library is one unit of 40.2 s (26.5 s front end, 13.7 s code
  generation) that starts 29.7 s in, when its dependencies are ready, and ends
  the build; it is 57% of the wall time and the part that does not get faster
  with more cores. The longest dependency is
  aws-lc-sys's build script at 17.2 s.
- **Build, dependencies cached**: 41.7 s wall, four units: the library at
  40.2 s (26.6 s front end) and the three binaries at 0.6 to 1.2 s. The front
  end is two thirds of it.
- **Test, dependencies cached**: 78.8 s wall, 25 units. The library compiled
  with its tests, `hotline_core` "lib" (test), is 78.5 s, which is the wall
  time: the library itself (41.6 s) and the thirteen integration test
  binaries (1.5 to 6.2 s each) all finish under it. So a cold test build is the
  test-profile library alone, about twice the plain one, and the integration
  tests are not where the time goes.
- **Test, dev-dependencies not yet built**: 90.4 s, 44 units; the 19
  dev-dependency units add 22.8 s of work, before the library can start.

What a split can and cannot do with that: it can shorten the part of the
critical path that moves out and let the moved crates compile alongside one
another, and `hotline-core` still starts after the slowest of them. Of the
163,049 lines, about 23,100 (14%) move across these five stages (contract
4,577, support 2,524, websearch 6,232, log/store/sent 5,150, tools 4,638, tests
inside the files included). So the most the cold build can lose is about a
seventh of the library's 40 s, less the time the moved crates take before
`hotline-core` can start, and an edit in the other 86% should cost what it
costs now, 5 s and 12.4 s. That is the expectation task-199 checks, not a
result. The one row that may get worse is `contract/command.rs`: everything
downstream of `hotline-contract` rebuilds when it changes, which is by design,
and why that crate is first and should change least. Any gain beyond this
rests on the next stage, the core, and task-199 says so in its numbers.

## The module map

`scripts/module-map`, run from the repository root with no arguments, reads
the module tree from `crates/hotline-core/src/lib.rs` and lists every reference
from one top-level module to another: `crate::X`, brace imports
(`use crate::{a, b::c}`) and `super::` chains that climb to the root. Comments,
strings and char literals are blanked first (so `contract`'s one mention of
`crate::paths` is a doc link, and is not an edge), and `#[cfg(test)]` code,
including files such as `tests.rs` that are only declared under it, is counted
apart from production code, because a test-only edge costs a crate a
dev-dependency and not a dependency. Re-run it after each stage; it also
prints the layers, the cuts and the test-only edges. As of 3987d089 the edges
are (production references, then `+N t` for test code):

```
capabilities -> contract(4), imagegen(2), log(1), models(3), room(1), vault(1), voice(5)
computer -> contract(9), mcp(1), paths(1), process_windows(3), test_support(0+1t), vault(2), wire(1)
contract: (none)
credentials: (none)
desk -> capabilities(1), contract(42+3t), credentials(6), driver(4+2t), imagegen(1), import(2), linked(1), log(1+1t), mcp(2+3t), models(15), providers(19), remote(3), room(7+1t), session(3+1t), skills(1), vault(3+1t), voice(12+2t), websearch(2), wire(1+1t)
driver -> computer(1), contract(25+11t), fence(2), images(1), log(0+3t), mcp(5+9t), models(4), paths(1), pricing(4), process_windows(3), providers(15+6t), room(0+1t), session(16+11t), tools(5+2t), vault(2), voice(6+4t), wire(1)
fence: (none)
imagegen -> contract(2), credentials(2+4t), driver(0+1t), providers(4+1t), session(1), vault(1)
images: (none)
import -> contract(2+5t), log(4+2t), mcp(1), paths(3+1t), room(3+3t), session(1+1t), store(1+1t), vault(1)
link_preview -> contract(1)
linked -> contract(1), log(1)
log -> credentials(2), paths(15)
log_file -> paths(1)
mcp -> computer(0+2t), contract(9+14t), driver(5+2t), fence(1), log(0+2t), paths(0+1t), process_windows(3), room(2), session(12+6t), store(2), vault(3+1t), websearch(5+5t), wire(2)
models -> contract(4+1t), imagegen(1), pricing(0+1t), session(1), voice(1)
paths -> linked(1)
pricing -> contract(1), models(1), session(2)
process_windows: (none)
providers -> contract(1), credentials(2+4t), driver(1), log(0+2t), models(1), process_windows(1), vault(4+1t)
push -> contract(1+1t), remote(1)
remote -> computer(1), contract(9+28t), credentials(2+2t), desk(0+1t), driver(0+1t), import(0+1t), linked(2+4t), log(1+7t), mcp(0+1t), paths(0+1t), room(0+5t), session(0+3t), test_support(0+1t), wire(5+2t)
room -> contract(6+6t), log(1), mcp(1), paths(0+1t), voice(0+1t), websearch(1)
room_lock: (none)
sent -> contract(2), images(1), log(1), paths(1), thumbs(2)
session -> computer(71+28t), contract(61+75t), credentials(1+5t), desk(0+2t), driver(26+24t), fence(11), imagegen(2+4t), images(3), linked(3+2t), log(16+14t), mcp(7+12t), models(1+1t), paths(6+8t), process_windows(1), push(6+2t), room(17+19t), sent(7+2t), skills(7+3t), spending(1+1t), store(7+12t), test_support(0+7t), thread(6+7t), thumbs(2), tools(4+1t), vault(4+7t), voice(14+25t), websearch(2+2t), wire(13+8t)
skills -> contract(1), paths(2)
spending -> contract(1)
store -> contract(1+1t), log(3+2t), paths(1+3t), sent(2), thread(1)
test_support: (none)
thread -> contract(4), log(2), paths(1), room(1), session(1)
thumbs -> images(1)
tools -> contract(2), driver(1+2t), process_windows(2)
vault -> contract(6), credentials(7+3t), log(1), mcp(6+5t), models(4), providers(1), room(3+7t), session(1), websearch(1)
voice -> contract(24+30t), credentials(3+23t), desk(2+21t), driver(11), log(5+7t), mcp(2), models(7+1t), paths(1), pricing(1+1t), providers(3+8t), room(10+8t), session(7+4t), store(0+2t), test_support(0+1t), thread(0+9t), vault(4), wire(14+6t)
websearch -> contract(1+1t)
wire -> computer(0+3t), contract(97+81t), credentials(0+18t), desk(0+20t), driver(4+5t), import(1+3t), link_preview(2), linked(3+3t), log(11+7t), mcp(4+2t), models(5+8t), paths(3+2t), remote(15+2t), room(8+11t), sent(2), session(5+5t), skills(4), spending(0+2t), store(3+1t), test_support(0+1t), thread(2), vault(0+2t), voice(9+4t), websearch(6)
```

### The leaves

Modules that name no other module: `contract`, `credentials`, `fence`,
`images`, `process_windows`, `room_lock` (and `test_support`, which is
`cfg(test)`). One layer up, naming only those: `link_preview`, `spending`,
`thumbs` (→ `images`) and `websearch` (→ `contract`). `skills` and `log_file`
name only `paths`, and `sent` names `images`, `thumbs`, `log`, `paths` and
`contract`, so they are leaves of the leaves once `paths` is.

That is the epic's list with two corrections the script makes: `paths`, `log`
and `linked` are one cycle (`log` → `paths` is 15 sites, and `log` → `credentials`
is no cut at all, `credentials` being a leaf), so `skills` and `log_file`,
which the epic counts as leaves, are only once `paths` is out of it.

### The one-link cuts

A module off the leaves by exactly one edge to one module (the script's "One-link
cuts" section): `linked` → `log` (1 site), `log` → `paths` (15), `paths` →
`linked` (1), `log_file` → `paths` (1), `skills` → `paths` (2), `push` →
`remote` (1) and `tools` → `driver` (1). The first three are one cycle, and
cutting `paths` → `linked` (cut 1 above) frees all of it. `store` → `thread`
is not in the list because `store` also names `log`, `sent` and `paths`, which
are off the leaves until that cut; after it `thread` is the only thing between
`store` and the leaves, which is why task-197 follows task-196.

With the three cuts made the script prints (its "Layers after the planned
cuts"): layer 0 `contract`, `credentials`, `fence`, `images`, `paths`,
`process_windows`, `room_lock`; layer 1 `log`, `log_file`, `thumbs`, `tools`,
`websearch`, `skills`, `spending`, `link_preview`; layer 2 `linked`, `sent`;
layer 3 `store`; and above them, strongly connected, the core. No stage moves a
module that names anything in the core, and the map shows the moved sets have
no test-only edge into it either (a moved crate would otherwise need
`hotline-core` as a dev-dependency, which is a cycle).

### The strongly connected core

Twenty modules reach one another today; with the three cuts made, eighteen
do: `capabilities`, `computer`, `desk`, `driver`, `imagegen`, `import`, `mcp`,
`models`, `pricing`, `providers`, `push`, `remote`, `room`, `session`,
`thread`, `vault`, `voice`, `wire` (`store` and `tools` are the two that leave).
`boot` sits above it and names `desk`, `remote`, `wire`, `credentials` and
`room_lock`. The heaviest edges, in production references, are
`session` → `computer` (71), `session` → `driver` (26), `desk` → `providers`
(19), `session` → `room` (17), `driver` → `session` (16), `desk` → `models`,
`driver` → `providers` and `wire` → `remote` (15 each), `session` → `voice`
and `voice` → `wire` (14 each). `session` is named by `driver`, `mcp`, `voice`,
`vault`, `thread`, `import`, `models`, `pricing` and `imagegen`, which is what
makes it the middle of the cycle.

Splitting it needs trait seams, which the map cannot design. This ADR leaves
them to task-199's plan, and sets two conditions on it: it follows
standards.md (no single-implementation trait to break `session`↔`driver`,
`session`↔`voice` or `mcp`↔`session`), so a seam is a boundary that something
else also implements (the wire already talks to a `RoomHandle`), and each is
judged by the numbers above. A core split that does not shorten the front end
of the largest crate does not earn its crate.

## After

Measured on 2026-10-19 on `release/0.41.0` at fd6ddfd1, with all five crates
landed, on the same machine, toolchain and profile as Before, in a fresh
worktree with an empty `target/` and the speech-engine archive fetched first.
Other agents were running again: the load average was 7 to 11 during the cold
runs, 7 to 10 during the touches and 11 to 15 during the edits, against 5.9 to
9.8 for Before, so read the edit rows as a ceiling.

The commands are Before's, with two changes the moves forced. The files
`store.rs` and `contract/command.rs` no longer exist in `hotline-core`: the
store's root is now `crates/hotline-store/src/lib.rs` and the command enum
is `crates/hotline-contract/src/command.rs`, so those are what was touched
and edited. And the probe line as written appends a second `timing_probe`
on the second run, which does not compile; here the file was restored
(`git checkout`) before each append, so every run is one fresh edit. Each was
run twice and the second kept, as before. Two rows are new: a cold build of
all six crates (`cargo clean` with `-p` for each, then the same build), which
compiles the same source lines Before's cold row did, and an edit in a moved
crate tested by that crate alone. `scripts/timings-summary` now counts the
five crates as the workspace's own units, beside `hotline-core`, and prints
when each unit started.

| What | build, before | build, after | test build, before | test build, after |
|---|---|---|---|---|
| Cold, and the dependencies not yet built (first run) | 71.1 s | 61.8 s | 90.5 s | 75.7 s |
| Cold, hotline-core only | 41.7 s | 33.5 s | 78.8 s | 65.1 s |
| Cold, hotline-core and the five crates | | 37.2 s | | 70.2 s |
| Touch `voice/dispatcher.rs` | 4.9 s | 3.2 s | 12.4 s | 8.6 s |
| Touch the store's root | 5.0 s | 3.4 s | 12.4 s | 8.7 s |
| Touch `command.rs` | 5.0 s | 5.0 s | 12.6 s | 10.6 s |
| Edit `voice/dispatcher.rs` | 5.0 s | 3.4 s | 12.4 s | 9.0 s |
| Edit the store's root | 5.0 s | 3.8 s | 12.6 s | 9.3 s |
| Edit `command.rs` | 5.0 s | 5.2 s | 12.6 s | 10.8 s |
| Edit the store's root, `cargo test -p hotline-store --no-run` | | | (12.6 s, the core's) | 0.7 s |
| Edit `command.rs`, `cargo test -p hotline-contract --no-run` | | | (12.6 s, the core's) | 2.2 s |

### What got faster

- **The library itself.** `hotline-core`'s library is one unit of 31.8 s
  (19.7 s front end, 12.0 s code generation), against 40.2 s (26.6 s, 13.7 s).
  That is 21% off the unit for 15% of its lines (163,049 then; 138,788 now,
  9,962 more in its tests/), and the front end lost 26%. The contract is why
  the share is larger than the lines: its serde and ts-rs derives make 4,928
  lines cost 3.5 s of front end on their own, five times what the core's
  lines cost apiece. The test-profile library is 64.2 s, against 78.5 s.
- **A cold build.** 61.8 s against 71.1 s from nothing, and 33.5 s against
  41.7 s with the dependencies cached. In the first build the five crates
  compile alongside the external dependencies and are done before
  `hotline-core` can start (29.3 s in, against 29.7 s), so they cost the
  critical path nothing there; with the dependencies cached the contract is
  on it, 3.8 s, which is the difference between the core-only row and the
  all-six row.
- **An edit to the core, which Before did not expect.** Before's reading was
  that a 5 s build and a 12.4 s test build were a floor belonging to the crate
  and not to the file. The floor fell by about a third (3.2 s and 8.6 s for a
  touch) though the file edited did not move, so it does scale with the
  crate. What it is made of is still not taken apart; the likeliest part is
  the incremental state rustc loads and checks for the whole crate on every
  build, which is proportional to its size.
- **An edit to a moved crate, tested alone.** The store's own tests build
  in 0.7 s and the contract's in 2.2 s, where testing that code meant the
  core's 12.6 s test build before. That is a different command, not a faster
  one, and it is what a crate gives for code with tests of its own.

### What did not

- **An edit to the contract, built.** 5.0 s before and after: every crate
  above it rebuilds, which is by design and why the contract changes least.
  Its test build fell (12.6 s to 10.6 s) with the core's floor.
- **The time before the core starts.** A first build still waits about 29 s
  for external dependencies (aws-lc-sys's build script alone is 16.9 s)
  before `hotline-core` begins. No workspace split touches that.
- **The test build's shape.** A cold `cargo test -p hotline-core --no-run`
  is still the test-profile library and nothing else (64.2 s of 65.1 s); the
  integration-test binaries (0.6 to 6.1 s each) finish under it.
- **Most of the direct dependencies.** Counted by name across its manifest's
  `[dependencies]` and target tables, `hotline-core` names 51 external crates
  and the five new ones, against 60 then (Before's 58 was counted another
  way). Nine left with the moves (cap-std, globset, grep-regex,
  grep-searcher, ignore, keyring, roxmltree, ts-rs, whatlang); the rest are
  used by the core as well.

No caller moved. `hotline-core` re-exports each crate under its old module
name (`pub use hotline_contract as contract`, `hotline_support::{credentials,
log_file, paths, room_lock}` and the private `fence`, `images`, `thumbs`,
`process_windows`, `hotline_store::log` and `sent`, `hotline_store as store`,
`hotline_tools as tools`, `hotline_websearch as websearch`), and `driver` and
`thread` re-export the types that left them, so every `crate::X` path in the
core and every `hotline_core::X` path in the app, the CLI and the harnesses
still holds. About 870 references in the core's own source go through those
re-exports (570 of them `crate::contract`), and some 70 more in its tests/,
the app and the CLI.

### Sharing a test fixture down the crates

`hotline-websearch`'s mock providers are public only under its
`test-support` feature, which turns on its optional `axum`, and
`hotline-core` enables it in `[dev-dependencies]` alone. With the workspace's
resolver 3, a feature a dev-dependency enables is off in any build that does
not build tests, so the release app has no mock in it. That is the pattern
for later crates: a fixture lives in the crate whose code it fakes, under a
`test-support` feature that only adds code (never changes the behaviour of
what is already there), with its extra dependencies optional, and turned on
only from the consumer's `[dev-dependencies]`. A separate fixtures crate is
the second-caller rule's answer, once two crates above share one fixture,
not before. A fixture that needs the core (a `Desk`, a `Room`) cannot be
shared down at all; the test that uses it lives in the core.

### The module map after

`scripts/module-map` reads the core's module tree, so the fourteen modules
that became crates (`contract`, `credentials`, `fence`, `images`, `log`,
`log_file`, `paths`, `process_windows`, `room_lock`, `sent`, `store`,
`thumbs`, `tools`, `websearch`) are not in it, and a reference through a
re-export is no longer an edge: the map is the core's internal shape alone.
The planned cuts are made, so it no longer prints layers after them. As of
fd6ddfd1:

```
boot -> desk(1), remote(1), wire(1)
capabilities -> imagegen(2), models(3), room(1), vault(1), voice(5)
computer -> mcp(1), test_support(0+1t), vault(2), wire(1)
desk -> capabilities(1), driver(4+2t), imagegen(1), import(2), linked(1), mcp(2+3t), models(15), providers(19), remote(3), room(7+1t), session(3+1t), skills(1), vault(4+1t), voice(12+2t), wire(1+1t)
driver -> computer(1), mcp(5+9t), models(4), pricing(4), providers(15+6t), room(0+1t), session(16+11t), vault(2), voice(6+4t), wire(1)
imagegen -> driver(0+1t), providers(4+1t), session(1), vault(1+2t)
import -> mcp(1), room(3+3t), session(1+1t), vault(1)
link_preview: (none)
linked: (none)
mcp -> computer(0+2t), driver(5+2t), room(2), session(12+6t), vault(3+1t), wire(2)
models -> imagegen(1), pricing(0+1t), session(1), voice(1)
pricing -> models(1), session(2)
providers -> driver(1), models(1), vault(4+3t)
push -> remote(1)
remote -> computer(1), desk(0+1t), driver(0+1t), import(0+1t), linked(2+4t), mcp(0+1t), room(0+5t), session(0+3t), test_support(0+1t), wire(5+2t)
room -> mcp(1), voice(0+1t)
session -> computer(71+28t), desk(0+2t), driver(26+24t), imagegen(2+4t), linked(3+2t), mcp(7+12t), models(1+1t), push(6+2t), room(17+19t), skills(7+3t), spending(1+1t), test_support(0+7t), thread(6+7t), vault(4+7t), voice(14+25t), wire(13+8t)
skills: (none)
spending: (none)
test_support: (none)
thread -> room(1), session(1)
vault -> mcp(6+5t), models(4), providers(1), room(3+7t), session(1)
voice -> desk(2+21t), driver(11), mcp(2), models(7+1t), pricing(1+1t), providers(3+8t), room(10+8t), session(7+4t), test_support(0+1t), thread(0+9t), vault(4+5t), wire(14+6t)
wire -> computer(0+3t), desk(0+20t), driver(4+5t), import(1+3t), link_preview(2), linked(3+3t), mcp(4+2t), models(5+8t), remote(15+2t), room(8+11t), session(5+5t), skills(4), spending(0+2t), test_support(0+1t), thread(2), vault(0+2t), voice(9+4t)
```

Layers: the leaves are `link_preview`, `linked`, `skills`, `spending` (and
`test_support`); above them the same eighteen modules Before predicted,
strongly connected; `boot` on top. Two things are new. `linked` is a leaf,
because its one edge went to `log`, which is a crate now. And the script finds
a new one-link cut beside `push` → `remote`: **`room` → `mcp`**
(`room.rs:283`), where the roster's settings fold calls
`mcp::normalize_servers`, a pure function over the settings' JSON. Cut that and
`room`, which `session`, `voice`, `wire`, `desk`, `vault`, `mcp` and `import`
all read, names nothing in the core.

## Next: splitting the core

A proposal for George, not started. It asks what would let `voice`, `remote`
and `mcp` leave `hotline-core`, what each costs, and in what order, judged by
two rules this ADR already set: no single-implementation trait to break the
`session`↔`driver`, `session`↔`voice` or `mcp`↔`session` cycles
(standards.md), and a crate that does not shorten the core's front end (now
19.7 s) does not earn itself.

### The seam that exists

The wire talks to the room through `RoomHandle` (`wire/room_handle.rs:29`), a
trait of about 110 methods, most with a default "this room cannot" body. It is
a real boundary: `Desk` implements it, and so do three test rooms
(`CoreHandle` and `Quiet` in the wire's tests, `Computered` in remote's). It
cannot move below anything yet, because five of its signatures name the core:
`voice()` returns `voice::Calls`, `remote()` returns `remote::Remote`,
`links()` returns `linked::Links`, and two methods return
`driver::auth::AuthStatus` and `import::Report`. Moving the trait down means
those three accessors leave it (the wire is handed the calls, the remote and
the links when it is built, as the desk already holds them) and the two
reports move down as the plain data they are. That is the one piece of work
every stage below shares.

### First, moves with no seam

Before any trait, the pieces the three modules reach for that hold no live
state can go down as epic-16's did: `room` with `mcp::normalize_servers`
beside it (the cut above), `linked` (700 lines, now a leaf), `session::ledger`
(361 lines, contract only), the `session::ProviderAuth` enum,
`wire::same_secret`, `driver::clip` and `failure::redact_text`,
`desk::install_crypto_provider`, `computer::HOME_MOUNT`, and `voice`'s
`ledger`, `metering` and `spoken`, which name only the contract, the log and
each other. With `RoomHandle`, the roster fold and `linked`, they would make a
crate of the room as everything above the core sees it. This is cheap, the
same work the five moves were, and it gains little time by itself (about
5,000 lines with their tests); its point is that every stage after it needs it.

### `mcp`: the client leaves, the server stays

`mcp` is two things. The client (`mcp.rs`, `oauth.rs`, `tool.rs`, about 2,500
lines without their tests) reaches the core only for the vault (six methods:
the credential store, the token lock, the OAuth registration read and saved,
a server's secret, and resolving a server), the capability lease (already in
`hotline-tools`) and `session::ledger` (moved down first). The seam there is
rmcp's own `CredentialStore` trait, which the vault already implements and
rmcp implements too; the rest becomes values the core resolves and hands in,
with the saving of a registration as the one call back. The server
(`TeammateTools`, holding a `Weak<Room>` and calling some 25 of its methods)
is the `mcp`↔`session` cycle the standards keep whole, so it stays in the core.

Cost: small; one crate, a few signatures from `&Vault` to resolved values,
and its tests that use the ledger or the computer's server id follow those
down. Gain: small, 2% of the core's lines off the front end; no dependency of
its own leaves (`rmcp` and `axum` are used across the core).

### `voice`: the engines leave, the calls stay

`voice` is the largest prize, 17,900 production lines, and the hardest to
move whole: its dispatcher runs the wire's whole command table
(`wire::commands::run`), and its calls report to the room through
`Weak<Room>` (`call_began`, `call_said`, `call_posted`, `call_ended`,
`voice_push`). A trait for the room's side would have one implementation and
is the `session`↔`voice` cycle the standards keep whole, and an event hop in
its place trades a direct call for a moving part. So moving `voice` whole is
not proposed.

What can leave is the engines: `speech/` and `realtime/`, about 7,700
production lines and 3,400 of tests. Their seams already exist and already
have several implementations: `Speech` (`speech.rs:139`) has five (the
front's, local, Google, the OpenAI shape, xAI) and `Realtime`
(`realtime.rs:226`) two. Outside `voice` they reach only for credentials (a
crate already), `ProviderAuth` and `install_crypto_provider` (moved down
first), `models::is_custom`, three credential helpers in `providers`
(ChatGPT's refreshed sign-in, OpenRouter's key, xAI's bearer), and the vault,
which the dispatcher could resolve and hand in, as for `mcp`. They take
`voice`'s own dependencies with them: sherpa-onnx, symphonia, webrtc, rtc and
opus.

Cost: medium; the credential helpers either move down with the engines or
become values handed in, and the engines' tests that sign in through
`providers` follow them. Gain: the largest of the three, about 5.5% of the
core's lines and 3,400 lines of tests out of its test build, and the engine code that changes on its own (a new speech
provider) is tested alone, as the store's is now.

### `remote`: last, and only with the wire's help

`remote` reaches the fewest modules (production references only to
`RoomHandle`, `linked`, two wire functions and a constant), and is the most
work to move, for three reasons. It needs `RoomHandle` below it, the shared
work above. It calls into the wire to serve a socket once it has accepted
one (`wire::federation` at `link.rs:557`, `wire::seated_phone_v2` at
`server.rs:497`), and the wire names it back fifteen times (`remote::Phone`,
`WireRole`, the commands that drive a `Remote`). The plain answer is for
remote to hand the accepted, authenticated socket back to the core to serve,
rather than call into the wire: a change in control flow, not a trait, and
the riskiest one here, since it is the path a phone and a linked desk take.
And half of `remote` is its tests (6,576 of 13,380 lines), most of which open
a real `Desk` and `Door`; those would move up into the core's tests.

Cost: large. Gain: about 6,800 production lines (5%), and its own
dependencies leave the core's list (snow, rcgen, hyper, hyper-util,
http-body-util, if-addrs, socket2, qrcode). `push` → `remote` (one call to
`remote::push_targets`) is cut on the way.

### Step 1, done: `hotline-room`

The name reads right: what it holds is the room as the code above the core
sees it, the roster, the settings, the links and the seam the wire calls,
and the alternatives (`hotline-roster`, `hotline-plain`) name a part of it
or nothing. It stands on `hotline-contract`, `hotline-support`,
`hotline-store` and `hotline-websearch` (the `webSearch` setting is checked
by websearch's own rule), and on nothing in the core.

Each moved file, by its old path in `crates/hotline-core/src/` and its new
one in `crates/hotline-room/src/`:

| Old path (`hotline-core/src/`) | New path (`hotline-room/src/`) | The core still reaches it as |
|---|---|---|
| `room.rs` | `room.rs` | `crate::room` |
| `linked.rs` | `linked.rs` | `crate::linked` |
| `session/ledger.rs` | `tool_ledger.rs` | `crate::session::ledger` |
| `wire/room_handle.rs` | `room_handle.rs` | `crate::wire::{RoomHandle, AuthStatus}` |
| `voice/ledger.rs` | `voice/ledger.rs` | `crate::voice::ledger` |
| `voice/metering.rs` | `voice/metering.rs` | `crate::voice::metering` |
| `voice/spoken.rs` | `voice/spoken.rs` | `crate::voice::spoken` |
| `voice/settings.rs` | `voice/settings.rs` | `crate::voice::settings` |

And the pieces that moved out of files that stayed:

| Item, from | To (`hotline-room/src/`) | The core still reaches it as |
|---|---|---|
| `mcp::normalize_servers` and its four private helpers, from `mcp.rs` | `mcp_servers.rs` | `crate::mcp::normalize_servers` |
| `session::ProviderAuth`, from `session.rs` | `provider_auth.rs` | `crate::session::ProviderAuth` |
| `wire::same_secret`, from `wire.rs` | `secret.rs` | `crate::wire::same_secret` |
| `failure::redact_text`, from `driver/failure.rs` | `secret.rs` | `crate::driver::failure::redact_text` |
| `driver::clip`, from `driver.rs` | `text.rs` | `crate::driver::clip` |
| `desk::install_crypto_provider`, from `desk.rs` | `tls.rs` | `crate::desk::install_crypto_provider` |
| `computer::HOME_MOUNT`, from `computer.rs` | `computer.rs` | `crate::computer::HOME_MOUNT` |
| `NOT_LINKED`, from `wire/socket.rs` | `room_handle.rs`, private | (its one user is the trait) |
| `sentences`, `take_sentences`, `spoken_lengths`, `SPOKEN_PIECE`, from `voice/synthesis.rs` | `voice/spoken.rs` | `crate::voice::spoken::…` |

`import::Report` was already the contract's `Report` (task-211), so it
did not move; `AuthStatus` moved with the trait. Two things moved that the
plan above did not name, because the plan's "the contract, the log and each
other" was wrong for two of voice's three. `metering` reads the budgets'
limits through `voice::settings::VoiceSettings`, so `settings.rs` came too:
it names only the contract, the log and `room`, and GPT-Live's eight voices,
which were a constant in `realtime/live.rs` used by nothing else, are now
written inline in it beside Grok's and GPT Realtime's. And `spoken` cuts a
reply into sentences with `synthesis`'s splitter, which is four pure
functions over a string, so they moved into `spoken.rs` and the core's
callers (`synthesis`, `turn`, `dispatcher` and the voice tests) name
`spoken::sentences` now. The tests went with their code: 101 in the crate,
three of them `mcp.rs`'s tests of the server-list reading.

What stayed: `wire::Parts` and `wire::Agents`, which name `voice::Calls`,
`remote::Remote` and `linked::Links` and so live with the wire; the three
test rooms (`CoreHandle`, `Quiet`, `Computered`), which implement the trait
from the core's tests as `Desk` does from the core. The moved items that
were `pub(crate)` are `pub` now (the room's fold helpers, `linked`'s
`Record` and the `Links` methods, `spoken`'s types), which the core alone
sees: nothing else depends on `hotline-room`. `Budget::charge`, a test's
way to spend without a reservation, is public only under the crate's
`test-support` feature, which the core turns on in `[dev-dependencies]`,
as websearch's mock is.

With the room out, `scripts/module-map` lists 22 modules, 17 of them
strongly connected (`room` and `linked` left; `store` and `tools` had
before). `remote`'s production edges are now `computer` (1) and `wire` (5)
and nothing else; and the script finds a new one-link cut, `thread` → `session`
(`thread/store.rs:203`). The crate is about 6,400 lines with its tests,
against the 132,700 left in the core's `src/`. No timings were taken: the
plan above says this step's point is the steps after it, and step 2's
measurement will include it.

### Step 2, done: `hotline-mcp`

The client is `hotline-mcp`. It stands on `hotline-contract`,
`hotline-support` (a stdio server's Windows job), `hotline-tools` (the
lease a granted tool checks) and `hotline-room` (the tool ledger and
`normalize_servers`), and on nothing in the core. The plan's seam held: no
trait was added, and the vault is reached only through rmcp's
`CredentialStore`, which it already implemented, and values.

Each moved file, by its old path in `crates/hotline-core/src/` and its new
one in `crates/hotline-mcp/src/`:

| Old path (`hotline-core/src/`) | New path (`hotline-mcp/src/`) | The core still reaches it as |
|---|---|---|
| `mcp.rs`, all but `pub mod server` | `lib.rs` | `crate::mcp::{McpServer, HttpAuth, connect, grant, servers, …}` |
| `mcp/oauth.rs` | `oauth.rs` | `crate::mcp::{manager_for_server, McpOAuthRegistration}`, and the core's `mcp::McpOAuth` |
| `mcp/tool.rs` | `tool.rs` | `crate::mcp::{McpTool, CallError, CallContent, CallImage}` |

And the piece that moved out of a file that stayed:

| Item, from | To (`hotline-mcp/src/`) | The core still reaches it as |
|---|---|---|
| `vault::McpOAuthRegistration`, from `vault.rs` | `oauth.rs` | `crate::mcp::McpOAuthRegistration` |

The core's `mcp.rs` is now the server's parent (`mcp/server.rs`,
`TeammateTools`, unchanged), the re-exports, and the vault's side of the
client, about 170 lines. Where the client took `Arc<Vault>` it now takes
what the vault keeps for one server, read by the core and handed in:
`OAuthVault` (the vault's `McpCredentialStore`, the server's token lock, and
its saved registration or the error reading it) and `Vaulted` (that, or a
bearer or header server's saved token). `connect_granted` takes each
granted server as a `Granted`, already resolved by the vault's
`resolve_mcp_server`, or with the reason it could not be, which fails that
server alone as before; `connect_with_capability_and_vault` in the core
builds them, so the drivers and the voice front did not change.
`McpOAuthService` holds no vault: `start` takes the `OAuthVault` and a
closure that saves a registration (the one call back), a pending login
keeps the token lock it was started with for its callback, `status` takes
the `Vaulted`, and `sign_out` clears through the store's
`CredentialStore::clear`, which deletes the server's whole vault record as
`clear_mcp_oauth` did. The core's `McpOAuth` pairs the service with the
vault, so the desk's four sign-in commands read as before. Values are read
before the client needs them rather than when it does, so a granted list's
records are read before its first handshake rather than one server at a
time; an error read early is kept as a value and returned at the step that
used to read it, so a failure reads the same sentence in the same order.
The items the core names that were `pub(crate)` (`validate_http_url`,
`public_servers` and the rest, `Presented`, `manager_for_server`, the
service) are `pub` in the crate and re-exported `pub(crate)`.

What a teammate can reach did not change. A granted tool still checks its
session's lease before calling its server and the connect loop still stops
on a revoked one; the lease is `hotline-tools`' and its fields stay
private, so the new crate can clone and check a lease but not forge one.
The vault's records stay bound to the server's id and URL, the tokens are
still loaded by rmcp through the vault's store for one operation at a
time, and the crate cannot open the vault at all. `HttpAuth::Bearer` was
already `pub`.

The tests went with their code where they could: 22 in the crate (the
grant, slug and prefix rules, the env and header refusals, a scripted
server over an in-memory pipe, the ledger rows a dead transport writes,
the revoked lease, and the callback page's escaping and URL checks). Three
groups stayed in the core because they need the core, by the rule in
[Sharing a test fixture](#sharing-a-test-fixture-down-the-crates): the six
OAuth flow tests and their loopback authorization server, which sign in
through a real `Vault`, are `mcp/tests.rs` and now drive `McpOAuth`, so
they prove the core's reading too; `how_to_use_says_granted_tools_are_named_server_tool`
reads the server's `HOW_TO_USE` and moved into `mcp/server.rs`; and
`a_computer_server_writes_its_eight_tools_on_the_ledger_under_origin_computer`,
which asserts what `driver::rig::publish_ledger` writes under
`computer::SERVER_ID`, is in `driver/rig/tests.rs` and serves its eight
tools over loopback HTTP to the public `mcp::connect`, where it used to
build tools from the client's private parts, so `Connections::for_test`
is gone. `tests/mcp.rs`, the desk harness, did not change.

`scripts/module-map`: `mcp -> driver(4), session(11+3t), vault(1+1t), wire(2)`,
against `computer(0+2t), driver(5+2t), session(12+6t), vault(3+1t), wire(2)`
before it; still 22 modules, 17 strongly connected, because the server is
what ties `mcp` to `session`. The crate is 3,159 lines with its tests
(about 2,440 without), and the core's `src/` is 129,734 lines, against
132,678.

Measured on 2026-10-19 on `dispatch/task-209` at 51cd5108, with steps 1
and 2 in, on the same machine, toolchain and profile as After, by After's
commands, each run twice and the second kept. Other agents were running:
the load average was 5 to 7 during the cold build and 7 to 13 after it, so
read the test-build row as a ceiling.

| What | After (five crates) | Steps 1 and 2 |
|---|---|---|
| Cold, hotline-core only, `cargo build` | 33.5 s | 31.6 s |
| `hotline-core`'s library unit (front end, code generation) | 31.8 s (19.7 s, 12.0 s) | 30.1 s (18.9 s, 11.2 s) |
| Cold, hotline-core only, `cargo test --no-run` | 65.1 s | 74.5 s |
| Touch `voice/dispatcher.rs`, `cargo build` | 3.2 s | 3.1 s |
| Touch `voice/dispatcher.rs`, `cargo test --no-run` | 8.6 s | 8.4 s |
| Touch `oauth.rs`, `cargo test -p hotline-mcp --no-run` | (8.6 s, the core's) | 0.9 s |

The two steps took about 6.5% of the core's lines and the front end lost
about 4% (0.8 s), the build 1.9 s; the touches moved by less than the
noise. The cold test build is the outlier, nine seconds slower under twice
the load, and not read as a result; it should be taken again on a quiet
machine before step 3 is judged against it. What the step does give is
the client's tests in under a second, alone.

### The re-exports

The five crates' re-exports are about 870 references in the core's source,
and a cleanup that renames them to `hotline_contract::` and the rest buys no
time and touches every file. The proposal is not to do it as a stage, but to
let each module rename its own as it leaves: a module moved out of the core
cannot say `crate::contract` anyway, so that renaming is part of the cost of
`mcp`, `voice` and `remote` above, and the re-exports go when the last caller
in the core does.

### The order, and when to stop

1. The room and the pure pieces down, with `RoomHandle` (moves only; the
   shared precondition).
   Its signatures are cut (task-211): `voice()`, `remote()` and `links()`
   left the trait, and the wire is handed a `wire::Parts` (the calls, the
   Remote held weakly, the links) beside the room handle, in a `wire::Agents`
   with the log, by whoever builds it: `Door::bind`, a Remote's phone seat
   (which adds itself), a call's dispatcher (the calls alone), and the test
   rooms with none. `import` answers the contract's `Report`, and
   `AuthStatus` moved beside the trait rather than into the contract, because
   `agent.auth.poll` is typed by hand in the window and exporting it would
   change `contract.ts`.
   Done (task-208): the crate is `hotline-room`, below.
2. The `mcp` client (the cheapest seam, an external trait).
   Done (task-209): the crate is `hotline-mcp`, below.
3. The `voice` engines (the largest gain, through traits with five and two
   implementations).
4. `remote`, only if the numbers after 3 say the core's front end is still the
   bottleneck, because it is the one stage that changes control flow on a
   network path.

After each, re-run this ADR's commands. The After figures show the incremental
floor follows the crate's size, so each stage's expected gain is roughly its
share of the core's lines off the 3.2 s and 8.6 s floors and the 19.7 s front
end; a stage that does not show that does not get the next one started. What
none of them can change is the 29 s of external dependencies before the core
begins, and the test build being the test-profile library: a smaller core
shortens both of those only in proportion.

## Consequences

- Seven new workspace crates (epic-16's five, `hotline-room` for step 1
  of Next and `hotline-mcp` for step 2), each with the workspace lints, a license and a crate-level doc. `hotline-core` keeps no Tauri.
- The stages share lines (workspace members, `Cargo.lock`, `hotline-core`'s
  `Cargo.toml` and `lib.rs`, standards.md's module map, AGENTS.md), so they land
  in order and alone.
- `pub(crate)` becomes `pub` where an item crosses a boundary. `hotline-core`
  may `pub use` the moved paths while callers move; nothing here widens what a
  teammate can reach, and task-198 states the one place that comes close.
- `ui/src/generated/contract.ts` stays byte-identical, written by an
  unfiltered `cargo test -p hotline-contract`, where the types live now (a
  filtered run truncates it).
