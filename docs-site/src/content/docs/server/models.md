---
title: Connect a model
description: A one-time step on the server so your teammates have something to run on.
---

A teammate runs either on **Hotline Agent**, the built-in agent, with a model
from a provider you connect, or on a **harness** such as Claude Code or Codex
that brings its own sign-in. On a server, connecting providers is a one-time
step you do on the server itself. Your phone can't do it.

Every command here runs as the service account against the running desk.
`hotline wire` reads its parameters from standard input, never from the
command line, so a key stays out of your shell history and the process list.
Start with a variable for the account:

```sh
H="sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline"
```

:::note
This is the stopgap. An interactive `hotline setup` that walks you through
providers, sign-ins and MCP servers is planned; until then, these commands are
the way.
:::

## A provider with an API key

List the providers and their ids:

```sh
echo '{}' | $H wire providers.list
```

Then add a key, typing it at a hidden prompt so it doesn't land in your
history:

```sh
read -rs KEY && printf '{"providerId":"anthropic","label":"Server","secret":"%s"}' "$KEY" | $H wire credential.create; unset KEY
```

Ids include `anthropic`, `openai`, `openrouter`, `google`, `xai`, `zai`,
`zai-coding-plan`, `groq`, `deepseek`, `mistral` and `ollama-cloud`.
`hotline status` then lists the model under **models**.

## A local model server

For Ollama, give its address. No key is stored:

```sh
echo '{"baseUrl":"http://127.0.0.1:11434"}' | $H wire credential.connect_local
```

## Providers that sign in

ChatGPT, GitHub Copilot, and the sign-in forms of OpenRouter and xAI use a
browser sign-in that `hotline wire` can't finish on its own yet. Use an API
key for now, or connect the provider in the desktop app and use that machine.

## A harness that is already signed in

A harness uses its own login, not Hotline's, so a teammate on Claude Code
needs no provider at all if Claude Code is signed in **for the `hotline`
account**. Harnesses live under that account's home (`/var/lib/hotline`),
not yours. Sign in as it, for example:

```sh
sudo -u hotline -H bash -lc 'npx -y @anthropic-ai/claude-code'   # then /login
```

When you add a teammate on the phone, the harness list shows what this
server can run. Choose the signed-in harness there.
