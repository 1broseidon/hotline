# Audit: dead code and dependencies (0.40)

Read-only audit of `998c78c22f1627eeba361d3f68a15aa07df5b92e`
(`release/0.40.0`), 2026-10-17, on macOS arm64. Only this report changes.
Scope is the three Rust workspace members and `ui/`; the website, docs-site
and Hotline Mobile are not dependency-audited here. Severity is the consequence
of leaving a finding; effort is XS (one mechanical edit), S (one focused change)
or M (upstream coordination or platform verification). Scanner alerts are not
proof of exploitation or permission to delete a public contract.

## Findings

**Ranked work.** Each row is an independent concern; the detailed removal lists
below distinguish unused declarations from declarations used inside their file.

| Rank | Severity × effort | Evidence | Finding |
| --- | --- | --- | --- |
| F1 | High × M | `crates/hotline-core/Cargo.toml:42`, `crates/hotline-core/src/driver/acp/install.rs:113`, `Cargo.lock:12749` | ZIP extraction reaches a version with three published extraction advisories. |
| F2 | Medium × XS | `Cargo.lock:8793`, `crates/hotline-core/Cargo.toml:38`, `crates/hotline-core/Cargo.toml:67` | rustls 0.23.45 has four advisories, fixed in 0.23.46. |
| F3 | Medium × S/M | `ui/bun.lock:615`, `ui/bun.lock:439`, `ui/src/components/Mermaid.tsx:127` | Two npm advisory families; one build dependency and one inside the diagram distribution. |
| F4 | Medium × S | `Cargo.toml:5`, `crates/hotline-core/Cargo.toml:1`, `crates/hotline-app/Cargo.toml:1`, `crates/hotline-cli/Cargo.toml:1`, `Makefile:7`, `.github/workflows/ci.yml:95` | Dependency policy is absent; all three crate manifests lack license metadata. |
| F5 | Low × XS | `crates/hotline-core/src/paths.rs:183`, `ui/src/icons.tsx:197`, `ui/src/voice/call.ts:1029` | Three unused implementations can be deleted. |
| F6 | Low × XS | `crates/hotline-core/src/process_windows.rs:38`, `crates/hotline-core/src/process_windows.rs:43` | `Job`'s field is read by production code; its dead-code allowance is redundant. |
| F7 | Low × S | `ui/src/ui/Menu.tsx:35`, `ui/src/links.ts:9`, `ui/src/wire.ts:71` | Unused exports expose live private implementations and types. Remove exports, not the implementations. |
| F8 | Low × S | `Cargo.toml:10`, `crates/hotline-core/Cargo.toml:114`, `crates/hotline-core/Cargo.toml:42` | Unused feature surface: Rig derive, a Windows namespace, and ZIP compression choices worth narrowing. |
| F9 | Low × S | `crates/hotline-app/Cargo.toml:35`, `ui/package.json:14`, `ui/bun.lock:687` | Some duplicate versions can be reduced locally; most require upstream changes. |
| F10 | Low × XS | `crates/hotline-core/src/lib.rs:5`, `Makefile:42`, `docs/design.md:663` | Stale phase wording, rather than demonstrably orphaned scripts, docs or fixtures. |

**Tool runs and limits.** Tools were installed/run ad hoc; no manifests,
lockfiles, tool configuration or generated code are committed by this audit.

| Command | Result |
| --- | --- |
| `cargo machete --with-metadata` (0.9.2) | Exit 1; only `hotline-app: tauri-build`. False positive: `build.rs:15`, `:22`, `:52` use `tauri_build` to generate the app's command manifest. No confirmed unused direct Cargo dependency. |
| `cargo +nightly udeps --workspace --all-targets --locked --target-dir /tmp/task104-udeps-target` (0.1.61) | Exit 0: “All deps seem to have been used.” Nightly was initially absent; rustup auto-installed 1.101.0-nightly (2026-10-16). All workspace targets checked on macOS; this does not certify Windows/Linux-only dependencies. |
| `cargo tree -d`; `cargo tree -d --target all` | Exit 0; 38 distinct multiversion names on macOS, 67 across targets. Same-version host/target or feature-separated build units are not additional version conflicts. Unfiltered metadata contains 1,104 packages and 93 multiversion names, including target-only resolution entries beyond the printed active tree. |
| `cargo deny --locked check advisories licenses bans` (0.20.2) | Exit 5: advisories FAILED, licenses FAILED, bans ok. No `deny.toml`; defaults reject licenses without an allow list. 10 vulnerability diagnostics, 9 unmaintained diagnostics, 66 duplicate warnings. |
| `cargo deny --locked --format json check advisories` | Same 19 advisory diagnostics; 7 distinct vulnerability IDs (ZIP IDs repeat for two versions), 9 distinct unmaintained IDs. |
| `cargo deny --locked list --format json` | Only the three workspace crates are classified unlicensed; registry crates have detected licenses. This is metadata evidence, not a legal compliance ruling. |
| `cd ui && bun install --frozen-lockfile` | Exit 0, 275 packages installed; lockfile unchanged. |
| `cd ui && npx --yes knip --reporter json` (6.41.0) | Exit 1: 30 unused value exports, 90 type exports, 1 stylesheet; no unused/unlisted dependencies or unresolved imports. |
| `cd ui && npx --yes knip --config /tmp/task104-knip.json --reporter json` | Same results with explicit `src/main.tsx`, `src/computerViewer.ts` and `tests/**/*.{ts,tsx}` entries; project includes source, tests and `vite.config.ts`. Config remains outside the checkout. |
| `cd ui && bun audit --json` (1.4.2) | Exit 1: KaTeX (low), source-map-js (high). No automatic fix run. |
| `cd ui && bun pm licenses --json`; `bun pm ls --all`; `bun pm why <package>` | Inventory and reverse paths inspected. Eight installed package names have multiple versions, counting lightningcss and its native binding separately. |
| `cd ui && bun run build` | Exit 0; viewer and main entries built. Main JS 750.42 kB, xterm 331.21 kB, lazy Mermaid classic script 3,572.66 kB (uncompressed); Vite warns about chunk size. |
| `make check` (rustc 1.99.0) | Exit 0: fmt, all-target clippy, 2,024 Rust tests (11 ignored), 420 UI tests, 9 scripts tests and 44 site tests. ts-rs emits existing serde-attribute notices; generated contract has no diff. |

Source checks used `rg`, declaration/reference counting and a fresh Cymbal index
(`--include-large-files`, 427 files, 11,379 symbols). Cymbal impact reports no
callers for `resolve_command`, `BookIcon` or `currentCall`; source searches
confirm each occurs only at its definition. A zero caller result for a type
does not prove it is unused: for example `ResourceRecord` is constructed and
returned by the importer. These checks do not establish that all public items
are dead-code-free, nor compile Linux/Windows branches on a Mac.

**F1–F3: dependency advisories and exposure.**

- ZIP 4.6.1 is used directly by ACP installation and Tauri's updater; ZIP 2.4.2
  comes from `sherpa-onnx-sys`'s build dependency. Both trigger
  [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367),
  [0368](https://rustsec.org/advisories/RUSTSEC-2026-0368) and
  [0369](https://rustsec.org/advisories/RUSTSEC-2026-0369); the fetched database
  marks `>=9.0.0` patched. `archive.extract(into)` at
  `crates/hotline-core/src/driver/acp/install.rs:114` is the affected API.
  A scratch directory and entry-path checks do not by themselves constrain a
  symlink's destination. Hash validation at `install.rs:39` happens only when
  the registry supplied a digest. This is a real extraction path, but no
  malicious archive was executed in this audit. SUID escalation additionally
  depends on privileges and filesystem policy. Tauri verifies signed updates;
  speech-engine archives are hash-pinned by `scripts/fetch-speech-engine` and
  `scripts/speech-engine.sha256`. Those trust boundaries reduce exposure without
  making the dependency alerts disappear. Update each upstream path separately;
  changing only Hotline's direct ZIP requirement leaves the other versions.
- rustls 0.23.45 triggers [0346](https://rustsec.org/advisories/RUSTSEC-2026-0346)
  (session-value Debug secrets), [0347](https://rustsec.org/advisories/RUSTSEC-2026-0347)
  (ECH rejection ordering), [0348](https://rustsec.org/advisories/RUSTSEC-2026-0348)
  (Brotli certificate allocation), and
  [0349](https://rustsec.org/advisories/RUSTSEC-2026-0349) (cached key-exchange
  downgrade). Feature inspection shows no `brotli`; source searches find no
  custom `ClientSessionStore` or `EchMode::Enable`. The first three therefore
  have no demonstrated trigger here; the last depends on peer policy and
  resumption. A focused lockfile update to at least 0.23.46 is preferable to
  replacing the TLS implementation.
- `source-map-js@1.2.1` comes through PostCSS/Vite and Tailwind's Node tooling.
  [GHSA-68fv-2mgg-jv7q](https://github.com/advisories/GHSA-68fv-2mgg-jv7q)
  describes indexed source-map processing denial of service, patched in 1.2.2.
  This is build/development exposure, not evidence that a chat message reaches
  the desktop process through this package. Its parent ranges accept a patch.
- `katex@0.16.47` comes through `mermaid@11.17.2`.
  [GHSA-238p-pmpm-9mq7](https://github.com/advisories/GHSA-238p-pmpm-9mq7)
  needs pre-existing prototype pollution or attacker-controlled option
  prototypes; patched in 0.18.2. Mermaid uses a `^0.16.47` range, so that fix is
  outside the current range. More importantly, Hotline loads
  `mermaid/dist/mermaid.min.js`, an already bundled distribution, at
  `ui/src/components/Mermaid.tsx:127`. Overriding the separately installed KaTeX
  would not prove that embedded copy fixed. Upgrade the distribution or use an
  upstream fix, then inspect the delivered asset. The component uses strict mode, disables HTML labels, and shows image
  output (`Mermaid.tsx:154`, `:75`); no application exploit was
  demonstrated. Preserve these boundaries while updating it.

Unmaintained alerts are a separate upkeep concern: proc-macro-error 1.0.4
(`RUSTSEC-2024-0370`) via Linux GTK macros; unic-char-range, unic-char-property,
unic-common, unic-ucd-ident and unic-ucd-version 0.9.0
(`2025-0075/0081/0080/0100/0098`) via Tauri/urlpattern; zbus_names 4.3.4,
zvariant and zvariant_derive 5.15.0 (`2026-0371/0370/0372`) via Linux
notification/opener plugins. The database reports no compatible safe upgrade.
Do not remove tray, notifications, URL handling or OS credential storage to
silence these; track the owning upstream upgrade.

**F4: licenses and missing policy.** The root ships `LICENSE-MIT` and
`LICENSE-APACHE`, but the workspace/package metadata does not state an SPDX
license. Add the matching declaration/inheritance in a later change. Default
cargo-deny rejected 806 crate license expressions and reported three
`no-license-field` plus six `unlicensed` diagnostics; the latter repeat the
three workspace crates. These counts do not mean 806 prohibited dependencies.
The detected registry inventory includes MIT/Apache, BSD, ISC, Unicode-3.0,
MPL-2.0 (11), CDLA-Permissive-2.0 (3) and bzip2-1.0.6 (1); LGPL appears as an
alternative in two expressions that also permit MIT/Apache. Policy and required
notices need an explicit review before deny becomes a CI gate.

The UI inventory includes MIT/BSD/ISC/Apache, MPL-2.0 (lightningcss),
MPL-2.0 OR Apache-2.0 (DOMPurify), OFL-1.1 (both fonts), and Unlicense
(robust-predicates). Bun calls `khroma@2.1.0` Unknown because its package metadata
lacks a license field; its installed `license` file contains MIT. Treat it as a
metadata/notice check, not an instruction to delete diagram support. Current
`make check` and normal CI contain no unused-dependency, advisory or license
gate. Test-run coverage and lint policy belong with tasks 106–108; do not add
overlapping configurations without the epic synthesis.

**F5–F8: exact dead declarations, visibility and feature candidates.**

Delete the whole `paths::resolve_command` wrapper at `paths.rs:183` and its
immediately preceding documentation; retain `resolve_command_in` at `:190`,
which the actual callers use with an explicit PATH. Delete `BookIcon` at
`ui/src/icons.tsx:197` and `currentCall` plus its comment at
`ui/src/voice/call.ts:1028`. Retain the call singleton and its other accessors.
No wholly unused Rust module was confirmed; exported modules lead to real
entrypoints, handlers or tests, and private code is checked by all-target clippy.

For these live declarations, remove only `export`; their internal callers stay:

| File:line | Value declarations to make private |
| --- | --- |
| `ui/src/components/Transcript.tsx:1310`, `:2034` | `REACTIONS`, `runTime` |
| `ui/src/voice/transcription.ts:180` | `PROCESSED_GAIN_DB` |
| `ui/src/computer-limits.ts:18` | `DEFAULT_MEMORY_MB` |
| `ui/src/dock.ts:15` | `CONVERSATION_MIN` |
| `ui/src/links.ts:9` | `runWords` |
| `ui/src/components/Rail.tsx:508` | `RAIL_WIDTH` |
| `ui/src/notify.ts:67` | `windowTitle` |
| `ui/src/stepText.ts:17` | `tidyPath` |
| `ui/src/useFor.ts:38`, `:68`, `:85`, `:94`, `:127` | `automaticText`, `selectedId`, `voicesOf`, `voiceOf`, `effortLabel` |
| `ui/src/voice/call.ts:155`, `:160` | `speechLevel`, `offersFront` |
| `ui/src/voice/desk.ts:244` | `refreshDeskHearing` |
| `ui/src/voice/dictation.ts:409` | `worthSending` |
| `ui/src/voice/stream.ts:7` | `MAX_PENDING_BYTES` |
| `ui/src/ui/Menu.tsx:35` | `Menu` (still used by `Picker` and `MenuButton`) |
| `ui/src/components/Skills.tsx:15` | `skillSourceName` |
| `ui/src/deskLinks.ts:29` | `readDeskLinks` |

Likewise make these 14 type declarations private, retaining their local uses:
`CallCard` (`ui/src/voice/call.ts:32`), `Microphone` (`ui/src/voice/desk.ts:36`),
`DeskKind`/`DeskState` (`ui/src/desks.ts:26`, `:29`),
`ThreadSearchResult`/`GlobalSearchResult`/`RosterHandlers`/`TapeHandlers`
(`ui/src/wire.ts:71`, `:74`, `:308`, `:310`), `Pulling`/`ThreadState`
(`ui/src/tape.ts:25`, `:35`), `ChordGroup`/`ChordMatch`
(`ui/src/chords.ts:33`, `:36`), and `McpStdioServer`/`McpHttpServer`
(`ui/src/mcp.ts:11`, `:29`). Recheck against task-107 before implementation.

Feature candidates are bounded, not a recommendation to disable all defaults:

- No workspace crate defines its own `[features]`, and no repository Rust
  `cfg(feature = ...)` branch was found. There is no abandoned local feature to
  remove. Dependency features still require separate scrutiny.
- `rig` defaults include `derive`; no Rig derive/attribute macro invocation was
  found. Change the workspace dependency at `Cargo.toml:10` to disable defaults
  and explicitly retain `agent`, `reqwest`, `rustls`, then prove all callers
  compile. The upstream 0.42.0 manifests show those preserve today's required
  defaults while dropping `rig-derive`; retaining `agent` is essential.
- `Win32_System_Memory` at `crates/hotline-core/Cargo.toml:114` has no direct
  namespace consumer. Capacity reads `System::SystemInformation` at
  `computer/capacity.rs:169`; retain that feature and every actually used Win32
  namespace. Remove the Memory feature only with Windows checks.
- `zip`'s `deflate` umbrella at `Cargo.toml:42` enables both zopfli compression
  and flate2. Runtime only extracts, while `ZipWriter` occurs in the install
  tests (`driver/acp/install.rs:216`). Consider a decoder-oriented feature set
  after selecting the patched ZIP version; do not assume the 4.x feature names
  or that removing this edge removes zopfli from sherpa's build dependency.

**F9: duplicates and dependency weight.**

| Family | Active reverse paths and disposition |
| --- | --- |
| ZIP 2.4.2 / 4.6.1; bzip2 0.4.4 / 0.5.2 / 0.6.1 | sherpa build, updater, and Hotline archive handling. Resolve security upgrades first; a direct-version change cannot unify upstream major requirements. |
| rand 0.8.8 / 0.9.5 / 0.10.2; getrandom 0.2.17 / 0.3.4 / 0.4.3 | Crypto, runtime libraries and app tokens/staging IDs (`crates/hotline-app/src/lib.rs:66`, `transfer.rs:182`, `laptop.rs:72`). The direct rand dependency is live; do not unify crypto versions forcibly. |
| png 0.17.16 / 0.18.1 | Tauri icon decoding vs Hotline image handling; retain both features. |
| schemars 0.8.22 / 1.2.2; syn 2.0.119 / 3.0.4; thiserror 1 / 2 | Different upstream API/proc-macro generations. Do not patch major versions under their callers. |
| base64 0.21.7 / 0.22.1 / 0.23.1 | Tauri build, common HTTP/auth, RTC. Direct use is already shared with much of the graph; no drop-in unification proved. |
| `@tauri-apps/api` 2.11.1 / 2.12.1 | The global-shortcut plugin's compatible `^2.8.0` range resolves separately (`ui/bun.lock:687`). A lockfile dedup to the existing direct pin is a small candidate; retain the explicit tested pin. |
| lightningcss 1.32.0 / 1.33.0 plus bindings | Tailwind pins 1.32.0; Vite requires `^1.33.0`. Wait for the parent ranges to converge. |
| internmap, commander, cose-base, layout-base, `@types/unist` | Different diagram/layout/Markdown generations. No safe override proved. |

Heavy dependencies have actual jobs. WebRTC/RTC/Opus implement the ChatGPT
subscription call (`voice/realtime/live.rs:35`, `:50`, `:812`); speech uses a
prebuilt sherpa engine and an AAC/MP4-only Symphonia configuration
(`voice/speech/local.rs:563`, `crates/hotline-core/Cargo.toml:104`). Replacing
them with a shell program or a second engine is not a mechanical cleanup.
Reqwest is already shared with Rig/rmcp; swapping only Hotline to a smaller
HTTP client would add a second client without removing the original.
Image codecs are restricted to jpeg/png/webp/gif (`Cargo.toml:98`), archive
defaults already disabled (`:42`), and SQLite's bundled FTS is intentional.
No lighter replacement was established that preserves these behaviors.

The UI's largest asset is Mermaid, already loaded on demand as a classic
script for WebKitGTK compatibility (`Mermaid.tsx:120`). A smaller renderer
would reduce supported diagram syntax; an ES-module switch would reopen the
documented platform failure. Xterm is also dynamically loaded only for agent
sign-in (`AgentSignIn.tsx:101`). Font CSS imports five weights with all supported
scripts and both font formats (`ui/src/tokens.css:12`); the build emits 56 font
assets. Dropping WOFF while retaining every language is a measurable later
packaging experiment, not evidence that either font dependency is unused.

## Leave alone

**Every dead-code suppression accounted for.** The task's count of five is
stale: this snapshot has seven unconditional allowances and three conditional
`cfg_attr` allowances, ten sites total. Retain the justified allowances below;
only the Windows Job allowance has a confirmed removal.

| Site | Verdict and evidence |
| --- | --- |
| `crates/hotline-core/src/import/records.rs:39` | Keep. `ResourceRecord` deliberately parses the complete legacy row; ownership/version/stamp fields are asserted at `:344`–`:347`. Removing fields would weaken import-shape coverage and contradict the explanation at `:34`. |
| `crates/hotline-core/src/process_windows.rs:38` | Remove only the allowance. `terminate` reads `self.0` at `:43`, tests read it too, and the owned handle must remain alive for kill-on-close. Never delete the field. |
| `crates/hotline-core/tests/common/mod.rs:26` | Keep on `store`: separate integration-test crates include the same module and use different subsets; the disposable credential store keeps tests out of the operator's keychain. |
| `crates/hotline-core/tests/common/mod.rs:32` | Keep on `open_desk`: used by `tests/threads.rs:50`, among others, but not every integration crate. |
| `crates/hotline-core/tests/common/mod.rs:37` | Keep on `open_vault`: used by `tests/desk.rs:88`, absent from other integration binaries. |
| `crates/hotline-core/tests/common/mod.rs:50` | Keep on Unix `browser`: used by `tests/voice.rs:2815` and `tests/processes.rs:296`; prevents real browser launches. |
| `crates/hotline-core/tests/common/stalled.rs:8` | Keep shared-fixture allowance. ACP startup/shutdown use different Client/helper subsets; all other `mod common` harnesses compile the module without using it. A future module-placement tidy may narrow its scope, but none of the fixture's behavior is shown obsolete. |
| `crates/hotline-core/src/push.rs:123` | Keep `cfg_attr(test, ...)`: unit tests capture messages instead of contacting Expo (`:114`–`:120`); production calls `send`. |
| `crates/hotline-core/src/mcp/mod.rs:166` | Keep conditional field allowance: Unix Drop reads the captured PID; Windows owns a kill-on-close Job instead. Cross-platform construction still initializes the PID field. |
| `crates/hotline-core/src/tools/shell.rs:474` | Keep conditional field allowance: Unix cancellation/Drop read it; on Windows the job handles termination. Shared construction/finished code writes it. |

**Platform code and public boundaries.** `#[cfg(unix)]`, Windows job handling,
macOS notifications/speech/Seatbelt, Linux bubblewrap/WebKit, and their inverse
stubs match the CI/release matrix (`.github/workflows/ci.yml:35`, `:112`, `:166`).
`--all-targets` means library/binary/test/example targets, not all operating
systems. The fallback shell branches for other OSes (`tools/shell.rs:386`,
`:447`) never run in that matrix but deny unsupported confinement explicitly;
keep them. No contradictory or misspelled cfg gate was identified.

`TranscriptPush` and `PeerThreadPush` have no handwritten Rust callers
(`crates/hotline-core/src/contract.rs:2208`, `:2218`) and describe old push
names, but are ts-rs exported declarations. Their removal changes the generated
public surface, outside the epic's no-contract-change constraint. Keep them
for a separate compatibility review. All 75 Knip-reported generated type exports
likewise remain: the window is not every consumer, and types referenced by other
generated types are not necessarily dead. Never edit the generated file to
satisfy Knip; unfiltered core export tests own it.

`ui/src/cadence.ts:1`, `ui/src/pacing.ts:1`, and `ui/src/reveal.ts:1` explicitly
declare a mobile file-copy consumer. Thus leave `PACE_WITHIN_MS`, `beatMs`,
`SHORT_UNIT_CHARS`, `paced`, `revealed`, `revealSegments`, and `Segment` exported
until that external sync contract is checked. This accounts for the remaining
six value exports and one handwritten type from Knip. Mobile is outside this
epic, so local absence of callers is insufficient. Keep `computerViewer.css`:
`ui/computer.html:7` links it and Vite includes the second HTML entry at
`ui/vite.config.ts:15`; Knip's stylesheet result is a false positive.

**Assets and operational files.** No exact deletion is justified by this pass:

| Inventory | Current consumer |
| --- | --- |
| `scripts/fetch-speech-engine`, `speech-engine.sha256`, `cargo-dev-sign` | Makefile speech-engine/dev and CI/release archive verification/signing. |
| `scripts/release_notes.py`, `updater_manifest.py` | Release workflow at `.github/workflows/desktop-release.yml:316`, `:324`. Their two `test_*.py` files run under `Makefile:8`. |
| `packaging/hotline.service`, `hotline-relay.service` | Server archive at `.github/workflows/desktop-release.yml:275`; operator instructions at `docs/serve.md:14`, `:171`. |
| `packaging/cloudflare/grizzly-https-redirect.json` | Deployment reference at `docs/serve.md:259`; a manually applied rule is not unused because Cargo does not open it. Live deployment state was not inspected. |
| Voice WAV/AAC fixtures and README | `tests/voice.rs:398`, `:470`; `tests/local_speech.rs:457`; `voice/speech/local/tests.rs:370`; `examples/local_speech_check.rs:58`. Ignored model/provider tests remain intentional consumers. |
| `tests/fixtures/noise_v2.json` | `remote/sealed.rs:490`, `:497`; cross-client wire vectors also documented in `docs/wire.md:1098`. |
| App updater key/payload/signature fixtures | `crates/hotline-app/src/updater.rs:492`–`:494`; signing test material, not shipped credentials. |
| `docs/voice-calls.md`, ADRs and release notes | Call-contract links in `docs/wire.md:1357`; design links to ADRs; release notes consumed by release generation. Historical records are not dead runtime modules. |

F10 is wording drift: replace the Phase-0-only present tense at `lib.rs:5` and
the future harness statement at `Makefile:42`; qualify historical phase exit
text at `docs/design.md:663`. Do not delete the design phases or whole docs to
repair these sentences. Coordinate broader docs drift with task-108.

## Phase 2 tasks

Proposals only; do not create or apply them until George's synthesis review.
Each item can land on its own, runs `make check`, and carries no unrelated
reformatting or file split. Platform-dependent items additionally need the
existing Linux/Windows/macOS CI. Re-run dependency scanners after lock changes.

1. **Patch rustls only (F2, XS).** Update the locked rustls from 0.23.45 to
   `>=0.23.46` within 0.23; do not update every dependency. Verify the four IDs
   disappear with cargo-deny; exercise the existing sealed-wire/TLS tests and
   `make check`.
2. **Patch ACP ZIP extraction (F1, M).** Upgrade Hotline's direct ZIP dependency
   to a version covered by the fixes, adapting only `driver/acp/install.rs` and
   its tests. Remove the old direct version when no upstream needs it. Add
   malicious symlink/permission archive denial cases and retain ordinary ZIP
   installation success. `make check` plus the ACP wire/install harnesses; do
   not claim Tauri/sherpa alerts fixed by this one task.
3. **Update the Tauri updater ZIP path (F1, M).** Upgrade only the owning plugin
   and any required matched configuration to an upstream release with patched
   extraction. If unavailable, file an upstream blocker with the exact IDs;
   do not force ZIP 9 under a ZIP-4 API. `make check`, updater signature/format
   tests and desktop platform CI. This can land independently of item 2.
4. **Update the sherpa build ZIP path (F1, M).** Choose a compatible patched
   upstream wrapper/build release and refresh its prebuilt-archive hashes as
   required. Keep the no-unverified-download build invariant. Validate fetch
   success/failure, local-speech fixtures, `make check`, and each release
   target. File an upstream blocker if no suitable release exists.
5. **Patch source-map-js (F3, S).** Refresh only that transitive resolution to
   `>=1.2.2` within parent ranges, retaining frozen installs. `bun audit`, the UI
   production build and `make check` prove both build and tests still work.
6. **Patch the shipped Mermaid distribution (F3, M).** Upgrade Mermaid to an
   upstream bundle containing the KaTeX fix, retaining classic-script loading,
   strict mode, plain SVG labels and image output. Verify the actual minified asset, not just
   Bun's dependency tree; run diagram rendering on WebKitGTK, UI build and
   `make check`. No blanket transitive override.
7. **Declare existing crate licenses (F4, XS).** Add the SPDX expression matching
   the root licenses to workspace metadata and inherit it in all three
   packages. `cargo deny list` must no longer classify the workspace crates
   unlicensed; `make check`. No license-policy rollout in this change.
8. **Remove three dead implementations (F5, XS).** Delete only
   `paths::resolve_command` and its own docs, `BookIcon`, and `currentCall` and
   its own docs, exactly as listed above. Keep `resolve_command_in`, the call
   singleton, and every other icon. Run `make check`; no new mirrored tests.
9. **Remove the redundant Windows allowance (F6, XS).** Delete only
   `#[allow(dead_code)]` on `Job`'s field, keeping its ownership and all Job
   behavior. Windows all-target clippy/tests and `make check`.
10. **Privatize window internals (F7, S).** Remove exactly the 22 value `export`
    keywords and 14 type `export` keywords listed in Findings. Do not delete
    bodies, generated types or mobile-shared exports. Coordinate with task-107
    and run Knip, UI build and `make check`.
11. **Drop Rig derive (F8, S).** Replace default features with explicit
    `agent`, `reqwest`, `rustls`; remove `rig-derive` from the lockfile if no
    remaining consumer. `cargo tree -i rig-derive` should have no match;
    `make check` including all binaries/examples. Keep provider HTTP behavior.
12. **Drop unused Win32 Memory feature (F8, XS).** Remove only
    `Win32_System_Memory` from core's dependency features; retain
    `Win32_System_SystemInformation`. Windows all-target checks and `make check`.
13. **Deduplicate the compatible Tauri JS API resolution (F9, S).** Resolve
    global-shortcut's API dependency to the tested direct 2.11.1 pin if Bun's
    normal resolver permits it. Delete only the redundant lockfile resolution;
    no plugin version churn or broad override. Verify shortcut tests, frozen
    install, UI build, desktop shortcuts and `make check`.
14. **Repair stale phase wording (F10, XS).** Edit only the three sentences
    identified above; keep history labeled as history. Coordinate with
    task-108 and run `make check`.
15. **Gate unused Cargo dependencies (F4, S).** Add a pinned machete invocation
    to `make check` and CI with the demonstrated `tauri-build` false positive
    explained by its build-script caller. It must pass without removing that
    dependency. Run the resulting `make check`.
16. **Gate unused window declarations (F4, S).** Add pinned Knip with both HTML
    entrypoints, tests, generated types, mobile-copy exports and the HTML
    stylesheet handled. Base the config on actual consumers, then require it
    from `make check` and CI after the approved visibility/dead-code edits
    land. Run the resulting `make check` and UI build.
17. **Gate dependency advisories (F4, S).** Add pinned cargo-deny advisories and
    Bun audit checks after the fixes above, with a deliberate severity policy.
    Record unresolved upstream exceptions by ID with a review condition,
    rather than blanket ignores; runtime and build alerts stay distinguishable.
    Run the resulting `make check` and all-platform dependency checks.
18. **Gate reviewed dependency licenses (F4, S).** Add cargo-deny's reviewed
    allow policy and a UI license/notice inventory after the metadata fix.
    Account for fonts, generated distributions and native speech assets as
    well as registry package declarations. Do not enable deny's empty default
    license policy or assert a compliance result from this audit alone. Run
    the resulting `make check` and verify the release notice inventory.

No Phase 2 task is proposed to delete a Cargo/npm direct dependency, whole
Rust module, operational script, packaging file, fixture or documentation
file: this audit did not establish one as unused. Compression-feature tuning,
font-format trimming, and upstream GTK/D-Bus maintenance
remain measured followups rather than speculative removals. Task-109 should
sequence dependency fixes before enabling hard gates, and reconcile this
report's visibility/docs edits with tasks 107/108.
