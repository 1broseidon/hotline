package opencode

import (
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// TestNewCodeMatchesRelayAlphabet proves the minted codes are exactly what the
// provider permission relay accepts: 5 letters from [a-km-z] (no 'l'), so a
// texted "yes <code>" and a "perm:allow:<code>" button both match.
func TestNewCodeMatchesRelayAlphabet(t *testing.T) {
	for i := 0; i < 2000; i++ {
		code := newCode()
		if len(code) != 5 {
			t.Fatalf("code %q length %d, want 5", code, len(code))
		}
		if strings.ContainsRune(code, 'l') {
			t.Fatalf("code %q contains forbidden 'l'", code)
		}
		if !mcpchan.PermReplyRe.MatchString("yes " + code) {
			t.Fatalf("code %q not accepted by PermReplyRe", code)
		}
		if !mcpchan.PermBtnRe.MatchString("perm:allow:" + code) {
			t.Fatalf("code %q not accepted by PermBtnRe", code)
		}
	}
}

// TestNewCodeVariesAcrossAlphabet sanity-checks that rejection sampling still
// reaches the whole alphabet (every allowed letter appears at least once over
// many draws).
func TestNewCodeVariesAcrossAlphabet(t *testing.T) {
	seen := map[rune]bool{}
	for i := 0; i < 5000; i++ {
		for _, r := range newCode() {
			seen[r] = true
		}
	}
	for _, r := range codeAlphabet {
		if !seen[r] {
			t.Fatalf("letter %q never produced across draws", string(r))
		}
	}
}
