---
title: Providers and keys
description: The AI services Hotline Agent runs on, how each connects, what draws and speaks, and what it may spend.
---

Hotline Agent, the built-in agent, runs on a model from an AI service you
connect under **Settings → Providers**. The same connections draw pictures,
hear and speak on calls. Keys, and the OpenRouter and xAI sign-ins, are kept
in your computer's keychain (the macOS Keychain, Windows Credential Manager,
or the Secret Service on Linux). ChatGPT and GitHub Copilot sign-ins are kept
in files only your account can read, in Hotline's
[data directory](/docs/reference/data/). None of them is ever written into a
conversation, a teammate's files or a log.

## Providers

**Add provider** lists every service not yet connected:

| Provider | Connects with |
| --- | --- |
| Anthropic | API key |
| OpenAI | API key |
| ChatGPT | Sign in with your ChatGPT subscription |
| OpenRouter | Sign in, or API key |
| Google | API key |
| xAI | Sign in with SuperGrok or X Premium+, or API key |
| Z.AI, and Z.AI Coding Plan | API key |
| Groq | API key |
| DeepSeek | API key |
| Mistral | API key |
| GitHub Copilot | Sign in |
| Ollama Local | A server URL, no key |
| Ollama Cloud | API key |
| OpenAI-compatible | Any endpoint that speaks the OpenAI API: URL, optional key, your own model IDs, and whether it takes Responses or Chat Completions |

An OpenAI-compatible server on the Responses API does not have to match
OpenAI's stream exactly. Servers with a looser stream, such as Meta's Muse
models at `api.meta.ai`, work from Hotline 0.36.2.

### Claude

Hotline connects to Anthropic with an API key only. It does not offer a
Claude subscription sign-in, and never asks for or holds a Claude login.
To work on a Claude subscription, add a teammate that runs
[Claude Code](/docs/setup/teammates/): Claude Code signs itself in with
your account, and Hotline runs the `claude` CLI you installed. Anthropic's
plans and terms decide what that use costs and whether your plan covers
Claude Code run from another app, so check them. To run Claude Code
teammates on more than one Claude subscription, give each an
[agent account](/docs/setup/agent-accounts/).

## Models

Open a connection to see it. The top shows how it is connected, with
**Change connection**, and **Sign out** for a sign-in or **Remove key** for
a key. An OpenAI-compatible connection has **Edit connection** and **Remove
connection** instead, and Ollama Local has **Disconnect**. Under it,
**Models in the picker** chooses **All** or **Only some**, each switched on
or off and saved at once.

Hotline ships a catalogue of the models each provider serves, and where the
provider can list its own models, **Refresh** reads them again, so a newly
released model is usable before Hotline ships an update. **A model by ID**
adds one the list is missing.

For Ollama Local, Hotline lists whatever the server has installed. For an
OpenAI-compatible endpoint you supply the model IDs, and Hotline lists them
from the server when it can.

GitHub Copilot lists the models your account is offered, and sends each
model to the endpoint Copilot names for it. A sign-in that has no record of
those endpoints fetches them on its first Copilot turn. If a Copilot model
still refuses with a message about `/chat/completions`, press **Refresh** on
the Copilot connection.

## The default model

**Settings → General → New teammates → Model** is the model a new Hotline
Agent teammate starts on. Change any teammate's model, and its reasoning
effort, from the picker beside its name at the head of its conversation.

## Use for

Under **Use for**, pick which connected service does each of the other jobs.
Each starts on **Automatic**:

- **Images**: what draws when a teammate uses `generate_image`. See
  [Pictures](/docs/use/images/).
- **Voice**: what speaks on calls.
- **Transcription**: what hears you, including the free speech models the
  desk can download and, on a Mac, the Mac's own recognition.
- **Call assistant**: the model that answers calls to the desk, with a
  thinking level where the model offers one.

See [Calls and dictation](/docs/use/voice/).

## Budgets

**Budgets**, under Use for, shows what paid use has cost today and this
month, as three budgets:

| Budget | Counts |
| --- | --- |
| Chat | Hotline Agent teammates on an API key, and the call assistant |
| Voice | Paid speech: hearing and speaking on calls |
| Images | Pictures drawn on an API key |

Open one to see what it went on and to set a daily and a monthly limit.
There is no limit until you set one; a limit of 0 turns that kind of paid use
off. When the Chat budget is spent, a teammate's turn stops with "Chat budget
spent" before anything is sent.

Sign-ins to a subscription, local models, the desk's own speech models and
coding tools such as Claude Code are not counted: their cost is the
subscription's, or nothing.

## Teammates that bring their own login

Teammates that run Claude Code, Codex, Cursor, opencode, Gemini CLI, Grok
Build or Muse Code sign in through that tool, not through Hotline. Hotline
holds no key for them. See [Teammates and drivers](/docs/setup/teammates/).
