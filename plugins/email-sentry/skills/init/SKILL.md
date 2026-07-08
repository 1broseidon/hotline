---
name: init
description: Scaffold an email-sentry install (Gmail triage engine, LLM judge, hotline notifications, and a hotline loop registration) into the current project or a chosen directory. Use when the user wants important-mail notifications on their hotline channel.
---

# email-sentry: init

Scaffold a working email-sentry install from the canonical template files
bundled with this plugin at `${CLAUDE_PLUGIN_ROOT}/skills/init/templates/`.

email-sentry is a headless inbox gatekeeper: a loop-managed engine searches
Gmail via the `gog` CLI, an LLM judge decides "should this notify the user
right now?", and only mail that matters buzzes the user's hotline channel.

## Step 1: Check prerequisites

Check each of these and report anything missing before scaffolding:

1. **gog on PATH and authenticated.** `which gog`, then `gog auth list` (add
   `-j` for JSON). At least one Gmail account must be authenticated. Note every
   account listed; you will offer them all in Step 3.
2. **hotline state exists.** `${XDG_CONFIG_HOME:-~/.config}/hotline/.env` must
   exist and contain `TELEGRAM_BOT_TOKEN`, and
   `${XDG_CONFIG_HOME:-~/.config}/hotline/access.json` should have at least one
   entry in `allowFrom` (that chat is where notifications go). Read only; never
   print token values. If missing, point the user at `hotline setup` and
   `hotline pair` first.
3. **claude CLI on PATH** (`which claude`). The judge runs through it.
4. **python3 on PATH.** Record the absolute path of `python3`; use it in the
   `hotline loop add` command you give the user.

If a prerequisite is missing, stop and tell the user how to fix it. Do not
scaffold a broken install. Never register the loop or run `hotline up` yourself
unless the user explicitly confirms they want it on.

## Step 2: Choose the target directory and check for conflicts

Ask the user where the install should live. Default suggestion: an
`email-sentry/` directory inside the current project. Engine config, prompt, and
calibration logs live in that directory; outer loop state and run logs live in
hotline's state dir.

Never overwrite. If any target file already exists, do NOT write it; collect
every conflict, report the list, and only create what is missing.

Targets (relative to the chosen directory):

- `run_sentry.py`
- `sentry-judge.md`
- `sentry-config.json`
- `README.md`

## Step 3: Scaffold and fill in the config

Copy each non-conflicting file from
`${CLAUDE_PLUGIN_ROOT}/skills/init/templates/` into the target directory. Copy
`run_sentry.py`, `sentry-judge.md`, and `README.md` as-is; do not edit or
reformat them. Make `run_sentry.py` executable.

If `${CLAUDE_PLUGIN_ROOT}/skills/init/templates/` does not exist, stop and tell
the user the plugin install looks broken; do not improvise file contents.

Then fill in `sentry-config.json` (this is the one file you edit):

- `accounts`: one entry per gog-authenticated account the user wants watched
  (ask which, and whether each is `personal` or `work`).
- `user_name`: ask for the user's first name.
- `user_addresses`: the account addresses, plus any aliases or group addresses
  the user says land in these inboxes.
- `timezone`: from `timedatectl` (or ask), as an IANA name.
- Leave `notify.chat_id` as `null` (auto: first allowlisted hotline chat)
  unless the user wants a specific chat pinned.
- Mention that `vip_senders` and `quiet_hours` are theirs to tune later.

The judge prompt uses `{{USER_NAME}}`, `{{USER_ADDRESSES}}`, and
`{{PRIMARY_ADDRESS}}` placeholders; the engine fills those at runtime from this
config. Do not substitute them in `sentry-judge.md` yourself.

## Step 4: Verify with a dry run, then give the loop command

1. Run a dry run yourself and show the summary:
   `cd <target dir> && ./run_sentry.py --max 5`
   Dry-run sends nothing and changes nothing in Gmail; it is safe. If gog,
   the judge, or the config fails here, fix it before going further.
2. Tell the user how to sanity-check one live pass:
   `cd <target dir> && ./run_sentry.py --live`
3. Give the user this loop registration command, with the absolute python path
   and target directory filled in:
   ```sh
   hotline loop add email-sentry --every 60s --timeout 25m \
     --cmd "{{PYTHON_BIN}} {{SENTRY_DIR}}/run_sentry.py --live"
   ```
4. Tell them `hotline up` must be running for managed loops to tick. If they do
   not use `hotline up`, they can drive the registered loop externally with
   `hotline loop run email-sentry --once`.
5. Remind them of the kill switch:
   `hotline loop pause email-sentry`. The full operating guide is in the
   scaffolded `README.md`.
