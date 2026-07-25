# Protocol fixtures

These shared goldens characterize frames-v1 payloads and mailbox-v1 transport controls. Each JSON file contains a prose `intent` and ordered `frames`; every frame has a direction and exact wire object. Server-to-app entries carry the app-facing `expect` event asserted by the mobile parser suite. Mailbox fixtures add frozen-rule `assertions` and per-step transport outcomes for M1/M2/M3 to implement; M0 validates their static JSON and schema shape without adding mailbox runtime logic.

- Go: `go test ./internal/app`
- Mobile: the mobile client's parser suite (`node --test`) walks the same
  inventory from its own repository

Both suites walk the same inventory and validate every wire object against `protocol/schema/frames-v1.schema.json` with dependency-free validators covering the schema's current assertion keywords. They JSON-round-trip every mailbox control, validate each `plain-v1` fixture payload as an inner frames-v1 object, and prove `mailbox_error` is marked for transport routing before `parseFrame`. History paging and upload transport remain HTTP contracts; their WebSocket frame results are represented by replay and attachment fixtures here.

## Inventory

- `artifact.json`
- `attachment.json`
- `buttons-tap.json`
- `edit.json`
- `error.json`
- `mailbox-bootstrap.json`
- `mailbox-dedup.json`
- `mailbox-enqueue-sync-ack.json`
- `mailbox-gap.json`
- `mailbox-isolation.json`
- `paced-reply.json`
- `pairing.json`
- `react-both-ways.json`
- `relay-photo.json`
- `relay-presence.json`
- `send-echo-ack.json`
- `staged-photos.json`
- `welcome-replay.json`
