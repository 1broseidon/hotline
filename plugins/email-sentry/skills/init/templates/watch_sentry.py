#!/usr/bin/env python3
"""
watch_sentry.py - always-on trigger layer for the email-sentry engine.

It does NOT judge mail itself. Every ~60s it asks Gmail (per account) "did
anything change since I last looked?" via the Gmail History API. On a detected
change it invokes the engine `run_sentry.py --live`, which does the real work
(judge -> notify -> label). The engine self-filters to
`in:inbox is:unread newer_than:2d -label:<dedup_label>`, so an extra/no-op
trigger is cheap and harmless; the dedup label prevents double notifications.

Detection:
  - Primary: `gog -j gmail history --since <baseline> --max 100`. The response
    envelope always carries the current `historyId` (advance baseline to it) and
    a `messages` list of changed message ids. Non-empty list => something changed
    => trigger the engine. (gog's history output is id-only, so we can't tell new
    mail from a label change - we just trigger; the engine filters.)
  - Fallback (history errors / baseline expired): cheap newest-message delta via
    `gog -j gmail search 'in:inbox' --max 1`; a changed top result => trigger.
    We also try to re-establish a history baseline so we return to history mode
    next cycle.

Baseline = NOW: on `--init` (and on first loop if no state) each account's
baseline historyId is set to the current mailbox historyId, so only mail that
arrives AFTER go-live is ever seen. Pre-existing unread backlog can be excluded
at go-live by labeling it with the dedup label (see the scaffold README) - the
engine never judges it.

Concurrency: the engine is launched under `flock -n -E 99 <lock>`, the SAME lock
the hourly cron backstop uses, so a watcher trigger never overlaps a cron run
(and vice-versa). If the lock is busy (rc 99) the watcher skips this trigger and
retries next cycle (the baseline is NOT advanced, so the change is re-detected).

Resilience: every cycle is wrapped; transient gog/network errors are logged and
the loop continues. systemd `Restart=always` is the outer safety net.

Usage:
  ./watch_sentry.py --init     # baseline all accounts to NOW, write state, exit
  ./watch_sentry.py            # run the watcher loop (default 60s interval)
  ./watch_sentry.py --once     # run a single cycle then exit (testing)
  ./watch_sentry.py --interval 90
"""

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

# ----------------------------- configuration -----------------------------
# All state lives next to this script, in the scaffold directory.
HERE = Path(__file__).resolve().parent
DEFAULT_CONFIG_PATH = HERE / "sentry-config.json"
STATE_PATH = HERE / "watcher-state.json"
LOCK_PATH = HERE / ".sentry.lock"
WATCH_LOG = HERE / "watcher.log"
RUN_ENGINE = HERE / "run_sentry.py"

LOCK_BUSY_RC = 99           # flock -E value: lock held (cron running) -> skip
RUN_TIMEOUT_SEC = 1500      # generous: engine may call the LLM judge per account


# ----------------------------- small utils -----------------------------

def expand(path):
    return os.path.expanduser(os.path.expandvars(path))


def log(msg):
    line = f"{dt.datetime.now().astimezone().strftime('%Y-%m-%d %H:%M:%S %z')}  {msg}"
    print(line, flush=True)
    try:
        with open(WATCH_LOG, "a") as f:
            f.write(line + "\n")
    except Exception:
        pass


def parse_env_file(path):
    """Parse `export KEY=VALUE` / `KEY=VALUE` lines into a dict (tolerates quotes)."""
    out = {}
    p = expand(path)
    if not os.path.exists(p):
        return out
    with open(p, "r") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("export "):
                line = line[len("export "):]
            if "=" not in line:
                continue
            k, v = line.split("=", 1)
            k, v = k.strip(), v.strip()
            if len(v) >= 2 and ((v[0] == '"' and v[-1] == '"') or (v[0] == "'" and v[-1] == "'")):
                v = v[1:-1]
            out[k] = v
    return out


def load_config(path):
    with open(path, "r") as f:
        return json.load(f)


def read_state():
    if STATE_PATH.exists():
        try:
            with open(STATE_PATH) as f:
                return json.load(f)
        except Exception as e:
            log(f"WARN: could not read state ({e}); treating as empty")
    return {"accounts": {}}


def write_state(state):
    tmp = STATE_PATH.with_suffix(".json.tmp")
    with open(tmp, "w") as f:
        json.dump(state, f, indent=2)
    os.replace(tmp, STATE_PATH)


# ----------------------------- gog plumbing -----------------------------

class Gog:
    def __init__(self, cfg):
        self.bin = cfg.get("gog", {}).get("bin", "gog")
        env_file = cfg.get("gog", {}).get("env_file", "~/.config/gogcli/env")
        # Merge the file keyring backend+password into the process env once, so
        # both our own gog calls and the spawned engine inherit it (no headless hang).
        merged = parse_env_file(env_file)
        for k, v in merged.items():
            os.environ.setdefault(k, v)
        self.env = dict(os.environ)
        if "GOG_KEYRING_BACKEND" not in self.env:
            log("NOTE: gog env has no GOG_KEYRING_BACKEND; on a headless box gog may hang waiting for a keyring.")

    def _run(self, args, account, timeout=60):
        cmd = [self.bin, "-j"] + args + ["-a", account, "--no-input"]
        try:
            p = subprocess.run(cmd, env=self.env, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, "timeout"
        if p.returncode != 0:
            return None, (p.stderr or "").strip()[:200]
        return p.stdout, None

    def history(self, account, since):
        out, err = self._run(["gmail", "history", "--since", str(since), "--max", "100"], account)
        if err:
            return None, err
        try:
            return json.loads(out), None
        except Exception as e:
            return None, f"json:{e}"

    def newest_marker(self, account):
        """(marker, thread_id) for the newest inbox thread, or (None, None)."""
        out, err = self._run(["gmail", "search", "in:inbox", "--max", "1"], account)
        if err:
            return None, None
        try:
            threads = (json.loads(out).get("threads") or [])
        except Exception:
            return None, None
        if not threads:
            return "", None
        t = threads[0]
        return f"{t.get('id','')}:{t.get('date','')}", t.get("id")

    def message_history_id(self, account, thread_id):
        out, err = self._run(["gmail", "get", thread_id, "--format", "metadata"], account)
        if err:
            return None
        try:
            return json.loads(out).get("message", {}).get("historyId")
        except Exception:
            return None

    def current_history_id(self, account):
        """Best-effort current mailbox historyId (the 'now' baseline)."""
        marker, tid = self.newest_marker(account)
        if not tid:
            return None
        mid = self.message_history_id(account, tid)
        if not mid:
            return None
        # The history envelope returns the true current historyId even for a
        # slightly-stale `since`; prefer it, fall back to the message's id.
        data, err = self.history(account, mid)
        if data and data.get("historyId"):
            return str(data["historyId"])
        return str(mid)


# ----------------------------- baseline / init -----------------------------

def init_baseline(cfg, gog, state):
    """Set every account's baseline historyId to NOW. No triggering."""
    accts = state.setdefault("accounts", {})
    for a in cfg["accounts"]:
        email = a["email"]
        hid = gog.current_history_id(email)
        marker, _ = gog.newest_marker(email)
        accts[email] = {"history_id": hid, "fallback_marker": marker}
        log(f"baseline {email}: history_id={hid} fallback_marker={marker!r}")
    state["initialized_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
    write_state(state)
    log("baseline initialized to NOW (no backlog will be judged).")


# ----------------------------- detection -----------------------------

def check_account(gog, email, st):
    """Return (changed: bool, new_history_id, new_fallback_marker, note)."""
    hid = st.get("history_id")
    # Primary: Gmail History API.
    if hid:
        data, err = gog.history(email, hid)
        if not err and data is not None:
            cur = str(data.get("historyId") or hid)
            msgs = data.get("messages") or []
            return (len(msgs) > 0, cur, st.get("fallback_marker"), f"history Δ={len(msgs)}")
        # history failed -> fall through to fallback, try to re-baseline history.
        note_err = err
    else:
        note_err = "no-baseline"

    # Fallback: newest-message delta.
    marker, _ = gog.newest_marker(email)
    new_hid = gog.current_history_id(email) or hid
    prev = st.get("fallback_marker")
    if prev is None:
        return (False, new_hid, marker, f"fallback baseline ({note_err})")
    changed = (marker is not None and marker != prev)
    return (changed, new_hid, marker, f"fallback Δ={changed} ({note_err})")


# ----------------------------- engine launch -----------------------------

def run_engine(env):
    """flock -n -E 99 <lock> python run_sentry.py --live. Returns rc (99=busy)."""
    flock_bin = shutil.which("flock") or "flock"
    py = sys.executable or shutil.which("python3") or "python3"
    cmd = [flock_bin, "-n", "-E", str(LOCK_BUSY_RC), str(LOCK_PATH),
           py, str(RUN_ENGINE), "--live"]
    try:
        p = subprocess.run(cmd, cwd=str(HERE), env=env,
                           capture_output=True, text=True, timeout=RUN_TIMEOUT_SEC)
    except subprocess.TimeoutExpired:
        log("ENGINE: timed out")
        return 1
    if p.returncode == LOCK_BUSY_RC:
        return LOCK_BUSY_RC
    # Capture a compact tail of the engine's own report for the log.
    tail = "\n".join((p.stdout or "").strip().splitlines()[-6:])
    log(f"ENGINE rc={p.returncode}\n{tail}")
    if p.returncode != 0 and (p.stderr or "").strip():
        log("ENGINE stderr: " + (p.stderr or "").strip()[-400:])
    return p.returncode


# ----------------------------- main loop -----------------------------

def cycle(cfg, gog, env):
    state = read_state()
    accts = state.setdefault("accounts", {})
    results = {}
    any_changed = False
    for a in cfg["accounts"]:
        email = a["email"]
        st = accts.setdefault(email, {"history_id": None, "fallback_marker": None})
        try:
            changed, new_hid, new_marker, note = check_account(gog, email, st)
        except Exception as e:
            log(f"WARN check {email}: {type(e).__name__}: {e}")
            continue
        results[email] = (changed, new_hid, new_marker)
        if changed:
            any_changed = True
            log(f"CHANGE {email}: {note}")

    if any_changed:
        rc = run_engine(env)
        if rc == LOCK_BUSY_RC:
            log("trigger skipped: lock busy (cron running) - baselines NOT advanced, will retry")
            return  # do not persist advanced baselines; re-detect next cycle
    # Advance baselines (no change, or engine ran).
    for email, (changed, new_hid, new_marker) in results.items():
        st = accts[email]
        if new_hid:
            st["history_id"] = str(new_hid)
        if new_marker is not None:
            st["fallback_marker"] = new_marker
    write_state(state)


def main():
    ap = argparse.ArgumentParser(description="email-sentry watcher (trigger layer).")
    ap.add_argument("--config", default=str(DEFAULT_CONFIG_PATH))
    ap.add_argument("--init", action="store_true", help="Baseline all accounts to NOW, then exit.")
    ap.add_argument("--once", action="store_true", help="Run a single detection cycle, then exit.")
    ap.add_argument("--interval", type=int, default=60, help="Seconds between cycles (default 60).")
    args = ap.parse_args()

    cfg = load_config(args.config)
    gog = Gog(cfg)              # also injects gog keyring env into os.environ
    env = dict(os.environ)

    if args.init:
        state = read_state()
        init_baseline(cfg, gog, state)
        return

    state = read_state()
    if not state.get("accounts") or not any(
        v.get("history_id") for v in state.get("accounts", {}).values()
    ):
        log("no baseline state found - initializing to NOW before looping")
        init_baseline(cfg, gog, state)

    if args.once:
        cycle(cfg, gog, env)
        return

    log(f"watcher up - interval {args.interval}s, accounts: "
        + ", ".join(a["email"] for a in cfg["accounts"]))
    while True:
        try:
            cycle(cfg, gog, env)
        except Exception as e:
            log(f"WARN cycle error: {type(e).__name__}: {e}")
        time.sleep(max(15, args.interval))


if __name__ == "__main__":
    main()
