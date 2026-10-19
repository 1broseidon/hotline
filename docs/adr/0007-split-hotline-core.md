# ADR 0007: Split hotline-core into crates, leaves first

- Status: Proposed (0.41, landing over epic-16)
- Date: 2026-10-19
- Supersedes: the "No new crates" line of [standards.md](../standards.md#module-layout),
  once task-196 rewrites it. The line after it, "Do not break the
  `session`↔`driver`, `session`↔`voice` and `mcp`↔`session` cycles with
  single-implementation traits", stands and shapes the last section.
- Tracking: epic-16 (task-193 this baseline, task-194 `hotline-contract`,
  task-195 `hotline-websearch`, task-196 `hotline-support`, task-197
  `hotline-store`, task-198 `hotline-tools`, task-199 the timings again and
  the plan for the core)

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

## Layers (production edges)
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

## Consequences

- Five new workspace crates, each with the workspace lints, a license and a
  crate-level doc. `hotline-core` keeps no Tauri.
- The stages share lines (workspace members, `Cargo.lock`, `hotline-core`'s
  `Cargo.toml` and `lib.rs`, standards.md's module map, AGENTS.md), so they land
  in order and alone.
- `pub(crate)` becomes `pub` where an item crosses a boundary. `hotline-core`
  may `pub use` the moved paths while callers move; nothing here widens what a
  teammate can reach, and task-198 states the one place that comes close.
- `ui/src/generated/contract.ts` stays byte-identical, written by an unfiltered
  `cargo test -p hotline-core` (a filtered run truncates it) or by the new
  crate's own tests once the types live there.
