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
