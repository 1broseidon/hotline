---
title: Agent accounts
description: Give each Claude Code teammate its own Claude login, so teammates on different accounts work at the same time.
---

An agent account is one sign-in to Claude Code, kept in a folder of its own
on this computer. A Claude Code teammate without one uses **System default**:
Claude Code's own sign-in on this computer, as before. Give two teammates two
accounts and each works on its own Claude subscription, at the same time.

Hotline never sees a password or token. Claude Code signs itself in, into the
account's folder, and Hotline only asks Claude Code whether it is signed in.

## Add an account

There are three places to add one:

- **Settings → Agent accounts**: choose **Claude Code**, then **Add account**.
- **New teammate → Thinks with**: pick Claude Code as the **Agent**, then
  **Add account** under **Account**.
- The teammate's pane: **Add account** under **Claude Code account**.

Give it a name, or leave it blank for the next **Account N**, and, if you
like, the email of the Claude account. The email only fills in Claude Code's
sign-in page, so the right account is used; Hotline does not keep it.
**Add and sign in** makes the account and starts its sign-in. One added in
**New teammate** is picked for the new teammate at once; one added in the
pane is given to the teammate once it is signed in.

### What a new account starts with

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
replace the login), which the copy leaves out and lists. A link in your
setup is skipped, not followed.

## Sign in

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

## Assign and switch

In the teammate's pane, **Claude Code account** lists **System default** and
every account, with **In use** beside the one running. Pick another and the
teammate moves to it when its work ends: no turn in its conversation, and no
side thread, job or subagent mid-turn. Until then the pane says **Switches to
… when its work ends**, and offers:

- **Keep …**: stay on the account in use, and change nothing.
- **Stop and switch now**: stop all of the teammate's work and start it on
  the new account.

An account that is not signed in, or still signing in, waits: the teammate
keeps its current account until it is.

A switch starts a new Claude Code session on the new account, handed this
conversation (**Fresh**). Another account's session is never reopened, so
going back to the first account later starts fresh too.

## Usage limits

When Claude Code stops at an account's usage limit, the card says so in
Claude Code's own words, with when the limit resets. It offers **Continue on
…** for each other account that is signed in, and System default, and
**Try again**. Continue on moves the teammate at once and starts fresh; the
failed turn may have done part of its work, so **Try again** tells it to check
before repeating anything.

## Setup

An account's page under **Settings → Agent accounts**, and the **…'s setup**
link in the teammate's pane, show the account's setup: **Instructions**,
**Settings**, **Skills**, **Plugins** and **MCP servers**, with counts.

- **Open**, on the desk itself, opens the file in your editor or the folder
  in your file manager. A setup item that is missing is made first, directly
  in the account's folder: an empty `CLAUDE.md`, a `{}` `settings.json`, or
  the folder; nothing that runs.
- Hotline does not read or check what you write there. It applies from the
  next session of the teammates on that account, and every teammate on the
  account shares it.
- **Copy again from System default** brings over what you changed in your
  own setup since the account was made. The ticked items replace the
  account's, including your changes to them; **Settings** is unticked unless
  you tick it. The sign-in is not touched.

## The folder

Each account is a folder in Hotline's data directory,
`agent-accounts/<id>`, private to your user. Claude Code keeps the
account's sign-in, history and settings there. The account's page shows it,
with **Show folder**.

**Remove account** is refused while a teammate uses the account; give the
teammate another account first, since Hotline never moves a teammate for you.
Removing an unused account makes Hotline forget it, and leaves the folder
and the sign-in in it. To sign it out, run Claude Code's own sign-out with
`CLAUDE_CONFIG_DIR` set to the folder:

```sh
CLAUDE_CONFIG_DIR="<folder>" claude auth logout
```

In PowerShell, `$env:CLAUDE_CONFIG_DIR = "<folder>"; claude auth logout`.

## What an account does not cover

- Claude Code also reads settings in the teammate's folder
  (`.claude/settings.json` and `.claude/settings.local.json`). One that sets
  `env` or `apiKeyHelper` can replace the account's sign-in, as it can for
  System default today, and Hotline does not see it.
- Settings your organization manages on this computer still apply.
- The account's folder is not hidden from Claude Code itself or from a
  teammate with **Whole machine**: any program running as your user can read
  it.
- Anthropic's plans and terms decide whether running several subscriptions
  this way is allowed, so check them.
