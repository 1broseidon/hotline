# Protocol compatibility

The mailbox freeze establishes a current-plus-previous support window. Mailbox-v1 is the current contract target for M1–M3; raw frames-v1 remains the previous shipped mode until a separately reviewed retirement.

## Current-plus-previous wire matrix

| Support slot | Capability negotiation | Relay routes | Payload | Required compatibility behavior |
| --- | --- | --- | --- | --- |
| Current: mailbox-v1 | Optional `hello.capabilities: ["mailbox-v1"]`; `welcome` echoes it | `/b/<slug>/mailboxes` and `/d/<slug>/<mailbox_id>` | Opaque `payload_encoding: "plain-v1"` text containing unchanged frames-v1 JSON | Use addressed mailboxes only after both peers negotiate the capability. A provisioned device does not opportunistically fall back to a broadcast raw socket. |
| Previous: raw frames-v1 | Capability field absent or ignored | `/b/<slug>` and `/d/<slug>` | Direct frames-v1 JSON text | Dual-stack relay and box preserve the raw path during the support window. Old peers ignore additive capability fields. An explicit rollback returns the deployment to this slot and must restore push continuity. |

Mailbox-v1 does not change numeric journal J or any inner frames-v1 payload. Its mailbox M is a separate decimal string. Normal compatibility selection is capability-gated; explicit rollback is an operator action, not per-connection fallback. Presence retirement occurs only after the current-plus-previous window and requires separate approval.

## Historical baseline

Compatibility manifest v0 records the Phase 0 app-channel baseline at its named commit.

| Manifest | Server commit | Protocol frames | Mobile app | Expo SDK | Notes |
| --- | --- | --- | --- | --- | --- |
| v0 | `1d2929fca28d35f384a5a71dfb7f9f6c6c72856b` | v1 | `1.0.0` | 54 (`expo` `54.0.35`) | Historical raw app-channel baseline. Server-side push was off by default; `HOTLINE_APP_PUSH=1` enabled it. |

The mobile app column records the version of the closed-source mobile client
that was current at each manifest. That client is not part of this repository,
so the version is not derivable from anything here — it is recorded by hand when
a manifest is cut. Only the server commit and the protocol frames version are
verifiable from this tree.
