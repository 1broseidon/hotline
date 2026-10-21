# Integration-test linking (task-246)

The core, CLI and app register their harnesses in `tests/integration.rs`
with Cargo autodiscovery disabled. Existing harness files, nested modules,
fixtures, test bodies and platform gates stay in place. Shared helper imports
now refer to the root's `common` module. Explicit paths preserve the existing
nested module resolution without moving files under concurrent work.

| Crate | Before binaries | After binaries |
| --- | ---: | ---: |
| hotline-core | 14 | 5 |
| hotline-cli | 4 | 1 |
| hotline-app | 2 | 2 |
| Total | 20 | 8 |

The exceptions are deliberately separate processes:

- Core `acp_shutdown` calls `end_every_agent`, which kills all registered ACP
  children in the process. Sharing it caused the startup harness to lose its
  child; restoring isolation resolves that interference.
- Core `tls` asserts that rustls has no default provider before opening a desk.
- Core `processes` and `voice` install a process-wide PATH stub for the browser
  opener. Each retains its own `common` module and stub.
- App `shell_path` is harness-free, changes the startup environment, and
  re-executes itself to model a Finder launch.

Each exception is explained beside its Cargo target. The registration check in
`scripts/test_integration_targets.py`, run by `make check` and all three CI
platforms, refuses an omitted or doubly registered top-level harness. A temporary
unregistered file was used to verify that the check actually fails. Existing
commands selecting `desk` and `rig_upgrade` now select the combined root; named
isolated targets and `make verify` retain their behavior.

## Local measurements

Baseline: `fd41e003234239804145921e79e4bc7c24c27336`.
Implementation: `00a3fd2d`. Measured 2026-10-20 (America/Chicago), macOS 27.0.1,
arm64 Mac17,14, 18 CPUs, 128 GiB RAM, rustc 1.99.0. These are single wall-clock
samples on a shared development machine, not a controlled benchmark.

| `cargo test --workspace --no-run` | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Empty build directory | 154.96 s | 129.40 s | 16.5% |
| After touching `crates/hotline-core/src/lib.rs` | 13.89 s | 9.21 s | 33.7% |

Both cold builds started with no build output, using the existing Cargo download
cache and the pinned speech archive fetched beforehand. The baseline used
`target/`; the final after build used a fresh
`/private/tmp/task-246-final-target`. Timings are
`/usr/bin/time -p`'s `real`, with normal workspace profiles and no job override.
The touch run immediately followed its corresponding cold build; the source
contents were unchanged. To reproduce, use a fresh target directory for each
revision, then touch that file and repeat the same command.

`cargo test --workspace -- --list` reports **2,180 tests before and after**.
The multiset of test function names (last path component) is identical, including
multiplicities; the new root adds a module prefix to merged tests. The SHA-256 of
the sorted JSON name-to-count inventory on both sides is
`ab80b9f5327f672c04c8efbddb150278d04a6fd14ea28d0e04967f8ba497fe06`.
Both baseline and final workspace executions passed **2,164 tests, with 16 ignored**.
The harness-free `shell_path` also ran successfully, outside libtest's count.

The first after execution used `/tmp`, which macOS symlinks to `/private/tmp`.
Four unchanged app updater unit tests rejected that executable path through
Tauri's symlink protection. Validation uses the canonical `/private/tmp` path;
no application code or tests were changed to bypass the protection.

## CI evidence still required for merge

This branch was not pushed, as instructed, so it has no after CI run or open PR.
The baseline [CI run 39532144605](https://github.com/1broseidon/hotline/actions/runs/39532144605)
uses the exact starting commit. Its Linux and Windows test commands share a
step with clippy and UI checks; that entire step is not a test-only timing.
This change gives each OS a separate Rust test step, preserving the command.

| CI test step | Before | After |
| --- | --- | --- |
| Linux | 2,172 passed / 26 ignored; shared check step 678 s (not test-only) | Requires branch CI |
| macOS | 2,164 passed / 16 ignored; test step 545 s | Requires branch CI |
| Windows | Baseline job still running at handoff; no separate test step | Requires branch CI |

The reviewer must run branch CI on all three OSes, compare each OS's before/after
passed and ignored counts, and put those results and test-step timings in the PR.
Local macOS results and Windows cross-compilation do not establish that evidence.
Raw local build, inventory and validation logs are in
`/private/tmp/task-246-measurements/` on the development host.

## Validation

- `make check`: passed, including all workspace tests (2,164 passed, 16 ignored),
  UI checks, dependency gates, Python tests, formatting and strict clippy.
- `make precheck`: all eight legs passed, including Windows cross-compilation,
  contract regeneration, strict clippy, workflow linting, security citations,
  and every UI test file in isolation.
- The final inventory still matches the baseline after restoring shutdown
  isolation. Two wrapper modules use `_harness` names to avoid clippy's
  same-name nesting lint without modifying the nested files.
