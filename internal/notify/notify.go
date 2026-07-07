// Package notify implements hotline's third ingress leg: event-driven notifies
// from local scripts and daemons (backup jobs, the email sentry, CI, watchers).
// A script calls `hotline notify --source <key>`; the CLI runs the full gate
// (capability-key lookup, level clamp, sanitize, dedup, per-source rate limit,
// quiet hours) and durably enqueues an accepted event into spool.json. A
// Dispatcher on the daemon side injects enqueued events as synthetic inbound
// turns (kind="notify") through the same sink real messages and schedules use.
//
// State lives under <state root>/notify/: sources.json (the capability-key
// registry, operator-owned) and spool.json (pending entries plus per-source gate
// state), both guarded by the same flock/atomic-write pattern as schedules.json.
// The design is deliberately the existing house patterns applied to a new noun.
package notify

import (
	"fmt"
	"hash/fnv"
	"path/filepath"
	"regexp"
	"strings"
	"time"
	"unicode/utf8"
)

// maxPending caps the spool document size, mirroring schedule.maxSchedules — a
// full spool rejects at the gate (backpressure, not unbounded growth).
const maxPending = 50

// maxMessageBytes bounds one stored message (matches access.MaxChunkLimit's 4096
// and schedule.maxPromptLen). Longer input is truncated, never rejected.
const maxMessageBytes = 4096

// Token-bucket defaults: burst 5, refill 1 token per 5 minutes (~12/hour
// sustained). Overridable per source via the registry.
const (
	defaultBurst      = 5
	defaultRefillMins = 5
)

// dedupWindow is how long an identical message from the same source coalesces
// rather than enqueues again (rolling from the last acceptance of that hash).
const dedupWindow = 10 * time.Minute

// maxStdinBytes bounds a piped message read before sanitization truncates it.
const maxStdinBytes = 1 << 20 // 1 MiB

// MaxStdinBytes is the hard bound the CLI applies to a piped message read;
// Sanitize then truncates to maxMessageBytes.
func MaxStdinBytes() int64 { return maxStdinBytes }

// Level is a notify urgency. urgent > normal > low; only urgent bypasses quiet
// hours (nothing bypasses the rate limit).
type Level string

const (
	LevelUrgent Level = "urgent"
	LevelNormal Level = "normal"
	LevelLow    Level = "low"
)

// levelRank orders levels for clamping. Unknown levels rank below low so a
// corrupt registry cap never silently escalates.
func levelRank(l Level) int {
	switch l {
	case LevelUrgent:
		return 2
	case LevelNormal:
		return 1
	case LevelLow:
		return 0
	default:
		return -1
	}
}

// ParseLevel normalizes a CLI --level value. Empty defaults to normal; anything
// other than urgent/normal/low is a usage error.
func ParseLevel(s string) (Level, error) {
	switch l := Level(strings.ToLower(strings.TrimSpace(s))); l {
	case "":
		return LevelNormal, nil
	case LevelUrgent, LevelNormal, LevelLow:
		return l, nil
	default:
		return "", fmt.Errorf("invalid level %q (want urgent, normal, or low)", s)
	}
}

// capOrDefault treats an unset/invalid registered cap as normal — urgent must be
// granted deliberately at `source add` time.
func capOrDefault(cap Level) Level {
	if levelRank(cap) < 0 {
		return LevelNormal
	}
	return cap
}

// ClampLevel returns min(requested, cap) and whether the request was clamped. A
// level above the source's cap is clamped, not rejected — a misconfigured script
// still gets its event through, just without the escalation it isn't entitled to.
func ClampLevel(requested, cap Level) (Level, bool) {
	cap = capOrDefault(cap)
	if levelRank(requested) > levelRank(cap) {
		return cap, true
	}
	return requested, false
}

// Rate is a per-source token-bucket override. Zero fields mean the defaults.
type Rate struct {
	Burst      int `json:"burst,omitempty"`
	RefillMins int `json:"refillMins,omitempty"`
}

// effectiveRate resolves a source's bucket parameters, applying defaults for
// unset/invalid fields.
func effectiveRate(r Rate) (burst, refillMins float64) {
	burst = defaultBurst
	if r.Burst > 0 {
		burst = float64(r.Burst)
	}
	refillMins = defaultRefillMins
	if r.RefillMins > 0 {
		refillMins = float64(r.RefillMins)
	}
	return burst, refillMins
}

// Source is one registered capability key. The key is a bearer credential; every
// human-facing surface shows Label, never Key.
type Source struct {
	Label     string `json:"label"`
	Key       string `json:"key"`
	LevelCap  Level  `json:"levelCap"`
	Rate      Rate   `json:"rate,omitempty"`
	ChatID    string `json:"chatId,omitempty"`
	CreatedAt string `json:"createdAt"`
}

// Registry is the full persisted sources.json document. quietHours and
// defaultChatId are subsystem-level settings riding the same operator-owned file.
type Registry struct {
	QuietHours    string   `json:"quietHours"`
	DefaultChatID string   `json:"defaultChatId"`
	Sources       []Source `json:"sources"`
}

// FindByKey resolves a bearer key to its source with a constant-time compare
// over every registered key (the property a bearer token needs; cheap locally,
// free forward-compatibility for a future webhook ingress).
func (r *Registry) FindByKey(key string) (Source, bool) {
	var match Source
	found := false
	for _, s := range r.Sources {
		if constantTimeEqual(s.Key, key) {
			match = s
			found = true
		}
	}
	return match, found
}

// FindByLabel resolves a source by its human label (provenance / chat routing).
func (r *Registry) FindByLabel(label string) (Source, bool) {
	for _, s := range r.Sources {
		if s.Label == label {
			return s, true
		}
	}
	return Source{}, false
}

// Entry is one pending spool item awaiting injection.
type Entry struct {
	ID      string `json:"id"`
	Label   string `json:"label"`
	Level   Level  `json:"level"`
	Message string `json:"message"`
	Hash    string `json:"hash"`
	Status  string `json:"status"` // statusReady | statusQueued
	Count   int    `json:"count"`  // dedup coalesce counter
	Clamped bool   `json:"clamped,omitempty"`
	FirstAt string `json:"firstAt"`
	LastAt  string `json:"lastAt"`
}

const (
	statusReady  = "ready"
	statusQueued = "queued"
)

// SourceState is the persisted per-source gate state: token bucket, dedup
// fingerprint, suppression counters, and lifetime counters.
type SourceState struct {
	Tokens          float64 `json:"tokens"`
	TokensAt        string  `json:"tokensAt,omitempty"`
	LastHash        string  `json:"lastHash,omitempty"`
	LastHashAt      string  `json:"lastHashAt,omitempty"`
	Suppressed      int     `json:"suppressed,omitempty"`
	SuppressedSince string  `json:"suppressedSince,omitempty"`
	Delivered       int     `json:"delivered,omitempty"`
	LastSeen        string  `json:"lastSeen,omitempty"`
}

// SpoolDoc is the full persisted spool.json document.
type SpoolDoc struct {
	Pending []Entry                 `json:"pending"`
	State   map[string]*SourceState `json:"state"`
}

// normalize guarantees non-nil collections so callers never nil-check.
func (d *SpoolDoc) normalize() {
	if d.Pending == nil {
		d.Pending = []Entry{}
	}
	if d.State == nil {
		d.State = map[string]*SourceState{}
	}
}

// stateFor returns the mutable per-source state, creating it on first use.
func (d *SpoolDoc) stateFor(label string) *SourceState {
	if d.State == nil {
		d.State = map[string]*SourceState{}
	}
	st := d.State[label]
	if st == nil {
		st = &SourceState{}
		d.State[label] = st
	}
	return st
}

// Path helpers: everything notify owns lives under <state root>/notify/.
func Dir(stateRoot string) string         { return filepath.Join(stateRoot, "notify") }
func SourcesPath(stateRoot string) string { return filepath.Join(Dir(stateRoot), "sources.json") }
func SpoolPath(stateRoot string) string   { return filepath.Join(Dir(stateRoot), "spool.json") }
func RejectsPath(stateRoot string) string { return filepath.Join(Dir(stateRoot), "rejects.log") }

// closeChannelRe matches a case-insensitive envelope-close token so a
// script-authored payload can't forge or terminate the <channel> wrapper.
var closeChannelRe = regexp.MustCompile(`(?i)</channel`)

// Sanitize cleans a script-authored message once, at enqueue, so the spool only
// ever holds clean payloads: it neutralizes envelope-close forgery, strips
// control characters (ANSI escapes) except \n and \t, and truncates to
// maxMessageBytes at a UTF-8 boundary.
func Sanitize(s string) string {
	// Neutralize "</channel" -> "<\channel" (preserving the tag name's case),
	// closing the one structural escape RenderChannel would otherwise pass raw.
	s = closeChannelRe.ReplaceAllStringFunc(s, func(m string) string {
		return "<\\" + m[2:]
	})
	// Strip control characters except newline and tab.
	s = strings.Map(func(r rune) rune {
		if r == '\n' || r == '\t' {
			return r
		}
		if r < 0x20 || r == 0x7f {
			return -1
		}
		return r
	}, s)
	// Cap length, truncating whole runes and flagging it (truncate, don't reject).
	// Budget for the suffix so the final stored message is a hard <= maxMessageBytes.
	if len(s) > maxMessageBytes {
		const suffix = "…[truncated]"
		s = truncateUTF8(s, maxMessageBytes-len(suffix)) + suffix
	}
	return s
}

// truncateUTF8 returns the longest prefix of whole runes fitting in max bytes.
func truncateUTF8(s string, max int) string {
	if len(s) <= max {
		return s
	}
	cut := 0
	for i, r := range s {
		size := utf8.RuneLen(r)
		if i+size > max {
			break
		}
		cut = i + size
	}
	return s[:cut]
}

// hashMessage fingerprints a sanitized message with FNV-64a. Not
// security-relevant — a collision merely coalesces two different messages from
// the same source within the dedup window, which is harmless.
func hashMessage(s string) string {
	h := fnv.New64a()
	_, _ = h.Write([]byte(s))
	return fmt.Sprintf("%016x", h.Sum64())
}

// rfc formats an instant as the stored RFC3339 UTC string (schedule's convention).
func rfc(t time.Time) string { return t.UTC().Format(time.RFC3339) }

// parseTime parses a stored RFC3339 instant; a bad/empty value yields the zero time.
func parseTime(s string) time.Time {
	t, err := time.Parse(time.RFC3339, s)
	if err != nil {
		return time.Time{}
	}
	return t
}

// hhmm renders an instant as local "HH:MM" for human-facing lines.
func hhmm(t time.Time) string {
	if t.IsZero() {
		return "??:??"
	}
	return t.In(time.Local).Format("15:04")
}
