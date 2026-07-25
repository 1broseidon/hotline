// Package mc implements Mission Control: the box agent's durable working memory
// as plain markdown under the state dir's mc/ folder. One file per thread (small
// front-matter truth + an append-mostly log), a regenerated INDEX.md whose
// Threads table lives between marker comments (everything else is preserved
// verbatim), a single overwritten handoff.md, and an archive/ for closed work.
//
// Reads are free — the injected map (see RenderIndex) carries the state into the
// model at session start. Writes go through one schema-validated verb tool (see
// Apply), so a weak harness can never malform the store: the tool owns every
// timestamp, front-matter block, and index regeneration; the agent only supplies
// the facts.
//
// All mutations run under a flock(LOCK_EX) on mc/.lock and land through
// tmp+rename atomic writes — the notify/jobspool idiom — so a CLI twin (P1) and
// the running box can never race or leave a half-written file.
package mc

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"
)

const (
	// maxField clamps every agent-supplied text field, at a rune boundary, so a
	// long value is truncated (jobspool.clampField idiom) rather than rejected.
	maxField = 2048
	// maxStandingNotes bounds the standing-notes list; overflow rolls oldest
	// into archive/standing-YYYY-MM.md.
	maxStandingNotes = 30
	// maxArchivedHandoffs bounds archive/handoffs.md (last N superseded handoffs).
	maxArchivedHandoffs = 20

	threadsBeginMarker = "<!-- mc:threads -->"
	threadsEndMarker   = "<!-- /mc:threads -->"
	standingHeader     = "## Standing notes"
)

// slugRe is the thread-slug grammar: kebab-case, 1–41 chars, leading alnum.
var slugRe = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,40}$`)

// Store is a Mission Control directory. It is safe to construct cheaply per call;
// all state lives on disk.
type Store struct {
	dir string
	now func() time.Time
}

// NewStore returns a Store rooted at dir (the mc/ folder).
func NewStore(dir string) *Store { return &Store{dir: dir, now: time.Now} }

// SetClock overrides the time source (tests pin a fixed clock).
func (s *Store) SetClock(fn func() time.Time) { s.now = fn }

// Dir returns the Mission Control root directory.
func (s *Store) Dir() string { return s.dir }

func (s *Store) threadsDir() string  { return filepath.Join(s.dir, "threads") }
func (s *Store) archiveDir() string  { return filepath.Join(s.dir, "archive") }
func (s *Store) indexPath() string   { return filepath.Join(s.dir, "INDEX.md") }
func (s *Store) handoffPath() string { return filepath.Join(s.dir, "handoff.md") }
func (s *Store) lockPath() string    { return filepath.Join(s.dir, ".lock") }

// Input is the decoded argument set for one mission verb (mirrors missionSchema).
type Input struct {
	Action  string
	Thread  string
	Text    string
	Status  string
	Summary string
	Next    string
	State   string
	Beware  string
	Outcome string
	// Trigger records what caused a handoff (P0 ambiguity resolution #1). The
	// mission MCP tool leaves it empty and the store defaults it to "manual"; the
	// CLI twin (hotline mission handoff --trigger …) is where pre-compact /
	// boundary / auto values enter.
	Trigger string
}

// Seed creates the directory skeleton and, if INDEX.md is absent, writes the seed
// map (one standing note, empty threads table). An existing mc/ is left untouched
// — no re-seed, no clobber, mirroring migrateState's posture. Idempotent.
func (s *Store) Seed() error {
	return s.withLock(func() error {
		for _, d := range []string{s.dir, s.threadsDir(), s.archiveDir()} {
			if err := os.MkdirAll(d, 0o700); err != nil {
				return err
			}
		}
		if _, err := os.Stat(s.indexPath()); err == nil {
			return nil // present — no clobber
		} else if !os.IsNotExist(err) {
			return err
		}
		date := s.now().UTC().Format("2006-01-02")
		// Seed an EMPTY marker block (spec §6): the table is regenerated on the
		// first real write. Seeding a header-only table would show a phantom
		// column header with zero threads.
		seed := "# INDEX — mission control map\n" +
			"Maintained by the mission tool. The Threads table regenerates; everything else is yours.\n\n" +
			standingHeader + "\n" +
			"- " + date + ": mission control initialized; file the first thread when real work starts.\n\n" +
			threadsBeginMarker + "\n" + threadsEndMarker + "\n"
		return atomicWrite(s.indexPath(), []byte(seed))
	})
}

// Apply validates and executes one mission verb under the store lock, returning
// a human-readable confirmation (or a "mission failed: …" message with isErr).
// It never panics on bad input — malformed args are reported, never fatal.
func (s *Store) Apply(in Input) (msg string, isErr bool) {
	in.Text = clampField(in.Text)
	in.Summary = clampField(oneLine(in.Summary))
	in.Next = clampField(oneLine(in.Next))
	in.State = clampField(in.State)
	in.Beware = clampField(in.Beware)
	in.Outcome = clampField(in.Outcome)

	if err := s.withLock(func() error {
		msg, isErr = s.apply(in)
		return nil
	}); err != nil {
		return "mission failed: " + err.Error(), true
	}
	return msg, isErr
}

func (s *Store) apply(in Input) (string, bool) {
	switch in.Action {
	case "note":
		return s.doNote(in)
	case "update":
		return s.doUpdate(in)
	case "handoff":
		return s.doHandoff(in)
	case "archive":
		return s.doArchive(in)
	default:
		return fmt.Sprintf("mission failed: unknown action %q (note, update, handoff, archive)", in.Action), true
	}
}

// doNote appends a durable fact: to a thread's log when Thread is set, otherwise
// to the standing-notes list.
func (s *Store) doNote(in Input) (string, bool) {
	if in.Text == "" {
		return "mission failed: note requires text", true
	}
	if in.Thread != "" {
		if !slugRe.MatchString(in.Thread) {
			return badSlug(), true
		}
		t, ok, err := s.loadThread(in.Thread)
		if err != nil {
			return "mission failed: " + err.Error(), true
		}
		if !ok {
			return s.unknownThread(in.Thread), true
		}
		t.appendLog(s.stamp(), in.Text)
		t.updated = s.nowRFC()
		if err := s.saveThread(t); err != nil {
			return "mission failed: " + err.Error(), true
		}
		if err := s.regenIndex(); err != nil {
			return "mission failed: " + err.Error(), true
		}
		return "mission noted (thread: " + t.slug + ")", false
	}
	if err := s.addStandingNote(in.Text); err != nil {
		return "mission failed: " + err.Error(), true
	}
	return "mission noted (standing notes)", false
}

// doUpdate upserts a thread: creates it if new, applies status/summary/next,
// appends an optional log line, and re-syncs the index. status=done archives.
func (s *Store) doUpdate(in Input) (string, bool) {
	if in.Thread == "" {
		return "mission failed: update requires thread", true
	}
	if !slugRe.MatchString(in.Thread) {
		return badSlug(), true
	}
	if in.Status == "" && in.Summary == "" && in.Next == "" && in.Text == "" {
		return `mission failed: update needs at least one of status, summary, next, or text`, true
	}
	if in.Status != "" && !validStatus(in.Status) {
		return fmt.Sprintf("mission failed: status must be active, paused, or done (got %q)", in.Status), true
	}

	t, ok, err := s.loadThread(in.Thread)
	if err != nil {
		return "mission failed: " + err.Error(), true
	}
	if !ok {
		t = newThread(in.Thread)
	}
	if in.Summary != "" {
		t.summary = in.Summary
	}
	if in.Next != "" {
		t.next = in.Next
	}
	if in.Status != "" {
		t.status = in.Status
	} else if t.status == "" {
		t.status = "active"
	}
	if in.Text != "" {
		t.appendLog(s.stamp(), in.Text)
	}
	t.updated = s.nowRFC()

	// status=done is equivalent to archive. With no outcome supplied, the summary
	// (or a generic line) stamps the closing log entry.
	if in.Status == "done" {
		outcome := in.Outcome
		if outcome == "" {
			outcome = "closed via update status=done"
		}
		return s.archiveThread(t, outcome)
	}

	if err := s.saveThread(t); err != nil {
		return "mission failed: " + err.Error(), true
	}
	if err := s.regenIndex(); err != nil {
		return "mission failed: " + err.Error(), true
	}
	return fmt.Sprintf("mission updated (%s: %s — next: %s)", t.slug, t.status, orDash(t.next)), false
}

// doArchive closes a thread, moving its file to archive/ with the outcome stamped
// as the final log line.
func (s *Store) doArchive(in Input) (string, bool) {
	if in.Thread == "" {
		return "mission failed: archive requires thread", true
	}
	if !slugRe.MatchString(in.Thread) {
		return badSlug(), true
	}
	if in.Outcome == "" {
		return "mission failed: archive requires outcome", true
	}
	t, ok, err := s.loadThread(in.Thread)
	if err != nil {
		return "mission failed: " + err.Error(), true
	}
	if !ok {
		return s.unknownThread(in.Thread), true
	}
	t.updated = s.nowRFC()
	return s.archiveThread(t, in.Outcome)
}

// archiveThread stamps the outcome, sets status done, moves threads/<slug>.md to
// archive/<slug>.md, and re-syncs the index. Shared by doArchive and doUpdate.
func (s *Store) archiveThread(t *thread, outcome string) (string, bool) {
	t.status = "done"
	t.appendLog(s.stamp(), "archived — "+outcome)
	if err := os.MkdirAll(s.archiveDir(), 0o700); err != nil {
		return "mission failed: " + err.Error(), true
	}
	dst := filepath.Join(s.archiveDir(), t.slug+".md")
	if err := atomicWrite(dst, []byte(t.serialize())); err != nil {
		return "mission failed: " + err.Error(), true
	}
	_ = os.Remove(filepath.Join(s.threadsDir(), t.slug+".md"))
	if err := s.regenIndex(); err != nil {
		return "mission failed: " + err.Error(), true
	}
	return fmt.Sprintf("mission archived (%s: %s)", t.slug, outcome), false
}

// doHandoff writes the resume snapshot, rotating any previous handoff into
// archive/handoffs.md first.
func (s *Store) doHandoff(in Input) (string, bool) {
	if in.State == "" {
		return "mission failed: handoff requires state", true
	}
	if in.Next == "" {
		return "mission failed: handoff requires next", true
	}
	// Trigger is optional (defaults to manual below) but, when supplied by either
	// face — the mission tool or the CLI — must be one of the known values.
	if t := oneLine(in.Trigger); t != "" && !validTrigger(t) {
		return `mission failed: trigger must be one of manual, pre-compact, boundary, auto`, true
	}
	if err := s.rotateHandoff(); err != nil {
		return "mission failed: " + err.Error(), true
	}
	now := s.now().UTC()
	trigger := oneLine(in.Trigger)
	if trigger == "" {
		trigger = "manual"
	}
	var b strings.Builder
	b.WriteString("---\n")
	b.WriteString("written: " + now.Format(time.RFC3339) + "\n")
	// The mission MCP tool leaves Trigger empty ⇒ "manual"; the CLI twin supplies
	// pre-compact / boundary / auto (P0 ambiguity resolution #1).
	b.WriteString("trigger: " + trigger + "\n")
	b.WriteString("---\n\n")
	b.WriteString("**State:** " + in.State + "\n")
	b.WriteString("**Next:** " + in.Next + "\n")
	if in.Beware != "" {
		b.WriteString("**Beware:** " + in.Beware + "\n")
	}
	if err := atomicWrite(s.handoffPath(), []byte(b.String())); err != nil {
		return "mission failed: " + err.Error(), true
	}
	return "mission handoff saved (next: " + in.Next + ")", false
}

// validTrigger reports whether v is one of the handoff trigger enum values.
func validTrigger(v string) bool {
	switch v {
	case "manual", "pre-compact", "boundary", "auto":
		return true
	}
	return false
}

// rotateHandoff appends the current handoff.md (if any) to archive/handoffs.md,
// keeping the last maxArchivedHandoffs entries, then removes handoff.md.
func (s *Store) rotateHandoff() error {
	raw, err := os.ReadFile(s.handoffPath())
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	if err := os.MkdirAll(s.archiveDir(), 0o700); err != nil {
		return err
	}
	arcPath := filepath.Join(s.archiveDir(), "handoffs.md")
	prev, err := os.ReadFile(arcPath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	entries := parseArchivedHandoffs(string(prev))
	entries = append(entries, strings.TrimSpace(string(raw)))
	if len(entries) > maxArchivedHandoffs {
		entries = entries[len(entries)-maxArchivedHandoffs:]
	}
	if err := atomicWrite(arcPath, []byte(encodeArchivedHandoffs(entries))); err != nil {
		return err
	}
	return os.Remove(s.handoffPath())
}

// handoffFramePrefix opens each archived-handoff frame. Frames are
// length-prefixed (bytes=N) so an entry body containing the frame text — or any
// would-be separator like a literal "---8<---" line — can never split an entry:
// the parser consumes exactly N bytes and resumes past them.
const handoffFramePrefix = "<!-- hotline:handoff bytes="

// encodeArchivedHandoffs serializes entries as length-prefixed frames, each
// followed by a blank line for human readability.
func encodeArchivedHandoffs(entries []string) string {
	var b strings.Builder
	for _, e := range entries {
		fmt.Fprintf(&b, "%s%d -->\n%s\n\n", handoffFramePrefix, len(e), e)
	}
	return b.String()
}

// parseArchivedHandoffs reverses encodeArchivedHandoffs. Unframed legacy content
// (or garbage) yields no entries rather than a malformed split.
func parseArchivedHandoffs(data string) []string {
	var out []string
	for {
		i := strings.Index(data, handoffFramePrefix)
		if i < 0 {
			break
		}
		rest := data[i+len(handoffFramePrefix):]
		j := strings.Index(rest, " -->\n")
		if j < 0 {
			break
		}
		n, err := strconv.Atoi(rest[:j])
		if err != nil || n < 0 {
			break
		}
		body := rest[j+len(" -->\n"):]
		if len(body) < n {
			break
		}
		out = append(out, body[:n])
		data = body[n:]
	}
	return out
}

// ---- thread files ----

type thread struct {
	slug    string
	status  string
	summary string
	next    string
	updated string // RFC3339 UTC
	body    string // "# slug\n\n- log…" — everything after the front-matter
}

func newThread(slug string) *thread {
	return &thread{slug: slug, status: "active", body: "# " + slug + "\n"}
}

func (t *thread) appendLog(stamp, text string) {
	if !strings.HasSuffix(t.body, "\n") {
		t.body += "\n"
	}
	t.body += "- " + stamp + " — " + text + "\n"
}

func (t *thread) serialize() string {
	var b strings.Builder
	b.WriteString("---\n")
	b.WriteString("slug: " + t.slug + "\n")
	b.WriteString("status: " + orValue(t.status, "active") + "\n")
	b.WriteString("summary: " + t.summary + "\n")
	b.WriteString("next: " + t.next + "\n")
	b.WriteString("updated: " + t.updated + "\n")
	b.WriteString("---\n\n")
	b.WriteString(strings.TrimLeft(t.body, "\n"))
	if !strings.HasSuffix(b.String(), "\n") {
		b.WriteString("\n")
	}
	return b.String()
}

func (s *Store) loadThread(slug string) (*thread, bool, error) {
	raw, err := os.ReadFile(filepath.Join(s.threadsDir(), slug+".md"))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, false, nil
		}
		return nil, false, err
	}
	t := parseThread(slug, string(raw))
	return t, true, nil
}

func (s *Store) saveThread(t *thread) error {
	return atomicWrite(filepath.Join(s.threadsDir(), t.slug+".md"), []byte(t.serialize()))
}

// parseThread splits front-matter from body. A file without a leading "---"
// fence is treated as all-body (front-matter defaults apply), so a hand-edited
// file can never make the tool error.
func parseThread(slug, raw string) *thread {
	t := &thread{slug: slug, status: "active"}
	if !strings.HasPrefix(raw, "---\n") {
		t.body = raw
		return t
	}
	rest := raw[len("---\n"):]
	end := strings.Index(rest, "\n---")
	if end < 0 {
		t.body = raw
		return t
	}
	fm := rest[:end]
	body := rest[end+len("\n---"):]
	body = strings.TrimPrefix(body, "\n")
	body = strings.TrimLeft(body, "\n")
	for _, line := range strings.Split(fm, "\n") {
		k, v, ok := strings.Cut(line, ":")
		if !ok {
			continue
		}
		v = strings.TrimSpace(v)
		switch strings.TrimSpace(k) {
		case "slug":
			if v != "" {
				t.slug = v
			}
		case "status":
			t.status = v
		case "summary":
			t.summary = v
		case "next":
			t.next = v
		case "updated":
			t.updated = v
		}
	}
	t.body = body
	return t
}

// activeThreads reads every threads/*.md (active/paused work; done threads live
// under archive/), sorted by updated descending.
func (s *Store) activeThreads() ([]*thread, error) {
	ents, err := os.ReadDir(s.threadsDir())
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var out []*thread
	for _, e := range ents {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".md") {
			continue
		}
		slug := strings.TrimSuffix(e.Name(), ".md")
		t, ok, err := s.loadThread(slug)
		if err != nil {
			return nil, err
		}
		if ok {
			out = append(out, t)
		}
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].updated > out[j].updated })
	return out, nil
}

func (s *Store) unknownThread(slug string) string {
	ts, _ := s.activeThreads()
	names := make([]string, 0, len(ts))
	for _, t := range ts {
		names = append(names, t.slug)
	}
	active := "none"
	if len(names) > 0 {
		active = strings.Join(names, ", ")
	}
	return fmt.Sprintf("mission failed: unknown thread %q — active: %s", slug, active)
}

// ---- helpers ----

func (s *Store) nowRFC() string { return s.now().UTC().Format(time.RFC3339) }
func (s *Store) stamp() string  { return s.now().UTC().Format("2006-01-02 15:04") }

func badSlug() string {
	return `mission failed: thread must be a short kebab-case slug like "relay-cors"`
}

func validStatus(v string) bool { return v == "active" || v == "paused" || v == "done" }

func oneLine(s string) string {
	s = strings.ReplaceAll(s, "\r\n", " ")
	s = strings.ReplaceAll(s, "\n", " ")
	return strings.TrimSpace(s)
}

func orDash(s string) string {
	if s == "" {
		return "—"
	}
	return s
}

func orValue(s, def string) string {
	if s == "" {
		return def
	}
	return s
}

// clampField truncates a text field to maxField at a UTF-8 boundary (jobspool
// idiom): overflow is trimmed, never rejected.
func clampField(s string) string {
	if len(s) <= maxField {
		return s
	}
	cut := 0
	for i, r := range s {
		size := utf8.RuneLen(r)
		if i+size > maxField {
			break
		}
		cut = i + size
	}
	return s[:cut]
}

// withLock runs fn under an exclusive flock on mc/.lock, so the tool and a CLI
// twin (P1) never race a read-modify-write.
func (s *Store) withLock(fn func() error) error {
	if err := os.MkdirAll(s.dir, 0o700); err != nil {
		return err
	}
	lf, err := os.OpenFile(s.lockPath(), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return err
	}
	defer lf.Close()
	if err := syscall.Flock(int(lf.Fd()), syscall.LOCK_EX); err != nil {
		return fmt.Errorf("locking %s: %w", s.lockPath(), err)
	}
	defer syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
	return fn()
}

// atomicWrite writes data to path via tmp+rename (0600), creating parents.
func atomicWrite(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}
