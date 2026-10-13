---
title: Calls and dictation
description: Talk to a teammate instead of typing, call the whole desk, and choose what hears and speaks.
---

There are three ways to use your voice in Hotline:

- **Dictation** turns what you say into words in the message box, for you to
  read before you send them.
- **A call to a teammate** is a spoken conversation with one teammate. It is
  the same conversation as the chat, so the teammate remembers the call and
  everything it does on the call appears in the chat.
- **A call to the desk** is a spoken conversation with the whole room. An
  assistant answers and hands work to teammates by name.

Voice needs no key of its own. It uses the providers you have already
connected, a speech model the desk downloads, or, on a Mac, the Mac's own
speech recognition.

## Something to hear you

Before you can dictate, Hotline needs something that turns speech into
words. Open **Settings → Providers → Use for → Transcription** and choose
**Set up**. There are three kinds:

- **On this Mac.** Apple's on-device recognition. It is free and private,
  and nothing you say leaves the Mac. The language model downloads once.
- **On the desk.** Speech models Hotline downloads once and runs itself, on
  macOS, Windows, Linux or a server. They are free and private, and they cost
  nothing against a budget.

  | Model | Good for | Download |
  | --- | --- | --- |
  | Parakeet | 25 European languages, the most accurate | 487 MB |
  | Whisper | 99 languages, slower | 564 MB |
  | Parakeet English | English, small and quick | 108 MB |
  | Moonshine | English | 251 MB |

  Each one shows its download progress and can be cancelled or removed. Once
  a model is installed, the desk hears you with it first, from every device
  that talks to this desk.
- **A provider you connected**, such as OpenAI, Google, Groq, Mistral,
  OpenRouter or xAI. These are paid by the request and count against your
  **Voice** budget.

**Words to listen for**, under Transcription, lists names the Parakeet models
should expect. They already know your teammates' names, Hotline, and the
providers you use, so they hear "Mack" rather than "Mac". Add anything else,
such as a project name.

To be spoken to on a call, the desk also needs a provider that can speak:
the **Voice** row under **Use for**. Without one, a call can still hear you,
but it has no voice.

## Dictate

With an empty message box, the key at its right is a microphone (**Dictate**).

- **Tap** it to start, and tap again to stop.
- **Hold** it to talk for as long as you hold it down.

Your words fill in as you speak. Press <kbd>Enter</kbd> to stop and
<kbd>Esc</kbd> to cancel and put the box back the way it was. Nothing is
sent until you send it.

**Settings → General → Shortcuts** has a **Dictate** shortcut that works from
any app (<kbd>Ctrl</kbd> + <kbd>Option</kbd> + <kbd>H</kbd> by default),
and **After dictating**:

- **Don't send** (the default) leaves the words in the box.
- **Send** sends them a second and a half after you stop talking, unless
  you press <kbd>Esc</kbd> first.

If the microphone is missing, busy or not allowed, the box says which. On a
Mac, allow Hotline under **Speech Recognition** and **Microphone** in
**System Settings → Privacy & Security**.

If nothing can hear you yet, the empty box shows the call key instead of
the microphone.

## Call a teammate

Press the phone key in the band at the top of the conversation
(**Call** and the teammate's name). If dictation is not set up, the key in an
empty message box (**Talk to** and the teammate's name) starts a call too.

On a call, the teammate is itself. No other model answers for it. What you
say goes into its own conversation, in the chapter you are in, with the same
folders, tools and permissions as when you type to it.

- The teammate answers twice: once in a short reply written to be heard, and
  once in the full reply written to be read, with the code, tables and links.
  The chat shows the full reply as an ordinary message. A quiet line above
  the full reply shows what was said on the call; press it to read it.
- While the teammate works, the call plays a soft blip-blip and keeps
  listening. You do not need to tap anything to talk: speak, and what you say
  goes into the work it already has open.
- Talking while it speaks cuts it off. That stops the speech, not the work.
- Anything that needs your approval still arrives as a card in the
  conversation. A call never takes a spoken yes as an approval.

The call floats as a card in the window's bottom corner, with **Hold**,
**Hang up**, and the teammate's mark, which you press to cut in. Its clock
counts only the time you were talking, so time on hold does not count. After
ten quiet minutes, the call ends by itself.

When it ends, the conversation keeps one quiet line for the call, such as
"Call · 4 min · Hung up". That line opens what was said. Teammates can find
earlier calls when they search their own conversations.

A teammate can have a voice of its own. Choose it in the **Voice** row of its
pane. Otherwise it speaks in the desk's voice.

## Call the desk

The phone in the window's title bar (**Talk to the desk**) calls the whole
room. An assistant answers in a sentence or two, hands work to your teammates
by name, and tells you when it lands. Use it when you want to start several
things at once without opening each conversation.

Pick the model that answers in **Settings → Providers → Use for → Call
assistant**. The same assistant also rewrites a teammate's reply for speech
when the teammate wrote only the full version. A call to the desk is not
kept as a thread; the work it hands out lands in each teammate's conversation.

**Settings → General → Shortcuts** has **Call your agent** and **Call the
desk**, which work from any app. Both are off until you give them a key.
Pressing either during a call hangs up.

## What it costs

Speech from a provider you pay is counted against the **Voice** budget, and
the call assistant's replies against the **Chat** budget. See
[Budgets](/docs/setup/providers/#budgets). A call that runs on the Mac's own
recognition, on a desk model, or on a subscription is never ended by a spent
budget. A call that a spent budget refuses names the budget.

## From your phone

The phone can call the desk and a teammate too. Hotline for iPhone is in
beta; see [Hotline on your phone](/docs/phone/).
