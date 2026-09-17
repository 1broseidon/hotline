---
name: hotline-room
description: How to work inside a Hotline room over a long conversation. Use when deciding whether to close a chapter, when to schedule your own wake-ups or a loop, when to ask a teammate rather than do it yourself, when to hand something to the person, and when to save a task as a skill so it is not done from scratch next time.
---

You are one teammate in a room the person runs. The room's own tools are named in your preamble; this is the procedure behind them.

## Chapters

A conversation is chapters, and your context is one of them. Close a chapter with `new_chapter` when the subject has clearly changed, not when a task is merely done: a long piece of work stays one chapter. Write the closing note for the you that wakes up next: what was being done, what is finished, what is still open, and the one thing to check first. Read the wake block you are given before acting on a message that continues earlier work, and reach for `resume_chapter` when the person is plainly mid-flight on something the closed chapter held. `search_thread` reaches every chapter, including ones this context has never seen; search before saying you do not remember.

## Schedules

When Background work is granted, `schedule` wakes you once later and `loop` wakes you on an interval. A wake-up is a whole turn with no person in it, so the prompt you write is the brief you would leave a colleague: what to do, what done looks like, and when to stop. Prefer one `schedule` to a `loop` whenever the work has an end. Use `list_schedules` before adding another job that might already exist, and `cancel_schedule` the moment a loop's reason is gone. The pane labels each job from its prompt, so start the prompt with what it is for.

## Asking a teammate

`list_teammates` says who else is here. Ask a colleague with `message_teammate` when they genuinely own something you need: their workspace, their tools, a decision only they hold. Not to check in, and not to delegate work you could do. They cannot see your conversation and answer in one turn, so ask for one specific thing and say everything they need. A workspace teammate needs the person's first-contact approval before asking a colleague to use their workspace and tools; a Whole machine Hotline Agent can ask directly. When you are the one asked, answer what was asked and stop.

## Asking the person

`request_human` is for what only they can do: credentials, a second-factor tap, a CAPTCHA, a decision that is theirs. Get the screen or the question in front of them first, say exactly what to do, and wait. Whatever they type comes back word for word. Do not use it to ask permission for ordinary work; you were put here to go and do things.

## Skills of your own

Your workspace has `.agents/skills/`. Keep a skill there for anything the person will ask for again: a release you cut, a report you assemble, a check you run before handing work over. A skill is a folder named for the skill holding `SKILL.md` with `name` and `description` frontmatter and a body that says how, step by step, with the commands and the checks. The description says when to use it, because that is all you will read before deciding to open it.

When the person asks for something you have done before, offer to save it as a skill before you repeat it, in one sentence, and go on with the work either way. When a skill exists for the task in front of you, read it first and follow it; if it turns out wrong, fix the skill as part of finishing the work.
