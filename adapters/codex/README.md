# hotline job cards — Codex adapter (DESIGN STUB)

> Not built yet. This sketches the event source and confirms the job-file
> contract is a fit. The box side (the `hotline job` CLI + dispatcher) already
> ships — an adapter only needs to call it.

## Event source

Codex has no in-process subagent event bus to hook the way Claude Code (hooks)
and Pi (`pi.events`) do; its unit of delegated work is a `codex exec`
invocation. The adapter is therefore a thin **wrapper** around that command
rather than a subscriber:

```sh
# adapters/codex/hotline-codex-exec.sh  (sketch)
cookie=$(uuidgen)
batch="${HOTLINE_JOB_BATCH:-$cookie}"   # caller sets a shared batch to roll up a fan-out
hotline job start --cookie "$cookie" --batch "$batch" --title "$*"
if codex exec "$@"; then state=ok; else state=err; fi
hotline job done  --cookie "$cookie" --batch "$batch" --state "$state"
```

| Wrapper phase | Job intent |
|---------------|-----------|
| before `codex exec` | `hotline job start --cookie <uuid> --batch <HOTLINE_JOB_BATCH> --title <task>` |
| after, exit 0       | `hotline job done  --cookie <uuid> --state ok` |
| after, non-zero     | `hotline job done  --cookie <uuid> --state err` |

**cookie** = a uuid minted per invocation. **batch** = an env var the dispatching
script sets so several `codex exec` calls from one orchestration roll up into one
card; unset means the codex run cards on its own.

## Contract

Same job-file contract: one `hotline job` call per lifecycle transition, keyed on
a stable `--cookie`, best-effort. Because the wrapper owns the whole lifespan of
the `codex exec` process, start/done bracket it exactly — the only gap is a
`kill -9` of the wrapper itself, which the box's lease reaper closes.

Note the [Codex background stdin stall](../../notes) gotcha when spawning
`codex exec` detached: redirect stdin from `/dev/null` or it stalls at startup.

## To build

1. Add `hotline-codex-exec.sh` wrapping `codex exec` per the sketch above.
2. Point your orchestration at the wrapper; set `HOTLINE_JOB_BATCH` to group a
   fan-out.
3. Verify `hotline job list` and the app card rollup.
