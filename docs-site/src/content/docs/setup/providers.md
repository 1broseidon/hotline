---
title: Providers and keys
description: The model providers Hotline Agent can run on, and how each one connects.
---

Hotline Agent, the built-in agent, runs on a model from a provider you connect
under **Settings → Providers**. Keys live in Hotline's vault on this machine and
are never written into a conversation, a teammate's files or a log.

## Providers

| Provider | Connects with |
| --- | --- |
| Anthropic | API key |
| OpenAI | API key |
| ChatGPT | Sign in with your ChatGPT subscription |
| OpenRouter | Sign in, or API key |
| Google | API key |
| xAI | Sign in (Grok device sign-in), or API key |
| Z.AI, and Z.AI Coding Plan | API key |
| Groq | API key |
| DeepSeek | API key |
| Mistral | API key |
| GitHub Copilot | Sign in |
| Ollama Local | A server URL, no key |
| Ollama Cloud | API key |
| OpenAI-compatible | Any endpoint that speaks the OpenAI API: URL, optional key, your own model IDs |

**Add provider** lists what is not connected yet. Once every provider is
connected the list says so.

## Models

Each connection has a **Connection** and a **Models shown** section. Hotline
ships a catalogue of the models each provider serves and refreshes the list
from the provider itself when it can, so a newly released model is usable
before Hotline ships an update. Untick models you do not want in the picker;
every model checked is the same as no filter. **Manual model IDs** adds an
ID the provider has not listed.

For Ollama Local, Hotline discovers whatever the server has installed. For an
OpenAI-compatible endpoint you supply the model IDs, and Hotline will list them
from the server when it can.

GitHub Copilot lists the models your account is offered, and each model is
sent to the endpoint Copilot names for it, so Grok and the newer OpenAI
models work alongside the Claude ones. If a Copilot model refuses with a
message about `/chat/completions`, press **Refresh** on the Copilot
connection once so the list is read again with its endpoints.

## The default model

**Settings → General → Default model** is what a new Hotline Agent teammate
starts on. **Last used** picks up whatever you chose most recently. Each
teammate's model can be changed any time from the picker in its title bar.

## Teammates that bring their own login

Teammates that run Claude Code, Codex, Cursor, opencode, Gemini CLI or Grok
Build sign in through that tool, not through Hotline. Hotline holds no key for
them. See [Teammates and drivers](/docs/setup/teammates/).
