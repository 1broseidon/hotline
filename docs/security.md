# Security — the standing method

This is the checklist an agent follows when it changes what a teammate can
reach, and the record of what the shipped code enforces. It was written
2026-09-14, after the five steps of the "Whole machine" plan landed
(revocation, collaboration grants, private room data and background work,
ACP trust boundaries, macOS sandbox parity). It describes measured
behaviour; where something is planned rather than shipped, it says so. The
mechanics of each boundary are in [sessions.md](sessions.md); the reasons
are in [design.md](design.md). This document is the method, and the index
of proofs.

## The threat model

A teammate does ordinary assistant work on content it did not write: a
cloned repository, a web page, an email, a tool result, a colleague's
message. Any of that content can carry instructions. Hotline's position is
that the model will sometimes follow them, so nothing the model says or
decides is an enforcement point. The person's standing choices are enforced
in the core, outside the model, at every entry a request can arrive
through — Hotline Agent's own tool closures, the in-process MCP server an ACP
child calls, the file callbacks Hotline performs for that child, the scheduler,
and the wire.

Two things are deliberately not defended against. The person: whoever holds
the desk token may do anything the desk may do, and the phone seat is a
smaller fixed set, not a policy engine. And the workspace: a teammate can
read and change everything in its working directory, including any secret
the person left there.

## Standing consent

The person decides once, in settings, and the agent is never asked to ask
again. Each capability below is a field on the teammate's record in the
`room` stream, folded on load; each has a default that grants nothing
beyond the workspace, and an explicit migration for records that predate
it.

| Capability | Field | Default | What it authorizes |
| --- | --- | --- | --- |
| Reach | `reach` | `workspace` | Hotline Agent's workspace tools and shell. `workspace`: the working directory is a wall; the shell runs confined (bubblewrap on Linux, Seatbelt on macOS, not offered on Windows) with selected host toolchains read-only, a private `HOME` and scratch, no host environment, and the host network. `machine`: the same tools with no wall. |
| MCP gateway | `mcpPolicy` | `none` | Each selected server's own capabilities, wherever it reaches. Hotline connects it as the client; it is not inside the shell sandbox. `all` includes servers added later. An imported or invalid policy grants nothing. |
| Collaboration | `allowedSenders` on the recipient, and the caller's reach | empty | Asking another teammate to use its workspace and tools. A `machine` Hotline Agent caller has this implicitly. A `workspace` caller needs the operator's first-contact decision per direction: a session grant bound to both live leases, or a standing grant recorded by the sender's stable id. Discovery gives a workspace caller ids and names only. |
| Background work | `backgroundWork` | `false`, including on older records | Creating its own schedules and loops. Jobs the person creates over the desk wire carry `operatorCreated` and run without it; an agent tool cannot set that flag. |
| Computer | `computer.enabled` | off | A containerized desktop, `--cap-drop=ALL`, `no-new-privileges`, with the workspace and the teammate's declared mounts bound in. It is a per-teammate capability, not a gateway server, and does not widen reach. |
| Secrets | `computer.secrets` | none | Named values from the operator's store (see below), in the environment of every job that computer runs. The computer redacts each value from what its tools answer and nothing returns one. A record from before the field, or a name nobody ticked, grants nothing. |
| ACP harness | `backendId` other than `hotline` | Hotline Agent | Trust in that harness: its process, tools, configuration and permission policy are its own, outside Hotline's sandbox. Hotline's file callbacks for it stay in the workspace whatever its saved `reach` or advertised mode says. Its runtime mode is shown as *Externally managed*. |

Reach, gateway, collaboration, background work and the computer are
independent axes. Changing one never changes another; in particular
`machine` reach grants no gateway server, and neither ACP mode nor a
computer counts as `machine` reach for collaboration.

## Accepted risks and known limits

These are the choices, so a change that "fixes" one of them is a product
decision for George, not a bug fix.

- **Network is on under every reach**, including localhost. Agents install
  things. The shell boundary is filesystem isolation, not network
  isolation: a local service can expose files or privileged actions of its
  own, and data the agent can legitimately read can leave.
- **A grant authorizes the grantee's own permissions.** A granted stdio MCP
  server, a computer, and an ACP harness each run with whatever the host
  gives them. Hotline states this in the ledger and the Reach card and does
  not pretend otherwise.
- **The workspace is the agent's**, along with `.hotline-home/` inside it and
  the private caches there. Teammates sharing a working directory share
  those.
- **Dispatched side effects survive revocation.** Revocation refuses the
  next call; it cannot recall a remote tool call already in flight or undo
  a file already written.
- **Platform differences.** Linux has mount, PID and IPC namespaces. macOS
  has default-deny Seatbelt with an enforcement probe, private `TMPDIR`,
  and denied AppleEvents, LaunchServices, launchd and Unix control sockets,
  but no namespaces: root directory names and ancestor metadata can remain
  visible, and programs that hardcode `/tmp` fail. Seatbelt's CLI is
  deprecated and its policy language undocumented; the probe fails closed
  if enforcement stops, which is protection against silent breakage, not a
  compatibility guarantee. Windows has no confinement Hotline can ship, so the
  confined shell is not offered there. No platform has resource limits or
  syscall filtering on the shell.
- **The person is asked nothing at call time.** There are no approval cards
  for Hotline Agent's tools and no per-path ACLs. A capability is standing or
  it is absent.

## Operator actions are not agent capabilities

Some things the person does from the desk are one-shot transfers, not standing
grants: attaching a file to a prompt, adding a mount, and importing a host
browser's cookies into a teammate's computer. They use the person's own
authority through the authenticated desk wire, are gated to the desk seat, and
add nothing to what the agent may reach on its own. They are not a call-time
approval prompt, which the standing-consent rule forbids, because the person
initiates them; there is no card the agent can raise.

Cookie import (`computer.browsers.list`, `computer.cookies.preview`,
`computer.cookies.import`, `computer.cookies.list`, `computer.cookies.forget`)
is the sharpest case, so it is spelled out. The agent has no tool that reads
the host's browsers; the five commands are desk-seat only and the phone
allowlist does not name them, so the model cannot pull cookies whatever it is
told. The person chooses the browser, the profile,
and the exact sites; a preview carries domains and counts, never a value. On
import the chosen cookies pass host → desk → container over the container's
authenticated loopback port and are written into the sandbox the person already
granted; they never enter the tape, the model's input, or a log. What the
agent gains is a browser already signed in to sites the person picked — the
same exposure as the person signing in there by hand inside the computer, and
the accepted risk of giving an agent a logged-in browser at all. Expired
cookies never leave the host: the browser would drop them on its next look,
and they would only pad the list. What was brought over is recorded — browser,
profile, time, domains and counts, never a value — one record per teammate on
the room stream, and listed on the teammate's pane, where each site and each
browser's whole import can be taken back. `computer.cookies.forget` names the
exact domains to the computer's bearer-guarded `DELETE /logins/{name}`, whose
browser drops those cookies at once and whose saved login loses them, so what
the agent's browser is signed in to is what the pane shows.

## Secrets a teammate uses without seeing

The operator keeps three kinds of secret in the vault, each under a name.
A variable — a GitHub PAT, an npm token, a Stripe test key — is a value
that becomes an environment variable: `secrets.set {name, value}`. A login
is one or more sites, a username, a password and optionally a TOTP seed:
`secrets.login.set`. A passkey is a WebAuthn credential of the teammate's
own, which is never sent in but made (below). Every record is written to
the OS credential store (macOS Keychain, Windows Credential Manager, Linux
Secret Service, the same opaque-reference records as provider keys; no
plaintext fallback), beside a private sidecar that says only what the
record is for — kind, sites, username, whether there is a seed; site and
user name for a passkey — so `secrets.list` answers that without opening
the keychain. Nothing on the wire, in the room stream, or in a subscription
ever carries a value, a password, a seed or a private key: the store is
write-only from the window. All seven commands are desk-seat only.

A teammate gets a secret only by the operator ticking its name on that
teammate's computer, `computer.secrets`, a standing choice like a mount,
nothing pre-ticked, set only through `persona.update`, which the phone may
not send. At every grant — session start, reattach, and whenever a stored
value is replaced or deleted — the desk reads the granted values from the
vault and hands the computer the whole set through `PUT /secrets` on its
authenticated loopback port, bearer in a header. A stopped computer is
handed nothing until its next start, which hands it the current set. The
container keeps the set in memory and puts it in the environment of every
job the agent starts through `shell` or `files run` — above the workspace's
saved environment, under the agent's own explicit `env` — and of no
preparation job, because what preparation captures is written into the
workspace. It redacts every value from every tool result at the one point
all its tools pass through, as `[redacted NAME]`, in the spelling JSON
gives a value too. `/secrets` is PUT-only; there is no route that returns
a value, because the bearer is the container's door key and not a secret:
a job no longer inherits it, but anything running in the container can
still read its service's environment, so holding the bearer must never be
worth a value. The agent is told the names and kinds, in the preamble and
by `state info`, and told it will never see a value.

A login goes to the computer as a record with its sites, and is typed only
there. The agent asks `browser fill` for `NAME.username`, `NAME.password`
or `NAME.code` — the six TOTP digits of this moment, computed in the
container from the seed — by name, in place of text; the computer types it
into the referenced field only when the page's origin is one of the login's
sites, same scheme and port, same host or a subdomain of one, and a site is
`https://` unless it is localhost. Any other page is refused with a
sentence, and the preamble tells the teammate that refusal is right. The
password and the digits are redacted from every answer as `[redacted
NAME.password]` and `[redacted NAME.totp]`; the username and the sites are
not secrets and are not. What the origin rule buys is that a page the
teammate was steered to — a phishing copy, an `http://` downgrade, a look-
alike host — cannot have the password typed into it; what it does not buy
is anything after a genuine sign-in, which is the same exposure as a
person signing in there by hand inside the computer, the accepted risk of
giving an agent a signed-in browser at all.

A passkey is the teammate's own, because a person's never leaves their
authenticator. The computer keeps every granted passkey in a WebAuthn
virtual authenticator on each browser tab, from the delivered set, so a
site's `navigator.credentials.get()` is answered inside Chromium and the
private key is never typed, never in a tool answer (`[redacted
NAME.privateKey]` if it ever were), and never in the model's input. Making
one is the operator's act, twice over. First the arming:
`secrets.passkey.register`, from the teammate's pane, arms that teammate's
computer for one site for ten minutes (`PUT /passkeys/registration`,
bearer-only, the computer started if it was stopped). Then the approval:
the operator signs in to the site through the teammate's screen and adds a
passkey in the site's security settings — or asks the teammate to — and
the site's `navigator.credentials.create()` is not answered by the browser
on its own. A guard script the computer installs on every document, keyed
by a token the page cannot read, rejects the call outright when nothing is
armed or another site is; under the arming it parks the request with what
the site asked for — the site, the origin, the account name and display
name — and the computer's next look records it and answers it on `GET
/passkeys/registration` as `asked`. The room, which polls that door every
two seconds while an arming stands, writes a `passkey_ask` card on the
teammate's tape and tells the phones, and the person's answer goes back
through `secrets.passkey.answer` and `POST /passkeys/registration/answer`.
Approved, the page is told to go ahead and the browser's authenticator
mints; denied, the site gets a `NotAllowedError` and the arming ends with
the denial, so neither a site nor a teammate can keep asking. One request
is before the person at a time; a request whose page went away, an arming
that ran out or was cancelled, and a computer that stopped each leave the
card expired rather than answerable; and an answer to a request that is
not waiting is refused by the room and by the computer (409), so a stale
card cannot let one through. The card has the standing of a permission
card — one answer to one request the operator armed for at the desk —
which is why the phone may give it, while arming stays at the desk. The
computer's own check on every look removes from the authenticator any
credential that is neither in the delivered set nor minted under an
approved request of the current arming, so a teammate cannot give itself
a passkey, keep one made for another site, or keep one after the ten
minutes. The look that finds the credential minted stores it in the
vault, ticks the name on `persona.computer.secrets` for that teammate —
the same tick the pane makes for any secret, made for the operator because
they asked for this passkey for this teammate — hands the computer the set
with it, which is what keeps it in the authenticator, `DELETE`s the
arming, and says so on the tape. That answer is the one time a private key
leaves the container: over the bearer-guarded loopback door, in the
direction the cookie import already trusts, into the vault, and into no
tape, room event or log. Revocation is any of three: untick it on the
teammate or remove it under Secrets, both of which hand the computer a set
without it and the authenticator drops it on the next look; or delete the
passkey in the site's own security settings, after which the credential
the computer holds signs nothing. The desk's floor for a new computer is
0.9.0, the first release that asks before it mints; a 0.8.x computer mints
under the arming without asking and the room stores what it minted as
before, an older one refuses a login record with 400 and has no passkey
door, and the pane says so and points at Update.

What this enforces: no agent tool reads, sets or grants a secret; a value
passes host keyring → desk memory → container memory → job environment and
is written to no tape, no log, no room event and no model input; and the
computer scrubs the value as it is from what the model reads. What it does
not enforce, stated plainly: a model that deliberately encodes a value —
`base64`, splitting it, printing it on the desktop and taking a screenshot
— can still read it, because the sandbox has to be able to use the value
with arbitrary programs. Redaction is a guard against incidental exposure
(`curl -v`, `env` in a debug dump, a tool printing its configuration), not a
wall against a determined model; the wall is the grant, and the guidance is
to grant a teammate only the secrets its job needs. Jobs already running
keep what they were started with when a secret is revoked, the accepted
"dispatched side effects survive revocation" rule; a running computer that
cannot be told of a change keeps its set until it is stopped, and the
teammate's tape says so. A release from before `/secrets` answers 404: the
teammate starts, the tape names what is not in its shell, and the pane's
Update is the fix. Network-layer injection, where a proxy adds the header
and the value never exists in the sandbox, is the stronger follow-up for
HTTP APIs and is not built.

## The method for a new capability

Follow this when adding a tool, a tool origin, a driver behaviour, or a
path by which one teammate's work reaches another's. Each step is a
question the change must answer in its PR description.

1. **Inventory the paths.** List every way the new thing executes or
   reads: Hotline Agent tool closures, the in-process MCP server, ACP file
   callbacks, the scheduler's queued turns, cached peer sessions, and the
   wire. If a path is missed, the boundary has a hole in exactly that
   place; the collaboration and background-work steps each found one in
   the built-in tools because those are always registered rather than
   granted.
2. **Name whose authority a request uses.** The caller's? The recipient's?
   The person's (only through the authenticated desk wire)? A request that
   arrives through a peer session carries the caller's lease as a
   dependency, so revoking the caller kills work done on its behalf. Never
   let a request borrow the authority of the process it happens to run in.
3. **Choose the default and the migration.** The default grants nothing
   beyond the workspace. Decide explicitly what an existing record without
   the field means, and write it down beside the field in `contract.rs`;
   "missing means off" is the house answer, and old implicit access is not
   translated into an explicit grant.
4. **Enforce outside the model, at every entry.** The check is a
   `CapabilityLease::check()` (or the policy read that it guards) in the
   handler, not a sentence in the preamble. Both drivers reach the same
   handler; a check that exists only on one driver's path is a hole on the
   other.
5. **Make revocation reach it.** A new handle is cloned from the session's
   lease or scoped from it; it is never a fresh lease of its own. Confirm
   the change is covered by the lifecycle below, including queued work and
   sessions cached for a peer.
6. **Tell the truth in three places.** The ledger row, the Reach or Tools
   card, and the preamble say what the code enforces, and distinguish what
   Hotline enforces from what a grantee enforces for itself.
7. **Prove it both ways.** A test that the allowed action works and a test
   that the denied action is refused, through the real handler (see the
   matrix). A denied path that only a model's good manners keeps closed
   does not count.

Do not add an ACL editor, a per-call approval prompt, or a new policy field
to solve a problem the existing axes already express; if a new axis is
genuinely needed, that is a design decision recorded in
[design.md](design.md) first.

## Grant lifecycle

Authority is a `CapabilityLease` (`driver/mod.rs`), a snapshot of a
teammate's `CapabilityEpoch` generation. Every handle a session hands a
driver — tool closures, the workspace directory, cloned MCP tools, ACP
callbacks, the OAuth proxy — clones the same lease and calls `check()`
before acting.

- **Creation.** `Room::start` takes a lease and checks it before and after
  reading the record, so a start racing a policy update cannot capture the
  old policy in a usable lease.
- **Policy change.** `invalidate` advances the generation and closes the
  epoch *before* the new record is appended; `activate` reopens it after
  the append is durable. In between, every lease — existing or newly
  captured — is dead, and a failed append leaves the teammate visibly
  stopped and refusing work until a retry succeeds. The wire holds one
  room-wide `policy_update_lock` across invalidate, append and reattach so
  a second socket cannot reactivate a generation between the steps.
- **Stop.** Advances the generation and leaves the epoch open: old handles
  are permanently dead, a later explicit start uses the current policy. A
  stop also invalidates a replacement session captured before its startup
  began.
- **Delegation.** A peer session holds a lease `scoped()` from the caller's
  and dependent on the target's. Peer teardown revokes only the scoped
  lease; a stop or policy change on either side revokes the peer through
  the shared epoch. Third-party sessions delegated from a peer follow the
  same chain and stop when the original request's authority goes, while
  each teammate's independent main session stays usable.
- **Caching.** Cached peer sessions are revoked whether the changed
  teammate was caller or recipient, including peer-only sessions whose
  main session is stopped.
- **Active and queued work.** Revocation cancels the activity's shell jobs
  and settles their results, clears queued turns, and releases collaboration
  and human waits so a late answer cannot establish a grant. Stale turns
  cannot publish state or checkpoints.
- **Schedules.** The grant is read when a job is created, again before a
  wake, and again immediately before a queued firing reaches the driver.
  Each queued firing carries its own provenance because a one-shot may be
  tombstoned by then. Revoking background work pauses agent jobs without
  deleting them.
- **Subagents.** The only delegation that exists is the peer session above.
  `persona.subagents` is a carried record field with no behaviour behind
  it yet; whatever wires it must give each subagent a lease dependent on
  its parent's, never a broader one.

## The regression matrix

Each row is one thing that must stay true, and the test that proves it
through the real handler. `make check` runs everything not marked
otherwise. When a change touches a row, the named test is the one to
extend; when a change adds a boundary, it adds a row.

| Must hold | Proof | Needs |
| --- | --- | --- |
| Workspace reach keeps another project's `.env` out of the read tool and the shell, over the wire, and the ledger says what is offered | `tests/desk.rs` `workspace_reach_keeps_another_projects_env_out_of_the_tools` | Linux with bubblewrap |
| Workspace tools refuse parent paths and a path outside the wall; machine reach resolves them; the overflow directory is the one read outside | `tools/workspace.rs` `parent_paths_are_rejected`, `reaching_the_machine_resolves_absolute_paths_and_parents`, `workspace_reach_can_read_the_teammates_overflow_directory`, `machine_reach_ignores_the_overflow_root` | — |
| A revoked workspace handle refuses reads and writes | `tools/workspace.rs` `a_revoked_workspace_handle_refuses_reads_and_writes` | — |
| Confined shell: writes inside, refuses writes and reads outside including from child processes, private `/tmp`, private persistent home, no host environment, a home symlink cannot escape, synthetic parents read-only | `tools/shell.rs` `workspace_reach_*`, `workspace_shell_*`, `a_private_home_symlink_cannot_create_files_outside`, `synthetic_parent_directories_are_read_only` | Linux with bubblewrap |
| Linux runtime exceptions expose executables, not neighbours, credentials or a redirected project | `tools/shell/linux.rs` `arbitrary_path_directories_do_not_grant_read_access`, `a_path_program_does_not_expose_its_neighbors_or_symlink_target_directory`, `home_toolchains_are_read_only_and_do_not_expose_credentials`, `redirected_installations_do_not_expose_other_projects` | Linux |
| macOS: availability requires real enforcement; aliases, symlinks and children cannot reach host data; no host environment or signals; hostile setup symlinks; helper services and control sockets denied; runtime exceptions exclude credentials and neighbours | `tools/shell/macos.rs` `availability_requires_real_enforcement`, `aliases_symlinks_and_children_cannot_access_host_data`, `host_process_environment_and_signals_are_not_available`, `private_scratch_and_hostile_setup_symlinks`, `host_helper_services_and_unix_control_sockets_are_denied`, `home_runtime_exceptions_do_not_include_credentials_or_neighbors`, `redirected_installations_never_grant_the_target` | a real Mac; these fail, not skip, without Seatbelt |
| macOS network stays available and package downloads land in private caches | `tools/shell/macos.rs` `ip_networking_still_works`, `public_dns_and_https_remain_available`, `package_downloads_use_private_caches` | a Mac and public registries; `--ignored` |
| Cancellation reaches the sandbox and everything it started; a deadline takes the whole tree | `tools/shell.rs` `cancellation_reaches_a_started_sandbox_and_its_children`, `a_deadline_takes_the_command_and_everything_it_started` | Linux with bubblewrap |
| A new teammate gets no gateway server even with machine reach; grant changes rebuild the live tools; a revoked MCP tool refuses before calling its server | `tests/mcp.rs` `a_new_teammate_gets_no_gateway_tools_even_with_machine_reach`, `gateway_access_is_explicit_and_changes_rebuild_the_live_tools`; `mcp/mod.rs` `a_revoked_mcp_tool_refuses_before_calling_its_server` | — |
| Secrets never enter settings: OAuth client secrets and non-string env values are refused before the write; legacy plaintext is hidden and repaired | `tests/mcp.rs` `oauth_client_secrets_are_removed_before_mcp_settings_are_written`, `a_non_string_env_value_is_rejected_before_settings_are_written`; `tests/desk.rs` `legacy_mcp_secrets_are_hidden_and_repair_removes_the_plaintext_history` | — |
| The ACP OAuth proxy requires its own bearer and a live lease | `driver/acp.rs` `oauth_proxy_requires_its_own_bearer_and_a_live_grant` | — |
| A stdio server's children die with the connection | `tests/mcp.rs` `a_stdio_servers_own_children_die_with_the_connection` | Unix |
| A reach update reattaches the session and a name patch does not; an MCP settings update reattaches every live session; an unfinished policy update refuses work; stop cannot revive the old generation | `wire/tests.rs` `persona_update_of_reach_reattaches_and_a_name_patch_does_not`, `settings_update_of_mcp_servers_reattaches_every_live_session`; `session/tests.rs` `an_unfinished_policy_update_refuses_work_until_reattached`, `stop_revokes_a_replacement_before_its_startup_begins`, `stop_during_policy_quarantine_cannot_revive_the_old_generation`, `reattach_during_a_turn_cancels_the_old_queue_before_rebuilding` | — |
| Collaboration: a machine caller needs no card; reach is re-read after discovery; session consent is directional and expires with either side; a standing grant survives restart and uses the stable id; removing it revokes cached work; a dropped wait cannot be answered later; revocation reaches delegated third parties but not their main sessions | `session/peers/tests.rs` `explicit_whole_machine_hotline_agent_can_collaborate_without_a_card`, `collaboration_rechecks_reach_after_discovery`, `session_consent_is_directional_and_expires_when_a_side_stops`, `permanent_consent_survives_peer_restart_and_uses_stable_sender_id`, `removing_a_permanent_grant_revokes_cached_work_and_requires_consent_again`, `a_dropped_collaboration_wait_is_expired_and_cannot_be_answered_later`, `invalidating_either_side_revokes_cached_peer_tools_without_a_main_session`, `peer_teardown_does_not_revoke_the_callers_main_tools_but_main_stop_does`, `nested_peer_leases_follow_the_outer_target_but_revoke_independently`, `revocation_reaches_a_third_teammates_delegated_tools_but_not_its_main_session` | — |
| The collaboration card is answered over the real wire before the peer starts; the phone seat can answer for the person — a permission card, a `request_human` card, a passkey card — but never grant a standing one | `wire/tests.rs` `the_wire_answers_a_core_owned_collaboration_card_before_peer_start`, `the_phone_seat_answers_for_the_person_but_never_grants_a_standing_one` | — |
| Discovery never derives public fields from private instructions | `mcp/server.rs` `teammate_discovery_never_derives_public_fields_from_private_instructions` | — |
| Background work: scheduling requires the grant, operator jobs do not; a due agent job waits for the grant then fires once; a queued line is dropped on revocation; an old job on the wire requires the grant; `list_schedules` and `cancel_schedule` stay own-teammate | `mcp/server.rs` `scheduling_requires_background_work_but_operator_jobs_do_not`, `list_schedules_lists_the_callers_jobs`, `cancel_schedule_refuses_another_teammates_job`; `session/tests.rs` `a_due_agent_job_waits_for_a_grant_then_fires_once`, `a_due_operator_job_runs_without_a_background_grant`, `a_queued_scheduled_line_is_dropped_when_background_work_is_revoked`; `session/schedule.rs` `scheduled_run_authority_reads_the_live_grant_and_trusted_source`; `tests/schedule.rs` `an_old_job_on_the_wire_requires_the_background_grant` | — |
| ACP callbacks stay in the workspace for legacy `machine` personas; a symlinked root alias is accepted; `AGENTS.md` refuses external and dangling symlinks; runtime mode is separate from effort and other configs stay hidden | `driver/acp.rs` `acp_callbacks_stay_in_workspace_for_legacy_machine_personas`, `callback_workspace_accepts_the_selected_symlinked_root_alias`, `agents_md_refuses_external_and_dangling_symlinks`, `disposition_separates_runtime_mode_from_effort_and_hides_other_configs` | — |
| A permission left open in a peer turn expires with the turn; a receipt cannot move machinery | `session/peers/tests.rs` `a_permission_left_open_in_a_peer_turn_is_expired_when_the_turn_ends`, `a_receipt_cannot_move_machinery` | — |
| The desk restart lease refuses new wire work and keeps saved data | `tests/desk.rs` `the_desktop_restart_lease_refuses_new_wire_work_and_keeps_saved_data` | — |
| A phone reaches a running computer's viewer only through the desk, with the desk's bearer and never its own copy; the door refuses the unpaired, a path naming anything but a teammate, and a stopped computer; revoking the device drops the socket | `remote/tests.rs` `a_phone_reaches_a_running_computer_through_the_desk_and_never_holds_its_bearer`, `the_computer_door_is_shut_to_the_unpaired_the_unnamed_and_the_stopped`, `revoking_the_phone_drops_its_computer_socket`, `the_computer_target_is_read_off_the_desk_s_own_viewer_and_only_while_running` | — |
| What was brought over is listed from the room's record and taken back by site or whole: the computer is told the exact domains, the record follows, a site never brought over is refused, a release from before the door is named with Update; the record is one entry per teammate and the latest whole list; the phone can neither list nor take back; expired cookies are left on the host | `session/tests.rs` `brought_over_cookies_are_listed_and_taken_back_by_site_or_whole`; `room.rs` `the_record_is_the_latest_whole_list_per_teammate`, `an_import_from_the_same_browser_and_profile_merges_and_another_is_listed_beside_it`; `wire/tests.rs` `only_the_desk_seat_may_import_host_cookies`; `computer/cookies.rs` `expired_cookies_are_left_behind_and_session_cookies_stay` | Unix for the first |
| Stored secrets are the desk's alone: the phone can neither list, store, delete nor arm one, nor grant one through `persona.update` | `wire/tests.rs` `only_the_desk_seat_may_touch_stored_secrets` | — |
| A secret's name is an environment variable, never Hotline's own or the shell's; a value is at least eight characters; the disk holds a reference and the room stream nothing; the directory and its records are private and a planted link is refused | `vault/shared.rs` `a_name_is_an_environment_variable_and_hotlines_own_are_refused`, `a_value_is_at_least_eight_characters`, `a_shared_secret_is_listed_by_name_and_never_by_value`, `the_shared_directory_and_its_records_are_private_and_a_planted_link_is_not_a_secret` | Unix for the last |
| A login needs an `https://` site of its own (or `http://` on localhost) and a password of eight characters; a passkey needs a host name and a key; a login and a passkey are listed by what they are for and never by password, seed or key, and the sidecar carries none either | `vault/shared.rs` `a_login_needs_a_site_of_its_own_and_a_passkey_a_key`, `a_login_and_a_passkey_are_listed_by_what_they_are_for_and_never_by_value` | — |
| A computer is handed only what its teammate is granted and what is stored; the tape names what is not; the preamble names what the computer has, by kind; no tape, room event or preamble carries a value; a replaced or deleted value reaches every running computer and no stopped one; a release from before secrets is named only when something was granted; a login travels as one record beside a variable's bare value | `session/tests.rs` `a_computers_granted_secrets_are_handed_to_it_at_start_by_name_and_never_seen`, `a_changed_secret_is_handed_again_to_every_running_computer`, `a_computer_from_before_secrets_is_named_only_when_something_was_granted`, `a_login_is_handed_to_the_computer_as_a_record_and_named_by_its_sites` | Unix |
| A passkey is made only under an arming for one teammate and one site, and only once the person approves the site's request: a bad site or name is refused before the computer is touched, arming starts the computer, the site's request raises a `passkey_ask` card on the tape with the site, origin and account, nothing is stored while the card waits, an answer must name the request, one answer per request, an approval lets the browser mint and the look that finds it made stores it, ticks it for the teammate, hands the computer the set and ends the arming; a denial ends the arming with nothing stored and the card says so; a request that left with its page expires its card and the next request is a new card; a cancel stores nothing and expires a waiting card; the room's own watch does all of this with no pane polling, and the pane is told once when it next asks; the private key is on no tape and in no room event; a release from before passkeys is named | `session/tests.rs` `a_passkey_is_made_under_an_arming_stored_and_ticked_for_the_teammate`, `a_passkey_made_while_no_pane_is_looking_is_stored_by_the_room`, `a_passkey_request_denied_on_the_tape_ends_the_arming_and_a_lost_one_expires` | Unix |
| Delivery puts the bearer in a header, replaces the whole set, and tells an old release apart from a refusal; the arming is put, polled, answered and ended over the bearer door, the request's site, origin and account are read as the computer answers them, an answer to a request that is not waiting is a refusal that says so, a bad site is a 400, a release before the answer door mints without asking and refuses an answer as too old, and an old release has no door | `computer/secrets.rs` `the_set_is_put_whole_with_the_bearer_in_a_header`, `a_release_from_before_secrets_is_told_apart_from_a_failure`; `computer/passkeys.rs` `an_arming_is_put_polled_answered_and_ended_over_the_bearer_door` | — |
| Inside the computer: `/secrets` wants the bearer and has no GET, every job sees the variables under the agent's own `env`, a login is typed only on its own sites and refused elsewhere, the TOTP digits are computed from the seed, every tool answer has the values redacted; a passkey is minted only while armed, only for the armed site, and only under a request the person approved — the site's request is parked and answered as `asked` with what the site asked for, a denial rejects the site's call and ends the arming, an answer to a request that is not waiting is a 409, one request is before the person at a time — a credential outside an approved request is removed on the next look, a granted one signs a site's challenge, and a revoked one is gone | Hotline Computer's `src/secrets.rs`, `src/passkeys.rs` and `src/browser.rs` tests and `tests/contract.rs`, run by that repository's `make check` and `make contract` | the computer repository |

Where real macOS execution is required: every `tools/shell/macos.rs` row.
`make check` in macOS CI on both architectures runs the isolation and
toolchain tests; the network rows are `--ignored` and run by hand. BRO-14
recorded macOS 26.4.1 on Apple Silicon locally and macOS 15 in CI; macOS
13 and 14 are untested, and the app's minimum version is not evidence
about this policy. A release that changes the Seatbelt profile or the
bubblewrap launcher reruns the platform rows on each supported version.

## The review rule

A pull request that adds or changes a capability states, in its
description, in this order: the **default** for a new record and the
meaning of a missing field on an old one; the **grant source** (which
field, set by whom, over which wire command); the **enforcement points**
(the handlers that call `check()` or read the policy, on both drivers);
the **tests** added to the matrix above; and the **residual risk** it
leaves, in a sentence a reader without context can act on.

The UI copy, the preamble, and the ledger reason are part of the change,
not follow-ups: if the code confines a thing, they say so; if a grantee
enforces its own permissions, they say that instead. Planned protection is
described as planned, in the board or in Linear, never in a settings card
or in this document's present tense. When a row in the matrix stops being
true, this document is wrong, and fixing it is part of the change that
made it so.
