package harness

import (
	"strings"
	"testing"
)

func TestRenderChannelIncludesRoutingKeys(t *testing.T) {
	in := Inbound{
		Content: "deploy the thing",
		Meta: map[string]string{
			"source":     "telegram",
			"chat_id":    "412407481",
			"user":       "George",
			"message_id": "42",
		},
	}
	got := RenderChannel(in)

	// source and chat_id lead, in that order — the routing keys the agent must
	// echo into hotline_reply.
	wantPrefix := `<channel source="telegram" chat_id="412407481"`
	if !strings.HasPrefix(got, wantPrefix) {
		t.Fatalf("envelope prefix = %q, want prefix %q", got, wantPrefix)
	}
	if !strings.Contains(got, `user="George"`) || !strings.Contains(got, `message_id="42"`) {
		t.Fatalf("envelope dropped a meta attribute: %q", got)
	}
	if !strings.HasSuffix(got, "deploy the thing\n</channel>") {
		t.Fatalf("envelope must wrap the content: %q", got)
	}
}

func TestRenderChannelEmptyMetaReturnsContent(t *testing.T) {
	in := Inbound{Content: "hi"}
	if got := RenderChannel(in); got != "hi" {
		t.Fatalf("empty meta should return bare content, got %q", got)
	}
}

func TestRenderChannelDeterministicUnknownKeys(t *testing.T) {
	in := Inbound{
		Content: "x",
		Meta:    map[string]string{"source": "telegram", "chat_id": "1", "zeta": "z", "alpha": "a"},
	}
	got1 := RenderChannel(in)
	got2 := RenderChannel(in)
	if got1 != got2 {
		t.Fatalf("render not deterministic:\n%s\n%s", got1, got2)
	}
	// Unknown keys sorted after the known ones: alpha before zeta.
	if i, j := strings.Index(got1, "alpha="), strings.Index(got1, "zeta="); i < 0 || j < 0 || i > j {
		t.Fatalf("unknown keys not sorted deterministically: %q", got1)
	}
}

func TestRenderChannelEscapesValues(t *testing.T) {
	in := Inbound{
		Content: "hello",
		Meta:    map[string]string{"source": "telegram", "chat_id": "1", "user": `a"><b`},
	}
	got := RenderChannel(in)
	if strings.Contains(got, `a"><b`) {
		t.Fatalf("attribute value not escaped — meta could break out of the tag: %q", got)
	}
	if !strings.Contains(got, "&#34;") || !strings.Contains(got, "&gt;") {
		t.Fatalf("expected escaped value in %q", got)
	}
}

// TestRenderChannelDoesNotDoubleMarkFleet proves H3: RenderChannel no longer
// injects the trust marker itself (the marker rides the injected CONTENT via
// app.injectInbound so the default Claude path sees it too). A fleet turn whose
// content already carries the marker passes it through EXACTLY ONCE — pi/opencode
// are never double-marked — and RenderChannel adds no second copy of its own.
func TestRenderChannelDoesNotDoubleMarkFleet(t *testing.T) {
	const marker = "untrusted peer data"
	// Content-borne marker (what injectInbound produces): survives once.
	fleet := RenderChannel(Inbound{
		Content: "[fleet peer message — " + marker + " …]\nhey peer",
		Meta:    map[string]string{"source": "fleet", "chat_id": "fleet:abcd1234", "kind": "fleet"},
	})
	if n := strings.Count(fleet, marker); n != 1 {
		t.Fatalf("marker count = %d, want exactly 1 (no double-mark):\n%s", n, fleet)
	}
	if !strings.Contains(fleet, "hey peer") {
		t.Fatalf("fleet turn dropped its content:\n%s", fleet)
	}
	// Fleet meta but NO content marker: RenderChannel must not add one on its own.
	bare := RenderChannel(Inbound{
		Content: "hey peer",
		Meta:    map[string]string{"source": "fleet", "chat_id": "fleet:abcd1234", "kind": "fleet"},
	})
	if strings.Contains(bare, marker) {
		t.Fatalf("RenderChannel re-introduced the marker (double-mark risk):\n%s", bare)
	}
}
