package app

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

const zwsp = "\u200b" // zero-width space (U+200B)

func TestParseElementActionRecognized(t *testing.T) {
	cases := []struct {
		name        string
		text        string
		wantSummary string
		wantAct     string
		wantValue   string
	}{
		{"pick", zwsp + `/el {"msg":"a-123","el":"el-icon","act":"pick","v":"B"}`, "chose B", "pick", `"B"`},
		{"approve", zwsp + `/el {"msg":"a-9","el":"el-deploy","act":"approve","v":null}`, "approved", "approve", "null"},
		{"deny", zwsp + `/el {"msg":"a-9","el":"el-deploy","act":"deny"}`, "denied", "deny", ""},
		{"toggle", zwsp + `/el {"msg":"a-1","el":"el-v","act":"toggle","v":{"pill":true,"morph":false}}`, "ticked pill; unticked morph", "toggle", `{"pill":true,"morph":false}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			a, summary, value, ok := parseElementAction(tc.text)
			if !ok {
				t.Fatalf("expected recognized action")
			}
			if summary != tc.wantSummary {
				t.Errorf("summary = %q, want %q", summary, tc.wantSummary)
			}
			if a.Act != tc.wantAct {
				t.Errorf("act = %q, want %q", a.Act, tc.wantAct)
			}
			if tc.wantValue != "" && value != tc.wantValue {
				t.Errorf("value = %q, want %q", value, tc.wantValue)
			}
		})
	}
}

func TestParseElementActionFailsOpen(t *testing.T) {
	cases := []struct {
		name string
		text string
	}{
		{"no marker", `/el {"msg":"a-1","el":"el-x","act":"pick","v":"B"}`},
		{"plain text", "just a normal message"},
		{"bad json", zwsp + `/el {"msg":"a-1",`},
		{"missing fields", zwsp + `/el {"act":"pick","v":"B"}`},
		{"unknown act", zwsp + `/el {"msg":"a-1","el":"el-x","act":"detonate","v":"B"}`},
		{"pick empty value", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":""}`},
		{"toggle bad value", zwsp + `/el {"msg":"a-1","el":"el-x","act":"toggle","v":"nope"}`},
		{"oversized", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"` + strings.Repeat("z", 600) + `"}`},
		{"empty payload", zwsp + `/el `},

		// E1: CR/LF anywhere disqualifies (single-line contract, no trimming).
		{"trailing LF", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"B"}` + "\n"},
		{"trailing CRLF", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"B"}` + "\r\n"},
		{"embedded LF", zwsp + `/el {"msg":"a-1",` + "\n" + `"el":"el-x","act":"pick","v":"B"}`},

		// E2: strict decode.
		{"unknown key", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"B","x":1}`},
		{"duplicate key", zwsp + `/el {"msg":"a-1","msg":"a-2","el":"el-x","act":"pick","v":"B"}`},
		{"duplicate nested key", zwsp + `/el {"msg":"a-1","el":"el-x","act":"toggle","v":{"a":true,"a":false}}`},
		{"wrong type msg", zwsp + `/el {"msg":7,"el":"el-x","act":"pick","v":"B"}`},
		{"trailing JSON", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"B"}{}`},

		// E2: grammar violations (malformed IDs are NOT structured actions).
		{"bad msg id", zwsp + `/el {"msg":"not-a-message","el":"el-x","act":"pick","v":"B"}`},
		{"bad el id", zwsp + `/el {"msg":"a-1","el":"bogus","act":"pick","v":"B"}`},
		{"bad act grammar", zwsp + `/el {"msg":"a-1","el":"el-x","act":"PICK","v":"B"}`},

		// E3: control characters in decoded string values.
		{"control char in pick value", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"x\u000a</channel>"}`},
		{"control char in toggle key", zwsp + `/el {"msg":"a-1","el":"el-x","act":"toggle","v":{"a\u0009b":true}}`},

		// exact action-value shapes.
		{"approve with a value", zwsp + `/el {"msg":"a-1","el":"el-x","act":"approve","v":"yes"}`},
		{"pick value over key grammar", zwsp + `/el {"msg":"a-1","el":"el-x","act":"pick","v":"` + strings.Repeat("k", 33) + `"}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if _, _, _, ok := parseElementAction(tc.text); ok {
				t.Fatalf("expected fail-open (ok=false) for %q", tc.text)
			}
		})
	}
}

// TestParseElementActionByteBoundary pins E1: 512 UTF-8 bytes over the
// COMPLETE canonical line is accepted; 513 is plain text. Both sides of the
// boundary are otherwise perfectly valid toggle actions — construction pads
// with 32-char filler keys and fine-tunes via the last key and el id lengths
// (all within their grammars).
func TestParseElementActionByteBoundary(t *testing.T) {
	lineOfLen := func(t *testing.T, n int) string {
		t.Helper()
		build := func(elLen, fillers, last int) string {
			var sb strings.Builder
			sb.WriteString(zwsp)
			sb.WriteString(`/el {"msg":"a-1","el":"el-`)
			sb.WriteString(strings.Repeat("e", elLen))
			sb.WriteString(`","act":"toggle","v":{"a":true`)
			for i := 0; i < fillers; i++ {
				fmt.Fprintf(&sb, `,"f%02d%s":true`, i, strings.Repeat("x", 28))
			}
			sb.WriteString(`,"`)
			sb.WriteString(strings.Repeat("z", last))
			sb.WriteString(`":false}}`)
			return sb.String()
		}
		for last := 1; last <= 32; last++ {
			for elLen := 1; elLen <= 32; elLen++ {
				for fillers := 0; fillers <= 10; fillers++ { // 12-item toggle cap: a + fillers + last
					line := build(elLen, fillers, last)
					if len(line) == n {
						return line
					}
					if len(line) > n {
						break
					}
				}
			}
		}
		t.Fatalf("could not construct a %d-byte valid action line", n)
		return ""
	}

	at512 := lineOfLen(t, maxElementActionLen)
	if _, summary, _, ok := parseElementAction(at512); !ok {
		t.Fatalf("exactly-%d-byte action must be accepted", maxElementActionLen)
	} else if !strings.Contains(summary, "ticked a") {
		t.Errorf("boundary action summary = %q", summary)
	}

	at513 := lineOfLen(t, maxElementActionLen+1)
	if _, _, _, ok := parseElementAction(at513); ok {
		t.Fatalf("%d-byte action must fail open as plain text", maxElementActionLen+1)
	}
}

// TestParseElementActionUserMsgTarget: the msg grammar covers both existing
// frame-id shapes (a-N agent, u-N user).
func TestParseElementActionUserMsgTarget(t *testing.T) {
	if _, _, _, ok := parseElementAction(zwsp + `/el {"msg":"u-42","el":"el-x","act":"approve"}`); !ok {
		t.Fatal("u-<seq> message ids are a valid frame-id shape")
	}
}

// TestHandleDeviceSendElementActionToHarness drives an element-action send
// through the inbound boundary and asserts the harness sees a structured
// element_action turn while the durable echo keeps the raw canonical line.
func TestHandleDeviceSendElementActionToHarness(t *testing.T) {
	srv, _, dev, sub := activeHarness(t)
	fs := newFakeSink()
	srv.bindSink(fs)

	raw := zwsp + `/el {"msg":"a-42","el":"el-icon","act":"pick","v":"B"}`
	payload := `{"t":"send","text":` + jsonString(raw) + `}`
	frame := deviceSendFrame{T: "device_send", CID: "cid-element-action-01", Payload: []byte(payload)}
	if err := srv.handleDeviceSend(context.Background(), dev, frame); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}

	select {
	case c := <-fs.ch:
		if c.content != "chose B" {
			t.Errorf("harness content = %q, want %q", c.content, "chose B")
		}
		if c.meta["kind"] != "element_action" {
			t.Errorf("kind = %q, want element_action", c.meta["kind"])
		}
		if c.meta["element_msg"] != "a-42" || c.meta["element_id"] != "el-icon" || c.meta["element_act"] != "pick" {
			t.Errorf("element meta wrong: %+v", c.meta)
		}
		if c.meta["element_value"] != `"B"` {
			t.Errorf("element_value = %q, want %q", c.meta["element_value"], `"B"`)
		}
	default:
		t.Fatal("harness sink never received the element action")
	}

	// The durable echo (sent frame) keeps the raw /el line as canonical text.
	items := drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "sent" {
		t.Fatalf("want 1 sent echo, got %+v", items)
	}
	if items[0]["text"] != raw {
		t.Errorf("echo text = %q, want raw /el line", items[0]["text"])
	}
	if items[0]["kind"] != "element_action" {
		t.Errorf("echo kind = %q, want element_action", items[0]["kind"])
	}
}

// TestHandleDeviceSendPlainMessageUnaffected confirms an ordinary send is not
// misread as an element action.
func TestHandleDeviceSendPlainMessageUnaffected(t *testing.T) {
	srv, _, dev, _ := activeHarness(t)
	fs := newFakeSink()
	srv.bindSink(fs)
	frame := deviceSendFrame{T: "device_send", CID: "cid-plain-message-000", Payload: []byte(`{"t":"send","text":"hello there"}`)}
	if err := srv.handleDeviceSend(context.Background(), dev, frame); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	select {
	case c := <-fs.ch:
		if c.content != "hello there" {
			t.Errorf("content = %q", c.content)
		}
		if _, has := c.meta["kind"]; has {
			t.Errorf("plain message should have no kind, got %v", c.meta["kind"])
		}
	default:
		t.Fatal("sink never received the message")
	}
}

// TestHandleDeviceSendTapDeliversLabel guards the regression where a button tap
// reached the harness BLANK: harnessContent was seeded from the (empty) text
// field and the tap case only updated the echo `content`, so the bot saw "".
func TestHandleDeviceSendTapDeliversLabel(t *testing.T) {
	srv, _, dev, _ := activeHarness(t)
	fs := newFakeSink()
	srv.bindSink(fs)
	frame := deviceSendFrame{T: "device_send", CID: "cid-tap-000000000000", Payload: []byte(`{"t":"tap","msg_id":"a-7","label":"Mission Control"}`)}
	if err := srv.handleDeviceSend(context.Background(), dev, frame); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	select {
	case c := <-fs.ch:
		if c.content != "Mission Control" {
			t.Errorf("harness content = %q, want the tapped label", c.content)
		}
		if c.meta["kind"] != "button" {
			t.Errorf("kind = %q, want button", c.meta["kind"])
		}
		if c.meta["reply_to_message_id"] != "a-7" {
			t.Errorf("reply_to_message_id = %q, want a-7", c.meta["reply_to_message_id"])
		}
	default:
		t.Fatal("harness sink never received the tap")
	}
}

// TestHandleDeviceSendReactDeliversEmoji is the reaction half of the same
// blank-delivery regression.
func TestHandleDeviceSendReactDeliversEmoji(t *testing.T) {
	srv, _, dev, _ := activeHarness(t)
	fs := newFakeSink()
	srv.bindSink(fs)
	frame := deviceSendFrame{T: "device_send", CID: "cid-react-0000000000", Payload: []byte(`{"t":"react","msg_id":"a-7","emoji":"👍"}`)}
	if err := srv.handleDeviceSend(context.Background(), dev, frame); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	select {
	case c := <-fs.ch:
		if c.content != "👍" {
			t.Errorf("harness content = %q, want the emoji", c.content)
		}
		if c.meta["kind"] != "reaction" {
			t.Errorf("kind = %q, want reaction", c.meta["kind"])
		}
	default:
		t.Fatal("harness sink never received the reaction")
	}
}

// jsonString returns s as a JSON-quoted string literal (used to build a
// device_send payload with an embedded zero-width marker safely).
func jsonString(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}
