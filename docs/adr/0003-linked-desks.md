# ADR 0003: Linked desks trust each other's keys, not each other's owners

- Status: Accepted
- Date: 2026-10-13
- Design: [design.md § Linked desks](../design.md#linked-desks),
  [security.md § Linked desks](../security.md#linked-desks-bro-177-adr-0003),
  [wire.md § Linked desks](../wire.md#linked-desks)
- Tracking: BRO-177 (epic), BRO-178 (one rail), BRO-179 (the link),
  BRO-180 (the linked roster), BRO-181 (cross-desk messaging); BRO-211
  (collaboration across systems, left open by ADR 0002)

## Context

A window can show several desks: this computer's, and the servers it is
paired with as an owner (BRO-145). That makes the *window* a client of each
desk. It does nothing for the *teammates*: Clementine on the laptop cannot
ask Lou on grizzly anything, because the two desks know nothing of each
other.

The obvious transport is the one already there: the laptop's pairing with
grizzly. But that pairing is an owner grant, and an owner may do everything
the desk may do. ADR 0002 said it plainly for relays: reachability is not
authority. A desk letting another desk's teammates message its own needs a
grant that permits exactly that and nothing else.

## Decision

### A link is a grant between two desks

Each desk records a link as a `desk_link` event on its room stream: the
other desk's id and name, which of the two dials, and — on the desk being
dialled — the other desk's own Noise identity key, which is the only key
that may open the link. The key is the *desk's* (`remote-noise-identity`),
not the device key the owner pairing holds, so ending the pairing does not
end the link and holding the pairing does not open it.

The link is made from Settings › Desks on the laptop, with one action. The
person holds the owner seat on both desks, so that one action consents on
both: the laptop's core opens its owner wire to grizzly and asks
`desks.link_accept` with its id, name and key; grizzly records the link;
the laptop records it and starts dialing. A connected desk is not linked
until someone does this.

It is not a third device role beside owner and companion. A device is a
person's client; a link is two desks. It has a seat of its own instead.

### `Seat::Link` permits the federation commands and nothing else

Grizzly answers the laptop's link channel under `Seat::Link`, through the
same `Seat::permits` the other seats are held to: `federation.roster`,
`federation.deliver`, `federation.unlink` and a ping. Every other command
and every subscription is `forbidden` before it runs. The desk and owner
seats, in turn, may never send a federation command: only the link's own
authenticated channel can speak for a linked desk, and the desk it speaks
for is the one the handshake proved, never one a frame names.

### The desk that made the link dials; the channel then goes both ways

The laptop dials grizzly's sealed `/v2/link` — directly, or through
grizzly's relay when that is how grizzly is reached (ADR 0002's routes
gain `/v2/link`) — with Noise IK under its desk key, binding
`{purpose:"link", deskId}` in the first message. Once open, the channel is
symmetric: either side sends `{id, cmd, params}` and answers the other's
by id. A laptop behind NAT never has to be reached.

It stays up by being dialled again: a closed or failed channel is redialled
with jittered backoff from half a second to thirty, for as long as the link
exists, including after the laptop restarts. Each side publishes its roster
when it changes (looked at every two seconds) and pings every 25; three
quiet pings close it. A refusal of the laptop's key, under grizzly's pinned
identity, is grizzly saying the link has ended, and the laptop ends it too.

### A linked teammate is a participant with nothing granted

Grizzly's teammates are known on the laptop as `grizzlyId/personaId`. They
are never personas in the laptop's roster. What the laptop holds of them is
the roster each side publishes — exactly the rows `list_teammates` gives a
local colleague (id, name, state, activity, what it is working on, when it
last spoke), typed so that no other field can travel — and their ids and
names kept on the link's record, so that a message can be addressed and
queued while grizzly is away.

Where the room's rules read a persona, a linked teammate is one made up on
the spot: its `node` set, its id qualified, and nothing granted. It is never
implicitly trusted, whatever its reach on its own desk. Nothing about its
workspace, tools, model, secrets or computer is known here, and nothing
about the laptop's crosses to grizzly.

### Messages ride both desks' exchange queues, under the recipient's grants

`message_teammate` to a linked teammate is accepted into the sender desk's
durable `exchange_pair` queue like any other. When the queue takes it up it
is forwarded over the link; while the link is down it waits there, queued.
On grizzly it enters grizzly's own queue as a request from
`laptopId/clementine`, where Lou's `allowedSenders` and the person's
first-contact card decide it exactly as for a local sender. The card is
raised on Lou's tape, on grizzly: the sender has no tape there, and the
sender's desk can neither raise, answer nor pre-approve it. A standing
grant is kept on Lou by the qualified id.

An ask runs in a side session of Lou's on grizzly; a handoff in a work
thread of Lou's. The result is sent back over the link — saved, and sent
again until it arrives, if the laptop is away — and is delivered in
Clementine's conversation as its own message, matched by `requestId`. Both
desks count the pair's twelve automatic messages. A request id sent twice is
the same request.

### A thread on each side

Each desk keeps its own copy of the pair's thread, keyed by the two ids as
that desk knows them (`clementine~grizzlyId/lou` on the laptop,
`laptopId/clementine~lou` on grizzly): the request and the answer on the
laptop, the whole exchange on grizzly. The `/` in a qualified id is escaped
in the file name, as every thread key already is.

### Either side ends it, on both

`desks.unlink` (Settings › Desks, or `hotline unlink` on a server) tells the
other desk over the open channel, or, on the dialing desk, by one last dial;
then it ends the link here whatever the other says. Ending it removes the
record, closes the channel, fails every message waiting on that desk with a
sentence ("Lou is on grizzly, which isn't linked any more."), withdraws the
cards its teammates were waiting on, stops work being done for them, ends
the standing grants the person gave them, and takes their teammates out of
`list_teammates`. A link made again starts from nothing.

## Consequences

- A compromised linked desk can publish any roster and send any message,
  under any of its own teammates' names. It can do no more than that: every
  message still meets the recipient's grants and the person's card, and is
  quoted to the recipient as a colleague's data, not instructions.
- A desk that answers a linked request uses the recipient's own reach,
  tools and computer, as for a local colleague, and may say anything in its
  answer that it would say to one.
- If the laptop ends a link while grizzly cannot be reached, grizzly keeps a
  link that is never dialled again, shown as offline, until someone ends it
  there too. The reverse cannot happen: the dialing desk learns at its next
  dial.
- Stopping an exchange on one desk does not stop work already running for it
  on the other; the answer, when it comes, is dropped.
- A sender waiting on a card on the other desk sees its message as sent, not
  as waiting for the person.
- One more listener route, `/v2/link`, on every desk with Remote on. It
  admits only a key a link record names, under the existing admission
  limits.

## Not decided here

- Files in cross-desk threads: `send_file` and attachments do not cross.
- The voice dispatcher handing work to a linked desk.
- The phone's view of links.
- Linking two servers with no desktop in between: a server dials only a
  desk it is paired with, and there is no way yet to pair a server to
  another as a client.
