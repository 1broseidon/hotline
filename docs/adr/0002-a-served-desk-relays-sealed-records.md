# ADR 0002: A served desk relays sealed records

- Status: Accepted
- Date: 2026-10-04
- Reviewed: Toad, 2026-10-04, at `6b275f1`: sound with changes
- Design: [serve.md § Relay](../serve.md#relay)
- Tracking: BRO-206 (epic), BRO-210 (hardening), BRO-211 (collaboration)

## Context

A phone reaches a desktop directly: on the same network, or over a VPN to
the desktop's Remote listener. Away from home that means a VPN on the phone,
and every byte of the desk's wire crossing it. A served desk (`hotline serve`)
already has a public HTTPS address, and desktops already pair with one as
devices, through `remote::client`, to use its teammates.

The phone's trust in a desk is the desk's Noise key, pinned at pairing. TLS
is only a carrier on protocol 2: the phone accepts any certificate, and a
proxy in between sees the handshake payloads and frames only as ciphertext.

## Decision

The served desk is the relay; there is no separate relay service and no
multiplexing protocol. Each relayed session is two sockets joined record for
record.

### Standing in

A desktop with a relay chosen dials out to that served desk over the sealed
control socket `/v2/relay` (payload `{purpose:"relay", deskId}`), as a device
paired as an **owner**, and stands in there under its own desk id. The relay
answers `ready` with the desk's relay URL, then sends `visit` notices and a
ping every 25 s.

Each registration is a **generation**. A newer registration for the same desk
id replaces the older one; everything pending on the older generation is
invalidated with it.

### A visit

1. A visitor dials `https://<server>/relay/<deskId>/v2…`, the same paths it
   would use on the desk itself (`/v2`, `/v2/pair`, `/v2/computer/<id>/ws`).
2. The relay admits the visit against the desk's budgets (below), mints a
   single-use capability bound to the current generation with an expiry,
   and sends `{type:"visit", id, path}` over the control socket.
3. The desktop answers with a **sealed claim**: a Noise IK handshake to the
   relay's own pinned key at `/v2/relay/accept`, from the same device key that
   holds the generation, carrying `{purpose:"relay-accept", capability}`. The
   relay checks the device, the generation, and the expiry when it claims,
   not only in a cleanup timer.
4. After the claim's two handshake messages the outer channel ends, and the
   socket carries the visitor's records as they are, binary only, in both
   directions. The desktop then runs `sealed_session` on it exactly as it
   would for a socket on its own listener.

The visitor's Noise handshake runs end to end with the desktop. Pairing,
revocation and the computer viewer are unchanged.

### Budgets

- **On the relay:** control sockets have their own pool of 64, separate from
  the 16 authenticated admission seats, so relay hosting cannot starve
  direct devices and is not starved by them. A replaced control socket keeps
  its pool slot until cleanup completes. At capacity, replacing a desk
  cancels its predecessor and waits for that slot within its pending TCP
  deadline; a new desk is refused. The pending permit is held until the
  control slot is acquired, so replacement waits cannot grow unbounded.
  - Each desk id has a stable budget of 16 sessions (waiting or joined) that
    survives re-registration.
  - There is a relay-wide ceiling of 256 sessions and a per-desk visit rate
    (30 a minute, burst 10; excess visits return `429 desk_busy`).
  - The stable budget registry holds at most 1,024 desk ids. Inactive ids
    are discarded only when their sessions have ended and their rate burst
    has fully replenished, so changing registrations cannot reset a limit.
  - Visitor sockets give up their pending admission permit once upgraded;
    callbacks do so after the sealed claim finishes. This keeps anonymous
    claims bounded through Noise authentication. Forwarded-IP headers are
    never trusted.
- **On the desktop:** a stable budget of 32 relayed sessions that survives
  `stand()`. A relayed session also takes an authenticated admission seat
  once the inner Noise handshake names the device, so the per-device limit
  (4) and the desk-wide limit (16) apply whichever way a phone arrives.

### Lifecycle

- Every callback task holds the standing generation's cancellation token.
  Choosing another relay or none, turning Remote off, or revoking the
  relay's grant ends in-flight dials and live relayed sessions.
- A callback from an old generation cannot finish after Remote goes off and
  on again.
- Writes and closes on joined and control sockets have a 10 s bound, so backpressure
  cannot hold cleanup.
- Losing the control socket also cancels that generation's callbacks. The
  inner wire receives cancellation through its own revocation cleanup,
  rather than dropping a future that still owns writer/subscription tasks.
- The published relay URL is cleared the moment standing in stops, before
  any reconnect.
- Reconnect backoff resets after a successful `ready` and is jittered.

### Discovery

- The desk's relay URL goes to phones in the pairing QR (`a=`), in
  `PairingPayload.relay`, and in every v2 hello (`endpoints`).
- A phone keeps it beside the desk's own address and dials the relay first,
  then the desk directly after a 4 s grace.
- The Rust client does the same: `pair` tries `payload.url` and then
  `payload.relay`, and a hello's endpoints update the stored `PairedDesk`.
  Only an authenticated hello can change where a desk is dialed.

## Consequences

- A relay can drop or delay a session. It cannot read, forge or redirect
  one: the visitor pins the desktop's key, and only the device that holds
  the generation can claim a visit. A TLS interceptor between the desktop
  and the relay sees ciphertext only and cannot steal a capability.
- A relay learns who connects when, and how much they send, as any proxy
  does. It also learns the desk ids it carries.
- Within today's trust domain, any owner device on a served desk may stand
  in for any desk id, newest first. That is denial of service at worst:
  pinned visitors refuse an impostor's key. Reachability is not authority.
- Phones need a native build that allows a path in a desk's address. An
  over-the-air update alone cannot dial a relay.

## Not decided here (BRO-211)

- **Collaboration across systems** (desktops and served agents talking to
  each other) uses this transport but needs its own authority:
  - a registration grant bound to the desk's identity, narrower than OWNER;
  - scoped collaboration permissions instead of the full desk command set.
- **A served desk choosing a relay itself**, so private servers are
  reachable, is not supported yet.
