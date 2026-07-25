#!/usr/bin/env bash
# Integration smoke: load the hotline-pi extension in real `pi --mode rpc`, wire
# it to the Go-shaped fake-hotline child, let the fake inject a synthetic inbound
# channel notification, and assert the full loop:
#   - extension spawns the child, handshakes, registers hotline's tools
#   - the injected envelope is forwarded via pi.sendUserMessage -> a turn fires
#   - the model calls `reply` on the child (recorded to $OUT)
#   - pi's stdout stays valid JSONL throughout (probe-2 invariant)
#
# Mirrors probe 2 (stdin held open via a FIFO so pi never EOF-exits) but drives
# the loop through OUR client end to end.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
EXT="$HERE/../src/index.ts"
WORK="$(mktemp -d)"
OUT="$WORK/calls.jsonl"
STDOUT="$WORK/stdout.jsonl"
STDERR="$WORK/stderr.log"
FIFO="$WORK/in.pipe"

PROVIDER="${SMOKE_PROVIDER:-github-copilot}"
MODEL="${SMOKE_MODEL:-gpt-5.4-mini}"

chmod +x "$HERE/hotline-fake-bin.sh"
mkfifo "$FIFO"

echo "smoke: provider=$PROVIDER model=$MODEL work=$WORK"

# Point the extension's `hotline` spawn at the fake, and have the fake inject one
# inbound channel notification 3s after the handshake, recording tools/call to $OUT.
export HOTLINE_BIN="$HERE/hotline-fake-bin.sh"
export HOTLINE_FAKE_OUT="$OUT"
export HOTLINE_FAKE_INJECT_MS="3000"
export HOTLINE_FAKE_ENVELOPE='<channel source="telegram" chat_id="777" user="george">
ping — please answer by calling the reply tool with chat_id "777" and bubbles ["pong"].
</channel>'
export HOTLINE_PI_LOG="$WORK/ext.log"

# Hold pi's stdin open (write end on fd 3) but never send an RPC command.
exec 3<>"$FIFO"
timeout 45 pi --mode rpc -e "$EXT" \
  --provider "$PROVIDER" --model "$MODEL" \
  <"$FIFO" >"$STDOUT" 2>"$STDERR"
RC=$?
exec 3>&-

echo "smoke: pi exited rc=$RC"
echo "---- extension log (tail) ----"; tail -20 "$WORK/ext.log" 2>/dev/null
echo "---- pi stderr (tail) ----"; tail -20 "$STDERR" 2>/dev/null

FAIL=0

# 1) stdout is valid JSONL (probe-2 invariant).
python3 - "$STDOUT" <<'PY'
import json, sys
n = 0
bad = 0
for line in open(sys.argv[1]):
    if not line.strip():
        continue
    n += 1
    try:
        json.loads(line)
    except Exception:
        bad += 1
print(f"stdout lines={n} invalid={bad}")
sys.exit(1 if bad or n == 0 else 0)
PY
if [ $? -ne 0 ]; then echo "FAIL: stdout not valid JSONL"; FAIL=1; else echo "ok: stdout valid JSONL"; fi

# 2) no extension log text leaked onto stdout.
if grep -q "hotline-pi" "$STDOUT"; then echo "FAIL: extension log leaked to stdout"; FAIL=1; else echo "ok: no extension log on stdout"; fi

# 3) the model called reply on the child with chat_id 777.
if [ -f "$OUT" ] && grep -q '"name":"reply"' "$OUT" && grep -q '777' "$OUT"; then
  echo "ok: model called reply on the child"; echo "   -> $(grep '"name":"reply"' "$OUT" | head -1)"
else
  echo "FAIL: no reply tool call recorded"; echo "   OUT contents:"; cat "$OUT" 2>/dev/null; FAIL=1
fi

# 4) a turn actually ran (agent lifecycle present on stdout).
if grep -q '"agent_end"\|"turn_end"\|tool_execution' "$STDOUT"; then echo "ok: a turn fired"; else echo "FAIL: no turn on stdout"; FAIL=1; fi

echo "SMOKE_RESULT=$([ $FAIL -eq 0 ] && echo PASS || echo FAIL) work=$WORK"
exit $FAIL
