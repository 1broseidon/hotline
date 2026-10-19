# ADR 0006: A server can just relay, and a desktop can pair with it only to relay

- Status: Accepted (0.40, landing over epic-11)
- Date: 2026-10-17
- Builds on: [ADR 0002](0002-a-served-desk-relays-sealed-records.md) (the
  relay), [ADR 0003](0003-linked-desks.md) (linked desks and `/v2/link`)
- Method: [security.md § The method for a new capability](../security.md#the-method-for-a-new-capability)
- Tracking: epic-11 (task-91 the role, task-92 the boot, task-93 and task-98
  the desktop, task-94 the phone's contract, task-95 the Pi guide, task-99
  the proof)

## Context

`hotline serve` always boots the whole desk: the room, its scheduler, the
agent drivers, the computer runtime, the model catalogue and the provider
keys. A desktop can stand in on a server's relay only as an **owner**
(`remote/server.rs`, the check above `refused` in `sealed_session`, and
again in `relay::accept`), and `DeviceRole` is Owner | Companion. So using a
Raspberry Pi as a relay means running an agent server on it and handing the
desktop full control of it. George, 2026-10-17: "what if I only want to
allow relay capability, like if the relay lives on a pi and I don't want to
run agents on there."

ADR 0002 already said reachability is not authority, and left "a
registration grant narrower than OWNER" open. ADR 0003 answered it for
desk-to-desk messaging with a seat of its own. This decides it for the relay.

There are two separate questions, and they get separate answers: what a
server runs, and what a device paired with it may do there.

## Decision

### What a server runs: its capabilities

A served desk has a set of **capabilities**, chosen when it is served:

- `agents` — the desk: room, scheduler, teammates, drivers, computer,
  models, provider keys, the wire at `/v2`, `/v2/computer/<id>/ws` and its
  own `/v2/link`.
- `relay` — ADR 0002's relay: `/v2/relay`, `/v2/relay/accept`, and the
  visitor routes `/relay/<deskId>/v2…`.

`hotline serve --capabilities agents,relay` sets them. At least one is
required. The set is written to `serve.json` in the data directory
(`{"capabilities":["relay"]}`), so the mode belongs to the data, not to the
unit file: a boot with the flag uses the flag and records it if it differs,
saying so on stderr (`capabilities: relay → agents,relay`); a boot without
it uses the record; with neither, it is `agents,relay`, so every existing
server is unchanged. A desktop's own Remote is not a served desk and has no
capabilities; it never hosts a relay, as today.

A route belongs to a capability, and `Purpose::of` answers a route only
when the server runs that capability (it already answers the relay routes
only on a served desk). A route of a capability the server does not run is
`404 not_found` before any Noise, exactly as an unknown path is.

### What a relay-only boot omits

`hotline serve --capabilities relay` takes the room lock (one process per
data directory) and claims the secret store, because Remote's TLS and Noise
identities live there. Then it opens Remote — listener, admission, pairing,
the relay hub — and a loopback door. It does **not** open the room stream,
the vault, the index, the scheduler, any session or driver, MCP, skills,
the computer runtime, voice, the model catalogue or the importer, and it
reads no provider key. Nothing under the room's directories is created or
touched.

Its door answers `status`, `pair`, `devices` and `revoke`, so the person
administers the relay from the server's own terminal. `links`, `unlink` and
`wire` are refused with a sentence saying the server runs only a relay.
`hotline status` reports the capabilities.

### What a device may do there: the `relay` grant role

`DeviceRole` gains a third role, `relay`, beside owner and companion. A
relay grant may do exactly three things, and each is ADR 0002's surface
already:

1. **Stand in:** open the sealed control socket `/v2/relay` with
   `{purpose:"relay", deskId}`.
2. **Accept:** claim a visit's capability at `/v2/relay/accept` with
   `{purpose:"relay-accept", capability}`, from the same key that holds the
   generation.
3. **Link through:** nothing more is needed. A linked desk reaches the
   standing desktop's `/v2/link` as a visit (`/relay/<deskId>/v2/link`), and
   a visit's authority is the end-to-end Noise handshake with the desktop,
   never a grant on the relay.

A relay grant has no seat. It may not open `/v2`, `/v2/computer/…` or the
server's own `/v2/link`, so it cannot run a command, subscribe to a stream,
read a conversation or a file, or change a setting. An owner grant keeps
standing in where the server relays: nothing an owner could do is taken
away by this decision.

Enforcement, all in the core and none in a model:

- **`Purpose::of`** — capability routes (above).
- **`sealed_session`** — the owner-only check (`Purpose::Relay &&
  role != Owner`) becomes a table of which roles each purpose admits:
  `Wire` and `Computer` admit owner and companion; `Relay` admits owner and
  relay. It runs after Noise message 1 authenticates the key and before any
  seat, wire or relay registration exists.
- **`relay::accept`** — its own owner check admits owner and relay, under
  the same device-and-generation check.
- **`Seat::for_phone`** — never yields a seat for a relay grant. A relay
  phone reaching it is a bug; task-91 makes the type say so, not a match
  arm that guesses a seat.
- **Admission** (`remote/admission.rs`) is unchanged: control sockets take
  the hub's own 64, accept callbacks give up their pending permit after
  the claim, and a relay grant counts toward the 16 keyed devices.

A key refused for its role, rather than unknown, is answered in the sealed
message 2 with `{"error":"role_not_permitted"}`. It is not
`device_not_authorized`: the client must not mark a working relay pairing
revoked because something tried a door that role does not open.

Revocation is the grant's existing lease: `hotline revoke` cancels the
device's token, which ends its generation, its pending visits and its
joined sessions (ADR 0002 § Lifecycle).

### The old-reader story: a relay grant is kept where an old reader does not look

`RemoteDevice.role` is a serde enum inside `remote.json`'s `grants`. A desk
from before this decision meeting `"role":"relay"` there fails to parse the
whole file: a served desk stops at "remote access settings did not open",
the app loses Remote, and `push_targets` silently sends nothing. Worse
answers were available and are refused: a missing role reads as owner on
every old reader, and `companion` would grant the phone command set.

So relay grants are saved in their own list, `relayGrants`, beside
`grants`, with the same `Grant` shape. `Saved` does not deny unknown fields,
so an old reader ignores the list entirely: it never authenticates a relay
key (that key is unknown to it, and gets `device_not_authorized`), it never
treats one as an owner, and every other pairing loads as before. A key is
in one list at most; re-pairing it moves it. The 16-device limit counts
both lists.

The cost is stated: an old reader that saves `remote.json` (pairing,
revoking, a settings change) writes it without `relayGrants`, so after a
downgrade that writes, the relay pairings are gone and each desktop pairs
again. That fails closed. A role added after this one follows the same
rule: never write a role where a reader that does not know it reads roles.

The desktop applies the same rule to its own record of servers: a server
paired as a relay is not written into `desks.json`, where an older window
would list it as a desk and dial its `/v2`. It is kept in `relays.json`
beside it, which the shell alone writes and the core reads to stand in.

### A pair claim declares its purpose

The relay invitation is a pairing window with role `relay`, opened by
`hotline pair` on a relay-only server (its only kind; `--companion` is
refused there) or by `hotline pair --relay` on a server that also runs
agents. The pair claim, today `{secret, name}`, gains an optional
`purpose`, and the window's role and the claim's purpose must agree:

- A relay window admits only a claim with `purpose:"relay"`.
- An owner or companion window refuses a claim that declares `relay`.

A mismatched claim makes no grant and does not consume the window, so the
desktop can still use the code within its two minutes. It is answered in
the sealed message 2 as `{"refused":"relay"}` (this invitation makes relay
grants) or `{"refused":"device"}` (this one does not), and counts toward the
pairing throttle as any failed claim does. So a phone that scans a relay's
code — an old phone that never sends `purpose`, or a new one ignoring the
marker below — gets no grant, and a new phone can say why. The pair answer
for a relay grant is `{"role":"relay", deskName, deskId, serves}`.

The claim's `Claim` struct denies unknown fields, so an old server meeting
`purpose` refuses the claim. A desktop sends `purpose` only for a relay
invitation, which an old server never issues, so nothing that works today
stops working.

### The QR and the link say it is a relay

The invitation's role is already in both forms: `r=` in the QR query and
`role` in the `hotline://pair?p=` payload. A relay invitation says
`r=relay` and `"role":"relay"`; nothing else is added. A phone that reads
`r=relay` says the code is for Hotline on a desktop, to use this server as
its relay, before it dials anything (task-94 writes that contract into
wire.md for hotline-mobile). An old phone reading `r=relay` may guess
another role, and its claim is then refused as above. An old desktop
reading a link with `"role":"relay"` fails to parse it and says the link is
invalid, which is a refusal too.

### Discovery: the server says what it serves

Where a device learns about the server, the server says which capabilities
it runs, as `serves` (`["relay"]`, `["agents","relay"]`). The field is not
called `capabilities` because the hello already uses that word for the
seat's commands.

- **The pair answer**, for every role, so a desktop records a relay as a
  relay from the moment it pairs.
- **The relay's `ready`** on the control socket, the only frame a relay
  grant ever reads.
- **The v2 hello**, beside `deskKind` and `version` and, like them, for an
  owner only. A relay-only server has no hello: it has no wire.

### Switching modes

The mode changes by serving again with another `--capabilities`.
`remote.json` is not touched, so every pairing survives the switch, and
each grant keeps its role:

- **relay → agents,relay** opens the room for the first time, as a new
  served desk: no teammates, no provider keys, no models. Those are set up
  as for any new server ([serve.md](../serve.md)); having relayed buys
  nothing there. A desktop that wants to use the teammates pairs again as
  an owner (re-pairing the same key replaces its relay grant).
- **agents,relay → relay** leaves the room on disk, unopened and unchanged.
  Owner and companion grants stay saved; an owner may still stand in, and
  `/v2` is `404` until agents return.
- **dropping relay** makes `/v2/relay` `404`; desktops standing in back off
  and keep trying, and the relay grants stay saved for a later return.

### Migration: an offer, made by the device over its own grant

A desktop paired as an owner only so it could use the relay is over-trusted:
a stolen desktop key could run the server's teammates and read its files.
The desktop offers to drop that grant to `relay`, when it stands in on that
server and the server has no teammates; a server whose teammates are used
stays owner, and is not offered. Declining changes nothing, and nothing is
ever moved automatically, by either side.

Accepting sends one owner-seat command over that device's own wire. The
grant it changes is the authenticated caller's, taken from the handshake;
the command takes no device id, so no device can change another's grant.
The core moves the grant from `grants` to `relayGrants` keeping its id,
key, name and pairing time, then ends the device's sessions as a revocation
would. The desktop stands in again under the new role and records the
server as a relay. Going back to owner is `hotline pair` on the server,
from its own terminal: the narrower grant cannot widen itself.

### Security record

- **Default:** a served desk runs `agents,relay`, exactly as today. A relay
  grant exists only when someone opens a relay invitation and a claim that
  declares relay takes it. No existing grant changes role.
- **Grant source:** the person at the server's terminal (`hotline pair` on a
  relay-only server, `hotline pair --relay` otherwise), and the device
  itself narrowing its own owner grant.
- **Enforcement points:** `Purpose::of` (capability routes), the role table
  in `sealed_session` (replacing the owner-only check), `relay::accept`,
  `claim_v2` (purpose against the window's role), and the narrowing
  command's handler (the caller's own grant only). Revocation through the
  grant's cancellation token.
- **Tests:** each child proves its door allowed and denied through the real
  handler: a relay grant stands in and is visited, and is refused at `/v2`,
  `/v2/computer` and the server's `/v2/link` with `role_not_permitted`; a
  phone's claim on a relay window and a relay claim on an owner window make
  no grant; an old-shaped reader loads a `remote.json` with `relayGrants`
  and authenticates nothing from it; a relay-only boot creates no room; a
  switch to agents keeps every grant (task-99, end to end on a Pi's mode).
  security.md indexes them as they land.
- **Residual risk:** a relay grant may still stand in for any desk id, as an
  owner may today (ADR 0002): denial of service at worst, since visitors pin
  the desktop's key. A downgrade that saves `remote.json` drops relay
  grants. An old phone shown a relay's code learns only that its claim
  failed. The relay learns what ADR 0002 says it learns.

## Consequences

- A Pi can relay for a desktop with no agent machinery, and a stolen
  desktop key there can stand in and nothing else.
- Three forms change shape: `DeviceRole` gains `relay` (the generated
  contract, `remote.devices`, the pairing payload), the pair claim gains
  `purpose`, and `remote.json` gains `relayGrants`. A phone build that
  decodes `remote.devices` strictly will fail on a relay row; task-94 tells
  hotline-mobile to read an unknown role as one it cannot act on.
- `serve.json` is a second settings file in a served data directory. It
  holds only what must be known before the room is opened, which is why it
  is not in the room stream.
- Tape and segment shapes are untouched.

## Not decided here

- A relay-only server choosing a relay itself, so a private server is
  reachable (still open from ADR 0002).
- Limiting which desk ids a relay grant may stand in for.
- Relays for anything other than a desktop: a phone never stands in.
