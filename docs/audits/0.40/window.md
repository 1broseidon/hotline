# Audit: the window (0.40)

Method: `bunx knip` (no config), Biome 2.5.15 with `recommended` plus
`useExhaustiveDependencies`, `useHookAtTopLevel`, `noUnusedImports`, and
`noExplicitAny`, and grep, all run ad hoc against `ui/src` with
`src/generated/**` excluded. Nothing was added to `ui/package.json`.
`typescript-eslint` was tried and refuses to load: it rejects TypeScript 7.0,
which `ui/package.json:41` pins. Counts below are from that run. `ui/` is
~30k lines of TypeScript, of which 2149 are the generated contract.

## Findings

Ranked by severity, then effort (S under an hour, M a half day, L a day or
more).

### 1. Effects the compiler cannot check: 52 dependency-list hits (high, M)

Biome's `useExhaustiveDependencies` reports 52 hits, 13 of them in
`src/App.tsx` (`:82`, `:180`, `:193`, `:234`, `:350`, `:361`, `:365`, `:421`,
`:486`), 10 in `Settings.tsx` (`:731`, `:1548`, `:1637`), and the rest in
`ConnectProvider.tsx:73,80`, `Welcome.tsx:120` and others. They are two kinds:

- *Missing* dependencies, mostly `setDock` / `setInspector` at
  `App.tsx:350-421`. These are `useState` setters, which are stable, so they
  are noise for Biome today; but a few of the others are real values read in
  an effect that does not list them.
- *Extra* dependencies (`App.tsx:82,180,193,234,486`): values listed to make
  the effect re-run when they change although the body does not read them.
  That is "re-run on change" expressed as a dep, a deliberate trick that
  nothing marks as deliberate.

Neither kind fails `tsc`, so nothing in `make check` looks at them. Triage
each hit: a stable setter is fixed by the rule's own convention, a re-run
trigger gets a one-line comment or becomes a keyed `useEffect`, and a
genuinely stale closure is a bug to fix. The last group is the point of the
task; the size of it is unknown until someone reads the 52.

### 2. `App()` is one 780-line function (high, L)

`src/App.tsx:88` to `:872` holds the whole window's state: the selection,
the dock and inspector panes, the roster and settings subscriptions, the
hotkey and chord handlers, the seen-marker bookkeeping and the layout. 23
`useEffect` calls live in it. Real seams:

- **Navigation** (`selectedId`, pane, inspector, `openOnDesk`, `takeChord`;
  `App.tsx:59`, `:873`, `:890-899`) into a `useNavigation()` hook.
- **Seen markers** (`loadSeen` / `saveSeen`, `App.tsx:912-935`) into a
  `useSeen()` hook, shared with finding 5.
- **Hotkeys and chords** (the effects near `App.tsx:350-421`) into
  `useWindowHotkeys()`.
- The return JSX keeps layout only.

Do these as separate landings; each extracts state plus its effects together
so the dependency lists (finding 1) get fixed while they are small.

### 3. Three 1,800-2,000 line components (medium, M each)

- `src/components/Settings.tsx` (2068): already a file of sections, so the
  seams are the existing top-level functions. `ProvidersSection` +
  `ProviderPage` (`:700-1236`, ~540 lines) and `ToolsSection` through
  `ServerForm` (`:1257-1955`, the MCP pages, ~700 lines) each go to their own
  file under `components/settings/`. `ComputerSection` (`:444`) follows.
  `Settings` itself (`:187`) stays a router. `ImportSection` (`:1956`) is
  already independent.
- `src/components/Transcript.tsx` (2043): `Transcript()` (`:73-472`) is the
  list; everything below `Row` (`:715`) is a leaf renderer. Split the leaves
  by kind: steps (`Steps`, `StepRows`, `Tool`, `Thought`, `EditLines`;
  `:1030-1740`), human-ask cards (`HumanAction`, `PasskeyAskCard`,
  `ExchangePaused`; `:1788-1998`), bubbles and reactions (`UserBubble`
  through `Reactions`; `:1237-1450`), and the pure folds (`superseded`,
  `foldReactions`, `stepRuns`, `runEnding`, `peerLine`) into a `.ts` that the
  tests can import without React. The four `use*` hooks at `:473-594` are a
  fifth unit.
- `src/components/Teammate.tsx` (1849): `ComputerRows` (`:692-1078`, ~390
  lines) with `SliderRow`, `LimitRow`, `AddMount` is one file;
  `ToolLedger` (`:1605-1714`) and `Threads` (`:1715-1849`) are two more;
  `McpRows`/`SkillRows` (`:1412-1604`) a fourth. The `Teammate()` body
  (`:59-420`) is then readable.
- `src/voice/call.ts` (1129): one `Call` class (`:172-1028`, ~860 lines).
  Splitting a class is not free; leave it unless a feature needs it
  (see Leave alone).

### 4. Dead exports: 30 values and 15 non-generated types (medium, S)

knip, confirmed by grep (no use in `src/` outside the definition file and
none in `tests/`):

`beatMs` and `PACE_WITHIN_MS` (`cadence.ts:17,20`), `RAIL_WIDTH`
(`Rail.tsx:508`), `skillSourceName` (`Skills.tsx:15`), `REACTIONS`
(`Transcript.tsx:1310`), `runTime` (`Transcript.tsx:2034`),
`DEFAULT_MEMORY_MB` (`computer-limits.ts:18`), `readDeskLinks`
(`deskLinks.ts:29`), `CONVERSATION_MIN` (`dock.ts:15`), `BookIcon`
(`icons.tsx:197`), `runWords` (`links.ts:9`), `revealed` and `revealSegments`
(`reveal.ts:8,45`), `tidyPath` (`stepText.ts:17`), `automaticText`,
`selectedId`, `voicesOf`, `voiceOf`, `effortLabel` (`useFor.ts:38-127`),
`speechLevel`, `offersFront`, `currentCall` (`voice/call.ts:155,160,1029`),
`refreshDeskHearing` (`voice/desk.ts:244`), `worthSending`
(`voice/dictation.ts:409`), `MAX_PENDING_BYTES` (`voice/stream.ts:7`),
`PROCESSED_GAIN_DB` (`voice/transcription.ts:180`).

Not all are dead code: `windowTitle` (`notify.ts:67`) and `paced`
(`pacing.ts:15`) are used inside their own file and only need `export`
dropped. `src/pacing.ts` and `src/reveal.ts` are close to being dead as a
pair (`paced`, `revealed`, `revealSegments`, `SHORT_UNIT_CHARS`), so check
whether anything still imports the module before deleting its members. The
repo rule is "if it isn't reachable from a caller or a test, it doesn't
ship", so these go.

The 75 unused exported types in `generated/contract.ts` are not findings:
ts-rs exports every contract type and the window uses a subset. Real unused
types elsewhere: `ChordGroup`/`ChordMatch` (`chords.ts:33,36`),
`DeskKind`/`DeskState` (`desks.ts:26,29`), `McpStdioServer`/`McpHttpServer`
(`mcp.ts:11,29`), `Segment` (`reveal.ts:38`), `Pulling`/`ThreadState`
(`tape.ts:25,35`), `CallCard` (`voice/call.ts:32`), `Microphone`
(`voice/desk.ts:36`), `ThreadSearchResult`/`GlobalSearchResult`
(`wire.ts:71,74`), `RosterHandlers`/`TapeHandlers` (`wire.ts:308,310`).
Unexport them (the types still document the file's own signatures).

### 5. Copy-pasted persistence helpers (medium, S)

`App.tsx:912-935` (`loadSeen`/`saveSeen`) and `Teammate.tsx:1814-1839`
(`loadThreadSeen`/`saveThreadSeen`) are the same 15 lines: read
localStorage, parse, reject non-objects, keep finite-number values, swallow
errors. They differ only in key. Twelve files touch `localStorage` (`App`,
`Rail`, `Teammate`, `UpdateFloat`, `dock`, `desks`, `drafts`, `deskWatch`,
`hotkeys`, `theme`, `voice/hearing`, `voice/dictation`), 28 raw
`getItem`/`setItem` calls in all, each with its own try/catch. One
`storedNumbers(key)` / `storeNumbers(key, record)` in a `storage.ts` removes
the two copies; whether the other ten want a typed wrapper is a second step
once there is a second real caller.

### 6. Linter: adopt Biome, not ESLint (medium, S to adopt, M to clean)

- ESLint's TypeScript parser, `typescript-eslint`, throws on TypeScript 7
  at import. The project pins `^7.0.2`, so ESLint cannot parse the source
  until upstream catches up (tracking issue linked from the error:
  typescript-eslint#10940). That is a blocker, not a preference.
- Biome has no such dependency, runs the whole `src` in well under a second,
  and ships the one rule that matters here (`useExhaustiveDependencies`).
- Out-of-the-box `recommended` is noisy for this tree; proposed rule set is
  `recommended` with these changed:
  - **on, error:** `useExhaustiveDependencies` (after finding 1 triage),
    `useHookAtTopLevel`, `noUnusedImports`, `noExplicitAny`.
  - **off:** `style/noNonNullAssertion` (86 hits; `noUncheckedIndexedAccess`
    is already on and the assertions are mostly after a length check),
    `style/noDescendingSpecificity` (37, all in CSS; Tailwind v4 makes
    order meaningful), `complexity/useLiteralKeys` (17; the contract
    objects index by string on purpose), and the whole CSS linter
    (`tokens.css:205-262` does not even parse).
  - **a11y: warn first.** 84 hits; 41 are `noSvgWithoutTitle`, 37 of those in
    `icons.tsx`, where icons are decorative and need `aria-hidden` rather
    than a title (one change in the icon wrapper). The real a11y findings are
    `noAutofocus` (9), `useSemanticElements` (8) and
    `noStaticElementInteractions` (8), each a UI change, not lint noise.
  - **fix, then on:** `noControlCharactersInRegex` (7, all `stepText.ts:14`:
    the escape stripper, intentional; one `biome-ignore` with the reason),
    `useIterableCallbackReturn` (`voice/call.ts:686`, `dictation.ts:549`),
    `useExponentiationOperator` (`voice/call.ts:156,938`).
- Wire it as `bun run lint` in `make ui-check`, formatter off: the tree is
  tab-indented with a consistent style and no formatter is configured, so
  turning one on would reflow files for no invariant.

### 7. tsconfig strictness is already high (low, S)

`ui/tsconfig.json` has `strict`, `noUncheckedIndexedAccess`,
`exactOptionalPropertyTypes`, `noUnusedLocals`/`Parameters`,
`verbatimModuleSyntax`, `noFallthroughCasesInSwitch`. Two cheap adds:
`noImplicitReturns` and `noPropertyAccessFromIndexSignature`; try each and
fix what surfaces (expected small). Do not add `erasableSyntaxOnly`
speculatively: nothing needs it. `include` omits `tests/`, so test files are
never typechecked (`bun test` strips types); adding `tests` to `include` is
the highest-value change here and needs a trial run to see how many errors
it reveals.

### 8. Any-casts are rare, and the three there are honest (low, S)

No `any`, `@ts-ignore` or `@ts-expect-error` in `src/` outside generated
code. Three `as unknown as`:

- `Teammate.tsx:154` and `:488` write `{ avatar: null }` and a voice reset
  through `Partial<Persona>` because `exactOptionalPropertyTypes` forbids
  `null` for a field the contract types as optional. If the contract made
  these `Option<Option<T>>`-style clears, the casts would go; that is a core
  change, not a window one. Leave as is, but see Phase 2.
- `SentFile.tsx:203` feature-detects `Uint8Array.fromBase64`, which
  lib `ES2023` does not declare. Correct.

### 9. Test layout (low, M)

54 files, 6.9k lines, flat in `ui/tests/`, running under `bun test`. 22
files each construct their own happy-dom `Window` (`tests/search.test.tsx:2`
and `:52`, `tests/desk-models.test.tsx:2`, and so on) with the same
install/teardown by hand and no shared preload or `bunfig.toml`. A
`tests/dom.ts` helper (build window, assign globals, return teardown) removes
the repetition without a test-framework change. Files are named by feature,
and `.ts` (pure) versus `.tsx` (DOM) is already a usable split; do not add
folders for 54 files. Components under split (finding 3) have DOM tests
(`settings.test.tsx`, `team.test.tsx`) that import by component name, so
re-exporting from the original path keeps them green during the split.

### 10. `generated/contract.ts`: how it is made, and the hazard (low, S)

The file is gitignored (`ui/src/generated/contract.ts` is untracked in this
checkout) and written by the ts-rs export tests under
`cargo test -p hotline-core`; `.cargo/config.toml` sets the output
directory. `AGENTS.md` and `docs/development.md` warn that a filtered
`cargo test session` rewrites only the matched types, leaving a file that
fails `tsc`. The hazard is real and documented; what is missing is detection
close to the cause:

- `bun run typecheck` fails with a pile of "has no exported member" errors
  that do not name the cause. A 5-line `ui` preflight (or a check in
  `make ui-check`) that confirms the file exports a known sentinel type
  and, if not, prints "contract.ts is partial: run `cargo test -p
  hotline-core` unfiltered" would turn that into one clear line.
- Because the file is ignored, a fresh clone of the window has nothing to
  typecheck against until Rust has run once. Say so in `ui/` docs.

### 11. Orphan file that is not orphaned (informational)

knip flags `src/computerViewer.css` as unused; `computer.html:7` links it
directly. Add `computer.html` as a knip entry if knip is ever kept, or leave
knip ad hoc as done here.

## Leave alone

- **`voice/call.ts` as one class.** The 860-line `Call` is a state machine
  over audio, transport and turns whose members all touch the same fields.
  Splitting it moves the coupling into an interface without removing it, and
  `voice-call.test.ts` and four other voice tests pin the current surface.
- **`strict`-level tsconfig.** Already stricter than the usual default;
  the adds in finding 7 are small and optional.
- **`generated/contract.ts` unused types.** They are the contract. Pruning
  them in the generator would couple ts-rs output to what one client uses.
- **ESLint.** Not adoptable until typescript-eslint supports TS 7.
- **A formatter.** No configured one exists and the style is uniform; a
  formatter pass is a diff with no invariant.
- **`Uint8Array.fromBase64` cast** (`SentFile.tsx:203`): feature detection,
  correct as written.
- **`bun test` and the flat `tests/` folder** at 54 files.
- **`noNonNullAssertion` hits** (86): most follow `noUncheckedIndexedAccess`
  after an explicit bound; replacing them with throws adds noise.

## Phase 2 tasks

Each lands alone behind `make check`.

1. **Delete the dead values** listed in finding 4 (30 values, plus
   `pacing.ts`/`reveal.ts` if nothing imports them) and drop `export` from
   `windowTitle` and `paced`. Re-run knip to confirm zero.
2. **Unexport the 15 unused non-generated types** from finding 4.
3. **Share the seen-record storage helpers**: add `storage.ts` with
   `storedNumbers`/`storeNumbers`; replace `App.tsx:912-935` and
   `Teammate.tsx:1814-1839`. Test: a round trip and a corrupt value.
4. **Typecheck `tests/`**: add `tests` to `tsconfig.json` `include`, fix
   what surfaces. If more than ~30 errors, land the config with the fixes
   split by test file.
5. **Contract preflight**: make `make ui-check` (or a `pretypecheck`
   script) fail with one sentence when `generated/contract.ts` is partial.
6. **Tests: one happy-dom helper**: `tests/dom.ts`, converted file by file
   (22 files); pure refactor, same count of tests.
7. **Adopt Biome for lint only**: `biome.json` with the finding-6 rule set,
   `lint` script, `make ui-check` step. Land with the rules that are clean
   today and the others at `warn`; fix `stepText.ts:14`, the two
   `useIterableCallbackReturn` and two `useExponentiationOperator` hits in
   the same change.
8. **Triage the 52 `useExhaustiveDependencies` hits**, one file per commit,
   `App.tsx` first; then flip the rule to error. Each real stale closure
   gets a failing test first.
9. **Split `App()`**: extract `useNavigation`, `useSeen`, `useWindowHotkeys`
   in that order, one per landing.
10. **Split `Settings.tsx`**: move providers and MCP pages to
    `components/settings/`; `settings.test.tsx` keeps importing from
    `Settings.tsx`.
11. **Split `Transcript.tsx`**: pure folds to `transcriptFolds.ts` first
    (testable without DOM), then the step renderers, then the human-ask cards.
12. **Split `Teammate.tsx`**: `ComputerRows` and its row widgets, then
    `Threads`, then `ToolLedger`.
13. **Icons are decorative**: `aria-hidden` in the icon wrapper
    (`icons.tsx`), clearing the 37 `noSvgWithoutTitle` hits in one change.
14. **a11y pass**: the 9 `noAutofocus`, 8 `useSemanticElements` and 8
    `noStaticElementInteractions` hits, as separate small changes.
