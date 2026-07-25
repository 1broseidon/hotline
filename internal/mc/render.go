package mc

import (
	"fmt"
	"os"
	"strings"
	"time"
)

const (
	// dormantAfter is the age past which a thread renders only as a count, not a
	// row — the injected map shows live work.
	dormantAfter = 30 * 24 * time.Hour
	// handoffStaleAfter is the age past which the handoff renders with a stale tag.
	handoffStaleAfter = 7 * 24 * time.Hour
	// maxRenderThreads / maxRenderStanding bound the rows before budget clipping.
	maxRenderThreads  = 24
	maxRenderStanding = 30
	// maxRowLen clamps a single thread row.
	maxRowLen = 160
	// handoffExcerptBytes bounds the handoff body excerpt.
	handoffExcerptBytes = 600
)

// RenderIndex returns the <mc-index> injection block, kept within budget bytes.
// Sections render in map order — standing notes, threads, handoff — but the
// budget drops thread rows from the bottom first (lowest priority), appending a
// "…+N more threads" pointer. Returns "" only when the store has no content at
// all (unseeded).
func (s *Store) RenderIndex(budget int) string {
	standing := s.renderStanding()
	threadsFull, dormant := s.renderThreads(maxRenderThreads)
	handoff := s.renderHandoff()

	assemble := func(standingRows, threadRows []string, extraNote, ho string) string {
		var b strings.Builder
		b.WriteString("<mc-index>\n")
		if len(standingRows) > 0 {
			b.WriteString("Standing notes:\n")
			for _, n := range standingRows {
				b.WriteString("- " + n + "\n")
			}
			b.WriteString("\n")
		}
		b.WriteString(threadsHeader(len(threadRows), dormant))
		for _, r := range threadRows {
			b.WriteString("- " + r + "\n")
		}
		if extraNote != "" {
			b.WriteString(extraNote + "\n")
		}
		if len(threadRows) > 0 || dormant > 0 || extraNote != "" {
			b.WriteString("\n")
		}
		if ho != "" {
			b.WriteString(ho + "\n")
		}
		b.WriteString("</mc-index>")
		return b.String()
	}

	out := assemble(standing, threadsFull, "", handoff)
	if len(out) <= budget {
		return out
	}
	// Over budget: shrink in reverse priority order so the highest-value content
	// survives longest — spec §2 priority is handoff → standing → threads.
	//
	// 1. Drop thread rows from the bottom (lowest priority) until it fits.
	rows := append([]string{}, threadsFull...)
	for {
		var note string
		if dropped := len(threadsFull) - len(rows); dropped > 0 {
			note = fmt.Sprintf("…+%d more threads — read INDEX.md", dropped)
		}
		out = assemble(standing, rows, note, handoff)
		if len(out) <= budget {
			return out
		}
		if len(rows) == 0 {
			break
		}
		rows = rows[:len(rows)-1]
	}
	allThreadsNote := ""
	if len(threadsFull) > 0 {
		allThreadsNote = fmt.Sprintf("…+%d threads — read INDEX.md", len(threadsFull))
	}

	// 2. Drop standing notes, oldest first. renderStanding is newest-first, so
	//    the oldest live at the tail.
	st := append([]string{}, standing...)
	for len(st) > 0 {
		st = st[:len(st)-1]
		out = assemble(st, nil, allThreadsNote, handoff)
		if len(out) <= budget {
			return out
		}
	}

	// 3. Clamp the handoff excerpt progressively.
	for ex := handoffExcerptBytes / 2; ex > 0; ex /= 2 {
		out = assemble(nil, nil, allThreadsNote, s.renderHandoffExcerpt(ex))
		if len(out) <= budget {
			return out
		}
	}

	// 4. Backstop: drop the handoff entirely, then hard-clamp at a rune boundary
	//    so the returned block is never larger than budget, for any budget.
	out = assemble(nil, nil, allThreadsNote, "")
	if len(out) > budget {
		out = clampField2(out, budget)
	}
	return out
}

func threadsHeader(shown, dormant int) string {
	if dormant > 0 {
		return fmt.Sprintf("Threads (%d active, …+%d dormant — INDEX.md):\n", shown, dormant)
	}
	return fmt.Sprintf("Threads (%d active):\n", shown)
}

func (s *Store) renderStanding() []string {
	notes := s.standingNotes()
	// Newest first, capped.
	rev := make([]string, 0, len(notes))
	for i := len(notes) - 1; i >= 0; i-- {
		rev = append(rev, notes[i])
		if len(rev) >= maxRenderStanding {
			break
		}
	}
	return rev
}

// renderThreads returns up to max live thread rows plus the count of dormant
// (>30d) threads. done threads never appear (they live in archive/).
func (s *Store) renderThreads(max int) (rows []string, dormant int) {
	threads, err := s.activeThreads()
	if err != nil {
		return nil, 0
	}
	now := s.now().UTC()
	for _, t := range threads {
		if t.status == "done" {
			continue
		}
		if isDormant(now, t.updated) {
			dormant++
			continue
		}
		if len(rows) >= max {
			dormant++ // beyond the row cap also counts toward the "more" tail
			continue
		}
		rows = append(rows, clampRow(threadRow(t)))
	}
	return rows, dormant
}

func threadRow(t *thread) string {
	line := t.slug + " [" + orValue(t.status, "active") + "] " + orDash(t.summary)
	if t.next != "" {
		line += " → " + t.next
	}
	line += " (upd " + mmdd(t.updated) + ")"
	return oneLine(line)
}

func clampRow(s string) string {
	if len(s) <= maxRowLen {
		return s
	}
	// Back the cut up to a rune boundary (clampField2) so a multi-byte glyph is
	// never sliced into invalid UTF-8; the ellipsis costs 3 bytes.
	return strings.TrimSpace(clampField2(s, maxRowLen-len("…"))) + "…"
}

// renderHandoff returns the injected handoff line(s), with a stale tag when the
// handoff is older than handoffStaleAfter. Empty when no handoff.md exists.
func (s *Store) renderHandoff() string {
	return s.renderHandoffExcerpt(handoffExcerptBytes)
}

// renderHandoffExcerpt is renderHandoff with a caller-chosen excerpt cap, so the
// budget packer can shrink the handoff body as a last resort before dropping it.
func (s *Store) renderHandoffExcerpt(maxExcerpt int) string {
	raw, err := os.ReadFile(s.handoffPath())
	if err != nil {
		return ""
	}
	written, trigger, body := parseHandoff(string(raw))
	head := "Handoff"
	if !written.IsZero() {
		head = fmt.Sprintf("Handoff (%s, %s)", written.Format("2006-01-02 15:04"), orValue(trigger, "manual"))
		if age := s.now().UTC().Sub(written); age > handoffStaleAfter {
			head += fmt.Sprintf(" (stale — %dd old)", int(age.Hours()/24))
		}
	}
	excerpt := body
	if len(excerpt) > maxExcerpt {
		excerpt = clampField2(excerpt, maxExcerpt)
	}
	excerpt = strings.TrimSpace(strings.ReplaceAll(excerpt, "\n", " "))
	return head + ":\n" + excerpt
}

// parseHandoff extracts the written time, trigger, and body from handoff.md.
func parseHandoff(raw string) (written time.Time, trigger, body string) {
	if !strings.HasPrefix(raw, "---\n") {
		return time.Time{}, "", strings.TrimSpace(raw)
	}
	rest := raw[len("---\n"):]
	end := strings.Index(rest, "\n---")
	if end < 0 {
		return time.Time{}, "", strings.TrimSpace(raw)
	}
	fm := rest[:end]
	body = strings.TrimSpace(strings.TrimPrefix(rest[end+len("\n---"):], "\n"))
	for _, line := range strings.Split(fm, "\n") {
		k, v, ok := strings.Cut(line, ":")
		if !ok {
			continue
		}
		k = strings.TrimSpace(k)
		v = strings.TrimSpace(v)
		switch k {
		case "written":
			if t, err := time.Parse(time.RFC3339, v); err == nil {
				written = t.UTC()
			}
		case "trigger":
			trigger = v
		}
	}
	return written, trigger, body
}

func isDormant(now time.Time, updated string) bool {
	t, err := time.Parse(time.RFC3339, updated)
	if err != nil {
		return false // unparseable/blank → treat as live
	}
	return now.Sub(t.UTC()) > dormantAfter
}

func mmdd(ts string) string {
	if t, err := time.Parse(time.RFC3339, ts); err == nil {
		return t.UTC().Format("01-02")
	}
	return "??-??"
}

// clampField2 truncates to n bytes at a rune boundary. Negative n clamps to
// empty rather than panicking (config rejects n<=0, but RenderIndex is exported).
func clampField2(s string, n int) string {
	if n < 0 {
		n = 0
	}
	if len(s) <= n {
		return s
	}
	for n > 0 && !isRuneStart(s[n]) {
		n--
	}
	return s[:n]
}

func isRuneStart(b byte) bool { return b&0xC0 != 0x80 }
