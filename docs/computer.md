# Computer

The computer is a containerized Linux desktop that a teammate drives through
one MCP server. It is local infrastructure rather than a special agent type:
Hotline grants the machine's `/mcp` endpoint to a teammate like any other server,
and the person can see and drive the same desktop through the viewer the
machine serves.

The agent and the image live in their own repository,
[Hotline Computer](https://github.com/1broseidon/hotline-computer), and release on
their own schedule. A new computer is created on the newest published release
on the desk's major line: Hotline asks the repository's releases endpoint when
the room opens, every six hours after, once more when a computer is about
to be created with nothing known yet, and whenever the Check now button
under Settings → Computer is pressed. That page also offers every published
release from the floor up as the room's image, with Newest the default; a
picked release is written as the full image reference and is a pin like any
other. `COMPUTER_VERSION` in
`crates/hotline-core/src/computer/mod.rs` is the floor — the release this desk
was tested with, what a computer is created on offline, and the line under
which nothing is offered — and never `latest`. A pinned image, the
teammate's or the room's, is used as written and never looked up. The desk
learns the release a running computer actually is from the guide it serves
(below), and when that is behind the newest, the teammate's pane offers the
update.

## The contract Hotline relies on

- `/health` is an open liveness probe on port 8787.
- `/mcp` is streamable HTTP MCP; with `HOTLINE_COMPUTER_TOKEN` set, every method
  wants `Authorization: Bearer <token>` and otherwise gets a JSON 401.
- `X-Computer-Holder` names the teammate for leases and queued runs.
- `/` is the viewer page and `/ws` its socket, which takes the same token as a
  `token` query. Hotline opens `http://127.0.0.1:<host port>/#<token>`; the page
  reads the fragment and the token never appears in a request line.
- Nothing in the image runs as root, so Hotline creates the container with
  `--cap-drop=ALL` and nothing added back, `no-new-privileges`, memory and PID
  limits, a sized `/dev/shm`, and the one port published on loopback.
- The workspace is mounted at `/home/agent/workspace`.
- `PUT /secrets` takes the whole set of secrets the teammate is granted, a
  JSON object of name to secret, bearer in the `Authorization` header. A
  variable is its bare value; a login is `{"kind":"login", sites, username,
  password, totp?}`; a passkey is `{"kind":"passkey", rpId, credentialId,
  privateKey, userHandle?, userName?, userDisplayName?}`. The computer keeps
  the set in memory, puts every variable in the environment of every job the
  agent starts through `shell` or `files run` (not of a preparation job,
  whose captured environment is written into the workspace), types a login
  through `browser fill` with `secret` only on a page of the login's own
  sites, loads every passkey into a WebAuthn virtual authenticator on each
  browser tab so a site's `navigator.credentials.get()` is answered without
  anything being typed, redacts every value from what its tools answer, and
  never answers one back — there is no GET, and a job does not inherit the
  bearer either. A release from before the route answers 404, and the desk
  says so on the teammate's tape; a release from before logins (0.7.x)
  refuses a typed record with 400.
- `PUT /passkeys/registration {"rpId"}` arms the computer for ten minutes,
  for that one site, and readies the browser. While armed, and only then, a
  site's `navigator.credentials.create()` is parked by the computer's guard
  with what the site asked for, and `GET /passkeys/registration` answers
  `asked` with that `ask` (`id`, `rpId`, `origin`, `rpName?`, `userName?`,
  `userDisplayName?`, `askedAt`); the desk raises the card, and
  `POST /passkeys/registration/answer {"id", "approved"}` carries the
  answer back. Approved, the browser mints and `GET` answers `approved`,
  then `registered` with the minted credential as a whole passkey record;
  the desk stores it, delivers the set with it, and `DELETE`s the arming.
  Denied, the site gets a `NotAllowedError` and the arming ends with it; an
  answer to a request that is not waiting is a 409. A credential minted
  outside an arming, without an approval, for another site, or after the
  ten minutes is removed from the authenticator on the next look, so the
  teammate cannot give itself a passkey, nor ask the person for one unless
  the person armed the site first. Bearer-only, like `/secrets`. A 0.8.x
  computer has no answer door and mints under the arming without asking;
  the desk stores what it minted as before.
- `DELETE /logins/{name}` with `{"domains": [...]}` takes a saved login's
  cookies back: the browser drops every cookie for each named site, or a
  host within it, from its running context, the saved login is pruned, and
  it is removed when nothing is left; with no body, every domain the saved
  login names. The desk's cookie import lands as the saved login
  `import-<browser>-<profile>`, loaded with `state login_load`, and this is
  its way back out; the desk always names the domains, from its own record.
  A release from before the door (0.8.0 and earlier) answers 404, and the
  desk says so and points at the teammate's Update. Bearer-only, like
  `/secrets`.

A paired phone reaches the same viewer socket through a door on the Remote
listener, `GET /computer/<personaId>/ws` with the phone's own bearer. The
desk checks the grant and that the computer is running, then carries bytes
between the phone and the container's `/ws` on loopback, presenting the
bearer it holds. Frames pass to the phone as they are; what the phone sends
passes to the computer as it is, text only. The phone never learns the
port or the token, a phone can name a teammate and nothing else, and
revoking the device drops the pipe. The route is refused while the
computer is stopped; waking it stays the session's business.

## Lifecycle

Wake is on session start: `ensure_running` pulls the image when absent, creates
the container with a token generated for it, starts it, and waits for
`/health`. It runs on its own task. With the image already present the start
waits for it and grants it. The first pull report instead lets the start go
ahead without the computer: the agent is told its computer is downloading and
has `computer_status` (optionally waiting up to 300 s) to follow it, the pull
streams as a `computer_pull` delta that the window draws as a ring where the
computer's button will be (never a line on the tape, never sent to the
phone), and when `ensure_running` returns the session is restarted with the
grant as soon as no turn is in flight (`run_turns` checks on its way out).
A computer that fails, at once or behind the session, leaves the teammate
answering without it, with the reason in its preamble and the desk's log;
the persona keeps the grant, so the next start tries again. None of this is
narrated on the tape: the conversation carries on as one conversation.
Peer sessions still wait for their computer. The token lives in the container's private process environment, never
in room settings. Hotline recovers it from runtime inspection after an app restart,
so an existing computer keeps its jobs and viewer. A container without a
recoverable token is recreated. Idle uses the room's sweep:
thirty minutes after a session stops the container is stopped and its rw layer
kept; seven days and it is removed.

The image a computer runs is chosen when its container is created: the
teammate's override, else the room's, else the release this desk pins. An
existing container, running or stopped, is reused as it is, and nothing
compares its image to the current choice or polls for a newer one, so a
computer made on an older release keeps running that release after Hotline is
updated. To move it, remove the computer from the teammate's pane; the next
start creates one on the current choice, and the workspace, scratch and home
volumes survive the container, so prepared environments, jobs and the browser
profile come back with it. The running release is always the one the guide
reports, never the configured tag.

The desk learns that release at every start: once the container is healthy,
Hotline calls its `state` tool with action `guide`, which answers the release's
own skill with its version and checksum, and writes it into the teammate's
workspace as the `hotline-computer` skill (see [design.md](design.md), section
7). `computer.status` reports that release, and when it differs from the one
the teammate's computer would be created on now, the release it would get
as `available`. The same start hands the computer the secrets the teammate
is granted (`persona.computer.secrets`, read from the vault), and so does
every reattach and every change to a stored value while the computer is
running; a stopped one gets the current set at its next start. Mounts are
different: they are bind mounts fixed when the container is created, so a
changed mount takes effect at the next Remove. `computer.update` is the pane's answer to that: remove the
container so the next one is made on the current choice. A teammate at rest
has its container removed at once. A running one is not stopped or held up:
the new release downloads while the old computer keeps working, and once the
turn in flight ends the container is removed and the session reattaches onto
the new one. The rest of the computer's settings (limits, mounts, a pinned
image, granted secrets) do not restart the teammate either. The first three
wait for the next container, and secrets are handed to the running one. Only
turning the computer on or off restarts the session. What survives an update is what survives a removal — the
volumes — and nothing else; the pane says so next to the button.

The runtimes, how they are found, and the fake runtime the tests drive are in
[development.md](development.md).
