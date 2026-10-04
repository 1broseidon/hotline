# ADR 0002: A served desk relays sealed records

- Status: Accepted
- Date: 2026-10-04
- Design: [serve.md § Relay](../serve.md#relay)

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

The served desk is the relay; there is no separate relay service.

- A desktop with a relay chosen dials out to that served desk over the
  sealed control socket `/v2/relay`, as a device paired as an **owner**,
  and stands in there under its own desk id. Only an owner's device may.
- A visitor dials `https://<server>/relay/<deskId>/v2…`, the same paths it
  would use on the desk itself (`/v2`, `/v2/pair`, `/v2/computer/<id>/ws`).
- The served desk hands the desktop a single-use capability over the
  control socket. The desktop dials `/relay/accept/<capability>` back, and
  the relay joins the two sockets record for record, binary only.
- The Noise handshake inside runs between the visitor and the desktop,
  through the same `sealed_session` the desktop's own listener uses. Pairing,
  revocation and the computer viewer are unchanged.
- The desktop's relay address goes to phones in the pairing QR (`a=`) and
  in every v2 hello (`endpoints`). A phone keeps it beside the desk's own
  address and dials the relay first, then the desk directly.

## Consequences

- A relay can drop or delay a session. It cannot read, forge or redirect
  one: the visitor pins the desktop's key, not the relay's, and the
  capability only joins a socket the desktop already chose to answer.
- A relay learns who connects when, and how much they send, as any proxy
  does. It also learns the desk ids it carries.
- Another owner's device on the same served desk can stand in for a desk id
  it does not own. That denies service to the desk's visitors, who still
  refuse its key, and is bounded to people the server already trusts with
  everything.
- The server bounds the desks it stands in for (64) and each desk's visits
  (16); the desktop bounds the relayed sessions it answers (32). Relayed
  sockets hold no admission seat once joined.
- A desktop paired with another desk through the Rust client also falls
  back to that desk's relay. That is the path for desktops and served
  agents to reach each other across systems.
- Phones need a native build that allows a path in a desk's address. The
  over-the-air update alone cannot dial a relay.
