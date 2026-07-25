package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/supervise"
)

// cmdTail is the read-only live viewer over the harness event log the
// supervisor already captures: `hotline tail [--state-dir <dir>] [-f|--follow]
// [--no-follow] [--json] [-n <lines>]`. It pretty-prints the pi harness's JSONL
// event stream (`<box-root>/supervisor/harness.log`) so an operator can watch a
// box think in real time — turns in, tool calls with a one-line arg digest,
// replies forming, usage on turn boundaries — the way a Claude Code session is
// attachable. Chat input is deliberately out of scope; this only reads.
//
// Rich rendering targets the pi harness (structured events). The opencode and
// claude harnesses write their own output to the same file (an HTTP daemon's
// server log; a pty TUI's raw frames), which lack the pi event schema — those
// lines fall through to a dim one-line passthrough rather than a turn view.
//
// Arg parsing mirrors cmd_job.go (hand-rolled --flag[=v] loop); the render core
// is a pure function so it can be table-tested against real sample lines.
func cmdTail(botName string, args []string, stdout, stderr io.Writer) int {
	stateDir := ""
	jsonOut := false
	lines := 40
	// follow tri-state: default follows when stdout is a TTY, but --no-follow
	// or -n 0 turns it off, and -f/--follow forces it on for a pipe.
	forceFollow := false
	noFollow := false

	for i := 0; i < len(args); i++ {
		a := args[i]
		key := a
		if eq := strings.IndexByte(a, '='); eq >= 0 && strings.HasPrefix(a, "-") {
			key = a[:eq]
		}
		val := func() (string, bool) {
			if eq := strings.IndexByte(a, '='); strings.HasPrefix(a, "-") && eq >= 0 {
				return a[eq+1:], true
			}
			if i+1 >= len(args) {
				return "", false
			}
			i++
			return args[i], true
		}
		switch key {
		case "--state-dir":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--state-dir needs a value")
			}
			stateDir = v
		case "--json":
			jsonOut = true
		case "-f", "--follow":
			forceFollow = true
		case "--no-follow":
			noFollow = true
		case "-n", "--lines":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "-n needs a value")
			}
			n, err := strconv.Atoi(strings.TrimSpace(v))
			if err != nil || n < 0 {
				return usageErr(stderr, "-n must be a non-negative integer")
			}
			lines = n
		case "-h", "--help":
			fmt.Fprint(stdout, tailHelp)
			return exitAccepted
		default:
			return usageErr(stderr, fmt.Sprintf("unknown flag %q (hotline tail [--state-dir <dir>] [-f|--follow] [--no-follow] [--json] [-n <lines>])", a))
		}
	}

	// An explicit --state-dir keeps its direct-directory meaning. Only the
	// implicit default is resolved through the selected box.
	boxRoot := stateDir
	if boxRoot == "" {
		r, err := config.BoxRoot(botName)
		if err != nil {
			fmt.Fprintf(stderr, "hotline: %v\n", err)
			return exitInternal
		}
		boxRoot = r
	}
	logPath := filepath.Join(supervise.Dir(boxRoot), supervise.HarnessLogName)

	// Follow when the terminal is interactive, unless told otherwise. -n 0 is
	// the scripting shorthand for "print nothing new, don't follow".
	follow := stdoutIsTTY(stdout)
	if forceFollow {
		follow = true
	}
	if noFollow || lines == 0 {
		follow = false
	}

	color := !jsonOut && stdoutIsTTY(stdout)
	r := newTailRenderer(color)

	// Backfill: render every historical line (accumulating renderer state — the
	// last-seen timestamp and any in-flight tool calls) but only print the last
	// N rendered lines. The file is size-capped by the supervisor's rotating
	// writer, so reading it whole is bounded.
	offset := int64(0)
	data, err := os.ReadFile(logPath)
	if err != nil {
		if os.IsNotExist(err) {
			fmt.Fprintf(stderr, "hotline: no harness log at %s (is a box running? `hotline up`)\n", logPath)
			if !follow {
				return exitInternal
			}
			// In follow mode, wait for the file to appear rather than bailing.
		} else {
			fmt.Fprintf(stderr, "hotline: reading %s: %v\n", logPath, err)
			return exitInternal
		}
	} else {
		offset = int64(len(data))
		rendered := make([]string, 0, 64)
		for _, raw := range splitLines(data) {
			if jsonOut {
				if s, ok := jsonPassthrough(raw); ok {
					rendered = append(rendered, s)
				}
				continue
			}
			if out := r.render(raw); out != "" {
				rendered = append(rendered, out)
			}
		}
		if lines > 0 && len(rendered) > lines {
			rendered = rendered[len(rendered)-lines:]
		}
		for _, out := range rendered {
			fmt.Fprintln(stdout, out)
		}
	}

	if !follow {
		return exitAccepted
	}

	// Live follow: poll the file, render appended lines. r.live flips on so
	// tool-call durations (wall-clock between start/end we observe) render.
	r.live = true
	poll := 250 * time.Millisecond
	for {
		chunk, newOffset, reset, err := readChunk(logPath, offset)
		if err != nil {
			// File may be missing mid-rotation or not created yet; wait and retry.
			time.Sleep(poll)
			continue
		}
		if reset {
			// Rotation or truncation: the writer renamed harness.log to .1 and
			// started fresh (or truncated in place). Re-read from the top.
			offset = 0
			continue
		}
		offset = newOffset
		for _, raw := range chunk {
			if jsonOut {
				if s, ok := jsonPassthrough(raw); ok {
					fmt.Fprintln(stdout, s)
				}
				continue
			}
			if out := r.render(raw); out != "" {
				fmt.Fprintln(stdout, out)
			}
		}
		time.Sleep(poll)
	}
}

const tailHelp = `hotline tail - watch a supervised box think, live

Usage:
  hotline tail [--state-dir <dir>] [-f|--follow] [--no-follow] [--json] [-n <lines>]

Reads the harness event log the supervisor captures at
<box-root>/supervisor/harness.log and pretty-prints it: inbound turns, tool calls
with a one-line arg digest, replies forming, and per-turn token usage.

Rich turn rendering targets the pi harness (structured JSONL events). The
opencode and claude harnesses write different output to the same file (an HTTP
daemon log; a pty TUI), so those lines show as a dim passthrough, not a turn
view. This is a read-only viewer — there is no chat input.

Flags:
  --state-dir <dir>  exact state root to read (default: the selected box root).
                     The log is <dir>/supervisor/harness.log.
  -f, --follow       keep following the log (default when stdout is a TTY).
  --no-follow        print and exit (also implied by -n 0), for scripting.
  --json             pass through raw parseable event lines, one per line.
  -n <lines>         how many past events to show first (default 40; 0 = none).
`

// ── render core ────────────────────────────────────────────────────────────

// tailRenderer turns raw harness.log lines into single-line, terminal-readable
// summaries. It carries a little state across lines: the last timestamp seen
// (so events without their own timestamp still get a coherent HH:MM:SS prefix)
// and the wall-clock start of each in-flight tool call (so tool_execution_end
// can show a duration during live follow).
type tailRenderer struct {
	color   bool
	live    bool
	now     func() time.Time
	lastTS  time.Time
	started map[string]time.Time
}

func newTailRenderer(color bool) *tailRenderer {
	return &tailRenderer{
		color:   color,
		now:     time.Now,
		started: map[string]time.Time{},
	}
}

// harnessEvent is the union of the fields we read across event types. Unknown
// types keep their Type and fall through to a dim one-liner — never a crash.
type harnessEvent struct {
	Type       string          `json:"type"`
	Message    *harnessMessage `json:"message"`
	ToolCallID string          `json:"toolCallId"`
	ToolName   string          `json:"toolName"`
	Args       json.RawMessage `json:"args"`
	Result     json.RawMessage `json:"result"`
	IsError    bool            `json:"isError"`
	Model      string          `json:"model"`
	Usage      *usageInfo      `json:"usage"`
	StatusText string          `json:"statusText"`
}

type harnessMessage struct {
	Role      string         `json:"role"`
	Content   []contentBlock `json:"content"`
	Model     string         `json:"model"`
	Usage     *usageInfo     `json:"usage"`
	Timestamp int64          `json:"timestamp"`
}

type contentBlock struct {
	Type      string          `json:"type"`
	Text      string          `json:"text"`
	Name      string          `json:"name"`
	Arguments json.RawMessage `json:"arguments"`
}

type usageInfo struct {
	Input       int `json:"input"`
	Output      int `json:"output"`
	CacheRead   int `json:"cacheRead"`
	TotalTokens int `json:"totalTokens"`
}

// render returns the rendered line(s) for one raw log line, or "" to skip it
// (delta spam, empty lines, silent lifecycle events). Multiple output lines are
// joined with '\n' (an assistant message can carry several content blocks).
func (r *tailRenderer) render(raw string) string {
	raw = strings.TrimRight(raw, "\r\n")
	if strings.TrimSpace(raw) == "" {
		return ""
	}

	var ev harnessEvent
	if err := json.Unmarshal([]byte(raw), &ev); err != nil || ev.Type == "" {
		// Non-JSON line: supervisor / harness plaintext (e.g. "hotline:
		// harness=pi", "[hotline-pi …] INFO …"). Show it dim so the operator
		// still sees lifecycle signal, without pretending it's a turn event.
		return r.plain("· " + firstLine(collapseWS(raw), 140))
	}

	if ts := r.tsOf(&ev); !ts.IsZero() {
		r.lastTS = ts
	}

	switch ev.Type {
	case "message_start":
		// Inbound user turn. (The assistant/toolResult variants are rendered
		// from message_end / tool_execution_end instead, to avoid duplicates.)
		if ev.Message != nil && ev.Message.Role == "user" {
			return r.line("◀", cCyan, "user: "+firstLine(unwrapChannel(ev.Message.firstText()), 100))
		}
		return ""

	case "message_end":
		if ev.Message == nil || ev.Message.Role != "assistant" {
			return "" // user/toolResult message_end duplicates other events
		}
		model := shortModel(ev.Message.Model)
		var out []string
		for _, c := range ev.Message.Content {
			switch c.Type {
			case "text":
				if strings.TrimSpace(c.Text) != "" {
					out = append(out, r.line("▶", cBold, model+": "+firstLine(collapseWS(c.Text), 140)))
				}
			case "toolCall":
				out = append(out, r.line("⚙", cYellow, c.Name+" "+digestJSON(c.Arguments, 80)))
			}
		}
		return strings.Join(out, "\n")

	case "tool_execution_start":
		// Record the wall-clock start so _end can show a duration; the ⚙ line
		// was already emitted from the assistant message_end content.
		if ev.ToolCallID != "" {
			r.started[ev.ToolCallID] = r.now()
		}
		return ""

	case "tool_execution_end":
		glyph, col := "✓", cGreen
		if ev.IsError {
			glyph, col = "✗", cRed
		}
		var b strings.Builder
		b.WriteString(ev.ToolName)
		if d := r.durationOf(ev.ToolCallID); d != "" {
			b.WriteString(" (" + d + ")")
		}
		if dig := digestResult(ev.Result); dig != "" {
			b.WriteString(" " + dig)
		}
		return r.line(glyph, col, b.String())

	case "turn_end":
		return r.plain("── turn " + usageDigest(ev.usageForTurn()))

	case "agent_end":
		return r.plain("══ agent done " + usageDigest(ev.Usage))

	case "message_update", "turn_start", "agent_start", "agent_settled", "extension_ui_request":
		// Known but intentionally silent: per-token deltas and lifecycle noise.
		return ""

	default:
		// Unknown/new event type: never crash, just note it dim.
		return r.plain("· " + ev.Type)
	}
}

// tsOf extracts an embedded millisecond timestamp when the event carries one.
func (r *tailRenderer) tsOf(ev *harnessEvent) time.Time {
	if ev.Message != nil && ev.Message.Timestamp > 0 {
		return time.UnixMilli(ev.Message.Timestamp)
	}
	return time.Time{}
}

// usageForTurn prefers the turn message's usage, falling back to top-level.
func (ev *harnessEvent) usageForTurn() *usageInfo {
	if ev.Message != nil && ev.Message.Usage != nil {
		return ev.Message.Usage
	}
	return ev.Usage
}

func (r *tailRenderer) durationOf(id string) string {
	if !r.live || id == "" {
		return ""
	}
	start, ok := r.started[id]
	if !ok {
		return ""
	}
	delete(r.started, id)
	return shortDuration(r.now().Sub(start))
}

func (m *harnessMessage) firstText() string {
	for _, c := range m.Content {
		if c.Type == "text" && c.Text != "" {
			return c.Text
		}
	}
	if len(m.Content) > 0 {
		return m.Content[0].Text
	}
	return ""
}

// ── line formatting ────────────────────────────────────────────────────────

// ANSI codes, applied only when the renderer is in color mode.
const (
	cReset  = "\x1b[0m"
	cDim    = "\x1b[2m"
	cRed    = "\x1b[31m"
	cGreen  = "\x1b[32m"
	cYellow = "\x1b[33m"
	cCyan   = "\x1b[36m"
	cBold   = "\x1b[1m"
)

// line renders "HH:MM:SS <glyph> <body>" with the glyph colorized.
func (r *tailRenderer) line(glyph, glyphColor, body string) string {
	g := glyph
	if r.color {
		g = glyphColor + glyph + cReset
	}
	return r.stamp() + g + " " + body
}

// plain renders a whole line dim (separators, passthrough, unknowns).
func (r *tailRenderer) plain(body string) string {
	if r.color {
		return r.stamp() + cDim + body + cReset
	}
	return r.stamp() + body
}

func (r *tailRenderer) stamp() string {
	t := r.lastTS
	if t.IsZero() {
		t = r.now()
	}
	s := t.Format("15:04:05") + " "
	if r.color {
		return cDim + s + cReset
	}
	return s
}

// ── digests ────────────────────────────────────────────────────────────────

// digestJSON compacts a raw JSON value to a single whitespace-collapsed line,
// truncated to n runes — the one-line arg summary for a tool call.
func digestJSON(raw json.RawMessage, n int) string {
	if len(raw) == 0 {
		return ""
	}
	var buf bytes.Buffer
	if err := json.Compact(&buf, raw); err != nil {
		return firstLine(collapseWS(string(raw)), n)
	}
	return firstLine(collapseWS(buf.String()), n)
}

// digestResult extracts a tool result's text content (the common
// {content:[{type:"text",text:…}]} shape), falling back to a compact digest.
func digestResult(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}
	var res struct {
		Content []struct {
			Text string `json:"text"`
		} `json:"content"`
	}
	if err := json.Unmarshal(raw, &res); err == nil {
		var parts []string
		for _, c := range res.Content {
			if strings.TrimSpace(c.Text) != "" {
				parts = append(parts, c.Text)
			}
		}
		if len(parts) > 0 {
			return firstLine(collapseWS(strings.Join(parts, " ")), 80)
		}
	}
	return digestJSON(raw, 80)
}

// usageDigest renders "(in→out tok, N total)" when usage is present.
func usageDigest(u *usageInfo) string {
	if u == nil || u.TotalTokens == 0 {
		return ""
	}
	return fmt.Sprintf("(%d→%d tok, %d total)", u.Input, u.Output, u.TotalTokens)
}

// ── small helpers ──────────────────────────────────────────────────────────

// jsonPassthrough returns the raw line unchanged if it parses as a JSON event
// (has a "type"), for --json scripting. Plaintext lines are filtered out.
func jsonPassthrough(raw string) (string, bool) {
	raw = strings.TrimRight(raw, "\r\n")
	if strings.TrimSpace(raw) == "" {
		return "", false
	}
	var probe struct {
		Type string `json:"type"`
	}
	if err := json.Unmarshal([]byte(raw), &probe); err != nil || probe.Type == "" {
		return "", false
	}
	return raw, true
}

// unwrapChannel pulls the human body out of a "<channel …>\n<body>\n</channel>"
// envelope, returning the input unchanged when it isn't wrapped.
func unwrapChannel(s string) string {
	s = strings.TrimSpace(s)
	if !strings.HasPrefix(s, "<channel") {
		return s
	}
	open := strings.IndexByte(s, '>')
	if open < 0 {
		return s
	}
	body := s[open+1:]
	if close := strings.LastIndex(body, "</channel>"); close >= 0 {
		body = body[:close]
	}
	return strings.TrimSpace(body)
}

// shortModel keeps the recognizable tail of a model id ("openai/gpt-5" →
// "gpt-5"), so the ▶ prefix stays compact.
func shortModel(m string) string {
	m = strings.TrimSpace(m)
	if m == "" {
		return "assistant"
	}
	if i := strings.LastIndexByte(m, '/'); i >= 0 && i+1 < len(m) {
		m = m[i+1:]
	}
	return m
}

// collapseWS turns any run of whitespace (including newlines) into one space.
func collapseWS(s string) string {
	return strings.Join(strings.Fields(s), " ")
}

func shortDuration(d time.Duration) string {
	switch {
	case d < 0:
		return ""
	case d < time.Second:
		return fmt.Sprintf("%dms", d.Milliseconds())
	case d < time.Minute:
		return fmt.Sprintf("%.1fs", d.Seconds())
	default:
		return fmt.Sprintf("%dm%02ds", int(d.Minutes()), int(d.Seconds())%60)
	}
}

// splitLines splits a byte buffer into lines without a trailing empty element.
func splitLines(data []byte) []string {
	s := string(data)
	s = strings.TrimRight(s, "\n")
	if s == "" {
		return nil
	}
	return strings.Split(s, "\n")
}

// ── file following ─────────────────────────────────────────────────────────

// readChunk reads any complete lines appended to path past offset. It returns
// reset=true when the file has shrunk below offset (the supervisor's rotating
// writer renamed harness.log→.1 and started fresh, or truncated in place), so
// the caller re-reads from the top. Only whole lines (up to the last newline)
// are returned; a trailing partial line is left for the next poll by not
// advancing the offset past it.
func readChunk(path string, offset int64) (lines []string, newOffset int64, reset bool, err error) {
	fi, err := os.Stat(path)
	if err != nil {
		return nil, offset, false, err
	}
	if fi.Size() < offset {
		return nil, 0, true, nil
	}
	if fi.Size() == offset {
		return nil, offset, false, nil
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, offset, false, err
	}
	defer f.Close()
	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return nil, offset, false, err
	}
	data, err := io.ReadAll(f)
	if err != nil {
		return nil, offset, false, err
	}
	nl := bytes.LastIndexByte(data, '\n')
	if nl < 0 {
		// No complete line yet; leave everything buffered for next poll.
		return nil, offset, false, nil
	}
	complete := data[:nl] // excludes the final newline
	return splitLines(complete), offset + int64(nl) + 1, false, nil
}

// stdoutIsTTY reports whether w is an interactive terminal, mirroring
// stdinIsTTY's os.ModeCharDevice check (no extra dependency).
func stdoutIsTTY(w io.Writer) bool {
	f, ok := w.(*os.File)
	if !ok {
		return false
	}
	fi, err := f.Stat()
	if err != nil {
		return false
	}
	return fi.Mode()&os.ModeCharDevice != 0
}
