# Standards

The conventions every change to this tree follows: where a module and its
tests go, how big a file may grow, how errors and panics are handled, which
lints run and how a dependency is kept. They were drawn from the 0.40 audits
(`docs/audits/0.40/`) and approved by George on 2026-10-17. `AGENTS.md`
holds the general code standards; this file holds the specific ones. The
module map itself is [development.md](development.md#layout).

## Module layout

- A module with children is `foo.rs` beside `foo/`, the Rust 2018 layout,
  never `foo/mod.rs`. The one exception is `crates/*/tests/common/mod.rs`,
  which is Cargo's idiom for helpers shared by integration tests.
- `#[path]` is used only in integration harnesses to keep their existing
  files and nested modules in place while sharing one test binary.
- One type's behaviour may be filed by concern across sibling files, the way
  `impl Room` is spread over `session/*`. Do not invent per-concern traits or
  types for that.
- `hotline-contract` is a leaf: it depends on no other Hotline crate, and
  `hotline-core` re-exports it as `contract`. Every `#[ts(export)]` type
  lives in it, because ts-rs starts `ui/src/generated/contract.ts` afresh in
  each test process: a type exported from a second crate would erase the
  rest, or be erased. The file is what an unfiltered
  `cargo test -p hotline-contract` writes.
- `hotline-support` is the other leaf: the utilities that hold no Hotline
  state (`credentials`, `paths`, `room_lock`, `log_file`, `images`,
  `thumbs`, `fence`, `process_windows`). It depends on no Hotline crate, not
  even `hotline-contract`, and `hotline-core` re-exports each module under
  its old name, so `crate::paths` still holds there.
- `hotline-store` is what is kept on disk and rebuilt from it: `log`,
  `sent`, the search index, a tape's chapters and previews, and the link
  line a thread leaves on its parent with the `ThreadState` it records. It
  stands on `hotline-contract` and `hotline-support` and on no other Hotline
  crate. `hotline-core` re-exports `log` and `sent` under their old names
  and the crate as `store`, so `crate::store::search` still holds there,
  and `thread` re-exports the link's types.
- `hotline-tools` is a teammate's workspace and shell tools, with the
  `CapabilityLease` and `CapabilityEpoch` each tool checks in its
  `capability` module. It stands on `hotline-contract` and
  `hotline-support` and on no other Hotline crate. The lease's fields are
  private to that module, so no other crate can forge or revive one.
  `hotline-core` re-exports the crate as `tools` and `driver` re-exports
  the two types, so `crate::tools::Workspace` and
  `crate::driver::CapabilityLease` still hold there.
- `hotline-websearch` is web search: the providers, the chain that asks
  them at once and fuses their lists, and the cleaning of what comes back.
  It stands on `hotline-contract` alone. The vault's keys, the desk's
  switches and the `web_search` tool stay in the core, which re-exports the
  crate as `websearch`, so `crate::websearch::Search` still holds there.
  Its mock provider is public only under the `test-support` feature, which
  the core turns on for its tests alone.
- `hotline-room` is the room as everything above the core sees it: the
  roster and settings folded from the room stream, with the `mcpServers`
  setting's reading beside them, linked desks, `RoomHandle` and its
  `AuthStatus`, the tool ledger, voice's settings, ledger, metering and
  spoken replies, and the stateless pieces the core's next crates need
  (`ProviderAuth`, `same_secret`, `redact_text`, `clip`, the TLS default,
  `HOME_MOUNT`). It stands on `hotline-contract`, `hotline-support`,
  `hotline-store` and `hotline-websearch`, and never on `hotline-core` or
  Tauri. `hotline-core` re-exports each piece at its old path, so
  `crate::room`, `crate::linked`, `crate::session::ledger`,
  `crate::voice::metering` and `crate::wire::RoomHandle` still hold there.
  The wire's `Parts` (the calls, the Remote, the links) names core types,
  so it stays in the core with the wire. The voice budget's unreserved
  `charge` is public only under its `test-support` feature.
- `hotline-mcp` is Hotline as an MCP client: the servers the `mcpServers`
  setting names, the grant a teammate's policy selects, the live
  connections and the tools they list, and OAuth sign-in to an HTTP
  server. It stands on `hotline-contract`, `hotline-support`,
  `hotline-tools` (the lease a tool checks) and `hotline-room` (the tool
  ledger), and never on `hotline-core` or Tauri. It never opens the vault:
  the core reads what the vault keeps for a server and hands it in as
  `Vaulted`, the OAuth tokens behind rmcp's `CredentialStore`, which the
  vault implements, and saving a client registration is the one call
  back. `hotline-core` re-exports its items at `crate::mcp`, where
  `TeammateTools`, the server of a teammate's own tools, stays: it calls
  into the live room, the `mcp`↔`session` cycle below.
- `hotline-voice` owns calls and their speech and realtime engines. It stands
  on `hotline-contract`, `hotline-support`, `hotline-store`, `hotline-room`,
  `hotline-tools` and `hotline-mcp`, never on `hotline-core` or Tauri. Its
  single `CallRoom` interface reaches sessions, the vault and real tool
  handlers through `hotline-core::voice::room`; `voice::connected` still
  reads the vault into the engines' `Connected`. Temporary interface
  methods name S5, S6 or task-203, which remove or reshape them. The
  real-room call tests stay beside that implementation; pure voice tests
  live with the runtime. `test-support` exposes their bounded setup and
  inspection, without compiling it into production.
- A new crate comes from the plan in
  [ADR 0007](adr/0007-split-hotline-core.md): it holds one dependency
  direction, its edges only go down, and nothing but the app and the CLI
  depends on `hotline-core`. It declares the workspace `[lints]`, a license,
  and a crate doc saying what it owns and what it must never depend on.
- A module is `pub` only when the app, the CLI, the harnesses, the examples
  or a crate above its own name it. Otherwise it is `pub(crate)`.
- Do not break the `session`↔`driver` and `mcp`↔`session`
  cycles with single-implementation traits. The call-room interface is the
  explicit exception approved in ADR 0008 §5. A trait with one implementation
  is indirection that buys nothing here.

## Test layout

- Tests stay in the file, in `mod tests`, while they are under 400 lines and
  smaller than the code they test.
- Past that, they move to a sibling `foo/tests.rs`.
- Past 2,000 lines, `foo/tests.rs` keeps only the shared fixtures, and the
  tests move to `foo/tests/<topic>.rs`.
- There is no `_tests.rs` suffix. Integration harnesses are
  `crates/*/tests/<area>.rs`, registered as modules in `tests/integration.rs`
  with `autotests = false` and one explicit `[[test]]` target per crate.
  Harnesses that require process isolation stay separate explicit targets,
  with the reason beside the target in Cargo.toml. Add every new harness to
  the root (or an explained separate target), or Cargo will not run it. Shared
  helpers live in `tests/common`: one bounded `Client`, one desk-and-door
  helper, one fake model server.
- A test waits on a signal through one `until(what, cond)` with one generous
  deadline. It asserts floors and order, never an elapsed-time ceiling,
  because a loaded runner makes every ceiling flaky.
- Time a test controls runs on tokio's paused clock. A real wait is only for
  a real child process or a real kernel buffer.
- Tests reach the desk over loopback. They never take a host interface or
  port 8788; they are handed a bound listener instead.
- Scratch space is a `tempfile::TempDir`.
- An opt-in test is `#[ignore = "needs …"]`, never an early return, so a
  skipped test shows as skipped. A missing tool fails by name.
- `docs/security.md` cites only tests that exist. Renaming a cited test, or
  changing what it asserts, updates the row in the same commit; precheck
  refuses a cited test that is not in the file it names.
- In the window, `ui/tests/` stays flat. A `.ts` test is pure and a `.tsx`
  test uses the DOM, through the one `tests/dom.ts`. The tests are
  typechecked.

## File budget

- Production code: aim for 1,000 lines per file. 1,500 lines triggers a
  split.
- Test files: 2,000 lines.
- Functions: 250 lines. `too_many_lines` is an audit lens, not a lint.
- The window uses the same numbers for files and for components. The one
  stated exception is the `Call` class in `ui/src/voice/call.ts`.

## Errors and panics

- Fallible external setup returns an error to one outer handler, which
  exits unsuccessfully. That covers disk, sockets, the TLS and HTTP client,
  the runtime, signals, entropy and the window shell. Setup never starts an
  empty room, regenerates unreadable settings, or falls back to weak
  entropy.
- An impossible state is an `expect` that names why it holds, not a bare
  `unwrap`. `unreachable!` names the narrowing that justifies it. Keep the
  guard beside the access, and add no generic safe-index helper.
- A lock is taken with `unwrap_or_else(PoisonError::into_inner)` through the
  module's `lock` helper, in destructors too. Once a lock is recovered,
  state that an interrupted mutation could break, such as an authorization
  record, is revalidated or cancelled before more work is admitted.
- A typed error is kept wherever a caller branches on it: retry, status,
  fallback, or revoked versus lost. `String` is for a terminal sentence at a
  boundary, and `io::Result` is for callers that read `ErrorKind`. There is
  no global error framework and no workspace `Result` alias.
- A person sees a few-word action label, with sanitized detail behind a
  disclosure that works. Diagnostics never go in tooltips, which are two to
  three words (decision-2 on the board). Notice shapes on the tape stay
  byte-compatible with the reference tree.
- Reading a `serde_json::Value` may index. A mutable or map index proves its
  key first. Build scripts and examples may panic.

## Lints

- Lint levels live in `[workspace.lints]` in the root `Cargo.toml`, and each
  crate declares `[lints] workspace = true`. `make check` keeps
  `-D warnings`. The workspace sets `rust-version`.
- rustfmt runs on its defaults, with no config file.
- Clippy runs its defaults, plus pedantic and restriction lints chosen one by
  one, never whole groups. The lints that `docs/audits/0.40/lints.md` lists
  as not worth it stay off. Blanket `unwrap_used`, `expect_used`,
  `indexing_slicing` and the print lints are review lenses, not lints.
- A suppression is `#[expect(lint, reason = "…")]`, so it fails once the
  reason goes away. `#[allow(lint, reason = "…")]` is only for code compiled
  into several binaries that use different parts of it, or code whose use
  depends on `cfg`. Every suppression has a reason.
- `too_many_arguments` is fixed by grouping values that already travel
  together, not by adding a layer.
- Generated output is never lint-fixed by hand.
- In the window, Biome lints and does not format, with the rule set from
  `docs/audits/0.40/window.md`. `useExhaustiveDependencies` is an error, and
  an effect that re-runs on purpose says so in a comment. ESLint waits until
  typescript-eslint supports TypeScript 7.
- A tool joins `make check` only once the tree passes it. An upstream
  exception is listed by its ID with the condition for reviewing it.

## Dependencies

- Each crate declares its SPDX license.
- An advisory is fixed on each path that owns the dependency. A major
  version is never forced under an API that cannot take it, and a lockfile
  override does not prove a bundled copy is fixed.
- A heavy dependency stays while it does a job nothing lighter does.
