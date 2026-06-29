package mcpchan

import (
	"encoding/json"
	"testing"
)

func TestPermReplyRe(t *testing.T) {
	accept := []string{"y abcde", "yes abcde", "n abcde", "NO ZMKJH", "  yes  abcde  "}
	for _, s := range accept {
		if PermReplyRe.FindStringSubmatch(s) == nil {
			t.Fatalf("expected accept: %q", s)
		}
	}
	reject := []string{
		"yes",           // no code
		"yes abcdef",    // 6 letters
		"yes abcl",      // too short
		"yes ablde",     // contains 'l'
		"sure abcde",    // not y/n
		"yes abcde now", // trailing chatter
		"yep abcde",     // not exact y/yes
	}
	for _, s := range reject {
		if PermReplyRe.FindStringSubmatch(s) != nil {
			t.Fatalf("expected reject: %q", s)
		}
	}
}

func TestPermBtnRe(t *testing.T) {
	if PermBtnRe.FindStringSubmatch("perm:allow:abcde") == nil {
		t.Fatal("expected allow match")
	}
	if PermBtnRe.FindStringSubmatch("perm:more:zmkjh") == nil {
		t.Fatal("expected more match")
	}
	if PermBtnRe.FindStringSubmatch("perm:bogus:abcde") != nil {
		t.Fatal("unexpected match on bad action")
	}
	if PermBtnRe.FindStringSubmatch("perm:allow:abcle") != nil {
		t.Fatal("code with 'l' should not match")
	}
}

func TestBehaviorFromYesNo(t *testing.T) {
	cases := map[string]string{"y": "allow", "yes": "allow", "Y": "allow", "n": "deny", "no": "deny", "N": "deny"}
	for in, want := range cases {
		if got := BehaviorFromYesNo(in); got != want {
			t.Fatalf("BehaviorFromYesNo(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestVerdictJSON(t *testing.T) {
	raw, err := json.Marshal(PermissionVerdictParams{RequestID: "abcde", Behavior: "allow"})
	if err != nil {
		t.Fatal(err)
	}
	if string(raw) != `{"request_id":"abcde","behavior":"allow"}` {
		t.Fatalf("verdict JSON = %s", raw)
	}
}

func TestPermissionRequestJSON(t *testing.T) {
	var p PermissionRequestParams
	in := `{"request_id":"abcde","tool_name":"Bash","description":"d","input_preview":"{}"}`
	if err := json.Unmarshal([]byte(in), &p); err != nil {
		t.Fatal(err)
	}
	if p.RequestID != "abcde" || p.ToolName != "Bash" {
		t.Fatalf("unmarshal mismatch: %+v", p)
	}
}

func TestInboundParamsMetaShape(t *testing.T) {
	// Meta is built by the caller, which omits absent keys. Verify marshaling
	// preserves exactly the provided keys and never injects empties.
	p := InboundParams{Content: "hi", Meta: map[string]string{"chat_id": "1", "user_id": "1"}}
	raw, err := json.Marshal(p)
	if err != nil {
		t.Fatal(err)
	}
	var back struct {
		Content string            `json:"content"`
		Meta    map[string]string `json:"meta"`
	}
	if err := json.Unmarshal(raw, &back); err != nil {
		t.Fatal(err)
	}
	if len(back.Meta) != 2 {
		t.Fatalf("expected 2 meta keys, got %d", len(back.Meta))
	}
	if _, ok := back.Meta["message_id"]; ok {
		t.Fatal("absent key should not be present")
	}
}
