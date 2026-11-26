# Phone contract

The rules the desk and the phone (`../hotline-mobile`) are built by. Message
shapes are generated from `crates/hotline-contract` into
`ui/src/generated/contract.ts`, which the phone copies with
`npm run sync:contract`. This document covers what the types cannot say: who
owns which rule, how settings move, and how a change gets made. It lives
here and is copied into the phone repo the way `contract.ts` is.

Each rule names the test that enforces it, or says it has none yet. A rule
with no test is a promise, not a guarantee.

## Why

The phone read the owner's call settings from the room stream, which a phone
is sent trimmed (newest 200 events, 512 KB of JSON:
`crates/hotline-core/src/wire/phone.rs`). A folded setting keeps the slot it
was first written in (`fold` in `crates/hotline-store/src/log.rs`), so an
old `voice`, `mcpServers` or `enabledModels` fell outside the window. The
phone showed its own defaults, then wrote back a whole key built from half a
setting, and the desk stored it whole. Each step was a rule that lived on
both sides, or on neither.

## Rules

**R1 · The desk owns every default and rule.** The phone shows what the desk
resolved and never works it out again. Today the phone keeps copies:
`frontOf` and `FRONTS` (`src/use-for-kit.ts`), the idle-hours limits and the
harnesses shown above the fold (`src/desk-settings.ts`). These move to the
desk. The phone decides layout only: sizes, order on screen, copy.

**R2 · Settings never travel as history.** The phone reads settings from a
view that sends resolved values and their changes, not from the room
stream's history. Until that view exists (0.51), a phone's room snapshot
carries every live `setting` outside its window and budget
(`phone_room` in `crates/hotline-core/src/wire/phone.rs`).

**R3 · Writes are narrow, and the desk merges them.** The phone changes one
thing per command, as `skills.offer` and `persona.pin` already do, and never
sends a whole object it holds only part of. `persona.update` already folds
its patch over the stored record on the desk. Until the narrow commands
exist, a whole-key `settings.update` starts from a fresh full read
(`settings.update` with `{patch: {}}` answers every setting), and the phone
refuses to write when that read fails.

**R4 · Features switch on capability.** The phone uses a command only when
the desk's hello advertises it (`PHONE_CAPABILITIES` in
`crates/hotline-core/src/wire/socket.rs`, plus `Seat::capabilities` in
`wire/seat.rs`). It never infers the desk's age from which fields are
present, as `servesCalls` (`src/use-for-kit.ts`) does today. A capability is
removed only in a version that says so. The phone also checks a
`providerOff` capability that this desk never advertises, and offers Off
for calls only because `servesCalls` guesses (`src/desk-choice.ts`).

**R5 · One schema, generated.** Shapes come from `hotline-contract` through
ts-rs. Neither side edits `contract.ts` by hand. On the desk, precheck
regenerates it from nothing and fails on a difference (`leg_contract` in
`scripts/precheck`). The phone's copy is planned to record the desk commit
it came from, and phone CI to fail when it does not match.

**R6 · Shared fixtures.** Planned: golden messages and "these settings
resolve to this" cases in `contract/fixtures/*.json`, written by desk tests
and read by phone tests, with a rule's fixture changing in the same PR as
the rule. None exist yet.

**R7 · Pairing gives full access.** Pairing gives full access by default,
like the desk itself: an invitation with no role named is an Owner
(`pairing` in `crates/hotline-core/src/remote/v2.rs`). The Companion role is
kept but not promoted, and is a candidate for removal. A Companion may not
subscribe to the room or change settings.

**R8 · Wire changes start here.** One desk PR changes the contract crate,
this document and the fixtures together; the phone follows with a sync.
Every rule links to its test, or is marked as having none.

## Rules and their tests

| Rule | Test | Status |
| --- | --- | --- |
| R1 | Planned: a phone lint that fails on desk-rule modules, and fixture tests (R6) whose expected screen value is the desk's resolved field. | Not yet enforced |
| R2 | `crates/hotline-core/src/wire/tests/windows.rs` `a_phones_room_snapshot_carries_every_live_setting_past_the_window`, `the_desk_and_an_owner_desk_still_get_the_whole_room`; `crates/hotline-core/src/wire/tests/phone_pressure.rs` `a_paired_phone_subscribing_to_the_room_gets_settings_older_than_its_window` | Partial: the room snapshot. Planned for 0.51: a phone seat's settings view holds every key the full fold holds, on a room of 2,000 events. |
| R3 | Planned: a desk test that two narrow commands on different parts of one key both survive, and a phone test that no write is built from the room-stream fold. | Not yet enforced |
| R4 | Planned: every capability the phone checks appears in the desk's advertised list. | Not yet enforced |
| R5 | `leg_contract` in `scripts/precheck` (the desk's copy). Planned: `sync:contract --check` in phone CI. | Partial: desk side only |
| R6 | Planned: both repos' CI read `contract/fixtures/*.json`. | Not yet enforced |
| R7 | `crates/hotline-core/src/remote/tests/sealed_network.rs` `missing_roles_migrate_to_owner_but_never_create_an_empty_room_grant`; `crates/hotline-core/src/remote/tests/rust_client.rs` `rust_client_companion_cannot_change_settings_or_subscribe_to_room` | Partial: a stored grant with no role reads as Owner. Planned: `remote.pairing` with no role on a desk makes an Owner invitation. |
| R8 | Review, and `scripts/security-citations`, which fails when a test named here is missing. | Enforced for names, not for content |

## Settings view

Planned for 0.51: a `settings` subscription for every seat. Its snapshot
holds each key as the desk resolves it; after that it sends one change per
write.

```json
{
  "voice": {
    "value": { "…": "what is stored, whole" },
    "isSet": true,
    "resolved": { "front": "…", "tts": "…", "dispatcher": "…" }
  },
  "chapterIdleHours": { "value": 8, "isSet": false, "limits": [1, 336] }
}
```

`resolved` is what the desk will actually do, such as which call model runs,
so the phone needs no rule to show it. Limits and choices come with the
value, so the phone keeps no copy of them either.

## Write commands

Planned for 0.51, each advertised as a capability (R4):

| Today (whole key) | Becomes |
| --- | --- |
| `voice` object | `voice.set {part, value}`, `part` one of front, tts, stt, dispatcher… |
| `mcpServers` list | `mcp.server_upsert {server}`, `mcp.server_remove {id}` |
| `enabledModels` record | `models.enabled_set {provider, ids}` |
| single values (idle hours, default harness, runtime…) | stay `settings.update`, with limits checked on the desk, not only on the phone |
| teammate edits | stay `persona.update`, which merges its patch on the desk |

## Order of work

1. Phone hotfix: every write to `voice`, `mcpServers` or `enabledModels`
   merges into a fresh full read and refuses to write without one; settings
   screens load from that read too. Works with a 0.50.0 desk.
2. Desk: settings outside the phone's window. Done on
   `fix/phone-settings-window` for 0.50.0 final.
3. Desk, 0.51: the settings view, with the resolved call model. The phone
   then deletes `frontOf`, `FRONTS` and the version guesses.
4. Desk and phone, 0.51: the narrow write commands. The phone switches to
   them.
5. Both: fixtures, `sync:contract --check`, and tests for every rule above.
6. Security, with Shield: what the relay can see, and whether the Companion
   role stays.

## Decisions

- The settings-window fix ships in 0.50.0 final.
- The settings view and the narrow writes ship in 0.51.
- This document lives at `docs/contract/phone.md` and is copied to the
  phone repo like `contract.ts`.
- Pairing's default stays Owner.
