# email-sentry

A headless inbox gatekeeper. It watches your Gmail account(s), asks an LLM
judge "should this notify you right now?", and buzzes your hotline channel only
for mail that matters: a human asking you something, money moving, real
deadlines, security events. Notified threads get a Gmail label so they are
never re-judged.

**Default is dry-run.** Nothing is sent and nothing in Gmail changes unless you
pass `--live`.

## Files in this directory

| File | What it is |
|------|------------|
| `run_sentry.py` | the engine: search, fetch, judge, notify, label |
| `watch_sentry.py` | the trigger: polls the Gmail History API and runs the engine on change |
| `sentry-judge.md` | the judge prompt (importance classifier, the I/O contract) |
| `sentry-config.json` | accounts, addresses, VIPs, quiet hours, model, knobs |
| `email-sentry.service` | systemd user unit for the watcher |
| `sentry-log.jsonl` | append-only verdict log (one line per email per run; for calibration) |
| `watcher.log`, `cron.log` | runtime logs |
| `debug/` | only created if the judge returns unparseable output (raw dump, no secrets) |

All state lives here, in this directory.

## How a run works

For each account in the config:

1. **Search** with `gog gmail search 'in:inbox is:unread newer_than:2d -label:Sentry/Triaged'`
   for candidate threads (cheap: sender/subject/date/labels only).
2. **Fetch** the latest message of each candidate with `gog gmail get`.
   Bodies are HTML-stripped and truncated.
3. **Judge** by substituting the real clock and your context into
   `sentry-judge.md` and running the `claude` CLI headless, with no tools and
   no MCP, from a neutral temp dir. Email bodies are fenced as untrusted data.
4. **Log** every verdict to `sentry-log.jsonl` (no bodies, no secrets).
5. **Act**: dry-run prints what it would do; `--live` sends one message per
   `notify:true` verdict through your hotline bot, then labels the thread.

The judge defaults to silence: a wrong buzz costs more than a missed one. Most
runs notify nothing. That is expected.

## Where notifications go

The engine reads the bot token from hotline's state env file
(`${XDG_CONFIG_HOME:-~/.config}/hotline/.env`, key `TELEGRAM_BOT_TOKEN`) and
sends to `notify.chat_id` from the config. If `chat_id` is null it uses the
first allowlisted chat in hotline's `access.json`, i.e. you, the person who
paired with the bot. No secrets are stored in this directory.

## Running by hand

```sh
./run_sentry.py                                  # dry-run, all accounts
./run_sentry.py --verbose                        # with the judge's reasoning
./run_sentry.py --query 'in:inbox newer_than:2d' # test on already-read mail
./run_sentry.py --account you@example.com --max 10
./run_sentry.py --live                           # actually notify + label
```

`--live` is the only flag with side effects. `--dry-run` wins over `--live`.

## Going live

1. Sanity-check a few dry runs, then a `--live` run by hand.
2. Baseline the watcher to NOW so no backlog is judged:
   ```sh
   ./watch_sentry.py --init
   ```
3. Install and start the watcher (the unit file is pre-filled by the init skill):
   ```sh
   cp email-sentry.service ~/.config/systemd/user/
   systemctl --user daemon-reload
   systemctl --user enable --now email-sentry
   ```
4. Add the hourly cron backstop (`:07` past the hour, same flock lock, so it
   can never overlap the watcher):
   ```cron
   7 * * * * flock -n -E 99 <SENTRY_DIR>/.sentry.lock python3 <SENTRY_DIR>/run_sentry.py --live >> <SENTRY_DIR>/cron.log 2>&1
   ```
   Replace `<SENTRY_DIR>` with this directory's absolute path. If `flock`,
   `python3`, or `claude` are not on cron's default PATH, use absolute paths or
   add a `PATH=` header line to the crontab.

## Operating

```sh
# health
systemctl --user is-active email-sentry
systemctl --user status email-sentry --no-pager
tail -f watcher.log
crontab -l | grep run_sentry

# restart after editing watch_sentry.py or the unit
systemctl --user restart email-sentry

# re-baseline to NOW (e.g. after a long downtime, to skip backlog)
systemctl --user stop email-sentry
./watch_sentry.py --init
systemctl --user start email-sentry
```

### Kill switch

```sh
# 1. stop and disable the always-on watcher
systemctl --user disable --now email-sentry

# 2. remove the hourly cron line (keeps any other crontab entries)
crontab -l | grep -v 'run_sentry.py' | crontab -
```

To pause temporarily without disabling: `systemctl --user stop email-sentry`
and comment out the cron line.

## Calibration

`sentry-log.jsonl` accumulates every verdict. To review what the judge has
been deciding:

```sh
jq -r '[.verdict.notify, .verdict.importance, .account, .email.subject] | @tsv' sentry-log.jsonl
```

Tune `vip_senders` and `quiet_hours` in the config and the rungs in
`sentry-judge.md`, then re-run dry-run and compare.

## Safety notes

- Dry-run is the default; only `--live` sends or writes.
- The judge runs with no tools and no MCP, from a neutral dir. It can only
  classify, and email bodies are fenced as untrusted data in the prompt.
- Secrets (gog keyring password, bot token) stay in process env and memory and
  are never printed or written to logs.
- gog usage is read plus label only; the engine never sends, trashes, or
  deletes mail.
