# Computer

The computer is a containerized Linux desktop that a teammate drives through
one MCP server. It is local infrastructure rather than a special agent type:
Toad grants the machine's `/mcp` endpoint to a teammate like any other server,
and the person can see and drive the same desktop through the viewer the
machine serves.

The agent and the image live in their own repository,
[toad.computer](https://github.com/1broseidon/toad-computer), and release on
their own schedule. Toad pins one published tag, `COMPUTER_VERSION` in
`crates/toad-core/src/computer/mod.rs`, and never `latest`.

## The contract Toad relies on

- `/health` is an open liveness probe on port 8787.
- `/mcp` is streamable HTTP MCP; with `TOAD_COMPUTER_TOKEN` set, every method
  wants `Authorization: Bearer <token>` and otherwise gets a JSON 401.
- `X-Computer-Holder` names the teammate for leases and queued runs.
- `/` is the viewer page and `/ws` its socket, which takes the same token as a
  `token` query. Toad opens `http://127.0.0.1:<host port>/#<token>`; the page
  reads the fragment and the token never appears in a request line.
- Nothing in the image runs as root, so Toad creates the container with
  `--cap-drop=ALL` and nothing added back, `no-new-privileges`, memory and PID
  limits, a sized `/dev/shm`, and the one port published on loopback.
- The workspace is mounted at `/home/agent/workspace`.

## Lifecycle

Wake is on session start: `ensure_running` pulls the image when absent, creates
the container with a token generated for it, starts it, and waits for
`/health`. The token lives in the container's private process environment, never
in room settings. Toad recovers it from runtime inspection after an app restart,
so an existing computer keeps its jobs and viewer. A container without a
recoverable token is recreated. Idle uses the room's sweep:
thirty minutes after a session stops the container is stopped and its rw layer
kept; seven days and it is removed.

The runtimes, how they are found, and the fake runtime the tests drive are in
[development.md](development.md).
