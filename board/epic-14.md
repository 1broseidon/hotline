---
id: epic-14
title: "0.40 easy wins: the small fixes from the backlog triage"
type: epic
column: todo
position: 1
priority: high
tags:
  - "0.40"
  - easy-wins
  - triage
createdAt: "2026-10-18T23:40:44.455Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T23:51:16.037Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T23:51:14.029Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-18T23:51:16.037Z"
assignee: claude
---

## Description
The next wave for 0.40: small, low-risk fixes found by the 2026-10-18 backlog triage (four reviewers checked all 44 backlog items against the post-restructure code at release/0.40.0 78255a4f). Each child is one slice. Where a backlog item was only partly in scope, the slice is a new task, and the original keeps the rest and stays deferred.

Rules: each child is one landable change behind `make precheck` (the hooks are on), no new feature surface, and follows docs/standards.md. QA at the end on the Linux QA desktop with George's xAI sign-in, as for the 2026-10-18 pass. All land on release/0.40.0.

## Log
- 2026-10-18T23:54:10.198Z: [claude] Lead plan (claude/opus, 2026-10-18). Every child has relatedFiles and a ready contract. Validation is fmt + clippy -D warnings + an unfiltered `cargo test -p hotline-core` + `git diff --exit-code ui/src/generated/contract.ts`, which proves the committed contract is whole (AGENTS.md trap 3), plus `make ui-check` for window work and the named harness where one applies. ORDER: chain A, the failure notices, runs one at a time because all three reword driver/failure.rs: task-8 → task-188 → task-181. Chain B, the wire: task-187 and task-183 → task-55, because task-55 changes wire/socket.rs (same as task-187) and the remote seats (near task-183). Chain C, the generated contract: task-180 → task-185, since both add contract types and conflict in contract.ts. Independent, run as slots free up: task-182 (oauth page), task-184 (Threads.tsx), task-186 (hotline-app desks.rs), task-37 (voice ui), task-49 (Transcript), task-89 (probe first, see its note). Last: new task-189 depends on all 14, runs make check/verify/precheck on release/0.40.0 with everything in, fixes only where slices meet, checks docs/, and writes George's Linux QA checklist here. First wave: task-8, task-180, task-187 (each heads a chain). Agent choice: claude opus for core and wire work, claude sonnet for the small window-only slices (184, 49, 37).
- 2026-10-19T00:27:28.336Z: [claude] Lead: task-188 left two follow-ups for task-97, which is outside this epic, so I can't edit it from here. Whoever triages next should copy them across. (1) Connecting a provider doesn't restart sessions that are waiting for one; only the next message does, so the band stays 'error' until then. (2) The composer's no-provider refusal is text only, with no Settings link.
- 2026-10-19T00:37:52.688Z: [claude] Lead: task-55 left two follow-ups for task-20, which is outside this epic, so whoever triages next should copy them across. (1) A command answered inline that hangs still stops the 90 s silence timer from running, because the timer isn't selected while an inline command is being answered. (2) A stalled seat only has its file descriptor dropped; nothing resets the connection (no SO_LINGER reset).
- 2026-10-19T01:07:15.276Z: [claude] QA checklist for George's pass on the Linux QA desktop (xAI sign-in), release/0.40.0 with every slice in (task-189):
- task-8: a Hotline Agent teammate on the xAI sign-in, healthy, answers normally. If xAI turns a turn away with 401/403 while the sign-in still lists models, the card reads 'xAI is refusing requests' / 'Your sign-in still works, so signing in again won't help…' with Run again, not 'Provider sign-in required'. Sign out of xAI (or revoke the token at x.ai) and send: you should see 'Provider sign-in required'. Settings never marks the credential.
- task-180: ask a teammate to call request_human, then tell it you no longer need it to wait and have it withdraw_request: the 'needs you' card leaves the desk and the phone, and answering it is refused. An unanswered card expires after 2 h (or the wait_hours it chose, up to a day) and the teammate is told once. Expected cosmetic gap: hotline-mobile labels a withdrawn card 'Expired' until it regenerates its contract.
- task-181: open the new-teammate picker: the ACP agent list shows at once. With the network off, or cache/acp-registry.json more than a day old and the fetch failing, the list is still the last good one, never empty. A Codex teammate on a model its installed version doesn't support fails with 'Codex <version> is too old for this model' plus update the harness or switch model (no retry button).
- task-182: Settings › MCP, sign in to an OAuth MCP server: the browser tab that opens after sign-in is a dark card reading 'Signed in' / 'You can close this tab.' (light if the OS is light), and cancelling shows 'Sign-in failed' with the error. DevTools › Network shows a Content-Security-Policy of default-src 'none'; style-src 'unsafe-inline'… and no script.
- task-183, by hand (no wire test): link two desks, stop the other desk before it answers the link ask, then start it again: the link record stays, keeps dialing, and opens (or ends if the other side refuses). Differs from the brief: when the other desk ended the link and this desk's room.unlinked fails, the link shows 'That desk ended the link, and this desk could not end it too: …' and keeps dialing; it does not clear 'dialing'. Linking with a desk on 0.39 says 'Update that desk to link with it.'
- task-184: a teammate card's Threads row lists its pair threads with names and the last line. Against an older desk (0.39) the card still lists its threads, through the peers.list fallback (a UI test now covers this too: ui/tests/threads.test.tsx).
- task-185: a teammate with a computer, Desktop fold: under memory/CPUs it says 'Applies the next time the computer starts.' Change memory while the computer runs: 'Restart to apply' appears; press it: the restart lands between turns and the desktop comes back with the new limits (docker stats shows them). The phone owner card has no restart.
- task-186: pair this desk with two others, stop one (or make it unreachable), relaunch the app: the reachable desks appear without waiting on the dead one, which shows unreachable with 'The local desk bridge did not start in time.' within about 5 s and is retried.
- task-187: open Settings › Providers / Models while an xAI model list is slow: the rest of the window (sending, switching teammates) keeps answering; nothing waits behind models.list or search.
- task-188: on a fresh data dir with no provider connected (HOTLINE_DATA_DIR=/tmp/empty), open Hotline Agent: the conversation shows 'Hotline Agent unavailable' with a Settings › Providers button before you send anything; a message is refused and goes back to the composer. Connect xAI, send again: it answers. Known: connecting alone doesn't clear the error band; the next message does (task-97).
- task-37, on a Mac only (the Linux desktop has no Apple speech engine): with System Settings › Privacy › Speech Recognition refused for Hotline, a call or dictation that hears 3 s of voice says 'No words heard', and 'Use desk model' appears only when the desk hears with its own local model.
- task-49: open the longest tape you have (many Mermaid diagrams or computer frames): it opens quickly, diagrams draw as they scroll into view, and loading older lines at the top keeps your place. Expected: a diagram drawing late above you can still shift the view if you aren't pinned to the bottom.
- task-55: connect the phone over LAN, then switch its Wi-Fi off without closing the app: the desk drops that seat within about 60-90 s (Settings › Devices shows it gone), not many minutes later.
- task-89, on GitHub: Actions → ci → Run workflow, then read the shell-probe job on macos-26 and macos-15. If socket.getfqdn("127.0.0.1") is slow there, the 0.39.0 hang was a reverse-DNS lookup in http.server, not the Seatbelt sandbox; the Seatbelt profile is unchanged until that run is read.
