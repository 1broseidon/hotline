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
Claude Code run from another app, so check them.

## Billing

Who you pay depends on how each teammate signs in. A coding tool uses its
own login; Hotline Agent uses the connection you add here.

### Coding tools

| Integration | Signs in with | Billed to |
| --- | --- | --- |
| Claude Code | The `claude` CLI's own login, or `ANTHROPIC_API_KEY` | The Claude account `claude` is signed in to, or the key's Anthropic API account |
| Codex | The `codex` CLI's own login, or `OPENAI_API_KEY` | The ChatGPT account `codex` is signed in to, or the key's OpenAI API account |
| Cursor | `cursor-agent`'s login, your Cursor account | Your Cursor account |
| Grok Build | `grok`'s login, your Grok account | Your Grok account |
| Gemini CLI | `gemini`'s own sign-in | The account or key it signed in with |
| opencode | The providers you set up in opencode | Each of those providers |
| Muse Code | `muse`'s own login | Your Muse Code account |
| Agents from the registry, such as Google Antigravity | Their own sign-in | Their own account; Antigravity names its Google plan beside the agent |

Hotline holds no key or login for these and counts none of it against a
[budget](#budgets). When a tool isn't signed in, the failed turn's card
offers **Sign in** with the methods the tool itself offers.

### Hotline Agent

| Provider | Signs in with | Billed to | Chat budget |
| --- | --- | --- | --- |
| Anthropic | API key | Anthropic API, per token | Counted |
| OpenAI | API key | OpenAI API, per token | Counted |
| ChatGPT | Sign in | Your ChatGPT subscription | Not counted |
| GitHub Copilot | Sign in | Your Copilot subscription | Not counted |
| xAI | Sign in with SuperGrok or X Premium+ | That subscription | Not counted |
| xAI | API key | xAI API, per token | Counted |
| OpenRouter | Sign in, or API key | Your OpenRouter credits, per token. The sign-in makes a key on your account. | Key counted; sign-in not counted |
| Google, Groq, DeepSeek, Mistral, Z.AI | API key | That provider's API, per token | Counted |
| Z.AI Coding Plan | API key | Your Coding Plan | Listed at no cost |
| Ollama Cloud | API key | Your Ollama account | Counted at listed prices |
| Ollama Local | Server URL | Nothing: it runs on your machine | Not counted |
| OpenAI-compatible | URL, optional key | Whoever runs the server | Counted with a key |

A model with no listed price on a key is counted at a deliberately high
guard rate, so its budget runs down faster than the bill.

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

**Use for → Images** picks what draws when a teammate uses
`generate_image`, starting on **Automatic**. See
[Pictures](/docs/use/images/).

**Calls**, below it, picks how calls run, what hears and speaks on them, and
the model that answers calls to the desk. See
[Calls and dictation](/docs/use/voice/#set-up-calls).

## Budgets

**Budgets**, under Calls, shows what paid use has cost today and this
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
