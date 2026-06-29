package access

import (
	"path/filepath"
	"testing"
)

func TestGateMatrix(t *testing.T) {
	tests := []struct {
		name string
		acc  *Access
		in   GateInput
		want Decision
	}{
		{
			name: "disabled drops allowlisted DM",
			acc:  &Access{DMPolicy: "disabled", AllowFrom: []string{"1"}},
			in:   GateInput{ChatID: "1", SenderID: "1"},
			want: Drop,
		},
		{
			name: "disabled drops group",
			acc:  &Access{DMPolicy: "disabled", Groups: map[string]GroupPolicy{"-100": {}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9"},
			want: Drop,
		},
		{
			name: "DM allowlisted sender allowed",
			acc:  &Access{DMPolicy: "pairing", AllowFrom: []string{"7"}},
			in:   GateInput{ChatID: "7", SenderID: "7"},
			want: Allow,
		},
		{
			name: "DM pairing unknown -> pair",
			acc:  &Access{DMPolicy: "pairing"},
			in:   GateInput{ChatID: "8", SenderID: "8"},
			want: Pair,
		},
		{
			name: "DM allowlist unknown -> drop",
			acc:  &Access{DMPolicy: "allowlist"},
			in:   GateInput{ChatID: "8", SenderID: "8"},
			want: Drop,
		},
		{
			name: "group not configured -> drop",
			acc:  &Access{DMPolicy: "pairing", Groups: map[string]GroupPolicy{}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9"},
			want: Drop,
		},
		{
			name: "group allowFrom excludes sender -> drop",
			acc:  &Access{DMPolicy: "pairing", Groups: map[string]GroupPolicy{"-100": {AllowFrom: []string{"5"}}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9"},
			want: Drop,
		},
		{
			name: "group requireMention no mention -> drop",
			acc:  &Access{DMPolicy: "pairing", Groups: map[string]GroupPolicy{"-100": {RequireMention: true}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9", Text: "hi"},
			want: Drop,
		},
		{
			name: "group requireMention with mention -> allow",
			acc:  &Access{DMPolicy: "pairing", Groups: map[string]GroupPolicy{"-100": {RequireMention: true}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9", MentionedBot: true},
			want: Allow,
		},
		{
			name: "group requireMention with reply -> allow",
			acc:  &Access{DMPolicy: "pairing", Groups: map[string]GroupPolicy{"-100": {RequireMention: true}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9", RepliedToBot: true},
			want: Allow,
		},
		{
			name: "group no mention required -> allow",
			acc:  &Access{DMPolicy: "pairing", Groups: map[string]GroupPolicy{"-100": {RequireMention: false}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9"},
			want: Allow,
		},
		{
			name: "group mention via pattern -> allow",
			acc: &Access{DMPolicy: "pairing", MentionPatterns: []string{"^hey claude"},
				Groups: map[string]GroupPolicy{"-100": {RequireMention: true}}},
			in:   GateInput{IsGroup: true, ChatID: "-100", SenderID: "9", Text: "Hey Claude do x"},
			want: Allow,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := Gate(tt.acc, tt.in); got != tt.want {
				t.Fatalf("Gate() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestMatchesMentionPattern(t *testing.T) {
	a := &Access{MentionPatterns: []string{"foo[", "bar"}} // first is invalid regex
	if !MatchesMentionPattern(a, "a BAR b") {
		t.Fatal("expected match on valid pattern (case-insensitive)")
	}
	if MatchesMentionPattern(a, "nothing here") {
		t.Fatal("unexpected match")
	}
}

func TestNewPairingCode(t *testing.T) {
	c := NewPairingCode()
	if len(c) != 6 {
		t.Fatalf("code length = %d, want 6", len(c))
	}
	for _, r := range c {
		if !((r >= '0' && r <= '9') || (r >= 'a' && r <= 'f')) {
			t.Fatalf("non-hex char %q in code %q", r, c)
		}
	}
}

func TestPairingLifecycle(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "access.json")

	code, send, err := CreatePairing(file, "42", "42")
	if err != nil || !send {
		t.Fatalf("CreatePairing: send=%v err=%v", send, err)
	}

	// Reuse: same sender gets the same code.
	code2, send2, err := CreatePairing(file, "42", "42")
	if err != nil {
		t.Fatal(err)
	}
	if code2 != code {
		t.Fatalf("expected reused code %q, got %q", code, code2)
	}
	if !send2 {
		t.Fatal("expected send=true on first reuse")
	}

	// Approve.
	p, err := ApprovePairing(file, code)
	if err != nil {
		t.Fatal(err)
	}
	if p.SenderID != "42" {
		t.Fatalf("approved sender = %q", p.SenderID)
	}
	a, _ := Load(file)
	if !contains(a.AllowFrom, "42") {
		t.Fatal("sender not added to allowFrom")
	}
	if _, ok := a.Pending[code]; ok {
		t.Fatal("pending entry not removed after approve")
	}

	// Approve missing -> error.
	if _, err := ApprovePairing(file, "zzzzzz"); err != ErrNotPending {
		t.Fatalf("want ErrNotPending, got %v", err)
	}
}

func TestPairingRateLimit(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "access.json")
	var code string
	for i := 0; i < maxPairingReplies; i++ {
		c, send, err := CreatePairing(file, "1", "1")
		if err != nil {
			t.Fatal(err)
		}
		if !send {
			t.Fatalf("iteration %d: expected send=true", i)
		}
		code = c
	}
	// Next call hits the limit.
	c, send, err := CreatePairing(file, "1", "1")
	if err != nil {
		t.Fatal(err)
	}
	if send {
		t.Fatal("expected send=false past rate limit")
	}
	if c != code {
		t.Fatalf("expected same code at limit")
	}
}

func TestDenyPairing(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "access.json")
	code, _, _ := CreatePairing(file, "5", "5")
	if err := DenyPairing(file, code); err != nil {
		t.Fatal(err)
	}
	a, _ := Load(file)
	if len(a.Pending) != 0 {
		t.Fatal("pending not cleared after deny")
	}
	if err := DenyPairing(file, code); err != ErrNotPending {
		t.Fatalf("want ErrNotPending, got %v", err)
	}
}

func TestPurgeExpired(t *testing.T) {
	a := &Access{Pending: map[string]Pending{
		"old": {SenderID: "1", ExpiresAt: "2000-01-01T00:00:00Z"},
		"bad": {SenderID: "2", ExpiresAt: "not-a-time"},
	}}
	PurgeExpired(a)
	if len(a.Pending) != 0 {
		t.Fatalf("expected all expired/invalid purged, got %d", len(a.Pending))
	}
}

func TestLoadSaveRoundTripAndClamp(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "access.json")

	// Absent file -> defaults.
	a, err := Load(file)
	if err != nil {
		t.Fatal(err)
	}
	if a.DMPolicy != "pairing" || a.TextChunkLimit != MaxChunkLimit {
		t.Fatalf("defaults wrong: %+v", a)
	}
	if a.BubbleMode != "paced" {
		t.Fatalf("bubbleMode default = %q, want paced", a.BubbleMode)
	}

	a.TextChunkLimit = 999999 // should clamp on save/normalize
	a.AllowFrom = []string{"100"}
	if err := Save(a, file); err != nil {
		t.Fatal(err)
	}
	b, err := Load(file)
	if err != nil {
		t.Fatal(err)
	}
	if b.TextChunkLimit != MaxChunkLimit {
		t.Fatalf("clamp failed: %d", b.TextChunkLimit)
	}
	if !contains(b.AllowFrom, "100") {
		t.Fatal("round-trip lost allowFrom")
	}
}

func TestLoadClampLow(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "access.json")
	if err := Save(&Access{TextChunkLimit: -5}, file); err != nil {
		t.Fatal(err)
	}
	b, _ := Load(file)
	if b.TextChunkLimit != 1 {
		t.Fatalf("want clamp to 1, got %d", b.TextChunkLimit)
	}
}

func TestMutateConcurrentSerialized(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "access.json")
	// Seed.
	if err := Save(Defaults(), file); err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 10; i++ {
		id := string(rune('A' + i))
		if err := Mutate(file, func(a *Access) error {
			a.AllowFrom = append(a.AllowFrom, id)
			return nil
		}); err != nil {
			t.Fatal(err)
		}
	}
	a, _ := Load(file)
	if len(a.AllowFrom) != 10 {
		t.Fatalf("expected 10 allowFrom entries, got %d", len(a.AllowFrom))
	}
}
