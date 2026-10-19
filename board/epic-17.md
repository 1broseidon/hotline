---
id: epic-17
title: "Team lead and one voice path: calling the desk reaches a real teammate, and both calls sound natural (0.41)"
type: epic
column: todo
position: 0
priority: high
tags:
  - voice
  - team-lead
  - "0.41"
assignee: claude
createdAt: "2026-10-19T13:59:36.300Z"
landsOn: release/0.41.0
updatedAt: "2026-10-19T20:00:48.164Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-19T20:00:45.071Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:00:48.164Z"
---

## Description
From George's call, 2026-10-19 08:52-08:58 (phone, on 0.40 through the Cloudflare proxy).

What's wrong today:
- Calling the desk feels relayed and slow. The desk's voice rules (voice/front_prompt.rs DESK_RULES) give "Got it, passing that on." as the acknowledgement, so it says it; it starts with no context (duplex.rs front_instructions passes Start::default(): no name, no recent lines, no roster, no lookup tools); and what it hands on goes to a second model (the dispatcher) which then picks a teammate: an extra round trip before work starts.
- On both kinds of call, the acknowledgement is copied from the prompt's two examples ("One sec, I'll pull that up.", "On it.") and often doesn't fit what was asked; sometimes it's skipped and the person hears silence while their words land in the chat. There is no threshold: the front model decides per turn whether to answer, look up or hand off.
- The voice sometimes guesses instead of checking (on this call: "it depends on what the desk is set to offer", "there is likely one in the transcript handling"), against its own rule.

George's decisions:
- A team lead is a role, not a new kind of agent: right-click a teammate (or its card) and Make team lead. One per desk; it keeps its own model, memory and tools, ACP or Hotline Agent.
- Calling the desk calls the team lead, through the same voice path as calling any teammate.
- The first teammate created in onboarding becomes lead, and onboarding says so in a line. A desk with exactly one teammate sets that one as lead (on upgrade too). A desk with two or more and no lead shows a small prompt to pick one, with a suggestion; nothing is picked silently. Removing the lead asks again.
- Acknowledgements are generated, not chosen from a list: a few words naming what was asked ("Let me look at the voice code"), with bad examples rather than good ones in the prompt, the call's recent acknowledgements fed back so it can't repeat itself, and a soft sound rather than words if a hand-off goes out with nothing said.

Until a lead exists (an old phone, or a desk mid-choice), desk calls behave as today. Children land on release/0.41.0; the phone part is on hotline-mobile's board.

## Log
- 2026-10-19T17:20:45.934Z: [claude] George, 2026-10-19 12:20: voice is a modality, not a separate agent (see task-213 in epic-19). Calling a teammate means talking to the full agent itself, speech in and out; the separate live-voice front answering on its behalf goes. epic-17 waits for epic-19's design: its 'one voice path' and acknowledgement tasks get rewritten against it; the team lead role (calling the desk reaches the lead) stands as is.
- 2026-10-19T17:32:37.815Z: [claude] Also waits for epic-20's ADR (task-215, the agent thread layer): acknowledgements and the chat pointer (task-204, task-207) are redone against it; the team lead role stands.
- 2026-10-19T20:00:05.471Z: [claude] George 14:58: epic-17 is split by release. 0.41: the team lead role (task-202) and its desktop UI (task-205), landing on release/0.41.0. 0.42 (epic-21): calling the desk reaches the lead (task-203, now ADR 0008 slice 7), acknowledgements (task-204) and the chat pointer (task-207), both rewritten into slice 6's front, and the end-to-end/docs (task-206). task-201 is closed, superseded by ADR 0008.
- 2026-10-19T20:02:48.635Z: [claude] Lead plan (0.41 half, lands on release/0.41.0): task-202 (core + contract: the lead on the room stream and roster, set/move/clear commands under owner checks, auto-lead for a one-teammate desk at create and load, cleared on delete) first, then task-205 (desktop: menu, badge, pick-a-lead prompt, onboarding line, CHANGELOG), which waits on 202 because it drives 202's commands and reads its roster field. task-205 is also the wiring child: its make verify runs 202's lead harness on the merged branch. Contracts added to both (validation: lead harness tests exist, make check, contract.ts written whole, make verify; 205 adds ui/tests/team-lead.test.tsx). The first-contact grant left 202 for task-203 (epic-21, 0.42), where desk calls first reach the lead; a grant with no caller would be dead code. relatedFiles fixed: room.rs now lives in crates/hotline-room. Both dispatched to claude/opus.
