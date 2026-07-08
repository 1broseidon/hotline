# Loop Templates

These are command forms for common long-running checks. They are examples only:
do not install them over an existing crontab, systemd unit, or running watcher
without an operator choosing that migration.

## Reddit Sub-Sweep

Use `--notify-llm` when the script prints only new hits and hotline should ask
the agent whether the batch is worth surfacing. The script should keep dedup
state under `$HOTLINE_LOOP_STATE_DIR`.

```sh
hotline source add reddit-watch --cap low
hotline loop add reddit-watch --every 6h --notify-llm --source reddit-watch --level low \
  --cmd "python3 /home/george/Projects/agents/beastie-boy/threads/ketch/scripts/sub-sweep.py"
```

## Email Sentry

Use the script-owned path when the engine already judges and notifies. hotline
replaces only the outer sleep/watch/lock layer.

```sh
hotline loop add email-sentry --every 60s --timeout 25m \
  --cmd "python3 /path/to/email-sentry/run_sentry.py --live"
```

The loop runner exports `HOTLINE_LOOP_STATE_DIR` for any watcher marker files,
logs each run to `<state>/loops/email-sentry.log`, and skips a tick if the
previous sentry pass is still running.
