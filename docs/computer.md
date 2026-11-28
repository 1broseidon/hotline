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
`crates/hotline-core/src/computer.rs` is the floor — the release this desk
was tested with, what a computer is created on offline, and the line under
which nothing is offered — and never `latest`. A pinned image, the
teammate's or the room's, is used as written and never looked up. The desk
learns the release a running computer actually is from the guide it serves,
or recovers it from the container's metadata (below). When it differs from
the chosen image, the teammate's pane offers the update.

## The contract Hotline relies on

- `/health` is an open liveness probe on port 8787.
- `/mcp` is streamable HTTP MCP; with `HOTLINE_COMPUTER_TOKEN` set, every method
  wants `Authorization: Bearer <token>` and otherwise gets a JSON 401.
- `X-Computer-Holder` names the teammate for leases and queued runs.
- `/` is the viewer page and `/ws` its socket, which takes the same token as a
  `token` query. Hotline opens `http://127.0.0.1:<host port>/#<token>`; the page
  reads the fragment and the token never appears in a request line.
- Up to 0.12 nothing in the image runs as root, so Hotline creates the
  container with `--cap-drop=ALL` and nothing added back, `no-new-privileges`,
  memory and PID limits, a sized `/dev/shm`, and the one port published on
  loopback. A release from 0.13 boots as root only to split itself into
  users, so for its image, and no other, Hotline adds `--user 0 --cap-add
  SETUID --cap-add SETGID` and nothing else; a tag that is not a release
  number gets the older flags.
- `GET /capabilities?nonce=N` says whether the computer keeps secrets from
  its jobs: `{version, boot, protected: {v, mode, …}, attest}`. The desk
  asks with a fresh nonce before every handing over and before Use
  Cookies. The proof is v2 (0.13.0-rc.3 on): each boot makes a 32-byte key
  and hands it once, `POST /capabilities/key` → `{v: 2, boot, key}`, then
  409; the desk takes it as soon as a computer it started answers and keeps
  it beside nothing of the container's: in its own memory and in the OS
  store, under `computer-attest/<teammate id>` with the boot and the
  SHA-256 of the bearer it was taken with, so a desk restart reads it back
  and a container made again (a new bearer) never uses it. It is never
  sent or logged, and the entry is deleted when the computer is removed,
  hibernated or made again; `attest` is base64 HMAC-SHA256 under that key
  of `hotline-broker-v2|<boot>|<mode>|<version>|<N>`. `mode: "split"` with
  an `attest` that checks is a protected computer. Handed what a legacy
  computer is: `legacy` with an `attest` that checks (a split computer
  whose key proxy did not start), a 404 from a release before 0.13,
  `legacy` or no door from a container not made to split, and a v2 boot
  whose key the desk does not hold (taken first by something else, or by a
  desk that did not keep it). Handed
  nothing at all, and treated by the pane as needing an update: a v1 claim
  of split (0.13.0-rc.2 and before, `attest` under the bearer, which the
  desk's own asking carries to whatever answers), `split` with any other
  `attest` (none, one for another nonce, boot, mode or release, one
  another key made), anything but v2 (split or legacy) from a container
  made to split, which always answers v2 — a 404, `legacy` under the older
  proof, no `protected`, or a later version such as a v3, which this desk
  treats as needing an update — and `split`, v1 or v2, from a container the desk
  did not make to split (its `hotline.release` label is before 0.13 or not
  a release number), before any key is asked for: there everything runs
  as one user, and a job could answer the port. The checks are
  `computer/secrets.rs` `door_on`, `verify_attest_v2` and `stepped_down`,
  the MAC's input `attest_input_v2`; the set goes only
  after the proof verified, on the same client. A v2 computer's
  `protected.readable` names what of the set it holds it cannot keep from
  its jobs (a login or passkey for a loopback site, a key used with an
  "Other" service); the desk asks again after handing the set over and
  gives those names to the pane as `computer.status` `readable`.
- The workspace is mounted at `/home/agent/workspace`.
- `PUT /secrets` takes the whole set of secrets the teammate is granted for
  its computer, bearer in the `Authorization` header. A protected computer
  is sent the v2 form, `Content-Type:
  application/vnd.hotline.secrets.v2+json`, `{v: 2, generation, secrets}`:
  a readable variable is `{kind: "variable", value, readable: true}`, a key
  granted protected is `{kind: "variable", value, use: {origins, header,
  schemes, refuse?, standIn: {prefix, length}}}` from its "Used with"
  (`crates/hotline-core/src/vault/services.json`), and a login or passkey is
  the record below, with `readable: true` when it is for a site on the
  computer itself (a v2 computer types or signs with one there only then). What the computer does with a key's `use` is that
  release's to document. Any other computer is sent the v1 form, as 0.49
  sent it, a JSON object of name to secret, and is never sent a key granted
  protected: that waits on the desk, and `computer.status` says
  `protection: "needsUpdate"` (`"legacy"` when nothing waits, `"protected"`
  for a protected computer). In the v1 form a
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
- On a protected computer, Use Cookies goes to `PUT /sessions/{name}` with
  `{sites, cookies, storage}` (storage `{origin: {key: value}}`) instead of
  a saved login in the agent's home, and taking a share back is
  `DELETE /sessions/{name}` with `{domains?}`. Any other computer takes the
  saved login as above.
- On a protected computer, `/sites/ws?token=<bearer>&site=<host>` is one
  site of the signed-in browser as a screen in the viewer protocol. The
  window on this desk opens it for Open Browser and for a sign-in to finish;
  a window or phone elsewhere reaches it through `/v2/sites/{persona}/ws`
  on the remote listener, the site bound in the Noise payload.
- `GET /signin/pending` (bearer-only) answers `{pending: [{kind: "finish",
  name, origin, account, since}]}`: sign-ins the computer could not finish.
  An entry appears 30 s after a secret was typed while the tab still shows
  the same document, and clears on a new document, the tab closing or a
  revoke. The desk asks it on a protected computer at the cadence it asks
  `GET /secrets/uses`, shows each entry as one card that opens the site,
  and reads a 404 (a release before 0.13, or legacy) as none. These are not
  among `/secrets/uses`, whose kinds an older desk reads strictly; the desk
  passes over a kind it does not know there.
- `GET /migration` answers `{state, at, snapshots, loginsDropped,
  sitesCleared, answer}` after a split boot (404 in legacy mode); while
  `state` is `pending` the pane asks what becomes of the snapshots and
  `POST /migration/answer {snapshots: "clean"|"delete"|"keep"}` carries the
  answer.

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
phone), and when `ensure_running` returns the main session is rebuilt with
the grant as soon as no turn is in flight (the turn loop checks on its way
out). The rebuild is a chapter's swap's (`Room::rebuild`): the teammate's
threads, and the requests it sent or was given, go on, since the computer
changes the session and not what the teammate may do.
A computer that fails, at once or behind the session, leaves the teammate
answering without it, with the reason in its preamble and the desk's log;
the persona keeps the grant, so the next start tries again. None of this is
narrated on the tape: the conversation carries on as one conversation.
The token lives in the container's private process environment, never
in room settings. It reaches the runtime at create in the CLI's own
environment, named bare as `-e HOTLINE_COMPUTER_TOKEN`, never on the
command line. Hotline recovers it from runtime inspection after an app restart,
so an existing computer keeps its jobs and viewer. A container without a
recoverable token is recreated. A container made again, for other folders
or limits or an update, gets a new token, never the one it replaces: an
older computer's programs could read that one. One the runtime will not
remove is stopped and never adopted or started again with its token; the
next start removes it first, and fails while it cannot. Idle uses the room's sweep:
thirty minutes after a session stops the container is stopped and its rw layer
kept; seven days and it is removed.

Turning a computer off stops its container before the change is answered;
it is not removed, so turned on again it is the same container, with its
home, rw layer and token. A folder or a secret taken from a computer that
stays on stops it the same way, and its next start makes it again with the
folders it has now and hands it the secrets granted then; a moved workspace
or a declared mount taken away removes it instead, since a container records
neither. Every start of a computer reads the teammate's record under the
container's own lock (`Computer::ensure_current`), so nothing that read the
teammate before such a change brings the old container back. The sweep
never counts a teammate whose computer is off as using one, and stops such a
container within a minute even while the teammate's session runs. A stop
that fails is said, and the container is stopped again before its next use.
The promise and its limits are in `docs/security.md` (Policy change).

The image a computer runs is chosen when its container is created: the
teammate's override, else the room's, else the release this desk pins. An
existing container, running or stopped, is reused as it is, and nothing
compares its image to the current choice or polls for a newer one, so a
computer made on an older release keeps running that release after Hotline is
updated. To move it, remove the computer from the teammate's pane; the next
start creates one on the current choice, and the workspace, scratch and home
volumes survive the container, so prepared environments, jobs and the browser
profile come back with it. The running release prefers what the guide reports.
Without a learned guide release, including after a desk restart, inspection
reads the `hotline.release` label written from the image tag at creation. A
container made before that label falls back to the runtime's image reference:
Apple's `configuration.image.reference`, Docker's `Config.Image`, or Podman's
`Config.Image` / `ImageName`. Neither fallback reads the current image setting.

The desk learns that release at every start: once the container is healthy,
Hotline calls its `state` tool with action `guide`, which answers the release's
own skill with its version and checksum, and writes it into the teammate's
workspace as the `hotline-computer` skill (see [design.md](design.md), section
7). `computer.status` reports that release, and when it differs from the one
the teammate's computer would be created on now, the release it would get
as `available`. The same start hands the computer the secrets the teammate
is granted (`persona.secrets`, the teammate's own grant, read from the vault), and so does
every reattach and every change to a stored value while the computer is
running; a stopped one gets the current set at its next start. Mounts are
different: they are bind mounts fixed when the container is created, so a
changed mount takes effect at the next Remove. `computer.update` is the pane's answer to that: remove the
container so the next one is made on the current choice. A teammate at rest
has its container removed at once. A running one is not stopped or held up:
the new release downloads while the old computer keeps working, and once the
turn in flight ends the container is removed and the main session is rebuilt
onto the new one, as when a download lands; a thread built on the old
container keeps it until the thread is parked and brought back, and its
computer calls fail meanwhile. The rest of the computer's settings (limits, mounts, a pinned
image, granted secrets) do not restart the teammate either. The first three
wait for the next container, and secrets are handed to the running one. Only
turning the computer on or off restarts the session. Memory and CPUs are taken
by the runtime when it makes a container, so a container records the limits it
was made with (`HOTLINE_LIMITS`) and the next start makes it again when they
differ; one made before the limits were recorded is made again the next time
it starts from stopped. `computer.status` says `limitsChanged` while the
running computer has other limits than the teammate's, and the pane offers
`computer.restart`: the same swap between turns as an update, or for a
teammate at rest a stop, which the next start makes again. What survives an update is what survives a removal — the
volumes — and nothing else; the pane says so next to the button.

The runtimes, how they are found, and the fake runtime the tests drive are in
[development.md](development.md).
