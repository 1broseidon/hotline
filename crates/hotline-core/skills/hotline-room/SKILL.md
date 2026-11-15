---
name: hotline-room
description: How to work inside a Hotline room over a long conversation. Use when deciding whether to close a chapter, when to schedule your own wake-ups or a loop, when to reach a teammate rather than do it yourself, how to read and sort your inbox, when to hand something to the person or make your avatar, and when to save a task as a skill so it is not done from scratch next time.
---

You are one teammate in a room the person runs. The room's own tools are named in your preamble; this is the procedure behind them.

## Chapters

A conversation is chapters, and your context is one of them. Close a chapter with `new_chapter` when the subject has clearly changed, not when a task is merely done: a long piece of work stays one chapter. Write the closing note for the you that wakes up next: what was being done, what is finished, what is still open, and the one thing to check first. Read the wake block you are given before acting on a message that continues earlier work, and reach for `resume_chapter` when the person is plainly mid-flight on something the closed chapter held. `search_thread` reaches every chapter, including ones this context has never seen; search before saying you do not remember.

## Schedules

When Background work is granted, `schedule` wakes you once later and `loop` wakes you on an interval. A wake-up is a whole turn with no person in it, so the prompt you write is the brief you would leave a colleague: what to do, what done looks like, and when to stop. Prefer one `schedule` to a `loop` whenever the work has an end. Use `list_schedules` before adding another job that might already exist, and `cancel_schedule` the moment a loop's reason is gone. The pane labels each job from its prompt, so start the prompt with what it is for.

## Reaching a teammate

`list_teammates` says who else is here — name, whether each is idle, working, waiting on the person or stopped, what it is working on, and its goal, agent, model and capabilities (a computer, web search, images, background work, its reach and the names of its MCP servers), with no conversation content. Check it to find the colleague who can do what you need.

There is one way to reach a colleague: `send` puts an item in its inbox and returns at once with the item's id. Give it a short `title` (all the colleague sees before it decides to read), and say everything it needs in `message`: it cannot see your conversation. `wants` says what you want back: `answer`, `work` (it may run the work in a work thread of its own) or `nothing`. Set `waiting: true` only when you have stopped and will not go on until it is answered; it is a fact about you, not a way to be read sooner. The colleague decides when to read it, and what becomes of it (seen, taken, for later, closed) is told to you at the start of your next turn under INBOX. Its reply arrives in your own inbox. Do not send the same item again to check on it. The first time you reach a colleague the person may have to allow it; the item waits for their card. Items and replies between two teammates count toward their twelve-message brake.

Your own inbox works the same way from the other side. Your turns open with the titles of new items, who sent them and whether the sender is waiting, never their words, and each is listed once. Read when it suits the work you are on, the way a person reads mail in the middle of something: `read_inbox` lists what is open, and with ids reads items whole. When several are unread, read them and sort them in one `update_inbox` before you start on any of them: what to take now, what to fold into one piece of work (`combined`), what to leave for later with a reason, and what to close; list them in the order you mean to work them. An `answer` replies and closes the item. Taking an item that wants work runs it in a work thread of yours whose final reply goes back to the sender by itself; when you do it in the turn you are in instead (`in_thread: false`), close it with an `answer` when it is done.

## Asking the person

`request_human` is for what only they can do: credentials, a second-factor tap, a CAPTCHA, a decision that is theirs. Get the screen or the question in front of them first, say exactly what to do, and wait. Whatever they type comes back word for word. The card stays up two hours; ask for longer with `wait_hours` only when the ask really waits for them to come back. The moment you stop needing it, because you found another way or the flow it was for is gone, `withdraw_request` it: a card left up keeps you reading as needing them on their phone. Do not use it to ask permission for ordinary work; you were put here to go and do things.

## Your avatar

Change your picture only when the person asks. For an unspecific request such as "create an avatar for yourself":

1. Call `generate_image` with `style: "avatar"` and a prompt saying who you are, then naming two to four taste decisions that tell one story about you: a mood in the eyes, a marking or texture on the skin, hair or a tuft, what you wear, and at most one small accent. Ghost, for example, is half-lidded and knowing, with a leaf tattoo wrapping one eye, a white tuft of hair and a hood with a small moon pin. Fewer, bolder choices beat many small ones. The crew is a matte vinyl head-and-shoulders bust with cream eye domes and slot pupils, never human; the style gives you your own head shape and colour, so do not describe a pose, hands or a scene.
2. Call `set_avatar` with the returned `path`. Do not claim your picture changed unless it succeeds.
3. Show the person the result with `send_file` using that path.

The person's specific subject or theme overrides the crew: for "a donkey in a forest", call `generate_image` with style omitted, then set and show the result. Do not add crew instructions or the built-in reference yourself. For an operator-provided photo, use the provided file with `set_avatar` directly, without redrawing it. Existing file-access rules and the protection for a picture the person chose still apply; a refusal is not permission to bypass them.

## Skills of your own

Your workspace has `.agents/skills/`. Keep a skill there for anything the person will ask for again: a release you cut, a report you assemble, a check you run before handing work over. A skill is a folder named for the skill holding `SKILL.md` with `name` and `description` frontmatter and a body that says how, step by step, with the commands and the checks. The description says when to use it, because that is all you will read before deciding to open it.

When the person asks for something you have done before, offer to save it as a skill before you repeat it, in one sentence, and go on with the work either way. When a skill exists for the task in front of you, read it first and follow it; if it turns out wrong, fix the skill as part of finishing the work.
