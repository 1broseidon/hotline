# email-sentry

A headless inbox gatekeeper. It checks your Gmail account(s), asks an LLM judge
"should this notify you right now?", and buzzes your hotline channel only for
mail that matters: a human asking you something, money moving, real deadlines,
security events. Notified threads get a Gmail label so they are never re-judged.

**Default is dry-run.** Nothing is sent and nothing in Gmail changes unless you
pass `--live`.

## Files in this directory

| File | What it is |
|------|------------|
| `run_sentry.py` | the engine: search, fetch, judge, notify, label |
| `sentry-judge.md` | the judge prompt (importance classifier, the I/O contract) |
| `sentry-config.json` | accounts, addresses, VIPs, quiet hours, model, knobs |
| `sentry-log.jsonl` | append-only verdict log (one line per email per run; for calibration) |
| `debug/` | only created if the judge returns unparseable output (raw dump, no secrets) |

The engine configuration and calibration logs live here. The outer polling
state and run log are owned by `hotline loop`.

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

## Running by hand

```sh
./run_sentry.py                                  # dry-run, all accounts
./run_sentry.py --verbose                        # with the judge's reasoning
./run_sentry.py --query 'in:inbox newer_than:2d' # test on already-read mail
./run_sentry.py --account you@example.com --max 10
./run_sentry.py --live                           # actually notify + label
```

`--live` is the only flag with side effects. `--dry-run` wins over `--live`.

## Going live with hotline loop

1. Sanity-check a few dry runs, then a `--live` run by hand.
2. Register the managed loop:
   ```sh
   hotline loop add email-sentry --every 60s --timeout 25m \
     --cmd "python3 <SENTRY_DIR>/run_sentry.py --live"
   ```
   Replace `<SENTRY_DIR>` with this directory's absolute path.
3. Run `hotline up` if it is not already running. The supervisor will run the
   loop eagerly on startup and then every 60 seconds.

The loop runner skips a tick if the previous pass is still running, kills the
whole process group after the timeout, exports `HOTLINE_LOOP_STATE_DIR` for any
future durable watcher state, and writes the run log to
`<state>/loops/email-sentry.log`.

## Operating

```sh
hotline loop list
hotline loop logs email-sentry -n 100
hotline loop pause email-sentry
hotline loop resume email-sentry
hotline loop run email-sentry --once
```

### Kill switch

```sh
hotline loop pause email-sentry
```

Remove it entirely with `hotline loop remove email-sentry`.

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
