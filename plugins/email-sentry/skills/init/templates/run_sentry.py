#!/usr/bin/env python3
"""
run_sentry.py - the email-sentry engine (one triage pass).

Flow per account:
  1. gog search for candidate threads (unread, recent, not yet triaged)
  2. fetch the LATEST message of each candidate (flattened body/headers/unsub)
  3. build the judge prompt from sentry-judge.md (real clock injected)
  4. call `claude` headless as the importance judge, parse the JSON verdicts
  5. append every verdict to sentry-log.jsonl (no secrets, no bodies)
  6. DRY-RUN (default): print what it WOULD notify/skip. Nothing is sent or changed.
     LIVE (--live): send one notification per notify, then label the thread.

Safety: dry-run is the default. Notifications + Gmail labels are touched ONLY
with --live. Secrets (gog keyring password, bot token) live in subprocess env /
memory only and are never printed or logged.

Usage:
  ./run_sentry.py                      # dry-run, all accounts, production query
  ./run_sentry.py --query 'in:inbox newer_than:2d'   # override query (e.g. test on read mail)
  ./run_sentry.py --account you@example.com          # limit to one account
  ./run_sentry.py --max 10             # cap candidates per account
  ./run_sentry.py --live               # ACTUALLY notify + label (opt-in)
"""

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
import urllib.error
from pathlib import Path

try:
    from zoneinfo import ZoneInfo
except Exception:  # pragma: no cover
    ZoneInfo = None

# ----------------------------- configuration -----------------------------
# Config, judge prompt, and verdict log live next to this script, in the
# directory /email-sentry:init scaffolded. The outer polling state/log belongs
# to `hotline loop`.
HERE = Path(__file__).resolve().parent
JUDGE_PROMPT_PATH = HERE / "sentry-judge.md"
DEFAULT_CONFIG_PATH = HERE / "sentry-config.json"

# Notifications ride the user's own hotline channel: the bot token comes from
# hotline's state .env, and the recipient defaults to the first allowlisted
# chat in hotline's access.json. Both can be overridden in sentry-config.json
# under "notify".
HOTLINE_DIR = Path(os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))) / "hotline"
HOTLINE_ENV_FILE = HOTLINE_DIR / ".env"
HOTLINE_ACCESS_FILE = HOTLINE_DIR / "access.json"
DEFAULT_TOKEN_KEY = "TELEGRAM_BOT_TOKEN"


# ----------------------------- helpers -----------------------------

def log_err(msg):
    print(f"[sentry] {msg}", file=sys.stderr)


def expand(path):
    return os.path.expanduser(os.path.expandvars(str(path)))


def load_config(path):
    with open(path, "r") as f:
        return json.load(f)


def parse_env_file(path):
    """Parse `export KEY=VALUE` / `KEY=VALUE` lines into a dict. Tolerates quotes."""
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
            k = k.strip()
            v = v.strip()
            if len(v) >= 2 and ((v[0] == '"' and v[-1] == '"') or (v[0] == "'" and v[-1] == "'")):
                v = v[1:-1]
            out[k] = v
    return out


def tzinfo_for(name):
    if ZoneInfo is not None:
        try:
            return ZoneInfo(name)
        except Exception:
            pass
    return dt.timezone.utc


def fmt_local(epoch_ms, tz):
    """Epoch milliseconds -> 'YYYY-MM-DD HH:MM ±HH:MM' in tz."""
    try:
        t = dt.datetime.fromtimestamp(int(epoch_ms) / 1000.0, tz)
        return t.strftime("%Y-%m-%d %H:%M %z")[:22]
    except Exception:
        return ""


def now_local_str(tz):
    return dt.datetime.now(tz).strftime("%Y-%m-%d %H:%M %z")


_TAG_RE = re.compile(r"<[^>]+>")
_STYLE_RE = re.compile(r"<(script|style)[^>]*>.*?</\1>", re.IGNORECASE | re.DOTALL)
_WS_RE = re.compile(r"[ \t]+")
_MULTINL_RE = re.compile(r"\n{3,}")


def clean_body(body, max_chars):
    """Light HTML->text + whitespace squeeze + truncate. Keeps the judge prompt
    small and reduces injection surface noise. URLs left intact."""
    if not body:
        return ""
    s = body
    if "<" in s and ">" in s and ("</" in s or "/>" in s or "<br" in s.lower()):
        s = _STYLE_RE.sub(" ", s)
        s = re.sub(r"<br\s*/?>", "\n", s, flags=re.IGNORECASE)
        s = re.sub(r"</p>", "\n", s, flags=re.IGNORECASE)
        s = _TAG_RE.sub(" ", s)
    s = s.replace("\r\n", "\n").replace("\r", "\n")
    s = _WS_RE.sub(" ", s)
    s = _MULTINL_RE.sub("\n\n", s)
    s = s.strip()
    if len(s) > max_chars:
        s = s[:max_chars] + " …[truncated]"
    return s


# ----------------------------- gog plumbing -----------------------------

class Gog:
    def __init__(self, cfg):
        self.bin = cfg.get("gog", {}).get("bin", "gog")
        env_file = cfg.get("gog", {}).get("env_file", "~/.config/gogcli/env")
        self.env = dict(os.environ)
        self.env.update(parse_env_file(env_file))  # keyring backend + password
        if "GOG_KEYRING_BACKEND" not in self.env:
            log_err("NOTE: gog env file has no GOG_KEYRING_BACKEND; on a headless box gog may hang waiting for a keyring.")

    def _run(self, args, account, dry_run_flag=False, timeout=120):
        cmd = [self.bin, "-j"] + args + ["-a", account, "--no-input"]
        if dry_run_flag:
            cmd.append("-n")
        try:
            p = subprocess.run(cmd, env=self.env, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            log_err(f"gog timeout: {' '.join(args)} (-a {account})")
            return None, "timeout"
        if p.returncode != 0:
            return None, (p.stderr or "").strip()[:300]
        return p.stdout, None

    def search(self, query, account, max_results):
        out, err = self._run(["gmail", "search", query, "--max", str(max_results)], account)
        if err:
            log_err(f"search failed for {account}: {err}")
            return []
        try:
            data = json.loads(out)
        except Exception as e:
            log_err(f"search JSON parse failed for {account}: {e}")
            return []
        return data.get("threads", []) or []

    def get_message(self, msg_id, account):
        out, err = self._run(["gmail", "get", msg_id, "--format", "full"], account)
        if err:
            log_err(f"get message {msg_id} failed for {account}: {err}")
            return None
        try:
            return json.loads(out)
        except Exception as e:
            log_err(f"get message {msg_id} JSON parse failed: {e}")
            return None

    def latest_msg_id(self, thread_id, account):
        out, err = self._run(["gmail", "thread", "get", thread_id], account)
        if err:
            log_err(f"thread get {thread_id} failed for {account}: {err}")
            return None
        try:
            data = json.loads(out)
            msgs = data.get("thread", {}).get("messages", []) or []
            if not msgs:
                return None
            return msgs[-1].get("id")
        except Exception as e:
            log_err(f"thread get {thread_id} parse failed: {e}")
            return None

    def ensure_label(self, name, account):
        # idempotent-ish: ignore "already exists" errors
        out, err = self._run(["gmail", "labels", "create", name], account)
        if err and "exist" not in err.lower() and "already" not in err.lower():
            log_err(f"label create '{name}' for {account}: {err}")

    def label_thread(self, thread_id, label, account):
        out, err = self._run(["gmail", "thread", "modify", thread_id, "--add", label], account)
        if err:
            log_err(f"label thread {thread_id} for {account}: {err}")
            return False
        return True


# ----------------------------- candidate building -----------------------------

def build_email_obj(gog, thread, account, role, tz, body_max):
    """Fetch the latest message of a candidate thread and shape it for the judge."""
    thread_id = thread.get("id")
    msg_count = thread.get("messageCount", 1) or 1
    if msg_count > 1:
        msg_id = gog.latest_msg_id(thread_id, account) or thread_id
    else:
        msg_id = thread_id

    msg = gog.get_message(msg_id, account)
    if msg is None:
        # fall back to the cheap search fields so the candidate isn't silently dropped
        return {
            "id": thread_id,
            "thread_id": thread_id,
            "account": account,
            "account_role": role,
            "from": thread.get("from", ""),
            "to": "",
            "cc": "",
            "subject": thread.get("subject", ""),
            "date": thread.get("date", ""),
            "unsubscribe_present": False,
            "labels": thread.get("labels", []),
            "body": "",
            "_fetch_error": True,
        }

    headers = msg.get("headers", {}) or {}
    message = msg.get("message", {}) or {}
    unsub = msg.get("unsubscribe")
    body = clean_body(msg.get("body", ""), body_max)
    date_local = fmt_local(message.get("internalDate"), tz) or headers.get("date", "")
    return {
        "id": thread_id,                 # keep thread id as the judge id (1 verdict per thread)
        "thread_id": thread_id,
        "msg_id": msg_id,
        "account": account,
        "account_role": role,
        "from": headers.get("from", ""),
        "to": headers.get("to", ""),
        "cc": headers.get("cc", ""),
        "subject": headers.get("subject", thread.get("subject", "")),
        "date": date_local,
        "unsubscribe_present": bool(unsub),
        "labels": message.get("labelIds", thread.get("labels", [])) or [],
        "body": body,
    }


def judge_input_obj(e):
    """The subset the judge actually sees (matches sentry-judge.md's example shape)."""
    return {
        "id": e["id"],
        "account": e["account"],
        "account_role": e["account_role"],
        "from": e["from"],
        "to": e["to"],
        "cc": e["cc"],
        "subject": e["subject"],
        "date": e["date"],
        "unsubscribe_present": e["unsubscribe_present"],
        "labels": e["labels"],
        "body": e["body"],
    }


# ----------------------------- prompt + judge -----------------------------

def format_vips(vip):
    senders = vip.get("senders", []) or []
    domains = vip.get("domains", []) or []
    parts = []
    if senders:
        parts.append("senders: " + ", ".join(senders))
    if domains:
        parts.append("domains: " + ", ".join(domains))
    if not parts:
        return "(none configured yet)"
    return "; ".join(parts)


def build_prompt(template, cfg, emails, tz):
    accts = ", ".join(f"{a['email']} ({a['role']})" for a in cfg["accounts"])
    addresses = cfg.get("user_addresses") or [a["email"] for a in cfg["accounts"]]
    subs = {
        "{{USER_NAME}}": cfg.get("user_name", "the user"),
        "{{PRIMARY_ADDRESS}}": addresses[0] if addresses else "",
        "{{NOW_LOCAL}}": now_local_str(tz),
        "{{TIMEZONE}}": cfg["timezone"],
        "{{USER_ADDRESSES}}": ", ".join(addresses),
        "{{ACCOUNTS_AND_ROLES}}": accts,
        "{{VIP_SENDERS}}": format_vips(cfg.get("vip_senders", {})),
        "{{QUIET_HOURS}}": f"{cfg['quiet_hours']} {cfg['timezone']}",
        "{{EMAILS_JSON}}": json.dumps([judge_input_obj(e) for e in emails], ensure_ascii=False, indent=2),
    }
    out = template
    for k, v in subs.items():
        out = out.replace(k, v)
    return out


def extract_json_array(text):
    """Robustly pull a JSON array out of possibly-dirty model text."""
    if text is None:
        raise ValueError("empty judge output")
    t = text.strip()
    # strip surrounding code fences
    if t.startswith("```"):
        t = re.sub(r"^```[a-zA-Z0-9]*\s*", "", t)
        t = re.sub(r"\s*```$", "", t)
        t = t.strip()
    try:
        v = json.loads(t)
        if isinstance(v, list):
            return v
    except Exception:
        pass
    start = t.find("[")
    end = t.rfind("]")
    if start != -1 and end != -1 and end > start:
        return json.loads(t[start:end + 1])
    raise ValueError("no JSON array found in judge output")


def run_judge(cfg, prompt, debug_dir):
    claude = cfg.get("claude", {})
    cmd = [
        claude.get("bin", "claude"),
        "-p",
        "--output-format", "json",
        "--strict-mcp-config",   # no MCP servers -> no contamination, far cheaper
        "--tools", "",           # no tools loaded
    ]
    model = claude.get("model")
    if model:
        cmd += ["--model", model]
    timeout = claude.get("timeout_sec", 240)

    # Run from a NEUTRAL temp dir so no project/global CLAUDE.md steers the judge.
    workdir = tempfile.mkdtemp(prefix="sentry-judge-")
    try:
        p = subprocess.run(
            cmd, input=prompt, cwd=workdir, capture_output=True, text=True, timeout=timeout
        )
    except subprocess.TimeoutExpired:
        log_err("claude judge timed out")
        return None
    finally:
        try:
            os.rmdir(workdir)
        except OSError:
            pass

    if p.returncode != 0:
        log_err(f"claude judge exited {p.returncode}: {(p.stderr or '')[:300]}")
        return None
    try:
        outer = json.loads(p.stdout)
    except Exception as e:
        log_err(f"claude outer JSON parse failed: {e}")
        _dump_debug(debug_dir, "claude-raw-stdout.txt", p.stdout)
        return None
    if outer.get("is_error"):
        log_err(f"claude reported is_error: {outer.get('result','')[:200]}")
    result_text = outer.get("result", "")
    try:
        return extract_json_array(result_text)
    except Exception as e:
        log_err(f"could not extract verdict array: {e}")
        _dump_debug(debug_dir, "claude-result-dirty.txt", result_text)
        return None


def _dump_debug(debug_dir, name, content):
    try:
        Path(debug_dir).mkdir(parents=True, exist_ok=True)
        with open(Path(debug_dir) / name, "w") as f:
            f.write(content or "")
        log_err(f"wrote debug to {Path(debug_dir)/name}")
    except Exception:
        pass


def normalize_verdict(v, fallback_id):
    """Coerce a verdict dict into the canonical shape; tolerate minor model drift."""
    def as_list(x):
        if x is None:
            return []
        if isinstance(x, list):
            return x
        return [x]

    imp = v.get("importance", 1)
    try:
        imp = int(imp)
    except Exception:
        imp = {"low": 1, "medium": 3, "high": 5}.get(str(imp).lower(), 1)
    return {
        "id": v.get("id", fallback_id),
        "reasoning": v.get("reasoning", ""),
        "importance": imp,
        "category": v.get("category", "other"),
        "rule_fired": v.get("rule_fired", ""),
        "notify": bool(v.get("notify", False)),
        "summary": v.get("summary", "") or "",
        "codes": as_list(v.get("codes")),
        "links": as_list(v.get("links")),
        "deadline": v.get("deadline"),
        "amount": v.get("amount"),
    }


# ----------------------------- notify (LIVE only) -----------------------------

def resolve_chat_id(cfg):
    """Explicit notify.chat_id wins; otherwise the first allowlisted chat in
    hotline's access.json (the person who paired with the bot)."""
    cid = (cfg.get("notify", {}) or {}).get("chat_id")
    if cid:
        return str(cid)
    try:
        with open(HOTLINE_ACCESS_FILE) as f:
            allow = json.load(f).get("allowFrom") or []
        if allow:
            return str(allow[0])
    except Exception as e:
        log_err(f"could not read hotline access.json: {type(e).__name__}")
    return None


def notify_send(cfg, text):
    ncfg = cfg.get("notify", {}) or {}
    env_file = ncfg.get("token_env_file") or str(HOTLINE_ENV_FILE)
    token_key = ncfg.get("token_key", DEFAULT_TOKEN_KEY)
    env = parse_env_file(env_file)
    # real env wins, matching hotline's own precedence
    token = os.environ.get(token_key) or env.get(token_key, "")
    if not token:
        log_err(f"bot token not found ({token_key} in env or {env_file}); cannot send.")
        return False
    chat_id = resolve_chat_id(cfg)
    if not chat_id:
        log_err("no notify.chat_id configured and no allowlisted chat in hotline access.json; cannot send.")
        return False
    url = f"https://api.telegram.org/bot{token}/sendMessage"
    payload = json.dumps({"chat_id": chat_id, "text": text, "disable_web_page_preview": True}).encode()
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            ok = resp.status == 200
            if not ok:
                log_err(f"notify HTTP {resp.status}")
            return ok
    except urllib.error.HTTPError as e:
        log_err(f"notify HTTPError {e.code}")  # never log token/url body
        return False
    except Exception as e:
        log_err(f"notify send error: {type(e).__name__}")
        return False


def notify_text(e, v):
    lines = [f"📧 {e['subject']}  ({e['account']})"]
    if v["summary"]:
        lines.append(v["summary"])
    if v["codes"]:
        lines.append("Code: " + ", ".join(str(c) for c in v["codes"]))
    if v["deadline"]:
        lines.append("Deadline: " + str(v["deadline"]))
    if v["amount"]:
        lines.append("Amount: " + str(v["amount"]))
    if v["links"]:
        lines.append("Link: " + str(v["links"][0]))
    return "\n".join(lines)


# ----------------------------- logging + report -----------------------------

def append_log(log_path, run_id, account, e, v):
    rec = {
        "run_id": run_id,
        "ts": dt.datetime.now(dt.timezone.utc).isoformat(),
        "account": account,
        "email": {"id": e["id"], "from": e["from"], "subject": e["subject"], "date": e["date"]},
        "verdict": v,
    }
    with open(log_path, "a") as f:
        f.write(json.dumps(rec, ensure_ascii=False) + "\n")


def print_email_report(e, v):
    flag = "🔔 NOTIFY" if v["notify"] else "·  skip  "
    print(f"  {flag} | imp {v['importance']} | {v['category']:<14} | {v['rule_fired']}")
    print(f"          from: {e['from'][:70]}")
    print(f"          subj: {e['subject'][:70]}")
    if v["notify"] and v["summary"]:
        print(f"          summ: {v['summary'][:160]}")
    extras = []
    if v["codes"]:
        extras.append(f"codes={v['codes']}")
    if v["links"]:
        extras.append(f"link={v['links'][0]}")
    if v["deadline"]:
        extras.append(f"deadline={v['deadline']}")
    if v["amount"]:
        extras.append(f"amount={v['amount']}")
    if extras:
        print("          " + "  ".join(extras))


# ----------------------------- main -----------------------------

def main():
    ap = argparse.ArgumentParser(description="email-sentry engine (dry-run by default).")
    ap.add_argument("--config", default=str(DEFAULT_CONFIG_PATH))
    ap.add_argument("--live", action="store_true", help="ACTUALLY send notifications + label threads (opt-in).")
    ap.add_argument("--dry-run", action="store_true", help="Force dry-run (default).")
    ap.add_argument("--query", default=None, help="Override the gog search query (testing).")
    ap.add_argument("--account", default=None, help="Limit to a single account email.")
    ap.add_argument("--max", type=int, default=None, help="Override per-account max candidates.")
    ap.add_argument("--verbose", action="store_true", help="Print judge reasoning per email.")
    args = ap.parse_args()

    try:
        sys.stdout.reconfigure(line_buffering=True)  # show per-account progress live
    except Exception:
        pass

    live = args.live and not args.dry_run
    cfg = load_config(args.config)
    tz = tzinfo_for(cfg["timezone"])
    gog = Gog(cfg)

    if not JUDGE_PROMPT_PATH.exists():
        log_err(f"judge prompt missing: {JUDGE_PROMPT_PATH}")
        sys.exit(2)
    template = JUDGE_PROMPT_PATH.read_text()

    log_path = HERE / cfg.get("log_file", "sentry-log.jsonl")
    debug_dir = HERE / "debug"
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")

    base_query = args.query or f"in:inbox is:unread {cfg['lookback']} -label:{cfg['dedup_label']}"
    per_max = args.max or cfg.get("per_account_max", 25)

    accounts = cfg["accounts"]
    if args.account:
        accounts = [a for a in accounts if a["email"] == args.account]
        if not accounts:
            log_err(f"account {args.account} not in config")
            sys.exit(2)

    mode = "LIVE" if live else "DRY-RUN"
    print("=" * 72)
    print(f"EMAIL SENTRY - {mode}  | now {now_local_str(tz)} | run {run_id}")
    print(f"query: {base_query}")
    print(f"judge: {cfg['claude'].get('model','(default)')}  | per-account max: {per_max}")
    if not live:
        print("DRY-RUN: nothing sent, no Gmail labels changed.")
    print("=" * 72)

    grand_candidates = 0
    grand_notify = 0
    notify_items = []

    for a in accounts:
        account, role = a["email"], a["role"]
        threads = gog.search(base_query, account, per_max)
        print(f"\n### {account} ({role}) - {len(threads)} candidate thread(s)")
        if not threads:
            continue
        grand_candidates += len(threads)

        emails = []
        for th in threads:
            e = build_email_obj(gog, th, account, role, tz, cfg.get("body_max_chars", 2500))
            emails.append(e)

        prompt = build_prompt(template, cfg, emails, tz)
        verdicts_raw = run_judge(cfg, prompt, debug_dir)

        # map verdicts by id, with safe fallback per email
        vmap = {}
        if verdicts_raw:
            for vr in verdicts_raw:
                if isinstance(vr, dict) and "id" in vr:
                    vmap[str(vr["id"])] = vr

        for e in emails:
            raw = vmap.get(str(e["id"]))
            if raw is None:
                v = normalize_verdict(
                    {"reasoning": "no verdict returned for this email (parse miss / judge failure) - defaulting to skip",
                     "importance": 1, "category": "other", "rule_fired": "harness:parse-miss", "notify": False},
                    e["id"])
            else:
                v = normalize_verdict(raw, e["id"])

            append_log(log_path, run_id, account, e, v)
            print_email_report(e, v)
            if args.verbose and v["reasoning"]:
                print(f"          why : {v['reasoning'][:240]}")

            if v["notify"]:
                grand_notify += 1
                notify_items.append((e, v))

    # ---- LIVE actions ----
    if live and notify_items:
        print("\n" + "-" * 72)
        print("LIVE: sending notifications + labeling…")
        labels_ensured = set()
        for e, v in notify_items:
            sent = notify_send(cfg, notify_text(e, v))
            print(f"  notify {'OK' if sent else 'FAIL'}: {e['subject'][:60]}")
            if sent:
                if e["account"] not in labels_ensured:
                    gog.ensure_label(cfg["dedup_label"], e["account"])
                    labels_ensured.add(e["account"])
                ok = gog.label_thread(e["thread_id"], cfg["dedup_label"], e["account"])
                print(f"  label  {'OK' if ok else 'FAIL'}: {e['thread_id']}")

    # ---- summary ----
    print("\n" + "=" * 72)
    print(f"SUMMARY: {grand_candidates} candidate(s) across {len(accounts)} account(s); "
          f"{grand_notify} would-notify.")
    if notify_items:
        print("WOULD NOTIFY:" if not live else "NOTIFIED:")
        for e, v in notify_items:
            print(f"  • [{e['account']}] {e['subject'][:64]}  (imp {v['importance']}, {v['category']})")
    else:
        print("Nothing crosses the notify bar. (Silence is the expected default.)")
    print(f"log: {log_path}")
    if not live:
        print("Confirmed: dry-run - nothing sent, no Gmail changes.")
    print("=" * 72)


if __name__ == "__main__":
    main()
