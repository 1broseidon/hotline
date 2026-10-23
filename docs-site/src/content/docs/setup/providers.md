---
title: Providers and accounts
description: The AI services your teammates run on, the accounts you keep for each, how a teammate picks one and moves to another, and what draws, speaks and spends.
---

Hotline Agent, the built-in agent, runs on a model from an AI service you
connect under **Settings → Providers**. The same connections draw pictures,
hear and speak on calls. Claude Code teammates can run on Claude
subscriptions you keep there too. Keys, and the OpenRouter and xAI sign-ins,
are kept in your computer's keychain (the macOS Keychain, Windows Credential
Manager, or the Secret Service on Linux). ChatGPT and GitHub Copilot sign-ins
are kept in files only your account can read, in Hotline's
[data directory](/docs/reference/data/). None of them is ever written into a
conversation, a teammate's files or a log.

## Providers

**Add provider** lists every service not yet connected:

| Provider | Connects with |
| --- | --- |
| Anthropic | API key, or a Claude subscription for Claude Code teammates |
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

Each provider you have connected is one row, which says what it holds, such
as "2 subscriptions · 1 API key · All models", or **Signed out** when none of
its accounts works any more. An OpenAI-compatible connection is a row of its
own, with **Edit connection** and **Remove connection** on its page.

### Claude

Hotline connects to Anthropic with an API key for Hotline Agent. A Claude
subscription works only through [Claude Code](/docs/setup/teammates/):
Claude Code signs itself in with your account, and Hotline runs the `claude`
CLI you installed and never asks for or holds a Claude login. Anthropic's
plans and terms decide what that use costs and whether your plan covers
Claude Code run from another app, so check them. To run Claude Code
teammates on more than one Claude subscription, add each under Anthropic
(see [Claude subscriptions](#claude-subscriptions)).

## Accounts

An account is one way into a provider, with a name you choose: a key, a
sign-in, a server, or a Claude subscription. A provider can hold any number
of each. Open a provider to see its page, where its accounts are listed in
up to three sections:

- **Subscriptions**: sign-ins to a plan, such as ChatGPT, GitHub Copilot or
  a Grok subscription. On Anthropic this is where the Claude subscriptions
  are, with **This computer**: Claude Code's own sign-in, as before, which
  every Claude Code teammate without a subscription of its own uses.
- **API keys**: keys, billed per use. An OpenRouter sign-in is here too,
  because it makes a key that is billed per use.
- **Servers**: Ollama Local servers.

Each account says how it is signed in and **Used by** which teammates. The
first of each section's rows adds one: **Add subscription**, **Add API key**
or **Add server**. Leave its name blank and it takes the next free one
("Anthropic", "Anthropic 2", and so on, or **Account N** for a Claude
subscription). Two accounts of one provider and section can't share a name,
whatever its case; two keys added before names were checked keep theirs and
are told apart by the day they were added. Your first sign-in, from the
welcome, takes no name and starts in one click.

On a key, a sign-in or a server:

- **Rename** changes its name in place.
- **Change key**, **Change server** or **Sign in again** changes that
  account in place: the teammates on it stay on it, and nothing restarts. A
  sign-in again that you cancel, or that fails, leaves the account as it
  was.
- **Remove key**, **Disconnect** or **Sign out** asks once, then removes it.
  Hotline forgets a key, but it still works at the provider until you revoke
  it there; **Revoke at GitHub** opens GitHub's page for a Copilot sign-in.
  Removing an account a teammate uses is refused, naming the teammates
  ("Work can't be removed while Ada uses it."): give them another account
  first, since Hotline never moves a teammate to another account for you.
- A signed-out one offers only **Remove**.

A Claude subscription has **Rename** on its row and opens its own page with
**›** ([Claude subscriptions](#claude-subscriptions)).

**Models in the picker** is under the sections; see [Models](#models).

### The default

A Hotline Agent teammate that has no account of its own runs on its model's
provider's default: the first of that provider's accounts that still works,
marked **Default** on the page when the provider has more than one. You
can't choose another default. A default nobody chose by name can be removed,
and the teammates that followed it move to the next. Speech on calls,
pictures, chapter notes and the list of models use the provider's default
too, whatever account a teammate is on.

### Claude subscriptions

A Claude subscription is one sign-in to Claude Code, kept in a folder of its
own on this computer. Give two teammates two subscriptions and each works on
its own Claude plan, at the same time. Hotline never sees a password or
token: Claude Code signs itself in, into the account's folder, and Hotline
only asks Claude Code whether it is signed in.

Add one under **Settings → Providers → Anthropic → Add subscription**, or
**Add provider → Anthropic → Claude subscription** when Anthropic is not
there yet. Once you have one, two more places offer **Add subscription**:
the **Account** list in **New teammate**, with Claude Code as the
**Agent**, and **Claude Code account** in the teammate's pane.

Give it a name, or leave it blank for the next **Account N**, and, if you
like, the email of the Claude account. The email only fills in Claude Code's
sign-in page, so the right account is used; Hotline does not keep it.
**Add and sign in** makes the account and starts its sign-in. One added in
**New teammate** is picked for the new teammate at once; one added in the
pane is given to the teammate once it is signed in. Copying a large setup,
such as plugins with their own dependencies, can take a few minutes; until
it ends the button reads **Adding…**, and the same name can't be added
anywhere else.

#### What a new account starts with

In New teammate and the pane, a new account starts with your Claude Code
setup; **Change or customize** opens the choice. Under **Start with**:

- **My Claude Code setup**: a copy of your own Claude Code setup, without the
  sign-in, keys or history. **Customize what is copied…** picks among
  **Instructions and rules**, **Skills, agents and commands**, **Plugins**,
  **MCP servers** and **Settings**.
- **Empty**: Claude Code's defaults; add what this account needs afterwards.

Your own setup is the folder `CLAUDE_CONFIG_DIR` names in Hotline's own
environment, or else `~/.claude` and `~/.claude.json`. Hotline only reads it.
Never copied: the sign-in (`.credentials.json`), `history.jsonl`,
`projects/`, `sessions/`, `todos/` and Claude Code's other records, any key
of `~/.claude.json` but `mcpServers`, and the sign-in keys of
`settings.json` (such as `apiKeyHelper`, or an `env` variable that would
replace the login), which are left out; **Copy again from This computer**
names the ones it left out. A link in your setup is skipped, not followed.

#### Sign in

- **Windows**: a Claude Code window opens on the desk's own screen. Finish
  signing in there; Hotline checks when it closes. Signing in starts only from
  the desk itself, not from another computer paired with it.
- **macOS and Linux**: the sign-in runs in a terminal in the window, which you
  can type into.

The browser uses whatever Claude sign-in is already open in it: use
**Switch account** on the page, or open the link in the right browser
profile. The account reads **Signed in**, with its email, only once Claude
Code confirms a claude.ai login in that account's own folder. A sign-in you
cancel, close or that times out changes nothing, and an account that was
signed in before stays signed in. **Check** asks Claude Code again.

A teammate on an account that is not signed in stops at its first message,
with a card that offers **Sign in …** and **Choose account**. A second
sign-in on an account starts its teammates fresh once their work ends.

#### Setup

An account's page, and the **…'s setup** link in the teammate's pane, show
the account's setup: **Instructions** and **Settings**, then **Skills**,
**Plugins** and **MCP servers** with how many of each it holds.

- **Open**, on the desk itself, opens the file in your editor or the folder
  in your file manager. A setup item that is missing is made first, directly
  in the account's folder: an empty `.md`, a `{}` `.json`, or the folder
  (`CLAUDE.md`, `settings.json` or `.claude.json`, `skills/` or
  `plugins/`); nothing that runs.
- Hotline counts what is there, but does not check or act on what you write.
  It applies from the next session of the teammates on that account, and
  every teammate on the account shares it.
- **Copy again from This computer** brings over what you changed in your
  own setup since the account was made. The ticked items replace the
  account's, including your changes to them; **Settings** is unticked unless
  you tick it. The sign-in is not touched.

#### The folder

Each account is a folder in Hotline's data directory,
`agent-accounts/<id>`, private to your user. Claude Code keeps the
account's sign-in, history and settings there. The account's page shows it,
with **Show folder**.

**Remove account** is refused while a teammate uses the account. Removing
an unused account makes Hotline forget it, and leaves the folder and the
sign-in in it. To sign it out, run Claude Code's own sign-out with
`CLAUDE_CONFIG_DIR` set to the folder:

```sh
CLAUDE_CONFIG_DIR="<folder>" claude auth logout
```

In PowerShell, clear the variable afterwards so the rest of that window is
not left on the account:

```powershell
$env:CLAUDE_CONFIG_DIR = "<folder>"; claude auth logout
Remove-Item Env:CLAUDE_CONFIG_DIR
```

#### What a Claude subscription does not cover

- Claude Code also reads settings in the teammate's folder
  (`.claude/settings.json` and `.claude/settings.local.json`). One that sets
  `env` or `apiKeyHelper` can replace the account's sign-in, as it can for
  This computer today, and Hotline does not see it.
- Settings your organization manages on this computer still apply.
- The account's folder is not hidden from Claude Code itself or from a
  teammate with **Whole machine**: any program running as your user can read
  it.
- Anthropic's plans and terms decide whether running several subscriptions
  this way is allowed, so check them.

## A teammate's account

A teammate offers a choice of account only when more than one fits it:

- A Hotline Agent teammate picks among the accounts of its model's provider.
- A Claude Code teammate picks among your Claude subscriptions and **This
  computer**. It is offered subscriptions only, never an API key.

The choice never crosses providers. A ChatGPT sign-in and an OpenAI key are
two providers with different models, so a teammate on a ChatGPT model picks
among ChatGPT sign-ins, and one on an OpenAI model among OpenAI keys. To
move from one to the other, change the teammate's model.

In **New teammate**, the **Account** field shows under the model for Hotline
Agent, or under the agent for Claude Code, and starts on the provider's
default or on This computer. Its list ends with **Add subscription** or
**Add API key**, where the provider offers them; an account added there is
picked at once. Choosing a model of another provider puts the field back on
that provider's default. Rename or remove accounts later in Settings →
Providers.

In the teammate's pane, **… account** lists the accounts that fit as a
choice, with **In use** beside the one running, and ends with the same
**Add** rows. Picking a provider's default keeps the teammate following the
default, so it moves when the default does.

## Switching accounts

Pick another account in the pane and the teammate moves to it when its work
ends: no turn in its conversation, and no side thread, call, job, subagent or
conversation with another teammate mid-turn. Until then the pane says
**Switches to … when its work ends**, and offers:

- **Keep …**: stay on the account in use, and change nothing.
- **Stop and switch now**: stop all of the teammate's work and start it on
  the new account.

A Claude subscription that is not signed in, or still signing in, waits: the
teammate keeps its current account until it is.

A switch starts a new session on the new account, handed this conversation
(**Fresh**). On Claude Code, another account's session is never reopened, so
going back to the first account later starts fresh too.

If you are on a call with the teammate when the switch lands, the call can
still talk but can no longer look anything up or work. Hang up and call
again. The call's thread is closed, so continue it in the Threads pane before
calling it again, or start a new call.

A Hotline Agent teammate that changes to a model of another provider runs on
that provider's default from its next message, and its own account applies
again when it comes back. If the account it is set to is signed out, its
next message stops with a card that says so and offers **Choose account**;
Hotline never moves it to another account by itself.

## Usage limits

When an account reaches its usage limit, the card says so: a Claude Code
limit in Claude Code's own words, a ChatGPT sign-in's limit, or a Hotline
Agent key that has run out of credit. Where the provider says when the limit
resets, the card says it as a time on your clock ("Resets at 5:40 PM.", or
"Resets tomorrow at 9:10 AM.").

When another account fits the teammate, the card offers to move it:
**Continue on** with a list of the other accounts, then **Continue** and
**Try again later**. The list opens over the card, under **Subscriptions**
and **API keys · pay per use**, with a search once there are more than eight.
It leaves out the account that reached its limit and, for Claude Code,
offers only subscriptions that are signed in, and This computer. It starts
on the first subscription it lists, else This computer, else the first key.

- **Continue** moves the teammate at once and starts it fresh: "Now on …".
  The failed turn may have done part of its work, so **Try again** tells it
  to check before repeating anything.
- **Try again later** closes the offer and sends nothing.

Only the latest card offers another account. With no other account to
offer, the card is as it always was: a usage limit offers **Try again**, and
a key out of credit offers nothing.

## Models

On a provider's page, **Models in the picker** chooses **All** or
**Chosen**, each model switched on or off and saved at once, with **All on**
and **All off** for the models shown. It shows once the provider has a key,
sign-in or server.

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
subscription's, or nothing. An OpenRouter sign-in is not counted either,
though OpenRouter bills it per use.

## Teammates that bring their own login

Teammates that run Codex, Cursor, opencode, Gemini CLI, Grok Build or Muse
Code sign in through that tool, not through Hotline, and Hotline holds no key
for them; so does Claude Code on This computer. See
[Teammates and drivers](/docs/setup/teammates/).
