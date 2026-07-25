# @1broseidon/hotline-harness-core

Shared plumbing for hotline's TypeScript harnesses (`harness/pi`,
`harness/claude-sdk`): the `hotline run` child manager (spawn/respawn/backoff,
fatal classification), the hand-rolled JSONL JSON-RPC client, inbound
`<channel>` envelope extraction, an async FIFO queue, session-id persistence,
the auth precedence report, and tagged stderr logging.

**Zero runtime dependencies** (node builtins only) — that is what makes it
trivially consumable by both harnesses: nothing to install, nothing to resolve,
it is just code the workspace links in.

## Consuming

Import by subpath, never by relative path across packages:

```ts
import { ChildManager } from "@1broseidon/hotline-harness-core/child";
import { timeoutsFromEnv, mcpInitialize } from "@1broseidon/hotline-harness-core/jsonrpc";
import { createLog } from "@1broseidon/hotline-harness-core/log";
```

Harness identity is parameters, not forks:

| parameter | pi | claude-sdk |
|---|---|---|
| `clientName` (MCP clientInfo) | `hotline-pi` | `hotline-claude-sdk` |
| timeouts env prefix | `HOTLINE_PI` | `HOTLINE_SDK` |
| log tag / file env | `hotline-pi` / `HOTLINE_PI_LOG` | `hotline-sdk` / `HOTLINE_SDK_LOG` |
| `installHint` | "See the hotline-pi README." | "See the harness/claude-sdk README." |
| session file name | n/a (`--session-id`) | `claude-sdk-session.json` |

Local development uses the npm workspace at `harness/` (`npm install` there
symlinks this package into both consumers). Build order: core first —
`npm run build -w core` before `-w claude-sdk` (the root `build` script does
this).

## Not published

This package, and every other harness package here, carries `"private": true`
and is never published to a registry. hotline distributes one signed binary from
`hotline.dev/install.sh`; the harnesses are built from source in this checkout
and loaded by explicit path (`HOTLINE_CLAUDE_SDK_ENTRY` for the SDK harnesses,
`pi install ./harness/pi` for the Pi extension). The npm workspace above is a
local development tool, not a distribution channel.

## Parity

`harness/PARITY.md` is the capability-by-harness ledger; update it in the same
commit as any harness change. `npm test` here runs `scripts/check-parity.mjs`,
which fails if the old pi copies of child/jsonrpc/log reappear or a core module
loses its PARITY.md row.
