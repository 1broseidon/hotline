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
message. Any of that content can carry instructions. Toad's position is
that the model will sometimes follow them, so nothing the model says or
decides is an enforcement point. The person's standing choices are enforced
in the core, outside the model, at every entry a request can arrive
through — Toad Agent's own tool closures, the in-process MCP server an ACP
child calls, the file callbacks Toad performs for that child, the scheduler,
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
| Reach | `reach` | `workspace` | Toad Agent's workspace tools and shell. `workspace`: the working directory is a wall; the shell runs confined (bubblewrap on Linux, Seatbelt on macOS, not offered on Windows) with selected host toolchains read-only, a private `HOME` and scratch, no host environment, and the host network. `machine`: the same tools with no wall. |
| MCP gateway | `mcpPolicy` | `none` | Each selected server's own capabilities, wherever it reaches. Toad connects it as the client; it is not inside the shell sandbox. `all` includes servers added later. An imported or invalid policy grants nothing. |
| Collaboration | `allowedSenders` on the recipient, and the caller's reach | empty | Asking another teammate to use its workspace and tools. A `machine` Toad Agent caller has this implicitly. A `workspace` caller needs the operator's first-contact decision per direction: a session grant bound to both live leases, or a standing grant recorded by the sender's stable id. Discovery gives a workspace caller ids and names only. |
| Background work | `backgroundWork` | `false`, including on older records | Creating its own schedules and loops. Jobs the person creates over the desk wire carry `operatorCreated` and run without it; an agent tool cannot set that flag. |
| Computer | `computer.enabled` | off | A containerized desktop, `--cap-drop=ALL`, `no-new-privileges`, with the workspace and the teammate's declared mounts bound in. It is a per-teammate capability, not a gateway server, and does not widen reach. |
| ACP harness | `backendId` other than `toad` | Toad Agent | Trust in that harness: its process, tools, configuration and permission policy are its own, outside Toad's sandbox. Toad's file callbacks for it stay in the workspace whatever its saved `reach` or advertised mode says. Its runtime mode is shown as *Externally managed*. |

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
  gives them. Toad states this in the ledger and the Reach card and does
  not pretend otherwise.
- **The workspace is the agent's**, along with `.toad-home/` inside it and
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
  compatibility guarantee. Windows has no confinement Toad can ship, so the
  confined shell is not offered there. No platform has resource limits or
  syscall filtering on the shell.
- **The person is asked nothing at call time.** There are no approval cards
  for Toad Agent's tools and no per-path ACLs. A capability is standing or
  it is absent.

## The method for a new capability

Follow this when adding a tool, a tool origin, a driver behaviour, or a
path by which one teammate's work reaches another's. Each step is a
question the change must answer in its PR description.

1. **Inventory the paths.** List every way the new thing executes or
   reads: Toad Agent tool closures, the in-process MCP server, ACP file
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
   Toad enforces from what a grantee enforces for itself.
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
| Collaboration: a machine caller needs no card; reach is re-read after discovery; session consent is directional and expires with either side; a standing grant survives restart and uses the stable id; removing it revokes cached work; a dropped wait cannot be answered later; revocation reaches delegated third parties but not their main sessions | `session/peers/tests.rs` `explicit_whole_machine_toad_agent_can_collaborate_without_a_card`, `collaboration_rechecks_reach_after_discovery`, `session_consent_is_directional_and_expires_when_a_side_stops`, `permanent_consent_survives_peer_restart_and_uses_stable_sender_id`, `removing_a_permanent_grant_revokes_cached_work_and_requires_consent_again`, `a_dropped_collaboration_wait_is_expired_and_cannot_be_answered_later`, `invalidating_either_side_revokes_cached_peer_tools_without_a_main_session`, `peer_teardown_does_not_revoke_the_callers_main_tools_but_main_stop_does`, `nested_peer_leases_follow_the_outer_target_but_revoke_independently`, `revocation_reaches_a_third_teammates_delegated_tools_but_not_its_main_session` | — |
| The collaboration card is answered over the real wire before the peer starts; the phone seat can answer for the person but never grant a standing one | `wire/tests.rs` `the_wire_answers_a_core_owned_collaboration_card_before_peer_start`, `the_phone_seat_answers_for_the_person_but_never_grants_a_standing_one` | — |
| Discovery never derives public fields from private instructions | `mcp/server.rs` `teammate_discovery_never_derives_public_fields_from_private_instructions` | — |
| Background work: scheduling requires the grant, operator jobs do not; a due agent job waits for the grant then fires once; a queued line is dropped on revocation; an old job on the wire requires the grant; `list_schedules` and `cancel_schedule` stay own-teammate | `mcp/server.rs` `scheduling_requires_background_work_but_operator_jobs_do_not`, `list_schedules_lists_the_callers_jobs`, `cancel_schedule_refuses_another_teammates_job`; `session/tests.rs` `a_due_agent_job_waits_for_a_grant_then_fires_once`, `a_due_operator_job_runs_without_a_background_grant`, `a_queued_scheduled_line_is_dropped_when_background_work_is_revoked`; `session/schedule.rs` `scheduled_run_authority_reads_the_live_grant_and_trusted_source`; `tests/schedule.rs` `an_old_job_on_the_wire_requires_the_background_grant` | — |
| ACP callbacks stay in the workspace for legacy `machine` personas; a symlinked root alias is accepted; `AGENTS.md` refuses external and dangling symlinks; runtime mode is separate from effort and other configs stay hidden | `driver/acp.rs` `acp_callbacks_stay_in_workspace_for_legacy_machine_personas`, `callback_workspace_accepts_the_selected_symlinked_root_alias`, `agents_md_refuses_external_and_dangling_symlinks`, `disposition_separates_runtime_mode_from_effort_and_hides_other_configs` | — |
| A permission left open in a peer turn expires with the turn; a receipt cannot move machinery | `session/peers/tests.rs` `a_permission_left_open_in_a_peer_turn_is_expired_when_the_turn_ends`, `a_receipt_cannot_move_machinery` | — |
| The desk restart lease refuses new wire work and keeps saved data | `tests/desk.rs` `the_desktop_restart_lease_refuses_new_wire_work_and_keeps_saved_data` | — |

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
